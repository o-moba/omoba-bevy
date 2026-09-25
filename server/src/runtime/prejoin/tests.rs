//! O4: the standalone pre-join table is bounded and unverified endpoints get
//! one small status reply per datagram instead of the world.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use serde_json::Value;
use shared::HeroClass;
use shared::map::Team;
use shared::wire::{CharacterChoice, ClientPacket};

use super::{MAX_PREJOIN_ENDPOINTS, PREJOIN_STATUS_INTERVAL};
use crate::career_backend;
use crate::match_rules::MatchConfig;
use crate::runtime::ports::{ManualClock, MemoryTransport};
use crate::runtime::{PLAYER_TIMEOUT, ServerRuntime};
use crate::snapshot::SNAPSHOT_INTERVAL;

/// A status reply carries no world state and stays far below one datagram.
const STATUS_REPLY_MAX_BYTES: usize = 600;

fn runtime() -> (ServerRuntime, MemoryTransport, ManualClock) {
    let clock = ManualClock::new(Instant::now());
    let transport = MemoryTransport::new("127.0.0.1:4000".parse().unwrap());
    let rt = ServerRuntime::for_test(
        transport.clone(),
        clock.clone(),
        career_backend::MemoryCareer::disabled(54100),
        MatchConfig::dev(),
    );
    (rt, transport, clock)
}

fn stranger(index: usize) -> SocketAddr {
    SocketAddr::from((
        [10, 0, (index / 250) as u8, (index % 250) as u8 + 1],
        40_000,
    ))
}

fn ping() -> Vec<u8> {
    serde_json::to_vec(&ClientPacket::Ping).unwrap()
}

fn step(rt: &mut ServerRuntime, clock: &ManualClock, by: Duration) {
    clock.advance(by);
    let (now, dt) = rt.prepare_tick();
    rt.tick(now, dt);
}

fn by_recipient(sent: Vec<(SocketAddr, Vec<u8>)>) -> HashMap<SocketAddr, Vec<Vec<u8>>> {
    let mut map: HashMap<SocketAddr, Vec<Vec<u8>>> = HashMap::new();
    for (to, bytes) in sent {
        map.entry(to).or_default().push(bytes);
    }
    map
}

fn assert_status_reply(bytes: &[u8]) -> Value {
    assert!(
        bytes.len() <= STATUS_REPLY_MAX_BYTES,
        "status reply is {} bytes",
        bytes.len()
    );
    let value: Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(value["type"], "snapshot");
    for field in [
        "players",
        "projectiles",
        "combat_events",
        "structures",
        "minions",
        "neutrals",
        "team_buffs",
        "forest_pickups",
    ] {
        assert_eq!(value[field], Value::Array(Vec::new()), "{field} leaked");
    }
    for field in ["vision", "sandbox", "scoreboard", "prematch"] {
        assert_eq!(value[field], Value::Null, "{field} leaked");
    }
    assert!(
        value.get("debug_access").is_none(),
        "debug access goes to joined players only"
    );
    value
}

#[test]
fn burst_from_unjoined_addresses_keeps_at_most_the_limit_and_one_small_reply_each() {
    let (mut rt, transport, clock) = runtime();
    let burst = MAX_PREJOIN_ENDPOINTS + 40;
    // Each address sends three datagrams in the same interval; the receive
    // loop drains at most MAX_PACKETS_PER_TICK per tick, so run a few ticks
    // inside one status interval.
    for index in 0..burst {
        for _ in 0..3 {
            transport.push_inbound(stranger(index), ping());
        }
    }
    let mut sent = Vec::new();
    for _ in 0..4 {
        step(&mut rt, &clock, SNAPSHOT_INTERVAL);
        sent.extend(transport.take_outbound());
    }
    assert!(rt.world.players.len() <= MAX_PREJOIN_ENDPOINTS);
    assert_eq!(rt.world.players.len(), MAX_PREJOIN_ENDPOINTS);
    let replies = by_recipient(sent);
    assert_eq!(replies.len(), MAX_PREJOIN_ENDPOINTS);
    for (to, datagrams) in &replies {
        assert!(rt.world.players.contains_key(to), "{to} was dropped");
        assert_eq!(datagrams.len(), 1, "{to} got more than one reply");
        let status = assert_status_reply(&datagrams[0]);
        assert_eq!(status["your_id"], rt.world.players[to].hero.identity.id);
    }
    for index in MAX_PREJOIN_ENDPOINTS..burst {
        assert!(!rt.world.players.contains_key(&stranger(index)));
    }
}

#[test]
fn a_silent_unjoined_endpoint_gets_one_reply_not_a_snapshot_stream() {
    let (mut rt, transport, clock) = runtime();
    let addr = stranger(0);
    transport.push_inbound(addr, ping());
    let mut replies = 0;
    let mut elapsed = Duration::ZERO;
    while elapsed < PLAYER_TIMEOUT {
        step(&mut rt, &clock, SNAPSHOT_INTERVAL);
        elapsed += SNAPSHOT_INTERVAL;
        for (to, bytes) in transport.take_outbound() {
            assert_eq!(to, addr);
            assert_status_reply(&bytes);
            replies += 1;
        }
    }
    assert_eq!(replies, 1, "one datagram earns one reply");
}

#[test]
fn a_chatty_unjoined_endpoint_gets_at_most_one_reply_per_interval() {
    let (mut rt, transport, clock) = runtime();
    let addr = stranger(0);
    let window = Duration::from_secs(2);
    let mut elapsed = Duration::ZERO;
    let mut replies = 0;
    while elapsed < window {
        transport.push_inbound(addr, ping());
        transport.push_inbound(addr, ping());
        step(&mut rt, &clock, SNAPSHOT_INTERVAL);
        elapsed += SNAPSHOT_INTERVAL;
        replies += transport
            .take_outbound()
            .into_iter()
            .inspect(|(_, bytes)| {
                assert_status_reply(bytes);
            })
            .count();
    }
    let most = (window.as_millis() / PREJOIN_STATUS_INTERVAL.as_millis()) as usize + 1;
    assert!(replies >= 2, "a live endpoint keeps getting status");
    assert!(replies <= most, "{replies} replies in {window:?}");
}

#[test]
fn a_rejected_hello_is_reported_in_the_status_reply() {
    let (mut rt, transport, clock) = runtime();
    let addr = stranger(0);
    transport.push_inbound(
        addr,
        serde_json::to_vec(&ClientPacket::Hello {
            protocol_version: shared::protocol::PROTOCOL_VERSION + 1,
        })
        .unwrap(),
    );
    step(&mut rt, &clock, SNAPSHOT_INTERVAL);
    let sent = transport.take_outbound();
    assert_eq!(sent.len(), 1);
    let mut assembler = shared::transport::SnapshotAssembler::default();
    let payload = assembler
        .push(&sent[0].1, clock_now(&clock))
        .unwrap()
        .expect("a status reply fits one frame");
    let status = assert_status_reply(&payload);
    assert_eq!(status["join_error"], "protocol_mismatch");
}

fn clock_now(clock: &ManualClock) -> Instant {
    crate::runtime::ports::Clock::now(clock)
}

#[test]
fn a_joined_player_still_gets_full_snapshots_beside_unjoined_strangers() {
    let (mut rt, transport, clock) = runtime();
    let player: SocketAddr = "127.0.0.1:54101".parse().unwrap();
    let join = ClientPacket::Join {
        prematch: false,
        team: Team::Green,
        character: CharacterChoice::Ipfs,
        hero_class: HeroClass::Mage,
        avatar: None,
        sprite_character: None,
        session_id: Some("o4-joined".into()),
        passport_ticket: None,
    };
    transport.push_inbound(player, serde_json::to_vec(&join).unwrap());
    for index in 0..MAX_PREJOIN_ENDPOINTS + 10 {
        transport.push_inbound(stranger(index), ping());
    }
    let mut full = 0;
    for _ in 0..10 {
        transport.push_inbound(player, ping());
        step(&mut rt, &clock, SNAPSHOT_INTERVAL);
        for (to, bytes) in transport.take_outbound() {
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            if to == player {
                assert!(!value["structures"].as_array().unwrap().is_empty());
                assert_eq!(value["players"].as_array().unwrap().len(), 1);
                full += 1;
            } else {
                assert_status_reply(&bytes);
            }
        }
    }
    assert!(rt.world.players[&player].joined);
    assert_eq!(full, 10, "one full snapshot per interval");
}
