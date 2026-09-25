//! Standalone pre-join endpoint safety (report O4).
//!
//! A standalone server (`MatchService::Standalone`, the LAN and beta-host
//! shape) has no transport admission: any UDP source address becomes a
//! pre-join endpoint on its first datagram. Two bounds apply there:
//!
//! - at most [`MAX_PREJOIN_ENDPOINTS`] unverified endpoints are kept; a new
//!   address beyond that is dropped until one of them joins or times out;
//! - an unverified endpoint never receives the full world snapshot. It gets
//!   one small status snapshot (no players, structures, minions, events or
//!   vision; only meta, map identity, `your_id`, `join_error` and the
//!   `game_state` phase) per datagram it sent, at most once per
//!   [`PREJOIN_STATUS_INTERVAL`].
//!
//! "Verified" means joined, or holding an authenticated career session.
//! Public roles run their own transport admission and are unchanged.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use shared::wire::ServerPacket;

use crate::game_world::GameWorld;
use crate::runtime::ServerRuntime;

/// Most unverified endpoints a standalone server keeps at once.
pub(crate) const MAX_PREJOIN_ENDPOINTS: usize = 64;

/// Least time between two status replies to one unverified endpoint.
pub(crate) const PREJOIN_STATUS_INTERVAL: Duration = Duration::from_millis(250);

/// Reply bookkeeping for one unverified endpoint.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct PrejoinReply {
    /// A datagram arrived since the last status reply.
    pub(crate) due: bool,
    pub(crate) last_sent: Option<Instant>,
}

impl ServerRuntime {
    /// Whether `addr` is a verified endpoint: joined, or authenticated to the
    /// career store. Unknown addresses are unverified.
    pub(crate) fn endpoint_verified(&self, addr: SocketAddr) -> bool {
        self.world
            .players
            .get(&addr)
            .is_some_and(|player| player.joined || player.hero.identity.is_bot)
            || self.career.backend.authenticated_session(addr).is_some()
    }

    fn unverified_endpoint_count(&self) -> usize {
        self.world
            .players
            .keys()
            .filter(|addr| !self.endpoint_verified(**addr))
            .count()
    }

    /// Standalone admission for one gameplay datagram from `addr`. Returns
    /// false when the datagram must be dropped: a new address while the
    /// pre-join table is full. An accepted datagram from an unverified
    /// endpoint earns it one status reply.
    pub(crate) fn admit_standalone_datagram(&mut self, addr: SocketAddr, now: Instant) -> bool {
        if self.match_service.is_public() {
            return true;
        }
        if !self.world.players.contains_key(&addr)
            && !self.endpoint_verified(addr)
            && self.unverified_endpoint_count() >= MAX_PREJOIN_ENDPOINTS
        {
            if let Some(suppressed) = self.prejoin_limit_diagnostic.record(now) {
                eprintln!(
                    "Dropped datagram from {addr}: {MAX_PREJOIN_ENDPOINTS} pre-join endpoints are already connected; suppressed {suppressed} similar drops"
                );
            }
            return false;
        }
        if !self.endpoint_verified(addr) {
            self.prejoin_replies.entry(addr).or_default().due = true;
        }
        true
    }
}

/// The status snapshot an unverified endpoint receives instead of the world:
/// enough for the client to see the server's protocol, epoch, map identity,
/// its reserved id, a join rejection and the match phase.
pub(crate) fn status_snapshot(
    runtime_meta: shared::protocol::SnapshotMeta,
    match_mode: &str,
    world: &GameWorld,
    addr: SocketAddr,
) -> Option<ServerPacket> {
    let player = world.players.get(&addr)?;
    Some(ServerPacket::Snapshot {
        vision: None,
        sandbox: None,
        match_mode: match_mode.into(),
        geometry_id: world.map_config.geometry_id.clone(),
        map_profile: world.map_config.map_profile.clone(),
        meta: runtime_meta,
        join_error: player.join_error,
        your_id: player.hero.identity.id,
        players: Vec::new(),
        scoreboard: None,
        prematch: None,
        projectiles: Vec::new(),
        combat_events: Vec::new(),
        structures: Vec::new(),
        minions: Vec::new(),
        neutrals: Vec::new(),
        team_buffs: Vec::new(),
        forest_pickups: Vec::new(),
        game_state: world.game_state.clone(),
        rematch_in_secs: None,
    })
}

/// Takes the endpoints owed a status reply at `now` and forgets endpoints
/// that left the world or became verified.
pub(crate) fn due_status_replies(
    replies: &mut HashMap<SocketAddr, PrejoinReply>,
    still_unverified: impl Fn(SocketAddr) -> bool,
    now: Instant,
) -> Vec<SocketAddr> {
    replies.retain(|addr, _| still_unverified(*addr));
    let mut due = Vec::new();
    for (addr, reply) in replies.iter_mut() {
        if reply.due
            && reply
                .last_sent
                .is_none_or(|at| now.saturating_duration_since(at) >= PREJOIN_STATUS_INTERVAL)
        {
            reply.due = false;
            reply.last_sent = Some(now);
            due.push(*addr);
        }
    }
    due.sort_unstable();
    due
}

#[cfg(test)]
mod tests;
