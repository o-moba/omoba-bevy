//! An isolated PostgreSQL database is mandatory; no in-memory substitute.
use axum::http::{HeaderMap, StatusCode};
use ed25519_dalek::{Signer, SigningKey};
use omoba_account_api::{accounts, auth, read, *};
use serde_json::{Value, json};
use shared::{HeroClass, career::*, map::Team, web_account::*};

#[tokio::test]
#[ignore = "requires isolated owner OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn legacy_v1_handles_migrate_atomically_without_changing_identity() {
    let url = std::env::var("OMOBA_PORTAL_TEST_DATABASE_URL").unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    let schema = format!("handles_fixture_{}", crypto::random::<8>());
    sqlx::raw_sql(&format!("CREATE SCHEMA {schema}; SET LOCAL search_path TO {schema}, public; CREATE TABLE career_schema_version(version INTEGER PRIMARY KEY);"))
        .execute(&mut *tx).await.unwrap();
    sqlx::raw_sql(include_str!(
        "../../server/migrations/postgres/001_career.sql"
    ))
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::raw_sql("INSERT INTO career_schema_version VALUES(1); INSERT INTO career_profiles(profile_id,nickname,rating) VALUES(repeat('a',64),'Лиса',1234),(repeat('b',64),'лиса',1100),(repeat('c',64),'Player',1000); INSERT INTO career_keys(public_key,profile_id) VALUES(repeat('d',64),repeat('a',64)); INSERT INTO career_friendships(low_id,high_id,requested_by,accepted) VALUES(repeat('a',64),repeat('b',64),repeat('a',64),true);")
        .execute(&mut *tx).await.unwrap();
    sqlx::raw_sql(include_str!(
        "../../server/migrations/postgres/002_player_handles.sql"
    ))
    .execute(&mut *tx)
    .await
    .unwrap();
    let names: Vec<String> =
        sqlx::query_scalar("SELECT nickname FROM career_profiles ORDER BY profile_id")
            .fetch_all(&mut *tx)
            .await
            .unwrap();
    assert!(names.iter().all(|n| normalize_player_handle(n).is_ok()));
    assert!(names[0].starts_with("Лиса#"));
    assert!(names[1].starts_with("лиса#"));
    assert_ne!(names[0].to_lowercase(), names[1].to_lowercase());
    assert!(!names[2].starts_with("Player#"));
    let (id, rating, accepted): (String, i32, bool) = sqlx::query_as("SELECT k.profile_id,p.rating,f.accepted FROM career_keys k JOIN career_profiles p USING(profile_id) JOIN career_friendships f ON f.low_id=p.profile_id")
        .fetch_one(&mut *tx).await.unwrap();
    assert_eq!((id, rating, accepted), ("a".repeat(64), 1234, true));
    let versions: Vec<i32> =
        sqlx::query_scalar("SELECT version FROM career_schema_version ORDER BY version")
            .fetch_all(&mut *tx)
            .await
            .unwrap();
    assert_eq!(versions, vec![1, 2]);
    // The schema and fixtures exist only in this transaction, including on failure.
    tx.rollback().await.unwrap();
}
async fn projected(app: &App, result_id: &str) {
    for _ in 0..100 {
        read::project(app, 256).await.unwrap();
        let ready: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM portal.projected_results WHERE result_id=$1)",
        )
        .bind(result_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
        if ready {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("projection did not complete for test result");
}

fn bytes(v: Value) -> Vec<u8> {
    serde_json::to_vec(&v).unwrap()
}
fn header(key: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert("idempotency-key", key.parse().unwrap());
    h
}
async fn pair(app: &App, key: &SigningKey) -> (Value, Vec<u8>, SignedWebPair) {
    let p = auth::create(app, b"{}").await.unwrap();
    let poll = bytes(json!({"pair_id":p["pair_id"],"poll_secret":p["poll_secret"]}));
    let found = auth::lookup(
        app,
        &bytes(json!({"code":p["code"],"public_key":crypto::hex(key.verifying_key().as_bytes())})),
    )
    .await
    .unwrap();
    let c: WebPairChallenge = serde_json::from_value(found["challenge"].clone()).unwrap();
    let proof = SignedWebPair {
        signature: crypto::hex(
            &key.sign(&c.signing_bytes(WebPairDecision::Approve))
                .to_bytes(),
        ),
        challenge: c,
        decision: WebPairDecision::Approve,
    };
    (p, poll, proof)
}
async fn login(app: &App, key: &SigningKey) -> (auth::Session, String) {
    let (_, poll, proof) = pair(app, key).await;
    auth::decide(
        app,
        &serde_json::to_vec(&proof).unwrap(),
        WebPairDecision::Approve,
    )
    .await
    .unwrap();
    let token = auth::complete(app, &poll).await.unwrap()["session_token"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut headers = HeaderMap::new();
    headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
    (auth::session(app, &headers).await.unwrap(), token)
}
fn result(a: &ProfileSummary, b: &ProfileSummary, id: &str, n: u64) -> MatchResult {
    let p = |profile: &ProfileSummary, player_id, team| ParticipantResult {
        player_id,
        is_bot: false,
        profile_id: Some(profile.profile_id.clone()),
        nickname: profile.nickname.clone(),
        team,
        hero_class: HeroClass::Ranger,
        character: "archer".into(),
        avatar: None,
        sprite_character: None,
        stats: MatchStats {
            kills: 3,
            assists: 2,
            damage_to_heroes: 1200.,
            final_level: 1,
            ..Default::default()
        },
        disconnected: false,
        rating: None,
        progression_xp_gained: 0,
    };
    MatchResult {
        result_id: id.into(),
        server_epoch: u64::MAX,
        match_id: n,
        started_at_ms: (now() as u64 - 120) * 1000,
        ended_at_ms: now() as u64 * 1000,
        duration_ms: 120000,
        map_profile: "verdant".into(),
        ruleset: "verdant-default-v1".into(),
        outcome: MatchOutcome::Completed,
        winner: Some(Team::Green),
        rated: true,
        unrated_reason: None,
        participants: vec![p(a, u64::MAX, Team::Green), p(b, 2, Team::Blue)],
        saved: false,
    }
}
async fn start(app: &App, r: &MatchResult) {
    let mut allocation = r.clone();
    allocation.ended_at_ms = 0;
    allocation.duration_ms = 0;
    allocation.winner = None;
    allocation.outcome = MatchOutcome::Interrupted;
    app.career.start(allocation).await.unwrap();
}
#[tokio::test]
#[ignore = "requires an isolated OMOBA_PORTAL_TEST_DATABASE_URL, never production"]
async fn portal_postgres_contract_and_security() {
    let url = std::env::var("OMOBA_PORTAL_TEST_DATABASE_URL").expect("isolated test DB URL");
    migrate(&url).await.unwrap();
    migrate(&url).await.unwrap();
    let app = App::connect(
        &url,
        Config {
            origin: "https://players.example".into(),
            secret: [9; 32],
            trust_loopback_proxy: false,
            release_file: None,
        },
    )
    .await
    .unwrap();
    let fresh = || {
        let mut seed = [0; 32];
        getrandom::fill(&mut seed).unwrap();
        SigningKey::from_bytes(&seed)
    };
    let key = fresh();
    let otherkey = fresh();
    let thirdkey = fresh();
    let a = app
        .career
        .authenticate(&crypto::hex(key.verifying_key().as_bytes()), "Алиса")
        .await
        .unwrap();
    let b = app
        .career
        .authenticate(&crypto::hex(otherkey.verifying_key().as_bytes()), "Bob")
        .await
        .unwrap();
    let c = app
        .career
        .authenticate(&crypto::hex(thirdkey.verifying_key().as_bytes()), "Carol")
        .await
        .unwrap();
    let baseline: i64 = sqlx::query_scalar("SELECT count(*) FROM career_profiles")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let (_, poll, proof) = pair(&app, &key).await;
    assert!(auth::complete(&app, &poll).await.is_err());
    let mut wrong = proof.clone();
    wrong.challenge.origin = "https://attacker.example".into();
    assert!(
        auth::decide(
            &app,
            &serde_json::to_vec(&wrong).unwrap(),
            WebPairDecision::Approve
        )
        .await
        .is_err()
    );
    assert!(
        auth::decide(
            &app,
            &serde_json::to_vec(&proof).unwrap(),
            WebPairDecision::Deny
        )
        .await
        .is_err()
    );
    auth::decide(
        &app,
        &serde_json::to_vec(&proof).unwrap(),
        WebPairDecision::Approve,
    )
    .await
    .unwrap();
    let (x, y) = tokio::join!(auth::complete(&app, &poll), auth::complete(&app, &poll));
    assert_eq!(x.as_ref().unwrap(), y.as_ref().unwrap());
    let token = x.unwrap()["session_token"].as_str().unwrap().to_owned();
    let mut h = HeaderMap::new();
    h.insert("authorization", format!("Bearer {token}").parse().unwrap());
    let sa = auth::session(&app, &h).await.unwrap();
    assert_eq!(sa.profile_id, a.profile_id);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM career_profiles")
            .fetch_one(&app.pool)
            .await
            .unwrap(),
        baseline
    );
    let (sb, _) = login(&app, &otherkey).await;
    let (sc, _) = login(&app, &thirdkey).await;
    let me = accounts::me(&app, &sa).await.unwrap();
    assert!(me["profile"]["progression_xp"].is_string());
    assert_eq!(me["settings"]["public_profile_enabled"], false);
    assert_eq!(
        accounts::player(&app, None, &a.profile_id)
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
    let request = bytes(json!({"target_profile_id":b.profile_id,"action":"request"}));
    let request_key = header("friend-request-000001");
    let (r1, r2) = tokio::join!(
        accounts::mutate(&app, &sa, "me/friend-actions", &request_key, &request),
        accounts::mutate(&app, &sa, "me/friend-actions", &request_key, &request)
    );
    assert_eq!(r1.unwrap(), r2.unwrap());
    let view = accounts::friends(&app, &sb).await.unwrap();
    assert_eq!(view["incoming"].as_array().unwrap().len(), 1);
    assert!(view["incoming"][0].get("presence").is_none());
    assert_eq!(
        accounts::player(&app, Some(&sb), &a.profile_id)
            .await
            .unwrap()["access"],
        "contact"
    );
    let accept = bytes(
        json!({"target_profile_id":a.profile_id,"action":"accept","expected_etag":view["incoming"][0]["etag"]}),
    );
    accounts::mutate(
        &app,
        &sb,
        "me/friend-actions",
        &header("friend-accept-000001"),
        &accept,
    )
    .await
    .unwrap();
    let f = accounts::friends(&app, &sa).await.unwrap();
    let old_etag = f["friends"][0]["etag"].clone();
    let remove =
        bytes(json!({"target_profile_id":b.profile_id,"action":"remove","expected_etag":old_etag}));
    let remove_key = header("friend-remove-000001");
    accounts::mutate(&app, &sa, "me/friend-actions", &remove_key, &remove)
        .await
        .unwrap();
    app.career
        .friend_action(&a.profile_id, &b.profile_id, FriendAction::Request)
        .await
        .unwrap();
    app.career
        .friend_action(&b.profile_id, &a.profile_id, FriendAction::Accept)
        .await
        .unwrap();
    accounts::mutate(&app, &sa, "me/friend-actions", &remove_key, &remove)
        .await
        .unwrap();
    assert_eq!(
        app.career
            .friends(&a.profile_id)
            .await
            .unwrap()
            .friends
            .len(),
        1
    );
    assert_eq!(
        accounts::mutate(
            &app,
            &sa,
            "me/friend-actions",
            &header("friend-remove-000002"),
            &remove
        )
        .await
        .unwrap_err()
        .0,
        StatusCode::CONFLICT
    );
    let rename = bytes(json!({"nickname":"Новый Ник"}));
    accounts::mutate(
        &app,
        &sa,
        "me/profile",
        &header("nickname-change-00001"),
        &rename,
    )
    .await
    .unwrap();
    assert_eq!(
        app.career.profile(&a.profile_id).await.unwrap().nickname,
        format!("Новый Ник#{}", a.nickname.rsplit_once('#').unwrap().1)
    );
    let bad = bytes(json!({"nickname":"Hacker","rating":5000}));
    assert!(
        accounts::mutate(
            &app,
            &sa,
            "me/profile",
            &header("nickname-change-00002"),
            &bad
        )
        .await
        .is_err()
    );
    let settings = bytes(
        json!({"public_profile_enabled":true,"public_stats_enabled":false,"discoverable":true,"locale":"en","timezone":"Asia/Shanghai","expected_version":"1"}),
    );
    accounts::mutate(
        &app,
        &sa,
        "me/settings",
        &header("privacy-change-000001"),
        &settings,
    )
    .await
    .unwrap();
    assert!(
        accounts::player(&app, None, &a.profile_id).await.unwrap()["profile"]
            .get("rating")
            .is_none()
    );
    assert!(
        accounts::search(&app, &sb, Some("query=%D0%9D%D0%BE"))
            .await
            .unwrap()["players"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["profile_id"] == a.profile_id)
    );
    // Older allocation settles after a newer result; both must eventually project exactly once.
    let suffix = crypto::random::<8>();
    let r = result(&a, &b, &format!("portal-{suffix}"), 1);
    start(&app, &r).await;
    let late = result(
        &c,
        &app.career
            .authenticate(&"12".repeat(32), "Dave")
            .await
            .unwrap(),
        &format!("late-{suffix}"),
        2,
    );
    start(&app, &late).await;
    let late_id = late.result_id.clone();
    app.career.settle(late).await.unwrap();
    projected(&app, &late_id).await;
    let settled = app.career.settle(r.clone()).await.unwrap();
    assert!(settled.saved);
    projected(&app, &r.result_id).await;
    read::project(&app, 128).await.unwrap();
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM portal.player_match_facts WHERE result_id=$1")
            .bind(&r.result_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(count, 2);
    let d = read::detail(&app, &sa, &r.result_id).await.unwrap();
    assert!(
        d["match"]["participants"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["player_id"].as_str() == Some("18446744073709551615"))
    );
    assert_eq!(d["match"]["server_epoch"], u64::MAX.to_string());
    assert_eq!(
        read::detail(&app, &sc, &r.result_id).await.unwrap_err().0,
        StatusCode::NOT_FOUND
    );
    let history = read::history(
        &app,
        &sa,
        Some("days=all&hero_class=ranger&rated=true&outcome=win&limit=1"),
    )
    .await
    .unwrap();
    assert_eq!(history["matches"][0]["result_id"], r.result_id);
    let stats = read::statistics(&app, &sa, Some("days=7")).await.unwrap();
    assert_eq!(stats["summary"]["damage_per_minute"], 600.);
    assert_eq!(stats["summary"]["kda"], 5.);
    assert_eq!(stats["pending_results"], 0);
    auth::logout(&app, &sa).await.unwrap();
    assert!(auth::session(&app, &h).await.is_err());
    assert!(auth::complete(&app, &poll).await.is_err());
    let unknown = SigningKey::from_bytes(&[99; 32]);
    let (_, _, proof) = pair(&app, &unknown).await;
    assert!(
        auth::decide(
            &app,
            &serde_json::to_vec(&proof).unwrap(),
            WebPairDecision::Approve
        )
        .await
        .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i32>("SELECT version FROM career_schema_version")
            .fetch_one(&app.pool)
            .await
            .unwrap(),
        1
    );
    println!(
        "PASS: real PostgreSQL migration, browser-bound pairing, concurrent completion, profile identity, strict writes, contact ACL, public opt-in, search, stale friendship protection, game/web synchronization, settlement, late projection, stats, u64, private match ACL and logout."
    );
}

#[tokio::test]
#[ignore = "requires the isolated local roles installed by ops/grants.sql"]
async fn runtime_roles_cannot_rewrite_game_rewards() {
    let url = std::env::var("OMOBA_PORTAL_ROLE_TEST_URL").expect("isolated limited-role URL");
    let app = App::connect(
        &url,
        Config {
            origin: "https://players.example".into(),
            secret: [2; 32],
            trust_loopback_proxy: false,
            release_file: None,
        },
    )
    .await
    .unwrap();
    for query in [
        "UPDATE career_profiles SET rating=5000 WHERE false",
        "UPDATE career_profiles SET progression_xp=999999 WHERE false",
        "UPDATE career_matches SET result='{}'::jsonb WHERE false",
        "INSERT INTO career_profiles(profile_id,nickname) VALUES(repeat('f',64),'Intruder')",
        "CREATE TABLE portal.unauthorized(id integer)",
        "UPDATE career_schema_version SET version=2 WHERE false",
    ] {
        let error = sqlx::query(query).execute(&app.pool).await.unwrap_err();
        assert_eq!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42501"),
            "{query}"
        );
    }
    sqlx::query("UPDATE career_profiles SET nickname=nickname WHERE false")
        .execute(&app.pool)
        .await
        .unwrap();
    read::project(&app, 128).await.unwrap();
    app.cleanup().await.unwrap();
    println!(
        "PASS: real restricted PostgreSQL role permits profile/portal operations and cannot write match results, rating, XP, profiles or schema."
    );
}

#[tokio::test]
#[ignore = "requires isolated OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn pairing_cancellation_expiry_idle_revoke_and_unrated_statistics() {
    let url = std::env::var("OMOBA_PORTAL_TEST_DATABASE_URL").unwrap();
    let app = App::connect(
        &url,
        Config {
            origin: "https://players.example".into(),
            secret: [33; 32],
            trust_loopback_proxy: false,
            release_file: None,
        },
    )
    .await
    .unwrap();
    let fresh = || {
        let mut seed = [0; 32];
        getrandom::fill(&mut seed).unwrap();
        SigningKey::from_bytes(&seed)
    };
    let key = fresh();
    let other = fresh();
    let a = app
        .career
        .authenticate(&crypto::hex(key.verifying_key().as_bytes()), "Session-test")
        .await
        .unwrap();
    let b = app
        .career
        .authenticate(
            &crypto::hex(other.verifying_key().as_bytes()),
            "Opponent-test",
        )
        .await
        .unwrap();
    let (p, poll, proof) = pair(&app, &key).await;
    let stolen = bytes(json!({"pair_id":p["pair_id"],"poll_secret":crypto::random::<32>()}));
    assert_eq!(
        auth::status(&app, &stolen).await.unwrap_err().0,
        StatusCode::NOT_FOUND
    );
    auth::cancel(&app, &poll).await.unwrap();
    assert_eq!(
        auth::decide(
            &app,
            &serde_json::to_vec(&proof).unwrap(),
            WebPairDecision::Approve
        )
        .await
        .unwrap_err()
        .0,
        StatusCode::CONFLICT
    );
    let (_, poll, mut proof) = pair(&app, &key).await;
    proof.decision = WebPairDecision::Deny;
    proof.signature = crypto::hex(
        &key.sign(&proof.challenge.signing_bytes(WebPairDecision::Deny))
            .to_bytes(),
    );
    auth::decide(
        &app,
        &serde_json::to_vec(&proof).unwrap(),
        WebPairDecision::Deny,
    )
    .await
    .unwrap();
    assert_eq!(auth::status(&app, &poll).await.unwrap()["state"], "denied");
    assert!(auth::complete(&app, &poll).await.is_err());
    let (p, poll, _) = pair(&app, &key).await;
    sqlx::query("UPDATE portal.web_pairings SET expires_at=$2 WHERE pair_id=$1")
        .bind(p["pair_id"].as_str().unwrap())
        .bind(now() - 1)
        .execute(&app.pool)
        .await
        .unwrap();
    assert_eq!(auth::status(&app, &poll).await.unwrap()["state"], "expired");
    assert!(auth::complete(&app, &poll).await.is_err());
    let (sa, token) = login(&app, &key).await;
    let (sa2, token2) = login(&app, &key).await;
    sqlx::query("UPDATE portal.web_sessions SET created_at=$2 WHERE session_id=$1")
        .bind(&sa.session_id)
        .bind(now() - 601)
        .execute(&app.pool)
        .await
        .unwrap();
    let mut h = HeaderMap::new();
    h.insert("authorization", format!("Bearer {token}").parse().unwrap());
    let stale = auth::session(&app, &h).await.unwrap();
    assert_eq!(
        auth::revoke(&app, &stale, &sa2.session_id)
            .await
            .unwrap_err()
            .1,
        "fresh_confirmation_required"
    );
    auth::revoke(&app, &sa2, &sa.session_id).await.unwrap();
    assert!(auth::session(&app, &h).await.is_err());
    let mut r = result(&a, &b, &format!("unrated-{}", crypto::random::<8>()), 1);
    r.rated = false;
    r.unrated_reason = Some("development".into());
    start(&app, &r).await;
    app.career.settle(r.clone()).await.unwrap();
    projected(&app, &r.result_id).await;
    let d = read::detail(&app, &sa2, &r.result_id).await.unwrap();
    assert!(
        d["match"]["participants"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["rating"].is_null())
    );
    let default_stats = read::statistics(&app, &sa2, Some("days=all"))
        .await
        .unwrap();
    assert_eq!(default_stats["summary"]["matches"], 0);
    assert!(default_stats["summary"]["win_rate"].is_null());
    let all = read::statistics(&app, &sa2, Some("days=all&rated=all"))
        .await
        .unwrap();
    assert_eq!(all["summary"]["matches"], 1);
    assert_eq!(all["rating"].as_array().unwrap().len(), 0);
    let interrupted_id = format!("interrupted-{}", crypto::random::<8>());
    r.result_id = interrupted_id.clone();
    r.match_id = 2;
    r.outcome = MatchOutcome::Interrupted;
    r.winner = None;
    r.unrated_reason = Some("server_interrupted".into());
    start(&app, &r).await;
    app.career.settle(r).await.unwrap();
    projected(&app, &interrupted_id).await;
    let history = read::history(&app, &sa2, Some("days=all&outcome=interrupted"))
        .await
        .unwrap();
    assert_eq!(history["matches"][0]["result_id"], interrupted_id);
    assert!(history["matches"][0]["won"].is_null());
    sqlx::query("UPDATE portal.web_sessions SET last_seen=$2 WHERE session_id=$1")
        .bind(&sa2.session_id)
        .bind(now() - 86401)
        .execute(&app.pool)
        .await
        .unwrap();
    let mut expired_headers = HeaderMap::new();
    expired_headers.insert("authorization", format!("Bearer {token2}").parse().unwrap());
    assert!(auth::session(&app, &expired_headers).await.is_err());
    println!(
        "PASS: cancellation, denial, expiry, browser binding, fresh reauthentication, revocation, empty rated statistics, unrated and interrupted receipts."
    );
}

#[tokio::test]
#[ignore = "requires isolated OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn bounded_sessions_and_cross_replica_statistics_lock() {
    let url = std::env::var("OMOBA_PORTAL_TEST_DATABASE_URL").unwrap();
    let app = App::connect(
        &url,
        Config {
            origin: "https://players.example".into(),
            secret: [3; 32],
            trust_loopback_proxy: false,
            release_file: None,
        },
    )
    .await
    .unwrap();
    let key = SigningKey::from_bytes(&crypto::decode::<32>(&crypto::random::<32>()).unwrap());
    app.career
        .authenticate(&crypto::hex(key.verifying_key().as_bytes()), "Bounded")
        .await
        .unwrap();
    let (old, old_token) = login(&app, &key).await;
    // Seed older sessions directly so this cap test does not evade pairing limits.
    sqlx::query("UPDATE portal.web_sessions SET created_at=$2 WHERE session_id=$1")
        .bind(&old.session_id)
        .bind(now() - 100)
        .execute(&app.pool)
        .await
        .unwrap();
    for _ in 0..19 {
        sqlx::query("INSERT INTO portal.web_sessions(session_id,profile_id,token_hash,created_at,last_seen,expires_at) VALUES($1,$2,$3,$4,$4,$5)")
            .bind(crypto::random::<32>()).bind(&old.profile_id).bind(crypto::random::<32>())
            .bind(now()-10).bind(now()+3600).execute(&app.pool).await.unwrap();
    }
    let (_, poll, proof) = pair(&app, &key).await;
    auth::decide(
        &app,
        &serde_json::to_vec(&proof).unwrap(),
        WebPairDecision::Approve,
    )
    .await
    .unwrap();
    assert_eq!(
        auth::complete(&app, &poll).await.unwrap_err().1,
        "session_limit"
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM portal.web_sessions WHERE profile_id=$1 AND NOT revoked AND expires_at>$2")
        .bind(&old.profile_id).bind(now()).fetch_one(&app.pool).await.unwrap();
    assert_eq!(count, 20);
    let mut h = HeaderMap::new();
    h.insert(
        "authorization",
        format!("Bearer {old_token}").parse().unwrap(),
    );
    assert!(auth::session(&app, &h).await.is_ok());
    auth::revoke(&app, &old, &old.session_id).await.unwrap();
    let (session, _) = login(&app, &key).await;
    let mut other_replica = app.pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,721946104))")
        .bind(&session.profile_id)
        .execute(&mut *other_replica)
        .await
        .unwrap();
    assert_eq!(
        read::statistics(&app, &session, Some("days=all"))
            .await
            .unwrap_err()
            .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    other_replica.rollback().await.unwrap();
    assert_eq!(
        read::statistics(&app, &session, Some("days=all"))
            .await
            .unwrap()["summary"]["matches"],
        0
    );
}

#[tokio::test]
#[ignore = "requires isolated OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn player_handles_defaults_uniqueness_rename_lookup_and_friend_identity() {
    let url = std::env::var("OMOBA_PORTAL_TEST_DATABASE_URL").unwrap();
    let app = App::connect(
        &url,
        Config {
            origin: "https://players.example".into(),
            secret: [4; 32],
            trust_loopback_proxy: false,
            release_file: None,
        },
    )
    .await
    .unwrap();
    let key_a = SigningKey::from_bytes(&crypto::decode::<32>(&crypto::random::<32>()).unwrap());
    let key_b = SigningKey::from_bytes(&crypto::decode::<32>(&crypto::random::<32>()).unwrap());
    let public_a = crypto::hex(key_a.verifying_key().as_bytes());
    let a = app.career.authenticate(&public_a, "Player").await.unwrap();
    let b = app
        .career
        .authenticate(&crypto::hex(key_b.verifying_key().as_bytes()), "Player")
        .await
        .unwrap();
    assert!(normalize_player_handle(&a.nickname).is_ok());
    assert!(!a.nickname.starts_with("Player#"));
    assert_eq!(
        app.career.authenticate(&public_a, "Ignored").await.unwrap(),
        a
    );
    let (sa, _) = login(&app, &key_a).await;
    let (sb, _) = login(&app, &key_b).await;
    app.career
        .friend_action(&a.profile_id, &b.profile_id, FriendAction::Request)
        .await
        .unwrap();
    app.career
        .friend_action(&b.profile_id, &a.profile_id, FriendAction::Accept)
        .await
        .unwrap();
    let handle = format!("Лиса{}#0042", crypto::random::<4>());
    accounts::mutate(
        &app,
        &sa,
        "me/profile",
        &header("handle-rename-operation-01"),
        &bytes(json!({"nickname":handle})),
    )
    .await
    .unwrap();
    let renamed = app.career.profile(&a.profile_id).await.unwrap();
    assert_eq!(renamed.nickname, handle);
    let found = app
        .career
        .lookup_player(&handle.to_uppercase())
        .await
        .unwrap();
    assert_eq!(found.profile_id, a.profile_id);
    let encoded = serde_json::to_value(&found).unwrap();
    assert_eq!(encoded.as_object().unwrap().len(), 2);
    assert!(encoded.get("rating").is_none());
    let conflict = accounts::mutate(
        &app,
        &sb,
        "me/profile",
        &header("handle-conflict-operation-01"),
        &bytes(json!({"nickname":handle.to_uppercase()})),
    )
    .await
    .unwrap_err();
    assert_eq!(conflict.1, "handle_taken");
    assert_eq!(
        app.career.profile(&b.profile_id).await.unwrap().nickname,
        b.nickname
    );
    assert_eq!(
        app.career.friends(&a.profile_id).await.unwrap().friends[0]
            .profile
            .profile_id,
        b.profile_id
    );
    let next = format!("Owl{}#9999", crypto::random::<4>());
    let (x, y) = tokio::join!(
        app.career.rename(&a.profile_id, &next),
        app.career.rename(&b.profile_id, &next)
    );
    assert_ne!(x.is_ok(), y.is_ok());
    if x.is_ok() {
        assert!(app.career.lookup_player(&handle).await.is_err());
    } else {
        assert_eq!(
            app.career.lookup_player(&handle).await.unwrap().profile_id,
            a.profile_id
        );
    }
    let winner = if x.is_ok() {
        &a.profile_id
    } else {
        &b.profile_id
    };
    assert_eq!(
        &app.career.lookup_player(&next).await.unwrap().profile_id,
        winner
    );
    assert_eq!(
        app.career.profile(&a.profile_id).await.unwrap().rating,
        a.rating
    );
    println!(
        "PASS: random default, stable account, exact Unicode/case-insensitive lookup, explicit tag edit, collision rollback, concurrent uniqueness and preserved friendship."
    );
}
