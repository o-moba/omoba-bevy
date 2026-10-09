//! Opt-in native capture of the boot splash: with
//! `OMOBA_BOOT_SPLASH_SHOTS=<dir>` the client starts as a player's launch
//! does (the splash is on), saves `boot-splash.png` while it is up and
//! `boot-home.png` after it left, writes `result.json` and exits non-zero
//! when a check fails. No server is needed. The window is 1180×820 logical
//! px at scale 1 (the iPad-class viewport of the other captures).
use std::path::PathBuf;
use std::time::{Duration, Instant};

use bevy::{
    app::AppExit,
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    window::PrimaryWindow,
};

use crate::frontend::AppScreen;
use crate::frontend::boot::{BOOT_QA_DIR, BootSplash};
use crate::ui::{ModalId, ModalStack};

const SIZE: UVec2 = UVec2::new(1180, 820);
/// Splash time before its frame is saved: the art and the fonts are in.
const SPLASH_SETTLE: Duration = Duration::from_millis(1100);
const HOME_SETTLE: Duration = Duration::from_millis(900);

pub(crate) struct BootQaPlugin;

impl Plugin for BootQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(directory) = std::env::var_os(BOOT_QA_DIR)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
        else {
            return;
        };
        app.insert_resource(bevy::winit::WinitSettings::continuous())
            .insert_resource(BootQa {
                directory,
                started: Instant::now(),
                since: Instant::now(),
                stage: 0,
                pending: false,
                checks: Vec::new(),
            })
            .add_systems(PostUpdate, step.after(bevy::ui::UiSystems::Layout));
    }
}

#[derive(Resource)]
struct BootQa {
    directory: PathBuf,
    started: Instant,
    since: Instant,
    stage: u8,
    pending: bool,
    checks: Vec<(&'static str, bool)>,
}

#[allow(clippy::too_many_arguments)]
fn step(
    mut commands: Commands,
    mut qa: ResMut<BootQa>,
    splash: Res<BootSplash>,
    modals: Res<ModalStack>,
    screen: Res<State<AppScreen>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.started.elapsed() > Duration::from_secs(60) {
        error!("BOOT_QA failed: timed out at stage {}", qa.stage);
        exit.write(AppExit::error());
        return;
    }
    if let Ok(mut window) = windows.single_mut() {
        window.resolution.set_scale_factor_override(Some(1.0));
        if window.physical_width() != SIZE.x || window.physical_height() != SIZE.y {
            window.resolution.set_physical_resolution(SIZE.x, SIZE.y);
        }
    }
    if qa.pending {
        return;
    }
    let shot = match qa.stage {
        0 => {
            if qa.since.elapsed() < SPLASH_SETTLE {
                return;
            }
            let up = splash.blocks_input();
            qa.checks.push(("the splash is up after launch", up));
            qa.checks.push((
                "the splash is the top modal, so the menu is inert",
                modals.top() == Some(ModalId::Boot),
            ));
            "boot-splash.png"
        }
        1 => {
            if splash.blocks_input() {
                qa.since = Instant::now();
                return;
            }
            if qa.since.elapsed() < HOME_SETTLE {
                return;
            }
            qa.checks.push((
                "the splash left and released the menu",
                !modals.contains(ModalId::Boot),
            ));
            qa.checks
                .push(("the shell is on Home", *screen.get() == AppScreen::Home));
            "boot-home.png"
        }
        _ => {
            let pass = qa.checks.iter().all(|(_, ok)| *ok);
            let result = serde_json::json!({
                "pass": pass,
                "size": [SIZE.x, SIZE.y],
                "seconds": qa.started.elapsed().as_secs_f32(),
                "checks": qa.checks.iter()
                    .map(|(name, ok)| serde_json::json!({"check": name, "pass": ok}))
                    .collect::<Vec<_>>(),
            });
            let written = std::fs::write(
                qa.directory.join("result.json"),
                serde_json::to_string_pretty(&result).unwrap_or_default(),
            );
            if pass && written.is_ok() {
                info!("BOOT_QA passed: {result}");
                exit.write(AppExit::Success);
            } else {
                error!("BOOT_QA failed: {result}");
                exit.write(AppExit::error());
            }
            qa.pending = true;
            return;
        }
    };
    if std::fs::create_dir_all(&qa.directory).is_err() {
        exit.write(AppExit::error());
        return;
    }
    qa.pending = true;
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(qa.directory.join(shot)))
        .observe(|_: On<ScreenshotCaptured>, mut qa: ResMut<BootQa>| {
            qa.pending = false;
            qa.stage += 1;
            qa.since = Instant::now();
        });
}
