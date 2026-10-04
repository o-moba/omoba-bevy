//! Live allocated seats survive transport loss; only their controller changes.
use std::collections::HashMap;
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
use std::time::Instant;

use shared::match_service::{TakeoverPolicy, TakeoverSeatView};
use shared::wire::GameState;

use crate::runtime::{PLAYER_TIMEOUT, ServerRuntime};

#[derive(Default)]
pub(crate) struct SessionRecovery {
    seats: HashMap<u64, DetachedSeat>,
    generation: u64,
}
impl SessionRecovery {
    /// A retained actor is controlled by the server, not an authenticated UDP
    /// endpoint. Only the exact registered identity can keep that exemption.
    pub(crate) fn retains(
        &self,
        addr: SocketAddr,
        player: &crate::entities::ConnectedPlayer,
    ) -> bool {
        crate::bots::is_bot_address(addr)
            && self
                .seats
                .get(&player.hero.identity.id)
                .is_some_and(|seat| {
                    seat.address == addr
                        && player.session_id.as_deref() == Some(seat.session.as_str())
                })
    }
}
struct DetachedSeat {
    address: SocketAddr,
    session: String,
    generation: u64,
    votes: HashMap<u64, TakeoverPolicy>,
    policy: TakeoverPolicy,
}
impl ServerRuntime {
    /// Returns false for pre-match/terminal/direct-server seats, whose existing
    /// finite admission and abandonment rules still apply.
    pub(crate) fn detach_running_seat(&mut self, addr: SocketAddr, now: Instant) -> bool {
        if self.match_service.worker().is_none()
            || !matches!(self.world.game_state, GameState::Running)
            || !self
                .world
                .players
                .get(&addr)
                .is_some_and(|p| p.joined && !p.hero.identity.is_bot && p.session_id.is_some())
        {
            return false;
        }
        let Some(port) = (1024..=u16::MAX)
            .find(|port| !self.world.players.contains_key(&internal_address(*port)))
        else {
            return false;
        };
        let address = internal_address(port);
        self.disconnect_career_player(addr, now);
        let mut player = self.world.players.remove(&addr).unwrap();
        common::recall::cancel(&mut player);
        // A deliberate Leave may be followed immediately by a Resume. The
        // internal actor is never an active competing network endpoint.
        player.last_seen = now - PLAYER_TIMEOUT - std::time::Duration::from_millis(1);
        let id = player.hero.identity.id;
        let session = player.session_id.clone().unwrap();
        self.bots
            .attach_existing(address, player.hero.identity.hero_class, now);
        self.world.players.insert(address, player);
        self.session_recovery.generation = self.session_recovery.generation.saturating_add(1);
        self.session_recovery.seats.insert(
            id,
            DetachedSeat {
                address,
                session,
                generation: self.session_recovery.generation,
                votes: HashMap::new(),
                policy: TakeoverPolicy::Bot,
            },
        );
        self.empty_since = None;
        true
    }

    /// Shared combat handlers update last_seen for AI actions too. Once the
    /// allocation/profile/session gates authorize Join, an internal controller
    /// must never masquerade as a competing live network endpoint.
    pub(crate) fn prepare_detached_reclaim(&mut self, session: Option<&str>, now: Instant) {
        let Some(seat) = self
            .session_recovery
            .seats
            .values()
            .find(|seat| Some(seat.session.as_str()) == session)
        else {
            return;
        };
        if let Some(player) = self.world.players.get_mut(&seat.address) {
            player.last_seen = now - PLAYER_TIMEOUT - std::time::Duration::from_millis(1);
        }
    }

    pub(crate) fn restore_manual_control(&mut self, addr: SocketAddr) {
        let Some(player) = self.world.players.get(&addr) else {
            return;
        };
        let id = player.hero.identity.id;
        if self
            .session_recovery
            .seats
            .get(&id)
            .is_some_and(|seat| player.session_id.as_deref() == Some(&seat.session))
            && let Some(seat) = self.session_recovery.seats.remove(&id)
        {
            self.bots.detach(seat.address);
        }
    }

    fn eligible_takeover_voters(&self, team: shared::map::Team) -> Vec<(SocketAddr, u64)> {
        self.world
            .players
            .iter()
            .filter(|(addr, player)| {
                !crate::bots::is_bot_address(**addr)
                    && player.joined
                    && !player.hero.identity.is_bot
                    && player.hero.identity.team == team
                    && self.career.backend.authenticated_session(**addr).as_deref()
                        == player.session_id.as_deref()
                    && player.session_id.is_some()
                    && self.career.backend.profile(**addr).is_some_and(|profile| {
                        player
                            .career_profile
                            .as_ref()
                            .is_some_and(|owned| owned.profile_id == profile.profile_id)
                    })
            })
            .map(|(addr, player)| (*addr, player.hero.identity.id))
            .collect()
    }

    pub(crate) fn handle_takeover_vote(
        &mut self,
        addr: SocketAddr,
        epoch: u64,
        match_id: u64,
        player_id: u64,
        generation: u64,
        policy: TakeoverPolicy,
        now: Instant,
    ) {
        if epoch != self.server_epoch
            || match_id != self.match_id
            || !matches!(self.world.game_state, GameState::Running)
        {
            return;
        }
        let Some(seat) = self
            .session_recovery
            .seats
            .get(&player_id)
            .filter(|seat| seat.generation == generation)
        else {
            return;
        };
        let Some(target) = self.world.players.get(&seat.address) else {
            return;
        };
        let Some((_, voter)) = self
            .eligible_takeover_voters(target.hero.identity.team)
            .into_iter()
            .find(|(candidate, _)| *candidate == addr)
        else {
            return;
        };
        self.session_recovery
            .seats
            .get_mut(&player_id)
            .unwrap()
            .votes
            .insert(voter, policy);
        self.refresh_takeover_controllers(now);
    }

    pub(crate) fn refresh_takeover_controllers(&mut self, now: Instant) {
        let ids: Vec<_> = self.session_recovery.seats.keys().copied().collect();
        for id in ids {
            let seat = &self.session_recovery.seats[&id];
            let Some(player) = self.world.players.get(&seat.address) else {
                continue;
            };
            let eligible = self.eligible_takeover_voters(player.hero.identity.team);
            let idle = eligible
                .iter()
                .filter(|(_, voter)| seat.votes.get(voter) == Some(&TakeoverPolicy::Idle))
                .count();
            // A strict majority of currently connected human teammates is
            // required to idle the hero. Ties/no votes default to bot support.
            let policy = if idle > eligible.len() / 2 {
                TakeoverPolicy::Idle
            } else {
                TakeoverPolicy::Bot
            };
            let seat = self.session_recovery.seats.get_mut(&id).unwrap();
            seat.votes
                .retain(|voter, _| eligible.iter().any(|(_, id)| id == voter));
            seat.policy = policy;
            match policy {
                TakeoverPolicy::Idle => self.bots.detach(seat.address),
                TakeoverPolicy::Bot => {
                    self.bots
                        .attach_existing(seat.address, player.hero.identity.hero_class, now)
                }
            }
        }
    }

    pub(crate) fn takeover_view(&self, addr: SocketAddr) -> Vec<TakeoverSeatView> {
        let Some(viewer) = self.world.players.get(&addr).filter(|p| p.joined) else {
            return Vec::new();
        };
        let eligible = self.eligible_takeover_voters(viewer.hero.identity.team);
        if !eligible.iter().any(|(candidate, _)| *candidate == addr) {
            return Vec::new();
        }
        let mut views: Vec<_> = self
            .session_recovery
            .seats
            .iter()
            .filter_map(|(id, seat)| {
                let player = self.world.players.get(&seat.address)?;
                if player.hero.identity.team != viewer.hero.identity.team {
                    return None;
                }
                Some(TakeoverSeatView {
                    player_id: *id,
                    generation: seat.generation,
                    nickname: player
                        .career_profile
                        .as_ref()
                        .map_or_else(|| format!("Player {id}"), |p| p.nickname.clone()),
                    policy: seat.policy,
                    idle_votes: eligible
                        .iter()
                        .filter(|(_, id)| seat.votes.get(id) == Some(&TakeoverPolicy::Idle))
                        .count() as u32,
                    bot_votes: eligible
                        .iter()
                        .filter(|(_, id)| seat.votes.get(id) == Some(&TakeoverPolicy::Bot))
                        .count() as u32,
                    eligible_voters: eligible.len() as u32,
                    my_vote: seat.votes.get(&viewer.hero.identity.id).copied(),
                })
            })
            .collect();
        views.sort_by_key(|view| view.player_id);
        views
    }
}
fn internal_address(port: u16) -> SocketAddr {
    SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, port, 0, 0))
}
