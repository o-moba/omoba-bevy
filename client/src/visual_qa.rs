//! Bounded, explicit native-renderer QA capture. Disabled unless an output
//! directory is supplied. Images are real Bevy window readbacks, not UI input
//! automation, Blender renders, or a substitute for interactive playtesting.

use bevy::ecs::system::SystemParam;
use bevy::{
    app::AppExit,
    asset::RecursiveDependencyLoadState,
    camera::{CameraUpdateSystems, ScalingMode},
    light::{CascadeShadowConfig, CascadeShadowConfigBuilder},
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    scene::{SceneInstance, SceneSpawner},
    window::PrimaryWindow,
};
use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, Instant},
};

use crate::{
    bosses::BossVisual,
    camera::MainCamera,
    combat::CombatStats,
    help_overlay::HelpOverlayVisible,
    maps::MapLayout,
    model_scale::NormalizeModelScale,
    net::{
        ClientSession, GameState, GameStateSnapshot, MinionBrainState, NetworkCommand,
        NetworkMinion, NetworkMinionBrainState, NetworkNeutral, NetworkPlayerId, NetworkStructure,
        NeutralAiState, NeutralAiStateTag, NeutralCampType,
    },
    pause_menu::PauseMenuState,
    player::{MovementTarget, Player},
    sprite::PlayerVisualMode,
    team::{CharacterChoice, Team, TeamSelectRoot, TeamSelection},
    verdant3d::{VerdantEnvironment, VerdantFoliage, VerdantStructureVisual},
};

const SETTLE_FRAMES: u32 = 45;
const MAX_SECONDS: u64 = 240;
const QA_DESTINATION: Vec3 = Vec3::new(2.0, 0.0, 1.0);

pub struct VisualQaPlugin;

impl Plugin for VisualQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(directory) = qa_output(std::env::var_os("OMOBA_VISUAL_QA_DIR")) else {
            return;
        };
        let max_seconds = bounded_timeout(std::env::var("OMOBA_VISUAL_QA_TIMEOUT").ok().as_deref());
        app.insert_resource(QaState::new(directory, max_seconds))
            .add_systems(
                Update,
                prepare_qa.after(crate::input_context::InputContextSet::Actions),
            )
            .add_systems(
                PostUpdate,
                capture_qa
                    .before(bevy::transform::TransformSystems::Propagate)
                    .before(CameraUpdateSystems)
                    .before(bevy::ui::UiSystems::Layout),
            );
    }
}

fn qa_output(raw: Option<std::ffi::OsString>) -> Option<PathBuf> {
    raw.filter(|path| !path.is_empty()).map(PathBuf::from)
}

fn bounded_timeout(raw: Option<&str>) -> u64 {
    raw.and_then(|value| value.parse().ok())
        .unwrap_or(MAX_SECONDS)
        .clamp(30, 600)
}

#[derive(Clone, Copy)]
struct View {
    file: &'static str,
    position: Vec3,
    target: Vec3,
    width: f32,
    pixels: UVec2,
    hud: bool,
    perspective: bool,
}

fn views() -> [View; 4] {
    // Exact art-script cameras mapped once: Blender (x,y,z) -> Bevy (x,z,-y).
    let base = -225.0_f32 / 2.0_f32.sqrt() / 2.0;
    [
        View {
            file: "01-overview.png",
            position: Vec3::new(230.0, 290.0, 300.0),
            target: Vec3::new(0.0, -1.0, 0.0),
            width: 315.0,
            pixels: UVec2::new(1600, 1400),
            hud: false,
            perspective: false,
        },
        View {
            file: "03-river-gameplay.png",
            position: Vec3::new(44.0, 49.0, 55.0),
            target: Vec3::new(2.0, 1.0, 1.0),
            width: 79.0,
            pixels: UVec2::new(1600, 1200),
            hud: true,
            perspective: false,
        },
        View {
            file: "02-sanctuary.png",
            position: Vec3::new(base - 12.0, 44.0, base - 68.0),
            target: Vec3::new(base, 5.0, base),
            width: 64.0,
            pixels: UVec2::new(1400, 1400),
            hud: false,
            perspective: false,
        },
        View {
            file: "04-follow-gameplay.png",
            position: QA_DESTINATION + crate::camera::locked_camera_offset(1.0),
            target: QA_DESTINATION,
            width: 0.0,
            pixels: UVec2::new(1600, 1000),
            hud: true,
            perspective: true,
        },
    ]
}

#[derive(Resource)]
struct QaState {
    directory: PathBuf,
    started: Instant,
    timeout: Duration,
    joined: bool,
    fixtures_spawned: bool,
    view: usize,
    stable_frames: u32,
    in_flight: bool,
    finished_readbacks: Vec<usize>,
    hidden_ui: HashMap<Entity, Display>,
    captures: Vec<serde_json::Value>,
    last_diagnostic: u32,
}

impl QaState {
    fn new(directory: PathBuf, max_seconds: u64) -> Self {
        Self {
            directory,
            started: Instant::now(),
            timeout: Duration::from_secs(max_seconds),
            joined: false,
            fixtures_spawned: false,
            view: 0,
            stable_frames: 0,
            in_flight: false,
            finished_readbacks: Vec::new(),
            hidden_ui: HashMap::new(),
            captures: Vec::new(),
            last_diagnostic: 0,
        }
    }
}

#[derive(Component)]
struct QaActor;
#[derive(Component)]
struct QaShot(usize);

fn prepare_qa(
    mut commands: Commands,
    mut qa: ResMut<QaState>,
    session: Res<ClientSession>,
    mut selection: ResMut<TeamSelection>,
    mut writer: MessageWriter<NetworkCommand>,
    layout: Res<MapLayout>,
    mut help: ResMut<HelpOverlayVisible>,
    mut pause: ResMut<PauseMenuState>,
    players: Query<(Entity, &Transform, &CombatStats, Option<&MovementTarget>), With<Player>>,
    join_ui: Query<Entity, With<TeamSelectRoot>>,
) {
    help.0 = false;
    pause.open = false;
    if !qa.joined && session.is_connected() {
        qa.joined = true;
        selection.team = Some(Team::Green);
        selection.character = CharacterChoice::Cube;
        selection.hero_class = shared::HeroClass::Warrior;
        selection.avatar = Some("agnes".to_owned());
        writer.write(NetworkCommand::Join {
            team: Team::Green,
            character: CharacterChoice::Cube,
            hero_class: shared::HeroClass::Warrior,
            avatar: Some("agnes".to_owned()),
            sprite_character: None,
        });
        for entity in &join_ui {
            commands
                .entity(entity)
                .despawn_related::<Children>()
                .despawn();
        }
        info!("VERDANT_QA autojoin=agnes team=green method=normal_network_command");
    }
    if session.join_confirmed()
        && let Ok((entity, transform, stats, target)) = players.single()
        && stats.is_alive()
        && target.is_none()
        && transform.translation.xz().distance(QA_DESTINATION.xz()) > 3.0
    {
        commands.entity(entity).insert(MovementTarget {
            target: QA_DESTINATION,
        });
        info!("VERDANT_QA movement=production_local_path destination={QA_DESTINATION:?}");
    }
    if session.join_confirmed() && !qa.fixtures_spawned {
        qa.fixtures_spawned = true;
        // Explicit render fixtures exercise the exact production factories.
        // They have no server identity and are counted separately in evidence.
        for (index, (team, x, z)) in [
            (Team::Green, -8.0, -5.0),
            (Team::Green, -10.0, -6.0),
            (Team::Blue, 8.0, 4.0),
            (Team::Blue, 10.0, 5.0),
        ]
        .into_iter()
        .enumerate()
        {
            commands.spawn((
                QaActor,
                NetworkMinion,
                team,
                NetworkMinionBrainState(MinionBrainState::Marching),
                CombatStats::default(),
                NormalizeModelScale::scaled_by(0.6),
                Transform::from_xyz(x, layout.terrain_height_3d(x, z), z),
                Visibility::default(),
                Name::new(format!("QA production minion fixture {index}")),
            ));
        }
        let (x, z) = (12.0, 8.0); // Clear stone approach, outside the river water.
        commands.spawn((
            QaActor,
            NetworkNeutral,
            BossVisual {
                camp_type: NeutralCampType::WendigoBoss,
            },
            NeutralAiStateTag(NeutralAiState::Idle),
            CombatStats::default(),
            Transform::from_xyz(x, layout.terrain_height_3d(x, z), z),
            Visibility::default(),
            Name::new("QA production stone sentinel fixture"),
        ));
        info!(
            "VERDANT_QA fixtures=5 source=production_minion_and_boss_factories authoritative=false bounded=true"
        );
    }
}

#[derive(Default, Debug, Clone, Copy)]
struct Readiness {
    admitted: bool,
    running: bool,
    environments: usize,
    foliage: usize,
    structures: usize,
    styled_structures: usize,
    scenes: usize,
    ready_scenes: usize,
    loaded_scenes: usize,
    local_at_river: bool,
    peers_at_river: bool,
    meshes: usize,
}

fn ready_for_capture(ready: Readiness) -> bool {
    ready.admitted
        && ready.running
        && ready.environments == 1
        && ready.foliage == 1
        && ready.structures == 8
        && ready.styled_structures == 8
        && ready.scenes >= 10
        && ready.ready_scenes == ready.scenes
        && ready.loaded_scenes == ready.scenes
        && ready.meshes > 100
        && ready.local_at_river
        && ready.peers_at_river
}

fn advance_stability(stable: u32, ready: bool) -> u32 {
    if ready {
        stable.saturating_add(1).min(SETTLE_FRAMES)
    } else {
        0
    }
}

#[derive(SystemParam)]
struct CaptureWorld<'w, 's> {
    scenes: Query<'w, 's, (&'static SceneRoot, Option<&'static SceneInstance>)>,
    environment: Query<'w, 's, (), With<VerdantEnvironment>>,
    foliage: Query<'w, 's, (), With<VerdantFoliage>>,
    structures: Query<'w, 's, (), With<NetworkStructure>>,
    styled: Query<'w, 's, (), With<VerdantStructureVisual>>,
    actors: Query<'w, 's, &'static Transform, (With<NetworkPlayerId>, Without<MainCamera>)>,
    fixtures: Query<'w, 's, (), With<QaActor>>,
    meshes: Query<'w, 's, (), With<Mesh3d>>,
    materials: Res<'w, Assets<StandardMaterial>>,
    local: Query<
        'w,
        's,
        (&'static Transform, &'static CombatStats),
        (With<Player>, Without<MainCamera>),
    >,
    camera: Query<
        'w,
        's,
        (
            &'static mut Transform,
            &'static mut GlobalTransform,
            &'static mut Projection,
        ),
        (With<MainCamera>, Without<Player>),
    >,
    shadows: Query<'w, 's, &'static mut CascadeShadowConfig, With<DirectionalLight>>,
    windows: Query<'w, 's, &'static mut Window, With<PrimaryWindow>>,
    ui: Query<'w, 's, (Entity, &'static mut Node, Option<&'static Name>), Without<ChildOf>>,
}

fn capture_qa(
    mut commands: Commands,
    mut qa: ResMut<QaState>,
    mode: Res<PlayerVisualMode>,
    session: Res<ClientSession>,
    game: Res<GameStateSnapshot>,
    asset_server: Res<AssetServer>,
    spawner: Res<SceneSpawner>,
    mut world: CaptureWorld,
    mut exit: MessageWriter<AppExit>,
) {
    if *mode != PlayerVisualMode::Models3d || qa.started.elapsed() >= qa.timeout {
        error!(
            "VERDANT_QA failed: 3D-only capture did not finish within {:?}; captures={}",
            qa.timeout,
            qa.captures.len()
        );
        let _ = std::fs::create_dir_all(&qa.directory);
        let _ = std::fs::write(
            qa.directory.join("qa-failure.json"),
            serde_json::json!({
                "reason":"wrong render mode or readiness/readback timeout", "captures":qa.captures,
                "elapsed_seconds":qa.started.elapsed().as_secs_f64(),
            })
            .to_string(),
        );
        exit.write(AppExit::error());
        return;
    }
    if qa.view >= views().len() {
        return;
    }
    let mut view = views()[qa.view];
    if view.perspective
        && let Ok((local, _)) = world.local.single()
    {
        view.position = local.translation + crate::camera::locked_camera_offset(1.0);
        view.target = Vec3::new(
            local.translation.x,
            crate::player::PLAYER_SIZE / 2.0,
            local.translation.z,
        );
    }
    // The source overview is ~480 m from the board, beyond the normal follow
    // shadow range. Only this explicitly captured view extends its cascades.
    let shadow_range = if qa.view == 0 { 650.0 } else { 360.0 };
    for mut shadow in &mut world.shadows {
        if shadow
            .bounds
            .last()
            .is_none_or(|last| (*last - shadow_range).abs() > 0.1)
        {
            *shadow = CascadeShadowConfigBuilder {
                first_cascade_far_bound: 35.0,
                maximum_distance: shadow_range,
                ..default()
            }
            .build();
        }
    }
    let Ok((mut transform, mut global, mut projection)) = world.camera.single_mut() else {
        return;
    };
    *transform = Transform::from_translation(view.position).looking_at(view.target, Vec3::Y);
    *global = GlobalTransform::from(*transform);
    *projection = if view.perspective {
        Projection::Perspective(PerspectiveProjection::default())
    } else {
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedHorizontal {
                viewport_width: view.width,
            },
            near: 0.1,
            far: 1500.0,
            ..OrthographicProjection::default_3d()
        })
    };
    if let Ok(mut window) = world.windows.single_mut() {
        if window.physical_width() != view.pixels.x || window.physical_height() != view.pixels.y {
            window.resolution.set_scale_factor_override(Some(1.0));
            window
                .resolution
                .set_physical_resolution(view.pixels.x, view.pixels.y);
        }
    }
    for (entity, mut node, name) in &mut world.ui {
        let overlay = name.is_some_and(|name| {
            matches!(
                name.as_str(),
                "HelpOverlayRoot" | "PauseMenuRoot" | "TeamSelectRoot" | "GameStateOverlay"
            )
        });
        if !view.hud || overlay {
            qa.hidden_ui.entry(entity).or_insert(node.display);
            node.display = Display::None;
        } else if let Some(display) = qa.hidden_ui.remove(&entity) {
            node.display = display;
        }
    }
    if qa.in_flight {
        if qa.finished_readbacks.contains(&qa.view)
            && qa
                .directory
                .join(view.file)
                .metadata()
                .is_ok_and(|file| file.len() > 32)
        {
            qa.view += 1;
            qa.in_flight = false;
            qa.stable_frames = 0;
            if qa.view == views().len() {
                let summary = serde_json::json!({ "version":env!("CARGO_PKG_VERSION"),
                    "method":"Bevy Screenshot::primary_window + save_to_disk", "captures":qa.captures,
                    "elapsed_seconds":qa.started.elapsed().as_secs_f64(), "authoritative_structures":world.structures.iter().count(),
                    "qa_fixtures":world.fixtures.iter().count(), "manual_interaction_verified":false });
                let result = std::fs::write(
                    qa.directory.join("qa-summary.json"),
                    serde_json::to_vec_pretty(&summary).unwrap(),
                );
                info!(
                    "VERDANT_QA completed images={} summary={summary}",
                    views().len()
                );
                exit.write(if result.is_ok() {
                    AppExit::Success
                } else {
                    AppExit::error()
                });
            }
        }
        return;
    }
    let ready = Readiness {
        admitted: session.join_confirmed(),
        running: matches!(game.state, GameState::Running),
        environments: world.environment.iter().count(),
        foliage: world.foliage.iter().count(),
        structures: world.structures.iter().count(),
        styled_structures: world.styled.iter().count(),
        scenes: world.scenes.iter().count(),
        ready_scenes: world
            .scenes
            .iter()
            .filter(|(_, instance)| {
                instance.is_some_and(|instance| spawner.instance_is_ready(**instance))
            })
            .count(),
        loaded_scenes: world
            .scenes
            .iter()
            .filter(|(scene, _)| {
                matches!(
                    asset_server.recursive_dependency_load_state(scene.0.id()),
                    RecursiveDependencyLoadState::Loaded
                )
            })
            .count(),
        local_at_river: world.local.single().is_ok_and(|(transform, stats)| {
            stats.is_alive() && transform.translation.xz().distance(QA_DESTINATION.xz()) < 3.0
        }),
        peers_at_river: world
            .actors
            .iter()
            .all(|actor| actor.translation.xz().distance(QA_DESTINATION.xz()) < 12.0),
        meshes: world.meshes.iter().count(),
    };
    qa.stable_frames = advance_stability(qa.stable_frames, ready_for_capture(ready));
    let diagnostic = qa.started.elapsed().as_secs() as u32 / 5;
    if diagnostic > qa.last_diagnostic {
        qa.last_diagnostic = diagnostic;
        info!(
            "VERDANT_QA readiness={ready:?} stable_frames={} view={}",
            qa.stable_frames, qa.view
        );
    }
    if qa.stable_frames < SETTLE_FRAMES {
        return;
    }
    if let Err(error) = std::fs::create_dir_all(&qa.directory) {
        error!("VERDANT_QA cannot create output directory: {error}");
        exit.write(AppExit::error());
        return;
    }
    let index = qa.view;
    let path = qa.directory.join(view.file);
    let capture = serde_json::json!({ "file":view.file, "position":view.position.to_array(),
        "target":view.target.to_array(),"orthographic_width":view.width,"pixels":view.pixels.to_array(),
        "hud":view.hud,"projection":if view.perspective {"production_default_perspective"} else {"source_orthographic"},
        "shadow_maximum_distance":shadow_range,"overview_shadow_range_override":qa.view == 0,"ready_scenes":ready.ready_scenes,"scenes":ready.scenes,
        "environment_roots":ready.environments,"foliage_roots":ready.foliage,"authoritative_structures":ready.structures,
        "styled_structure_visuals":ready.styled_structures,"network_player_entities":world.actors.iter().count(),
        "qa_render_fixtures":world.fixtures.iter().count(),"render_mesh_entities":ready.meshes,
        "shared_material_assets":world.materials.len(),"snapshot_tick":game.meta.snapshot_tick,
        "asset_root":shared::client_asset_root(),"version":env!("CARGO_PKG_VERSION"),
        "source_camera":if view.perspective {"client/src/camera.rs::locked_camera_offset(1.0)"} else {"art/verdant-confluence/scripts/build_scene.py"},"stable_frames":qa.stable_frames,
        "setup":"authoritative server structures and local agnes; five tagged render-only production creature fixtures" });
    info!("VERDANT_QA capture_request={capture}");
    qa.captures.push(capture);
    qa.in_flight = true;
    commands
        .spawn((Screenshot::primary_window(), QaShot(index)))
        .observe(save_to_disk(path))
        .observe(record_readback);
}

fn record_readback(
    captured: On<ScreenshotCaptured>,
    shots: Query<&QaShot>,
    mut qa: ResMut<QaState>,
) {
    if let Ok(shot) = shots.get(captured.entity) {
        qa.finished_readbacks.push(shot.0);
        info!(
            "VERDANT_QA readback_complete view={} dimensions={}x{}",
            shot.0,
            captured.image.width(),
            captured.image.height()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_is_strictly_opt_in_and_time_budget_is_bounded() {
        assert!(qa_output(None).is_none());
        assert!(qa_output(Some("".into())).is_none());
        assert_eq!(
            qa_output(Some("/tmp/qa".into())),
            Some(PathBuf::from("/tmp/qa"))
        );
        assert_eq!(bounded_timeout(None), 240);
        assert_eq!(bounded_timeout(Some("0")), 30);
        assert_eq!(bounded_timeout(Some("99999")), 600);
    }

    #[test]
    fn screenshot_waits_for_authoritative_structures_scene_instances_assets_and_actor() {
        let ready = Readiness {
            admitted: true,
            running: true,
            environments: 1,
            foliage: 1,
            structures: 8,
            styled_structures: 8,
            scenes: 11,
            ready_scenes: 11,
            loaded_scenes: 11,
            local_at_river: true,
            peers_at_river: true,
            meshes: 200,
        };
        assert!(ready_for_capture(ready));
        for changed in [
            Readiness {
                peers_at_river: false,
                ..ready
            },
            Readiness {
                ready_scenes: 10,
                ..ready
            },
            Readiness {
                loaded_scenes: 10,
                ..ready
            },
            Readiness {
                styled_structures: 7,
                ..ready
            },
            Readiness {
                structures: 9,
                ..ready
            },
            Readiness {
                environments: 2,
                ..ready
            },
            Readiness {
                local_at_river: false,
                ..ready
            },
            Readiness {
                admitted: false,
                ..ready
            },
        ] {
            assert!(!ready_for_capture(changed));
        }
        assert_eq!(advance_stability(44, true), 45);
        assert_eq!(advance_stability(45, true), 45);
        assert_eq!(advance_stability(44, false), 0);
    }

    #[test]
    fn cameras_match_art_axis_conversion_and_capture_set_is_finite() {
        let plan = views();
        assert_eq!(plan.len(), 4);
        assert!(plan[3].perspective && plan[3].hud);
        assert_eq!(plan[0].position, Vec3::new(230.0, 290.0, 300.0));
        assert_eq!(plan[1].position, Vec3::new(44.0, 49.0, 55.0));
        assert!(plan[1].hud);
        assert!(!plan[0].hud);
        assert!((plan[2].target.x - plan[2].target.z).abs() < 0.001);
    }
}
