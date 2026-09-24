use super::*;

#[test]
fn progression_levels_up_and_scales_stats() {
    let mut world = GameWorld::empty();
    let addr: SocketAddr = "127.0.0.1:34568".parse().unwrap();
    let now = Instant::now();

    world.ensure_connected(addr, now);
    let player = world.players.get_mut(&addr).unwrap();
    let first_threshold = player.state.next_level_xp;
    let second_threshold = xp_threshold_for_level(STARTING_LEVEL + 1);

    grant_player_xp(&mut player.state, first_threshold + second_threshold + 17);

    assert_eq!(player.state.level, STARTING_LEVEL + 2);
    assert_eq!(player.state.skill_points, 2);
    assert_eq!(player.state.xp, 17);
    assert_eq!(
        player.state.next_level_xp,
        xp_threshold_for_level(STARTING_LEVEL + 2)
    );
    assert!((player.state.max_hp - (MAX_HP + LEVEL_UP_HP_BONUS * 2.0)).abs() < EPSILON);
    assert!((player.state.max_mana - (MAX_MANA + LEVEL_UP_MANA_BONUS * 2.0)).abs() < EPSILON);
}

#[test]
fn respawn_restores_scaled_maximums() {
    let mut world = GameWorld::empty();
    world.structures = build_structures(&world.map_layout);
    let addr: SocketAddr = "127.0.0.1:34569".parse().unwrap();
    let now = Instant::now();

    world.ensure_connected(addr, now);
    let player = world.players.get_mut(&addr).unwrap();
    let first_threshold = player.state.next_level_xp;
    let second_threshold = xp_threshold_for_level(STARTING_LEVEL + 1);
    grant_player_xp(&mut player.state, first_threshold + second_threshold);
    player.state.hp = 0.0;
    player.state.mana = 0.0;
    player.timers.respawn_at = Some(now - Duration::from_millis(1));

    handle_respawns(&mut world, now);

    let player = world.players.get(&addr).unwrap();
    assert_eq!(player.state.level, STARTING_LEVEL + 2);
    assert!(player.state.max_hp > MAX_HP);
    assert!(player.state.max_mana > MAX_MANA);
    assert!((player.state.hp - player.state.max_hp).abs() < EPSILON);
    assert!((player.state.mana - player.state.max_mana).abs() < EPSILON);
}
