use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use shared::HeroClass;
use shared::map::Team;
use shared::wire::{CharacterChoice, GameState};

use crate::balance::{
    EMPTY_ROSTER_GRACE, FIRST_MINION_WAVE_DELAY, MINION_WAVE_INTERVAL, MOVEMENT_MAX_DELTA_SECONDS,
    PLAYER_GROUND_Y, SESSION_RECLAIM_WINDOW,
};
use crate::combat_feedback::CombatLog;
use crate::entities::{ConnectedPlayer, DisconnectedSession, MapLayoutState, Structure, Vec3f};
use crate::formation::{advance_formation_on_join, joined_count, joined_team_counts};
use crate::game_world::GameWorld;
use crate::hero::{Hero, HeroEconomy, HeroProgress};
use crate::hero_stats::StatModifiers;
use crate::hero_timers::HeroTimers;
use crate::neutrals::{build_boss_neutrals, build_neutral_camps};
use crate::runtime::{PLAYER_TIMEOUT, ServerRuntime};
use crate::world::{
    build_configured_structures, spawn_position_for_team, spawn_position_for_team_from_base,
    structure_collision_radius,
};
use crate::{forest_pickups, hero_stats, prematch};

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

impl GameWorld {
    /// Reserves a player id and a pre-join placeholder for a new endpoint.
    pub(crate) fn ensure_connected(&mut self, addr: SocketAddr, now: Instant) {
        let Self {
            players,
            map_layout,
            next_player_id,
            ..
        } = self;
        players.entry(addr).or_insert_with(|| {
            let player_id = *next_player_id;
            *next_player_id += 1;
            println!("Endpoint {addr} connected (pre-join), reserved player id {player_id}");
            let spawn = spawn_position_for_team(map_layout, Team::Green);

            ConnectedPlayer {
                career_profile: None,
                career_capable: false,
                draft: Default::default(),
                joined: false,
                session_id: None,
                framed_snapshots: false,
                protocol_compatible: true,
                join_error: None,
                last_seen: now,
                timers: HeroTimers::new(now),
                modifiers: StatModifiers::default(),
                hero: Hero::new(player_id, spawn),
                economy: HeroEconomy::starting(),
            }
        });
    }

    /// Admits `addr` for a join, reclaiming a retained session when allowed.
    pub(crate) fn ensure_player_for_join(
        &mut self,
        addr: SocketAddr,
        session_id: Option<String>,
        now: Instant,
    ) -> bool {
        let players = &mut self.players;
        let career_capable = players
            .get(&addr)
            .is_some_and(|player| player.career_capable);
        let framed_snapshots = players
            .get(&addr)
            .is_some_and(|player| player.framed_snapshots);
        let Some(session_id) = session_id else {
            self.ensure_connected(addr, now);
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
                    "Reclaiming timed-out player {} from {existing_addr} to {addr}",
                    player.hero.identity.id
                );
                player.framed_snapshots = framed_snapshots;
                player.career_capable |= career_capable;
                player.protocol_compatible = true;
                player.join_error = None;
                player.session_id = Some(session_id);
                player.last_seen = now;
                player.timers.last_movement_at = now;
                players.insert(addr, player);
                return true;
            }
        }

        if let Some(mut disconnected) = self.disconnected_sessions.remove(&session_id) {
            if now.duration_since(disconnected.disconnected_at) <= SESSION_RECLAIM_WINDOW {
                println!(
                    "Reclaiming disconnected player {} from new endpoint {addr}",
                    disconnected.player.hero.identity.id
                );
                disconnected.player.framed_snapshots = framed_snapshots;
                disconnected.player.career_capable |= career_capable;
                disconnected.player.protocol_compatible = true;
                disconnected.player.join_error = None;
                disconnected.player.session_id = Some(session_id);
                disconnected.player.last_seen = now;
                disconnected.player.timers.last_movement_at = now;
                players.insert(addr, disconnected.player);
                return true;
            }
        }

        if let Some(player) = players.get_mut(&addr) {
            player.session_id = Some(session_id);
            return true;
        }

        self.ensure_connected(addr, now);
        if let Some(player) = self.players.get_mut(&addr) {
            player.session_id = Some(session_id);
        }
        true
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
    let normalized_avatar = omoba_passport::avatars::normalize_avatar_slug(avatar);
    let normalized_sprite = shared::normalize_sprite_character_id(sprite_character);
    if avatar.is_some() && normalized_avatar.is_none() {
        eprintln!(
            "Player {} requested unknown avatar {:?}; falling back to default model",
            player.hero.identity.id, avatar
        );
    }
    if sprite_character.is_some_and(|requested| requested.trim() != normalized_sprite) {
        eprintln!(
            "Player {} requested unknown sprite {:?}; falling back to {:?}",
            player.hero.identity.id, sprite_character, normalized_sprite
        );
    }
    println!(
        "Player {} joined team {:?} as {:?} (class {}, avatar {:?}, sprite {:?})",
        player.hero.identity.id,
        team,
        character,
        hero_class.id(),
        normalized_avatar,
        normalized_sprite
    );
    player.joined = true;
    player.hero.identity.team = team;
    player.hero.identity.character = character;
    player.hero.identity.hero_class = hero_class;
    player.hero.identity.avatar = normalized_avatar.map(str::to_owned);
    player.hero.identity.sprite_character = Some(normalized_sprite.to_owned());
    reset_player_round(player, map_layout, now);
}

/// Gameplay reset shared by fresh admission and every subsequent round.
pub(crate) fn reset_player_round(
    player: &mut ConnectedPlayer,
    map_layout: &MapLayoutState,
    now: Instant,
) {
    let spawn = spawn_position_for_team(map_layout, player.hero.identity.team);
    player.hero.x = spawn.x;
    player.hero.y = PLAYER_GROUND_Y;
    player.hero.z = spawn.z;
    player.hero.yaw = 0.0;
    player.modifiers = StatModifiers::default();
    player.economy = HeroEconomy::starting();
    player.hero.progress = HeroProgress::starting();
    player.hero.max_hp = hero_stats::max_hp(player);
    player.hero.hp = player.hero.max_hp;
    player.hero.max_mana = hero_stats::max_mana(player);
    player.hero.mana = player.hero.max_mana;
    player.hero.utility = Default::default();
    player.hero.last_action = Default::default();
    player.timers.dash_ready_at = None;
    player.timers.haste_ready_at = None;
    player.timers.haste_expires_at = None;
    player.timers.last_movement_at = now;
    player.timers.last_cast_at = [None; 4];
    player.timers.last_basic_attack_at = None;
    player.timers.respawn_at = None;
}

#[cfg(test)]
pub(crate) fn handle_transform_request(
    player: &mut ConnectedPlayer,
    map_layout: &MapLayoutState,
    x: f32,
    y: f32,
    z: f32,
    yaw: f32,
    now: Instant,
) {
    handle_transform_request_with_structures(
        player,
        map_layout,
        &HashMap::new(),
        x,
        y,
        z,
        yaw,
        now,
    );
}

pub(crate) fn handle_transform_request_with_structures(
    player: &mut ConnectedPlayer,
    map_layout: &MapLayoutState,
    structures: &HashMap<u64, Structure>,
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
    let current = Vec3f::new(player.hero.x, PLAYER_GROUND_Y, player.hero.z);
    let dx = requested.x - current.x;
    let dz = requested.z - current.z;
    let distance = (dx * dx + dz * dz).sqrt();
    let elapsed = now
        .duration_since(player.timers.last_movement_at)
        .as_secs_f32()
        .clamp(0.0, MOVEMENT_MAX_DELTA_SECONDS);
    let max_distance = hero_stats::movement_envelope(player, now, elapsed);

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

    // Validate the entire swept XZ segment, including long/debug-speed steps.
    // This is the same immutable forest collision map used for client routes.
    let accepted_xz = shared::navigation::world_navigation()
        .clip_movement([current.x, current.z], [accepted.x, accepted.z]);
    let accepted_xz = clip_live_structures([current.x, current.z], accepted_xz, structures);
    player.hero.x = accepted_xz[0];
    player.hero.y = PLAYER_GROUND_Y;
    player.hero.z = accepted_xz[1];
    if yaw.is_finite() {
        player.hero.yaw = yaw;
    }
    player.timers.last_movement_at = now;
}

/// Sweep every live gameplay footprint. Starting overlaps may recover only
/// outward, which also keeps reset/legacy-position recovery from freezing.
pub(crate) fn clip_live_structures(
    from: [f32; 2],
    to: [f32; 2],
    structures: &HashMap<u64, Structure>,
) -> [f32; 2] {
    let discs: Vec<_> = structures
        .values()
        .filter(|s| s.state.hp > 0.0)
        .map(|s| shared::navigation::Disc {
            center: [s.state.x, s.state.z],
            radius: structure_collision_radius(s.state.kind),
        })
        .collect();
    shared::navigation::clip_discs(from, to, &discs)
}

pub(crate) fn handle_respawns(world: &mut GameWorld, now: Instant) {
    if !matches!(world.game_state, GameState::Running) {
        return;
    }
    let GameWorld {
        players,
        structures,
        map_layout,
        ..
    } = world;
    for player in players.values_mut() {
        if !player.modifiers.respawns {
            continue;
        }
        let Some(respawn_at) = player.timers.respawn_at else {
            continue;
        };
        if now < respawn_at {
            continue;
        }
        let spawn =
            spawn_position_for_team_from_base(structures, map_layout, player.hero.identity.team);
        player.hero.x = spawn.x;
        player.hero.y = PLAYER_GROUND_Y;
        player.hero.z = spawn.z;
        player.hero.yaw = 0.0;
        player.hero.hp = player.hero.max_hp;
        player.hero.mana = player.hero.max_mana;
        player.timers.respawn_at = None;
        player.timers.haste_expires_at = None;
        player.timers.last_movement_at = now;
        player.timers.last_cast_at = [None; 4];
        player.timers.last_basic_attack_at = None;
        // Preserve request high-water through death; delayed strikes from the
        // same round must not become fresh attacks after respawn.
    }
}

impl GameWorld {
    /// Canonical clean-round state, before formation/start arms the clocks.
    pub(crate) fn reset_round(&mut self, now: Instant) {
        self.structures = build_configured_structures(&self.map_config);
        self.minions.clear();
        self.projectiles.clear();
        let mut next_neutral_id = 9_001;
        self.neutrals = build_neutral_camps(&mut next_neutral_id);
        self.neutrals
            .extend(build_boss_neutrals(&mut next_neutral_id));
        self.team_buffs.clear();
        self.last_wave_spawn_at = now;
        for player in self.players.values_mut() {
            reset_player_round(player, &self.map_layout, now);
            player.join_error = None;
        }
        self.game_state = GameState::Lobby;
    }
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
        match session.player.hero.identity.team {
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
            .world
            .players
            .iter()
            .filter(|(_, player)| {
                !player.hero.identity.is_bot
                    && now.saturating_duration_since(player.last_seen) > PLAYER_TIMEOUT
            })
            .map(|(addr, _)| *addr)
            .collect::<Vec<_>>();
        let draft_roster_changed = expired
            .iter()
            .any(|addr| self.world.players.get(addr).is_some_and(|p| p.joined));
        for addr in expired {
            self.disconnect_career_player(addr, now);
            let player = self.world.players.remove(&addr).unwrap();
            let disconnected_at = player.last_seen + PLAYER_TIMEOUT;
            if player.joined {
                println!(
                    "MATCH_METRIC event=disconnect epoch={} match={} player={} elapsed_ms={}",
                    self.server_epoch,
                    self.match_id,
                    player.hero.identity.id,
                    self.elapsed_match_ms(now)
                );
                if let Some(session_id) = player.session_id.clone() {
                    self.world.disconnected_sessions.insert(
                        session_id,
                        DisconnectedSession {
                            player,
                            disconnected_at,
                        },
                    );
                }
            }
        }
        if draft_roster_changed {
            self.invalidate_prematch_roster(now);
        }
        self.world.disconnected_sessions.retain(|_, session| {
            now.saturating_duration_since(session.disconnected_at) <= SESSION_RECLAIM_WINDOW
        });
        if self
            .world
            .players
            .values()
            .any(|p| p.joined && !p.hero.identity.is_bot)
        {
            self.empty_since = None;
        } else if !matches!(self.world.game_state, GameState::Lobby)
            || !self.world.disconnected_sessions.is_empty()
        {
            let empty_since = self.empty_since.get_or_insert(now);
            let grace = if self.match_service.worker().is_some() {
                SESSION_RECLAIM_WINDOW + Duration::from_secs(1)
            } else {
                EMPTY_ROSTER_GRACE
            };
            if now.saturating_duration_since(*empty_since) >= grace {
                println!(
                    "MATCH_METRIC event=abandoned epoch={} match={} elapsed_ms={}",
                    self.server_epoch,
                    self.match_id,
                    self.elapsed_match_ms(now)
                );
                self.finish_career_round(shared::career::MatchOutcome::Abandoned, None, now);
                self.restart_round(now);
            }
        }
    }

    pub(crate) fn restart_round(&mut self, now: Instant) {
        if self.sandbox_allowed() {
            let now = self.sandbox.as_ref().unwrap().now;
            self.reset_sandbox_duel(now);
            return;
        }
        self.finish_career_round(shared::career::MatchOutcome::Abandoned, None, now);
        if let crate::match_service::MatchService::Worker(worker) = &mut self.match_service {
            worker.aborted = true;
            return;
        }
        if self.rules.fills_with_bots {
            self.world
                .players
                .retain(|_, player| !player.hero.identity.is_bot);
            self.bots.clear();
        }
        self.reset_career_round();
        self.prematch = Default::default();
        for player in self.world.players.values_mut() {
            let capable = player.draft.capable;
            player.draft = prematch::DraftState {
                capable,
                ..Default::default()
            };
        }
        self.world.reset_round(now);
        // Only currently connected admitted identities participate in a rematch.
        self.world.disconnected_sessions.clear();
        self.match_id = self.match_id.saturating_add(1);
        self.world.forest_pickups = forest_pickups::ForestPickups::default();
        self.combat_log = CombatLog::default();
        self.match_started_at = None;
        self.victory_at = None;
        self.empty_since = None;
        self.metrics_players.clear();
        self.metrics_objectives.clear();
        if joined_count(&self.world.players) > 0 && !self.prematch_required() {
            advance_formation_on_join(&mut self.world, self.rules, now);
        }
        self.fill_practice_bots(now);
        self.tick_prematch(now);
        self.track_round_start(now);
        println!(
            "MATCH_METRIC event=round_reset epoch={} match={} connected={}",
            self.server_epoch,
            self.match_id,
            joined_count(&self.world.players)
        );
    }

    pub(crate) fn track_round_start(&mut self, now: Instant) {
        if matches!(self.world.game_state, GameState::Running) && self.match_started_at.is_none() {
            if !self.begin_career_round(now) {
                self.world.game_state = GameState::Starting { countdown_ms: 0 };
                return;
            }
            self.match_started_at = Some(now);
            self.world.last_wave_spawn_at = now - (MINION_WAVE_INTERVAL - FIRST_MINION_WAVE_DELAY);
            println!(
                "MATCH_METRIC event=round_start epoch={} match={} connected={} first_wave_secs={}",
                self.server_epoch,
                self.match_id,
                joined_count(&self.world.players),
                FIRST_MINION_WAVE_DELAY.as_secs()
            );
        }
    }

    pub(crate) fn elapsed_match_ms(&self, now: Instant) -> u128 {
        self.match_started_at
            .map_or(0, |start| now.saturating_duration_since(start).as_millis())
    }

    pub(crate) fn record_match_metrics(&mut self, now: Instant) {
        let elapsed = self.elapsed_match_ms(now);
        for player in self.world.players.values().filter(|player| player.joined) {
            let alive = player.hero.hp > 0.0;
            let previous = self
                .metrics_players
                .insert(player.hero.identity.id, (player.hero.progress.level, alive));
            if previous.is_none_or(|(level, _)| level != player.hero.progress.level) {
                println!(
                    "MATCH_METRIC event=progression epoch={} match={} player={} team={:?} level={} xp={} gold={} elapsed_ms={elapsed}",
                    self.server_epoch,
                    self.match_id,
                    player.hero.identity.id,
                    player.hero.identity.team,
                    player.hero.progress.level,
                    player.hero.progress.xp,
                    player.economy.gold
                );
            }
            if previous.is_some_and(|(_, was_alive)| was_alive) && !alive {
                println!(
                    "MATCH_METRIC event=death epoch={} match={} player={} elapsed_ms={elapsed}",
                    self.server_epoch, self.match_id, player.hero.identity.id
                );
            }
        }
        for structure in self
            .world
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
        if let GameState::Victory { winner } = self.world.game_state {
            self.finish_career_round(shared::career::MatchOutcome::Completed, Some(winner), now);
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
