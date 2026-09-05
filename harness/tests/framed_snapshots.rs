//! Actual release 5v5 traffic through the bounded native snapshot transport.
//! Build the current server before this test; do not use a stale prebuilt binary.

use harness::{Bot, Character, HeroClass, ServerPacket, ServerProcess, Team};
use serde_json::Value;
use shared::protocol::{PROTOCOL_VERSION, SnapshotMeta, SnapshotOrder};
use shared::transport::{MAX_DATAGRAM_BYTES, SnapshotAssembler};
use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

const OLD_CLIENT_BOUNDARY: usize = 8 * 1024;
// Release formation adds a 5s countdown; the first wave is 10s after Running.
const FIRST_WAVE_BUDGET: Duration = Duration::from_secs(25);

fn array<'a>(snapshot: &'a Value, name: &str) -> &'a [Value] {
    snapshot[name].as_array().expect("snapshot array field")
}

fn keys(value: &Value) -> BTreeSet<&str> {
    value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect()
}

#[test]
fn real_release_5v5_snapshots_fit_framed_datagrams_and_keep_complete_ordered_schema() {
    let avatar = shared::avatar_roster()
        .iter()
        .max_by_key(|avatar| avatar.slug.len())
        .expect("shipped avatar roster");
    let server =
        ServerProcess::spawn_with_env(&[("OMOBA_MATCH_MODE", "release"), ("OMOBA_TEAM_SIZE", "5")]);
    // Capture a real legacy packet before upgrading this endpoint. The first
    // roster member fits host legacy limits, even when a complete 5v5 does not.
    let mut observer = Bot::connect(server.addr());
    observer.join_with_cosmetics(
        Team::Green,
        Character::Ipfs,
        HeroClass::Warrior,
        Some(&avatar.slug),
        Some("cathedral-moth-bellringer"),
    );
    let legacy_deadline = Instant::now() + Duration::from_secs(3);
    let legacy = loop {
        assert!(
            Instant::now() < legacy_deadline,
            "legacy admission snapshot missing"
        );
        observer.ping();
        let Some(bytes) = observer.recv_raw_datagram(legacy_deadline) else {
            continue;
        };
        let snapshot: Value = serde_json::from_slice(&bytes).expect("whole legacy JSON");
        if array(&snapshot, "players").len() == 1 {
            break snapshot;
        }
    };
    let legacy_own_id = legacy["your_id"].as_u64().unwrap();
    let hello = serde_json::to_vec(&serde_json::json!({
        "type": "hello", "protocol_version": PROTOCOL_VERSION,
    }))
    .unwrap();
    observer.send_raw(&hello);
    let mut bots = (0..9)
        .map(|_| Bot::connect_framed(server.addr()))
        .collect::<Vec<_>>();
    for bot in &bots {
        bot.join_with_cosmetics(
            Team::Green,
            Character::Ipfs,
            HeroClass::Warrior,
            Some(&avatar.slug),
            Some("cathedral-moth-bellringer"),
        );
    }
    let deadline = Instant::now() + FIRST_WAVE_BUDGET;
    let mut assembler = SnapshotAssembler::default();
    let mut order = SnapshotOrder::default();
    let mut frames = HashMap::<(u64, u64), Vec<Vec<u8>>>::new();
    let mut maximum_datagram = 0;
    let mut completed = 0;
    let mut last_meta: Option<SnapshotMeta> = None;
    let (payload, snapshot, actual_frames) = loop {
        assert!(
            Instant::now() < deadline,
            "no complete populated 5v5 snapshot after release countdown and first wave; completed={completed}"
        );
        observer.send_raw(&hello);
        for bot in &bots {
            bot.ping();
        }
        let Some(bytes) = observer.recv_raw_datagram(Instant::now() + Duration::from_millis(200))
        else {
            continue;
        };
        if !bytes.starts_with(b"OMB1") {
            // Only previously queued legacy packets are legal during negotiation.
            assert_eq!(completed, 0, "legacy packet after native negotiation");
            continue;
        }
        maximum_datagram = maximum_datagram.max(bytes.len());
        assert!(bytes.len() <= MAX_DATAGRAM_BYTES);
        assert!(bytes.len() >= 30, "complete framing header");
        let epoch = u64::from_le_bytes(bytes[6..14].try_into().unwrap());
        let tick = u64::from_le_bytes(bytes[14..22].try_into().unwrap());
        frames.entry((epoch, tick)).or_default().push(bytes.clone());
        let Some(payload) = assembler
            .push(&bytes, Instant::now())
            .expect("valid actual server framing")
        else {
            continue;
        };
        let snapshot: Value =
            serde_json::from_slice(&payload).expect("whole reconstructed snapshot JSON");
        let typed: ServerPacket = serde_json::from_slice(&payload)
            .expect("harness gameplay mirror accepts native schema");
        let meta = typed.meta();
        assert_eq!((meta.server_epoch, meta.snapshot_tick), (epoch, tick));
        assert!(
            order.accept(meta),
            "complete live snapshots must advance authoritative order"
        );
        if let Some(previous) = last_meta {
            assert_eq!(meta.server_epoch, previous.server_epoch);
            assert_eq!(meta.match_id, previous.match_id);
            assert!(meta.snapshot_tick > previous.snapshot_tick);
        }
        last_meta = Some(meta);
        completed += 1;
        let actual_frames = frames.remove(&(epoch, tick)).unwrap();
        frames.retain(|(_, old_tick), _| *old_tick > tick);
        if snapshot["game_state"]["type"] == "running"
            && array(&snapshot, "players").len() == 10
            && array(&snapshot, "structures").len() == 8
            && array(&snapshot, "minions").len() == 18
        {
            break (payload, snapshot, actual_frames);
        }
    };
    assert!(payload.len() > OLD_CLIENT_BOUNDARY);
    assert_eq!(
        keys(&snapshot),
        keys(&legacy),
        "framing must preserve all legacy gameplay fields"
    );
    assert_eq!(snapshot["your_id"].as_u64(), Some(legacy_own_id));
    let player_keys = keys(&array(&legacy, "players")[0]);
    let mut teams = HashMap::new();
    for player in array(&snapshot, "players") {
        assert_eq!(keys(player), player_keys);
        assert_eq!(player["avatar"], avatar.slug.as_str());
        assert_eq!(player["sprite_character"], "cathedral-moth-bellringer");
        *teams.entry(player["team"].as_str().unwrap()).or_insert(0) += 1;
    }
    assert_eq!(teams.get("green"), Some(&5));
    assert_eq!(teams.get("blue"), Some(&5));
    assert_eq!(array(&snapshot, "minions").last().unwrap()["id"], 18);
    assert_eq!(array(&snapshot, "structures").last().unwrap()["id"], 8);
    for field in ["neutrals", "projectiles", "team_buffs"] {
        let _ = array(&snapshot, field);
    }

    // Feed the actual captured wire packets backwards with duplicates. Only
    // the final missing fragment can produce JSON, with byte-for-byte identity.
    let mut reordered = SnapshotAssembler::default();
    let mut reconstructed = None;
    for (index, frame) in actual_frames.iter().rev().enumerate() {
        let result = reordered.push(frame, Instant::now()).unwrap();
        assert_eq!(result.is_some(), index + 1 == actual_frames.len());
        if result.is_some() {
            reconstructed = result;
        }
        assert!(reordered.push(frame, Instant::now()).unwrap().is_none());
    }
    assert_eq!(reconstructed.as_ref(), Some(&payload));
    assert!(
        !order.accept(last_meta.unwrap()),
        "duplicated complete snapshot cannot apply twice"
    );
    let mut older = last_meta.unwrap();
    older.snapshot_tick -= 1;
    assert!(
        !order.accept(older),
        "delayed complete snapshot cannot roll state back"
    );

    // Exercise the production harness decoder independently of the raw collector.
    // This peer has intentionally been unread since formation, so drain its
    // socket backlog before requiring the same populated tick origin.
    let typed_deadline = Instant::now() + Duration::from_secs(3);
    let typed = loop {
        assert!(
            Instant::now() < typed_deadline,
            "framed decoder never caught up to first wave"
        );
        for bot in &bots {
            bot.ping();
        }
        if let Some(packet) = bots[0].recv_snapshot(Instant::now() + Duration::from_millis(250))
            && packet.meta().snapshot_tick >= last_meta.unwrap().snapshot_tick
            && packet.minions().len() == 18
        {
            break packet;
        }
    };
    assert_eq!(typed.players().len(), 10);
    assert_eq!(typed.structures().len(), 8);
    assert_eq!(typed.minions().len(), 18);
    eprintln!(
        "FRAMED_5V5_MEASUREMENT {}",
        serde_json::json!({
            "reconstructed_bytes":payload.len(), "maximum_datagram_bytes":maximum_datagram,
            "populated_fragment_count":actual_frames.len(), "completed_ordered_snapshots":completed,
            "players":10,"structures":8,"minions":18,"server_epoch":last_meta.unwrap().server_epoch,
            "match_id":last_meta.unwrap().match_id,"snapshot_tick":last_meta.unwrap().snapshot_tick,
            "transport":"loopback; remote/Wi-Fi unverified",
        })
    );
}
