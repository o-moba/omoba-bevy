//! Run only against an isolated test database. VK ID is a loopback stand-in;
//! no real VK app, user or token is involved.
use axum::{Form, Json, Router, http::StatusCode, routing::post};
use ed25519_dalek::{Signer, SigningKey};
use omoba_account_api::{auth::Session, crypto, devices, social, *};
use serde_json::{Value, json};
use shared::device_account::*;
use std::{collections::HashMap, sync::Arc};

const ORIGIN: &str = "https://players.test";
const CLIENT_ID: &str = "51234567";

fn random_key() -> SigningKey {
    let mut seed = [0; 32];
    getrandom::fill(&mut seed).unwrap();
    SigningKey::from_bytes(&seed)
}
fn public(key: &SigningKey) -> String {
    crypto::hex(key.verifying_key().as_bytes())
}
fn body(value: Value) -> Vec<u8> {
    serde_json::to_vec(&value).unwrap()
}

/// id.vk.ru stand-in: code `user-<id>` signs in VK user `<id>`; any other code is
/// refused. Token exchange checks every parameter VK ID requires.
async fn vk_fixture() -> String {
    async fn token(Form(f): Form<HashMap<String, String>>) -> (StatusCode, Json<Value>) {
        let ok = f.get("grant_type").map(String::as_str) == Some("authorization_code")
            && f.get("client_id").map(String::as_str) == Some(CLIENT_ID)
            && matches!(
                f.get("redirect_uri").map(String::as_str),
                Some("https://players.test/api/auth/vk/callback" | "https://players.test")
            )
            && f.get("device_id").is_some_and(|d| !d.is_empty())
            && f.get("state").is_some_and(|d| !d.is_empty())
            && f.get("code_verifier").is_some_and(|v| v.len() >= 43)
            && !f.contains_key("client_secret");
        match f.get("code").and_then(|c| c.strip_prefix("user-")) {
            Some(id) if ok => {
                let mut token = json!({"access_token":format!("token-{id}"),"expires_in":3600,"state":f["state"]});
                if f["state"] == "wrong-state" {
                    token["state"] = json!("foreign-state");
                } else if f["state"] == "missing-state" {
                    token.as_object_mut().unwrap().remove("state");
                }
                (StatusCode::OK, Json(token))
            }
            _ => (
                StatusCode::BAD_REQUEST,
                Json(json!({"error":"invalid_grant"})),
            ),
        }
    }
    async fn user_info(Form(f): Form<HashMap<String, String>>) -> (StatusCode, Json<Value>) {
        match f.get("access_token").and_then(|t| t.strip_prefix("token-")) {
            Some(id) if f.get("client_id").map(String::as_str) == Some(CLIENT_ID) => (
                StatusCode::OK,
                Json(json!({"user":{"user_id":id,"first_name":"Иван","last_name":"Петров"}})),
            ),
            _ => (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error":"invalid_token"})),
            ),
        }
    }
    let app = Router::new()
        .route("/oauth2/auth", post(token))
        .route("/oauth2/user_info", post(user_info));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{address}")
}
async fn connect(url: &str, vk: &str) -> App {
    let mut app = App::connect(
        url,
        Config {
            origin: ORIGIN.into(),
            secret: [17; 32],
            trust_loopback_proxy: false,
            release_file: None,
        },
    )
    .await
    .unwrap();
    app.social = Arc::new(social::SocialConfig::vk_fixture(CLIENT_ID, vk, [29; 32]).unwrap());
    app
}
async fn app() -> App {
    let url =
        std::env::var("OMOBA_PORTAL_TEST_DATABASE_URL").expect("isolated test database required");
    migrate(&url).await.unwrap();
    connect(&url, &vk_fixture().await).await
}
fn callback(code: &str) -> Vec<u8> {
    body(
        json!({"method":"ok_ru","code":code,"code_verifier":"v".repeat(43),
        "device_id":"device-1","redirect_uri":"https://players.test/api/auth/vk/callback","state":"s1"}),
    )
}
fn registration(user: &str, method: &str) -> Vec<u8> {
    let mut request: Value = serde_json::from_slice(&callback(&format!("user-{user}"))).unwrap();
    request["method"] = json!(method);
    request["register"] = json!(true);
    body(request)
}
fn vk_user() -> String {
    // Unique per test run: identities persist in the shared test database.
    (u64::from_str_radix(&crypto::random::<6>(), 16).unwrap() + 1).to_string()
}

#[tokio::test]
#[ignore = "requires isolated owner OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn one_tap_root_can_register_and_sign_back_into_the_same_profile() {
    let app = app().await;
    let user = vk_user();
    let mut request: Value = serde_json::from_slice(&registration(&user, "vkid")).unwrap();
    request["redirect_uri"] = json!(ORIGIN);
    let first = social::vk_sign_in(&app, &body(request.clone()))
        .await
        .unwrap();
    let next = social::vk_sign_in(&app, &body(request)).await.unwrap();
    assert_eq!(first["state"], "signed_in");
    assert_eq!(
        first["profile"]["profile_id"],
        next["profile"]["profile_id"]
    );
    assert_ne!(first["session_token"], next["session_token"]);
}

#[tokio::test]
#[ignore = "requires isolated owner OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn one_tap_game_link_at_root_still_requires_the_native_signed_profile_link() {
    let app = app().await;
    let mut request: Value =
        serde_json::from_slice(&callback(&format!("user-{}", vk_user()))).unwrap();
    request["redirect_uri"] = json!(ORIGIN);
    let result = social::vk_sign_in(&app, &body(request)).await.unwrap();
    assert_eq!(result["state"], "unlinked");
    assert!(result.get("session_token").is_none());
    assert_eq!(result["social_ticket"].as_str().unwrap().len(), 64);
}
fn enrollment(key: &SigningKey) -> DeviceEnrollment {
    DeviceEnrollment {
        enrollment_id: crypto::random::<32>(),
        public_key: public(key),
        origin: ORIGIN.into(),
        label: "Desktop".into(),
        expires_at: (now() + 299).to_string(),
    }
}
fn signed(
    key: &SigningKey,
    e: &DeviceEnrollment,
    action: DeviceAction,
    target: Option<&str>,
) -> Vec<u8> {
    serde_json::to_vec(&SignedDeviceEnrollment {
        enrollment: e.clone(),
        action,
        target_profile: target.map(str::to_owned),
        recovery_code: None,
        signature: crypto::hex(&key.sign(&e.signing_bytes(action, target, None)).to_bytes()),
    })
    .unwrap()
}
fn link(active: &SigningKey, e: &DeviceEnrollment) -> Vec<u8> {
    let link = SocialLink::for_enrollment(e, public(active));
    serde_json::to_vec(&SignedSocialLink {
        signature: crypto::hex(&active.sign(&link.signing_bytes()).to_bytes()),
        link,
    })
    .unwrap()
}
/// A game asks to sign in: a fresh pending key, its enrollment and code.
async fn game_request(app: &App) -> (SigningKey, DeviceEnrollment, String) {
    let pending = random_key();
    let e = enrollment(&pending);
    let created = devices::create(app, &signed(&pending, &e, DeviceAction::Create, None))
        .await
        .unwrap();
    (pending, e, created["code"].as_str().unwrap().to_owned())
}
fn session_of(app: &App, token: &str) -> impl std::future::Future<Output = Session> {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
    let app = app.clone();
    async move { auth::session(&app, &headers).await.unwrap() }
}

#[tokio::test]
#[ignore = "requires isolated owner OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn website_registration_links_to_the_game_profile_only_after_game_confirmation() {
    let app = app().await;
    let user = vk_user();
    // The player has played: their install key owns a game profile.
    let active = random_key();
    let profile = app
        .career
        .authenticate(&public(&active), "Лиса")
        .await
        .unwrap();
    sqlx::query("UPDATE career_profiles SET rating=1234,matches_played=3,progression_xp=1000 WHERE profile_id=$1")
        .bind(&profile.profile_id).execute(&app.pool).await.unwrap();
    // Website first: VK sign-in of an unknown VK user registers, but signs nobody in.
    let first = social::vk_sign_in(&app, &callback(&format!("user-{user}")))
        .await
        .unwrap();
    assert_eq!(first["state"], "unlinked");
    assert_eq!(first["display_name"], "Иван П.");
    let ticket = first["social_ticket"].as_str().unwrap().to_owned();
    let named = social::ticket(&app, &body(json!({"social_ticket":ticket})))
        .await
        .unwrap();
    assert_eq!(named["linked"], false);
    // Nothing identifying is stored: only the keyed hash of the VK id.
    let stored: Vec<String> =
        sqlx::query_scalar("SELECT subject_hash FROM portal.social_identities")
            .fetch_all(&app.pool)
            .await
            .unwrap();
    assert!(stored.iter().all(|h| h.len() == 64 && !h.contains(&user)));

    // The game opens the browser with its enrollment code; the browser offers the sign-in.
    let (pending, e, code) = game_request(&app).await;
    let preview = social::enrollment(&app, &body(json!({"code":code,"social_ticket":ticket})))
        .await
        .unwrap();
    assert_eq!(preview["enrollment_id"], e.enrollment_id);
    assert_eq!(preview["public_key"], public(&pending));
    assert_eq!(preview["label"], "Desktop");
    let offers: i64 =
        sqlx::query_scalar("SELECT count(*) FROM portal.social_links WHERE enrollment_id=$1")
            .bind(&e.enrollment_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(offers, 0, "review does not offer or approve the identity");
    let attached = social::attach(&app, &body(json!({"code":code,"social_ticket":ticket})))
        .await
        .unwrap();
    assert_eq!(attached["status"], "social_pending");
    assert_eq!(attached["label"], "Desktop");
    // Attaching the same code again is idempotent; a different sign-in cannot take it.
    social::attach(&app, &body(json!({"code":code,"social_ticket":ticket})))
        .await
        .unwrap();
    let other = social::vk_sign_in(&app, &callback(&format!("user-{}", vk_user())))
        .await
        .unwrap();
    assert_eq!(
        social::attach(
            &app,
            &body(json!({"code":code,"social_ticket":other["social_ticket"]}))
        )
        .await
        .unwrap_err()
        .0,
        StatusCode::CONFLICT
    );
    // The game sees who asks, and nothing is linked yet.
    let status = devices::status(&app, &signed(&pending, &e, DeviceAction::Status, None))
        .await
        .unwrap();
    assert_eq!(status["state"], "social_pending");
    assert_eq!(status["social"]["display_name"], "Иван П.");
    assert_eq!(
        social::vk_sign_in(&app, &callback(&format!("user-{user}")))
            .await
            .unwrap()["state"],
        "unlinked"
    );

    // Only the active key's signature links; the pending key or a stranger cannot.
    assert_eq!(
        social::link(&app, &link(&random_key(), &e))
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
    let mut forged: SignedSocialLink = serde_json::from_slice(&link(&active, &e)).unwrap();
    forged.link.enrollment_key = public(&random_key());
    assert!(
        social::link(&app, &serde_json::to_vec(&forged).unwrap())
            .await
            .is_err()
    );
    let linked = social::link(&app, &link(&active, &e)).await.unwrap();
    assert_eq!(linked["state"], "linked");
    assert_eq!(linked["account_region"], shared::market::MARKET.key());
    assert_eq!(linked["profile_id"], profile.profile_id);
    let saved = app.career.profile(&profile.profile_id).await.unwrap();
    assert_eq!(
        (saved.rating, saved.matches_played, saved.progression_xp),
        (1234, 3, 1000)
    );
    // A lost reply is safe to retry; the code can no longer approve any device.
    assert_eq!(
        social::link(&app, &link(&active, &e)).await.unwrap()["state"],
        "linked"
    );
    assert!(
        devices::lookup(
            &app,
            &session_of(
                &app,
                social::vk_sign_in(&app, &callback(&format!("user-{user}")))
                    .await
                    .unwrap()["session_token"]
                    .as_str()
                    .unwrap()
            )
            .await,
            &body(json!({"code":code}))
        )
        .await
        .is_err()
    );

    // From now on VK signs the browser into that game account.
    let signed_in = social::vk_sign_in(&app, &callback(&format!("user-{user}")))
        .await
        .unwrap();
    assert_eq!(signed_in["state"], "signed_in");
    assert_eq!(signed_in["profile"]["profile_id"], profile.profile_id);
    let s = session_of(&app, signed_in["session_token"].as_str().unwrap()).await;
    assert_eq!(s.profile_id, profile.profile_id);
    let me = accounts::me(&app, &s).await.unwrap();
    assert_eq!(me["social"][0]["provider"], "vk");
    assert_eq!(me["social"][0]["method"], "ok_ru");
    // The old ticket names a linked identity and cannot be attached any more.
    let (_, _, code2) = game_request(&app).await;
    assert_eq!(
        social::attach(&app, &body(json!({"code":code2,"social_ticket":ticket})))
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
    // One VK account per profile: a second VK user cannot link to the same profile.
    let second = social::vk_sign_in(&app, &callback(&format!("user-{}", vk_user())))
        .await
        .unwrap();
    let (_, e3, code3) = game_request(&app).await;
    social::attach(
        &app,
        &body(json!({"code":code3,"social_ticket":second["social_ticket"]})),
    )
    .await
    .unwrap();
    let refused = social::link(&app, &link(&active, &e3)).await.unwrap_err();
    assert_eq!(
        (refused.0, refused.1),
        (StatusCode::CONFLICT, "profile_already_linked")
    );
}

#[tokio::test]
#[ignore = "requires isolated owner OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn vk_sign_in_on_a_new_device_enrolls_its_key_into_the_linked_account() {
    let app = app().await;
    let user = vk_user();
    let home = random_key();
    let profile = app
        .career
        .authenticate(&public(&home), "Волк")
        .await
        .unwrap();
    let ticket = social::vk_sign_in(&app, &callback(&format!("user-{user}")))
        .await
        .unwrap()["social_ticket"]
        .as_str()
        .unwrap()
        .to_owned();
    let (_, e, code) = game_request(&app).await;
    social::attach(&app, &body(json!({"code":code,"social_ticket":ticket})))
        .await
        .unwrap();
    social::link(&app, &link(&home, &e)).await.unwrap();

    // A second computer: a fresh install whose key has its own empty profile.
    let laptop = random_key();
    app.career
        .authenticate(&public(&laptop), "Guest")
        .await
        .unwrap();
    let (pending, e2, code2) = game_request(&app).await;
    let token = social::vk_sign_in(&app, &callback(&format!("user-{user}")))
        .await
        .unwrap()["session_token"]
        .as_str()
        .unwrap()
        .to_owned();
    let s = session_of(&app, &token).await;
    // The website approves the game's code as the signed-in account (existing device flow).
    let preview = devices::lookup(&app, &s, &body(json!({"code":code2})))
        .await
        .unwrap();
    assert_eq!(preview["public_key"], public(&pending));
    devices::approve(
        &app,
        &s,
        &body(json!({"enrollment_id":e2.enrollment_id,"public_key":e2.public_key})),
    )
    .await
    .unwrap();
    let status = devices::status(&app, &signed(&pending, &e2, DeviceAction::Status, None))
        .await
        .unwrap();
    assert_eq!(status["state"], "approved");
    assert_eq!(status["nickname"], profile.nickname);
    devices::complete(
        &app,
        &signed(
            &pending,
            &e2,
            DeviceAction::Complete,
            Some(&profile.profile_id),
        ),
    )
    .await
    .unwrap();
    assert!(
        app.career
            .key_is_active(&public(&pending), &profile.profile_id)
            .await
            .unwrap()
    );
    // Unlinking needs a fresh sign-in, then VK no longer reaches the account.
    social::unlink(&app, &s).await.unwrap();
    assert_eq!(
        social::vk_sign_in(&app, &callback(&format!("user-{user}")))
            .await
            .unwrap()["state"],
        "unlinked"
    );
}

#[tokio::test]
#[ignore = "requires isolated owner OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn vk_codes_are_verified_by_the_service_and_bound_to_the_portal_callback() {
    let app = app().await;
    let refused = social::vk_sign_in(&app, &callback("forged"))
        .await
        .unwrap_err();
    assert_eq!(
        (refused.0, refused.1),
        (StatusCode::BAD_GATEWAY, "provider_rejected")
    );
    let mut other_callback: Value = serde_json::from_slice(&callback("user-77")).unwrap();
    other_callback["redirect_uri"] = json!("https://evil.example/api/auth/vk/callback");
    assert_eq!(
        social::vk_sign_in(&app, &body(other_callback))
            .await
            .unwrap_err()
            .0,
        StatusCode::BAD_REQUEST
    );
    let mut unknown_method: Value = serde_json::from_slice(&callback("user-77")).unwrap();
    unknown_method["method"] = json!("google");
    assert!(
        social::vk_sign_in(&app, &body(unknown_method))
            .await
            .is_err()
    );
    let mut short_verifier: Value = serde_json::from_slice(&callback("user-77")).unwrap();
    short_verifier["code_verifier"] = json!("abc");
    assert!(
        social::vk_sign_in(&app, &body(short_verifier))
            .await
            .is_err()
    );
    // Unknown fields are refused rather than ignored.
    let mut extra: Value = serde_json::from_slice(&callback("user-77")).unwrap();
    extra["user_id"] = json!("1");
    assert!(social::vk_sign_in(&app, &body(extra)).await.is_err());
    // An unavailable provider is reported as such.
    let mut offline = app.clone();
    offline.social = Arc::new(
        social::SocialConfig::vk_fixture(CLIENT_ID, "http://127.0.0.1:9", [29; 32]).unwrap(),
    );
    let down = social::vk_sign_in(&offline, &callback("user-77"))
        .await
        .unwrap_err();
    assert_eq!(
        (down.0, down.1),
        (StatusCode::SERVICE_UNAVAILABLE, "provider_unavailable")
    );
    // Without configuration every social route is absent.
    let mut disabled = app.clone();
    disabled.social = Arc::new(social::SocialConfig::disabled());
    assert_eq!(
        social::vk_sign_in(&disabled, &callback("user-77"))
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
    // Expired tickets cannot be used and are cleaned up.
    let ticket = social::vk_sign_in(&app, &callback(&format!("user-{}", vk_user())))
        .await
        .unwrap()["social_ticket"]
        .as_str()
        .unwrap()
        .to_owned();
    sqlx::query("UPDATE portal.social_tickets SET expires_at=$1")
        .bind(now() - 1)
        .execute(&app.pool)
        .await
        .unwrap();
    assert!(
        social::ticket(&app, &body(json!({"social_ticket":ticket})))
            .await
            .is_err()
    );
    app.cleanup().await.unwrap();
    let left: i64 =
        sqlx::query_scalar("SELECT count(*) FROM portal.social_tickets WHERE expires_at<$1")
            .bind(now())
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
#[ignore = "requires isolated OMOBA_PORTAL_TEST_DATABASE_URL and OMOBA_PORTAL_ROLE_TEST_URL"]
async fn social_sign_in_works_with_the_restricted_portal_role() {
    let owner = app().await;
    let runtime = connect(
        &std::env::var("OMOBA_PORTAL_ROLE_TEST_URL").expect("isolated portal runtime role"),
        &vk_fixture().await,
    )
    .await;
    let active = random_key();
    let profile = owner
        .career
        .authenticate(&public(&active), "Restricted")
        .await
        .unwrap();
    let user = vk_user();
    let ticket = social::vk_sign_in(&runtime, &callback(&format!("user-{user}")))
        .await
        .unwrap()["social_ticket"]
        .as_str()
        .unwrap()
        .to_owned();
    let (_, e, code) = game_request(&runtime).await;
    social::attach(&runtime, &body(json!({"code":code,"social_ticket":ticket})))
        .await
        .unwrap();
    social::link(&runtime, &link(&active, &e)).await.unwrap();
    let signed_in = social::vk_sign_in(&runtime, &callback(&format!("user-{user}")))
        .await
        .unwrap();
    assert_eq!(signed_in["profile"]["profile_id"], profile.profile_id);
    let fresh = social::vk_sign_in(&runtime, &registration(&vk_user(), "mail_ru"))
        .await
        .unwrap();
    assert_eq!(fresh["state"], "signed_in");
    let registered = owner
        .career
        .profile(fresh["profile"]["profile_id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(
        (
            registered.rating,
            registered.matches_played,
            registered.progression_xp
        ),
        (1000, 0, 0)
    );
    for (provider, subject) in [
        ("other", crypto::random::<32>()),
        ("vk", "bad".into()),
        ("vk", crypto::random::<32>()),
    ] {
        let error = sqlx::query("SELECT portal.register_social_profile($1,$2)")
            .bind(provider)
            .bind(subject)
            .execute(&runtime.pool)
            .await
            .unwrap_err();
        assert_eq!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("22023")
        );
    }
    runtime.cleanup().await.unwrap();
    // The runtime role still cannot touch game progress or move keys.
    for query in [
        "UPDATE career_keys SET profile_id=profile_id WHERE false",
        "UPDATE career_profiles SET progression_xp=0 WHERE false",
        "INSERT INTO career_profiles(profile_id,nickname) SELECT repeat('9',64),'x' WHERE false",
    ] {
        let error = sqlx::query(query).execute(&runtime.pool).await.unwrap_err();
        assert_eq!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42501"),
            "{query}"
        );
    }
}

async fn http(
    app: &App,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    http_from(app, method, path, token, body, "127.0.0.41:42124").await
}
async fn http_from(
    app: &App,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Value,
    address: &str,
) -> (StatusCode, Value) {
    use axum::body::{Body, to_bytes};
    use tower::ServiceExt;
    let mut request = axum::http::Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let mut request = request
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    request.extensions_mut().insert(axum::extract::ConnectInfo(
        address.parse::<std::net::SocketAddr>().unwrap(),
    ));
    let response = router(app.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
#[ignore = "requires isolated owner OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn social_routes_are_served_over_http_and_unlink_needs_a_session() {
    let app = app().await;
    let active = random_key();
    let profile = app
        .career
        .authenticate(&public(&active), "Http")
        .await
        .unwrap();
    let user = vk_user();
    let (status, first) = http(
        &app,
        "POST",
        "/v1/auth/social/vk",
        None,
        serde_json::from_slice(&callback(&format!("user-{user}"))).unwrap(),
    )
    .await;
    assert_eq!(
        (status, &first["state"]),
        (StatusCode::OK, &json!("unlinked"))
    );
    let ticket = first["social_ticket"].clone();
    let (status, named) = http(
        &app,
        "POST",
        "/v1/auth/social/ticket",
        None,
        json!({"social_ticket":ticket}),
    )
    .await;
    assert_eq!(
        (status, &named["display_name"]),
        (StatusCode::OK, &json!("Иван П."))
    );
    let (_, e, code) = game_request(&app).await;
    let (status, _) = http(
        &app,
        "POST",
        "/v1/auth/social/attach",
        None,
        json!({"code":code,"social_ticket":ticket}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, linked) = http(
        &app,
        "POST",
        "/v1/auth/social/link",
        None,
        serde_json::from_slice(&link(&active, &e)).unwrap(),
    )
    .await;
    assert_eq!(
        (status, &linked["profile_id"]),
        (StatusCode::OK, &json!(profile.profile_id))
    );
    let (_, signed_in) = http(
        &app,
        "POST",
        "/v1/auth/social/vk",
        None,
        serde_json::from_slice(&callback(&format!("user-{user}"))).unwrap(),
    )
    .await;
    let token = signed_in["session_token"].as_str().unwrap().to_owned();
    let (status, me) = http(&app, "GET", "/v1/me", Some(&token), json!(null)).await;
    assert_eq!(
        (status, &me["social"][0]["provider"]),
        (StatusCode::OK, &json!("vk"))
    );
    let (status, _) = http(&app, "DELETE", "/v1/me/social/vk", None, json!(null)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, unlinked) = http(
        &app,
        "DELETE",
        "/v1/me/social/vk",
        Some(&token),
        json!(null),
    )
    .await;
    assert_eq!(
        (status, &unlinked["status"]),
        (StatusCode::OK, &json!("unlinked"))
    );
    let (status, _) = http(&app, "POST", "/v1/auth/social/unknown", None, json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires isolated owner OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn website_registration_creates_default_accounts_for_every_entry_method() {
    let app = app().await;
    let mut profiles = std::collections::HashSet::new();
    for method in ["vkid", "ok_ru", "mail_ru"] {
        let user = vk_user();
        let first = social::vk_sign_in(&app, &registration(&user, method))
            .await
            .unwrap();
        assert_eq!(first["state"], "signed_in");
        let profile = first["profile"]["profile_id"].as_str().unwrap();
        assert!(profiles.insert(profile.to_owned()));
        let saved = app.career.profile(profile).await.unwrap();
        assert_eq!(
            (
                saved.rating,
                saved.rated_matches,
                saved.matches_played,
                saved.wins,
                saved.losses,
                saved.progression_xp
            ),
            (1000, 0, 0, 0, 0, 0)
        );
        let keys: i64 = sqlx::query_scalar("SELECT count(*) FROM career_keys WHERE profile_id=$1")
            .bind(profile)
            .fetch_one(&app.pool)
            .await
            .unwrap();
        assert_eq!(
            keys, 0,
            "website signup does not invent a native device key"
        );
        let session = session_of(&app, first["session_token"].as_str().unwrap()).await;
        let me = accounts::me(&app, &session).await.unwrap();
        assert_eq!(me["social"][0]["method"], method);
        assert_eq!(me["profile"]["profile_id"], profile);
        let refusal = social::unlink(&app, &session).await.unwrap_err();
        assert_eq!(
            (refusal.0, refusal.1),
            (StatusCode::CONFLICT, "recovery_required_before_unlink")
        );

        // A normal repeat (without registration intent) signs into the same profile.
        let mut request: Value =
            serde_json::from_slice(&callback(&format!("user-{user}"))).unwrap();
        request["method"] = json!(method);
        let (status, repeated) = http_from(
            &app,
            "POST",
            "/v1/auth/social/vk",
            None,
            request,
            "127.0.0.42:42124",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(repeated["profile"]["profile_id"], profile);
        assert_ne!(repeated["session_token"], first["session_token"]);
        let linked = social::vk_sign_in(&app, &registration(&user, method))
            .await
            .unwrap();
        assert_eq!(linked["profile"]["profile_id"], profile);
    }
}

#[tokio::test]
#[ignore = "requires isolated owner OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn concurrent_website_registration_creates_one_profile_for_the_verified_subject() {
    let app = app().await;
    let user = vk_user();
    let mut attempts = Vec::new();
    for method in ["vkid", "mail_ru", "ok_ru", "vkid", "mail_ru", "ok_ru"] {
        let app = app.clone();
        let request = registration(&user, method);
        attempts.push(tokio::spawn(async move {
            social::vk_sign_in(&app, &request).await.unwrap()
        }));
    }
    let mut profile = None;
    let mut sessions = std::collections::HashSet::new();
    for attempt in attempts {
        let signed_in = attempt.await.unwrap();
        assert_eq!(signed_in["state"], "signed_in");
        let id = signed_in["profile"]["profile_id"]
            .as_str()
            .unwrap()
            .to_owned();
        if let Some(expected) = &profile {
            assert_eq!(&id, expected);
        } else {
            profile = Some(id);
        }
        assert!(sessions.insert(signed_in["session_token"].as_str().unwrap().to_owned()));
    }
    let linked: i64 =
        sqlx::query_scalar("SELECT count(*) FROM portal.social_identities WHERE profile_id=$1")
            .bind(profile.unwrap())
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(linked, 1);
}

#[tokio::test]
#[ignore = "requires isolated owner OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn token_exchange_requires_the_providers_returned_state() {
    let app = app().await;
    for state in ["missing-state", "wrong-state"] {
        let mut request: Value = serde_json::from_slice(&registration(&vk_user(), "vkid")).unwrap();
        request["state"] = json!(state);
        let error = social::vk_sign_in(&app, &body(request)).await.unwrap_err();
        assert_eq!(
            (error.0, error.1),
            (StatusCode::BAD_GATEWAY, "provider_rejected")
        );
    }
}

#[tokio::test]
#[ignore = "requires isolated owner OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn polling_more_than_ten_times_does_not_spend_device_completion_quota() {
    let app = app().await;
    let registered = social::vk_sign_in(&app, &registration(&vk_user(), "vkid"))
        .await
        .unwrap();
    let session = session_of(&app, registered["session_token"].as_str().unwrap()).await;
    let key = random_key();
    let e = enrollment(&key);
    let address = "127.0.0.43:42124";
    let (status, created) = http_from(
        &app,
        "POST",
        "/v1/auth/devices/create",
        None,
        serde_json::from_slice(&signed(&key, &e, DeviceAction::Create, None)).unwrap(),
        address,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    for _ in 0..15 {
        let (status, _) = http_from(
            &app,
            "POST",
            "/v1/auth/devices/status",
            None,
            serde_json::from_slice(&signed(&key, &e, DeviceAction::Status, None)).unwrap(),
            address,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
    let preview = devices::lookup(&app, &session, &body(json!({"code":created["code"]})))
        .await
        .unwrap();
    devices::approve(
        &app,
        &session,
        &body(json!({"enrollment_id":preview["enrollment_id"],"public_key":preview["public_key"]})),
    )
    .await
    .unwrap();
    let (status, completed) = http_from(
        &app,
        "POST",
        "/v1/auth/devices/complete",
        None,
        serde_json::from_slice(&signed(
            &key,
            &e,
            DeviceAction::Complete,
            Some(&session.profile_id),
        ))
        .unwrap(),
        address,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(completed["state"], "consumed");
    assert_eq!(completed["profile_id"], session.profile_id);
    assert!(
        app.career
            .key_is_active(&public(&key), &session.profile_id)
            .await
            .unwrap()
    );
    // The newly registered website account now has a native way back in.
    social::unlink(&app, &session).await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated owner OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn a_key_revoked_while_social_link_waits_for_the_profile_cannot_link() {
    let app = app().await;
    let active = random_key();
    let profile = app
        .career
        .authenticate(&public(&active), "Revocation race")
        .await
        .unwrap();
    let ticket = social::vk_sign_in(&app, &callback(&format!("user-{}", vk_user())))
        .await
        .unwrap();
    let (_, e, code) = game_request(&app).await;
    social::attach(
        &app,
        &body(json!({"code":code,"social_ticket":ticket["social_ticket"]})),
    )
    .await
    .unwrap();

    // Hold the same profile lock revocation owns, but let link observe the old
    // active key before it waits. This makes the TOCTOU ordering deterministic.
    let mut revocation = app.pool.begin().await.unwrap();
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *revocation)
        .await
        .unwrap();
    sqlx::query("SELECT profile_id FROM career_profiles WHERE profile_id=$1 FOR UPDATE")
        .bind(&profile.profile_id)
        .fetch_one(&mut *revocation)
        .await
        .unwrap();
    let proof = link(&active, &e);
    let linking_app = app.clone();
    let linking = tokio::spawn(async move { social::link(&linking_app, &proof).await });
    let mut waiting = false;
    for _ in 0..100 {
        waiting = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)) AND query LIKE 'SELECT nickname FROM career_profiles%')")
            .bind(blocker).fetch_one(&app.pool).await.unwrap();
        if waiting {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(
        waiting,
        "social link must reach the profile lock before revocation commits"
    );
    sqlx::query("UPDATE career_keys SET revoked_at=clock_timestamp() WHERE public_key=$1")
        .bind(public(&active))
        .execute(&mut *revocation)
        .await
        .unwrap();
    revocation.commit().await.unwrap();
    let error = linking.await.unwrap().unwrap_err();
    assert_eq!(error.0, StatusCode::NOT_FOUND);
    let linked: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM portal.social_identities WHERE profile_id=$1)",
    )
    .bind(profile.profile_id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert!(!linked);
}
