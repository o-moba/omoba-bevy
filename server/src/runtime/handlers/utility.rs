//! Utility packets: `Utility` (dash, blink and the other utility actions).
use std::net::SocketAddr;
use std::ops::ControlFlow;
use std::time::Instant;

use shared::utility::UtilityAction;

use crate::runtime::ServerRuntime;
use crate::utility::handle_utility_request;

impl ServerRuntime {
    /// A request stamped for another server epoch or match is dropped
    /// before the post-command tail.
    pub(in crate::runtime) fn handle_utility(
        &mut self,
        addr: SocketAddr,
        action: UtilityAction,
        direction: [f32; 2],
        requested_epoch: u64,
        requested_match: u64,
        request_id: u64,
        now: Instant,
    ) -> ControlFlow<()> {
        if requested_epoch != self.server_epoch || requested_match != self.match_id {
            return ControlFlow::Break(());
        }
        let world = &mut self.world;
        if let Some(player) = world.players.get_mut(&addr) {
            handle_utility_request(
                player,
                &world.map_layout,
                &world.structures,
                &world.game_state,
                action,
                direction,
                request_id,
                now,
            );
        }
        ControlFlow::Continue(())
    }
}
