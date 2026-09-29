//! Online tick lifecycle around the shared combat phases.
use crate::{
    balance::VICTORY_REMATCH_DELAY, formation::tick_match_formation, runtime::ServerRuntime,
};
use shared::wire::GameState;
use std::time::Instant;

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
        common::tick::prepare(&mut self.world, &mut self.combat_log, now, dt);
        if self.match_service.is_lobby() {
            self.maintain_roster(now);
            self.send_lobby_snapshots(now);
            self.send_career_views(now);
            self.tick_party(now);
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
        common::tick::skills(&mut self.world, &mut self.combat_log, now, dt);
        self.simulate_bots(now, dt);
        self.simulate_sandbox(now, dt);
        let sandbox_minions_running = self
            .sandbox
            .as_ref()
            .is_none_or(|s| s.config.environment.minions && !s.config.environment.minions_paused);
        let sandbox_simulating = self.sandbox.is_none() || dt > 0.0;
        let career_flow = self.career_flow_active();
        common::tick::finish(
            &mut self.world,
            &mut self.combat_log,
            now,
            dt,
            gold_dt,
            common::tick::TickOptions {
                targeting_qa: self.targeting_qa,
                minions_running: sandbox_minions_running,
                simulating: sandbox_simulating,
            },
        );

        self.broadcast_snapshots(now, career_flow);
        self.record_match_metrics(now);
        self.settle_finished_round(now);
        self.checkpoint_career_round(now);
        self.send_career_views(now);
        self.send_social_views(now);
        self.tick_party(now);
    }
}
