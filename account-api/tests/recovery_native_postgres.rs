//! In-game recovery codes (LR-18): `POST /v1/auth/devices/recovery-codes`, signed
//! by the game's active device key. Run only against an isolated test database.
use axum::http::StatusCode;
use ed25519_dalek::{Signer, SigningKey};
use omoba_account_api::{auth::Session, crypto, devices, *};
use serde_json::{Value, json};
use shared::device_account::*;

const ORIGIN: &str = "https://players.test";

fn random_key() -> SigningKey {
    let mut seed = [0; 32];
    getrandom::fill(&mut seed).unwrap();
    SigningKey::from_bytes(&seed)
}
fn public(key: &SigningKey) -> String {
    crypto::hex(key.verifying_key().as_bytes())
}
fn config() -> Config {
    Config {
        origin: ORIGIN.into(),
        secret: [17; 32],
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
fn request(key: &SigningKey, action: RecoveryAction) -> NativeRecoveryRequest {
    NativeRecoveryRequest {
        origin: ORIGIN.into(),
        public_key: public(key),
        nonce: crypto::random::<32>(),
        expires_at: now() as u64 + 55,
        action,
    }
}
fn sign(signer: &SigningKey, request: NativeRecoveryRequest) -> Vec<u8> {
    let signature = crypto::hex(&signer.sign(&request.signing_bytes()).to_bytes());
    serde_json::to_vec(&SignedNativeRecoveryRequest { request, signature }).unwrap()
}
fn signed(key: &SigningKey, action: RecoveryAction) -> Vec<u8> {
    sign(key, request(key, action))
}
fn protection(value: &Value) -> AccountProtection {
    serde_json::from_value(value.clone()).unwrap()
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
fn enrollment(key: &SigningKey) -> DeviceEnrollment {
    DeviceEnrollment {
        enrollment_id: crypto::random::<32>(),
        public_key: public(key),
        origin: ORIGIN.into(),
        label: "Android".into(),
        expires_at: (now() + 299).to_string(),
    }
}
fn enroll(
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
/// A reinstalled game: a fresh key recovers with `code`, then completes; `auth`
/// is the game server that signs it in after the restart.
async fn reinstall_on(app: &App, auth: &App, code: &str) -> std::result::Result<String, Error> {
    let fresh = random_key();
    let e = enrollment(&fresh);
    devices::create(app, &enroll(&fresh, &e, DeviceAction::Create, None, None)).await?;
    let recovered = devices::recover(
        app,
        &enroll(&fresh, &e, DeviceAction::Recover, None, Some(code)),
    )
    .await?;
    let profile = recovered["profile_id"].as_str().unwrap().to_owned();
    devices::complete(
        app,
        &enroll(&fresh, &e, DeviceAction::Complete, Some(&profile), None),
    )
    .await?;
    // After the restart the game signs in with the recovered key.
    Ok(auth
        .career
        .authenticate(&public(&fresh), "Ignored")
        .await
        .unwrap()
        .profile_id)
}
async fn reinstall_with(app: &App, code: &str) -> std::result::Result<String, Error> {
    reinstall_on(app, app, code).await
}

#[tokio::test]
#[ignore = "requires isolated owner OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn in_game_codes_are_issued_by_the_active_key_and_bring_a_reinstall_back() {
    let app = app().await;
    let key = random_key();
    let profile = app
        .career
        .authenticate(&public(&key), "Protect me")
        .await
        .unwrap();
    // A fresh install: one device, nothing that survives losing it.
    let status = protection(
        &devices::native_recovery(&app, &signed(&key, RecoveryAction::Status))
            .await
            .unwrap(),
    );
    assert_eq!(
        (
            status.recovery_codes_remaining,
            status.active_devices,
            status.linked_sign_ins.len()
        ),
        (0, 1, 0)
    );
    assert!(!status.protected());
    assert!(status.recovery_codes.is_empty());

    let rotate = signed(&key, RecoveryAction::Rotate);
    let issued = protection(&devices::native_recovery(&app, &rotate).await.unwrap());
    assert_eq!(issued.recovery_codes.len(), 8);
    assert_eq!(issued.recovery_codes_remaining, 8);
    assert!(issued.protected());
    assert!(
        issued
            .recovery_codes
            .iter()
            .all(|code| code.len() == 64 && crypto::decode::<32>(code).is_some())
    );
    // Only keyed hashes are stored.
    let stored: Vec<String> =
        sqlx::query_scalar("SELECT code_hash FROM portal.recovery_codes WHERE profile_id=$1")
            .bind(&profile.profile_id)
            .fetch_all(&app.pool)
            .await
            .unwrap();
    assert!(
        !stored
            .iter()
            .any(|hash| issued.recovery_codes.contains(hash))
    );
    let audited: i64 = sqlx::query_scalar("SELECT count(*) FROM portal.audit_events WHERE profile_id=$1 AND event='recovery_codes_rotated_in_game'")
        .bind(&profile.profile_id).fetch_one(&app.pool).await.unwrap();
    assert_eq!(audited, 1);
    // A captured request cannot be replayed for a second set of codes.
    let replay = devices::native_recovery(&app, &rotate).await.unwrap_err();
    assert_eq!(
        (replay.0, replay.1),
        (StatusCode::CONFLICT, "request_replayed")
    );
    // The portal sees the same set.
    let s = session(&app, &profile.profile_id).await;
    assert_eq!(
        devices::list(&app, &s).await.unwrap()["recovery_codes_remaining"],
        8
    );

    // Reinstall: an in-game code brings back the same profile_id.
    let back = reinstall_with(&app, &issued.recovery_codes[0])
        .await
        .unwrap();
    assert_eq!(back, profile.profile_id);
    let after = protection(
        &devices::native_recovery(&app, &signed(&key, RecoveryAction::Status))
            .await
            .unwrap(),
    );
    assert_eq!(
        (after.recovery_codes_remaining, after.active_devices),
        (7, 2)
    );

    // A new set replaces the unused codes of the old one.
    let replaced = protection(
        &devices::native_recovery(&app, &signed(&key, RecoveryAction::Rotate))
            .await
            .unwrap(),
    );
    assert_eq!(replaced.recovery_codes_remaining, 8);
    assert!(
        reinstall_with(&app, &issued.recovery_codes[1])
            .await
            .is_err()
    );
    assert_eq!(
        reinstall_with(&app, &replaced.recovery_codes[0])
            .await
            .unwrap(),
        profile.profile_id
    );
}

#[tokio::test]
#[ignore = "requires isolated owner OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn only_an_active_key_with_a_fresh_signed_request_is_answered() {
    let app = app().await;
    let key = random_key();
    let profile = app
        .career
        .authenticate(&public(&key), "Strict")
        .await
        .unwrap();
    // A key that never played has no account to protect.
    let stranger = random_key();
    assert_eq!(
        devices::native_recovery(&app, &signed(&stranger, RecoveryAction::Rotate))
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
    // Another key cannot sign for this one.
    assert_eq!(
        devices::native_recovery(
            &app,
            &sign(&stranger, request(&key, RecoveryAction::Rotate))
        )
        .await
        .unwrap_err()
        .0,
        StatusCode::NOT_FOUND
    );
    // Expired, too long-lived, another portal or a malformed nonce.
    for (expires_at, origin, nonce) in [
        (now() as u64 - 1, ORIGIN, crypto::random::<32>()),
        (now() as u64 + 120, ORIGIN, crypto::random::<32>()),
        (
            now() as u64 + 30,
            "https://evil.test",
            crypto::random::<32>(),
        ),
        (now() as u64 + 30, ORIGIN, "zz".repeat(32)),
    ] {
        let bad = NativeRecoveryRequest {
            origin: origin.into(),
            nonce,
            expires_at,
            ..request(&key, RecoveryAction::Rotate)
        };
        assert_eq!(
            devices::native_recovery(&app, &sign(&key, bad))
                .await
                .unwrap_err()
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        devices::native_recovery(&app, br#"{"request":{},"signature":""}"#)
            .await
            .unwrap_err()
            .0,
        StatusCode::BAD_REQUEST
    );
    let untouched: i64 =
        sqlx::query_scalar("SELECT count(*) FROM portal.recovery_codes WHERE profile_id=$1")
            .bind(&profile.profile_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(untouched, 0);

    // A linked VK sign-in protects the account too.
    sqlx::query("INSERT INTO portal.social_identities(provider,subject_hash,method,profile_id,created_at,last_login_at,linked_at) VALUES('vk',$1,'vkid',$2,$3,$3,$3)")
        .bind(crypto::random::<32>()).bind(&profile.profile_id).bind(now()).execute(&app.pool).await.unwrap();
    let linked = protection(
        &devices::native_recovery(&app, &signed(&key, RecoveryAction::Status))
            .await
            .unwrap(),
    );
    assert_eq!(linked.linked_sign_ins, ["vk"]);
    assert!(linked.protected());

    // A revoked key is refused, like every other route it signs.
    let s = session(&app, &profile.profile_id).await;
    devices::recovery_codes(&app, &s, b"{}").await.unwrap();
    devices::revoke(&app, &s, &public(&key)).await.unwrap();
    assert_eq!(
        devices::native_recovery(&app, &signed(&key, RecoveryAction::Status))
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
#[ignore = "requires isolated owner OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn the_route_is_served_over_http_with_its_own_address_bucket() {
    use axum::body::{Body, to_bytes};
    use tower::ServiceExt;
    let app = app().await;
    let key = random_key();
    app.career
        .authenticate(&public(&key), "Over http")
        .await
        .unwrap();
    let call = |body: Vec<u8>| {
        let app = app.clone();
        async move {
            let mut request = axum::http::Request::builder()
                .method("POST")
                .uri("/v1/auth/devices/recovery-codes")
                .body(Body::from(body))
                .unwrap();
            request.extensions_mut().insert(axum::extract::ConnectInfo(
                "127.0.0.18:42125".parse::<std::net::SocketAddr>().unwrap(),
            ));
            let response = router(app).oneshot(request).await.unwrap();
            let status = response.status();
            let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
            (status, serde_json::from_slice::<Value>(&bytes).unwrap())
        }
    };
    let (status, value) = call(signed(&key, RecoveryAction::Rotate)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["recovery_codes"].as_array().unwrap().len(), 8);
    assert_eq!(value["active_devices"], json!(1));
    let (status, value) = call(signed(&key, RecoveryAction::Status)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(value.get("recovery_codes").is_none());
    assert_eq!(value["recovery_codes_remaining"], json!(8));
    // The device-enrollment bucket is untouched: ten enrollments still pass.
    let charged: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM portal.rate_limits WHERE identity_hash=$1 AND bucket=$2",
    )
    .bind(crypto::mac(&[17; 32], "device-enrollment", "127.0.0.18"))
    .bind(now() / 60)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    let recovery: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM portal.rate_limits WHERE identity_hash=$1 AND bucket=$2",
    )
    .bind(crypto::mac(&[17; 32], "recovery-native-ip", "127.0.0.18"))
    .bind(now() / 60)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!((charged, recovery), (0, 1));
}

#[tokio::test]
#[ignore = "requires isolated OMOBA_PORTAL_TEST_DATABASE_URL and OMOBA_PORTAL_ROLE_TEST_URL"]
async fn the_restricted_portal_role_serves_in_game_codes() {
    let owner = app().await;
    let runtime = App::connect(
        &std::env::var("OMOBA_PORTAL_ROLE_TEST_URL").expect("isolated portal runtime role"),
        config(),
    )
    .await
    .unwrap();
    let key = random_key();
    let profile = owner
        .career
        .authenticate(&public(&key), "Restricted codes")
        .await
        .unwrap();
    let issued = protection(
        &devices::native_recovery(&runtime, &signed(&key, RecoveryAction::Rotate))
            .await
            .unwrap(),
    );
    assert_eq!(issued.recovery_codes.len(), 8);
    assert_eq!(
        reinstall_on(&runtime, &owner, &issued.recovery_codes[0])
            .await
            .unwrap(),
        profile.profile_id
    );
    let status = protection(
        &devices::native_recovery(&runtime, &signed(&key, RecoveryAction::Status))
            .await
            .unwrap(),
    );
    assert_eq!(
        (status.recovery_codes_remaining, status.active_devices),
        (7, 2)
    );
}
