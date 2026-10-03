use crate::{
    balance::{
        EMPTY_ROSTER_GRACE, FIRST_MINION_WAVE_DELAY, MINION_WAVE_INTERVAL, SESSION_RECLAIM_WINDOW,
    },
    combat_feedback::CombatLog,
    entities::{ConnectedPlayer, DisconnectedSession, MapLayoutState},
    forest_pickups,
    formation::{advance_formation_on_join, joined_count},
    prematch,
    runtime::{PLAYER_TIMEOUT, ServerRuntime},
};
pub(crate) use common::session::*;
use shared::{
    HeroClass,
    map::Team,
    wire::{CharacterChoice, GameState},
};
use std::time::{Duration, Instant};

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
            let mut player = self.world.players.remove(&addr).unwrap();
            common::recall::cancel(&mut player);
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
        let (outcome, winner) = self.teardown_outcome();
        self.finish_career_round(outcome, winner, now);
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
    }

    /// Round lifecycle at the end of a tick: a won round is finalized as
    /// Completed for the career store and starts the rematch timer
    /// (`victory_at`). Idempotent; logging lives in `record_match_metrics`.
    pub(crate) fn settle_finished_round(&mut self, now: Instant) {
        if let GameState::Victory { winner } = self.world.game_state {
            self.finish_career_round(shared::career::MatchOutcome::Completed, Some(winner), now);
            if self.victory_at.is_none() {
                self.victory_at = Some(now);
                println!(
                    "MATCH_METRIC event=victory epoch={} match={} winner={winner:?} duration_ms={}",
                    self.server_epoch,
                    self.match_id,
                    self.elapsed_match_ms(now)
                );
            }
        }
    }

    /// How an unfinalized career round ends when the round is torn down: a
    /// won round is Completed, anything else is Abandoned.
    fn teardown_outcome(&self) -> (shared::career::MatchOutcome, Option<shared::map::Team>) {
        match self.world.game_state {
            GameState::Victory { winner } => {
                (shared::career::MatchOutcome::Completed, Some(winner))
            }
            _ => (shared::career::MatchOutcome::Abandoned, None),
        }
    }
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
    common::session::handle_join_request_with_sprite(
        player,
        team,
        character,
        hero_class,
        omoba_passport::avatars::normalize_avatar_slug(avatar),
        sprite_character,
        map_layout,
        now,
    );
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
