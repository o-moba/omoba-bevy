//! Opt-in end-to-end capture of the front-end flow against a real server.
//!
//! Unlike [`crate::frontend_qa`], which sets each screen directly, this harness
//! presses the actual buttons (it flips their [`Interaction`] to `Pressed`, the
//! same signal a click produces) and then lets the session drive the shell. It
//! exists to prove the ordering the shell promises: nothing is sent to the
//! server from the home screen, and the match world only appears after a hero
//! is locked in.
//!
//! Run it against a practice server:
//! `OMOBA_MATCH_MODE=practice cargo run -p server` then
//! `OMOBA_FRONTEND_QA_FLOW=1 OMOBA_FRONTEND_QA_OUTPUT=<dir> cargo run -p client`.

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use bevy::{
    app::AppExit,
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
    window::PrimaryWindow,
};

use crate::frontend::AppScreen;
use crate::net::ClientSession;
use crate::player::Player;

/// Frames to hold the home screen before pressing PLAY, so the "no join was
/// sent from the menus" observation covers more than a single tick.
const HOME_FRAMES: u32 = 45;
/// Frames to spend on hero select before locking in.
const SELECT_FRAMES: u32 = 20;
/// Frames to let the match settle before the in-match capture.
const MATCH_FRAMES: u32 = 45;

pub(crate) struct FrontendFlowQaPlugin;

impl Plugin for FrontendFlowQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(directory) = std::env::var_os("OMOBA_FRONTEND_QA_OUTPUT")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
        else {
            return;
        };
        app.insert_resource(FlowQa {
            directory,
            started: Instant::now(),
            step: FlowStep::Home,
            frames: 0,
            trace: Vec::new(),
            join_committed_on_home: false,
            world_before_lock_in: false,
            captured: false,
            finished: false,
        })
        .insert_resource(bevy::winit::WinitSettings::continuous())
        .add_systems(Update, drive_flow);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FlowStep {
    Home,
    HeroSelect,
    AwaitMatch,
    Capture,
    Done,
}

#[derive(Resource)]
struct FlowQa {
    directory: PathBuf,
    started: Instant,
    step: FlowStep,
    frames: u32,
    /// Screen sequence the session actually produced, in order.
    trace: Vec<String>,
    /// Set if a join was committed while the player was still in the menus.
    join_committed_on_home: bool,
    /// Set if a local hero existed before the lock-in.
    world_before_lock_in: bool,
    captured: bool,
    finished: bool,
}

impl FlowQa {
    fn record(&mut self, screen: AppScreen) {
        let name = format!("{screen:?}");
        if self.trace.last() != Some(&name) {
            self.trace.push(name);
        }
    }

    fn finish(&mut self, status: &str, reason: Option<&str>, exit: &mut MessageWriter<AppExit>) {
        self.finished = true;
        let _ = std::fs::create_dir_all(&self.directory);
        let passed = status == "passed";
        let value = serde_json::json!({
            "status": status,
            "reason": reason,
            "scenario": "frontend-flow",
            "version": env!("CARGO_PKG_VERSION"),
            "method": "synthetic Interaction::Pressed on the real buttons, live UDP server",
            "screen_trace": self.trace,
            "join_committed_from_the_menus": self.join_committed_on_home,
            "local_hero_before_lock_in": self.world_before_lock_in,
            "in_match_capture": passed.then(|| "07-in-match.png".to_owned()),
            "manual_input_verified": false,
        });
        let _ = std::fs::write(
            self.directory.join("qa-flow.json"),
            serde_json::to_vec_pretty(&value).unwrap(),
        );
        if passed {
            exit.write(AppExit::Success);
        } else {
            error!("FRONTEND_FLOW_QA failed: {}", reason.unwrap_or(status));
            exit.write(AppExit::error());
        }
    }
}

/// Presses the button with this name, if it is on screen.
fn press(buttons: &mut Query<(&Name, &mut Interaction)>, wanted: &str) -> bool {
    for (name, mut interaction) in buttons.iter_mut() {
        if name.as_str() == wanted {
            *interaction = Interaction::Pressed;
            return true;
        }
    }
    false
}

fn drive_flow(
    mut commands: Commands,
    mut qa: ResMut<FlowQa>,
    screen: Res<State<AppScreen>>,
    session: Res<ClientSession>,
    players: Query<(), With<Player>>,
    mut buttons: Query<(&Name, &mut Interaction)>,
    windows: Query<Entity, With<PrimaryWindow>>,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.finished {
        return;
    }
    let current = *screen.get();
    qa.record(current);
    if qa.started.elapsed() > Duration::from_secs(180) {
        qa.finish("failed", Some("180-second flow deadline"), &mut exit);
        return;
    }
    qa.frames += 1;
    match qa.step {
        FlowStep::Home => {
            // The promise under test: the menus never talk to the match.
            if session.join_flow_committed {
                qa.join_committed_on_home = true;
            }
            if !players.is_empty() {
                qa.world_before_lock_in = true;
            }
            if qa.frames < HOME_FRAMES {
                return;
            }
            if qa.join_committed_on_home || qa.world_before_lock_in {
                qa.finish(
                    "failed",
                    Some("the client joined a match from the home screen"),
                    &mut exit,
                );
                return;
            }
            if press(&mut buttons, "HomePlay") {
                qa.step = FlowStep::HeroSelect;
                qa.frames = 0;
            }
        }
        FlowStep::HeroSelect => {
            if current != AppScreen::HeroSelect {
                return;
            }
            if session.join_flow_committed {
                qa.join_committed_on_home = true;
            }
            if !players.is_empty() {
                qa.world_before_lock_in = true;
            }
            if qa.frames < SELECT_FRAMES {
                return;
            }
            if press(&mut buttons, "TeamGreenButton") {
                qa.step = FlowStep::AwaitMatch;
                qa.frames = 0;
            }
        }
        FlowStep::AwaitMatch => {
            if current == AppScreen::InMatch {
                qa.step = FlowStep::Capture;
                qa.frames = 0;
            }
        }
        FlowStep::Capture => {
            if qa.frames < MATCH_FRAMES {
                return;
            }
            if !qa.captured {
                if std::fs::create_dir_all(&qa.directory).is_err() {
                    qa.finish("failed", Some("cannot create capture directory"), &mut exit);
                    return;
                }
                commands
                    .spawn(Screenshot::primary_window())
                    .observe(save_to_disk(qa.directory.join("07-in-match.png")));
                qa.captured = true;
                return;
            }
            if qa
                .directory
                .join("07-in-match.png")
                .metadata()
                .is_ok_and(|file| file.len() > 32)
            {
                qa.step = FlowStep::Done;
            }
        }
        FlowStep::Done => {
            for window in &windows {
                commands.entity(window).despawn();
            }
            qa.finish("passed", None, &mut exit);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_trace_keeps_order_without_repeating_a_held_screen() {
        let mut qa = FlowQa {
            directory: PathBuf::from("/tmp"),
            started: Instant::now(),
            step: FlowStep::Home,
            frames: 0,
            trace: Vec::new(),
            join_committed_on_home: false,
            world_before_lock_in: false,
            captured: false,
            finished: false,
        };
        qa.record(AppScreen::Home);
        qa.record(AppScreen::Home);
        qa.record(AppScreen::HeroSelect);
        qa.record(AppScreen::Home);
        assert_eq!(qa.trace, ["Home", "HeroSelect", "Home"]);
    }
}
