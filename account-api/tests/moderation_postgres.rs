//! Moderation (LR-15) against an isolated test database: the operator commands,
//! the account ban on website sessions and sign-ins, and the name lock.
use axum::http::{HeaderMap, StatusCode};
use ed25519_dalek::{Signer, SigningKey};
use omoba_account_api::{accounts, auth, crypto, moderation, *};
use omoba_career_store::career_store::CareerStore;
use serde_json::json;
use shared::web_account::*;

fn random_key() -> SigningKey {
    let mut seed = [0; 32];
    getrandom::fill(&mut seed).unwrap();
    SigningKey::from_bytes(&seed)
}
fn config() -> Config {
    Config {
        origin: "https://players.example".into(),
        secret: [23; 32],
        trust_loopback_proxy: false,
        release_file: None,
    }
}
fn bytes(v: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&v).unwrap()
}
/// Pairs a browser with `key` and approves it; returns the poll body for
/// completion, or the approval's error.
async fn approve(app: &App, key: &SigningKey) -> std::result::Result<Vec<u8>, Error> {
    let p = auth::create(app, b"{}").await.unwrap();
    let poll = bytes(json!({"pair_id":p["pair_id"],"poll_secret":p["poll_secret"]}));
    let found = auth::lookup(
        app,
        &bytes(json!({"code":p["code"],"public_key":crypto::hex(key.verifying_key().as_bytes())})),
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
    .await?;
    Ok(poll)
}
fn bearer(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
    headers
}
fn args(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| (*w).to_owned()).collect()
}
async fn operator(store: &CareerStore, words: &[&str]) -> String {
    moderation::run(store, moderation::parse(&args(words)).unwrap())
        .await
        .unwrap()
}
fn code(result: std::result::Result<impl std::fmt::Debug, Error>) -> &'static str {
    result.unwrap_err().1
}

#[tokio::test]
#[ignore = "requires isolated OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn sanctions_suspend_website_sessions_and_lock_renames_until_lifted() {
    let url = std::env::var("OMOBA_PORTAL_TEST_DATABASE_URL").unwrap();
    migrate(&url).await.unwrap();
    let app = App::connect(&url, config()).await.unwrap();
    let key = random_key();
    let profile = app
        .career
        .authenticate(&crypto::hex(key.verifying_key().as_bytes()), "Rowdy")
        .await
        .unwrap();
    let poll = approve(&app, &key).await.unwrap();
    let token = auth::complete(&app, &poll).await.unwrap()["session_token"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(auth::session(&app, &bearer(&token)).await.is_ok());

    // An account ban: the session is refused, and no new sign-in is approved.
    let applied = operator(
        &app.career,
        &[
            "sanction",
            "add",
            &profile.nickname,
            "account_ban",
            "--for",
            "7d",
            "--reason",
            "Cheating",
            "--by",
            "ops.anna",
        ],
    )
    .await;
    assert!(applied.contains("account_ban"), "{applied}");
    let id = applied
        .split_once('#')
        .and_then(|(_, rest)| rest.split_once(' '))
        .map(|(id, _)| id.to_owned())
        .unwrap();
    let refused = auth::session(&app, &bearer(&token)).await;
    assert_eq!(
        refused.as_ref().err().map(|e| e.0),
        Some(StatusCode::FORBIDDEN)
    );
    assert_eq!(code(refused.map(|_| ())), "account_banned");
    assert_eq!(code(approve(&app, &key).await), "account_banned");
    let listed = operator(&app.career, &["sanction", "list", &profile.profile_id]).await;
    assert!(
        listed.contains("1 active sanction(s)") && listed.contains("Cheating"),
        "{listed}"
    );

    // Lifted: the same session works again, with no restart and no new sign-in.
    let lifted = operator(
        &app.career,
        &[
            "sanction",
            "lift",
            &id,
            "--reason",
            "Appeal",
            "--by",
            "ops.boris",
        ],
    )
    .await;
    assert!(lifted.contains(&format!("Lifted #{id}")), "{lifted}");
    let session = auth::session(&app, &bearer(&token)).await.unwrap();

    // A name reset renames to Player#NNNN and locks the website rename.
    let renamed = operator(
        &app.career,
        &[
            "rename",
            &profile.nickname,
            "--for",
            "1d",
            "--reason",
            "Offensive name",
            "--by",
            "ops.anna",
        ],
    )
    .await;
    assert!(renamed.contains("-> Player#"), "{renamed}");
    let current = app.career.profile(&profile.profile_id).await.unwrap();
    assert!(current.nickname.starts_with("Player#"));
    let mut headers = HeaderMap::new();
    headers.insert("idempotency-key", "moderation-rename-0001".parse().unwrap());
    assert_eq!(
        code(
            accounts::mutate(
                &app,
                &session,
                "me/profile",
                &headers,
                &bytes(json!({"nickname":"Rowdy"})),
            )
            .await
        ),
        "name_locked"
    );
    assert_eq!(
        app.career.profile(&profile.profile_id).await.unwrap(),
        current
    );

    // The lookup shows the keys, every sanction and the audit trail.
    let lookup = operator(&app.career, &["player", &current.nickname]).await;
    for expected in [
        "device keys:",
        "Original device",
        "account_ban",
        "lifted",
        "name_reset",
        "sanction_added",
        "sanction_lifted",
        "ops.boris",
    ] {
        assert!(lookup.contains(expected), "{expected} in {lookup}");
    }

    // The chat switch.
    assert!(
        operator(&app.career, &["chat"])
            .await
            .contains("Chat is on")
    );
    operator(
        &app.career,
        &["chat", "off", "--reason", "Raid", "--by", "ops.anna"],
    )
    .await;
    assert!(
        operator(&app.career, &["chat", "status"])
            .await
            .contains("Chat is off")
    );
    operator(
        &app.career,
        &["chat", "on", "--reason", "Over", "--by", "ops.anna"],
    )
    .await;
    assert!(app.career.chat_enabled().await.unwrap());
    // An unknown player is an error, not a panic.
    let unknown = moderation::run(
        &app.career,
        moderation::parse(&args(&["player", "Nobody#0000"])).unwrap(),
    )
    .await;
    assert!(unknown.is_err());
}

#[tokio::test]
#[ignore = "requires the isolated local roles installed by ops/grants.sql"]
async fn the_portal_role_moderates_but_cannot_erase_the_trail() {
    let owner = std::env::var("OMOBA_PORTAL_TEST_DATABASE_URL").unwrap();
    migrate(&owner).await.unwrap();
    let app = App::connect(&owner, config()).await.unwrap();
    let key = random_key();
    let profile = app
        .career
        .authenticate(&crypto::hex(key.verifying_key().as_bytes()), "Roled")
        .await
        .unwrap();
    let role = CareerStore::connect_runtime(
        &std::env::var("OMOBA_PORTAL_ROLE_TEST_URL").expect("isolated limited-role URL"),
    )
    .await
    .unwrap();
    let applied = operator(
        &role,
        &[
            "sanction",
            "add",
            &profile.profile_id,
            "chat_mute",
            "--for",
            "30m",
            "--reason",
            "Spam",
            "--by",
            "ops.anna",
        ],
    )
    .await;
    let id = applied
        .split_once('#')
        .and_then(|(_, rest)| rest.split_once(' '))
        .map(|(id, _)| id.to_owned())
        .unwrap();
    operator(
        &role,
        &[
            "rename",
            &profile.profile_id,
            "--permanent",
            "--reason",
            "Slur",
            "--by",
            "ops.anna",
        ],
    )
    .await;
    operator(
        &role,
        &[
            "sanction", "lift", &id, "--reason", "Mistake", "--by", "ops.anna",
        ],
    )
    .await;
    operator(
        &role,
        &["chat", "off", "--reason", "Test", "--by", "ops.anna"],
    )
    .await;
    operator(
        &role,
        &["chat", "on", "--reason", "Test", "--by", "ops.anna"],
    )
    .await;
    let portal = App::connect(
        &std::env::var("OMOBA_PORTAL_ROLE_TEST_URL").unwrap(),
        config(),
    )
    .await
    .unwrap();
    for statement in [
        "DELETE FROM career_sanctions WHERE false",
        "UPDATE career_sanctions SET reason='rewritten' WHERE false",
        "UPDATE career_sanctions SET expires_at=NULL WHERE false",
        "DELETE FROM career_moderation_events WHERE false",
        "UPDATE career_moderation_events SET actor='x' WHERE false",
        "DELETE FROM career_chat_switch WHERE false",
    ] {
        let error = sqlx::query(statement)
            .execute(&portal.pool)
            .await
            .unwrap_err();
        assert_eq!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42501"),
            "{statement}"
        );
    }
}
