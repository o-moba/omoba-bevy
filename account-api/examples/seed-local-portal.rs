//! Public synthetic fixture for the browser test. Refuses non-local test databases.
use ed25519_dalek::SigningKey;
use omoba_account_api::{App, Config, crypto, now};
use shared::{HeroClass, career::*, map::Team};
use std::str::FromStr;
#[tokio::main]
async fn main() {
    let url = std::env::var("OMOBA_PORTAL_TEST_DATABASE_URL")
        .expect("Explicit test database URL required");
    let options = sqlx::postgres::PgConnectOptions::from_str(&url).expect("valid PostgreSQL URL");
    assert_eq!(
        options.get_host(),
        "127.0.0.1",
        "Synthetic keys are only for the disposable loopback fixture"
    );
    assert_eq!(options.get_port(), 55581, "Dedicated fixture port required");
    omoba_account_api::migrate(&url).await.unwrap();
    let app = App::connect(
        &url,
        Config {
            origin: "http://127.0.0.1:3010".into(),
            secret: [0; 32],
            trust_loopback_proxy: false,
            release_file: None,
        },
    )
    .await
    .unwrap();
    let mut profiles = Vec::new();
    for (seed, name) in [(7, "Portal Ranger"), (8, "Portal Mage")] {
        let key = SigningKey::from_bytes(&[seed; 32]);
        let p = app
            .career
            .authenticate(&crypto::hex(key.verifying_key().as_bytes()), name)
            .await
            .unwrap();
        app.career.rename(&p.profile_id, name).await.unwrap();
        profiles.push(p);
    }
    let result_id = format!("browser-fixture-{}", crypto::random::<8>());
    let participants = profiles
        .iter()
        .enumerate()
        .map(|(i, p)| ParticipantResult {
            player_id: i as u64 + 1,
            is_bot: false,
            profile_id: Some(p.profile_id.clone()),
            nickname: p.nickname.clone(),
            team: if i == 0 { Team::Green } else { Team::Blue },
            hero_class: if i == 0 {
                HeroClass::Ranger
            } else {
                HeroClass::Mage
            },
            character: "archer".into(),
            avatar: None,
            sprite_character: None,
            stats: MatchStats {
                kills: if i == 0 { 7 } else { 3 },
                deaths: if i == 0 { 3 } else { 7 },
                assists: 2,
                damage_to_heroes: if i == 0 { 18450. } else { 12880. },
                damage_to_structures: 3100.,
                damage_to_creeps: 22000.,
                damage_taken: 16000.,
                minion_last_hits: 89,
                jungle_last_hits: 12,
                structures_destroyed: 2,
                final_level: 12,
            },
            disconnected: false,
            rating: None,
            progression_xp_gained: 0,
        })
        .collect();
    let mut r = MatchResult {
        result_id: result_id.clone(),
        server_epoch: 1,
        match_id: now() as u64,
        started_at_ms: (now() as u64 - 900) * 1000,
        ended_at_ms: now() as u64 * 1000,
        duration_ms: 900000,
        map_profile: "verdant".into(),
        ruleset: "verdant-default-v1".into(),
        outcome: MatchOutcome::Completed,
        winner: Some(Team::Green),
        rated: true,
        unrated_reason: None,
        participants,
        saved: false,
    };
    let mut allocation = r.clone();
    allocation.outcome = MatchOutcome::Interrupted;
    allocation.winner = None;
    allocation.ended_at_ms = 0;
    allocation.duration_ms = 0;
    app.career.start(allocation).await.unwrap();
    r = app.career.settle(r).await.unwrap();
    assert!(r.saved);
    println!(
        "{}",
        serde_json::json!({"synthetic":true,"profile_id":profiles[0].profile_id,"result_id":result_id,"saved":true})
    );
    // Leave projection to the running API worker, so freshness is measurable.
}
