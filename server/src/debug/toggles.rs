//! The development toggles: god mode and the speed boost. `handle_debug`
//! has already checked `debug_access().toggles` (Dev or local Practice, never
//! a worker-allocated round).
use std::net::SocketAddr;
use std::ops::ControlFlow;
use std::time::Instant;

use crate::balance::DEBUG_SPEED_MULTIPLIER;
use crate::runtime::ServerRuntime;

impl ServerRuntime {
    /// Invulnerability plus, outside Combat Test, a full resource pool.
    /// `Continue` once applied, so the dispatcher's post-command tail runs.
    pub(super) fn set_god_mode(
        &mut self,
        addr: SocketAddr,
        enabled: bool,
        now: Instant,
    ) -> ControlFlow<()> {
        let combat_sandbox = self.sandbox_allowed();
        let world = &mut self.world;
        world.ensure_connected(addr, now);
        if let Some(player) = world.players.get_mut(&addr) {
            player.last_seen = now;
            if !player.joined {
                return ControlFlow::Break(());
            }
            if player.modifiers.god_mode != enabled {
                println!("Player {} god_mode={}", player.hero.identity.id, enabled);
            }
            player.modifiers.god_mode = enabled;
            // The development toggle is invulnerability plus a full
            // pool; a sandbox actor's resources stay its own setting.
            if !combat_sandbox {
                player.modifiers.infinite_resource = enabled;
            }
            if enabled {
                player.hero.hp = player.hero.max_hp;
                player.hero.mana = player.hero.max_mana;
                player.timers.respawn_at = None;
            }
        }
        ControlFlow::Continue(())
    }

    /// `DEBUG_SPEED_MULTIPLIER` on the hero's movement speed.
    pub(super) fn set_speed_boost(
        &mut self,
        addr: SocketAddr,
        enabled: bool,
        now: Instant,
    ) -> ControlFlow<()> {
        let world = &mut self.world;
        world.ensure_connected(addr, now);
        if let Some(player) = world.players.get_mut(&addr) {
            player.last_seen = now;
            if !player.joined {
                return ControlFlow::Break(());
            }
            let mult = if enabled { DEBUG_SPEED_MULTIPLIER } else { 1.0 };
            if (player.modifiers.move_speed_mult - mult).abs() > f32::EPSILON {
                println!("Player {} speed_boost={}", player.hero.identity.id, enabled);
            }
            player.modifiers.move_speed_mult = mult;
        }
        ControlFlow::Continue(())
    }
}
