//! Explicit native renderer QA for the supporter panel: previews only, without
//! changing any account grant. `OMOBA_SUPPORTER_QA_DIR` (with
//! `OMOBA_QA_SUPPORTER=1`, which opens the panel) writes the screenshots;
//! `OMOBA_SUPPORTER_QA_MOTION=1` captures the moving Hero sequence instead.
//! Every palette uses one viewport (`OMOBA_QA_WIDTH/HEIGHT`, default 1280x720).
use crate::{
    i18n::Locale,
    supporter::{HERO_AURA_ELEMENT_BUDGET, SupporterUiState},
};
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
                    pixels: UVec2::new(
                        dimension("OMOBA_QA_WIDTH", 1280, 320, 3840),
                        dimension("OMOBA_QA_HEIGHT", 720, 180, 2160),
                    ),
                    last_capture: 0.,
                    captures: Vec::new(),
                })
                .add_systems(
                    PostUpdate,
                    capture_preview.after(bevy::ui::UiSystems::Layout),
                );
        }
    }
}

fn dimension(key: &str, fallback: u32, min: u32, max: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|v| (min..=max).contains(v))
        .unwrap_or(fallback)
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
    pixels: UVec2,
    last_capture: f64,
    captures: Vec<serde_json::Value>,
}
fn capture_preview(
    mut commands: Commands,
    mut qa: ResMut<SupporterQa>,
    mut state: ResMut<SupporterUiState>,
    mut windows: Query<&mut Window, With<bevy::window::PrimaryWindow>>,
    mut exit: MessageWriter<bevy::app::AppExit>,
    time: Res<Time>,
    locale: Res<Locale>,
) {
    use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk};
    if qa.started.elapsed() > std::time::Duration::from_secs(120) {
        exit.write(bevy::app::AppExit::error());
        return;
    }
    if qa.stage >= if qa.motion { 24 } else { AuraStyle::ALL.len() } {
        let manifest = serde_json::json!({
            "renderer": "native Bevy",
            "scene": "isolated cosmetic preview; same spawn_aura renderer as world actors",
            "world_session_verified": false,
            "account_entitlement_created": false,
            "physical_device_verified": false,
            "locale": locale.id().code(),
            "viewport": [qa.pixels.x, qa.pixels.y],
            "mesh_elements_per_aura": HERO_AURA_ELEMENT_BUDGET,
            "motion": qa.motion,
            "captures": qa.captures,
        });
        let written = serde_json::to_vec_pretty(&manifest)
            .ok()
            .is_some_and(|data| {
                std::fs::write(qa.directory.join("hero-preview-manifest.json"), data).is_ok()
            });
        exit.write(if written {
            bevy::app::AppExit::Success
        } else {
            bevy::app::AppExit::error()
        });
        return;
    }
    // Every style, base and tier (LR-67): `hero-<id>-preview.png`.
    let (name, style) = if qa.motion {
        (format!("hero-motion-{:03}.png", qa.stage), AuraStyle::Solar)
    } else {
        let style = AuraStyle::ALL[qa.stage];
        (format!("hero-{}-preview.png", style.id()), style)
    };
    state.open = true;
    state.selected = style;
    if let Ok(mut window) = windows.single_mut() {
        window.resolution.set_scale_factor_override(Some(1.0));
        if window.physical_width() != qa.pixels.x || window.physical_height() != qa.pixels.y {
            window
                .resolution
                .set_physical_resolution(qa.pixels.x, qa.pixels.y);
        }
    }
    if qa.pending {
        return;
    }
    qa.frames += 1;
    if if qa.motion && qa.stage > 0 {
        time.elapsed_secs_f64() - qa.last_capture < 0.12
    } else {
        qa.frames < 60
    } {
        return;
    }
    if std::fs::create_dir_all(&qa.directory).is_err() {
        exit.write(bevy::app::AppExit::error());
        return;
    }
    let path = qa.directory.join(&name);
    qa.last_capture = time.elapsed_secs_f64();
    qa.captures.push(serde_json::json!({
        "file": name, "palette": style.id(), "animation_seconds": time.elapsed_secs_f64(),
    }));
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
