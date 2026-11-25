//! Synthetic paid periods in an explicitly isolated loopback PostgreSQL database.
mod common;

use axum::{
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{HeaderMap, Request, StatusCode},
};
use ed25519_dalek::SigningKey;
use omoba_account_api::{
    App, Config, SUPPORTER_RECOGNITION_MIGRATION, accounts, auth, crypto, migrate, now, router,
    supporter::{self, VerifiedEvent, recognition},
};
use serde_json::{Value, json};
use std::net::SocketAddr;
use tower::ServiceExt;

async fn app() -> App {
    let url = std::env::var("OMOBA_PORTAL_TEST_DATABASE_URL")
        .expect("explicit isolated OMOBA_PORTAL_TEST_DATABASE_URL required");
    let options: sqlx::postgres::PgConnectOptions = url.parse().unwrap();
    assert_eq!(options.get_host(), "127.0.0.1");
    assert!(
        matches!(
            (options.get_port(), options.get_database()),
            (55581, Some("hero_tests")) | (5432, Some("omoba_test"))
        ) || common::launcher_database(&options),
        "Only the dedicated disposable Hero or CI test database"
    );
    migrate(&url).await.unwrap();
    migrate(&url).await.unwrap();
    App::connect(
        &url,
        Config {
            origin: "http://127.0.0.1:3034".into(),
            secret: [71; 32],
            trust_loopback_proxy: false,
            release_file: None,
        },
    )
    .await
    .unwrap()
}

async fn account(app: &App, name: &str) -> String {
    let key = SigningKey::from_bytes(&crypto::decode::<32>(&crypto::random::<32>()).unwrap());
    app.career
        .authenticate(&crypto::hex(key.verifying_key().as_bytes()), name)
        .await
        .unwrap()
        .profile_id
}

fn period(id: &str, start: i64, end: i64) -> VerifiedEvent {
    VerifiedEvent {
        event_id: format!("event-{id}"),
        provider: "solana".into(),
        period_id: id.into(),
        original_transaction_id: None,
        app_account_token: None,
        product_id: None,
        environment: None,
        valid_from: start,
        valid_until: end,
        revoked_at: None,
        event_version: 1,
        renewal_enabled: Some(false),
        amount_minor: None,
        refunded_minor: None,
    }
}

async fn privacy(app: &App, id: &str, public: bool, stats: bool) {
    sqlx::query("INSERT INTO portal.profile_settings(profile_id,public_profile_enabled,public_stats_enabled) VALUES($1,$2,$3) ON CONFLICT(profile_id) DO UPDATE SET public_profile_enabled=$2,public_stats_enabled=$3")
        .bind(id).bind(public).bind(stats).execute(&app.pool).await.unwrap();
}

async fn cleanup(app: &App, profiles: &[String]) {
    for table in [
        "portal.supporter_events",
        "portal.supporter_grants",
        "portal.supporter_preferences",
        "portal.profile_settings",
        "portal.web_sessions",
        "career_keys",
        "career_profiles",
    ] {
        sqlx::query(&format!("DELETE FROM {table} WHERE profile_id=ANY($1)"))
            .bind(profiles)
            .execute(&app.pool)
            .await
            .unwrap();
    }
}

async fn http(
    app: &App,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> (StatusCode, HeaderMap, Value) {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let mut request = request
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:42123".parse::<SocketAddr>().unwrap(),
    ));
    let response = router(app.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    (status, headers, serde_json::from_slice(&bytes).unwrap())
}

/// HTTP authorization is real; this fixture creates a synthetic already-consented session.
async fn token(app: &App, profile: &str, authorization_version: i16) -> String {
    let token = crypto::random::<32>();
    sqlx::query("INSERT INTO portal.web_sessions(session_id,profile_id,token_hash,created_at,last_seen,expires_at,authorization_version) VALUES($1,$2,$3,$4,$4,$5,$6)")
        .bind(crypto::random::<32>()).bind(profile)
        .bind(crypto::mac(&app.config.secret, "session-token", &token))
        .bind(now()).bind(now()+3600).bind(authorization_version)
        .execute(&app.pool).await.unwrap();
    token
}

#[tokio::test]
#[ignore = "requires isolated loopback OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn recognition_migration_keeps_existing_and_new_preferences_private() {
    let app = app().await;
    let mut tx = app.pool.begin().await.unwrap();
    let schema = format!("recognition_fixture_{}", crypto::random::<8>());
    sqlx::raw_sql(&format!("CREATE SCHEMA {schema}; CREATE TABLE {schema}.schema_version(version integer PRIMARY KEY); CREATE TABLE {schema}.supporter_preferences(profile_id text PRIMARY KEY,aura text); INSERT INTO {schema}.supporter_preferences VALUES('existing','lunar');"))
        .execute(&mut *tx).await.unwrap();
    sqlx::raw_sql(&SUPPORTER_RECOGNITION_MIGRATION.replace("portal.", &format!("{schema}.")))
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(&format!(
        "INSERT INTO {schema}.supporter_preferences(profile_id) VALUES('new')"
    ))
    .execute(&mut *tx)
    .await
    .unwrap();
    let visibility: Vec<bool> = sqlx::query_scalar(&format!(
        "SELECT recognition_visible FROM {schema}.supporter_preferences ORDER BY profile_id"
    ))
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    assert_eq!(visibility, [false, false]);
    let versions: Vec<i32> =
        sqlx::query_scalar("SELECT version FROM portal.schema_version ORDER BY version")
            .fetch_all(&mut *tx)
            .await
            .unwrap();
    assert_eq!(versions, omoba_account_api::PORTAL_SCHEMA);
    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated loopback OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn lifetime_months_include_expired_future_and_more_than_status_history_without_replay_or_refund_credit()
 {
    let app = app().await;
    let profile = account(&app, "HistoryHero").await;
    let initial_profile = app.career.profile(&profile).await.unwrap();
    assert_eq!(
        recognition::get(&app, &profile).await.unwrap(),
        json!({"months_supported":0,"visible":false,"public_profile_enabled":false})
    );
    // The fixture represents 105 separate already-paid past months, beyond both
    // bounded status response limits. No grants enter via a client assertion.
    sqlx::query("INSERT INTO portal.supporter_grants(provider,period_id,profile_id,valid_from,valid_until,event_version) SELECT 'solana',$1||'-past-'||n::text,$1,$2-((n+1)*2592000),$2-(n*2592000),1 FROM generate_series(1,105) n")
        .bind(&profile).bind(now()).execute(&app.pool).await.unwrap();
    let current = now();
    let mut live = period(&format!("{profile}-live"), current - 10, current + 600);
    assert!(supporter::apply_event(&app, &profile, &live).await.unwrap());
    let future = period(
        &format!("{profile}-future"),
        current + 600,
        current + 2592600,
    );
    supporter::apply_event(&app, &profile, &future)
        .await
        .unwrap();
    assert_eq!(
        supporter::status(&app, &profile)
            .await
            .unwrap()
            .grants
            .len(),
        shared::supporter::STATUS_GRANT_LIMIT as usize
    );
    let dashboard = supporter::dashboard(&app, &profile).await.unwrap();
    assert_eq!(dashboard["recognition"]["months_supported"], 107);
    assert_eq!(dashboard["supporter"]["active"], true);
    assert!(!supporter::apply_event(&app, &profile, &live).await.unwrap());
    // A new provider event for the same payment period changes cancellation state,
    // not the number of purchased months or the already-paid access.
    live.event_id.push_str("-cancel");
    live.event_version = 2;
    live.renewal_enabled = Some(false);
    supporter::apply_event(&app, &profile, &live).await.unwrap();
    assert_eq!(
        recognition::get(&app, &profile).await.unwrap()["months_supported"],
        107
    );
    assert!(supporter::status(&app, &profile).await.unwrap().active);
    let mut refunded = period(&format!("{profile}-refunded"), current - 10, current + 600);
    supporter::apply_event(&app, &profile, &refunded)
        .await
        .unwrap();
    assert_eq!(
        recognition::get(&app, &profile).await.unwrap()["months_supported"],
        108
    );
    refunded.event_id.push_str("-refund");
    refunded.event_version = 2;
    refunded.revoked_at = Some(current);
    supporter::apply_event(&app, &profile, &refunded)
        .await
        .unwrap();
    assert_eq!(
        recognition::get(&app, &profile).await.unwrap()["months_supported"],
        107
    );
    live.event_id.push_str("-refund");
    live.event_version = 3;
    live.revoked_at = Some(current);
    supporter::apply_event(&app, &profile, &live).await.unwrap();
    assert_eq!(
        recognition::get(&app, &profile).await.unwrap()["months_supported"],
        106
    );
    assert!(!supporter::status(&app, &profile).await.unwrap().active);
    live.event_id.push_str("-late");
    live.event_version = 4;
    live.revoked_at = None;
    supporter::apply_event(&app, &profile, &live).await.unwrap();
    assert_eq!(
        recognition::get(&app, &profile).await.unwrap()["months_supported"],
        106
    );
    assert!(!supporter::status(&app, &profile).await.unwrap().active);
    assert_eq!(app.career.profile(&profile).await.unwrap(), initial_profile);
    cleanup(&app, &[profile]).await;
}

#[tokio::test]
#[ignore = "requires isolated loopback OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn public_recognition_respects_consent_privacy_stats_auth_and_stable_pagination() {
    let app = app().await;
    let mut profiles = Vec::new();
    for name in [
        "ActiveHero",
        "ExpiredHero",
        "FutureHero",
        "RefundedHero",
        "HiddenHero",
        "PrivateHero",
        "NoPaidHero",
    ] {
        profiles.push(account(&app, name).await);
    }
    let current = now();
    for (i, profile) in profiles.iter().enumerate() {
        privacy(&app, profile, i != 5, i == 1).await;
        if i != 6 {
            let (from, until) = match i {
                1 => (current - 600, current - 1),
                2 => (current + 600, current + 1200),
                _ => (current - 10, current + 600),
            };
            let mut event = period(&format!("{profile}-month"), from, until);
            if i == 3 {
                event.revoked_at = Some(current);
            }
            supporter::apply_event(&app, profile, &event).await.unwrap();
        }
        if i != 4 {
            recognition::set(&app, profile, br#"{"visible":true}"#)
                .await
                .unwrap();
        }
    }
    let owner = token(&app, &profiles[0], 2).await;
    let legacy = token(&app, &profiles[0], 1).await;
    for auth in [None, Some(legacy.as_str())] {
        let (status, _, _) = http(
            &app,
            "PATCH",
            "/v1/me/supporter/recognition",
            auth,
            json!({"visible":false}),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(
        recognition::get(&app, &profiles[0]).await.unwrap()["visible"],
        true
    );
    let (status, headers, first) =
        http(&app, "GET", "/v1/supporters?limit=2", None, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["cache-control"], "private, no-store");
    assert!(first["data_as_of"].is_string());
    assert_eq!(first["supporters"].as_array().unwrap().len(), 2);
    let cursor = first["next_cursor"].as_str().unwrap();
    let (status, _, second) = http(
        &app,
        "GET",
        &format!("/v1/supporters?limit=2&cursor={cursor}"),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(second["supporters"].as_array().unwrap().len(), 1);
    assert!(second["next_cursor"].is_null());
    let supporters: Vec<_> = first["supporters"]
        .as_array()
        .unwrap()
        .iter()
        .chain(second["supporters"].as_array().unwrap())
        .collect();
    let found: Vec<_> = supporters
        .iter()
        .map(|p| p["profile_id"].as_str().unwrap())
        .collect();
    let mut expected: Vec<_> = profiles[..3].iter().map(String::as_str).collect();
    expected.sort();
    assert_eq!(found, expected);
    for supporter in supporters {
        let mut keys: Vec<_> = supporter
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "hero_active",
                "hero_until",
                "months_supported",
                "nickname",
                "profile_id"
            ]
        );
        assert_eq!(supporter["months_supported"], 1);
        let active = supporter["profile_id"] == profiles[0];
        assert_eq!(supporter["hero_active"], active);
        assert_eq!(supporter["hero_until"].is_null(), !active);
    }
    for (i, id) in profiles.iter().enumerate() {
        let (status, _, response) =
            http(&app, "GET", &format!("/v1/players/{id}"), None, Value::Null).await;
        if i == 5 {
            assert_eq!(status, StatusCode::NOT_FOUND);
            continue;
        }
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["profile"].get("supporter").is_some(), i < 3);
        assert_eq!(response["profile"].get("rating").is_some(), i == 1);
        assert_eq!(response["profile"].get("wins").is_some(), i == 1);
    }
    for path in [
        "/v1/supporters?limit=0",
        "/v1/supporters?limit=51",
        "/v1/supporters?cursor=bad",
        "/v1/supporters?sort=months",
        "/v1/supporters?limit=1&limit=2",
    ] {
        assert_eq!(
            http(&app, "GET", path, None, Value::Null).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    for body in [
        json!({}),
        json!({"visible":"true"}),
        json!({"visible":true,"months_supported":99}),
        json!({"visible":true,"profile_id":profiles[4]}),
    ] {
        assert_eq!(
            http(
                &app,
                "PATCH",
                "/v1/me/supporter/recognition",
                Some(&owner),
                body
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    for _ in 0..2 {
        let (status, _, value) = http(
            &app,
            "PATCH",
            "/v1/me/supporter/recognition",
            Some(&owner),
            json!({"visible":false}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            value,
            json!({"visible":false,"public_profile_enabled":true,"months_supported":1})
        );
    }
    assert!(supporter::status(&app, &profiles[0]).await.unwrap().active);
    let (status, _, response) = http(
        &app,
        "GET",
        &format!("/v1/players/{}", profiles[0]),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(response["profile"].get("supporter").is_none());
    assert_eq!(
        recognition::directory(&app, None).await.unwrap()["supporters"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    http(
        &app,
        "PATCH",
        "/v1/me/supporter/recognition",
        Some(&owner),
        json!({"visible":true}),
    )
    .await;
    privacy(&app, &profiles[0], false, false).await;
    assert_eq!(
        recognition::get(&app, &profiles[0]).await.unwrap(),
        json!({"visible":true,"public_profile_enabled":false,"months_supported":1})
    );
    assert_eq!(
        recognition::directory(&app, None).await.unwrap()["supporters"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let own = auth::Session {
        session_id: String::new(),
        profile_id: profiles[0].clone(),
        created_at: now(),
    };
    let private = accounts::player(&app, Some(&own), &profiles[0])
        .await
        .unwrap();
    assert!(private["profile"].get("rating").is_some());
    assert!(private["profile"].get("supporter").is_none());
    let aura: Option<String> =
        sqlx::query_scalar("SELECT aura FROM portal.supporter_preferences WHERE profile_id=$1")
            .bind(&profiles[0])
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(aura.as_deref(), Some("solar"));
    cleanup(&app, &profiles).await;
}
