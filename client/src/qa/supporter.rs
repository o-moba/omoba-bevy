//! Explicit native renderer QA for the supporter panel: previews only, without
//! changing any account grant. `OMOBA_SUPPORTER_QA_DIR` (with
//! `OMOBA_QA_SUPPORTER=1`, which opens the panel) writes the screenshots;
//! `OMOBA_SUPPORTER_QA_MOTION=1` captures the orbit sequence instead.
use crate::supporter::SupporterUiState;
use bevy::prelude::*;
use shared::supporter::AuraStyle;

pub(crate) struct SupporterQaPlugin;
impl Plugin for SupporterQaPlugin {
    fn build(&self, app: &mut App) {
        if let Some(directory) =
            std::env::var_os("OMOBA_SUPPORTER_QA_DIR").filter(|p| !p.is_empty())
        {
            app.insert_resource(bevy::winit::WinitSettings::continuous())
                .insert_resource(SupporterQa {
                    directory: directory.into(),
                    started: std::time::Instant::now(),
                    stage: 0,
                    frames: 0,
                    pending: false,
                    motion: std::env::var("OMOBA_SUPPORTER_QA_MOTION").is_ok_and(|v| v == "1"),
                })
                .add_systems(
                    PostUpdate,
                    capture_preview.after(bevy::ui::UiSystems::Layout),
                );
        }
    }
}

/// Explicit native renderer QA: previews only, without changing any account grant.
#[derive(Resource)]
struct SupporterQa {
    directory: std::path::PathBuf,
    started: std::time::Instant,
    stage: usize,
    frames: u32,
    pending: bool,
    motion: bool,
}
fn capture_preview(
    mut commands: Commands,
    mut qa: ResMut<SupporterQa>,
    mut state: ResMut<SupporterUiState>,
    mut windows: Query<&mut Window, With<bevy::window::PrimaryWindow>>,
    mut exit: MessageWriter<bevy::app::AppExit>,
) {
    use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk};
    if qa.started.elapsed() > std::time::Duration::from_secs(120) {
        exit.write(bevy::app::AppExit::error());
        return;
    }
    if qa.stage >= if qa.motion { 24 } else { 3 } {
        exit.write(bevy::app::AppExit::Success);
        return;
    }
    let captures = [
        ("solar-desktop.png", AuraStyle::Solar, 960, 600),
        ("lunar-phone.png", AuraStyle::Lunar, 844, 390),
        ("verdant-phone.png", AuraStyle::Verdant, 844, 390),
    ];
    let (name, style, width, height) = if qa.motion {
        (
            format!("orbit-{:03}.png", qa.stage),
            AuraStyle::Solar,
            960,
            600,
        )
    } else {
        let (name, style, width, height) = captures[qa.stage];
        (name.to_string(), style, width, height)
    };
    state.open = true;
    state.selected = style;
    if let Ok(mut window) = windows.single_mut() {
        window.resolution.set_scale_factor_override(Some(1.0));
        if window.physical_width() != width || window.physical_height() != height {
            window.resolution.set_physical_resolution(width, height);
        }
    }
    if qa.pending {
        return;
    }
    qa.frames += 1;
    if qa.frames < if qa.motion && qa.stage > 0 { 6 } else { 60 } {
        return;
    }
    if std::fs::create_dir_all(&qa.directory).is_err() {
        exit.write(bevy::app::AppExit::error());
        return;
    }
    let path = qa.directory.join(name);
    qa.pending = true;
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path))
        .observe(|_: On<ScreenshotCaptured>, mut qa: ResMut<SupporterQa>| {
            qa.pending = false;
            qa.frames = 0;
            qa.stage += 1;
        });
}
