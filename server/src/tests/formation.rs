use super::*;

// ---- TASK-22: matchmaking / match formation ----

fn test_addr(index: u16) -> SocketAddr {
    format!("127.0.0.1:{}", 40_000 + index).parse().unwrap()
}

/// Builds a lobby world with `count` joined players, teams assigned
/// release-style.
fn joined_roster(count: u32, team_size: u32) -> GameWorld {
    let mut world = GameWorld::empty();
    world.game_state = GameState::Lobby;
    for index in 0..count {
        let addr = test_addr(index as u16);
        world.ensure_connected(addr, Instant::now());
        let team = assign_release_team(&world.players, team_size)
            .expect("roster should not be full while building");
        let player = world.players.get_mut(&addr).unwrap();
        player.joined = true;
        player.state.team = team;
    }
    world
}

#[test]
fn match_mode_parses_with_release_as_safe_default() {
    assert_eq!(parse_match_mode(None), MatchMode::Release);
    assert_eq!(parse_match_mode(Some("release")), MatchMode::Release);
    assert_eq!(parse_match_mode(Some("normal")), MatchMode::Release);
    assert_eq!(parse_match_mode(Some("dev")), MatchMode::Dev);
    assert_eq!(parse_match_mode(Some("DEBUG")), MatchMode::Dev);
    assert_eq!(parse_match_mode(Some("garbage")), MatchMode::Release);
}

#[test]
fn team_size_parses_and_clamps() {
    assert_eq!(parse_team_size(None), DEFAULT_TEAM_SIZE);
    assert_eq!(parse_team_size(Some("3")), 3);
    assert_eq!(parse_team_size(Some("0")), MIN_TEAM_SIZE);
    assert_eq!(parse_team_size(Some("99")), MAX_TEAM_SIZE);
    assert_eq!(parse_team_size(Some("not-a-number")), DEFAULT_TEAM_SIZE);
}

#[test]
fn release_solo_join_forms_but_never_starts() {
    let config = MatchConfig::release(5);
    let mut world = joined_roster(1, config.team_size);
    world.game_state = GameState::Lobby;
    let now = Instant::now();

    advance_formation_on_join(&mut world, config, now);
    assert_eq!(
        world.game_state,
        GameState::Forming {
            ready: 1,
            needed: 10
        }
    );
    // Many ticks later the solo player is still waiting.
    for _ in 0..100 {
        tick_match_formation(&mut world, config, 0.5, now);
    }
    assert_eq!(
        world.game_state,
        GameState::Forming {
            ready: 1,
            needed: 10
        }
    );
}

#[test]
fn release_match_waits_at_nine_players() {
    let config = MatchConfig::release(5);
    let mut world = joined_roster(9, config.team_size);
    world.game_state = GameState::Forming {
        ready: 1,
        needed: 10,
    };
    tick_match_formation(&mut world, config, 0.01, Instant::now());
    assert_eq!(
        world.game_state,
        GameState::Forming {
            ready: 9,
            needed: 10
        }
    );
}

#[test]
fn release_match_starts_after_full_roster_and_countdown() {
    let config = MatchConfig::release(5);
    let mut world = joined_roster(10, config.team_size);
    world.game_state = GameState::Forming {
        ready: 9,
        needed: 10,
    };
    let now = Instant::now();

    tick_match_formation(&mut world, config, 0.01, now);
    assert_eq!(
        world.game_state,
        GameState::Starting {
            countdown_ms: MATCH_START_COUNTDOWN_MS
        }
    );

    // Countdown ticks down monotonically, then flips to Running.
    tick_match_formation(&mut world, config, 1.0, now);
    assert_eq!(
        world.game_state,
        GameState::Starting {
            countdown_ms: MATCH_START_COUNTDOWN_MS - 1_000
        }
    );
    tick_match_formation(&mut world, config, 5.0, now);
    assert_eq!(world.game_state, GameState::Running);
}

#[test]
fn release_teams_form_five_vs_five_and_full_match_rejects_joins() {
    let team_size = 5;
    let world = joined_roster(10, team_size);
    let (green, blue) = joined_team_counts(&world.players);
    assert_eq!(green, 5);
    assert_eq!(blue, 5);
    // The 11th player has no seat.
    assert_eq!(assign_release_team(&world.players, team_size), None);
}

#[test]
fn release_team_assignment_always_fills_smaller_team() {
    let mut world = joined_roster(3, 5);
    let (green, blue) = joined_team_counts(&world.players);
    assert_eq!((green, blue), (2, 1));
    // Next assignment must balance to 2v2.
    let team = assign_release_team(&world.players, 5).unwrap();
    assert_eq!(team, Team::Blue);
    let addr = test_addr(99);
    world.ensure_connected(addr, Instant::now());
    let player = world.players.get_mut(&addr).unwrap();
    player.joined = true;
    player.state.team = team;
    assert_eq!(joined_team_counts(&world.players), (2, 2));
}

#[test]
fn dev_mode_first_join_starts_match_immediately() {
    let config = MatchConfig::dev();
    let mut world = joined_roster(1, config.team_size);
    world.game_state = GameState::Lobby;

    advance_formation_on_join(&mut world, config, Instant::now());
    assert_eq!(world.game_state, GameState::Running);
}

#[test]
fn starting_countdown_rolls_back_when_a_player_drops() {
    let config = MatchConfig::release(1);
    let mut world = joined_roster(2, config.team_size);
    world.game_state = GameState::Starting {
        countdown_ms: MATCH_START_COUNTDOWN_MS,
    };
    // One player disconnects mid-countdown.
    world.players.remove(&test_addr(0));
    tick_match_formation(&mut world, config, 0.01, Instant::now());
    assert_eq!(
        world.game_state,
        GameState::Forming {
            ready: 1,
            needed: 2
        }
    );
}

#[test]
fn forming_returns_to_lobby_when_queue_empties() {
    let config = MatchConfig::release(5);
    let mut world = GameWorld::empty();
    world.game_state = GameState::Forming {
        ready: 1,
        needed: 10,
    };
    tick_match_formation(&mut world, config, 0.01, Instant::now());
    assert_eq!(world.game_state, GameState::Lobby);
}
