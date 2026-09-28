//! Opt-in native QA for the controls guide's controller path and the Settings
//! return (R6.5), which the match and shell runs do not reach: with
//! `OMOBA_HELP_QA_OUTPUT=<dir>` the client opens the guide over Home with the
//! kit focus driven as a controller would, closes it with a gamepad back
//! press, then opens it from Settings → Controls and closes it again. It
//! writes one frame per step and `result.json`, and exits non-zero when a
//! check fails. No server is needed. Window size from `OMOBA_QA_WIDTH` /
//! `OMOBA_QA_HEIGHT` (phone: add `OMOBA_TOUCH_CONTROLS=1`).
use std::path::PathBuf;
use std::time::{Duration, Instant};

use bevy::{
    app::AppExit,
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    window::PrimaryWindow,
};

use crate::frontend::AppScreen;
use crate::help_overlay::HelpOverlayVisible;
use crate::pause_menu::PauseMenuState;
use crate::ui::{BackPress, UiFocus};

pub(crate) struct HelpQaPlugin;

impl Plugin for HelpQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(directory) = std::env::var_os("OMOBA_HELP_QA_OUTPUT")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
        else {
            return;
        };
        let dimension = |name: &str, fallback: u32| {
            std::env::var(name)
                .ok()
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(fallback)
                .clamp(320, 3840)
        };
        app.insert_resource(bevy::winit::WinitSettings::continuous())
            .insert_resource(HelpQa {
                directory,
                size: UVec2::new(
                    dimension("OMOBA_QA_WIDTH", 1280),
                    dimension("OMOBA_QA_HEIGHT", 720),
                ),
                started: Instant::now(),
                stage: 0,
                frames: 0,
                pending: false,
                checks: Vec::new(),
                next: None,
                action: None,
            })
            .add_systems(
                Update,
                drive_focus
                    .before(crate::ui::UiSet::Focus)
                    .in_set(crate::input_context::InputContextSet::Modal),
            )
            .add_systems(PostUpdate, step.after(bevy::ui::UiSystems::Layout));
    }
}

#[derive(Resource)]
struct HelpQa {
    directory: PathBuf,
    size: UVec2,
    started: Instant,
    stage: usize,
    frames: u32,
    pending: bool,
    checks: Vec<(String, bool)>,
    /// The action after the pending frame, then the one to run this frame.
    next: Option<QaAction>,
    action: Option<QaAction>,
}

#[derive(Clone, Copy, Debug)]
enum QaAction {
    OpenHelp,
    /// A gamepad East (`BackPress`).
    Back,
    OpenSettings,
    PressControls,
}

/// Drives the kit focus as a connected controller does and runs the step's
/// action in `Update`, where the guide and the menu read their input.
#[allow(clippy::too_many_arguments)]
fn drive_focus(
    mut qa: ResMut<HelpQa>,
    mut focus: ResMut<UiFocus>,
    mut help: ResMut<HelpOverlayVisible>,
    mut menu: ResMut<PauseMenuState>,
    mut back: ResMut<BackPress>,
    ids: Query<(Entity, &crate::ui::TestId)>,
    mut presses: MessageWriter<crate::ui::SyntheticPress>,
) {
    focus.set_enabled(true);
    match qa.action.take() {
        Some(QaAction::OpenHelp) => help.0 = true,
        Some(QaAction::Back) => back.press(),
        Some(QaAction::OpenSettings) => {
            menu.open = true;
            menu.in_settings = true;
        }
        Some(QaAction::PressControls) => {
            let button = ids
                .iter()
                .find(|(_, id)| id.as_str() == "PauseMenuSettingsControlsButton")
                .map(|(entity, _)| entity);
            match button {
                Some(button) => {
                    presses.write(crate::ui::SyntheticPress(button));
                }
                None => qa.checks.push(("Settings has Controls".into(), false)),
            }
        }
        None => {}
    }
}

/// Frames to settle each step (layout, the open animation, the focus ring).
const SETTLE: u32 = 45;

fn step(
    mut commands: Commands,
    mut qa: ResMut<HelpQa>,
    screen: Res<State<AppScreen>>,
    help: Res<HelpOverlayVisible>,
    menu: Res<PauseMenuState>,
    focus: Res<UiFocus>,
    ids: Query<(Entity, &crate::ui::TestId)>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.started.elapsed() > Duration::from_secs(120) {
        error!("HELP_QA failed: timed out at stage {}", qa.stage);
        exit.write(AppExit::error());
        return;
    }
    if let Ok(mut window) = windows.single_mut() {
        window.resolution.set_scale_factor_override(Some(1.0));
        if window.physical_width() != qa.size.x || window.physical_height() != qa.size.y {
            window
                .resolution
                .set_physical_resolution(qa.size.x, qa.size.y);
        }
    }
    if qa.pending || *screen.get() != AppScreen::Home {
        return;
    }
    qa.frames += 1;
    if qa.frames < SETTLE {
        return;
    }
    let dismiss = ids
        .iter()
        .find(|(_, id)| id.as_str() == "HelpDismissButton")
        .map(|(entity, _)| entity);
    let h = qa.size.y;
    let (shot, action) = match qa.stage {
        0 => (None, Some(QaAction::OpenHelp)),
        1 => {
            qa.checks.push((
                "focus starts on HelpDismissButton".into(),
                dismiss.is_some() && focus.focused() == dismiss,
            ));
            (
                Some(format!("01-help-focus-{h}p.png")),
                Some(QaAction::Back),
            )
        }
        2 => {
            qa.checks.push((
                "gamepad back closed the guide".into(),
                !help.0 && !menu.open,
            ));
            (
                Some(format!("02-help-closed-{h}p.png")),
                Some(QaAction::OpenSettings),
            )
        }
        3 => (
            Some(format!("03-settings-{h}p.png")),
            Some(QaAction::PressControls),
        ),
        4 => {
            qa.checks.push((
                "Controls opened the guide over the closed menu".into(),
                help.0 && !menu.open,
            ));
            qa.checks.push((
                "focus on HelpDismissButton over Settings".into(),
                dismiss.is_some() && focus.focused() == dismiss,
            ));
            (
                Some(format!("04-settings-controls-{h}p.png")),
                Some(QaAction::Back),
            )
        }
        5 => {
            qa.checks.push((
                "closing the guide returned to Settings (R6.5)".into(),
                !help.0 && menu.open && menu.in_settings,
            ));
            (Some(format!("05-settings-return-{h}p.png")), None)
        }
        _ => {
            let pass = qa.checks.iter().all(|(_, ok)| *ok);
            let result = serde_json::json!({
                "pass": pass,
                "size": [qa.size.x, qa.size.y],
                "checks": qa.checks.iter().map(|(name, ok)| serde_json::json!({"check": name, "pass": ok})).collect::<Vec<_>>(),
            });
            let written = std::fs::create_dir_all(&qa.directory).and_then(|()| {
                std::fs::write(
                    qa.directory.join("result.json"),
                    serde_json::to_string_pretty(&result).unwrap_or_default(),
                )
            });
            if pass && written.is_ok() {
                info!("HELP_QA passed: {result}");
                exit.write(AppExit::Success);
            } else {
                error!("HELP_QA failed: {result}");
                exit.write(AppExit::error());
            }
            qa.stage += 1;
            return;
        }
    };
    qa.frames = 0;
    let Some(name) = shot else {
        qa.action = action;
        qa.stage += 1;
        return;
    };
    if std::fs::create_dir_all(&qa.directory).is_err() {
        exit.write(AppExit::error());
        return;
    }
    // The frame shows this step's state; its action runs once it is saved.
    qa.pending = true;
    qa.next = action;
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(qa.directory.join(name)))
        .observe(|_: On<ScreenshotCaptured>, mut qa: ResMut<HelpQa>| {
            qa.pending = false;
            qa.stage += 1;
            qa.action = qa.next.take();
        });
}
