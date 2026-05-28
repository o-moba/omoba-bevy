use super::*;

const MAX_SESSION_ID_LEN: usize = 64;

pub(crate) fn normalize_session_id(raw: Option<String>) -> Option<String> {
    let raw = raw?;
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_SESSION_ID_LEN {
        return None;
    }
    if !trimmed
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return None;
    }
    Some(trimmed.to_string())
}

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
                y: PLAYER_GROUND_Y,
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
            session_id: None,
            last_seen: now,
            last_movement_at: now,
            last_cast_at: None,
            respawn_at: None,
        }
    });
}

pub(crate) fn ensure_player_for_join(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    disconnected_sessions: &mut HashMap<String, DisconnectedSession>,
    map_layout: &MapLayoutState,
    addr: SocketAddr,
    session_id: Option<String>,
    next_player_id: &mut u64,
    now: Instant,
) -> bool {
    let Some(session_id) = session_id else {
        ensure_player_connected(players, map_layout, addr, next_player_id, now);
        return true;
    };

    if players
        .get(&addr)
        .and_then(|player| player.session_id.as_deref())
        == Some(session_id.as_str())
    {
        return true;
    }

    let active_match = players
        .iter()
        .find(|(existing_addr, player)| {
            **existing_addr != addr && player.session_id.as_deref() == Some(session_id.as_str())
        })
        .map(|(existing_addr, player)| (*existing_addr, player.last_seen));

    if let Some((existing_addr, last_seen)) = active_match {
        if now.duration_since(last_seen) <= PLAYER_TIMEOUT {
            eprintln!(
                "Rejecting session id reuse from {addr}: session is still active at {existing_addr}"
            );
            return false;
        }

        if let Some(mut player) = players.remove(&existing_addr) {
            println!(
                "Reclaiming timed-out player {} for session {session_id} from {existing_addr} to {addr}",
                player.state.id
            );
            player.session_id = Some(session_id);
            player.last_seen = now;
            player.last_movement_at = now;
            players.insert(addr, player);
            return true;
        }
    }

    if let Some(mut disconnected) = disconnected_sessions.remove(&session_id) {
        if now.duration_since(disconnected.disconnected_at) <= SESSION_RECLAIM_WINDOW {
            println!(
                "Reclaiming disconnected player {} for session {session_id} from new endpoint {addr}",
                disconnected.player.state.id
            );
            disconnected.player.session_id = Some(session_id);
            disconnected.player.last_seen = now;
            disconnected.player.last_movement_at = now;
            players.insert(addr, disconnected.player);
            return true;
        }
    }

    if let Some(player) = players.get_mut(&addr) {
        player.session_id = Some(session_id);
        return true;
    }

    ensure_player_connected(players, map_layout, addr, next_player_id, now);
    if let Some(player) = players.get_mut(&addr) {
        player.session_id = Some(session_id);
    }
    true
}

#[cfg(test)]
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
    now: Instant,
) {
    println!(
        "Player {} joined team {:?} as {:?}",
        player.state.id, team, character
    );
    player.state.team = team;
    player.state.character = character;
    let spawn = spawn_position_for_team(map_layout, team);
    player.state.x = spawn.x;
    player.state.y = PLAYER_GROUND_Y;
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
    player.last_seen = now;
    player.last_movement_at = now;
    player.last_cast_at = None;
    player.respawn_at = None;
}

pub(crate) fn handle_transform_request(
    player: &mut ConnectedPlayer,
    map_layout: &MapLayoutState,
    x: f32,
    y: f32,
    z: f32,
    yaw: f32,
    now: Instant,
) {
    if !x.is_finite() || !y.is_finite() || !z.is_finite() {
        return;
    }

    let requested = map_layout.clamp_player_position(Vec3f::new(x, y, z));
    let current = Vec3f::new(player.state.x, PLAYER_GROUND_Y, player.state.z);
    let dx = requested.x - current.x;
    let dz = requested.z - current.z;
    let distance = (dx * dx + dz * dz).sqrt();
    let elapsed = now
        .duration_since(player.last_movement_at)
        .as_secs_f32()
        .clamp(0.0, MOVEMENT_MAX_DELTA_SECONDS);
    let max_distance = PLAYER_SPEED * elapsed + MOVEMENT_POSITION_TOLERANCE;

    let accepted = if distance <= max_distance || distance <= 0.000_1 {
        requested
    } else {
        let scale = max_distance / distance;
        map_layout.clamp_player_position(Vec3f::new(
            current.x + dx * scale,
            PLAYER_GROUND_Y,
            current.z + dz * scale,
        ))
    };

    player.state.x = accepted.x;
    player.state.y = PLAYER_GROUND_Y;
    player.state.z = accepted.z;
    if yaw.is_finite() {
        player.state.yaw = yaw;
    }
    player.last_movement_at = now;
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
        player.state.y = PLAYER_GROUND_Y;
        player.state.z = spawn.z;
        player.state.yaw = 0.0;
        player.state.hp = player.state.max_hp;
        player.state.mana = player.state.max_mana;
        player.respawn_at = None;
        player.last_movement_at = now;
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
        player.state.y = PLAYER_GROUND_Y;
        player.state.z = spawn.z;
        player.state.yaw = 0.0;
        player.state.hp = MAX_HP;
        player.state.max_hp = MAX_HP;
        player.state.mana = MAX_MANA;
        player.state.max_mana = MAX_MANA;
        player.state.gold = 0;
        player.state.xp = 0;
        player.last_movement_at = Instant::now();
        player.last_cast_at = None;
        player.respawn_at = None;
    }
    *game_state = GameState::Running;
}
