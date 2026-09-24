//! The Combat Test `Sandbox` packet. The debug command family (`SetGodMode`,
//! `SetSpeedBoost`, `Practice`) goes through `crate::debug`.
use std::net::SocketAddr;
use std::ops::ControlFlow;
use std::time::Instant;

use shared::sandbox::SandboxRequest;

use crate::runtime::ServerRuntime;

impl ServerRuntime {
    /// Runs before the pause gate and skips the post-command tail.
    pub(in crate::runtime) fn handle_sandbox_packet(
        &mut self,
        addr: SocketAddr,
        request: SandboxRequest,
        now: Instant,
    ) -> ControlFlow<()> {
        self.initialize_sandbox_players();
        self.handle_sandbox(addr, request);
        if let Some(p) = self.world.players.get_mut(&addr) {
            p.last_seen = now;
        }
        ControlFlow::Break(())
    }
}
