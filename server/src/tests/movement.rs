use std::net::SocketAddr;
use std::time::{Duration, Instant};

use shared::HeroClass;
use shared::map::{Lane, Team};
use shared::wire::CharacterChoice;

use super::*;
use crate::balance::{
    MANA_REGEN_PER_SECOND, MAX_MANA, MOVEMENT_POSITION_TOLERANCE, PLAYER_GROUND_Y, PLAYER_SPEED,
};
use crate::game_world::GameWorld;
use crate::hero::Hero;
use crate::session::{handle_join_request, handle_transform_request};
use crate::sim::regenerate_mana;
use crate::world::{build_map_layout, build_minion_path, lane_control_points};

fn horizontal_distance(a: &Hero, b: &Hero) -> f32 {
    let dx = a.x - b.x;
    let dz = a.z - b.z;
    (dx * dx + dz * dz).sqrt()
}

#[test]
fn map_layout_is_symmetric() {
    let layout = build_map_layout();
    assert!((layout.home.x + layout.away.x).abs() < EPSILON);
    assert!((layout.home.z + layout.away.z).abs() < EPSILON);
    assert!(layout.left_x < layout.right_x);
    assert!(layout.bottom_z < layout.top_z);
}

#[test]
fn lane_paths_connect_bases() {
    let layout = build_map_layout();
    for lane in [Lane::Top, Lane::Mid, Lane::Bot] {
        let points = lane_control_points(&layout, lane);
        assert!((points.first().unwrap().x - layout.home.x).abs() < EPSILON);
        assert!((points.first().unwrap().z - layout.home.z).abs() < EPSILON);
        assert!((points.last().unwrap().x - layout.away.x).abs() < EPSILON);
        assert!((points.last().unwrap().z - layout.away.z).abs() < EPSILON);
    }
}

#[test]
fn blue_path_is_reverse_of_green() {
    let layout = build_map_layout();
    let green = build_minion_path(&layout, Lane::Mid, Team::Green);
    let blue = build_minion_path(&layout, Lane::Mid, Team::Blue);
    assert!((green.first().unwrap().x - layout.home.x).abs() < EPSILON);
    assert!((blue.first().unwrap().x - layout.away.x).abs() < EPSILON);
    assert!((green.last().unwrap().x - layout.away.x).abs() < EPSILON);
    assert!((blue.last().unwrap().x - layout.home.x).abs() < EPSILON);
}

#[test]
fn mana_regenerates_and_is_clamped() {
    let mut world = GameWorld::empty();
    let addr: SocketAddr = "127.0.0.1:34567".parse().unwrap();
    let now = Instant::now();

    world.ensure_connected(addr, now);
    let player = world.players.get_mut(&addr).unwrap();
    player.joined = true;
    player.hero.mana = 10.0;
    player.hero.max_mana = MAX_MANA;

    regenerate_mana(&mut world.players, 2.5);
    let expected = 10.0 + MANA_REGEN_PER_SECOND * 2.5;
    let current = world.players.get(&addr).unwrap().hero.mana;
    assert!((current - expected).abs() < EPSILON);

    regenerate_mana(&mut world.players, 100.0);
    let clamped = world.players.get(&addr).unwrap().hero.mana;
    assert!((clamped - MAX_MANA).abs() < EPSILON);
}

#[test]
fn movement_authority_clamps_teleports_and_accepts_normal_steps() {
    let mut world = GameWorld::empty();
    let addr: SocketAddr = "127.0.0.1:35001".parse().unwrap();
    let now = Instant::now();

    world.ensure_connected(addr, now);
    handle_join_request(
        world.players.get_mut(&addr).unwrap(),
        Team::Green,
        CharacterChoice::Ipfs,
        HeroClass::default(),
        None,
        &world.map_layout,
        now,
    );
    let start = world.players.get(&addr).unwrap().hero.clone();
    let normal_at = now + Duration::from_millis(100);
    let normal_x = start.x + PLAYER_SPEED * 0.1;
    handle_transform_request(
        world.players.get_mut(&addr).unwrap(),
        &world.map_layout,
        normal_x,
        PLAYER_GROUND_Y,
        start.z,
        0.25,
        normal_at,
    );

    let after_normal = world.players.get(&addr).unwrap().hero.clone();
    assert!((after_normal.x - normal_x).abs() < EPSILON);
    assert!((after_normal.y - PLAYER_GROUND_Y).abs() < EPSILON);
    assert!((after_normal.yaw - 0.25).abs() < EPSILON);

    let teleport_at = normal_at + Duration::from_millis(50);
    let before_teleport = world.players.get(&addr).unwrap().hero.clone();
    handle_transform_request(
        world.players.get_mut(&addr).unwrap(),
        &world.map_layout,
        before_teleport.x + 500.0,
        PLAYER_GROUND_Y,
        before_teleport.z,
        0.5,
        teleport_at,
    );

    let after_teleport = world.players.get(&addr).unwrap().hero.clone();
    let accepted_distance = horizontal_distance(&before_teleport, &after_teleport);
    let max_distance = PLAYER_SPEED * 0.05 + MOVEMENT_POSITION_TOLERANCE + EPSILON;
    assert!(accepted_distance <= max_distance);
    assert!(after_teleport.x < before_teleport.x + 500.0);

    let before_invalid = after_teleport.clone();
    handle_transform_request(
        world.players.get_mut(&addr).unwrap(),
        &world.map_layout,
        f32::NAN,
        PLAYER_GROUND_Y,
        before_invalid.z + 1.0,
        1.0,
        teleport_at + Duration::from_millis(50),
    );
    let after_invalid = world.players.get(&addr).unwrap().hero.clone();
    assert!((after_invalid.x - before_invalid.x).abs() < EPSILON);
    assert!((after_invalid.z - before_invalid.z).abs() < EPSILON);
}

#[test]
fn movement_authority_keeps_players_inside_map_bounds() {
    let mut world = GameWorld::empty();
    let addr: SocketAddr = "127.0.0.1:35002".parse().unwrap();
    let now = Instant::now();

    world.ensure_connected(addr, now);
    handle_join_request(
        world.players.get_mut(&addr).unwrap(),
        Team::Green,
        CharacterChoice::Ipfs,
        HeroClass::default(),
        None,
        &world.map_layout,
        now,
    );
    {
        let player = world.players.get_mut(&addr).unwrap();
        player.hero.x = world.map_layout.max_x - 0.1;
        player.hero.z = world.map_layout.max_z - 0.1;
        player.timers.last_movement_at = now;
    }

    handle_transform_request(
        world.players.get_mut(&addr).unwrap(),
        &world.map_layout,
        world.map_layout.max_x + 5.0,
        PLAYER_GROUND_Y,
        world.map_layout.max_z + 5.0,
        0.0,
        now + Duration::from_secs(1),
    );

    let player = world.players.get(&addr).unwrap();
    assert!(player.hero.x <= world.map_layout.max_x);
    assert!(player.hero.z <= world.map_layout.max_z);
    assert!(player.hero.x >= world.map_layout.min_x);
    assert!(player.hero.z >= world.map_layout.min_z);
}

/// Joins one hero on a runtime driven by a manual clock and parks it in open
/// ground, so every step below is limited by the movement budget alone.
fn budget_runtime(
    port: u16,
) -> (
    crate::runtime::ServerRuntime,
    crate::runtime::ports::MemoryTransport,
    crate::runtime::ports::ManualClock,
    SocketAddr,
) {
    use crate::runtime::ports::{ManualClock, MemoryTransport};
    let clock = ManualClock::new(Instant::now());
    let transport = MemoryTransport::new("127.0.0.1:4000".parse().unwrap());
    let mut rt = crate::runtime::ServerRuntime::for_test(
        transport.clone(),
        clock.clone(),
        crate::career_backend::MemoryCareer::disabled(u64::from(port)),
        crate::match_rules::MatchConfig::dev(),
    );
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let join = shared::wire::ClientPacket::Join {
        prematch: false,
        team: Team::Green,
        character: CharacterChoice::Ipfs,
        hero_class: HeroClass::default(),
        avatar: None,
        sprite_character: None,
        session_id: Some(format!("budget-{port}")),
        passport_ticket: None,
    };
    transport.push_inbound(addr, serde_json::to_vec(&join).unwrap());
    let (now, dt) = rt.prepare_tick();
    rt.tick(now, dt);
    let player = rt.world.players.get_mut(&addr).unwrap();
    assert!(player.joined);
    player.hero.x = -12.0;
    player.hero.z = 0.0;
    player.timers.last_movement_at = now;
    (rt, transport, clock, addr)
}

fn send_transform(
    rt: &mut crate::runtime::ServerRuntime,
    transport: &crate::runtime::ports::MemoryTransport,
    clock: &crate::runtime::ports::ManualClock,
    addr: SocketAddr,
    step: Duration,
    x: f32,
) {
    clock.advance(step);
    let packet = shared::wire::ClientPacket::Transform {
        dash_sequence: 0,
        x,
        y: PLAYER_GROUND_Y,
        z: 0.0,
        yaw: 0.0,
    };
    transport.push_inbound(addr, serde_json::to_vec(&packet).unwrap());
    let (now, dt) = rt.prepare_tick();
    rt.tick(now, dt);
    transport.take_outbound();
}

/// O3: the position tolerance is a budget, not a per-packet allowance. A
/// client flooding 120 transforms a second, each asking for a far point,
/// covers no more than a second of normal speed plus one tolerance (the old
/// envelope granted `+0.10` per packet: 17 u/s instead of 5).
#[test]
fn movement_budget_caps_a_packet_flood_at_normal_speed() {
    let (mut rt, transport, clock, addr) = budget_runtime(35101);
    let speed = crate::hero_stats::move_speed(&rt.world.players[&addr]);
    let start_x = rt.world.players[&addr].hero.x;
    let step = Duration::from_secs(1) / 120;
    for _ in 0..120 {
        let x = rt.world.players[&addr].hero.x + 100.0;
        send_transform(&mut rt, &transport, &clock, addr, step, x);
    }
    let covered = rt.world.players[&addr].hero.x - start_x;
    let ceiling = speed * step.as_secs_f32() * 120.0 + MOVEMENT_POSITION_TOLERANCE + 0.001;
    assert!(covered > speed * 0.9, "the flood still moves: {covered}");
    assert!(
        covered <= ceiling,
        "a 120 packet/s flood covered {covered} in 1 s (ceiling {ceiling})"
    );
}

/// O3: a normal 20 Hz client stepping at its own speed, with ±15 ms arrival
/// jitter, is never clipped by the time budget.
#[test]
fn movement_budget_leaves_a_normal_20hz_client_unclipped() {
    let (mut rt, transport, clock, addr) = budget_runtime(35102);
    let speed = crate::hero_stats::move_speed(&rt.world.players[&addr]);
    let start_x = rt.world.players[&addr].hero.x;
    let mut client_x = start_x;
    // Arrival gaps around the 50 ms send interval; they sum to 1 s.
    let gaps_ms = [35_u64, 65, 50, 40, 60, 50, 50, 35, 65, 50];
    for round in 0..2 {
        for gap in gaps_ms {
            client_x += speed * 0.05;
            let step = Duration::from_millis(gap);
            send_transform(&mut rt, &transport, &clock, addr, step, client_x);
            let hero_x = rt.world.players[&addr].hero.x;
            assert!(
                (hero_x - client_x).abs() < 0.001,
                "round {round}: step clipped to {hero_x}, client at {client_x}"
            );
        }
    }
    assert!((rt.world.players[&addr].hero.x - start_x - speed * 1.0 * 2.0).abs() < 0.01);
}
