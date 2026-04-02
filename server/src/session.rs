use super::*;

pub(crate) fn ensure_player_connected(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    map_layout: &MapLayoutState,
    addr: SocketAddr,
    next_player_id: &mut u64,
    now: Instant,
) {
    players.entry(addr).or_insert_with(|| {
        let player_id = *next_player_id;
        *next_player_id += 1;
        println!("Player {player_id} connected from {addr}");
        let spawn = spawn_position_for_team(map_layout, Team::Green);

        ConnectedPlayer {
            state: PlayerState {
                id: player_id,
                x: spawn.x,
                y: 0.5,
                z: spawn.z,
                yaw: 0.0,
                team: Team::Green,
                hp: MAX_HP,
                max_hp: MAX_HP,
                mana: MAX_MANA,
                max_mana: MAX_MANA,
                gold: 0,
                xp: 0,
                level: STARTING_LEVEL,
                next_level_xp: xp_threshold_for_level(STARTING_LEVEL),
                skill_points: 0,
                character: default_character_choice(),
            },
            last_seen: now,
            last_cast_at: None,
            respawn_at: None,
        }
    });
}

pub(crate) fn regenerate_mana(players: &mut HashMap<SocketAddr, ConnectedPlayer>, dt: f32) {
    for player in players.values_mut() {
        if player.state.hp <= 0.0 {
            continue;
        }
        if player.state.max_mana <= 0.0 {
            player.state.max_mana = MAX_MANA;
        }
        player.state.mana =
            (player.state.mana + MANA_REGEN_PER_SECOND * dt).clamp(0.0, player.state.max_mana);
    }
}

pub(crate) fn handle_join_request(
    player: &mut ConnectedPlayer,
    team: Team,
    character: CharacterChoice,
    map_layout: &MapLayoutState,
) {
    println!(
        "Player {} joined team {:?} as {:?}",
        player.state.id, team, character
    );
    player.state.team = team;
    player.state.character = character;
    let spawn = spawn_position_for_team(map_layout, team);
    player.state.x = spawn.x;
    player.state.y = 0.5;
    player.state.z = spawn.z;
    player.state.yaw = 0.0;
    player.state.hp = MAX_HP;
    player.state.max_hp = MAX_HP;
    player.state.mana = MAX_MANA;
    player.state.max_mana = MAX_MANA;
    player.state.gold = 0;
    player.state.xp = 0;
    player.state.level = STARTING_LEVEL;
    player.state.next_level_xp = xp_threshold_for_level(STARTING_LEVEL);
    player.state.skill_points = 0;
    player.last_cast_at = None;
    player.respawn_at = None;
}

pub(crate) fn handle_respawns(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    structures: &HashMap<u64, Structure>,
    map_layout: &MapLayoutState,
    game_state: &GameState,
    now: Instant,
) {
    if !matches!(game_state, GameState::Running) {
        return;
    }
    for player in players.values_mut() {
        let Some(respawn_at) = player.respawn_at else {
            continue;
        };
        if now < respawn_at {
            continue;
        }
        let spawn = spawn_position_for_team_from_base(structures, map_layout, player.state.team);
        player.state.x = spawn.x;
        player.state.y = 0.5;
        player.state.z = spawn.z;
        player.state.yaw = 0.0;
        player.state.hp = player.state.max_hp;
        player.state.mana = player.state.max_mana;
        player.respawn_at = None;
        player.last_cast_at = None;
    }
}

pub(crate) fn reset_match(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    structures: &mut HashMap<u64, Structure>,
    minions: &mut HashMap<u64, Minion>,
    projectiles: &mut HashMap<u64, Projectile>,
    map_layout: &MapLayoutState,
    last_wave_spawn_at: &mut Instant,
    game_state: &mut GameState,
) {
    println!("Resetting match for rematch");
    // Reset structures HP
    for structure in structures.values_mut() {
        let max = structure.state.max_hp;
        structure.state.hp = max;
        structure.last_attack_at = None;
    }
    // Clear minions and projectiles
    minions.clear();
    projectiles.clear();
    // Reset wave timer so first wave isn't immediate
    *last_wave_spawn_at = Instant::now();
    // Reset all players to spawn
    for player in players.values_mut() {
        let spawn = spawn_position_for_team(map_layout, player.state.team);
        player.state.x = spawn.x;
        player.state.y = 0.5;
        player.state.z = spawn.z;
        player.state.yaw = 0.0;
        player.state.hp = MAX_HP;
        player.state.max_hp = MAX_HP;
        player.state.mana = MAX_MANA;
        player.state.max_mana = MAX_MANA;
        player.state.gold = 0;
        player.state.xp = 0;
        player.last_cast_at = None;
        player.respawn_at = None;
    }
    *game_state = GameState::Running;
}
