//! Session packets: `Hello`, `Ping`, `Leave` and `RequestRematch`.
use std::net::SocketAddr;
use std::ops::ControlFlow;
use std::time::Instant;

use shared::wire::GameState;

use crate::runtime::ServerRuntime;

impl ServerRuntime {
    pub(in crate::runtime) fn handle_hello(
        &mut self,
        addr: SocketAddr,
        protocol_version: u16,
        now: Instant,
    ) -> ControlFlow<()> {
        let world = &mut self.world;
        world.ensure_connected(addr, now);
        let player = world.players.get_mut(&addr).unwrap();
        player.last_seen = now;
        player.framed_snapshots = true;
        player.protocol_compatible = protocol_version == shared::protocol::PROTOCOL_VERSION;
        player.join_error = (!player.protocol_compatible)
            .then_some(shared::protocol::JoinRejection::ProtocolMismatch);
        ControlFlow::Continue(())
    }

    pub(in crate::runtime) fn handle_ping(
        &mut self,
        addr: SocketAddr,
        now: Instant,
    ) -> ControlFlow<()> {
        let world = &mut self.world;
        world.ensure_connected(addr, now);
        if let Some(player) = world.players.get_mut(&addr) {
            player.last_seen = now;
        }
        ControlFlow::Continue(())
    }

    /// Runs before the pause gate and skips the post-command tail.
    pub(in crate::runtime) fn handle_leave(
        &mut self,
        addr: SocketAddr,
        now: Instant,
    ) -> ControlFlow<()> {
        // Public transport authenticated this command independently of the
        // account-action throttle. It is also the reliable local-exit path
        // when an immediately preceding CancelQueue datagram was dropped.
        if self.match_service.is_lobby()
            && self.career.backend.authenticated_session(addr).is_some()
            && let Some(profile) = self.career.backend.profile(addr)
        {
            self.match_service.cancel(&profile.profile_id);
        }
        self.leave_match(addr, now);
        ControlFlow::Break(())
    }

    /// `RequestRematch` while the career flow is active: the career's play
    /// again path. Runs before the pause gate and skips the post-command tail.
    pub(in crate::runtime) fn handle_career_rematch(
        &mut self,
        addr: SocketAddr,
        now: Instant,
    ) -> ControlFlow<()> {
        self.career_play_again(addr, now);
        ControlFlow::Break(())
    }

    /// `RequestRematch` outside the career flow: restart a finished round.
    pub(in crate::runtime) fn handle_request_rematch(
        &mut self,
        addr: SocketAddr,
        now: Instant,
    ) -> ControlFlow<()> {
        let world = &mut self.world;
        world.ensure_connected(addr, now);
        let mut joined = false;
        if let Some(player) = world.players.get_mut(&addr) {
            player.last_seen = now;
            joined = player.joined;
        }
        if joined && matches!(world.game_state, GameState::Victory { .. }) {
            self.restart_round(now);
        }
        ControlFlow::Continue(())
    }
}
