//! Synthetic prior public completions for disposable process-test identities.
//! Never linked into the server. No live database or client-supplied count can
//! use this helper: it accepts only an explicit loopback omoba_test database.
#[path = "../../harness/src/database.rs"]
mod test_database;

use omoba_career_store::career_store::CareerStore;
use shared::{
    career::{MatchOutcome, MatchResult, MatchStats, ParticipantResult},
    map::Team,
};
use std::io::{self, Read};

fn main() -> Result<(), String> {
    let database = test_database::test_database_url()?;
    let mut input = String::new();
    io::stdin()
        .take(1024 * 1024)
        .read_to_string(&mut input)
        .map_err(|_| "fixture input unavailable")?;
    let input: serde_json::Value =
        serde_json::from_str(&input).map_err(|_| "invalid fixture input")?;
    let ids = input["profiles"]
        .as_array()
        .ok_or("profiles array required")?;
    if ids.is_empty() || ids.len() > 1000 {
        return Err("require 1..1000 explicit profiles".into());
    }
    let target = input["target"].as_u64().ok_or("explicit target required")?;
    if target > 10 {
        return Err("fixture target must be 0..10".into());
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "fixture runtime unavailable")?;
    rt.block_on(async {
        let store = CareerStore::connect_runtime(&database).await?;
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|_| "clock unavailable")?.as_nanos();
        let mut completed = 0;
        for id in ids {
            let id = id.as_str().ok_or("profile identifier required")?;
            let profile = store.profile(id).await?;
            for n in u64::from(profile.completed_public_matches)..target {
                let participants = (0..10).map(|i| ParticipantResult {
                    input_kind: Default::default(),
                    player_id: i + 1,
                    profile_id: (i == 0).then(|| id.to_owned()),
                    nickname: if i == 0 { profile.nickname.clone() } else { format!("FixtureBot{i}") },
                    is_bot: i != 0,
                    team: if i < 5 { Team::Green } else { Team::Blue },
                    hero_class: shared::HeroClass::Ranger,
                    character: "archer".into(), avatar: None, sprite_character: None,
                    stats: MatchStats { final_level: 1, ..Default::default() },
                    disconnected: false, participation: None, rating: None, progression_xp_gained: 0,
                }).collect();
                let terminal = MatchResult {
                    result_id: format!("synthetic-public-{stamp}-{id}-{n}"),
                    server_epoch: 1, match_id: n + 1, started_at_ms: 1000,
                    ended_at_ms: 2000, duration_ms: 1000, gameplay_started: true,
                    map_profile: shared::map::ResolvedMap::default().map_profile,
                    ruleset: "public-casual-v1".into(), outcome: MatchOutcome::Completed,
                    winner: Some(Team::Green), rated: false,
                    unrated_reason: Some("allocated_bots".into()), end_reason: None,
                    participants, saved: false,
                };
                let mut start = terminal.clone();
                start.outcome = MatchOutcome::Interrupted; start.winner = None;
                start.duration_ms = 0; start.ended_at_ms = 0;
                store.start(start).await?;
                store.settle(terminal).await?;
                completed += 1;
            }
            if u64::from(store.profile(id).await?.completed_public_matches) < target {
                return Err("fixture settlement did not produce qualifying progress".into());
            }
        }
        println!("{}", serde_json::json!({"method":"synthetic_public_casual_store_receipts", "profiles":ids.len(),"target":target,"receipts_created":completed,"real_games_played":0}));
        Ok(())
    })
}
