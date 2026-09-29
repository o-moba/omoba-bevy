use crate::{formation::joined_count, runtime::ServerRuntime, session::normalize_session_id};
pub(crate) use common::bots::*;
use shared::{
    HeroClass,
    map::Team,
    wire::{ClientPacket, GameState},
};
use std::{net::SocketAddr, time::Instant};
impl ServerRuntime {
    pub(crate) fn prepare_practice_join(
        &mut self,
        addr: SocketAddr,
        packet: &ClientPacket,
        now: Instant,
    ) -> bool {
        if !self.rules.fills_with_bots || self.match_service.worker().is_some() {
            return true;
        }
        let ClientPacket::Join { session_id, .. } = packet else {
            return true;
        };
        if self.world.players.get(&addr).is_some_and(|p| p.joined) {
            return true;
        }
        let session = normalize_session_id(session_id.clone());
        if !self
            .world
            .players
            .get(&addr)
            .is_some_and(|p| p.protocol_compatible)
        {
            return false;
        }
        let retained = session
            .as_ref()
            .and_then(|session| self.world.disconnected_sessions.get(session));
        let available = retained.map_or_else(
            || assign_human_team(&self.world.players, self.rules.team_size).is_some(),
            |p| {
                human_count(&self.world.players, p.player.hero.identity.team) < self.rules.team_size
            },
        );
        if !available {
            self.world.players.get_mut(&addr).unwrap().join_error =
                Some(shared::protocol::JoinRejection::MatchFull);
            return false;
        }
        let retained_id = retained.map(|p| p.player.hero.identity.id);
        let roster = self.combat_log.ledger.snapshot();
        let existing = roster.iter().any(|p| {
            Some(p.player_id) == retained_id
                || self
                    .world
                    .players
                    .get(&addr)
                    .is_some_and(|a| a.hero.identity.id == p.player_id)
        });
        if matches!(self.world.game_state, GameState::Victory { .. })
            || (!existing && roster.len() >= shared::career::MAX_PARTICIPANTS)
        {
            // Admit the new human before refilling. A 16v16 arena otherwise
            // freezes 32 fresh identities and immediately overflows on this join.
            self.bots.defer_fill = true;
            self.restart_round(now);
            self.bots.defer_fill = false;
        }
        true
    }

    pub(crate) fn fill_practice_bots(&mut self, now: Instant) {
        // Allocated bots are created exactly once after every frozen human arrives.
        if self.match_service.worker().is_some()
            && (!self.allocated_humans_ready() || self.match_started_at.is_some())
        {
            return;
        }
        if !self.rules.fills_with_bots
            || self.bots.defer_fill
            || self.bots.sandbox
            || matches!(self.world.game_state, GameState::Victory { .. })
            || !self
                .world
                .players
                .values()
                .any(|p| p.joined && !p.hero.identity.is_bot)
        {
            return;
        }
        // Reclaimed humans keep their own identity and gameplay state. Remove
        // their temporary replacement before another simulation or snapshot.
        for team in [Team::Green, Team::Blue] {
            while seated_count(&self.world.players, &self.bots, team)
                > self.rules.team_size as usize
            {
                let before = self.world.players.len();
                remove_replaced_bot(
                    &mut self.world.players,
                    &mut self.bots,
                    &mut self.combat_log.ledger,
                    team,
                );
                if self.world.players.len() == before {
                    break;
                }
            }
        }
        let missing = self
            .rules
            .roster_size()
            .saturating_sub(joined_count(&self.world.players)) as usize;
        if self.combat_log.ledger.is_started()
            && self.combat_log.ledger.snapshot().len() + missing > shared::career::MAX_PARTICIPANTS
        {
            // Immutable historical identities never get recycled to make room.
            // A fresh practice round retains connected humans and clears combat.
            self.restart_round(now);
            return;
        }
        for team in [Team::Green, Team::Blue] {
            while seated_count(&self.world.players, &self.bots, team)
                < self.rules.team_size as usize
            {
                if self.spawn_bot(team, None, BotKind::Lane, now).is_none() {
                    break;
                }
            }
        }
    }

    pub(crate) fn combat_host(&mut self) -> common::host::CombatHost<'_> {
        common::host::CombatHost {
            world: &mut self.world,
            bots: &mut self.bots,
            combat_log: &mut self.combat_log,
            rules: self.rules,
            match_id: self.match_id,
        }
    }
    pub(crate) fn spawn_bot(
        &mut self,
        team: Team,
        class: Option<HeroClass>,
        kind: BotKind,
        now: Instant,
    ) -> Option<SocketAddr> {
        let addr = self.combat_host().spawn_bot(team, class, kind, now)?;
        self.register_career_participant(addr);
        Some(addr)
    }
    pub(crate) fn simulate_bots(&mut self, now: Instant, dt: f32) {
        self.combat_host().simulate_bots(now, dt);
    }
}
