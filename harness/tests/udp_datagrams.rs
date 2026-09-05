//! Legacy JSON compatibility and the real host's UDP application-send limit.
//! Full 5v5 payloads above 8 KiB are covered through the native framed path in
//! framed_snapshots.rs. A local OS ceiling is measured, never raised or hidden.

use std::{
    net::UdpSocket,
    time::{Duration, Instant},
};

use harness::{Bot, Character, HeroClass, ServerProcess, Team};
use serde_json::Value;

const OLD_CLIENT_BOUNDARY: usize = 8 * 1024;
const IPV4_UDP_MAX_PAYLOAD_BYTES: usize = 65_507;

fn array_len(snapshot: &Value, field: &str) -> usize {
    snapshot[field]
        .as_array()
        .unwrap_or_else(|| panic!("snapshot field {field} should be an array"))
        .len()
}

fn host_legacy_send_ceiling() -> usize {
    let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
    receiver
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
    sender.connect(receiver.local_addr().unwrap()).unwrap();
    let mut received = vec![0; 65_536];
    let mut lower = 0;
    let mut upper = IPV4_UDP_MAX_PAYLOAD_BYTES + 1;
    while upper - lower > 1 {
        let size = lower + (upper - lower) / 2;
        let payload = vec![0x5a; size];
        match sender.send(&payload) {
            Ok(sent) => {
                assert_eq!(sent, size, "UDP send must never report a partial success");
                let len = receiver
                    .recv(&mut received)
                    .expect("successful loopback send must arrive");
                assert_eq!(
                    &received[..len],
                    payload.as_slice(),
                    "legacy datagram must remain whole"
                );
                lower = size;
            }
            Err(error) => {
                // EMSGSIZE on macOS, Linux or Winsock; any unrelated socket
                // failure is a test failure, not evidence of a size ceiling.
                assert!(
                    matches!(error.raw_os_error(), Some(40 | 90 | 10040)),
                    "unexpected error while probing UDP size {size}: {error}"
                );
                upper = size;
            }
        }
    }
    assert!(lower >= shared::transport::MAX_DATAGRAM_BYTES);
    lower
}

#[test]
fn host_legacy_datagram_ceiling_is_measured_without_changing_settings() {
    let ceiling = host_legacy_send_ceiling();
    eprintln!(
        "LEGACY_HOST_UDP_MEASUREMENT {}",
        serde_json::json!({
            "maximum_observed_send_bytes": ceiling,
            "ipv4_protocol_ceiling_bytes": IPV4_UDP_MAX_PAYLOAD_BYTES,
            "native_frame_budget_bytes": shared::transport::MAX_DATAGRAM_BYTES,
            "settings_changed": false,
            "scope": "loopback application-send ceiling; remote/Wi-Fi unverified",
        })
    );
}

#[test]
fn real_server_legacy_json_compatibility_survives_malformed_requests() {
    let host_ceiling = host_legacy_send_ceiling();
    let avatar = shared::avatar_roster()
        .iter()
        .max_by_key(|avatar| avatar.slug.len())
        .unwrap();
    let server = ServerProcess::spawn();
    let mut bots = [Bot::connect(server.addr()), Bot::connect(server.addr())];
    for (index, bot) in bots.iter().enumerate() {
        bot.join_with_cosmetics(
            if index == 0 { Team::Green } else { Team::Blue },
            Character::Ipfs,
            HeroClass::Warrior,
            Some(&avatar.slug),
            Some("cathedral-moth-bellringer"),
        );
    }
    let deadline = Instant::now() + Duration::from_secs(4);
    let (payload, snapshot) = loop {
        assert!(
            Instant::now() < deadline,
            "small legacy snapshot never arrived"
        );
        for bot in &bots {
            bot.ping();
        }
        let Some(payload) = bots[0].recv_raw_datagram(Instant::now() + Duration::from_millis(200))
        else {
            continue;
        };
        assert!(
            !payload.starts_with(b"OMB1"),
            "legacy peer did not opt in to framing"
        );
        let snapshot: Value =
            serde_json::from_slice(&payload).expect("complete legacy JSON snapshot");
        if snapshot["game_state"]["type"] == "running" && array_len(&snapshot, "players") == 2 {
            break (payload, snapshot);
        }
    };
    assert!(payload.len() <= host_ceiling);
    assert_eq!(array_len(&snapshot, "structures"), 8);
    assert_eq!(
        snapshot["protocol_version"],
        shared::protocol::PROTOCOL_VERSION
    );
    let expected_your_id = snapshot["your_id"].as_u64().unwrap();
    for player in snapshot["players"].as_array().unwrap() {
        assert_eq!(player["avatar"].as_str(), Some(avatar.slug.as_str()));
    }
    bots[0].send_raw(br#"{"type":"transform","x":"#);
    if host_ceiling > OLD_CLIENT_BOUNDARY {
        bots[0].send_raw(&vec![b' '; OLD_CLIENT_BOUNDARY + 1]);
    }
    let recovery_deadline = Instant::now() + Duration::from_secs(3);
    loop {
        assert!(
            Instant::now() < recovery_deadline,
            "malformed requests stopped legacy snapshot flow"
        );
        for bot in &bots {
            bot.ping();
        }
        let Some(bytes) = bots[0].recv_raw_datagram(Instant::now() + Duration::from_millis(200))
        else {
            continue;
        };
        let next: Value = serde_json::from_slice(&bytes).expect("whole recovery snapshot");
        if next["your_id"].as_u64() == Some(expected_your_id)
            && array_len(&next, "players") == 2
            && next["snapshot_tick"].as_u64() > snapshot["snapshot_tick"].as_u64()
        {
            assert_eq!(array_len(&next, "structures"), 8);
            break;
        }
    }
    eprintln!(
        "LEGACY_COMPATIBILITY_MEASUREMENT {}",
        serde_json::json!({
            "snapshot_bytes": payload.len(), "players":2,"structures":8,
            "maximum_observed_host_send_bytes":host_ceiling,
            "complete_json":true, "malformed_request_recovery":true,
            "full_5v5_payload_budget":"measured separately by framed_snapshots; payloads above host ceiling cannot use legacy UDP",
        })
    );
}

// Keep the original populated legacy regression wherever the host can deliver it.
// A real host-size failure remains a failure, and the ceiling probe explains it.
const POLL_BUDGET: Duration = Duration::from_secs(25);
const LONGEST_SPRITE_ID: &str = "cathedral-moth-bellringer";

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
