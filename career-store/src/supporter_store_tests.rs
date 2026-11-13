//! Run against an isolated database after Account API migrations.
use super::*;
use shared::supporter::AuraStyle;

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL database with Account API migrations"]
async fn supporter_runtime_checks_grants_expiry_revocation_and_never_changes_progress() {
    let url = std::env::var("OMOBA_TEST_DATABASE_URL").expect("isolated database URL required");
    let store = CareerStore::connect(&url).await.unwrap();
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).unwrap();
    let key: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let profile = store.authenticate(&key, "AuraTester").await.unwrap();
    assert!(
        !store
            .supporter_status(&profile.profile_id)
            .await
            .unwrap()
            .active
    );
    assert!(
        store
            .equip_supporter_aura(&profile.profile_id, Some(AuraStyle::Solar))
            .await
            .is_err()
    );
    let now: i64 = sqlx::query_scalar("SELECT EXTRACT(EPOCH FROM now())::bigint")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    let grant = |period: String, from: i64, until: i64| {
        sqlx::query("INSERT INTO portal.supporter_grants(provider,period_id,profile_id,valid_from,valid_until,event_version) VALUES('solana',$1,$2,$3,$4,1)").bind(period).bind(profile.profile_id.clone()).bind(from).bind(until)
    };
    let period = format!("test-{key}");
    grant(period.clone(), now - 100, now - 1)
        .execute(&store.pool)
        .await
        .unwrap();
    assert!(
        store
            .equip_supporter_aura(&profile.profile_id, Some(AuraStyle::Lunar))
            .await
            .is_err()
    );
    sqlx::query("UPDATE portal.supporter_grants SET valid_until=$1 WHERE period_id=$2")
        .bind(now + 3600)
        .bind(&period)
        .execute(&store.pool)
        .await
        .unwrap();
    // Future/revoked receipts must never crowd an active grant out of a
    // bounded status payload. The live grant expires before all these rows.
    sqlx::query("INSERT INTO portal.supporter_grants(provider,period_id,profile_id,valid_from,valid_until,event_version) SELECT 'solana',$1 || '-' || n::text,$2,$3,$4+n,1 FROM generate_series(1,101) AS n")
        .bind(format!("future-{key}")).bind(&profile.profile_id).bind(now+7200).bind(now+10800).execute(&store.pool).await.unwrap();
    let active = store
        .equip_supporter_aura(&profile.profile_id, Some(AuraStyle::Verdant))
        .await
        .unwrap();
    assert!(active.active);
    assert_eq!(active.equipped_aura, Some(AuraStyle::Verdant));
    assert_eq!(active.active_until, Some(now + 3600));
    let hidden = store
        .equip_supporter_aura(&profile.profile_id, None)
        .await
        .unwrap();
    assert!(hidden.active);
    assert_eq!(hidden.equipped_aura, None);
    sqlx::query("UPDATE portal.supporter_grants SET revoked_at=$1 WHERE period_id=$2")
        .bind(now)
        .bind(&period)
        .execute(&store.pool)
        .await
        .unwrap();
    assert!(
        store
            .equip_supporter_aura(&profile.profile_id, Some(AuraStyle::Solar))
            .await
            .is_err()
    );
    assert!(
        !store
            .supporter_status(&profile.profile_id)
            .await
            .unwrap()
            .active
    );
    assert_eq!(store.profile(&profile.profile_id).await.unwrap(), profile);
    // Leave no fixture rows; this database belongs only to the current task.
    sqlx::query("DELETE FROM portal.supporter_preferences WHERE profile_id=$1")
        .bind(&profile.profile_id)
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM portal.supporter_grants WHERE profile_id=$1")
        .bind(&profile.profile_id)
        .execute(&store.pool)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL database with Account API migrations"]
async fn tier_styles_need_this_months_points_and_step_down_with_expiry() {
    let url = std::env::var("OMOBA_TEST_DATABASE_URL").expect("isolated database URL required");
    let store = CareerStore::connect(&url).await.unwrap();
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).unwrap();
    let key: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let profile = store.authenticate(&key, "TierTester").await.unwrap();
    let id = profile.profile_id.clone();
    let now: i64 = sqlx::query_scalar("SELECT EXTRACT(EPOCH FROM now())::bigint")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    // Synthetic YooKassa grants: 900 ₽ ending in an hour, 300 ₽ ending in a day.
    let grant = |period: &str, until: i64, rubles: i64| {
        sqlx::query("INSERT INTO portal.supporter_grants(provider,period_id,profile_id,valid_from,valid_until,event_version,amount_minor,tier_points) VALUES('yookassa',$1,$2,$3,$4,0,$5,$6)")
            .bind(format!("{period}-{key}")).bind(id.clone()).bind(until - 30 * 86400).bind(until)
            .bind(rubles * 100).bind(rubles as i32)
    };
    grant("a", now + 3600, 900)
        .execute(&store.pool)
        .await
        .unwrap();
    grant("b", now + 86400, 300)
        .execute(&store.pool)
        .await
        .unwrap();
    // No preference row yet: the highest unlocked style is shown.
    let status = store.supporter_status(&id).await.unwrap();
    assert!(status.active);
    assert_eq!(status.tier_points, 1200);
    assert_eq!(
        status.tier(),
        Some(shared::supporter::SupporterTier::Crimson)
    );
    assert_eq!(status.tier_until, Some(now + 3600));
    assert_eq!(
        status.next_tier(),
        Some(shared::supporter::SupporterTier::Jade)
    );
    assert_eq!(status.equipped_aura, Some(AuraStyle::Crimson));
    assert_eq!(
        status.grants[0].tier_points + status.grants[1].tier_points,
        1200
    );
    let refused = store
        .equip_supporter_aura(&id, Some(AuraStyle::PrismI))
        .await
        .unwrap_err();
    assert!(refused.contains("higher Hero tier"), "{refused}");
    for style in [AuraStyle::Lunar, AuraStyle::Rose, AuraStyle::Crimson] {
        let equipped = store.equip_supporter_aura(&id, Some(style)).await.unwrap();
        assert_eq!(equipped.equipped_aura, Some(style));
    }
    // The 900 ₽ grant expires: 300 points (Jade) remain and Crimson steps down.
    sqlx::query("UPDATE portal.supporter_grants SET valid_until=$1 WHERE period_id=$2")
        .bind(now - 1)
        .bind(format!("a-{key}"))
        .execute(&store.pool)
        .await
        .unwrap();
    let stepped = store.supporter_status(&id).await.unwrap();
    assert_eq!(stepped.tier_points, 300);
    assert_eq!(stepped.equipped_aura, Some(AuraStyle::Jade));
    assert!(
        store
            .equip_supporter_aura(&id, Some(AuraStyle::Crimson))
            .await
            .is_err()
    );
    // A partial refund recomputes the grant's points below the first tier.
    sqlx::query("UPDATE portal.supporter_grants SET tier_points=50 WHERE period_id=$1")
        .bind(format!("b-{key}"))
        .execute(&store.pool)
        .await
        .unwrap();
    let base = store.supporter_status(&id).await.unwrap();
    assert!(base.active && base.tier().is_none() && base.tier_until.is_none());
    assert_eq!(base.equipped_aura, Some(AuraStyle::Solar));
    assert!(
        store
            .equip_supporter_aura(&id, Some(AuraStyle::Verdant))
            .await
            .is_ok()
    );
    assert_eq!(store.profile(&id).await.unwrap(), profile);
    for table in ["portal.supporter_preferences", "portal.supporter_grants"] {
        sqlx::query(&format!("DELETE FROM {table} WHERE profile_id=$1"))
            .bind(&id)
            .execute(&store.pool)
            .await
            .unwrap();
    }
}

// Independent previous query plan: one read-only transaction, separate grant
// and preference reads. It is the semantic oracle for the single-statement read.
async fn legacy_supporter_snapshot(
    store: &CareerStore,
    id: &str,
) -> shared::supporter::SupporterStatus {
    use shared::supporter::{
        STATUS_GRANT_LIMIT, SupporterGrantSummary, SupporterStatus, tier_points,
    };
    let mut tx = store.pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await
        .unwrap();
    let (now,points):(i64,i64)=sqlx::query_as("SELECT EXTRACT(EPOCH FROM now())::bigint,portal.supporter_points($1,EXTRACT(EPOCH FROM now())::bigint)").bind(id).fetch_one(&mut *tx).await.unwrap();
    let rows=sqlx::query("SELECT provider,valid_from,valid_until,revoked_at,renewal_enabled,tier_points FROM portal.supporter_grants WHERE profile_id=$1 ORDER BY (revoked_at IS NULL AND valid_from<=$2 AND valid_until>$2) DESC,valid_until DESC LIMIT $3").bind(id).bind(now).bind(STATUS_GRANT_LIMIT).fetch_all(&mut *tx).await.unwrap();
    let grants = rows
        .iter()
        .map(|r| SupporterGrantSummary {
            provider: r.get("provider"),
            valid_from: r.get("valid_from"),
            valid_until: r.get("valid_until"),
            revoked: r.get::<Option<i64>, _>("revoked_at").is_some(),
            renewal_enabled: r.get("renewal_enabled"),
            tier_points: tier_points(i64::from(r.get::<i32, _>("tier_points"))),
        })
        .collect();
    let preference: Option<Option<String>> =
        sqlx::query_scalar("SELECT aura FROM portal.supporter_preferences WHERE profile_id=$1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .unwrap();
    tx.commit().await.unwrap();
    SupporterStatus::from_grants(
        grants,
        now,
        tier_points(points),
        preference.map(|v| v.as_deref().and_then(AuraStyle::from_id)),
    )
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL and Account API migrations; compares supporter snapshots and read cost"]
async fn supporter_single_statement_matches_bounded_legacy_snapshots_and_preferences() {
    use shared::supporter::STATUS_GRANT_LIMIT;
    let url = std::env::var("OMOBA_TEST_DATABASE_URL").expect("isolated database URL");
    let store = CareerStore::connect(&url).await.unwrap();
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).unwrap();
    let key: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let profile = store.authenticate(&key, "SnapshotTester").await.unwrap();
    let id = &profile.profile_id;
    assert_eq!(
        store.supporter_status(id).await.unwrap(),
        legacy_supporter_snapshot(&store, id).await
    );
    let start = std::time::Instant::now();
    for _ in 0..20 {
        let _ = legacy_supporter_snapshot(&store, id).await;
    }
    let legacy_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = std::time::Instant::now();
    for _ in 0..20 {
        let _ = store.supporter_status(id).await.unwrap();
    }
    println!(
        "SUPPORTER_READ_BENCH {}",
        serde_json::json!({"shape":"empty","reads":20,"legacy_ms":legacy_ms,"single_statement_ms":start.elapsed().as_secs_f64()*1000.0,"scope":"local query/read cost only; not1000-client acceptance"})
    );
    let now: i64 = sqlx::query_scalar("SELECT EXTRACT(EPOCH FROM now())::bigint")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO portal.supporter_grants(provider,period_id,profile_id,valid_from,valid_until,event_version,amount_minor,tier_points) SELECT 'yookassa',$1||'-'||n::text,$2,$3,$4+n,0,1000,10 FROM generate_series(1,$5::integer) n").bind(&key).bind(id).bind(now-100).bind(now+3600).bind((STATUS_GRANT_LIMIT+7) as i32).execute(&store.pool).await.unwrap();
    sqlx::query("INSERT INTO portal.supporter_grants(provider,period_id,profile_id,valid_from,valid_until,event_version,amount_minor,tier_points) VALUES ('yookassa',$1,$2,$3,$4,0,100000,1000)").bind(format!("future-{key}")).bind(id).bind(now+7200).bind(now+10800).execute(&store.pool).await.unwrap();
    sqlx::query("INSERT INTO portal.supporter_grants(provider,period_id,profile_id,valid_from,valid_until,revoked_at,event_version) VALUES ('solana',$1,$2,$3,$4,$3,0)").bind(format!("revoked-{key}")).bind(id).bind(now-100).bind(now+20000).execute(&store.pool).await.unwrap();
    let automatic = store.supporter_status(id).await.unwrap();
    assert_eq!(automatic, legacy_supporter_snapshot(&store, id).await);
    assert_eq!(automatic.grants.len(), STATUS_GRANT_LIMIT as usize);
    assert_eq!(
        automatic.tier_points,
        ((STATUS_GRANT_LIMIT + 7) * 10) as u32
    );
    assert!(automatic.grants.iter().all(|g| g.covers(now)));
    assert!(
        automatic
            .grants
            .windows(2)
            .all(|w| w[0].valid_until >= w[1].valid_until)
    );
    for aura in [None, Some("verdant"), Some("prism_i")] {
        sqlx::query("INSERT INTO portal.supporter_preferences(profile_id,aura) VALUES($1,$2) ON CONFLICT(profile_id) DO UPDATE SET aura=EXCLUDED.aura").bind(id).bind(aura).execute(&store.pool).await.unwrap();
        let status = store.supporter_status(id).await.unwrap();
        assert_eq!(status, legacy_supporter_snapshot(&store, id).await);
        if aura.is_none() {
            assert_eq!(status.equipped_aura, None);
        }
    }
    let start = std::time::Instant::now();
    for _ in 0..20 {
        let _ = legacy_supporter_snapshot(&store, id).await;
    }
    let legacy_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = std::time::Instant::now();
    for _ in 0..20 {
        let _ = store.supporter_status(id).await.unwrap();
    }
    println!(
        "SUPPORTER_READ_BENCH {}",
        serde_json::json!({"shape":"bounded_active","reads":20,"legacy_ms":legacy_ms,"single_statement_ms":start.elapsed().as_secs_f64()*1000.0,"scope":"local query/read cost only; not1000-client acceptance"})
    );
    assert_eq!(store.profile(id).await.unwrap(), profile);
    for table in ["portal.supporter_preferences", "portal.supporter_grants"] {
        sqlx::query(&format!("DELETE FROM {table} WHERE profile_id=$1"))
            .bind(id)
            .execute(&store.pool)
            .await
            .unwrap();
    }
}
