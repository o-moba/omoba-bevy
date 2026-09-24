use std::net::SocketAddr;
use std::time::{Duration, Instant};

use shared::wire::{GameState, ServerPacket};

use crate::game_world::GameWorld;
use crate::runtime::{NETWORK_DIAGNOSTIC_INTERVAL, RateLimitedDiagnostic};
use crate::snapshot::{
    IPV4_UDP_MAX_PAYLOAD_BYTES, SnapshotDatagramError, build_players_snapshot,
    serialize_snapshot_datagram, validate_snapshot_payload_size,
};

fn empty_snapshot() -> ServerPacket {
    ServerPacket::Snapshot {
        vision: None,
        sandbox: None,
        match_mode: "dev".into(),
        geometry_id: shared::map::GEOMETRY_ID.to_owned(),
        map_profile: "verdant_default".to_owned(),
        meta: Default::default(),
        join_error: None,
        your_id: 1,
        players: Vec::new(),
        scoreboard: None,
        prematch: None,
        projectiles: Vec::new(),
        combat_events: Vec::new(),
        structures: Vec::new(),
        minions: Vec::new(),
        neutrals: Vec::new(),
        team_buffs: Vec::new(),
        forest_pickups: Vec::new(),
        game_state: GameState::Running,
        rematch_in_secs: None,
    }
}

#[test]
fn udp_payload_guard_accepts_limit_and_rejects_one_byte_over() {
    assert!(validate_snapshot_payload_size(IPV4_UDP_MAX_PAYLOAD_BYTES - 1).is_ok());
    assert!(validate_snapshot_payload_size(IPV4_UDP_MAX_PAYLOAD_BYTES).is_ok());
    assert!(matches!(
        validate_snapshot_payload_size(IPV4_UDP_MAX_PAYLOAD_BYTES + 1),
        Err(SnapshotDatagramError::PayloadTooLarge {
            actual: 65_508,
            limit: 65_507
        })
    ));
}

#[test]
fn snapshot_serializer_rejects_whole_over_limit_payload() {
    let mut world = GameWorld::empty();
    let addr: SocketAddr = "127.0.0.1:34600".parse().unwrap();
    let now = Instant::now();
    world.ensure_connected(addr, now);
    let player = world.players.get_mut(&addr).unwrap();
    player.joined = true;
    player.hero.identity.avatar = Some("x".repeat(IPV4_UDP_MAX_PAYLOAD_BYTES));
    let your_id = player.hero.identity.id;

    let packet = ServerPacket::Snapshot {
        vision: None,
        sandbox: None,
        match_mode: "dev".into(),
        geometry_id: shared::map::GEOMETRY_ID.to_owned(),
        map_profile: "verdant_default".to_owned(),
        meta: Default::default(),
        join_error: None,
        your_id,
        players: build_players_snapshot(&world, Some(your_id), now),
        scoreboard: None,
        prematch: None,
        projectiles: Vec::new(),
        combat_events: Vec::new(),
        structures: Vec::new(),
        minions: Vec::new(),
        neutrals: Vec::new(),
        team_buffs: Vec::new(),
        forest_pickups: Vec::new(),
        game_state: GameState::Running,
        rematch_in_secs: None,
    };
    let error = serialize_snapshot_datagram(&packet).unwrap_err();
    assert!(matches!(
        error,
        SnapshotDatagramError::PayloadTooLarge { actual, limit }
            if actual > IPV4_UDP_MAX_PAYLOAD_BYTES
                && limit == IPV4_UDP_MAX_PAYLOAD_BYTES
    ));

    let legal = serialize_snapshot_datagram(&empty_snapshot()).unwrap();
    assert!(legal.len() < IPV4_UDP_MAX_PAYLOAD_BYTES);
    assert!(serde_json::from_slice::<ServerPacket>(&legal).is_ok());
}

#[test]
fn network_diagnostic_bursts_are_suppressed_and_reported() {
    let started = Instant::now();
    let mut diagnostic = RateLimitedDiagnostic::default();
    assert_eq!(diagnostic.record(started), Some(0));
    assert_eq!(diagnostic.record(started + Duration::from_millis(1)), None);
    assert_eq!(diagnostic.record(started + Duration::from_millis(2)), None);
    assert_eq!(
        diagnostic.record(started + NETWORK_DIAGNOSTIC_INTERVAL),
        Some(2)
    );
}
