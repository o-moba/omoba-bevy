//! Explicit disposable PostgreSQL + local HTTP protocol fixture; no hosted writes.
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use ed25519_dalek::{Signer, SigningKey};
use omoba_account_api::{auth, crypto, ekza_library, *};
use serde_json::{Value, json};
use shared::web_account::{SignedWebPair, WebPairChallenge, WebPairDecision};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone)]
struct Fixture {
    origin: String,
    mode: Arc<AtomicUsize>,
}
async fn start(State(s): State<Fixture>, Json(body): Json<Value>) -> Json<Value> {
    assert_eq!(body, json!({"projectId":"omoba","scope":"library"}));
    Json(
        json!({"deviceCode":"D".repeat(48),"userCode":"ABCDEFGH","verificationUrl":format!("{}/studio?view=connect&code=ABCDEFGH",s.origin),"expiresAt":"2099-01-01T00:00:00Z","interval":3}),
    )
}
async fn poll(State(s): State<Fixture>, Json(body): Json<Value>) -> Json<Value> {
    assert_eq!(body["deviceCode"], "D".repeat(48));
    if s.mode.load(Ordering::SeqCst) == 0 {
        return Json(json!({"status":"pending"}));
    }
    let mut result = json!({"status":"approved","scope":"library","accessToken":"T".repeat(64),"expiresAt":"2099-01-01T00:00:00Z","projectId":"omoba","account":{"username":"SyntheticArtist"}});
    if s.mode.load(Ordering::SeqCst) == 6 {
        result["scope"] = json!("space-session");
    }
    if s.mode.load(Ordering::SeqCst) == 8 {
        result["projectId"] = json!("other-game");
    }
    Json(result)
}
async fn library(State(s): State<Fixture>, headers: HeaderMap) -> axum::response::Response {
    assert_eq!(
        headers["authorization"],
        format!("Bearer {}", "T".repeat(64))
    );
    match s.mode.load(Ordering::SeqCst) {
        2 => (StatusCode::SERVICE_UNAVAILABLE,Json(json!({"code":"fixture_outage"}))).into_response(),
        4 => (StatusCode::NOT_FOUND,Json(json!({"code":"fixture_route_unavailable"}))).into_response(),
        5 => (StatusCode::FORBIDDEN,Json(json!({"code":"fixture_proxy_denied"}))).into_response(),
        3 => (StatusCode::UNAUTHORIZED,Json(json!({"code":"link_invalid"}))).into_response(),
        _ => Json(json!({"schema":"ekza.account.library.v1","projectId":"omoba","account":{"username":"SyntheticArtistRenamed"},"expiresAt":"2099-01-01T00:00:00Z","items":[]})).into_response(),
    }
}
async fn login(app: &App) -> auth::Session {
    let key = SigningKey::from_bytes(&crypto::decode::<32>(&crypto::random::<32>()).unwrap());
    let public = crypto::hex(key.verifying_key().as_bytes());
    app.career
        .authenticate(&public, "FixtureArtist")
        .await
        .unwrap();
    let p = auth::create(app, b"{}").await.unwrap();
    let found = auth::lookup(
        app,
        &serde_json::to_vec(&json!({"code":p["code"],"public_key":public})).unwrap(),
    )
    .await
    .unwrap();
    let challenge: WebPairChallenge = serde_json::from_value(found["challenge"].clone()).unwrap();
    let proof = SignedWebPair {
        signature: crypto::hex(
            &key.sign(&challenge.signing_bytes(WebPairDecision::Approve))
                .to_bytes(),
        ),
        challenge,
        decision: WebPairDecision::Approve,
    };
    auth::decide(
        app,
        &serde_json::to_vec(&proof).unwrap(),
        WebPairDecision::Approve,
    )
    .await
    .unwrap();
    let completed = auth::complete(
        app,
        &serde_json::to_vec(&json!({"pair_id":p["pair_id"],"poll_secret":p["poll_secret"]}))
            .unwrap(),
    )
    .await
    .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        "authorization",
        format!("Bearer {}", completed["session_token"].as_str().unwrap())
            .parse()
            .unwrap(),
    );
    auth::session(app, &headers).await.unwrap()
}
async fn call(app: &App, s: &auth::Session, action: &str) -> Value {
    ekza_library::handle(
        app,
        s,
        action,
        None,
        if matches!(action, "connect" | "poll") {
            b"{}"
        } else {
            b""
        },
    )
    .await
    .unwrap()
}
async fn allow_poll(app: &App, s: &auth::Session) {
    sqlx::query("UPDATE portal.ekza_library_links SET next_poll=0 WHERE profile_id=$1")
        .bind(&s.profile_id)
        .execute(&app.pool)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires explicit disposable OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn owner_bound_library_consent_keeps_credentials_private_and_survives_outage() {
    let url = std::env::var("OMOBA_PORTAL_TEST_DATABASE_URL")
        .expect("explicit disposable database required");
    let parsed: sqlx::postgres::PgConnectOptions = url.parse().unwrap();
    assert_eq!(parsed.get_host(), "127.0.0.1");
    // Accept only the known local workshop fixture or existing CI service.
    assert!(
        matches!(
            (parsed.get_port(), parsed.get_database()),
            (55581, Some("workshop_test")) | (5432, Some("omoba_test"))
        ),
        "Only the dedicated workshop or CI test database"
    );
    migrate(&url).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let mode = Arc::new(AtomicUsize::new(0));
    let fixture = Router::new()
        .route("/v1/account/device", post(start))
        .route("/v1/account/device/poll", post(poll))
        .route("/v1/account/library", get(library))
        .with_state(Fixture {
            origin: origin.clone(),
            mode: mode.clone(),
        });
    let server = tokio::spawn(async move { axum::serve(listener, fixture).await.unwrap() });
    let mut app = App::connect(
        &url,
        Config {
            origin: "http://127.0.0.1:3010".into(),
            secret: [19; 32],
            trust_loopback_proxy: false,
            release_file: None,
        },
    )
    .await
    .unwrap();
    app.ekza =
        Arc::new(ekza_library::EkzaConfig::local_fixture(&origin, &app.config.origin).unwrap());
    let owner = login(&app).await;
    let other = login(&app).await;
    assert_eq!(
        call(&app, &owner, "get").await,
        json!({"state":"not_connected","prototype":true})
    );
    let pending = call(&app, &owner, "connect").await;
    assert_eq!(pending["state"], "pending");
    assert!(pending.get("deviceCode").is_none());
    assert_eq!(
        call(&app, &owner, "connect").await,
        pending,
        "repeated connect reuses pending consent"
    );
    assert_eq!(call(&app, &other, "get").await["state"], "not_connected");
    allow_poll(&app, &owner).await;
    assert_eq!(call(&app, &owner, "poll").await["state"], "pending");
    for mismatched_consent in [6, 8] {
        mode.store(mismatched_consent, Ordering::SeqCst);
        allow_poll(&app, &owner).await;
        assert_eq!(
            ekza_library::handle(&app, &owner, "poll", None, b"{}")
                .await
                .unwrap_err()
                .0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        let phase: String =
            sqlx::query_scalar("SELECT phase FROM portal.ekza_library_links WHERE profile_id=$1")
                .bind(&owner.profile_id)
                .fetch_one(&app.pool)
                .await
                .unwrap();
        assert_eq!(phase, "pending");
    }
    mode.store(1, Ordering::SeqCst);
    allow_poll(&app, &owner).await;
    let linked = call(&app, &owner, "poll").await;
    assert_eq!(linked["username"], "SyntheticArtist");
    assert_eq!(linked["state"], "connected");
    assert!(linked.get("accessToken").is_none());
    assert!(linked.get("deviceCode").is_none());
    let cipher: Vec<u8> = sqlx::query_scalar(
        "SELECT sealed_secret FROM portal.ekza_library_links WHERE profile_id=$1",
    )
    .bind(&owner.profile_id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert!(!cipher.windows(64).any(|w| w == "T".repeat(64).as_bytes()));
    assert!(
        crypto::open(
            &app.config.secret,
            &format!("ekza-library:{}:{origin}:connected", other.profile_id),
            &cipher
        )
        .is_err()
    );
    assert_eq!(
        call(&app, &owner, "get").await["username"],
        "SyntheticArtistRenamed"
    );
    call(&app, &other, "disconnect").await;
    assert_eq!(call(&app, &owner, "get").await["state"], "connected");
    mode.store(2, Ordering::SeqCst);
    assert_eq!(call(&app, &owner, "get").await["state"], "unavailable");
    mode.store(1, Ordering::SeqCst);
    assert_eq!(
        call(&app, &owner, "get").await["state"],
        "connected",
        "outage must retain encrypted consent"
    );
    for upstream_failure in [4, 5] {
        mode.store(upstream_failure, Ordering::SeqCst);
        assert_eq!(call(&app, &owner, "get").await["state"], "unavailable");
        mode.store(1, Ordering::SeqCst);
        assert_eq!(call(&app, &owner, "get").await["state"], "connected");
    }
    mode.store(3, Ordering::SeqCst);
    assert_eq!(
        call(&app, &owner, "get").await["state"],
        "not_connected",
        "revocation clears unusable grant"
    );
    mode.store(0, Ordering::SeqCst);
    call(&app, &owner, "connect").await;
    sqlx::query("UPDATE portal.ekza_library_links SET pending_until=0 WHERE profile_id=$1")
        .bind(&owner.profile_id)
        .execute(&app.pool)
        .await
        .unwrap();
    assert_eq!(call(&app, &owner, "get").await["state"], "not_connected");
    assert_eq!(
        ekza_library::handle(&app, &owner, "connect", None, br#"{"profile_id":"forged"}"#)
            .await
            .unwrap_err()
            .0,
        StatusCode::BAD_REQUEST
    );
    auth::logout(&app, &owner).await.unwrap();
    assert_eq!(
        ekza_library::handle(&app, &owner, "connect", None, b"{}")
            .await
            .unwrap_err()
            .0,
        StatusCode::UNAUTHORIZED
    );
    server.abort();
}
