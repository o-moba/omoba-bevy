//! One presentation clock for particles, impact marks, damage numbers,
//! projectile trails and skill-effect animation. In a Combat Test sandbox it
//! follows the authoritative simulation (time scale, pause, frame step), so a
//! slowed or paused frame shows every layer at the same moment. Without a
//! sandbox it equals Bevy `Time`.
use bevy::prelude::*;

use crate::{net::GameStateSnapshot, player::SandboxVisualClock};

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct VfxClock {
    /// Sandbox `simulation_secs` when a sandbox snapshot exists, else `Time::elapsed`.
    pub now: f64,
    /// Seconds one-shot presentation ages by this frame.
    pub delta: f32,
}

pub(crate) struct VfxClockPlugin;
impl Plugin for VfxClockPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VfxClock>()
            .add_systems(First, advance.after(bevy::time::TimeSystems));
    }
}

/// Adds the clock for a consumer plugin that is built on its own (unit
/// tests); in the game `PresentationPlugins` adds it before every consumer.
pub(crate) fn ensure(app: &mut App) {
    if !app.is_plugin_added::<VfxClockPlugin>() {
        app.add_plugins(VfxClockPlugin);
    }
}

fn advance(
    time: Res<Time>,
    game: Option<Res<GameStateSnapshot>>,
    mut pace: Local<SandboxVisualClock>,
    mut clock: ResMut<VfxClock>,
) {
    let game = game.as_deref();
    clock.delta = pace.delta(&time, game);
    clock.now = game
        .and_then(|game| game.sandbox.as_ref())
        .map_or(time.elapsed_secs_f64(), |sandbox| sandbox.simulation_secs);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const FRAME: Duration = Duration::from_millis(16);

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(FRAME))
            .add_plugins(VfxClockPlugin);
        app
    }

    fn sandbox_game() -> GameStateSnapshot {
        GameStateSnapshot {
            meta: shared::protocol::SnapshotMeta::new(77, 1, 1),
            sandbox: Some(shared::sandbox::SandboxSnapshot {
                config: Default::default(),
                ack: None,
                last_request_id: 0,
                actors: Vec::new(),
                analytics: Default::default(),
                simulation_secs: 10.0,
                frame: 0,
            }),
            ..Default::default()
        }
    }

    fn sandbox(app: &mut App) -> Mut<'_, shared::sandbox::SandboxSnapshot> {
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .map_unchanged(|game| game.sandbox.as_mut().unwrap())
    }

    #[test]
    fn vfx_clock_equals_time_without_a_sandbox() {
        let mut app = app();
        for with_snapshot in [false, true] {
            if with_snapshot {
                // A match snapshot without a sandbox changes nothing.
                app.insert_resource(GameStateSnapshot::default());
            }
            for _ in 0..3 {
                app.update();
                let time = *app.world().resource::<Time>();
                let clock = *app.world().resource::<VfxClock>();
                assert_eq!(clock.delta, time.delta_secs());
                assert_eq!(clock.now, time.elapsed_secs_f64());
            }
        }
        assert!(app.world().resource::<VfxClock>().delta > 0.0);
    }

    #[test]
    fn vfx_clock_follows_sandbox_time_scale_pause_and_frame_step() {
        let mut app = app();
        app.insert_resource(sandbox_game());
        app.update();
        let frame = FRAME.as_secs_f32();
        for scale in shared::sandbox::TIME_SCALES {
            sandbox(&mut app).config.environment.time_scale = scale;
            app.update();
            let clock = *app.world().resource::<VfxClock>();
            assert!((clock.delta - frame * scale).abs() < 1e-6, "{scale}");
            assert_eq!(clock.now, 10.0);
        }
        // Entering pause stops presentation even while snapshots keep arriving.
        sandbox(&mut app).config.environment.paused = true;
        for _ in 0..3 {
            app.update();
            assert_eq!(app.world().resource::<VfxClock>().delta, 0.0);
        }
        // An authoritative frame step advances it by exactly that step.
        sandbox(&mut app).simulation_secs += 1.0 / 60.0;
        app.update();
        let clock = *app.world().resource::<VfxClock>();
        assert!((clock.delta - 1.0 / 60.0).abs() < 1e-6);
        assert!((clock.now - (10.0 + 1.0 / 60.0)).abs() < 1e-9);
        app.update();
        assert_eq!(app.world().resource::<VfxClock>().delta, 0.0);
        // Leaving the sandbox returns to `Time`.
        app.world_mut().resource_mut::<GameStateSnapshot>().sandbox = None;
        app.update();
        let time = *app.world().resource::<Time>();
        let clock = *app.world().resource::<VfxClock>();
        assert_eq!(clock.delta, time.delta_secs());
        assert_eq!(clock.now, time.elapsed_secs_f64());
    }
}
