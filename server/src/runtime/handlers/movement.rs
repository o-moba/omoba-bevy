//! Movement packets: `Transform`.
use std::net::SocketAddr;
use std::ops::ControlFlow;
use std::time::Instant;

use shared::wire::GameState;

use crate::runtime::ServerRuntime;
use crate::session::handle_transform_request_with_structures;

impl ServerRuntime {
    pub(in crate::runtime) fn handle_transform(
        &mut self,
        addr: SocketAddr,
        x: f32,
        y: f32,
        z: f32,
        yaw: f32,
        dash_sequence: u64,
        now: Instant,
    ) -> ControlFlow<()> {
        let world = &mut self.world;
        world.ensure_connected(addr, now);
        if let Some(player) = world.players.get_mut(&addr) {
            player.last_seen = now;
        }
        if matches!(world.game_state, GameState::Running)
            && let Some(player) = world.players.get_mut(&addr)
            && player.hero.hp > 0.0
            && dash_sequence == player.hero.utility.dash_sequence
        {
            handle_transform_request_with_structures(
                player,
                &world.map_layout,
                &world.structures,
                x,
                y,
                z,
                yaw,
                now,
            );
        }
        ControlFlow::Continue(())
    }
}
