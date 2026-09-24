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

mod practice;
mod toggles;

use std::net::SocketAddr;
use std::ops::ControlFlow;
use std::time::Instant;

use shared::debug::{DebugAccess, DebugCommand};

use crate::runtime::ServerRuntime;

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
    use std::net::UdpSocket;

    use shared::debug::DebugAccess;

    use crate::match_rules::{MatchConfig, MatchMode};
    use crate::runtime::ServerRuntime;

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
}
