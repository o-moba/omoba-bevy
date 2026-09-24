//! Combat packets: `Cast`, `BasicAttack` and `UpgradeSkill`.
use std::net::SocketAddr;
use std::ops::ControlFlow;
use std::time::Instant;

use shared::wire::TargetId;

use crate::basic_attack::handle_basic_attack_request;
use crate::runtime::ServerRuntime;
use crate::sim::cast::{apply_skill_upgrade, handle_cast_request};

impl ServerRuntime {
    pub(in crate::runtime) fn handle_cast(
        &mut self,
        addr: SocketAddr,
        target: TargetId,
        slot: u8,
        now: Instant,
    ) -> ControlFlow<()> {
        let world = &mut self.world;
        world.ensure_connected(addr, now);
        if let Some(player) = world.players.get_mut(&addr) {
            player.last_seen = now;
        }
        handle_cast_request(world, addr, target, slot, now);
        ControlFlow::Continue(())
    }

    /// A request stamped for another server epoch or match is dropped
    /// before the post-command tail.
    pub(in crate::runtime) fn handle_basic_attack(
        &mut self,
        addr: SocketAddr,
        target: TargetId,
        requested_epoch: u64,
        requested_match: u64,
        request_id: u64,
        now: Instant,
    ) -> ControlFlow<()> {
        if requested_epoch != self.server_epoch || requested_match != self.match_id {
            return ControlFlow::Break(());
        }
        handle_basic_attack_request(&mut self.world, addr, target, request_id, now);
        ControlFlow::Continue(())
    }

    pub(in crate::runtime) fn handle_upgrade_skill(
        &mut self,
        addr: SocketAddr,
        slot: u8,
        now: Instant,
    ) -> ControlFlow<()> {
        let world = &mut self.world;
        world.ensure_connected(addr, now);
        if let Some(player) = world.players.get_mut(&addr) {
            player.last_seen = now;
            if player.joined {
                apply_skill_upgrade(player, slot);
            }
        }
        ControlFlow::Continue(())
    }
}
