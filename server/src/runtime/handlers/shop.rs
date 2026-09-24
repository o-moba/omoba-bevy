//! Shop packets: `BuyItem`.
use std::net::SocketAddr;
use std::ops::ControlFlow;
use std::time::Instant;

use crate::runtime::ServerRuntime;
use crate::shop::handle_purchase;

impl ServerRuntime {
    /// A purchase stamped for another match or server epoch is dropped
    /// before the post-command tail.
    pub(in crate::runtime) fn handle_buy_item(
        &mut self,
        addr: SocketAddr,
        item_id: String,
        request_id: u64,
        requested_match: u64,
        requested_epoch: u64,
        now: Instant,
    ) -> ControlFlow<()> {
        let match_id = self.match_id;
        if requested_match != match_id || requested_epoch != self.server_epoch {
            // A delayed datagram must not consume the new round's sequence.
            return ControlFlow::Break(());
        }
        let world = &mut self.world;
        world.ensure_connected(addr, now);
        if let Some(player) = world.players.get_mut(&addr) {
            player.last_seen = now;
            handle_purchase(
                player,
                &world.map_layout,
                &world.game_state,
                &item_id,
                request_id,
                match_id,
            );
        }
        ControlFlow::Continue(())
    }
}
