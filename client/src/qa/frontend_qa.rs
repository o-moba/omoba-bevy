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

mod avatar;

/// Frames each screen is given to lay out. The collection also waits for a
/// glTF to load, so it gets its own budget.
const SETTLE_FRAMES: u32 = 32;
const COLLECTION_SETTLE_FRAMES: u32 = 240;

const VIEWS: [(&str, AppScreen, &str); 13] = [
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
    ("07-post-match.png", AppScreen::PostMatch, "PostMatchScreen"),
    ("08-menu.png", AppScreen::Home, "PauseMenuPanel"),
    ("09-settings-sound.png", AppScreen::Home, "PauseMenuPanel"),
    (
        "10-settings-graphics.png",
        AppScreen::Home,
        "PauseMenuPanel",
    ),
    ("11-server.png", AppScreen::Home, "ServerEntryPanel"),
    ("12-home-help.png", AppScreen::Home, "HelpOverlayRoot"),
    ("13-home-help-closed.png", AppScreen::Home, "HomeScreen"),
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
        if std::env::var("OMOBA_AVATAR_QA").is_ok_and(|value| value == "1") {
            app.add_plugins(avatar::AvatarQaPlugin { directory });
            return;
        }
        if std::env::var("OMOBA_FRONTEND_QA_FLOW").is_ok_and(|value| value == "1") {
            // Live-flow capture owns the screens instead of this harness.
            app.add_plugins(super::frontend_flow_qa::FrontendFlowQaPlugin);
            return;
        }
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
        .add_systems(
            PreUpdate,
            prepare_help_buttons.after(bevy::ui::UiSystems::Focus),
        )
        .add_systems(
            Update,
            drive
                .after(crate::pause_menu::PauseMenuSet::Close)
                .before(crate::pause_menu::PauseMenuSet::Visuals),
        )
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

fn prepare_help_buttons(
    qa: Res<FrontendQa>,
    help: Res<crate::help_overlay::HelpOverlayVisible>,
    mut buttons: crate::qa::TestIdPresses,
) {
    if qa.finished || qa.applied_stage != Some(qa.stage) {
        return;
    }
    let wanted = match qa.stage {
        11 if !help.0 => "PhoneHelpButton",
        12 if help.0 => "HelpDismissButton",
        _ => return,
    };
    buttons.press(wanted);
}

fn watermark(mut commands: Commands) {
    // Showcase captures record the offline-shell provenance in their manifest
    // instead of burning it into the published frame.
    if std::env::var("OMOBA_QA_CLEAN_FRAME").is_ok_and(|value| value == "1") {
        return;
    }
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
    mut pause: ResMut<crate::pause_menu::PauseMenuState>,
    server: Option<ResMut<crate::mobile_ui::ServerEntry>>,
    mut scrolls: Query<(crate::qa::QaName, &ComputedNode, &mut ScrollPosition)>,
    mut career: ResMut<crate::career::CareerClient>,
) {
    if qa.finished || qa.stage >= VIEWS.len() {
        return;
    }
    if std::env::var("OMOBA_PUBLIC_MVP_QA").as_deref() == Ok("1") {
        career.view.match_service = Some(shared::match_service::MatchServiceView::Idle);
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
    // Only this explicitly enabled screenshot harness drives the modal fixture.
    // The production modal visibility, scrolling and input gates still render it.
    pause.open = (7..=9).contains(&qa.stage);
    pause.in_settings = (8..=9).contains(&qa.stage);
    if let Some(mut server) = server {
        server.open = qa.stage == 10;
    }
    for (name, node, mut scroll) in &mut scrolls {
        if name.as_str() == "PauseMenuSettingsSection" {
            scroll.y = if qa.stage == 9 {
                ((node.content_size().y - node.size().y) * node.inverse_scale_factor()).max(0.0)
            } else {
                0.0
            };
        }
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

fn observe(
    mut commands: Commands,
    mut qa: ResMut<FrontendQa>,
    screen: Res<State<AppScreen>>,
    preview: Res<AvatarPreview>,
    ui_scale: Res<UiScale>,
    mobile: Res<crate::mobile_controls::MobileControls>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    nodes: Query<(
        crate::qa::QaName,
        &ComputedNode,
        &UiGlobalTransform,
        Option<&InheritedVisibility>,
        Option<&ZIndex>,
    )>,
    buttons: Query<crate::qa::QaName, With<Button>>,
    preview_cameras: Query<&Camera, With<crate::frontend::preview::PreviewCamera>>,
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
        // The hosted-server form is a phone-only surface. Desktop settings
        // retain their existing server-address hint rather than this keypad.
        if qa.stage == VIEWS.len() || (qa.stage == 10 && !mobile.enabled) {
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
    let mut root_bounds = None;
    let mut nodes_json = Vec::new();
    for (name, node, transform, inherited, z_index) in &nodes {
        let size = node.size() * transform.to_scale_angle_translation().0.abs();
        let min = transform.translation - size * 0.5;
        let visible = inherited.is_none_or(|visibility| visibility.get());
        if name.as_str() == root_name {
            root_fit = visible && fits(min, size, viewport);
            root_bounds = Some((min, size));
        }
        if name.as_str() == root_name
            || name.as_str().starts_with("Avatar")
            || name.as_str().starts_with("Home")
            || name.as_str().starts_with("Card")
            || name.as_str().starts_with("Collection")
            || name.as_str().starts_with("Searching")
            || name.as_str().starts_with("PostMatch")
            || name.as_str().starts_with("Server")
            || name.as_str().starts_with("Help")
            || name.as_str() == "PhoneMenuBar"
            || name.as_str().starts_with("HeroSelect")
            || matches!(name.as_str(), "FindMatchButton")
            || name.as_str().starts_with("PauseMenu")
            || matches!(name.as_str(), "SettingsButton" | "BackButton")
        {
            nodes_json.push(serde_json::json!({
                "name": name.as_str(),
                "min": min.to_array(),
                "size": size.to_array(),
                "visible": visible,
                "z_index": z_index.map_or(0, |z| z.0),
            }));
        }
    }
    let required: &[&str] = match qa.stage {
        0 if std::env::var("OMOBA_PUBLIC_MVP_QA").as_deref() == Ok("1") => &[
            "HomePlay",
            "HomeHumansOnly",
            "HomeBotPractice",
            "HomeCustomizeCard",
            "HomeAccount",
            "HomeCollection",
            "HomeHistory",
            "HomeFriends",
        ],
        0 => &[
            "HomePlay",
            "HomeCustomizeCard",
            "HomeAccount",
            "HomeCollection",
            "HomeHistory",
            "HomeFriends",
        ],
        1 => &["CardBack", "CardOpenCollection"],
        2 => &[
            "CollectionBack",
            "AvatarAutoSpin",
            "AvatarShowcase",
            "AvatarEquip",
        ],
        3 => &["HeroSelectBack", "FindMatchButton"],
        4 => &["SearchingCancel"],
        6 => &["PostMatchPlayAgain", "PostMatchBackToMenu"],
        7 => &[
            "PauseMenuResumeButton",
            "SettingsButton",
            "PauseMenuHelpButton",
            "PauseMenuExitButton",
        ],
        8 => &[
            "PauseMenuAudioMasterControls",
            "PauseMenuAudioMusicControls",
        ],
        9 => &[
            "PauseMenuScaleControls",
            "PauseMenuResetGraphicsButton",
            "BackButton",
        ],
        10 => &[
            "ServerConnectButton",
            "ServerCloseButton",
            "ServerKeyboardButton",
        ],
        11 => &["HelpDismissButton"],
        12 => &[
            "HomePlay",
            "HomeCustomizeCard",
            "HomeAccount",
            "HomeCollection",
            "HomeHistory",
            "HomeFriends",
        ],
        _ => &[],
    };
    let controls_fit = required.iter().all(|required| {
        nodes_json.iter().any(|record| {
            record["name"] == *required
                && record["visible"] == true
                && fits(
                    Vec2::new(
                        record["min"][0].as_f64().unwrap() as f32,
                        record["min"][1].as_f64().unwrap() as f32,
                    ),
                    Vec2::new(
                        record["size"][0].as_f64().unwrap() as f32,
                        record["size"][1].as_f64().unwrap() as f32,
                    ),
                    viewport,
                )
        })
    });
    // A control can fit the viewport yet be clipped by its parent panel.
    let menu_controls_fit_panel = qa.stage != 7
        || root_bounds.is_some_and(|(panel_min, panel_size)| {
            required.iter().all(|required| {
                nodes_json.iter().any(|record| {
                    let min = Vec2::new(
                        record["min"][0].as_f64().unwrap() as f32,
                        record["min"][1].as_f64().unwrap() as f32,
                    );
                    let size = Vec2::new(
                        record["size"][0].as_f64().unwrap() as f32,
                        record["size"][1].as_f64().unwrap() as f32,
                    );
                    record["name"] == *required
                        && record["visible"] == true
                        && fits(min - panel_min, size, panel_size)
                        && (!mobile.enabled || size.cmpge(Vec2::splat(44.0)).all())
                })
            })
        });
    let interactive: Vec<String> = buttons
        .iter()
        .map(|name| name.as_str().to_owned())
        .filter(|name| !name.is_empty())
        .collect();
    let help_transition_scale_correct = match qa.stage {
        11 => (ui_scale.0 - 1.0).abs() < 0.001,
        12 => (ui_scale.0 - crate::frontend::menu_scale(window.height())).abs() < 0.001,
        _ => true,
    };
    let help_above_shell = qa.stage != 11
        || nodes_json
            .iter()
            .find(|node| node["name"] == "HelpOverlayRoot")
            .and_then(|node| node["z_index"].as_i64())
            .is_some_and(|help_z| {
                nodes_json
                    .iter()
                    .filter(|node| node["name"] == "HomeScreen" || node["name"] == "PhoneMenuBar")
                    .all(|node| node["z_index"].as_i64().is_some_and(|z| help_z > z))
            });
    let record = serde_json::json!({
        "file": VIEWS[qa.stage].0,
        "screen": format!("{:?}", VIEWS[qa.stage].1),
        "root": root_name,
        "root_fits_viewport": root_fit,
        "required_controls": required,
        "required_controls_fit": controls_fit,
        "menu_controls_fit_panel": menu_controls_fit_panel,
        "help_transition_scale_correct": help_transition_scale_correct,
        "help_above_shell": help_above_shell,
        "production_help_button_transition": qa.stage >= 11,
        "ui_profile": if mobile.enabled { "Mobile" } else { "Desktop" },
        "ui_scale": ui_scale.0,
        "pixels": viewport.to_array(),
        "buttons": interactive,
        "modal_fixture": qa.stage >= 7,
        "preview_avatar": preview.slug,
        "preview_status": format!("{:?}", preview.status),
        "preview_camera_active": preview_cameras.iter().any(|camera| camera.is_active),
        "preview_clips": preview.clips.iter().map(|clip| clip.name.clone()).collect::<Vec<_>>(),
        "nodes": nodes_json,
    });
    qa.captures.push(record);
    if !root_fit
        || !controls_fit
        || !menu_controls_fit_panel
        || !help_transition_scale_correct
        || !help_above_shell
    {
        fail(
            &mut qa,
            "screen root or essential controls are missing or leave their viewport/panel",
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
