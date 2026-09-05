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
        println!("Endpoint {addr} connected (pre-join), reserved player id {player_id}");
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
                ranks: [1; 4],
                character: default_character_choice(),
                hero_class: HeroClass::default(),
                avatar: None,
                sprite_character: None,
                action_sequence: 0,
                action_kind: PlayerActionKind::None,
                action_slot: 0,
            },
            joined: false,
            session_id: None,
            framed_snapshots: false,
            protocol_compatible: true,
            join_error: None,
            last_seen: now,
            last_movement_at: now,
            last_cast_at: [None; 4],
            respawn_at: None,
            god_mode: false,
            speed_mult: 1.0,
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
    let framed_snapshots = players
        .get(&addr)
        .is_some_and(|player| player.framed_snapshots);
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
            player.framed_snapshots = framed_snapshots;
            player.protocol_compatible = true;
            player.join_error = None;
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
            disconnected.player.framed_snapshots = framed_snapshots;
            disconnected.player.protocol_compatible = true;
            disconnected.player.join_error = None;
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

#[cfg(test)]
pub(crate) fn handle_join_request(
    player: &mut ConnectedPlayer,
    team: Team,
    character: CharacterChoice,
    hero_class: HeroClass,
    avatar: Option<&str>,
    map_layout: &MapLayoutState,
    now: Instant,
) {
    handle_join_request_with_sprite(
        player, team, character, hero_class, avatar, None, map_layout, now,
    );
}

pub(crate) fn handle_join_request_with_sprite(
    player: &mut ConnectedPlayer,
    team: Team,
    character: CharacterChoice,
    hero_class: HeroClass,
    avatar: Option<&str>,
    sprite_character: Option<&str>,
    map_layout: &MapLayoutState,
    now: Instant,
) {
    if player.joined {
        return;
    }
    // Unknown avatar slugs are dropped (client falls back to the default model);
    // unknown class strings already decoded to the default class in serde.
    let normalized_avatar = shared::normalize_avatar_slug(avatar);
    let normalized_sprite = shared::normalize_sprite_character_id(sprite_character);
    if avatar.is_some() && normalized_avatar.is_none() {
        eprintln!(
            "Player {} requested unknown avatar {:?}; falling back to default model",
            player.state.id, avatar
        );
    }
    if sprite_character.is_some_and(|requested| requested.trim() != normalized_sprite) {
        eprintln!(
            "Player {} requested unknown sprite {:?}; falling back to {:?}",
            player.state.id, sprite_character, normalized_sprite
        );
    }
    println!(
        "Player {} joined team {:?} as {:?} (class {}, avatar {:?}, sprite {:?})",
        player.state.id,
        team,
        character,
        hero_class.id(),
        normalized_avatar,
        normalized_sprite
    );
    player.joined = true;
    player.state.team = team;
    player.state.character = character;
    player.state.hero_class = hero_class;
    player.state.avatar = normalized_avatar.map(str::to_owned);
    player.state.sprite_character = Some(normalized_sprite.to_owned());
    reset_player_round(player, map_layout, now);
}

/// Gameplay reset shared by fresh admission and every subsequent round.
fn reset_player_round(player: &mut ConnectedPlayer, map_layout: &MapLayoutState, now: Instant) {
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
    player.state.level = STARTING_LEVEL;
    player.state.next_level_xp = xp_threshold_for_level(STARTING_LEVEL);
    player.state.skill_points = 0;
    player.state.ranks = [1; 4];
    player.state.action_sequence = 0;
    player.state.action_kind = PlayerActionKind::None;
    player.state.action_slot = 0;
    player.last_movement_at = now;
    player.last_cast_at = [None; 4];
    player.respawn_at = None;
    player.god_mode = false;
    player.speed_mult = 1.0;
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
    // Movement authority only applies to joined players; pre-join endpoints
    // (heartbeat-only connections) have no simulated presence to move.
    if !player.joined {
        return;
    }
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
    let speed_mult = player.speed_mult.max(1.0);
    let max_distance = PLAYER_SPEED * speed_mult * elapsed + MOVEMENT_POSITION_TOLERANCE;

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
        player.last_cast_at = [None; 4];
    }
}

/// Canonical clean-round state, before formation/start arms the clocks.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reset_match(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    structures: &mut HashMap<u64, Structure>,
    minions: &mut HashMap<u64, Minion>,
    projectiles: &mut HashMap<u64, Projectile>,
    neutrals: &mut HashMap<u64, Neutral>,
    team_buffs: &mut TeamBuffs,
    map_layout: &MapLayoutState,
    last_wave_spawn_at: &mut Instant,
    game_state: &mut GameState,
    now: Instant,
) {
    *structures = build_structures(map_layout);
    minions.clear();
    projectiles.clear();
    let mut next_neutral_id = 9_001;
    *neutrals = build_neutral_camps(&mut next_neutral_id);
    neutrals.extend(build_boss_neutrals(&mut next_neutral_id));
    team_buffs.clear();
    *last_wave_spawn_at = now;
    for player in players.values_mut() {
        reset_player_round(player, map_layout, now);
        player.join_error = None;
    }
    *game_state = GameState::Lobby;
}

/// Count reserved seats as well as connected heroes when assigning release teams.
pub(crate) fn assign_reserved_release_team(
    players: &HashMap<SocketAddr, ConnectedPlayer>,
    reservations: &HashMap<String, DisconnectedSession>,
    team_size: u32,
) -> Option<Team> {
    let (mut green, mut blue) = joined_team_counts(players);
    for session in reservations
        .values()
        .filter(|session| session.player.joined)
    {
        match session.player.state.team {
            Team::Green => green += 1,
            Team::Blue => blue += 1,
        }
    }
    if green >= team_size && blue >= team_size {
        None
    } else if green <= blue && green < team_size {
        Some(Team::Green)
    } else {
        Some(Team::Blue)
    }
}

impl ServerRuntime {
    pub(crate) fn maintain_roster(&mut self, now: Instant) {
        let expired = self
            .players
            .iter()
            .filter(|(_, player)| now.saturating_duration_since(player.last_seen) > PLAYER_TIMEOUT)
            .map(|(addr, _)| *addr)
            .collect::<Vec<_>>();
        for addr in expired {
            let player = self.players.remove(&addr).unwrap();
            let disconnected_at = player.last_seen + PLAYER_TIMEOUT;
            if player.joined {
                println!(
                    "MATCH_METRIC event=disconnect epoch={} match={} player={} elapsed_ms={}",
                    self.server_epoch,
                    self.match_id,
                    player.state.id,
                    self.elapsed_match_ms(now)
                );
                if let Some(session_id) = player.session_id.clone() {
                    self.disconnected_sessions.insert(
                        session_id,
                        DisconnectedSession {
                            player,
                            disconnected_at,
                        },
                    );
                }
            }
        }
        self.disconnected_sessions.retain(|_, session| {
            now.saturating_duration_since(session.disconnected_at) <= SESSION_RECLAIM_WINDOW
        });
        if joined_count(&self.players) > 0 {
            self.empty_since = None;
        } else if !matches!(self.game_state, GameState::Lobby)
            || !self.disconnected_sessions.is_empty()
        {
            let empty_since = self.empty_since.get_or_insert(now);
            if now.saturating_duration_since(*empty_since) >= EMPTY_ROSTER_GRACE {
                println!(
                    "MATCH_METRIC event=abandoned epoch={} match={} elapsed_ms={}",
                    self.server_epoch,
                    self.match_id,
                    self.elapsed_match_ms(now)
                );
                self.restart_round(now);
            }
        }
    }

    pub(crate) fn restart_round(&mut self, now: Instant) {
        reset_match(
            &mut self.players,
            &mut self.structures,
            &mut self.minions,
            &mut self.projectiles,
            &mut self.neutrals,
            &mut self.team_buffs,
            &self.map_layout,
            &mut self.last_wave_spawn_at,
            &mut self.game_state,
            now,
        );
        // Only currently connected admitted identities participate in a rematch.
        self.disconnected_sessions.clear();
        self.match_id = self.match_id.saturating_add(1);
        self.match_started_at = None;
        self.victory_at = None;
        self.empty_since = None;
        self.metrics_players.clear();
        self.metrics_objectives.clear();
        if joined_count(&self.players) > 0 {
            advance_formation_on_join(
                &mut self.game_state,
                &self.players,
                &mut self.neutrals,
                self.match_config,
                now,
            );
        }
        self.track_round_start(now);
        println!(
            "MATCH_METRIC event=round_reset epoch={} match={} connected={}",
            self.server_epoch,
            self.match_id,
            joined_count(&self.players)
        );
    }

    pub(crate) fn track_round_start(&mut self, now: Instant) {
        if matches!(self.game_state, GameState::Running) && self.match_started_at.is_none() {
            self.match_started_at = Some(now);
            self.last_wave_spawn_at = now - (MINION_WAVE_INTERVAL - FIRST_MINION_WAVE_DELAY);
            println!(
                "MATCH_METRIC event=round_start epoch={} match={} connected={} first_wave_secs={}",
                self.server_epoch,
                self.match_id,
                joined_count(&self.players),
                FIRST_MINION_WAVE_DELAY.as_secs()
            );
        }
    }

    fn elapsed_match_ms(&self, now: Instant) -> u128 {
        self.match_started_at
            .map_or(0, |start| now.saturating_duration_since(start).as_millis())
    }

    pub(crate) fn record_match_metrics(&mut self, now: Instant) {
        let elapsed = self.elapsed_match_ms(now);
        for player in self.players.values().filter(|player| player.joined) {
            let alive = player.state.hp > 0.0;
            let previous = self
                .metrics_players
                .insert(player.state.id, (player.state.level, alive));
            if previous.is_none_or(|(level, _)| level != player.state.level) {
                println!(
                    "MATCH_METRIC event=progression epoch={} match={} player={} team={:?} level={} xp={} gold={} elapsed_ms={elapsed}",
                    self.server_epoch,
                    self.match_id,
                    player.state.id,
                    player.state.team,
                    player.state.level,
                    player.state.xp,
                    player.state.gold
                );
            }
            if previous.is_some_and(|(_, was_alive)| was_alive) && !alive {
                println!(
                    "MATCH_METRIC event=death epoch={} match={} player={} elapsed_ms={elapsed}",
                    self.server_epoch, self.match_id, player.state.id
                );
            }
        }
        for structure in self
            .structures
            .values()
            .filter(|structure| structure.state.hp <= 0.0)
        {
            if self.metrics_objectives.insert(structure.state.id) {
                println!(
                    "MATCH_METRIC event=objective epoch={} match={} kind={:?} team={:?} id={} elapsed_ms={elapsed}",
                    self.server_epoch,
                    self.match_id,
                    structure.state.kind,
                    structure.state.team,
                    structure.state.id
                );
            }
        }
        if let GameState::Victory { winner } = self.game_state {
            if self.victory_at.is_none() {
                self.victory_at = Some(now);
                println!(
                    "MATCH_METRIC event=victory epoch={} match={} winner={winner:?} duration_ms={elapsed}",
                    self.server_epoch, self.match_id
                );
            }
        }
    }
}
