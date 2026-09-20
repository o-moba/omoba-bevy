//! Run only against an isolated test database. No production credentials or receipts.
use axum::http::StatusCode;
use ed25519_dalek::{Signer, SigningKey};
use omoba_account_api::{auth::Session, crypto, devices, *};
use serde_json::{Value, json};
use shared::device_account::*;

fn random_key() -> SigningKey {
    let mut seed = [0; 32];
    getrandom::fill(&mut seed).unwrap();
    SigningKey::from_bytes(&seed)
}
fn public(key: &SigningKey) -> String {
    crypto::hex(key.verifying_key().as_bytes())
}
fn enrollment(key: &SigningKey) -> DeviceEnrollment {
    DeviceEnrollment {
        enrollment_id: crypto::random::<32>(),
        public_key: public(key),
        origin: "https://players.test".into(),
        label: "Test phone".into(),
        expires_at: (now() + 299).to_string(),
    }
}
fn signed(
    key: &SigningKey,
    e: &DeviceEnrollment,
    action: DeviceAction,
    target: Option<&str>,
    code: Option<&str>,
) -> Vec<u8> {
    serde_json::to_vec(&SignedDeviceEnrollment {
        enrollment: e.clone(),
        action,
        target_profile: target.map(str::to_owned),
        recovery_code: code.map(str::to_owned),
        signature: crypto::hex(&key.sign(&e.signing_bytes(action, target, code)).to_bytes()),
    })
    .unwrap()
}
async fn app() -> App {
    let url =
        std::env::var("OMOBA_PORTAL_TEST_DATABASE_URL").expect("isolated test database required");
    migrate(&url).await.unwrap();
    App::connect(
        &url,
        Config {
            origin: "https://players.test".into(),
            secret: [17; 32],
            trust_loopback_proxy: false,
            release_file: None,
        },
    )
    .await
    .unwrap()
}
async fn session(app: &App, id: &str) -> Session {
    let sid = crypto::random::<32>();
    sqlx::query("INSERT INTO portal.web_sessions(session_id,profile_id,token_hash,created_at,last_seen,expires_at,authorization_version) VALUES($1,$2,$3,$4,$4,$5,2)")
        .bind(&sid).bind(id).bind(crypto::random::<32>()).bind(now()).bind(now()+3600).execute(&app.pool).await.unwrap();
    Session {
        session_id: sid,
        profile_id: id.into(),
        created_at: now(),
    }
}
async fn start(app: &App, key: &SigningKey) -> (DeviceEnrollment, Value) {
    let e = enrollment(key);
    let result = devices::create(app, &signed(key, &e, DeviceAction::Create, None, None))
        .await
        .unwrap();
    (e, result)
}
async fn approve(app: &App, s: &Session, e: &DeviceEnrollment, code: &str) {
    let preview = devices::lookup(app, s, &serde_json::to_vec(&json!({"code":code})).unwrap())
        .await
        .unwrap();
    assert_eq!(preview["public_key"], e.public_key);
    assert_eq!(preview["profile_id"], s.profile_id);
    devices::approve(
        app,
        s,
        &serde_json::to_vec(&json!({"enrollment_id":e.enrollment_id,"public_key":e.public_key}))
            .unwrap(),
    )
    .await
    .unwrap();
}
#[tokio::test]
#[ignore = "requires isolated owner OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn device_enrollment_proves_new_key_preserves_profile_and_rejects_reassignment() {
    let app = app().await;
    let old = random_key();
    let profile = app
        .career
        .authenticate(&public(&old), "Device owner")
        .await
        .unwrap();
    let s = session(&app, &profile.profile_id).await;
    let new = random_key();
    let (e, started) = start(&app, &new).await;
    assert!(
        !app.career
            .key_is_active(&public(&new), &profile.profile_id)
            .await
            .unwrap()
    );
    approve(&app, &s, &e, started["code"].as_str().unwrap()).await;
    // The enrollment proof cannot be replayed as another operation or target.
    assert_eq!(
        devices::complete(&app, &signed(&new, &e, DeviceAction::Status, None, None))
            .await
            .unwrap_err()
            .0,
        StatusCode::BAD_REQUEST
    );
    assert!(
        devices::complete(
            &app,
            &signed(
                &old,
                &e,
                DeviceAction::Complete,
                Some(&profile.profile_id),
                None
            )
        )
        .await
        .is_err()
    );
    let other = app
        .career
        .authenticate(&public(&random_key()), "Other owner")
        .await
        .unwrap();
    assert!(
        devices::complete(
            &app,
            &signed(
                &new,
                &e,
                DeviceAction::Complete,
                Some(&other.profile_id),
                None
            )
        )
        .await
        .is_err()
    );
    let complete = signed(
        &new,
        &e,
        DeviceAction::Complete,
        Some(&profile.profile_id),
        None,
    );
    assert_eq!(
        devices::complete(&app, &complete).await.unwrap()["state"],
        "consumed"
    );
    assert_eq!(
        devices::complete(&app, &complete).await.unwrap()["state"],
        "consumed"
    );
    assert_eq!(
        app.career
            .authenticate(&public(&new), "Ignored new name")
            .await
            .unwrap(),
        profile
    );
    assert!(
        app.career
            .key_is_active(&public(&old), &profile.profile_id)
            .await
            .unwrap()
    );
    // Crash after registration but before local activation: proving the persisted
    // candidate again recovers its existing account, never creates a new one.
    let recovered = devices::create(
        &app,
        &signed(&new, &enrollment(&new), DeviceAction::Create, None, None),
    )
    .await
    .unwrap();
    assert_eq!(recovered["state"], "consumed");
    assert_eq!(recovered["profile_id"], profile.profile_id);
    // A candidate registered in the game while its portal approval was pending
    // is never silently moved away from that progressed account.
    let raced = random_key();
    let (race, started) = start(&app, &raced).await;
    approve(&app, &s, &race, started["code"].as_str().unwrap()).await;
    let separate = app
        .career
        .authenticate(&public(&raced), "Separate progress")
        .await
        .unwrap();
    assert_ne!(separate.profile_id, profile.profile_id);
    assert_eq!(
        devices::complete(
            &app,
            &signed(
                &raced,
                &race,
                DeviceAction::Complete,
                Some(&profile.profile_id),
                None
            )
        )
        .await
        .unwrap_err()
        .1,
        "device_key_already_registered"
    );
    assert!(
        app.career
            .key_is_active(&public(&raced), &separate.profile_id)
            .await
            .unwrap()
    );
}
#[tokio::test]
#[ignore = "requires isolated owner OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn recovery_codes_are_hashed_one_use_and_revoked_keys_stay_revoked() {
    let app = app().await;
    let old = random_key();
    let profile = app
        .career
        .authenticate(&public(&old), "Recovery owner")
        .await
        .unwrap();
    let s = session(&app, &profile.profile_id).await;
    assert_eq!(
        devices::revoke(&app, &s, &public(&old))
            .await
            .unwrap_err()
            .1,
        "recovery_required_before_last_device"
    );
    let codes = devices::recovery_codes(&app, &s, b"{}").await.unwrap();
    let codes = codes["recovery_codes"].as_array().unwrap();
    assert_eq!(codes.len(), 8);
    let code = codes[0].as_str().unwrap();
    assert_eq!(code.len(), 64);
    let hashes: Vec<String> =
        sqlx::query_scalar("SELECT code_hash FROM portal.recovery_codes WHERE profile_id=$1")
            .bind(&profile.profile_id)
            .fetch_all(&app.pool)
            .await
            .unwrap();
    assert!(!hashes.iter().any(|h| h == code));
    devices::revoke(&app, &s, &public(&old)).await.unwrap();
    assert!(
        !app.career
            .key_is_active(&public(&old), &profile.profile_id)
            .await
            .unwrap()
    );
    assert!(
        app.career
            .authenticate(&public(&old), "New identity attempt")
            .await
            .unwrap_err()
            .contains("revoked")
    );
    assert!(
        devices::create(
            &app,
            &signed(&old, &enrollment(&old), DeviceAction::Create, None, None)
        )
        .await
        .is_err()
    );
    let still_original: Option<String> =
        sqlx::query_scalar("SELECT profile_id FROM career_keys WHERE public_key=$1")
            .bind(public(&old))
            .fetch_optional(&app.pool)
            .await
            .unwrap();
    assert_eq!(still_original.as_deref(), Some(profile.profile_id.as_str()));
    let new = random_key();
    let (e, _) = start(&app, &new).await;
    let invalid_code = crypto::random::<32>();
    assert!(
        devices::recover(
            &app,
            &signed(&new, &e, DeviceAction::Recover, None, Some(&invalid_code))
        )
        .await
        .is_err()
    );
    let proof = signed(&new, &e, DeviceAction::Recover, None, Some(code));
    let recovered = devices::recover(&app, &proof).await.unwrap();
    assert_eq!(recovered["profile_id"], profile.profile_id);
    // Network retry of the same operation is safe; another device cannot reuse it.
    assert_eq!(
        devices::recover(&app, &proof).await.unwrap()["state"],
        "approved"
    );
    let attacker = random_key();
    let (a, _) = start(&app, &attacker).await;
    assert!(
        devices::recover(
            &app,
            &signed(&attacker, &a, DeviceAction::Recover, None, Some(code))
        )
        .await
        .is_err()
    );
    devices::complete(
        &app,
        &signed(
            &new,
            &e,
            DeviceAction::Complete,
            Some(&profile.profile_id),
            None,
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        app.career
            .authenticate(&public(&new), "Ignored")
            .await
            .unwrap()
            .profile_id,
        profile.profile_id
    );
    assert!(devices::recovery_codes(&app, &s, b"{}").await.is_err());
    let fresh = session(&app, &profile.profile_id).await;
    assert_eq!(
        devices::list(&app, &fresh).await.unwrap()["recovery_codes_remaining"],
        7
    );
}
#[tokio::test]
#[ignore = "requires isolated owner OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn fresh_confirmation_and_expiry_are_required() {
    let app = app().await;
    let owner = random_key();
    let profile = app
        .career
        .authenticate(&public(&owner), "Fresh owner")
        .await
        .unwrap();
    let s = session(&app, &profile.profile_id).await;
    sqlx::query("UPDATE portal.web_sessions SET created_at=$2 WHERE session_id=$1")
        .bind(&s.session_id)
        .bind(now() - 601)
        .execute(&app.pool)
        .await
        .unwrap();
    assert_eq!(
        devices::recovery_codes(&app, &s, b"{}")
            .await
            .unwrap_err()
            .1,
        "fresh_confirmation_required"
    );
    let new = random_key();
    let mut e = enrollment(&new);
    e.expires_at = now().to_string();
    assert!(
        devices::create(&app, &signed(&new, &e, DeviceAction::Create, None, None))
            .await
            .is_err()
    );
    e.expires_at = (now() + 3600).to_string();
    assert!(
        devices::create(&app, &signed(&new, &e, DeviceAction::Create, None, None))
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires isolated owner OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn recovery_code_race_authorizes_exactly_one_new_device() {
    let app = app().await;
    let owner = random_key();
    let profile = app
        .career
        .authenticate(&public(&owner), "Recovery race")
        .await
        .unwrap();
    let s = session(&app, &profile.profile_id).await;
    let result = devices::recovery_codes(&app, &s, b"{}").await.unwrap();
    let code = result["recovery_codes"][0].as_str().unwrap();
    let a = random_key();
    let b = random_key();
    let (ea, _) = start(&app, &a).await;
    let (eb, _) = start(&app, &b).await;
    let pa = signed(&a, &ea, DeviceAction::Recover, None, Some(code));
    let pb = signed(&b, &eb, DeviceAction::Recover, None, Some(code));
    let (ra, rb) = tokio::join!(devices::recover(&app, &pa), devices::recover(&app, &pb));
    assert_eq!(usize::from(ra.is_ok()) + usize::from(rb.is_ok()), 1);
    let approved: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM portal.device_enrollments WHERE profile_id=$1 AND state='approved'",
    )
    .bind(&profile.profile_id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(approved, 1);
}

#[tokio::test]
#[ignore = "requires isolated OMOBA_PORTAL_TEST_DATABASE_URL and OMOBA_PORTAL_ROLE_TEST_URL"]
async fn enrollment_and_revocation_work_with_restricted_portal_role() {
    let owner = app().await;
    let runtime = App::connect(
        &std::env::var("OMOBA_PORTAL_ROLE_TEST_URL").expect("isolated portal runtime role"),
        Config {
            origin: "https://players.test".into(),
            secret: [17; 32],
            trust_loopback_proxy: false,
            release_file: None,
        },
    )
    .await
    .unwrap();
    let old = random_key();
    let profile = owner
        .career
        .authenticate(&public(&old), "Restricted role")
        .await
        .unwrap();
    let s = session(&runtime, &profile.profile_id).await;
    let new = random_key();
    let (e, created) = start(&runtime, &new).await;
    approve(&runtime, &s, &e, created["code"].as_str().unwrap()).await;
    devices::complete(
        &runtime,
        &signed(
            &new,
            &e,
            DeviceAction::Complete,
            Some(&profile.profile_id),
            None,
        ),
    )
    .await
    .unwrap();
    devices::recovery_codes(&runtime, &s, b"{}").await.unwrap();
    devices::revoke(&runtime, &s, &public(&old)).await.unwrap();
    assert!(
        owner
            .career
            .key_is_active(&public(&new), &profile.profile_id)
            .await
            .unwrap()
    );
    assert!(
        !owner
            .career
            .key_is_active(&public(&old), &profile.profile_id)
            .await
            .unwrap()
    );
    runtime.cleanup().await.unwrap();
    for query in [
        "UPDATE career_keys SET profile_id=profile_id WHERE false",
        "DELETE FROM career_keys WHERE false",
        "UPDATE career_profiles SET progression_xp=0 WHERE false",
        "DELETE FROM portal.supporter_events WHERE false",
    ] {
        let error = sqlx::query(query).execute(&runtime.pool).await.unwrap_err();
        assert_eq!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42501"),
            "{query}"
        );
    }
}

#[tokio::test]
#[ignore = "requires isolated owner OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn legacy_browser_consent_cannot_gain_device_or_recovery_authority() {
    let app = app().await;
    let key = random_key();
    let profile = app
        .career
        .authenticate(&public(&key), "Legacy consent")
        .await
        .unwrap();
    let token = crypto::random::<32>();
    let sid = crypto::random::<32>();
    // An old running replica omits the version: default1 remains unprivileged.
    sqlx::query("INSERT INTO portal.web_sessions(session_id,profile_id,token_hash,created_at,last_seen,expires_at) VALUES($1,$2,$3,$4,$4,$5)")
        .bind(&sid).bind(&profile.profile_id).bind(crypto::mac(&app.config.secret,"session-token",&token)).bind(now()).bind(now()+3600).execute(&app.pool).await.unwrap();
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
    assert!(auth::session(&app, &headers).await.is_err());
    let legacy = Session {
        session_id: sid,
        profile_id: profile.profile_id.clone(),
        created_at: now(),
    };
    assert_eq!(
        devices::recovery_codes(&app, &legacy, b"{}")
            .await
            .unwrap_err()
            .1,
        "fresh_confirmation_required"
    );
    let pair = auth::create(&app, b"{}").await.unwrap();
    let pair_id = pair["pair_id"].as_str().unwrap();
    let poll =
        serde_json::to_vec(&json!({"pair_id":pair_id,"poll_secret":pair["poll_secret"]})).unwrap();
    // Old-replica approval cannot be completed into an expanded-scope session.
    sqlx::query("UPDATE portal.web_pairings SET state='approved',profile_id=$2,approved_key=$3 WHERE pair_id=$1")
        .bind(pair_id).bind(&profile.profile_id).bind(public(&key)).execute(&app.pool).await.unwrap();
    assert!(auth::complete(&app, &poll).await.is_err());
    let fresh_pair = auth::create(&app, b"{}").await.unwrap();
    let lookup = auth::lookup(
        &app,
        &serde_json::to_vec(&json!({"code":fresh_pair["code"],"public_key":public(&key)})).unwrap(),
    )
    .await
    .unwrap();
    let mut challenge: shared::web_account::WebPairChallenge =
        serde_json::from_value(lookup["challenge"].clone()).unwrap();
    challenge.scopes.truncate(4);
    let decision = shared::web_account::WebPairDecision::Approve;
    let signed = shared::web_account::SignedWebPair {
        signature: crypto::hex(&key.sign(&challenge.signing_bytes(decision)).to_bytes()),
        challenge,
        decision,
    };
    assert!(
        auth::decide(&app, &serde_json::to_vec(&signed).unwrap(), decision)
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires isolated owner OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn device_migration_revokes_legacy_sessions_and_pending_pairings() {
    let app = app().await;
    let profile = app
        .career
        .authenticate(&public(&random_key()), "Upgrade fixture")
        .await
        .unwrap();
    let mut tx = app.pool.begin().await.unwrap();
    let schema = format!("device_upgrade_{}", crypto::random::<8>());
    sqlx::raw_sql(&PORTAL_MIGRATION.replace("portal", &schema))
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(&format!("INSERT INTO {schema}.web_sessions(session_id,profile_id,token_hash,created_at,last_seen,expires_at) VALUES('legacy',$1,'hash',1,1,9999999999)"))
        .bind(&profile.profile_id).execute(&mut *tx).await.unwrap();
    for state in ["pending", "approved", "consumed"] {
        sqlx::query(&format!("INSERT INTO {schema}.web_pairings(pair_id,code_hash,poll_hash,nonce,expires_at,state,delivery,delivery_until) VALUES($1,$2,'poll','nonce',9999999999,$2,$3,9999999999)"))
            .bind(crypto::random::<32>()).bind(state).bind(vec![1u8,2,3]).execute(&mut *tx).await.unwrap();
    }
    sqlx::raw_sql(&DEVICES_MIGRATION.replace("portal", &schema))
        .execute(&mut *tx)
        .await
        .unwrap();
    let (revoked, version): (bool, i16) = sqlx::query_as(&format!(
        "SELECT revoked,authorization_version FROM {schema}.web_sessions WHERE session_id='legacy'"
    ))
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert!(revoked);
    assert_eq!(version, 1);
    let pending:i64=sqlx::query_scalar(&format!("SELECT count(*) FROM {schema}.web_pairings WHERE state!='cancelled' OR delivery IS NOT NULL OR delivery_until IS NOT NULL OR authorization_version!=1"))
        .fetch_one(&mut *tx).await.unwrap();
    assert_eq!(pending, 0);
    tx.rollback().await.unwrap();
}
