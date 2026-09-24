use crate::runtime::ports::Clock;
use crate::career_backend;
use crate::game_world::GameWorld;
use crate::session::handle_transform_request;
use crate::match_rules::MatchConfig;
use crate::entities::DisconnectedSession;
use crate::runtime::PLAYER_TIMEOUT;
use crate::runtime::ports::MemoryTransport;
use shared::wire::ClientPacket;
use shared::wire::GameState;
use std::time::Duration;
use std::time::Instant;
use crate::balance::PLAYER_GROUND_Y;
use crate::snapshot::build_players_snapshot;
use shared::wire::CharacterChoice;
use std::net::SocketAddr;
use crate::session::handle_join_request;
use crate::runtime::ServerRuntime;
use shared::wire::TargetId;
use crate::snapshot::SNAPSHOT_INTERVAL;
use crate::runtime::ports::ManualClock;
use crate::balance::SESSION_RECLAIM_WINDOW;
use shared::HeroClass;
use shared::wire::TargetKind;
use shared::map::Team;
use super::*;

#[test]
fn session_id_reclaims_timed_out_player_from_new_endpoint() {
    let mut world = GameWorld::empty();
    let old_addr: SocketAddr = "127.0.0.1:52001".parse().unwrap();
    let new_addr: SocketAddr = "127.0.0.1:52002".parse().unwrap();
    let now = Instant::now();
    let session_id = "stable-player-1".to_string();

    world.ensure_player_for_join(old_addr, Some(session_id.clone()), now);
    let original_id = world.players.get(&old_addr).unwrap().hero.identity.id;
    world.players.get_mut(&old_addr).unwrap().last_seen =
        now - PLAYER_TIMEOUT - Duration::from_secs(1);

    assert!(world.ensure_player_for_join(new_addr, Some(session_id.clone()), now));

    assert!(!world.players.contains_key(&old_addr));
    let reclaimed = world.players.get(&new_addr).unwrap();
    assert_eq!(reclaimed.hero.identity.id, original_id);
    assert_eq!(reclaimed.session_id.as_deref(), Some(session_id.as_str()));
}

#[test]
fn active_session_id_cannot_be_stolen_by_another_endpoint() {
    let mut world = GameWorld::empty();
    let old_addr: SocketAddr = "127.0.0.1:52101".parse().unwrap();
    let new_addr: SocketAddr = "127.0.0.1:52102".parse().unwrap();
    let now = Instant::now();
    let session_id = "active-player-1".to_string();

    assert!(world.ensure_player_for_join(old_addr, Some(session_id.clone()), now));
    world.ensure_connected(new_addr, now + Duration::from_millis(1));
    let placeholder_id = world.players.get(&new_addr).unwrap().hero.identity.id;
    assert!(!world.ensure_player_for_join(
        new_addr,
        Some(session_id),
        now + Duration::from_secs(1)
    ));

    assert!(world.players.contains_key(&old_addr));
    let placeholder = world.players.get(&new_addr).unwrap();
    assert_eq!(placeholder.hero.identity.id, placeholder_id);
    assert!(placeholder.session_id.is_none());
}

#[test]
fn connected_placeholder_can_reclaim_timed_out_session_id() {
    let mut world = GameWorld::empty();
    let old_addr: SocketAddr = "127.0.0.1:52111".parse().unwrap();
    let new_addr: SocketAddr = "127.0.0.1:52112".parse().unwrap();
    let now = Instant::now();
    let session_id = "placeholder-reclaim-1".to_string();

    assert!(world.ensure_player_for_join(old_addr, Some(session_id.clone()), now));
    let original_id = world.players.get(&old_addr).unwrap().hero.identity.id;
    world.players.get_mut(&old_addr).unwrap().last_seen =
        now - PLAYER_TIMEOUT - Duration::from_secs(1);

    world.ensure_connected(new_addr, now + Duration::from_millis(1));
    let placeholder_id = world.players.get(&new_addr).unwrap().hero.identity.id;
    assert_ne!(placeholder_id, original_id);

    assert!(world.ensure_player_for_join(new_addr, Some(session_id.clone()), now));

    assert!(!world.players.contains_key(&old_addr));
    let reclaimed = world.players.get(&new_addr).unwrap();
    assert_eq!(reclaimed.hero.identity.id, original_id);
    assert_eq!(reclaimed.session_id.as_deref(), Some(session_id.as_str()));
}

#[test]
fn stale_disconnected_session_gets_new_player_id() {
    let mut world = GameWorld::empty();
    let old_addr: SocketAddr = "127.0.0.1:52201".parse().unwrap();
    let new_addr: SocketAddr = "127.0.0.1:52202".parse().unwrap();
    let now = Instant::now();
    let session_id = "stale-player-1".to_string();

    world.ensure_connected(old_addr, now);
    let mut old_player = world.players.remove(&old_addr).unwrap();
    old_player.session_id = Some(session_id.clone());
    let original_id = old_player.hero.identity.id;
    world.disconnected_sessions.insert(
        session_id.clone(),
        DisconnectedSession {
            player: old_player,
            disconnected_at: now - SESSION_RECLAIM_WINDOW - Duration::from_secs(1),
        },
    );

    assert!(world.ensure_player_for_join(new_addr, Some(session_id), now));

    assert_ne!(
        world.players.get(&new_addr).unwrap().hero.identity.id,
        original_id
    );
    assert!(world.disconnected_sessions.is_empty());
}

#[test]
fn pre_join_endpoint_is_hidden_and_inert_until_join() {
    let mut world = GameWorld::empty();
    let ghost_addr: SocketAddr = "127.0.0.1:53001".parse().unwrap();
    let enemy_addr: SocketAddr = "127.0.0.1:53002".parse().unwrap();
    let now = Instant::now();

    // A heartbeat-only endpoint (Ping before Join) must not be replicated.
    world.ensure_connected(ghost_addr, now);
    assert!(!world.players.get(&ghost_addr).unwrap().joined);
    assert!(build_players_snapshot(&world, None, now).is_empty());

    // It cannot move...
    let before = world.players.get(&ghost_addr).unwrap().hero.clone();
    handle_transform_request(
        world.players.get_mut(&ghost_addr).unwrap(),
        &world.map_layout,
        before.x + 1.0,
        PLAYER_GROUND_Y,
        before.z,
        1.0,
        now + Duration::from_secs(1),
    );
    let after = world.players.get(&ghost_addr).unwrap().hero.clone();
    assert!((after.x - before.x).abs() < EPSILON);
    assert!((after.yaw - before.yaw).abs() < EPSILON);

    // ...and cannot cast, even at a valid joined enemy in range.
    world.ensure_connected(enemy_addr, now);
    handle_join_request(
        world.players.get_mut(&enemy_addr).unwrap(),
        Team::Blue,
        CharacterChoice::Wang,
        HeroClass::default(),
        None,
        &world.map_layout,
        now,
    );
    let ghost_pos = {
        let ghost = world.players.get(&ghost_addr).unwrap();
        (ghost.hero.x, ghost.hero.z)
    };
    {
        let enemy = world.players.get_mut(&enemy_addr).unwrap();
        enemy.hero.x = ghost_pos.0 + 2.0;
        enemy.hero.z = ghost_pos.1;
    }
    let enemy_id = world.players.get(&enemy_addr).unwrap().hero.identity.id;
    let mana_before = world.players.get(&ghost_addr).unwrap().hero.mana;
    cast_slot(
        &mut world,
        ghost_addr,
        TargetId {
            kind: TargetKind::Player,
            id: enemy_id,
        },
        0,
        now,
    );
    assert!(world.projectiles.is_empty());
    assert!((world.players.get(&ghost_addr).unwrap().hero.mana - mana_before).abs() < EPSILON);

    // Joining flips the flag and the player becomes visible in snapshots.
    handle_join_request(
        world.players.get_mut(&ghost_addr).unwrap(),
        Team::Green,
        CharacterChoice::Ipfs,
        HeroClass::default(),
        None,
        &world.map_layout,
        now,
    );
    let ghost_id = world.players.get(&ghost_addr).unwrap().hero.identity.id;
    let snapshot = build_players_snapshot(&world, None, now);
    assert_eq!(snapshot.len(), 2);
    assert!(snapshot.iter().any(|player| player.id == ghost_id));
}

/// The whole runtime without a socket: the join is a datagram on the memory
/// transport, snapshots come back through it, and the endpoint times out
/// when the injected clock passes `PLAYER_TIMEOUT`, keeping its session for
/// a reclaim. The career store is the guest-only one, as in a dev server
/// without a database.
#[test]
fn runtime_on_memory_transport_and_manual_clock_times_out_a_silent_endpoint() {
    let clock = ManualClock::new(Instant::now());
    let transport = MemoryTransport::new("127.0.0.1:4000".parse().unwrap());
    let mut rt = ServerRuntime::for_test(
        transport.clone(),
        clock.clone(),
        career_backend::MemoryCareer::disabled(53100),
        MatchConfig::dev(),
    );
    let player: SocketAddr = "127.0.0.1:53101".parse().unwrap();
    let packet = ClientPacket::Join {
        prematch: false,
        team: Team::Green,
        character: CharacterChoice::Ipfs,
        hero_class: HeroClass::Mage,
        avatar: None,
        sprite_character: None,
        session_id: Some("manual-clock".into()),
        passport_ticket: None,
    };
    transport.push_inbound(player, serde_json::to_vec(&packet).unwrap());
    let (now, dt) = rt.prepare_tick();
    assert_eq!(now, clock.now());
    assert_eq!(dt, 0.0);
    rt.tick(now, dt);
    assert!(rt.world.players[&player].joined);
    assert_eq!(rt.world.game_state, GameState::Running);
    assert!(
        rt.career_view(player, now)
            .error
            .is_some_and(|e| e.contains("not configured"))
    );
    assert!(
        transport.take_outbound().is_empty(),
        "snapshots are throttled"
    );

    clock.advance(SNAPSHOT_INTERVAL);
    let (now, dt) = rt.prepare_tick();
    assert!((dt - SNAPSHOT_INTERVAL.as_secs_f32()).abs() < EPSILON);
    rt.tick(now, dt);
    let sent = transport.take_outbound();
    assert!(!sent.is_empty());
    assert!(sent.iter().all(|(to, _)| *to == player));

    clock.advance(PLAYER_TIMEOUT + Duration::from_secs(1));
    let (now, dt) = rt.prepare_tick();
    assert_eq!(dt, 0.1, "the wall-clock step is clamped");
    rt.tick(now, dt);
    assert!(!rt.world.players.contains_key(&player));
    assert!(rt.world.disconnected_sessions.contains_key("manual-clock"));
    assert!(transport.take_outbound().is_empty());
}
