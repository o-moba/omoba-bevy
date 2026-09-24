//! Debug and tooling packets: `SetGodMode`, `SetSpeedBoost`, `Practice` and
//! `Sandbox`.
use std::net::SocketAddr;
use std::ops::ControlFlow;
use std::time::Instant;

use shared::practice::PracticeCommand;
use shared::sandbox::SandboxRequest;

use crate::balance::DEBUG_SPEED_MULTIPLIER;
use crate::runtime::ServerRuntime;

impl ServerRuntime {
    /// Development and local bot practice only; both are unrated.
    pub(in crate::runtime) fn handle_set_god_mode(
        &mut self,
        addr: SocketAddr,
        enabled: bool,
        now: Instant,
    ) -> ControlFlow<()> {
        let combat_sandbox = self.sandbox_allowed();
        if !self.rules.debug_commands {
            return ControlFlow::Break(());
        }
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

    pub(in crate::runtime) fn handle_set_speed_boost(
        &mut self,
        addr: SocketAddr,
        enabled: bool,
        now: Instant,
    ) -> ControlFlow<()> {
        if !self.rules.debug_commands {
            return ControlFlow::Break(());
        }
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

    /// Runs before the pause gate and skips the post-command tail.
    pub(in crate::runtime) fn handle_practice(
        &mut self,
        addr: SocketAddr,
        command: PracticeCommand,
        now: Instant,
    ) -> ControlFlow<()> {
        if let Some(player) = self.world.players.get_mut(&addr) {
            player.last_seen = now;
        }
        self.handle_practice_command(addr, command, now);
        ControlFlow::Break(())
    }

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
