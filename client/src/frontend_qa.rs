//! Opt-in screenshot capture of the front-end shell.
//!
//! This proves the screens actually render and stay inside a 1280x720 window
//! (the smallest resolution the playtest audit flagged for clipping). It drives
//! the real screens, not fixtures: only the career view behind the profile card
//! is stubbed, exactly like the career UI harness does.

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use bevy::{
    app::AppExit,
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    ui::FocusPolicy,
    window::PrimaryWindow,
};

use crate::frontend::{AppScreen, ScreenDriverPaused, preview::AvatarPreview};

/// Frames each screen is given to lay out. The collection also waits for a
/// glTF to load, so it gets its own budget.
const SETTLE_FRAMES: u32 = 32;
const COLLECTION_SETTLE_FRAMES: u32 = 240;

const VIEWS: [(&str, AppScreen, &str); 6] = [
    ("01-home.png", AppScreen::Home, "HomeScreen"),
    ("02-profile-card.png", AppScreen::Card, "CardScreen"),
    (
        "03-collection.png",
        AppScreen::Collection,
        "CollectionScreen",
    ),
    (
        "04-hero-select.png",
        AppScreen::HeroSelect,
        "TeamSelectOverlay",
    ),
    ("05-searching.png", AppScreen::Searching, "SearchingScreen"),
    ("06-loading.png", AppScreen::Loading, "LoadingScreen"),
];

pub(crate) struct FrontendQaPlugin;

impl Plugin for FrontendQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(directory) = std::env::var_os("OMOBA_FRONTEND_QA_OUTPUT")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
        else {
            return;
        };
        let dimension = |name: &str, fallback| {
            std::env::var(name)
                .ok()
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(fallback)
                .clamp(200, 3840)
        };
        app.insert_resource(FrontendQa {
            directory,
            started: Instant::now(),
            pixels: UVec2::new(
                dimension("OMOBA_QA_WIDTH", 1280),
                dimension("OMOBA_QA_HEIGHT", 720),
            ),
            stage: 0,
            applied_stage: None,
            settled: 0,
            in_flight: false,
            readbacks: Vec::new(),
            captures: Vec::new(),
            finished: false,
        })
        .insert_resource(bevy::winit::WinitSettings::continuous())
        // The capture drives the screens itself; the session must not pull a
        // screen away (the search screen has no real queue entry behind it).
        .insert_resource(ScreenDriverPaused(true))
        .add_systems(Startup, watermark)
        .add_systems(Update, drive)
        .add_systems(
            PostUpdate,
            observe
                .after(bevy::ui::UiSystems::Layout)
                .after(bevy::transform::TransformSystems::Propagate),
        );
    }
}

#[derive(Resource)]
struct FrontendQa {
    directory: PathBuf,
    started: Instant,
    pixels: UVec2,
    stage: usize,
    applied_stage: Option<usize>,
    settled: u32,
    in_flight: bool,
    readbacks: Vec<usize>,
    captures: Vec<serde_json::Value>,
    finished: bool,
}

impl FrontendQa {
    fn settle_target(&self) -> u32 {
        if VIEWS[self.stage].1 == AppScreen::Collection {
            COLLECTION_SETTLE_FRAMES
        } else {
            SETTLE_FRAMES
        }
    }
}

#[derive(Component)]
struct Shot(usize);

fn watermark(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(2.0),
            right: Val::Px(8.0),
            ..default()
        },
        Text::new("QA CAPTURE · offline shell, no live match"),
        TextFont {
            font_size: 8.0,
            ..default()
        },
        TextColor(Color::WHITE),
        BackgroundColor(Color::BLACK),
        FocusPolicy::Pass,
        ZIndex(300),
        Name::new("FrontendQaWatermark"),
    ));
}

fn drive(
    mut qa: ResMut<FrontendQa>,
    screen: Res<State<AppScreen>>,
    mut next: ResMut<NextState<AppScreen>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    if qa.finished || qa.stage >= VIEWS.len() {
        return;
    }
    if let Ok(mut window) = windows.single_mut()
        && (window.resolution.physical_width() != qa.pixels.x
            || window.resolution.physical_height() != qa.pixels.y)
    {
        window.resolution.set_scale_factor_override(Some(1.0));
        window
            .resolution
            .set_physical_resolution(qa.pixels.x, qa.pixels.y);
    }
    let wanted = VIEWS[qa.stage].1;
    if qa.applied_stage != Some(qa.stage) {
        if *screen.get() != wanted {
            next.set(wanted);
            return;
        }
        qa.applied_stage = Some(qa.stage);
        qa.settled = 0;
    }
}

fn fits(min: Vec2, size: Vec2, viewport: Vec2) -> bool {
    size.x > 0.0
        && size.y > 0.0
        && min.cmpge(-Vec2::ONE).all()
        && (min + size).cmple(viewport + Vec2::ONE).all()
}

fn fail(qa: &mut FrontendQa, reason: &str, exit: &mut MessageWriter<AppExit>) {
    qa.finished = true;
    let _ = std::fs::create_dir_all(&qa.directory);
    let value = serde_json::json!({
        "status": "failed",
        "reason": reason,
        "stage": qa.stage,
        "captures": qa.captures,
    });
    let _ = std::fs::write(
        qa.directory.join("qa-failure.json"),
        serde_json::to_vec_pretty(&value).unwrap(),
    );
    error!("FRONTEND_QA failed: {reason}");
    exit.write(AppExit::error());
}

#[allow(clippy::too_many_arguments)]
fn observe(
    mut commands: Commands,
    mut qa: ResMut<FrontendQa>,
    screen: Res<State<AppScreen>>,
    preview: Res<AvatarPreview>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    nodes: Query<(
        &Name,
        &ComputedNode,
        &UiGlobalTransform,
        Option<&InheritedVisibility>,
    )>,
    buttons: Query<&Name, With<Button>>,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.finished {
        return;
    }
    if qa.started.elapsed() > Duration::from_secs(180) {
        fail(&mut qa, "180-second capture deadline", &mut exit);
        return;
    }
    if qa.in_flight {
        if !qa.readbacks.contains(&qa.stage)
            || !qa
                .directory
                .join(VIEWS[qa.stage].0)
                .metadata()
                .is_ok_and(|file| file.len() > 32)
        {
            return;
        }
        qa.stage += 1;
        qa.in_flight = false;
        qa.applied_stage = None;
        if qa.stage == VIEWS.len() {
            let summary = serde_json::json!({
                "status": "passed",
                "scenario": "frontend-shell",
                "version": env!("CARGO_PKG_VERSION"),
                "method": "real Bevy primary_window ScreenshotCaptured + save_to_disk",
                "live_match_evidence": false,
                "manual_input_verified": false,
                "captures": qa.captures,
            });
            let saved = std::fs::write(
                qa.directory.join("qa-summary.json"),
                serde_json::to_vec_pretty(&summary).unwrap(),
            );
            qa.finished = true;
            if let Ok((entity, _)) = windows.single() {
                commands.entity(entity).despawn();
            }
            exit.write(if saved.is_ok() {
                AppExit::Success
            } else {
                AppExit::error()
            });
        }
        return;
    }
    if qa.applied_stage != Some(qa.stage) || *screen.get() != VIEWS[qa.stage].1 {
        return;
    }
    qa.settled += 1;
    if qa.settled < qa.settle_target() {
        return;
    }
    let Ok((_, window)) = windows.single() else {
        return;
    };
    let viewport = Vec2::new(
        window.resolution.physical_width() as f32,
        window.resolution.physical_height() as f32,
    );
    if viewport.as_uvec2() != qa.pixels {
        return;
    }
    let root_name = VIEWS[qa.stage].2;
    let mut root_fit = false;
    let mut nodes_json = Vec::new();
    for (name, node, transform, inherited) in &nodes {
        let size = node.size() * transform.to_scale_angle_translation().0.abs();
        let min = transform.translation - size * 0.5;
        let visible = inherited.is_none_or(|visibility| visibility.get());
        if name.as_str() == root_name {
            root_fit = visible && fits(min, size, viewport);
        }
        if name.as_str() == root_name || name.as_str().starts_with("Avatar") {
            nodes_json.push(serde_json::json!({
                "name": name.as_str(),
                "min": min.to_array(),
                "size": size.to_array(),
                "visible": visible,
            }));
        }
    }
    let interactive: Vec<&str> = buttons.iter().map(Name::as_str).collect();
    let record = serde_json::json!({
        "file": VIEWS[qa.stage].0,
        "screen": format!("{:?}", VIEWS[qa.stage].1),
        "root": root_name,
        "root_fits_viewport": root_fit,
        "pixels": viewport.to_array(),
        "buttons": interactive,
        "preview_avatar": preview.slug,
        "preview_status": format!("{:?}", preview.status),
        "preview_clips": preview.clips.iter().map(|clip| clip.name.clone()).collect::<Vec<_>>(),
        "nodes": nodes_json,
    });
    qa.captures.push(record);
    if !root_fit {
        fail(
            &mut qa,
            "front-end screen root is missing or leaves the viewport",
            &mut exit,
        );
        return;
    }
    if std::fs::create_dir_all(&qa.directory).is_err() {
        fail(&mut qa, "cannot create capture directory", &mut exit);
        return;
    }
    let stage = qa.stage;
    commands
        .spawn((Screenshot::primary_window(), Shot(stage)))
        .observe(save_to_disk(qa.directory.join(VIEWS[stage].0)))
        .observe(readback);
    qa.in_flight = true;
}

fn readback(captured: On<ScreenshotCaptured>, shots: Query<&Shot>, mut qa: ResMut<FrontendQa>) {
    if let Ok(shot) = shots.get(captured.entity)
        && captured.image.width() == qa.pixels.x
        && captured.image.height() == qa.pixels.y
        && !qa.readbacks.contains(&shot.0)
    {
        qa.readbacks.push(shot.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_captured_screen_has_a_distinct_file_and_root() {
        let mut files: Vec<&str> = VIEWS.iter().map(|(file, _, _)| *file).collect();
        files.sort_unstable();
        let before = files.len();
        files.dedup();
        assert_eq!(before, files.len());
        assert!(VIEWS.iter().all(|(_, _, root)| !root.is_empty()));
    }

    #[test]
    fn bounds_reject_clipped_and_empty_roots() {
        assert!(fits(Vec2::ZERO, Vec2::splat(10.0), Vec2::splat(10.0)));
        assert!(!fits(Vec2::ZERO, Vec2::ZERO, Vec2::splat(10.0)));
        assert!(!fits(
            Vec2::splat(5.0),
            Vec2::splat(10.0),
            Vec2::splat(10.0)
        ));
    }
}
