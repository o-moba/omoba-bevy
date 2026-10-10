//! Explicit disposable PostgreSQL fixture; never inherit the production database URL.
use axum::body::{Body, to_bytes};
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Request, StatusCode};
use ed25519_dalek::{Signer, SigningKey};
use omoba_account_api::{auth, class_builds, crypto, *};
use serde_json::{Value, json};
use shared::loadout::CoreId;
use shared::web_account::{SignedWebPair, WebPairChallenge, WebPairDecision};
use std::{net::SocketAddr, str::FromStr};
use tower::ServiceExt;

fn database(variable: &str) -> String {
    let url = std::env::var(variable).expect("Explicit disposable database required");
    let options = sqlx::postgres::PgConnectOptions::from_str(&url).expect("valid test URL");
    assert_eq!(
        options.get_host(),
        "127.0.0.1",
        "Only isolated loopback fixtures"
    );
    // The local workshop fixture and the existing CI service use distinct pairs.
    assert!(
        matches!(
            (options.get_port(), options.get_database()),
            (55581, Some("workshop_test")) | (5432, Some("omoba_test"))
        ),
        "Only the dedicated workshop or CI test database"
    );
    url
}
fn config() -> Config {
    Config {
        origin: "https://players.example".into(),
        secret: [91; 32],
        trust_loopback_proxy: false,
        release_file: None,
    }
}
async fn app() -> App {
    let url = database("OMOBA_PORTAL_TEST_DATABASE_URL");
    migrate(&url).await.unwrap();
    migrate(&url).await.unwrap();
    App::connect(&url, config()).await.unwrap()
}
fn bytes(value: Value) -> Vec<u8> {
    serde_json::to_vec(&value).unwrap()
}
fn headers(key: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("idempotency-key", key.parse().unwrap());
    headers
}
fn operation() -> HeaderMap {
    headers(&crypto::random::<16>())
}
fn document(name: &str) -> Value {
    json!({"schema":"omoba.class-build.v1","name":name,"description":"Private synthetic class preference","recipe":CoreId::Dawnweaver.preset()})
}
async fn login(app: &App) -> (auth::Session, String) {
    let mut seed = [0; 32];
    getrandom::fill(&mut seed).unwrap();
    let key = SigningKey::from_bytes(&seed);
    let public = crypto::hex(key.verifying_key().as_bytes());
    app.career
        .authenticate(&public, "Class fixture")
        .await
        .unwrap();
    let created = auth::create(app, b"{}").await.unwrap();
    let found = auth::lookup(
        app,
        &bytes(json!({"code":created["code"],"public_key":public})),
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
    let complete = auth::complete(
        app,
        &bytes(json!({"pair_id":created["pair_id"],"poll_secret":created["poll_secret"]})),
    )
    .await
    .unwrap();
    let token = complete["session_token"].as_str().unwrap().to_owned();
    let mut headers = HeaderMap::new();
    headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
    (auth::session(app, &headers).await.unwrap(), token)
}
async fn create(app: &App, s: &auth::Session, name: &str) -> Value {
    class_builds::mutate(
        app,
        s,
        "POST",
        None,
        None,
        &operation(),
        &bytes(json!({"document":document(name)})),
    )
    .await
    .unwrap()
}
async fn http(
    app: &App,
    token: Option<&str>,
    method: &str,
    path: &str,
    payload: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(format!("/v1/{path}"));
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    request = request.header("idempotency-key", crypto::random::<16>());
    let mut request = request
        .body(payload.map_or_else(Body::empty, |p| Body::from(bytes(p))))
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:32123".parse::<SocketAddr>().unwrap(),
    ));
    let response = router(app.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 65536).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
#[ignore = "requires explicit disposable OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn private_build_crud_signed_sessions_isolation_and_http_revocation() {
    let app = app().await;
    let (owner, token) = login(&app).await;
    let (other, _) = login(&app).await;
    assert_eq!(
        http(&app, None, "GET", "me/class-builds", None).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, created) = http(
        &app,
        Some(&token),
        "POST",
        "me/class-builds",
        Some(json!({"document":document("Browser build")})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let id = created["build"]["id"].as_str().unwrap();
    assert_eq!(created["build"]["version"], "1");
    assert!(
        created["build"]["created_at"]
            .as_str()
            .unwrap()
            .ends_with('Z')
    );
    assert_eq!(
        class_builds::get(&app, &owner, id, None).await.unwrap(),
        created
    );
    assert_eq!(
        class_builds::list(&app, &owner, None).await.unwrap()["builds"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        class_builds::list(&app, &other, None).await.unwrap()["builds"],
        json!([])
    );
    assert_eq!(
        class_builds::get(&app, &other, id, None)
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
    for method in ["PATCH", "DELETE"] {
        let body = if method == "PATCH" {
            bytes(json!({"expected_version":"1","document":document("Hijacked")}))
        } else {
            vec![]
        };
        let query = (method == "DELETE").then_some("version=1");
        assert_eq!(
            class_builds::mutate(&app, &other, method, Some(id), query, &operation(), &body)
                .await
                .unwrap_err()
                .0,
            StatusCode::NOT_FOUND
        );
    }
    let injected = bytes(json!({"document":document("Injected"),"profile_id":other.profile_id}));
    assert_eq!(
        class_builds::mutate(&app, &owner, "POST", None, None, &operation(), &injected)
            .await
            .unwrap_err()
            .0,
        StatusCode::BAD_REQUEST
    );
    let (status, got) = http(
        &app,
        Some(&token),
        "GET",
        &format!("me/class-builds/{id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got, created);
    auth::logout(&app, &owner).await.unwrap();
    assert_eq!(
        http(&app, Some(&token), "GET", "me/class-builds", None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        class_builds::mutate(
            &app,
            &owner,
            "POST",
            None,
            None,
            &operation(),
            &bytes(json!({"document":document("After logout")}))
        )
        .await
        .unwrap_err()
        .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
#[ignore = "requires explicit disposable OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn idempotent_mutations_version_conflicts_and_delete_preconditions() {
    let app = app().await;
    let (owner, _) = login(&app).await;
    let key = operation();
    let original = bytes(json!({"document":document("Original")}));
    let first = class_builds::mutate(&app, &owner, "POST", None, None, &key, &original)
        .await
        .unwrap();
    assert_eq!(
        class_builds::mutate(&app, &owner, "POST", None, None, &key, &original)
            .await
            .unwrap(),
        first
    );
    let id = first["build"]["id"].as_str().unwrap();
    let changed = bytes(json!({"document":document("Changed")}));
    assert_eq!(
        class_builds::mutate(&app, &owner, "POST", None, None, &key, &changed)
            .await
            .unwrap_err()
            .1,
        "idempotency_conflict"
    );
    let patch = bytes(json!({"document":document("Updated"),"expected_version":"1"}));
    assert_eq!(
        class_builds::mutate(&app, &owner, "PATCH", Some(id), None, &key, &patch)
            .await
            .unwrap_err()
            .1,
        "idempotency_conflict"
    );
    let update_key = operation();
    let updated = class_builds::mutate(&app, &owner, "PATCH", Some(id), None, &update_key, &patch)
        .await
        .unwrap();
    assert_eq!(updated["build"]["version"], "2");
    assert_eq!(
        class_builds::mutate(&app, &owner, "PATCH", Some(id), None, &update_key, &patch)
            .await
            .unwrap(),
        updated
    );
    assert_eq!(
        class_builds::mutate(&app, &owner, "PATCH", Some(id), None, &operation(), &patch)
            .await
            .unwrap_err()
            .1,
        "state_changed"
    );
    for query in [
        None,
        Some("version=01"),
        Some("version=0"),
        Some("version=-1"),
        Some("version=2&version=2"),
        Some("version=2&profile_id=other"),
    ] {
        assert_eq!(
            class_builds::mutate(&app, &owner, "DELETE", Some(id), query, &operation(), b"")
                .await
                .unwrap_err()
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        class_builds::mutate(
            &app,
            &owner,
            "DELETE",
            Some(id),
            Some("version=1"),
            &operation(),
            b""
        )
        .await
        .unwrap_err()
        .1,
        "state_changed"
    );
    let delete_key = operation();
    let deleted = class_builds::mutate(
        &app,
        &owner,
        "DELETE",
        Some(id),
        Some("version=2"),
        &delete_key,
        b"",
    )
    .await
    .unwrap();
    assert_eq!(
        class_builds::mutate(
            &app,
            &owner,
            "DELETE",
            Some(id),
            Some("version=2"),
            &delete_key,
            b""
        )
        .await
        .unwrap(),
        deleted
    );
    assert_eq!(
        class_builds::get(&app, &owner, id, None)
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
#[ignore = "requires explicit disposable OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn invalid_documents_and_concurrent_quota_fail_without_partial_saves() {
    let app = app().await;
    let (owner, _) = login(&app).await;
    let mut invalids = vec![];
    let mut value = document("Bad");
    value["recipe"]["catalog_revision"] = json!("old");
    invalids.push(value);
    let mut value = document("Bad");
    value["recipe"]["skills"][1] = value["recipe"]["skills"][0].clone();
    invalids.push(value);
    let mut value = document("Bad");
    value["recipe"]["skills"][0] = json!("wild_switch");
    invalids.push(value);
    let mut value = document("Bad");
    value["recipe"]["core"] = json!("warrior");
    invalids.push(value);
    let mut value = document(" ");
    invalids.push(value.clone());
    value["name"] = json!("Bad");
    value["executable"] = json!(true);
    invalids.push(value);
    for document in invalids {
        assert_eq!(
            class_builds::mutate(
                &app,
                &owner,
                "POST",
                None,
                None,
                &operation(),
                &bytes(json!({"document":document}))
            )
            .await
            .unwrap_err()
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        class_builds::list(&app, &owner, None).await.unwrap()["builds"],
        json!([])
    );
    for i in 0..49 {
        create(&app, &owner, &format!("Quota {i}")).await;
    }
    let a = operation();
    let b = operation();
    let payload = bytes(json!({"document":document("Last available")}));
    let (a, b) = tokio::join!(
        class_builds::mutate(&app, &owner, "POST", None, None, &a, &payload),
        class_builds::mutate(&app, &owner, "POST", None, None, &b, &payload)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert_eq!(a.err().or_else(|| b.err()).unwrap().1, "class_build_limit");
    assert_eq!(
        class_builds::list(&app, &owner, None).await.unwrap()["builds"]
            .as_array()
            .unwrap()
            .len(),
        50
    );
}

#[tokio::test]
#[ignore = "requires explicit loopback owner and least-privilege workshop_portal role"]
async fn runtime_grants_support_private_builds_without_game_authority() {
    let owner_app = app().await;
    let (session, _) = login(&owner_app).await;
    let url = database("OMOBA_PORTAL_ROLE_TEST_URL");
    let runtime = App::connect(&url, config()).await.unwrap();
    let created = create(&runtime, &session, "Limited runtime").await;
    let id = created["build"]["id"].as_str().unwrap();
    assert_eq!(
        class_builds::get(&runtime, &session, id, None)
            .await
            .unwrap(),
        created
    );
    assert!(
        sqlx::query("UPDATE career_profiles SET rating=1234 WHERE profile_id=$1")
            .bind(&session.profile_id)
            .execute(&runtime.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("CREATE TABLE portal.runtime_must_not_create(id int)")
            .execute(&runtime.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE portal.schema_version SET version=99 WHERE version=4")
            .execute(&runtime.pool)
            .await
            .is_err()
    );
}
