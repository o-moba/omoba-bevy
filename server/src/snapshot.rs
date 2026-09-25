//! Snapshot assembly, UDP payload validation and per-recipient broadcast.

use std::time::{Duration, Instant};
use std::{fmt, io};

use shared::wire::{GameState, PlayerState, ServerPacket};

use crate::balance::VICTORY_REMATCH_DELAY;
use crate::game_world::GameWorld;
use crate::runtime::ports::Transport;
use crate::runtime::{RateLimitedDiagnostic, ServerRuntime};
use crate::sim::towers::structure_is_protected;
use crate::{prematch, vision};

use std::net::SocketAddr;

pub(crate) const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(50);

/// Largest application payload that can be carried by one IPv4 UDP datagram.
pub(crate) const IPV4_UDP_MAX_PAYLOAD_BYTES: usize = shared::transport::MAX_SNAPSHOT_BYTES;

#[derive(Debug)]
pub(crate) enum SnapshotDatagramError {
    Serialize(serde_json::Error),
    PayloadTooLarge { actual: usize, limit: usize },
}

impl fmt::Display for SnapshotDatagramError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Serialize(error) => write!(formatter, "failed to serialize snapshot: {error}"),
            Self::PayloadTooLarge { actual, limit } => write!(
                formatter,
                "serialized snapshot is {actual} bytes; IPv4 UDP payload limit is {limit} bytes"
            ),
        }
    }
}

pub(crate) fn serialize_snapshot_datagram(
    packet: &ServerPacket,
) -> Result<Vec<u8>, SnapshotDatagramError> {
    let mut payload = serde_json::to_vec(packet).map_err(SnapshotDatagramError::Serialize)?;
    if payload.len() > IPV4_UDP_MAX_PAYLOAD_BYTES {
        // Cosmetic history must never crowd otherwise-valid gameplay out of a
        // snapshot. Keep the newest receipts that fit, without dropping state.
        let mut trimmed = packet.clone();
        let ServerPacket::Snapshot { combat_events, .. } = &mut trimmed else {
            return Err(SnapshotDatagramError::PayloadTooLarge {
                actual: payload.len(),
                limit: IPV4_UDP_MAX_PAYLOAD_BYTES,
            });
        };
        let mut excess = payload.len() - IPV4_UDP_MAX_PAYLOAD_BYTES;
        let mut remove = 0;
        for event in combat_events.iter() {
            if excess == 0 {
                break;
            }
            let bytes = serde_json::to_vec(event)
                .map_err(SnapshotDatagramError::Serialize)?
                .len();
            let comma = usize::from(remove + 1 < combat_events.len());
            excess = excess.saturating_sub(bytes + comma);
            remove += 1;
        }
        combat_events.drain(..remove);
        payload = serde_json::to_vec(&trimmed).map_err(SnapshotDatagramError::Serialize)?;
    }
    validate_snapshot_payload_size(payload.len())?;
    Ok(payload)
}

pub(crate) fn validate_snapshot_payload_size(
    payload_len: usize,
) -> Result<(), SnapshotDatagramError> {
    if payload_len > IPV4_UDP_MAX_PAYLOAD_BYTES {
        return Err(SnapshotDatagramError::PayloadTooLarge {
            actual: payload_len,
            limit: IPV4_UDP_MAX_PAYLOAD_BYTES,
        });
    }
    Ok(())
}

/// Replicated player list for the recipient with hero id `recipient`: their
/// own entry is the `owner_view`, everyone else (teammates included) the
/// redacted `public_view`; `None` redacts every entry. Only joined players
/// are visible to clients. Pre-join endpoints are still addressable (public
/// lobby views, standalone status replies) but must not appear in the world
/// as ghost players.
pub(crate) fn build_players_snapshot(
    world: &GameWorld,
    recipient: Option<u64>,
    now: Instant,
) -> Vec<PlayerState> {
    let mut snapshot = world
        .players
        .values()
        .filter(|player| player.joined)
        .map(|player| {
            if recipient == Some(player.hero.identity.id) {
                player.owner_view(now, &world.map_layout, &world.game_state)
            } else {
                player.public_view(now, &world.map_layout, &world.game_state)
            }
        })
        .collect::<Vec<_>>();
    snapshot.sort_unstable_by_key(|player| player.id);
    snapshot
}

impl ServerRuntime {
    /// The replicated view of one player at `now`, as the broadcast builds it.
    #[cfg(test)]
    pub(crate) fn player_view(&self, addr: SocketAddr, now: Instant) -> PlayerState {
        self.world.players[&addr].owner_view(now, &self.world.map_layout, &self.world.game_state)
    }

    /// Builds one vision-filtered snapshot per human recipient and sends it,
    /// at most once per `SNAPSHOT_INTERVAL`.
    pub(crate) fn broadcast_snapshots(&mut self, now: Instant, career_flow: bool) {
        let snapshot_now = if self.sandbox.is_some() {
            self.clock.now()
        } else {
            now
        };
        if snapshot_now.saturating_duration_since(self.last_snapshot_at) < SNAPSHOT_INTERVAL {
            return;
        }
        self.snapshot_tick = self.snapshot_tick.saturating_add(1);
        let world = &self.world;
        for player in world.players.values().filter(|p| p.joined) {
            self.combat_log.ledger.update_player(
                player.hero.identity.id,
                player.hero.progress.level,
                false,
            );
            self.combat_log
                .ledger
                .update_earned_gold(player.hero.identity.id, player.economy.earned_gold);
        }
        let scoreboard = self.combat_log.ledger.live_scoreboard();

        let mut projectiles_snapshot = world
            .projectiles
            .values()
            .map(|projectile| projectile.state.clone())
            .collect::<Vec<_>>();
        projectiles_snapshot.sort_unstable_by_key(|projectile| projectile.id);

        let mut structures_snapshot = world
            .structures
            .values()
            .filter(|structure| structure.state.hp > 0.0)
            .map(|structure| {
                let mut state = structure.state.clone();
                state.protected = structure_is_protected(&world.structures, state.id);
                state
            })
            .collect::<Vec<_>>();
        structures_snapshot.sort_unstable_by_key(|structure| structure.id);

        let mut minions_snapshot = world
            .minions
            .values()
            .filter(|minion| minion.state.hp > 0.0)
            .map(|minion| minion.state.clone())
            .collect::<Vec<_>>();
        minions_snapshot.sort_unstable_by_key(|minion| minion.id);

        let mut neutrals_snapshot = world
            .neutrals
            .values()
            .filter(|neutral| neutral.dead_until.is_none() && neutral.state.hp > 0.0)
            .map(|neutral| neutral.state.clone())
            .collect::<Vec<_>>();
        neutrals_snapshot.sort_unstable_by_key(|neutral| neutral.id);

        let team_buffs_snapshot = world.team_buffs.snapshot(now);

        let rematch_in_secs = if !career_flow && let GameState::Victory { .. } = world.game_state {
            self.victory_at.map(|t| {
                VICTORY_REMATCH_DELAY
                    .saturating_sub(now.duration_since(t))
                    .as_secs()
            })
        } else {
            None
        };

        // Recipients are fixed before sending: the combat log is drained per
        // recipient below, which needs it mutably. A standalone server sends
        // the world only to verified endpoints (joined, or career-authenticated).
        let recipients =
            world
                .players
                .iter()
                .filter(|(addr, player)| {
                    !player.hero.identity.is_bot
                        && (!self.match_service.is_public() && self.endpoint_verified(**addr)
                            || self.match_service.is_public()
                                && self.public_transport.validated(**addr, now)
                                && self.career.backend.gameplay_principal(**addr).is_some_and(
                                    |p| {
                                        self.match_service.can_observe(
                                            &p.session_id,
                                            self.career
                                                .backend
                                                .profile(**addr)
                                                .as_ref()
                                                .map(|p| p.profile_id.as_str())
                                                .unwrap_or(""),
                                        )
                                    },
                                ))
                })
                .map(|(addr, _)| *addr)
                .collect::<Vec<_>>();

        // Step 11f: the same for every recipient of this match, but only a
        // joined player gets it (see `snapshot_debug_access`).
        let debug_access = self.debug_access();
        for addr in recipients {
            let player = &world.players[&addr];
            let mut packet = ServerPacket::Snapshot {
                vision: None,
                sandbox: self
                    .sandbox
                    .as_ref()
                    .map(|s| s.snapshot(addr, &world.players, &self.combat_log)),
                debug_access: crate::debug::snapshot_debug_access(player, debug_access),
                match_mode: self.rules.mode_id().into(),
                geometry_id: world.map_config.geometry_id.clone(),
                map_profile: world.map_config.map_profile.clone(),
                meta: shared::protocol::SnapshotMeta::new(
                    self.server_epoch,
                    self.match_id,
                    self.snapshot_tick,
                ),
                join_error: player.join_error,
                your_id: player.hero.identity.id,
                // The recipient's own entry is the owner view; every other
                // player is redacted, in the sandbox too.
                players: build_players_snapshot(world, Some(player.hero.identity.id), now),
                scoreboard: scoreboard.clone(),
                prematch: prematch::snapshot(
                    &self.prematch,
                    &world.players,
                    player,
                    self.rules,
                    now,
                ),
                projectiles: projectiles_snapshot.clone(),
                combat_events: self.combat_log.snapshot(now),
                structures: structures_snapshot.clone(),
                minions: minions_snapshot.clone(),
                neutrals: neutrals_snapshot.clone(),
                team_buffs: team_buffs_snapshot.clone(),
                forest_pickups: world.forest_pickups.snapshot(&world.game_state),
                game_state: world.game_state.clone(),
                rematch_in_secs,
            };

            if self.sandbox.is_none() {
                vision::filter_snapshot(&mut packet, player, world, now);
            }

            send_snapshot(
                self.transport.as_ref(),
                &mut self.snapshot_send_diagnostic,
                &packet,
                player.framed_snapshots,
                self.server_epoch,
                self.snapshot_tick,
                addr,
                now,
            );
        }

        // Unverified standalone endpoints get a small status snapshot, one
        // per datagram they sent and at most once per interval (report O4).
        if !self.match_service.is_public() {
            let mut replies = std::mem::take(&mut self.prejoin_replies);
            let due = crate::runtime::prejoin::due_status_replies(
                &mut replies,
                |addr| world.players.contains_key(&addr) && !self.endpoint_verified(addr),
                now,
            );
            self.prejoin_replies = replies;
            let meta = shared::protocol::SnapshotMeta::new(
                self.server_epoch,
                self.match_id,
                self.snapshot_tick,
            );
            for addr in due {
                let Some(packet) = crate::runtime::prejoin::status_snapshot(
                    meta,
                    self.rules.mode_id(),
                    world,
                    addr,
                ) else {
                    continue;
                };
                send_snapshot(
                    self.transport.as_ref(),
                    &mut self.snapshot_send_diagnostic,
                    &packet,
                    world.players[&addr].framed_snapshots,
                    self.server_epoch,
                    self.snapshot_tick,
                    addr,
                    now,
                );
            }
        }

        self.last_snapshot_at = snapshot_now;
    }
}

/// Serializes `packet` (framed for a recipient that said Hello) and sends it
/// to `addr`, reporting failures through the rate-limited `diagnostic`.
fn send_snapshot(
    transport: &dyn Transport,
    diagnostic: &mut RateLimitedDiagnostic,
    packet: &ServerPacket,
    framed: bool,
    server_epoch: u64,
    snapshot_tick: u64,
    addr: SocketAddr,
    now: Instant,
) {
    let payloads = if framed {
        serialize_snapshot_datagram(packet)
            .map_err(|error| error.to_string())
            .and_then(|payload| {
                shared::transport::encode_snapshot(&payload, server_epoch, snapshot_tick)
                    .map_err(|error| error.to_string())
            })
    } else {
        serialize_snapshot_datagram(packet)
            .map(|payload| vec![payload])
            .map_err(|error| error.to_string())
    };
    match payloads {
        Ok(payloads) => {
            for payload in payloads {
                let result = transport.send_to(&payload, addr).and_then(|sent| {
                    if sent == payload.len() {
                        Ok(())
                    } else {
                        Err(io::Error::new(
                            io::ErrorKind::WriteZero,
                            "incomplete UDP datagram",
                        ))
                    }
                });
                if let Err(error) = result {
                    if let Some(suppressed) = diagnostic.record(now) {
                        eprintln!(
                            "Failed to send complete {}-byte snapshot datagram to {addr}: {error}; suppressed {suppressed} similar errors",
                            payload.len()
                        );
                    }
                    break;
                }
            }
        }
        Err(error) => {
            if let Some(suppressed) = diagnostic.record(now) {
                eprintln!(
                    "Rejected snapshot for {addr}: {error}; suppressed {suppressed} similar errors"
                );
            }
        }
    }
}
