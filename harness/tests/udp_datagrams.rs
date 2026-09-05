//! Real-server UDP regression coverage for snapshots above the former 8 KiB
//! client receive boundary. These tests inspect complete datagrams rather than
//! relying only on an in-memory serde round trip.

use std::time::{Duration, Instant};

use harness::{Bot, Character, HeroClass, ServerProcess, Team};
use serde_json::Value;

const OLD_CLIENT_BOUNDARY: usize = 8 * 1024;
const IPV4_UDP_MAX_PAYLOAD_BYTES: usize = 65_507;
const POLL_BUDGET: Duration = Duration::from_secs(15);
const LONGEST_SPRITE_ID: &str = "cathedral-moth-bellringer";

fn array_len(snapshot: &Value, field: &str) -> usize {
    snapshot[field]
        .as_array()
        .unwrap_or_else(|| panic!("snapshot field {field} should be an array"))
        .len()
}

fn last_entity_id(snapshot: &Value, field: &str) -> Option<u64> {
    snapshot[field]
        .as_array()
        .and_then(|entities| entities.last())
        .and_then(|entity| entity["id"].as_u64())
}

#[test]
fn real_server_emits_complete_populated_snapshot_above_8_kib() {
    // Arena-synced avatars are local-only; use the same available roster
    // that the server validates instead of naming an optional download.
    let avatar = shared::avatar_roster()
        .iter()
        .max_by_key(|avatar| avatar.slug.len())
        .expect("the shipped avatar roster must not be empty");
    let server =
        ServerProcess::spawn_with_env(&[("OMOBA_MATCH_MODE", "release"), ("OMOBA_TEAM_SIZE", "5")]);
    let mut bots = (0..10)
        .map(|_| Bot::connect(server.addr()))
        .collect::<Vec<_>>();
    for bot in &bots {
        // Release mode assigns authoritative balanced teams. Longest valid
        // cosmetic ids make the real populated snapshot deterministically
        // cross the old receive boundary without inventing protocol fields.
        bot.join_with_cosmetics(
            Team::Green,
            Character::Ipfs,
            HeroClass::Warrior,
            Some(&avatar.slug),
            Some(LONGEST_SPRITE_ID),
        );
    }

    let deadline = Instant::now() + POLL_BUDGET;
    let mut qualifying: Option<(Vec<u8>, Value)> = None;
    while Instant::now() < deadline {
        for bot in &bots {
            bot.ping();
        }
        let Some(payload) = bots[0].recv_raw_datagram(Instant::now() + Duration::from_millis(250))
        else {
            continue;
        };
        let Ok(snapshot) = serde_json::from_slice::<Value>(&payload) else {
            continue;
        };
        let running = snapshot["game_state"]["type"].as_str() == Some("running");
        if running
            && array_len(&snapshot, "players") == 10
            && array_len(&snapshot, "structures") == 8
            && array_len(&snapshot, "minions") == 18
            && payload.len() > OLD_CLIENT_BOUNDARY
        {
            qualifying = Some((payload, snapshot));
            break;
        }
    }

    let (payload, snapshot) = qualifying.expect(
        "real server never produced a complete >8 KiB 5v5 snapshot with 8 structures and 18 minions",
    );
    eprintln!(
        "received complete real-server snapshot: {} bytes (runtime-dependent; asserted {} < bytes <= {}), 10 players, 8 structures, 18 minions",
        payload.len(),
        OLD_CLIENT_BOUNDARY,
        IPV4_UDP_MAX_PAYLOAD_BYTES,
    );
    assert!(payload.len() <= IPV4_UDP_MAX_PAYLOAD_BYTES);
    for player in snapshot["players"].as_array().expect("players array") {
        assert_eq!(player["avatar"].as_str(), Some(avatar.slug.as_str()));
    }
    assert_eq!(last_entity_id(&snapshot, "structures"), Some(8));
    assert_eq!(last_entity_id(&snapshot, "minions"), Some(18));

    // Both a malformed request and a whole oversized request are rejected;
    // neither may kill the server or remove/mutate the already joined player.
    bots[0].send_raw(br#"{"type":"transform","x":"#);
    bots[0].send_raw(&vec![b' '; OLD_CLIENT_BOUNDARY + 1]);

    let expected_your_id = snapshot["your_id"].as_u64().expect("your_id");
    let recovery_deadline = Instant::now() + Duration::from_secs(3);
    let recovered = loop {
        assert!(
            Instant::now() < recovery_deadline,
            "server stopped producing valid snapshots after malformed requests"
        );
        for bot in &bots {
            bot.ping();
        }
        let Some(next_payload) =
            bots[0].recv_raw_datagram(Instant::now() + Duration::from_millis(250))
        else {
            continue;
        };
        let Ok(next) = serde_json::from_slice::<Value>(&next_payload) else {
            continue;
        };
        if next["your_id"].as_u64() == Some(expected_your_id) && array_len(&next, "players") == 10 {
            break next;
        }
    };
    assert_eq!(array_len(&recovered, "structures"), 8);
    assert_eq!(array_len(&recovered, "minions"), 18);
}
