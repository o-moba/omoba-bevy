//! Human participation is keyed by round/seat, not a UDP address or controller.
use std::{collections::BTreeMap, net::SocketAddr, time::Instant};

use crate::runtime::ServerRuntime;
use shared::{
    career::ParticipantResult,
    wire::{ClientPacket, GameState},
};
mod policy;
#[cfg(test)]
mod tests;

struct Human {
    input_kind: shared::ranked::InputKind,
    time: policy::Seat,
}

#[derive(Default)]
pub(crate) struct FairPlay {
    round: Option<(u64, u64)>,
    humans: BTreeMap<u64, Human>,
}

impl FairPlay {
    pub(crate) fn currently_absent(&self, id: u64, now: Instant) -> bool {
        self.humans
            .get(&id)
            .is_some_and(|h| h.time.currently_absent(now))
    }
    pub(crate) fn annotate(&self, participants: &mut [ParticipantResult]) {
        for participant in participants {
            if let Some(human) = self.humans.get(&participant.player_id) {
                participant.input_kind = human.input_kind;
            }
            participant.participation = self.humans.get(&participant.player_id).map(|human| {
                let p = human.time.snapshot();
                shared::career::Participation {
                    connected_ms: p.connected_ms,
                    disconnected_ms: p.disconnected_ms,
                    afk_ms: p.afk_ms,
                    abandoned: p.abandoned,
                }
            });
        }
    }
}

impl ServerRuntime {
    pub(crate) fn observe_fair_play(&mut self, now: Instant, finalize: bool) {
        let Some(started) = self.match_started_at else {
            self.fair_play = Default::default();
            return;
        };
        if self.match_service.worker().is_none() {
            return;
        }
        let key = (self.server_epoch, self.match_id);
        if self.fair_play.round != Some(key) {
            self.fair_play = FairPlay {
                round: Some(key),
                ..Default::default()
            };
        }
        let mut seen = std::collections::BTreeSet::new();
        let observations: Vec<_> = self
            .world
            .players
            .iter()
            .filter(|(_, p)| p.joined && !p.hero.identity.is_bot)
            .map(|(addr, p)| (Some(*addr), p))
            .chain(
                self.world
                    .disconnected_sessions
                    .values()
                    .map(|s| (None, &s.player)),
            )
            .filter(|(_, p)| !p.hero.identity.is_bot)
            .map(|(addr, p)| (p.hero.identity.id, addr, p.hero.hp > 0.0))
            .collect();
        let mut afk = Vec::new();
        for (id, addr, alive) in observations {
            if !seen.insert(id) {
                continue;
            }
            if !self.fair_play.humans.contains_key(&id)
                && self.fair_play.humans.len() >= shared::career::MAX_PARTICIPANTS
            {
                continue;
            }
            let connected = addr.is_some_and(|a| !crate::bots::is_bot_address(a));
            let human = self.fair_play.humans.entry(id).or_insert_with(|| Human {
                input_kind: Default::default(),
                time: policy::Seat::new(started, connected, alive),
            });
            if human.time.observe(now, connected, alive)
                && !finalize
                && let Some(addr) = addr.filter(|_| connected)
            {
                afk.push(addr);
            }
        }
        // A retired/missing actor still belongs to its original participation row.
        for (id, human) in &mut self.fair_play.humans {
            if !seen.contains(id) {
                human.time.observe(now, false, false);
            }
            if finalize {
                human.time.finish(now);
            }
        }
        if matches!(self.world.game_state, GameState::Running) {
            for addr in afk {
                self.attach_afk_controller(addr, now);
            }
        }
    }

    /// Called only after public transport and career authorization, never by AI.
    /// Returns false for idle transforms while AI owns the same connected seat.
    pub(crate) fn human_network_activity(
        &mut self,
        addr: SocketAddr,
        packet: &ClientPacket,
        now: Instant,
    ) -> bool {
        if self.match_service.worker().is_none() || self.match_started_at.is_none() {
            return true;
        }
        let Some(player) = self
            .world
            .players
            .get(&addr)
            .filter(|p| p.joined && !p.hero.identity.is_bot)
        else {
            return true;
        };
        if self.career.backend.authenticated_session(addr).as_deref()
            != player.session_id.as_deref()
            || player.session_id.is_none()
        {
            return true;
        }
        let id = player.hero.identity.id;
        let afk = self.session_recovery.is_afk(addr);
        self.touch_afk_peer(addr, now);
        if matches!(packet,
            ClientPacket::BasicAttack { server_epoch, match_id, .. }
            | ClientPacket::CastSkill { server_epoch, match_id, .. }
            | ClientPacket::Utility { server_epoch, match_id, .. }
            | ClientPacket::BuyItem { server_epoch, match_id, .. }
                if *server_epoch != self.server_epoch || *match_id != self.match_id)
        {
            return true;
        }
        let Some(human) = self.fair_play.humans.get_mut(&id) else {
            return true;
        };
        if let ClientPacket::Transform {
            input_kind,
            x,
            y,
            z,
            yaw,
            ..
        } = packet
            && x.is_finite()
            && y.is_finite()
            && z.is_finite()
            && yaw.is_finite()
        {
            human.input_kind = human.input_kind.observe(*input_kind);
        }
        let meaningful = match packet {
            ClientPacket::Transform {
                input_active,
                x,
                y,
                z,
                ..
            } if x.is_finite() && y.is_finite() && z.is_finite() => *input_active,
            ClientPacket::Cast { .. }
            | ClientPacket::CastSkill { .. }
            | ClientPacket::BasicAttack { .. }
            | ClientPacket::Utility { .. }
            | ClientPacket::Interact { .. }
            | ClientPacket::UpgradeSkill { .. }
            | ClientPacket::BuyItem { .. } => true,
            ClientPacket::Join { .. } if afk => true,
            _ => false,
        };
        if meaningful {
            human.time.input(now);
            self.reclaim_afk_controller(addr, now);
        }
        !afk || meaningful || !matches!(packet, ClientPacket::Transform { .. })
    }
}
