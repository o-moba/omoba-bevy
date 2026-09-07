//! Explicit 720p UI readbacks. Entry/help/gameplay use real admission and
//! production button handlers. Only the final result snapshot is synthetic;
//! it is labeled in the image, filename and report, never full-match evidence.
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use bevy::{
    app::AppExit,
    asset::RecursiveDependencyLoadState,
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    scene::{SceneInstance, SceneSpawner},
    window::PrimaryWindow,
};

use crate::{
    help_overlay::HelpOverlayVisible,
    net::{ClientSession, GameState, GameStateSnapshot},
    team::{Team, TeamSelectRoot},
    verdant3d::VerdantEnvironment,
};

const FILES: [&str; 4] = [
    "01-entry-720p.png",
    "02-help-720p.png",
    "03-gameplay-720p.png",
    "04-result-fixture-720p.png",
];
const SETTLE_FRAMES: u32 = 45;

pub(crate) struct BetaUiQaPlugin;
impl Plugin for BetaUiQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(directory) = std::env::var_os("OMOBA_VISUAL_QA_DIR")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
        else {
            return;
        };
        let seconds = std::env::var("OMOBA_VISUAL_QA_TIMEOUT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(240)
            .clamp(30, 600);
        app.insert_resource(BetaUiQa {
            directory,
            started: Instant::now(),
            timeout: Duration::from_secs(seconds),
            stage: 0,
            frames: 0,
            in_flight: false,
            readbacks: Vec::new(),
            captures: Vec::new(),
            fixture_label: false,
        })
        .add_systems(
            PreUpdate,
            prepare_controls.after(bevy::ui::UiSystems::Focus),
        )
        .add_systems(
            Update,
            prepare_result_fixture
                .after(crate::net::ClientNetPipeline::ApplySnapshot)
                .before(crate::game_state::GameStateUiSet)
                .before(crate::input_context::InputContextSet::Modal),
        )
        .add_systems(PostUpdate, capture.after(bevy::ui::UiSystems::Layout));
    }
}

#[derive(Resource)]
struct BetaUiQa {
    directory: PathBuf,
    started: Instant,
    timeout: Duration,
    stage: usize,
    frames: u32,
    in_flight: bool,
    readbacks: Vec<usize>,
    captures: Vec<serde_json::Value>,
    fixture_label: bool,
}
#[derive(Component)]
struct BetaUiShot(usize);

fn prepare_controls(
    qa: Res<BetaUiQa>,
    session: Res<ClientSession>,
    help: Res<HelpOverlayVisible>,
    mut buttons: Query<(&Name, &mut Interaction), With<Button>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    if let Ok(mut window) = windows.single_mut() {
        window.resolution.set_scale_factor_override(Some(1.0));
        if window.physical_width() != 1280 || window.physical_height() != 720 {
            window.resolution.set_physical_resolution(1280, 720);
        }
    }
    for (name, mut interaction) in &mut buttons {
        let press = (qa.stage == 1
            && session.is_connected()
            && !session.join_confirmed()
            && name.as_str() == "TeamGreenButton")
            || (qa.stage == 2 && help.0 && name.as_str() == "HelpDismissButton");
        if press {
            *interaction = Interaction::Pressed;
        }
    }
}

fn prepare_result_fixture(
    mut commands: Commands,
    mut qa: ResMut<BetaUiQa>,
    mut game: ResMut<GameStateSnapshot>,
) {
    if qa.stage != 3 {
        return;
    }
    // Applied after each real network snapshot, exclusively in this opt-in
    // renderer scenario. The authoritative server and actors remain unchanged.
    game.state = GameState::Victory {
        winner: Team::Green,
    };
    game.rematch_in_secs = Some(10);
    if !qa.fixture_label {
        qa.fixture_label = true;
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(16.0),
                top: Val::Px(16.0),
                padding: UiRect::all(Val::Px(8.0)),
                ..default()
            },
            Text::new("QA FIXTURE: result layout only"),
            TextFont {
                font_size: 16.0,
                ..default()
            },
            TextColor(Color::srgb(1.0, 0.8, 0.3)),
            BackgroundColor(Color::srgb(0.05, 0.05, 0.05)),
            ZIndex(200),
        ));
    }
}

#[derive(bevy::ecs::system::SystemParam)]
struct UiScene<'w, 's> {
    windows: Query<'w, 's, Entity, With<PrimaryWindow>>,
    environment: Query<'w, 's, Entity, With<VerdantEnvironment>>,
    scenes: Query<'w, 's, (&'static SceneRoot, Option<&'static SceneInstance>)>,
    join: Query<'w, 's, Entity, With<TeamSelectRoot>>,
    nodes: Query<
        'w,
        's,
        (
            &'static Name,
            &'static ComputedNode,
            &'static UiGlobalTransform,
            Option<&'static InheritedVisibility>,
        ),
    >,
}

fn capture(
    mut commands: Commands,
    mut qa: ResMut<BetaUiQa>,
    session: Res<ClientSession>,
    game: Res<GameStateSnapshot>,
    help: Res<HelpOverlayVisible>,
    assets: Res<AssetServer>,
    spawner: Res<SceneSpawner>,
    scene: UiScene,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.stage >= FILES.len() {
        return;
    }
    if qa.started.elapsed() >= qa.timeout {
        error!(
            "BETA_UI_QA failed: readiness/readback timeout stage={}",
            qa.stage
        );
        exit.write(AppExit::error());
        return;
    }
    if qa.in_flight {
        if qa.readbacks.contains(&qa.stage)
            && qa
                .directory
                .join(FILES[qa.stage])
                .metadata()
                .is_ok_and(|p| p.len() > 32)
        {
            qa.stage += 1;
            qa.frames = 0;
            qa.in_flight = false;
            if qa.stage == FILES.len() {
                let summary = serde_json::json!({"version":env!("CARGO_PKG_VERSION"), "scenario":"beta-ui", "pixels":[1280,720],
                    "method":"Bevy Screenshot::primary_window + save_to_disk", "captures":qa.captures,
                    "elapsed_seconds":qa.started.elapsed().as_secs_f64(), "manual_interaction_verified":false,
                    "button_handler_interactions":"scripted Interaction::Pressed on production Join and Help buttons",
                    "full_match_proof":false, "result_snapshot":"synthetic presentation fixture only"});
                let saved = std::fs::write(
                    qa.directory.join("qa-summary.json"),
                    serde_json::to_vec_pretty(&summary).unwrap(),
                );
                info!("BETA_UI_QA completed summary={summary}");
                // Match the real Exit button: close the native window before
                // asking the runner to leave, releasing its render surface.
                if let Ok(window) = scene.windows.single() {
                    commands.entity(window).despawn();
                }
                exit.write(if saved.is_ok() {
                    AppExit::Success
                } else {
                    AppExit::error()
                });
            }
        }
        return;
    }
    let scenes_ready = !scene.environment.is_empty()
        && !scene.scenes.is_empty()
        && scene.scenes.iter().all(|(root, instance)| {
            instance.is_some_and(|instance| spawner.instance_is_ready(**instance))
                && matches!(
                    assets.recursive_dependency_load_state(root.0.id()),
                    RecursiveDependencyLoadState::Loaded
                )
        });
    let ready = scenes_ready
        && match qa.stage {
            0 => session.is_connected() && !session.join_confirmed() && !scene.join.is_empty(),
            1 => {
                session.join_confirmed()
                    && matches!(game.state, GameState::Running)
                    && help.0
                    && scene.join.is_empty()
            }
            2 => {
                session.join_confirmed()
                    && matches!(game.state, GameState::Running)
                    && !help.0
                    && scene.join.is_empty()
            }
            3 => session.join_confirmed() && matches!(game.state, GameState::Victory { .. }),
            _ => false,
        };
    qa.frames = if ready { qa.frames + 1 } else { 0 };
    if qa.frames < SETTLE_FRAMES {
        return;
    }
    if std::fs::create_dir_all(&qa.directory).is_err() {
        error!("BETA_UI_QA failed: cannot create output directory");
        exit.write(AppExit::error());
        return;
    }
    let primary_nodes: Vec<_> = scene.nodes.iter().filter(|(name, _, _, _)| matches!(name.as_str(),
        "TeamGreenButton" | "TeamBlueButton" | "AvatarGrid" | "HelpDismissButton" | "HelpOverlayRoot" | "GameStateLabel" | "ConnectionStatusPanel" | "MinimapRoot" | "MatchHudColumn" | "SkillBarRoot" | "SkillSlot-Q" | "SkillSlot-R"))
        .map(|(name, node, transform, visible)| {
            let center = transform.translation;
            let size = node.size();
            serde_json::json!({"name":name.as_str(), "center":[center.x,center.y], "size":[size.x,size.y],
                "visible":visible.is_none_or(|visibility| visibility.get()),
                "fits_viewport": center.x-size.x/2.0 >= -1.0 && center.y-size.y/2.0 >= -1.0
                    && center.x+size.x/2.0 <= 1281.0 && center.y+size.y/2.0 <= 721.0})
        }).collect();
    let stage = qa.stage;
    let required: &[&str] = match stage {
        0 => &["TeamGreenButton", "TeamBlueButton", "AvatarGrid"],
        1 => &["HelpDismissButton", "HelpOverlayRoot"],
        2 => &[
            "MinimapRoot",
            "MatchHudColumn",
            "SkillBarRoot",
            "SkillSlot-Q",
            "SkillSlot-R",
        ],
        3 => &["GameStateLabel"],
        _ => &[],
    };
    let controls_fit = required.iter().all(|name| {
        primary_nodes.iter().any(|node| {
            node["name"] == *name
                && node["visible"] == true
                && node["fits_viewport"] == true
                && node["size"][0].as_f64().is_some_and(|size| size > 0.0)
                && node["size"][1].as_f64().is_some_and(|size| size > 0.0)
        })
    });
    if !controls_fit {
        error!(
            "BETA_UI_QA failed: primary controls are missing, hidden or outside 720p: {primary_nodes:?}"
        );
        exit.write(AppExit::error());
        return;
    }
    let record = serde_json::json!({"file":FILES[stage], "stage":stage, "pixels":[1280,720],
        "admitted":session.join_confirmed(), "snapshot_tick":game.meta.snapshot_tick,
        "synthetic_result":stage == 3, "primary_controls_fit":controls_fit, "primary_nodes":primary_nodes});
    info!("BETA_UI_QA capture_request={record}");
    qa.captures.push(record);
    qa.in_flight = true;
    commands
        .spawn((Screenshot::primary_window(), BetaUiShot(stage)))
        .observe(save_to_disk(qa.directory.join(FILES[stage])))
        .observe(record_readback);
}

fn record_readback(
    captured: On<ScreenshotCaptured>,
    shots: Query<&BetaUiShot>,
    mut qa: ResMut<BetaUiQa>,
) {
    if let Ok(shot) = shots.get(captured.entity) {
        if captured.image.width() != 1280 || captured.image.height() != 720 {
            error!("BETA_UI_QA failed: readback dimensions must be 1280x720");
        }
        qa.readbacks.push(shot.0);
        info!(
            "BETA_UI_QA readback_complete stage={} dimensions={}x{}",
            shot.0,
            captured.image.width(),
            captured.image.height()
        );
    }
}
