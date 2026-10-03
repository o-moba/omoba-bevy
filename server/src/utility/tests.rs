use shared::utility::*;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use shared::HeroClass;
use shared::map::Team;
use shared::wire::{CharacterChoice, ClientPacket, GameState, StructureKind};

use super::*;
use crate::balance::{MAX_HP, MOVEMENT_POSITION_TOLERANCE, PLAYER_GROUND_Y, PLAYER_SPEED};
use crate::combat_feedback::apply_player_damage;
use crate::entities::Vec3f;
use crate::hero::HeroUtility;
use crate::hero_timers;
use crate::match_rules::MatchConfig;
use crate::runtime::ServerRuntime;
use crate::session::{handle_transform_request, reset_player_round};

fn fixture() -> (ServerRuntime, SocketAddr, Instant) {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    let mut rt = ServerRuntime::new(socket, MatchConfig::dev());
    let addr = "127.0.0.1:58112".parse().unwrap();
    let now = Instant::now();
    rt.handle_packet(
        addr,
        ClientPacket::Join {
            handheld: Default::default(),
            prematch: false,
            team: Team::Green,
            character: CharacterChoice::Cube,
            hero_class: HeroClass::Warrior,
            avatar: None,
            sprite_character: None,
            session_id: Some("utility-fixture".into()),
            passport_ticket: None,
        },
        now,
    );
    let p = rt.world.players.get_mut(&addr).unwrap();
    p.hero.x = 0.0;
    p.hero.z = 0.0;
    (rt, addr, now)
}

fn request(
    rt: &mut ServerRuntime,
    addr: SocketAddr,
    action: UtilityAction,
    direction: [f32; 2],
    id: u64,
    now: Instant,
) {
    // Keep this utility fixture connected across synthetic cooldown jumps.
    rt.world.players.get_mut(&addr).unwrap().last_seen = now;
    let packet = ClientPacket::Utility {
        action,
        direction,
        request_id: id,
        server_epoch: rt.server_epoch,
        match_id: rt.match_id,
    };
    // Exercise the actual request wire format and packet dispatch.
    let bytes = serde_json::to_vec(&packet).unwrap();
    rt.handle_packet(addr, serde_json::from_slice(&bytes).unwrap(), now);
}

#[test]
fn dash_uses_match_identity_replay_protection_and_independent_twenty_second_cooldown() {
    let (mut rt, addr, now) = fixture();
    let bad = ClientPacket::Utility {
        action: UtilityAction::Dash,
        direction: [1.0, 0.0],
        request_id: 100,
        server_epoch: rt.server_epoch + 1,
        match_id: rt.match_id,
    };
    rt.handle_packet(addr, bad, now);
    assert_eq!(rt.world.players[&addr].hero.utility.last_request_id, 0);
    request(&mut rt, addr, UtilityAction::Dash, [100.0, 0.0], 1, now);
    assert!((rt.world.players[&addr].hero.x - DASH_DISTANCE).abs() < 0.001);
    assert_eq!(rt.world.players[&addr].hero.utility.dash_sequence, 1);
    assert_eq!(
        rt.player_view(addr, now).utility.dash_remaining_secs,
        DASH_COOLDOWN_SECS
    );
    request(
        &mut rt,
        addr,
        UtilityAction::Dash,
        [-1.0, 0.0],
        2,
        now + Duration::from_secs(1),
    );
    request(
        &mut rt,
        addr,
        UtilityAction::Dash,
        [-1.0, 0.0],
        2,
        now + Duration::from_secs(21),
    );
    assert_eq!(
        rt.world.players[&addr].hero.utility.dash_sequence, 1,
        "rejected request cannot replay after cooldown"
    );
    request(
        &mut rt,
        addr,
        UtilityAction::Dash,
        [-1.0, 0.0],
        3,
        now + Duration::from_secs(21),
    );
    assert_eq!(rt.world.players[&addr].hero.utility.dash_sequence, 2);
    assert!((rt.world.players[&addr].hero.x).abs() < 0.001);
}

#[test]
fn utilities_require_admission_alive_running_and_valid_direction() {
    let (mut rt, addr, now) = fixture();
    rt.world.players.get_mut(&addr).unwrap().joined = false;
    request(&mut rt, addr, UtilityAction::Haste, [0.0, 0.0], 1, now);
    assert_eq!(rt.world.players[&addr].hero.utility.last_request_id, 0);
    rt.world.players.get_mut(&addr).unwrap().joined = true;
    rt.world.players.get_mut(&addr).unwrap().hero.hp = 0.0;
    request(&mut rt, addr, UtilityAction::Haste, [0.0, 0.0], 1, now);
    rt.world.players.get_mut(&addr).unwrap().hero.hp = MAX_HP;
    request(&mut rt, addr, UtilityAction::Haste, [0.0, 0.0], 1, now);
    assert_eq!(rt.player_view(addr, now).utility.haste_active_secs, 0.0);
    rt.world.game_state = GameState::Lobby;
    request(&mut rt, addr, UtilityAction::Haste, [0.0, 0.0], 2, now);
    assert_eq!(rt.player_view(addr, now).utility.haste_active_secs, 0.0);
    rt.world.game_state = GameState::Running;
    request(&mut rt, addr, UtilityAction::Dash, [0.0, 0.0], 3, now);
    let p = rt.world.players.get_mut(&addr).unwrap();
    handle_utility_request(
        p,
        &rt.world.map_layout,
        &rt.world.structures,
        &rt.world.game_state,
        UtilityAction::Dash,
        [f32::NAN, 0.0],
        4,
        now,
    );
    assert_eq!(p.hero.utility.dash_sequence, 0);
    assert_eq!(hero_timers::dash_remaining(p, now), 0.0);
}

#[test]
fn haste_lasts_three_seconds_and_cancels_on_death_without_clearing_replay_or_cooldowns() {
    let (mut rt, addr, now) = fixture();
    request(&mut rt, addr, UtilityAction::Haste, [0.0, 0.0], 1, now);
    let p = &rt.world.players[&addr];
    assert_eq!(hero_timers::haste_active(p, now), HASTE_DURATION_SECS);
    assert_eq!(hero_timers::haste_remaining(p, now), HASTE_COOLDOWN_SECS);
    assert_eq!(
        utility_movement_multiplier(p, now + Duration::from_millis(2999)),
        1.4
    );
    assert_eq!(
        utility_movement_multiplier(p, now + Duration::from_secs(3)),
        1.0
    );
    let p = rt.world.players.get_mut(&addr).unwrap();
    p.timers.last_movement_at = now;
    handle_transform_request(
        p,
        &rt.world.map_layout,
        10.0,
        PLAYER_GROUND_Y,
        0.0,
        0.0,
        now + Duration::from_millis(100),
    );
    assert!((p.hero.x - (PLAYER_SPEED * 1.4 * 0.1 + MOVEMENT_POSITION_TOLERANCE)).abs() < 0.001);
    let id = p.hero.identity.id;
    apply_player_damage(
        &mut rt.world.players,
        id,
        MAX_HP * 2.0,
        now + Duration::from_secs(1),
    );
    let p = rt.world.players.get_mut(&addr).unwrap();
    assert_eq!(
        utility_movement_multiplier(p, now + Duration::from_secs(1)),
        1.0
    );
    assert_eq!(
        hero_timers::haste_active(p, now + Duration::from_secs(1)),
        0.0
    );
    assert_eq!(p.hero.utility.last_request_id, 1);
    assert!(p.timers.haste_ready_at.is_some());
    reset_player_round(p, &rt.world.map_layout, now);
    assert_eq!(p.hero.utility, HeroUtility::default());
    assert!(p.timers.haste_ready_at.is_none());
}

#[test]
fn dash_clips_live_structures_and_old_transform_packets_cannot_undo_short_dash() {
    let (mut rt, addr, now) = fixture();
    let structure = rt.world.structures.values_mut().next().unwrap();
    structure.state.x = 3.0;
    structure.state.z = 0.0;
    structure.state.kind = StructureKind::Tower;
    structure.state.hp = 100.0;
    request(&mut rt, addr, UtilityAction::Dash, [1.0, 0.0], 1, now);
    let x = rt.world.players[&addr].hero.x;
    assert!(
        x > 0.0 && x < 2.0,
        "expected clipped short displacement: {x}"
    );
    assert_eq!(rt.world.players[&addr].hero.utility.dash_sequence, 1);
    rt.handle_packet(
        addr,
        ClientPacket::Transform {
            x: 0.0,
            y: PLAYER_GROUND_Y,
            z: 0.0,
            yaw: 0.0,
            dash_sequence: 0,
        },
        now + Duration::from_millis(100),
    );
    assert_eq!(rt.world.players[&addr].hero.x, x);
    rt.handle_packet(
        addr,
        ClientPacket::Transform {
            x: x - 0.1,
            y: PLAYER_GROUND_Y,
            z: 0.0,
            yaw: 0.0,
            dash_sequence: 1,
        },
        now + Duration::from_millis(100),
    );
    assert!(rt.world.players[&addr].hero.x < x);
}

#[test]
fn dash_sweeps_static_obstacles_and_honors_world_bounds() {
    let (mut rt, addr, now) = fixture();
    rt.world.structures.clear();
    let nav = shared::navigation::world_navigation();
    let from = nav
        .obstacles()
        .iter()
        .filter(|o| o.kind == "tree_trunk")
        .find_map(|o| {
            let c = o
                .vertices
                .iter()
                .fold([0.0, 0.0], |a, p| [a[0] + p[0], a[1] + p[1]])
                .map(|x| x / o.vertices.len() as f32);
            let p = [c[0] - 2.5, c[1]];
            (nav.point_clear(p) && !nav.segment_clear(p, [p[0] + 5.0, p[1]])).then_some(p)
        })
        .unwrap();
    let p = rt.world.players.get_mut(&addr).unwrap();
    p.hero.x = from[0];
    p.hero.z = from[1];
    request(&mut rt, addr, UtilityAction::Dash, [1.0, 0.0], 1, now);
    let p = &rt.world.players[&addr];
    assert!(p.hero.x - from[0] < DASH_DISTANCE);
    assert!(nav.point_clear([p.hero.x, p.hero.z]));
    assert!(nav.segment_clear(from, [p.hero.x, p.hero.z]));
    let edge = rt
        .world
        .map_layout
        .clamp_player_position(Vec3f::new(1.0e6, PLAYER_GROUND_Y, 0.0));
    let p = rt.world.players.get_mut(&addr).unwrap();
    p.hero.x = edge.x;
    p.hero.z = edge.z;
    request(
        &mut rt,
        addr,
        UtilityAction::Dash,
        [1.0, 0.0],
        2,
        now + Duration::from_secs(21),
    );
    assert!(rt.world.players[&addr].hero.x <= edge.x);
}

#[test]
fn standard_kits_retain_shared_utility_cooldowns_and_replay_protection() {
    for class in [
        HeroClass::Dawnweaver,
        HeroClass::Wildspark,
        HeroClass::Stormfist,
    ] {
        let (mut rt, addr, now) = fixture();
        let p = rt.world.players.get_mut(&addr).unwrap();
        p.hero.identity.hero_class = class;
        p.hero.skills.loadout = shared::loadout::preset_for_class(class);
        request(&mut rt, addr, UtilityAction::Dash, [1.0, 0.0], 1, now);
        assert_eq!(rt.world.players[&addr].hero.utility.dash_sequence, 1);
        request(&mut rt, addr, UtilityAction::Dash, [1.0, 0.0], 1, now);
        assert_eq!(rt.world.players[&addr].hero.utility.dash_sequence, 1);
        request(&mut rt, addr, UtilityAction::Haste, [0.0, 0.0], 2, now);
        assert_eq!(
            hero_timers::haste_active(&rt.world.players[&addr], now),
            HASTE_DURATION_SECS
        );
    }
}

#[test]
fn signed_roster_dash_and_haste_dispatch_to_authority_and_replicate() {
    use crate::public_transport::{Decision, PublicTransport};
    use ed25519_dalek::{Signer, SigningKey};
    use shared::public_transport::{
        GameplayPrincipal, PublicClientDatagram, PublicServerDatagram, SignedCommand, hex,
    };
    let (mut rt, addr, now) = fixture();
    rt.world.players.get_mut(&addr).unwrap().hero.skills.loadout =
        Some(shared::loadout::resolve(&shared::loadout::CoreId::Dawnweaver.preset()).unwrap());
    let mut gate = PublicTransport::default();
    let probe = serde_json::to_vec(&PublicClientDatagram::TransportProbe {
        protocol_version: shared::protocol::PROTOCOL_VERSION,
        client_nonce: "a".repeat(32),
        padding: "0".repeat(384),
    })
    .unwrap();
    let Decision::Reply(reply) =
        gate.receive(addr, &probe, rt.server_epoch, rt.match_id, false, None, now)
    else {
        panic!("missing path challenge");
    };
    let PublicServerDatagram::TransportChallenge { path_nonce, .. } =
        serde_json::from_slice(&reply).unwrap();
    let proof = serde_json::to_vec(&PublicClientDatagram::TransportProof {
        server_epoch: rt.server_epoch,
        path_nonce: path_nonce.clone(),
    })
    .unwrap();
    assert!(matches!(
        gate.receive(addr, &proof, rt.server_epoch, rt.match_id, false, None, now),
        Decision::Dispatch(ClientPacket::Hello { .. })
    ));
    let key = SigningKey::from_bytes(&[19; 32]);
    let principal = GameplayPrincipal {
        public_key: hex(key.verifying_key().as_bytes()),
        session_id: "utility-fixture".into(),
        session_nonce: "c".repeat(64),
    };
    for (request_id, action) in [(1, UtilityAction::Dash), (2, UtilityAction::Haste)] {
        let packet = ClientPacket::Utility {
            action,
            direction: [1.0, 0.0],
            server_epoch: rt.server_epoch,
            match_id: rt.match_id,
            request_id,
        };
        let mut command = SignedCommand {
            server_epoch: rt.server_epoch,
            match_id: rt.match_id,
            session_id: principal.session_id.clone(),
            session_nonce: principal.session_nonce.clone(),
            path_nonce: path_nonce.clone(),
            sequence: request_id,
            payload: serde_json::to_string(&packet).unwrap(),
            signature: String::new(),
        };
        command.signature = hex(&key.sign(&command.signing_bytes()).to_bytes());
        let bytes = serde_json::to_vec(&PublicClientDatagram::SignedCommand { command }).unwrap();
        let Decision::Dispatch(packet) = gate.receive(
            addr,
            &bytes,
            rt.server_epoch,
            rt.match_id,
            false,
            Some(principal.clone()),
            now,
        ) else {
            panic!("signed utility was not dispatched");
        };
        rt.handle_packet(addr, packet, now);
        assert!(
            matches!(
                gate.receive(
                    addr,
                    &bytes,
                    rt.server_epoch,
                    rt.match_id,
                    false,
                    Some(principal.clone()),
                    now
                ),
                Decision::Drop
            ),
            "signed replay cannot act twice"
        );
    }
    // This is the exact state serialized into the owning client's snapshot.
    let state = rt.player_view(addr, now);
    assert!(state.loadout.unwrap().recipe.is_some());
    assert!((state.x - DASH_DISTANCE).abs() < 0.001);
    assert_eq!(state.utility.dash_sequence, 1);
    assert_eq!(state.utility.last_request_id, 2);
    assert_eq!(state.utility.dash_remaining_secs, DASH_COOLDOWN_SECS);
    assert_eq!(state.utility.haste_remaining_secs, HASTE_COOLDOWN_SECS);
    assert_eq!(state.utility.haste_active_secs, HASTE_DURATION_SECS);
    assert_eq!(
        utility_movement_multiplier(&rt.world.players[&addr], now),
        HASTE_SPEED_MULTIPLIER
    );
}
