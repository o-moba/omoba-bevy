//! YooKassa settlement and Hero tiers (LR-67) against a disposable PostgreSQL
//! database. Provider objects are synthetic: they stand for payments and
//! refunds the provider flow re-read from YooKassa with the shop credentials.
use omoba_account_api::{
    App, Config, crypto, migrate, now,
    supporter::{
        self, recognition,
        yookassa::{
            self, KOPECKS, PaymentStatus, Settlement, VerifiedPayment, VerifiedRefund,
            settle_payment, settle_refund,
        },
    },
};
use shared::supporter::{AuraStyle, SupporterTier};
use sqlx::Row;

fn config() -> Config {
    Config {
        origin: "https://players.example".into(),
        secret: [83; 32],
        trust_loopback_proxy: false,
        release_file: None,
    }
}

async fn app() -> App {
    let url =
        std::env::var("OMOBA_PORTAL_TEST_DATABASE_URL").expect("isolated test database required");
    migrate(&url).await.unwrap();
    App::connect(&url, config()).await.unwrap()
}

async fn account(app: &App) -> String {
    app.career
        .authenticate(&crypto::random::<32>(), "Donor")
        .await
        .unwrap()
        .profile_id
}

/// An attempt as the checkout leaves it: key persisted, create submitted.
async fn attempt(app: &App, profile: &str, rubles: i64, payment: Option<&str>) -> String {
    let id = crypto::random::<16>();
    let receipt = serde_json::json!({
        "customer": {"email": "donor@example.test"},
        "items": [{
            "description": "OMOBA — поддержка проекта (аура на 30 дней)",
            "quantity": "1.00",
            "amount": {"value": format!("{rubles}.00"), "currency": "RUB"},
            "vat_code": 1,
            "payment_mode": "full_payment",
            "payment_subject": "service"
        }]
    });
    sqlx::query("INSERT INTO portal.supporter_yookassa_attempts(attempt_id,profile_id,request_key,idempotence_key,provider_payment_id,amount_minor,receipt,offer_version,status,first_provider_attempt_at,created_at,updated_at) VALUES($1,$2,$3,$4,$5,$6,$7,'0-draft','submitted',$8,$8,$8)")
        .bind(&id).bind(profile).bind(crypto::random::<32>()).bind(crypto::random::<16>())
        .bind(payment).bind(rubles * 100).bind(receipt).bind(now())
        .execute(&app.pool).await.unwrap();
    id
}

fn paid(id: &str, rubles: i64, at: i64) -> VerifiedPayment {
    VerifiedPayment {
        payment_id: id.into(),
        status: PaymentStatus::Succeeded,
        amount_minor: rubles * 100,
        paid_at: Some(at),
        attempt_id: None,
    }
}

fn refund(id: &str, payment: &str, rubles: i64) -> VerifiedRefund {
    VerifiedRefund {
        refund_id: id.into(),
        payment_id: payment.into(),
        amount_minor: rubles * 100,
        refunded_at: now(),
    }
}

async fn status_of(app: &App, attempt: &str) -> String {
    sqlx::query_scalar("SELECT status FROM portal.supporter_yookassa_attempts WHERE attempt_id=$1")
        .bind(attempt)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn grant(app: &App, payment: &str) -> (i64, i32, Option<i64>) {
    let row = sqlx::query("SELECT amount_minor,tier_points,revoked_at FROM portal.supporter_grants WHERE provider='yookassa' AND period_id=$1")
        .bind(payment)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    (
        row.get("amount_minor"),
        row.get("tier_points"),
        row.get("revoked_at"),
    )
}

async fn cleanup(app: &App, profile: &str) {
    for query in [
        "DELETE FROM portal.supporter_yookassa_refunds WHERE provider_payment_id IN (SELECT provider_payment_id FROM portal.supporter_yookassa_attempts WHERE profile_id=$1)",
        "DELETE FROM portal.supporter_yookassa_attempts WHERE profile_id=$1",
        "DELETE FROM portal.supporter_events WHERE profile_id=$1",
        "DELETE FROM portal.supporter_grants WHERE profile_id=$1",
        "DELETE FROM portal.supporter_preferences WHERE profile_id=$1",
    ] {
        sqlx::query(query)
            .bind(profile)
            .execute(&app.pool)
            .await
            .unwrap();
    }
}

#[tokio::test]
#[ignore = "requires isolated OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn a_donation_grants_once_with_its_points_and_a_late_cancel_never_downgrades() {
    let app = app().await;
    let profile = account(&app).await;
    let payment = format!("pay-{}", crypto::random::<8>());
    let id = attempt(&app, &profile, 1000, Some(&payment)).await;
    let paid_at = now() - 60;
    let succeeded = paid(&payment, 1000, paid_at);
    assert_eq!(
        settle_payment(&app, &succeeded).await.unwrap(),
        Settlement::Changed
    );
    assert_eq!(status_of(&app, &id).await, "succeeded");
    assert_eq!(grant(&app, &payment).await, (100_000, 1000, None));
    // Duplicate webhooks and the reconciliation loop settle nothing twice.
    for _ in 0..2 {
        assert_eq!(
            settle_payment(&app, &succeeded).await.unwrap(),
            Settlement::Unchanged
        );
    }
    let canceled = VerifiedPayment {
        status: PaymentStatus::Canceled,
        paid_at: None,
        ..succeeded.clone()
    };
    assert_eq!(
        settle_payment(&app, &canceled).await.unwrap(),
        Settlement::Unchanged
    );
    assert_eq!(status_of(&app, &id).await, "succeeded");
    let grants: i64 =
        sqlx::query_scalar("SELECT count(*) FROM portal.supporter_grants WHERE profile_id=$1")
            .bind(&profile)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(grants, 1);
    // 1000 points: Crimson, the default after a first grant, until the grant ends.
    let status = supporter::status(&app, &profile).await.unwrap();
    assert!(status.active);
    assert_eq!(status.tier_points, 1000);
    assert_eq!(status.tier(), Some(SupporterTier::Crimson));
    assert_eq!(status.tier_until, Some(paid_at + yookassa::GRANT_SECONDS));
    assert_eq!(status.equipped_aura, Some(AuraStyle::Crimson));
    assert_eq!(status.grants[0].tier_points, 1000);
    let error = supporter::equip(&app, &profile, br#"{"aura":"prism_i"}"#)
        .await
        .unwrap_err();
    assert_eq!(error.1, "tier_required");
    let dashboard = supporter::equip(&app, &profile, br#"{"aura":"rose"}"#)
        .await
        .unwrap();
    assert_eq!(dashboard["supporter"]["equipped_aura"], "rose");
    let unlocked: Vec<(String, bool)> = dashboard["catalog"]
        .as_array()
        .unwrap()
        .iter()
        .map(|style| {
            (
                style["id"].as_str().unwrap().to_owned(),
                style["unlocked"].as_bool().unwrap(),
            )
        })
        .collect();
    for (id, open) in &unlocked {
        let style = AuraStyle::from_id(id).unwrap();
        assert_eq!(*open, style.required_points() <= 1000, "{id}");
    }
    assert_eq!(unlocked.len(), AuraStyle::ALL.len());
    cleanup(&app, &profile).await;
}

#[tokio::test]
#[ignore = "requires isolated OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn refunds_overtake_payments_and_partial_refunds_recompute_the_tier() {
    let app = app().await;
    let profile = account(&app).await;
    let payment = format!("pay-{}", crypto::random::<8>());
    let id = attempt(&app, &profile, 2500, Some(&payment)).await;
    let succeeded = paid(&payment, 2500, now() - 120);
    // The refund webhook arrives first: the payment is settled, then refunded.
    let first = refund(&format!("ref-{}", crypto::random::<8>()), &payment, 1500);
    assert_eq!(
        settle_refund(&app, &succeeded, &first).await.unwrap(),
        Settlement::Changed
    );
    assert_eq!(status_of(&app, &id).await, "succeeded");
    assert_eq!(grant(&app, &payment).await, (250_000, 1000, None));
    let status = supporter::status(&app, &profile).await.unwrap();
    assert_eq!(status.tier(), Some(SupporterTier::Crimson));
    // A replayed refund and a replayed payment change nothing.
    assert_eq!(
        settle_refund(&app, &succeeded, &first).await.unwrap(),
        Settlement::Unchanged
    );
    assert_eq!(
        settle_payment(&app, &succeeded).await.unwrap(),
        Settlement::Unchanged
    );
    assert_eq!(grant(&app, &payment).await.1, 1000);
    // The same refund id with another amount is a conflict, not a second refund.
    let forged = VerifiedRefund {
        amount_minor: 100 * KOPECKS,
        ..first.clone()
    };
    assert_eq!(
        settle_refund(&app, &succeeded, &forged)
            .await
            .unwrap_err()
            .1,
        "state_changed"
    );
    // A second partial refund: 2500 - 1500 - 750 = 250 points (Jade).
    let second = refund(&format!("ref-{}", crypto::random::<8>()), &payment, 750);
    settle_refund(&app, &succeeded, &second).await.unwrap();
    assert_eq!(grant(&app, &payment).await.1, 250);
    let status = supporter::status(&app, &profile).await.unwrap();
    assert_eq!(status.tier(), Some(SupporterTier::Jade));
    assert_eq!(status.equipped_aura, Some(AuraStyle::Jade));
    // The rest: a full refund revokes the grant and ends access.
    let last = refund(&format!("ref-{}", crypto::random::<8>()), &payment, 250);
    settle_refund(&app, &succeeded, &last).await.unwrap();
    let (_, points, revoked) = grant(&app, &payment).await;
    assert_eq!(points, 0);
    assert!(revoked.is_some());
    let status = supporter::status(&app, &profile).await.unwrap();
    assert!(!status.active && status.tier().is_none() && status.equipped_aura.is_none());
    // Nothing can be refunded beyond the payment, and nothing revives it.
    let extra = refund(&format!("ref-{}", crypto::random::<8>()), &payment, 1);
    assert!(settle_refund(&app, &succeeded, &extra).await.is_err());
    assert_eq!(
        settle_payment(&app, &succeeded).await.unwrap(),
        Settlement::Unchanged
    );
    assert!(!supporter::status(&app, &profile).await.unwrap().active);
    cleanup(&app, &profile).await;
}

#[tokio::test]
#[ignore = "requires isolated OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn unknown_payments_write_nothing_and_lost_creates_bind_by_attempt() {
    let app = app().await;
    let profile = account(&app).await;
    let unknown = paid(&format!("pay-{}", crypto::random::<8>()), 500, now());
    assert_eq!(
        settle_payment(&app, &unknown).await.unwrap(),
        Settlement::Unknown
    );
    let stranger = refund("ref-unknown", &unknown.payment_id, 500);
    assert_eq!(
        settle_refund(&app, &unknown, &stranger).await.unwrap(),
        Settlement::Unknown
    );
    let events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM portal.supporter_events WHERE period_id=$1")
            .bind(&unknown.payment_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(events, 0);
    // The create response was lost: the payment carries our attempt id.
    let id = attempt(&app, &profile, 500, None).await;
    let pending = VerifiedPayment {
        status: PaymentStatus::Pending,
        paid_at: None,
        attempt_id: Some(id.clone()),
        ..unknown.clone()
    };
    sqlx::query(
        "UPDATE portal.supporter_yookassa_attempts SET status='charging' WHERE attempt_id=$1",
    )
    .bind(&id)
    .execute(&app.pool)
    .await
    .unwrap();
    assert_eq!(
        settle_payment(&app, &pending).await.unwrap(),
        Settlement::Changed
    );
    assert_eq!(status_of(&app, &id).await, "submitted");
    let bound: Option<String> = sqlx::query_scalar(
        "SELECT provider_payment_id FROM portal.supporter_yookassa_attempts WHERE attempt_id=$1",
    )
    .bind(&id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(bound.as_deref(), Some(unknown.payment_id.as_str()));
    // A different amount than the player accepted grants nothing.
    let wrong = VerifiedPayment {
        amount_minor: 600 * KOPECKS,
        ..unknown.clone()
    };
    assert_eq!(
        settle_payment(&app, &wrong).await.unwrap_err().1,
        "payment_invalid"
    );
    assert!(!supporter::status(&app, &profile).await.unwrap().active);
    // A canceled payment closes its attempt and grants nothing.
    let canceled = VerifiedPayment {
        status: PaymentStatus::Canceled,
        paid_at: None,
        ..unknown.clone()
    };
    assert_eq!(
        settle_payment(&app, &canceled).await.unwrap(),
        Settlement::Changed
    );
    assert_eq!(status_of(&app, &id).await, "canceled");
    assert!(!supporter::status(&app, &profile).await.unwrap().active);
    cleanup(&app, &profile).await;
}

/// Other backends of this database waiting on a lock.
async fn lock_waiters(app: &App) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid() AND wait_event_type='Lock'")
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires isolated OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn settlements_racing_a_lost_create_binding_never_settle_as_unknown() {
    let app = app().await;
    let profile = account(&app).await;
    // The create response was lost: the attempt has no payment id yet.
    let id = attempt(&app, &profile, 1000, None).await;
    sqlx::query(
        "UPDATE portal.supporter_yookassa_attempts SET status='charging' WHERE attempt_id=$1",
    )
    .bind(&id)
    .execute(&app.pool)
    .await
    .unwrap();
    let payment = VerifiedPayment {
        attempt_id: Some(id.clone()),
        ..paid(&format!("pay-{}", crypto::random::<8>()), 1000, now() - 30)
    };
    // The first settlement binds the payment and grants, not yet committed.
    let mut first = app.pool.begin().await.unwrap();
    assert_eq!(
        yookassa::settle_payment_tx(&mut first, &payment)
            .await
            .unwrap(),
        Settlement::Changed
    );
    // A full refund and a duplicate success arrive meanwhile: both miss the
    // uncommitted binding and wait on the attempt row.
    let full = refund(
        &format!("ref-{}", crypto::random::<8>()),
        &payment.payment_id,
        1000,
    );
    let refunding = tokio::spawn({
        let (app, payment, full) = (app.clone(), payment.clone(), full.clone());
        async move { settle_refund(&app, &payment, &full).await }
    });
    let duplicate = tokio::spawn({
        let (app, payment) = (app.clone(), payment.clone());
        async move { settle_payment(&app, &payment).await }
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while lock_waiters(&app).await < 2 {
        assert!(
            std::time::Instant::now() < deadline,
            "the concurrent settlements never waited on the attempt"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    first.commit().await.unwrap();
    // Neither is Unknown: the refund counts once and revokes, the duplicate is one.
    assert_eq!(
        refunding.await.unwrap().unwrap(),
        Settlement::Changed,
        "the refund of a payment of ours was dropped"
    );
    assert_eq!(duplicate.await.unwrap().unwrap(), Settlement::Unchanged);
    let refunds: i64 =
        sqlx::query_scalar("SELECT count(*) FROM portal.supporter_yookassa_refunds WHERE refund_id=$1 AND provider_payment_id=$2")
            .bind(&full.refund_id)
            .bind(&payment.payment_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(refunds, 1);
    let (amount, points, revoked) = grant(&app, &payment.payment_id).await;
    assert_eq!((amount, points), (1000 * KOPECKS, 0));
    assert!(revoked.is_some());
    assert_eq!(status_of(&app, &id).await, "succeeded");
    assert!(!supporter::status(&app, &profile).await.unwrap().active);
    // Another payment carrying the same attempt id is not this attempt's: it
    // writes nothing and never rebinds the attempt.
    let other = VerifiedPayment {
        payment_id: format!("pay-{}", crypto::random::<8>()),
        ..payment.clone()
    };
    assert_eq!(
        settle_payment(&app, &other).await.unwrap(),
        Settlement::Unknown
    );
    let bound: Option<String> = sqlx::query_scalar(
        "SELECT provider_payment_id FROM portal.supporter_yookassa_attempts WHERE attempt_id=$1",
    )
    .bind(&id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(bound.as_deref(), Some(payment.payment_id.as_str()));
    let events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM portal.supporter_events WHERE period_id=$1")
            .bind(&other.payment_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(events, 0);
    cleanup(&app, &profile).await;
}

#[tokio::test]
#[ignore = "requires isolated OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn the_tier_sums_this_windows_donations_and_months_count_calendar_months() {
    let app = app().await;
    let profile = account(&app).await;
    // A Solana period from before first (0 points): base styles stay available.
    sqlx::query("INSERT INTO portal.supporter_grants(provider,period_id,profile_id,valid_from,valid_until,event_version) VALUES('solana',$1,$2,$3,$4,1)")
        .bind(crypto::random::<16>()).bind(&profile).bind(now() - 100).bind(now() + 86_400)
        .execute(&app.pool).await.unwrap();
    // The recognition toggle wrote the placeholder before any donation.
    recognition::set(&app, &profile, br#"{"visible":false}"#)
        .await
        .unwrap();
    // Two donations this UTC month (covering now), one long expired in an earlier month.
    let month_start: i64 =
        sqlx::query_scalar("SELECT EXTRACT(EPOCH FROM date_trunc('month', now() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC')::bigint")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    let current = now();
    for (rubles, at) in [
        (1500, (current - 120).max(month_start)),
        (1500, (current - 60).max(month_start)),
        (300, month_start - 90 * 86_400),
    ] {
        let payment = format!("pay-{}", crypto::random::<8>());
        attempt(&app, &profile, rubles, Some(&payment)).await;
        settle_payment(&app, &paid(&payment, rubles, at))
            .await
            .unwrap();
    }
    let points: i64 = sqlx::query_scalar("SELECT portal.supporter_points($1,$2)")
        .bind(&profile)
        .bind(now())
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(points, 3000);
    let status = supporter::status(&app, &profile).await.unwrap();
    assert_eq!(status.tier(), Some(SupporterTier::PrismI));
    // The Solana grant was first, so the placeholder base style stays chosen.
    assert_eq!(status.equipped_aura, Some(AuraStyle::Solar));
    // 1 Solana period + 2 UTC calendar months with donations.
    assert_eq!(
        recognition::get(&app, &profile).await.unwrap()["months_supported"],
        3
    );
    // Points are a function of the moment: nothing covers a year from now.
    let later: i64 = sqlx::query_scalar("SELECT portal.supporter_points($1,$2)")
        .bind(&profile)
        .bind(now() + 365 * 86_400)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(later, 0);
    cleanup(&app, &profile).await;
}

#[tokio::test]
#[ignore = "requires isolated OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn a_first_donation_upgrades_the_placeholder_aura_but_keeps_a_hidden_one() {
    let app = app().await;
    for (hidden, expected) in [(false, Some(AuraStyle::Azure)), (true, None)] {
        let profile = account(&app).await;
        recognition::set(&app, &profile, br#"{"visible":true}"#)
            .await
            .unwrap();
        if hidden {
            supporter::equip(&app, &profile, br#"{"aura":null}"#)
                .await
                .unwrap();
        }
        let payment = format!("pay-{}", crypto::random::<8>());
        attempt(&app, &profile, 500, Some(&payment)).await;
        settle_payment(&app, &paid(&payment, 500, now() - 5))
            .await
            .unwrap();
        let status = supporter::status(&app, &profile).await.unwrap();
        assert_eq!(status.equipped_aura, expected, "hidden={hidden}");
        cleanup(&app, &profile).await;
    }
}

#[tokio::test]
#[ignore = "requires isolated OMOBA_PORTAL_TEST_DATABASE_URL and OMOBA_PORTAL_ROLE_TEST_URL"]
async fn the_restricted_portal_role_settles_and_the_game_role_reads_points() {
    let owner = app().await;
    let runtime = App::connect(
        &std::env::var("OMOBA_PORTAL_ROLE_TEST_URL").expect("isolated portal runtime role"),
        config(),
    )
    .await
    .unwrap();
    let profile = account(&owner).await;
    let payment = format!("pay-{}", crypto::random::<8>());
    let id = attempt(&owner, &profile, 1000, Some(&payment)).await;
    let succeeded = paid(&payment, 1000, now() - 30);
    settle_refund(
        &runtime,
        &succeeded,
        &refund(&format!("ref-{}", crypto::random::<8>()), &payment, 400),
    )
    .await
    .unwrap();
    assert_eq!(status_of(&owner, &id).await, "succeeded");
    assert_eq!(grant(&owner, &payment).await.1, 600);
    let status = supporter::status(&runtime, &profile).await.unwrap();
    assert_eq!(status.tier(), Some(SupporterTier::Azure));
    supporter::dashboard(&runtime, &profile).await.unwrap();
    recognition::get(&runtime, &profile).await.unwrap();
    // Billing records are audit: the runtime role cannot delete them.
    for query in [
        "DELETE FROM portal.supporter_yookassa_attempts WHERE false",
        "DELETE FROM portal.supporter_yookassa_refunds WHERE false",
        "UPDATE portal.supporter_yookassa_refunds SET amount_minor=1 WHERE false",
        "DELETE FROM portal.supporter_yookassa_refund_scan WHERE false",
    ] {
        let error = sqlx::query(query).execute(&runtime.pool).await.unwrap_err();
        assert_eq!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42501"),
            "{query}"
        );
    }
    // Roles that may write cosmetic preferences but not grants (the game role
    // from ops/grants.sql) can execute the tier sum.
    let game_roles: Vec<(String, bool)> = sqlx::query_as(
        "SELECT r.rolname, has_function_privilege(r.oid,'portal.supporter_points(text,bigint)','EXECUTE')
         FROM pg_roles r WHERE NOT r.rolsuper
           AND has_table_privilege(r.oid,'portal.supporter_preferences','INSERT')
           AND NOT has_table_privilege(r.oid,'portal.supporter_grants','INSERT')",
    )
    .fetch_all(&owner.pool)
    .await
    .unwrap();
    for (role, execute) in &game_roles {
        assert!(execute, "{role} cannot execute portal.supporter_points");
    }
    cleanup(&owner, &profile).await;
}
