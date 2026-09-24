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
    assert!(build_players_snapshot(&world, now).is_empty());

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
    let snapshot = build_players_snapshot(&world, now);
    assert_eq!(snapshot.len(), 2);
    assert!(snapshot.iter().any(|player| player.id == ghost_id));
}
