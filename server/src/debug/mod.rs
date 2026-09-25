//! The debug command family on the server (`shared::debug`): one permission
//! predicate, [`ServerRuntime::debug_access`], and one entry point,
//! [`ServerRuntime::handle_debug`], for the development toggles
//! (`toggles.rs`) and the practice sandbox (`practice.rs`).
//!
//! The dispatcher (`runtime/dispatch.rs`) converts the packet with
//! `DebugCommand::from_packet` at the arms the packets always had: `Practice`
//! before the paused-sandbox gate on the wall clock, the toggles after it on
//! the (sandbox) simulation clock. The Combat Test protocol is a separate
//! family (`crate::sandbox`, dispatched through `runtime/handlers/tools.rs`).
//!
//! The predicate also goes out on the wire (step 11f): every world snapshot
//! to a joined player carries `debug_access` ([`snapshot_debug_access`]), so
//! the client's tools page follows the server instead of guessing from the
//! `match_mode` string (which reads `"practice"` in a worker round too).

mod practice;
mod toggles;

use std::net::SocketAddr;
use std::ops::ControlFlow;
use std::time::Instant;

use shared::debug::{DebugAccess, DebugCommand};

use crate::entities::ConnectedPlayer;
use crate::runtime::ServerRuntime;

/// `Snapshot.debug_access` for one recipient of a world snapshot: the match's
/// access for a joined player, `None` for anyone else (an endpoint that is
/// only career-authenticated, or whose join was rejected). The prejoin status
/// reply and the lobby snapshot never carry it.
///
/// A public worker round sends `Some` with both flags false: it reveals
/// nothing (no debug command works there) and tells a new client not to show
/// the tools page, which the `"practice"` mode string alone would.
pub(crate) fn snapshot_debug_access(
    player: &ConnectedPlayer,
    access: DebugAccess,
) -> Option<DebugAccess> {
    player.joined.then_some(access)
}

impl ServerRuntime {
    /// Which debug commands this match accepts: the toggles in Dev and
    /// Practice, the practice sandbox in Practice, neither in a
    /// worker-allocated round. A worker round decides its durable ruleset
    /// once at round start (`begin_career_round`), so a toggle flipped
    /// afterwards would ride into a saved result, and its bots come from the
    /// manifest. Without a worker this equals
    /// `DebugAccess::for_match_mode(rules.mode_id())` (pinned by a test).
    pub(crate) fn debug_access(&self) -> DebugAccess {
        let local = self.match_service.worker().is_none();
        DebugAccess {
            toggles: self.rules.debug_commands && local,
            practice: self.rules.fills_with_bots && local,
        }
    }

    /// Run one debug command from `addr`. The flow per variant is what the
    /// packet handlers returned before the family existed:
    /// - a toggle: `Break` when refused (before the endpoint is touched) or
    ///   when the sender has not joined; `Continue` once applied, so the
    ///   post-command tail runs;
    /// - a practice command: always `Break`, and the endpoint is touched even
    ///   when the command is refused.
    pub(crate) fn handle_debug(
        &mut self,
        addr: SocketAddr,
        command: DebugCommand,
        now: Instant,
    ) -> ControlFlow<()> {
        let allowed = self.debug_access().allows(command);
        match command {
            DebugCommand::GodMode(enabled) if allowed => self.set_god_mode(addr, enabled, now),
            DebugCommand::SpeedBoost(enabled) if allowed => {
                self.set_speed_boost(addr, enabled, now)
            }
            DebugCommand::GodMode(_) | DebugCommand::SpeedBoost(_) => ControlFlow::Break(()),
            DebugCommand::Practice(command) => {
                if let Some(player) = self.world.players.get_mut(&addr) {
                    player.last_seen = now;
                }
                if allowed {
                    self.handle_practice_command(addr, command, now);
                }
                ControlFlow::Break(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{SocketAddr, UdpSocket};
    use std::time::Instant;

    use shared::HeroClass;
    use shared::debug::DebugAccess;
    use shared::map::Team;
    use shared::wire::{CharacterChoice, ClientPacket, ServerPacket};

    use crate::career_backend;
    use crate::match_rules::{MatchConfig, MatchMode};
    use crate::runtime::ServerRuntime;
    use crate::runtime::ports::{ManualClock, MemoryTransport};
    use crate::snapshot::SNAPSHOT_INTERVAL;

    /// Every mode; the `match` fails to compile when a mode is added, so the
    /// parity test below cannot silently skip it.
    fn every_mode() -> [MatchMode; 3] {
        let modes = [MatchMode::Release, MatchMode::Dev, MatchMode::Practice];
        for mode in modes {
            match mode {
                MatchMode::Release | MatchMode::Dev | MatchMode::Practice => {}
            }
        }
        modes
    }

    #[test]
    fn match_mode_access_equals_the_server_access_without_a_worker() {
        for mode in every_mode() {
            let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
            let rt = ServerRuntime::new(socket, MatchConfig { mode, team_size: 2 });
            assert!(rt.match_service.worker().is_none());
            assert_eq!(
                DebugAccess::for_match_mode(rt.rules.mode_id()),
                rt.debug_access(),
                "{mode:?}"
            );
        }
    }

    /// Step 11f: the world snapshot to a joined player carries the server's
    /// own access (`Some`, even when both flags are false); the worker case is
    /// pinned in `match_allocation` (`saved_worker_repeats_victory_...`), the
    /// unjoined status reply in `runtime/prejoin/tests.rs`.
    #[test]
    fn joined_players_receive_the_server_access_in_every_snapshot() {
        let toggles_only = DebugAccess {
            toggles: true,
            practice: false,
        };
        let all = DebugAccess {
            toggles: true,
            practice: true,
        };
        let cases = [
            (MatchMode::Dev, toggles_only),
            (MatchMode::Practice, all),
            (MatchMode::Release, DebugAccess::default()),
        ];
        for (mode, expected) in cases {
            let clock = ManualClock::new(Instant::now());
            let transport = MemoryTransport::new("127.0.0.1:4100".parse().unwrap());
            let mut rt = ServerRuntime::for_test(
                transport.clone(),
                clock.clone(),
                career_backend::MemoryCareer::disabled(54200),
                MatchConfig { mode, team_size: 1 },
            );
            let addr: SocketAddr = "127.0.0.1:58990".parse().unwrap();
            let join = ClientPacket::Join {
                prematch: false,
                team: Team::Green,
                character: CharacterChoice::Ipfs,
                hero_class: HeroClass::Mage,
                avatar: None,
                sprite_character: None,
                session_id: Some(format!("debug-access-{mode:?}")),
                passport_ticket: None,
            };
            transport.push_inbound(addr, serde_json::to_vec(&join).unwrap());
            let (now, dt) = rt.prepare_tick();
            rt.tick(now, dt);
            assert!(rt.world.players[&addr].joined, "{mode:?}");
            transport.take_outbound();
            clock.advance(SNAPSHOT_INTERVAL);
            let (now, dt) = rt.prepare_tick();
            rt.tick(now, dt);
            let sent = transport.take_outbound();
            let snapshot = sent
                .iter()
                .filter(|(to, _)| *to == addr)
                .find_map(|(_, bytes)| match serde_json::from_slice(bytes) {
                    Ok(packet @ ServerPacket::Snapshot { .. }) => Some(packet),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{mode:?}: no snapshot"));
            let ServerPacket::Snapshot {
                debug_access,
                match_mode,
                ..
            } = snapshot
            else {
                unreachable!()
            };
            assert_eq!(debug_access, Some(expected), "{mode:?}");
            assert_eq!(debug_access, Some(rt.debug_access()), "{mode:?}");
            assert_eq!(
                DebugAccess::for_match_mode(&match_mode),
                expected,
                "{mode:?}: without a worker the fallback agrees"
            );
        }
    }
}
