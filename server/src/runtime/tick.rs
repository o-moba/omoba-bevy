//! One simulation tick: prepare, simulate, then broadcast.

use crate::shop::accrue_passive_gold;

use shared::wire::TargetKind;

use crate::balance::VICTORY_REMATCH_DELAY;

use crate::formation::tick_match_formation;

use crate::game_world::TickCtx;

use crate::sim::neutrals::simulate_neutrals;

use crate::session::handle_respawns;

use std::time::Instant;

use crate::sim::regenerate_team_buff_hp;

use crate::game_world::GameWorld;

use crate::sim::minions::simulate_minions;

use shared::wire::GameState;

use crate::sim::regenerate_base_hp;

use crate::runtime::ServerRuntime;

use crate::world::spawn_minion_waves_if_due;

use crate::sim::regenerate_mana;

use std::collections::HashSet;

use crate::sim::towers::simulate_tower_attacks;

use crate::hero_timers;

use crate::sim::projectiles::simulate_projectiles_filtered;

use crate::sim::restore_god_mode_players;

impl ServerRuntime {
    pub(crate) fn prepare_tick(&mut self) -> (Instant, f32) {
        self.poll_career(self.clock.now());
        self.receive_packets();
        self.tick_match_service(self.clock.now());
        if self.match_service.is_public() {
            self.send_lobby_snapshots(self.clock.now());
        }

        let now = self.clock.now();
        let dt = now
            .duration_since(self.last_simulation_at)
            .as_secs_f32()
            .clamp(0.0, 0.1);
        self.last_simulation_at = now;
        self.sandbox.as_mut().map_or((now, dt), |s| s.advance(dt))
    }

    /// Advances the world by one simulation step and sends what changed.
    pub(crate) fn tick(&mut self, now: Instant, dt: f32) {
        regenerate_mana(&mut self.world.players, dt);
        // Minion-targeted projectiles resolve ahead of formation, bots and the
        // rest of the simulation, where the ECS combat systems used to run;
        // like them, a zero-length step leaves those projectiles alone.
        if dt > 0.0 {
            let minion_hits =
                simulate_projectiles_filtered(&mut self.world, TickCtx { now, dt }, |kind| {
                    kind == TargetKind::Minion
                });
            self.combat_log.extend(now, minion_hits);
        }
        if self.match_service.is_lobby() {
            self.maintain_roster(now);
            self.send_lobby_snapshots(now);
            self.send_career_views(now);
            return;
        }
        if self
            .match_service
            .worker()
            .is_some_and(|w| w.recovery || w.aborted)
        {
            self.send_career_views(now);
            return;
        }
        // Completed workers keep repeating Victory snapshots throughout their
        // retirement grace period. Victory gates gameplay below; a durable
        // result ACK must not turn one lossy UDP snapshot into the only signal.
        self.maintain_roster(if self.sandbox.is_some() {
            self.clock.now()
        } else {
            now
        });
        self.fill_practice_bots(now);
        // Formation's final interval belongs to the countdown, not earned income.
        let gold_dt = if matches!(self.world.game_state, GameState::Running) {
            dt
        } else {
            0.0
        };
        if !self.career_flow_active()
            && self
                .victory_at
                .is_some_and(|at| now.saturating_duration_since(at) >= VICTORY_REMATCH_DELAY)
        {
            self.restart_round(now);
        }
        self.advance_career_queue(now);
        if !self.tick_prematch(now) {
            tick_match_formation(&mut self.world, self.rules, dt, now);
        }
        self.track_round_start(now);
        self.simulate_bots(now, dt);
        self.simulate_sandbox(now, dt);
        let sandbox_minions_running = self
            .sandbox
            .as_ref()
            .is_none_or(|s| s.config.environment.minions && !s.config.environment.minions_paused);
        let sandbox_simulating = self.sandbox.is_none() || dt > 0.0;
        let career_flow = self.career_flow_active();
        let tick = TickCtx { now, dt };
        let world = &mut self.world;

        if !self.targeting_qa && sandbox_minions_running && sandbox_simulating {
            spawn_minion_waves_if_due(world, now);
            self.combat_log.extend(now, simulate_minions(world, tick));
        }
        if !self.targeting_qa && sandbox_simulating {
            let tower_events = simulate_tower_attacks(world, now);
            self.combat_log.extend(now, tower_events);
        }
        let projectile_events = if sandbox_simulating {
            simulate_projectiles_filtered(world, tick, |kind| kind != TargetKind::Minion)
        } else {
            Vec::new()
        };
        self.combat_log.extend(now, projectile_events);
        if !self.targeting_qa && sandbox_simulating {
            self.combat_log.extend(now, simulate_neutrals(world, tick));
        }
        if sandbox_simulating {
            world
                .forest_pickups
                .tick(&mut world.players, &world.game_state, now);
        }
        regenerate_team_buff_hp(world, tick);
        regenerate_base_hp(world, dt);
        accrue_passive_gold(&mut world.players, &world.game_state, gold_dt);
        restore_god_mode_players(world);
        handle_respawns(world, now);
        hero_timers::normalize_hero_timers(world);

        let live_player_ids = world
            .players
            .values()
            .map(|player| player.hero.identity.id)
            .collect::<HashSet<_>>();
        let GameWorld {
            projectiles,
            minions,
            structures,
            neutrals,
            ..
        } = world;
        projectiles.retain(|_, projectile| match projectile.target.kind {
            TargetKind::Player => live_player_ids.contains(&projectile.target.id),
            TargetKind::Minion => minions
                .get(&projectile.target.id)
                .is_some_and(|minion| minion.state.hp > 0.0),
            TargetKind::Structure => structures
                .get(&projectile.target.id)
                .is_some_and(|structure| structure.state.hp > 0.0),
            TargetKind::Neutral => neutrals
                .get(&projectile.target.id)
                .is_some_and(|neutral| neutral.dead_until.is_none() && neutral.state.hp > 0.0),
        });

        minions.retain(|_, minion| minion.state.hp > 0.0);

        self.broadcast_snapshots(now, career_flow);
        self.record_match_metrics(now);
        self.checkpoint_career_round(now);
        self.send_career_views(now);
        self.send_social_views(now);
    }
}
