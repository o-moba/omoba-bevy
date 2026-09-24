use super::*;

fn horizontal_distance(a: &PlayerState, b: &PlayerState) -> f32 {
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
    player.state.mana = 10.0;
    player.state.max_mana = MAX_MANA;

    regenerate_mana(&mut world.players, 2.5);
    let expected = 10.0 + MANA_REGEN_PER_SECOND * 2.5;
    let current = world.players.get(&addr).unwrap().state.mana;
    assert!((current - expected).abs() < EPSILON);

    regenerate_mana(&mut world.players, 100.0);
    let clamped = world.players.get(&addr).unwrap().state.mana;
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
    let start = world.players.get(&addr).unwrap().state.clone();
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

    let after_normal = world.players.get(&addr).unwrap().state.clone();
    assert!((after_normal.x - normal_x).abs() < EPSILON);
    assert!((after_normal.y - PLAYER_GROUND_Y).abs() < EPSILON);
    assert!((after_normal.yaw - 0.25).abs() < EPSILON);

    let teleport_at = normal_at + Duration::from_millis(50);
    let before_teleport = world.players.get(&addr).unwrap().state.clone();
    handle_transform_request(
        world.players.get_mut(&addr).unwrap(),
        &world.map_layout,
        before_teleport.x + 500.0,
        PLAYER_GROUND_Y,
        before_teleport.z,
        0.5,
        teleport_at,
    );

    let after_teleport = world.players.get(&addr).unwrap().state.clone();
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
    let after_invalid = world.players.get(&addr).unwrap().state.clone();
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
        player.state.x = world.map_layout.max_x - 0.1;
        player.state.z = world.map_layout.max_z - 0.1;
        player.last_movement_at = now;
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
    assert!(player.state.x <= world.map_layout.max_x);
    assert!(player.state.z <= world.map_layout.max_z);
    assert!(player.state.x >= world.map_layout.min_x);
    assert!(player.state.z >= world.map_layout.min_z);
}
