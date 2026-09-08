//! Opt-in bounded native navigation proof. All orders use production button
//! input and the actual window cursor; no movement/transform fixtures are inserted.
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
    camera::MainCamera,
    combat::CombatStats,
    help_overlay::HelpOverlayVisible,
    maps::MapLayout,
    minimap::MinimapNavigationState,
    net::{ClientSession, GameState, GameStateSnapshot, NetworkPlayerId},
    player::{MovementRoute, MovementTarget, Player},
    team::Team,
    verdant3d::VerdantEnvironment,
};

const FILES: [&str; 4] = [
    "01-minimap-route.png",
    "02-minimap-arrival.png",
    "03-world-route.png",
    "04-navigation-complete.png",
];

pub(crate) struct NavigationQaPlugin;
impl Plugin for NavigationQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(directory) = std::env::var_os("OMOBA_VISUAL_QA_DIR").map(PathBuf::from) else {
            return;
        };
        let timeout = std::env::var("OMOBA_VISUAL_QA_TIMEOUT")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(90)
            .clamp(30, 180);
        app.insert_resource(NavigationQa {
            directory,
            started: Instant::now(),
            stage_started: Instant::now(),
            timeout: Duration::from_secs(timeout),
            stage: 0,
            frames: 0,
            pending: None,
            cursor: None,
            start: Vec3::ZERO,
            minimap_target: Vec3::ZERO,
            world_target: Vec3::ZERO,
            obstacle: Vec3::ZERO,
            obstacle_radius: 0.0,
            obstacle_id: String::new(),
            approach: Vec3::ZERO,
            camera_focus: None,
            player_id: 0,
            events: Vec::new(),
            samples: Vec::new(),
            captures: Vec::new(),
            readbacks: Vec::new(),
            last_tick: 0,
            persistent: false,
            width: env_pixels("OMOBA_QA_WIDTH", 1280),
            height: env_pixels("OMOBA_QA_HEIGHT", 720),
        })
        .add_systems(Startup, label_scripted_input)
        .add_systems(
            PreUpdate,
            inject_input
                .after(bevy::input::InputSystems)
                .before(bevy::ui::UiSystems::Focus),
        )
        .add_systems(
            PreUpdate,
            admission_buttons.after(bevy::ui::UiSystems::Focus),
        )
        .add_systems(
            PostUpdate,
            observe_navigation
                .after(bevy::ui::UiSystems::Layout)
                .after(bevy::transform::TransformSystems::Propagate),
        );
    }
}

fn label_scripted_input(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(16.0),
            top: Val::Px(70.0),
            padding: UiRect::all(Val::Px(6.0)),
            ..default()
        },
        Text::new("QA: scripted input / live server"),
        TextFont {
            font_size: 14.0,
            ..default()
        },
        TextColor(Color::srgb(1.0, 0.85, 0.4)),
        BackgroundColor(Color::srgb(0.04, 0.04, 0.04)),
        ZIndex(200),
    ));
}

fn env_pixels(name: &str, fallback: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(fallback)
}

#[derive(Clone, Copy, Debug)]
enum ScriptedInput {
    Click(MouseButton, bool),
    Key(KeyCode),
}

#[derive(Resource)]
struct NavigationQa {
    directory: PathBuf,
    started: Instant,
    stage_started: Instant,
    timeout: Duration,
    stage: u8,
    frames: u32,
    pending: Option<ScriptedInput>,
    cursor: Option<Vec2>,
    start: Vec3,
    minimap_target: Vec3,
    world_target: Vec3,
    obstacle: Vec3,
    obstacle_radius: f32,
    obstacle_id: String,
    approach: Vec3,
    camera_focus: Option<Vec3>,
    player_id: u64,
    events: Vec<serde_json::Value>,
    samples: Vec<serde_json::Value>,
    captures: Vec<serde_json::Value>,
    readbacks: Vec<usize>,
    last_tick: u64,
    persistent: bool,
    width: u32,
    height: u32,
}

impl NavigationQa {
    fn advance(&mut self, stage: u8) {
        self.stage = stage;
        self.frames = 0;
        self.stage_started = Instant::now();
    }
    fn click(&mut self, cursor: Vec2, button: MouseButton, alt: bool) {
        self.cursor = Some(cursor);
        self.pending = Some(ScriptedInput::Click(button, alt));
    }
    fn event(&mut self, name: &str, position: Vec3, tick: u64, detail: serde_json::Value) {
        let event = serde_json::json!({"event":name,"position":position.to_array(),"snapshot_tick":tick,"elapsed_seconds":self.started.elapsed().as_secs_f64(),"detail":detail});
        info!("NAVIGATION_QA event={event}");
        self.events.push(event);
    }
}

fn inject_input(
    mut qa: ResMut<NavigationQa>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    // Release on the following frame: continued travel is never sustained input.
    mouse.release(MouseButton::Left);
    mouse.release(MouseButton::Right);
    for key in [
        KeyCode::AltLeft,
        KeyCode::KeyP,
        KeyCode::Escape,
        KeyCode::Space,
    ] {
        keys.release(key);
    }
    if let Ok(mut window) = windows.single_mut() {
        window.resolution.set_scale_factor_override(Some(1.0));
        if window.physical_width() != qa.width || window.physical_height() != qa.height {
            window
                .resolution
                .set_physical_resolution(qa.width, qa.height);
        }
        if let Some(cursor) = qa.cursor {
            window.set_cursor_position(Some(cursor));
        }
    }
    if let Some(input) = qa.pending.take() {
        match input {
            ScriptedInput::Click(button, alt) => {
                if alt {
                    keys.press(KeyCode::AltLeft);
                }
                mouse.press(button);
            }
            ScriptedInput::Key(key) => keys.press(key),
        }
    }
}

fn admission_buttons(
    qa: Res<NavigationQa>,
    session: Res<ClientSession>,
    help: Res<HelpOverlayVisible>,
    mut buttons: Query<(&Name, &mut Interaction), With<Button>>,
) {
    if qa.stage != 0 {
        return;
    }
    let team_button = if std::env::var("OMOBA_QA_TEAM").as_deref() == Ok("blue") {
        "TeamBlueButton"
    } else {
        "TeamGreenButton"
    };
    for (name, mut interaction) in &mut buttons {
        if (session.is_connected() && !session.join_confirmed() && name.as_str() == team_button)
            || (session.join_confirmed() && help.0 && name.as_str() == "HelpDismissButton")
        {
            *interaction = Interaction::Pressed;
        }
    }
}

#[derive(bevy::ecs::system::SystemParam)]
struct NavigationScene<'w, 's> {
    keys: Res<'w, ButtonInput<KeyCode>>,
    players: Query<
        'w,
        's,
        (
            &'static Transform,
            &'static CombatStats,
            &'static Team,
            &'static NetworkPlayerId,
            Option<&'static MovementTarget>,
            Option<&'static MovementRoute>,
        ),
        With<Player>,
    >,
    cameras: Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<MainCamera>>,
    environment: Query<'w, 's, Entity, With<VerdantEnvironment>>,
    scenes: Query<'w, 's, (&'static SceneRoot, Option<&'static SceneInstance>)>,
    windows: Query<'w, 's, Entity, With<PrimaryWindow>>,
}

/// Choose an actual tree between two reachable forest points. Approach is
/// performed by ordinary RMB input from spawn, never by editing a transform.
fn forest_crossing(spawn: Vec3) -> Option<(Vec3, Vec3, Vec3, f32, String)> {
    let map = shared::navigation::world_navigation();
    let mut candidates: Vec<_> = map
        .obstacles()
        .iter()
        .filter(|o| o.kind == "tree_trunk")
        .map(|o| {
            let center = o
                .vertices
                .iter()
                .fold(Vec2::ZERO, |a, p| a + Vec2::from_array(*p))
                / o.vertices.len() as f32;
            (center, o)
        })
        .collect();
    candidates.sort_by(|a, b| {
        a.0.distance_squared(spawn.xz())
            .total_cmp(&b.0.distance_squared(spawn.xz()))
    });
    for (center, obstacle) in candidates {
        let direction = (center - spawn.xz()).normalize_or_zero();
        let a = center - direction * 15.0;
        let b = center + direction * 15.0;
        if !map.point_clear(a.to_array()) || !map.point_clear(b.to_array()) {
            continue;
        }
        let Some(route) = map.plan_route(a.to_array(), b.to_array(), &[]) else {
            continue;
        };
        let mut previous = a;
        let length: f32 = route
            .iter()
            .map(|p| {
                let point = Vec2::from_array(*p);
                let distance = previous.distance(point);
                previous = point;
                distance
            })
            .sum();
        if route.len() < 2 || length >= 54.0 {
            continue;
        }
        let radius = obstacle
            .vertices
            .iter()
            .map(|p| center.distance(Vec2::from_array(*p)))
            .fold(0.0_f32, f32::max)
            * (std::f32::consts::PI / 16.0).cos()
            + shared::navigation::HERO_RADIUS;
        return Some((
            Vec3::new(a.x, 0.5, a.y),
            Vec3::new(b.x, 0.5, b.y),
            Vec3::new(center.x, 0.5, center.y),
            radius,
            obstacle.id.clone(),
        ));
    }
    None
}

fn minimap_cursor(
    layout: MapLayout,
    diagnostics: &serde_json::Value,
    target: Vec3,
) -> Option<Vec2> {
    let rect = diagnostics["container_rect"].as_array()?;
    let min = Vec2::new(rect[0].as_f64()? as f32, rect[1].as_f64()? as f32);
    let max = Vec2::new(rect[2].as_f64()? as f32, rect[3].as_f64()? as f32);
    let normalized = (target.xz() - layout.min) / layout.size();
    Some(min + Vec2::new(normalized.y, 1.0 - normalized.x) * (max - min))
}

fn world_cursor(scene: &NavigationScene, target: Vec3) -> Option<Vec2> {
    let (camera, transform) = scene.cameras.single().ok()?;
    camera
        .world_to_viewport(transform, Vec3::new(target.x, 0.0, target.z))
        .ok()
}

#[allow(clippy::too_many_arguments)]
fn observe_navigation(
    mut commands: Commands,
    mut qa: ResMut<NavigationQa>,
    scene: NavigationScene,
    session: Res<ClientSession>,
    game: Res<GameStateSnapshot>,
    help: Res<HelpOverlayVisible>,
    shop: Res<crate::shop::ShopState>,
    pause: Res<crate::pause_menu::PauseMenuState>,
    context: Res<crate::input_context::GameplayInputContext>,
    nav: Res<MinimapNavigationState>,
    layout: Res<MapLayout>,
    minimap: crate::minimap::MinimapQaScene,
    assets: Res<AssetServer>,
    spawner: Res<SceneSpawner>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.stage == 255 {
        return;
    }
    if qa.started.elapsed() >= qa.timeout {
        fail(&mut qa, &mut exit, "bounded scenario timed out");
        return;
    }
    qa.frames += 1;
    let Ok((transform, stats, _team, id, target, route)) = scene.players.single() else {
        return;
    };
    let position = transform.translation;
    let tick = game.meta.snapshot_tick;
    let diagnostics = minimap.diagnostics();
    if tick != qa.last_tick {
        qa.last_tick = tick;
        let stage = qa.stage;
        qa.samples.push(serde_json::json!({"snapshot_tick":tick,"stage":stage,"position":position.to_array(),"hp":stats.hp,"movement_target":target.map(|t|t.target.to_array()),"route":route.map(|r|r.waypoints.iter().map(|p|p.to_array()).collect::<Vec<_>>()),"rmb_held":mouse.pressed(MouseButton::Right),"minimap":diagnostics}));
    }
    let idle = target.is_none() && route.is_none();
    if qa.stage > 0
        && (!session.join_confirmed()
            || !matches!(game.state, GameState::Running)
            || !stats.is_alive())
    {
        fail(
            &mut qa,
            &mut exit,
            "admission/running/living precondition lost",
        );
        return;
    }
    match qa.stage {
        0 => {
            let ready = session.join_confirmed()
                && matches!(game.state, GameState::Running)
                && !help.0
                && context.gameplay_allowed()
                && !scene.environment.is_empty()
                && !scene.scenes.is_empty()
                && scene.scenes.iter().all(|(root, instance)| {
                    instance.is_some_and(|instance| spawner.instance_is_ready(**instance))
                        && matches!(
                            assets.recursive_dependency_load_state(root.0.id()),
                            RecursiveDependencyLoadState::Loaded
                        )
                });
            if !ready {
                qa.frames = 0;
                return;
            }
            if qa.frames < 45 {
                return;
            }
            let Some((approach, destination, center, radius, obstacle_id)) =
                forest_crossing(position)
            else {
                fail(
                    &mut qa,
                    &mut exit,
                    "no bounded real forest crossing is available",
                );
                return;
            };
            qa.approach = approach;
            qa.obstacle = center;
            qa.obstacle_radius = radius;
            qa.obstacle_id = obstacle_id;
            qa.minimap_target = destination;
            qa.player_id = id.0;
            let Some(cursor) = minimap_cursor(*layout, &diagnostics, approach) else {
                return;
            };
            qa.event("forest_approach_input", position, tick,
                serde_json::json!({"destination":approach.to_array(),"cursor_logical":cursor.to_array(),"ordinary_movement":true}));
            qa.click(cursor, MouseButton::Right, false);
            qa.advance(100);
        }
        100 => {
            if idle && position.xz().distance(qa.approach.xz()) < 0.3 {
                qa.advance(101);
            }
        }
        101 => {
            if qa.stage_started.elapsed() < Duration::from_millis(800) {
                return;
            }
            qa.start = position;
            qa.camera_focus = nav.focus_target;
            let Some(cursor) = minimap_cursor(*layout, &diagnostics, qa.minimap_target) else {
                return;
            };
            let detail = serde_json::json!({"destination":qa.minimap_target.to_array(),
                "cursor_logical":cursor.to_array(),"minimap":diagnostics,
                "obstacle_center":qa.obstacle.to_array(),"collision_radius":qa.obstacle_radius,
                "forest_obstacle_id":qa.obstacle_id});
            qa.event("minimap_rmb_input", position, tick, detail);
            qa.click(cursor, MouseButton::Right, false);
            qa.advance(1);
        }
        1 => {
            if !mouse.just_pressed(MouseButton::Right) {
                fail(
                    &mut qa,
                    &mut exit,
                    "minimap RMB input edge was not observed",
                );
                return;
            }
            let Some(route) = route else {
                fail(&mut qa, &mut exit, "minimap RMB did not create a route");
                return;
            };
            let mut previous = qa.start;
            let planned_length: f32 = route
                .waypoints
                .iter()
                .map(|point| {
                    let distance = previous.xz().distance(point.xz());
                    previous = *point;
                    distance
                })
                .sum();
            if !planned_length.is_finite()
                || planned_length
                    >= (qa.start.xz().distance(qa.minimap_target.xz()) * 1.8).max(25.0)
            {
                fail(
                    &mut qa,
                    &mut exit,
                    "forest crossing planned an excessive detour",
                );
                return;
            }
            if route.destination.xz().distance(qa.minimap_target.xz()) > 0.3
                || route.waypoints.len() < 2
                || nav.focus_target != qa.camera_focus
            {
                fail(
                    &mut qa,
                    &mut exit,
                    "minimap order did not detour, preserve target and preserve camera focus",
                );
                return;
            }
            qa.event("minimap_order_accepted", position, tick, serde_json::json!({"waypoints":route.waypoints.iter().map(|p|p.to_array()).collect::<Vec<_>>(),"destination":route.destination.to_array(),"planned_route_length":planned_length,"camera_focus_unchanged":true}));
            qa.advance(2);
        }
        2 => {
            if !capture_travel_after_settle(
                &mut commands,
                &mut qa,
                &mut exit,
                0,
                position,
                tick,
                target,
                route,
                &diagnostics,
            ) {
                return;
            }
            if !mouse.pressed(MouseButton::Right)
                && route.is_some()
                && position.xz().distance(qa.start.xz()) > 1.0
            {
                qa.persistent = true;
            }
            if idle && position.xz().distance(qa.minimap_target.xz()) < 0.3 {
                if !qa.persistent {
                    fail(
                        &mut qa,
                        &mut exit,
                        "travel after RMB release was not observed",
                    );
                    return;
                }
                let destination = qa.minimap_target.to_array();
                qa.event("minimap_arrival", position, tick, serde_json::json!({"destination":destination,"intent_cleared":true,"travel_after_release":true,"minimap":diagnostics}));
                capture(&mut commands, &mut qa, 1, position, tick);
                qa.advance(3);
            }
        }
        3 => {
            // Leave enough real snapshots at the stopped destination for the
            // independent hello-only observer in the native wrapper.
            if qa.stage_started.elapsed() < Duration::from_millis(800) {
                return;
            }
            let navigation = shared::navigation::world_navigation();
            let Some(world_target) = [
                Vec3::new(5.0, 0.0, -4.0),
                Vec3::new(-5.0, 0.0, -4.0),
                Vec3::new(4.0, 0.0, 5.0),
                Vec3::new(-4.0, 0.0, 5.0),
            ]
            .into_iter()
            .map(|offset| position + offset)
            .find(|p| navigation.segment_clear(position.xz().to_array(), p.xz().to_array())) else {
                fail(&mut qa, &mut exit, "no nearby open ground order");
                return;
            };
            qa.world_target = world_target;
            let Some(cursor) = world_cursor(&scene, qa.world_target) else {
                fail(
                    &mut qa,
                    &mut exit,
                    "world destination is outside camera projection",
                );
                return;
            };
            let destination = qa.world_target.to_array();
            qa.event(
                "world_rmb_input",
                position,
                tick,
                serde_json::json!({"destination":destination,"cursor_logical":cursor.to_array()}),
            );
            qa.click(cursor, MouseButton::Right, false);
            qa.advance(4);
        }
        4 => {
            if !mouse.just_pressed(MouseButton::Right) {
                fail(&mut qa, &mut exit, "world RMB input edge was not observed");
                return;
            }
            let Some(route) = route else {
                fail(&mut qa, &mut exit, "world RMB did not create a route");
                return;
            };
            if route.destination.xz().distance(qa.world_target.xz()) > 0.3 {
                fail(
                    &mut qa,
                    &mut exit,
                    "world RMB destination differs from projected ground",
                );
                return;
            }
            qa.event("world_order_accepted", position, tick, serde_json::json!({"destination":route.destination.to_array(),"waypoints":route.waypoints.iter().map(|p|p.to_array()).collect::<Vec<_>>()}));
            qa.advance(5);
        }
        5 => {
            if !capture_travel_after_settle(
                &mut commands,
                &mut qa,
                &mut exit,
                2,
                position,
                tick,
                target,
                route,
                &diagnostics,
            ) {
                return;
            }
            if idle && position.xz().distance(qa.world_target.xz()) < 0.3 {
                let destination = qa.world_target.to_array();
                qa.event(
                    "world_arrival",
                    position,
                    tick,
                    serde_json::json!({"destination":destination,"intent_cleared":true,"minimap":diagnostics}),
                );
                qa.advance(6);
            }
        }
        6 => {
            if qa.stage_started.elapsed() < Duration::from_millis(800) {
                return;
            }
            let Some(cursor) = world_cursor(&scene, position + Vec3::new(3.0, 0.0, 3.0)) else {
                return;
            };
            qa.click(cursor, MouseButton::Right, true);
            qa.advance(7);
        }
        7 | 8 | 10 | 11 | 14 | 15 => {
            let alt_held = scene.keys.pressed(KeyCode::AltLeft);
            if !mouse.just_pressed(MouseButton::Right) || alt_held != matches!(qa.stage, 7 | 8) {
                fail(
                    &mut qa,
                    &mut exit,
                    "expected isolated RMB input edge/modifier was not observed",
                );
                return;
            }
            if !idle || position.xz().distance(qa.world_target.xz()) > 0.3 {
                fail(&mut qa, &mut exit, "orbit/modal RMB leaked into movement");
                return;
            }
            let name = match qa.stage {
                7 => "alt_world_blocked",
                8 => "alt_minimap_blocked",
                10 => "shop_world_blocked",
                11 => "shop_minimap_blocked",
                14 => "pause_world_blocked",
                _ => "pause_minimap_blocked",
            };
            if (matches!(qa.stage, 10 | 11) && (!shop.open || context.gameplay_allowed()))
                || (matches!(qa.stage, 14 | 15) && (!pause.open || context.gameplay_allowed()))
            {
                fail(&mut qa, &mut exit, "expected production modal was not open");
                return;
            }
            qa.event(name, position, tick, serde_json::json!({"intent_absent":true,"rmb_just_pressed":true,"alt_held":alt_held,"shop_open":shop.open,"pause_open":pause.open}));
            match qa.stage {
                7 | 10 | 14 => {
                    let Some(cursor) = minimap_cursor(*layout, &diagnostics, qa.minimap_target)
                    else {
                        return;
                    };
                    // Separate press edges even on systems with very fast frames.
                    qa.cursor = Some(cursor);
                    let next = qa.stage + 20;
                    qa.advance(next);
                }
                8 => {
                    qa.pending = Some(ScriptedInput::Key(KeyCode::KeyP));
                    qa.advance(9);
                }
                11 => {
                    qa.pending = Some(ScriptedInput::Key(KeyCode::Escape));
                    qa.advance(12);
                }
                _ => {
                    qa.pending = Some(ScriptedInput::Key(KeyCode::Escape));
                    qa.advance(16);
                }
            }
        }
        27 | 30 | 34 => {
            let alt = qa.stage == 27;
            qa.pending = Some(ScriptedInput::Click(MouseButton::Right, alt));
            let next = qa.stage - 19;
            qa.advance(next);
        }
        9 | 13 => {
            if (qa.stage == 9 && !shop.open) || (qa.stage == 13 && !pause.open) {
                return;
            }
            let Some(cursor) = world_cursor(&scene, position + Vec3::new(3.0, 0.0, 3.0)) else {
                return;
            };
            qa.click(cursor, MouseButton::Right, false);
            let next = qa.stage + 1;
            qa.advance(next);
        }
        12 => {
            if shop.open || pause.open {
                fail(&mut qa, &mut exit, "Escape did not close shop alone");
                return;
            }
            if qa.frames < 2 {
                return;
            }
            qa.pending = Some(ScriptedInput::Key(KeyCode::Escape));
            qa.advance(13);
        }
        16 => {
            if pause.open || shop.open || !context.gameplay_allowed() {
                return;
            }
            let Some(cursor) = minimap_cursor(*layout, &diagnostics, qa.minimap_target) else {
                return;
            };
            qa.click(cursor, MouseButton::Left, false);
            qa.advance(17);
        }
        17 => {
            if !mouse.just_pressed(MouseButton::Left)
                || !idle
                || nav
                    .focus_target
                    .is_none_or(|focus| focus.xz().distance(qa.minimap_target.xz()) > 0.3)
            {
                fail(
                    &mut qa,
                    &mut exit,
                    "LMB minimap camera pan did not remain isolated from movement",
                );
                return;
            }
            qa.event("lmb_minimap_pan", position, tick, serde_json::json!({"camera_focus":nav.focus_target.map(|p|p.to_array()),"intent_absent":true}));
            qa.pending = Some(ScriptedInput::Key(KeyCode::Space));
            qa.advance(18);
        }
        18 => {
            if qa.frames < 20 {
                return;
            }
            capture(&mut commands, &mut qa, 3, position, tick);
            qa.advance(19);
        }
        19 => {
            if qa.readbacks.len() != FILES.len()
                || FILES.iter().any(|file| {
                    !qa.directory
                        .join(file)
                        .metadata()
                        .is_ok_and(|m| m.len() > 32)
                })
            {
                return;
            }
            let summary = serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"scenario":"navigation","scripted_input":true,"input_method":"ButtonInput mouse/key press and release with actual logical Window cursor; production admission/help button interactions","manual_interaction_verified":false,"teleport_or_movement_fixture":false,"capture_method":"Bevy Screenshot::primary_window + save_to_disk","player_id":qa.player_id,"start":qa.start.to_array(),"obstacle":{"center":qa.obstacle.to_array(),"collision_radius":qa.obstacle_radius,"id":qa.obstacle_id,"kind":"tree"},"route_display":"minimap_only","events":qa.events,"samples":qa.samples,"captures":qa.captures,"pixels":[qa.width,qa.height],"elapsed_seconds":qa.started.elapsed().as_secs_f64(),"pass":true});
            if std::fs::write(
                qa.directory.join("qa-summary.json"),
                serde_json::to_vec_pretty(&summary).unwrap(),
            )
            .is_err()
            {
                fail(&mut qa, &mut exit, "cannot save summary");
                return;
            }
            info!("NAVIGATION_QA completed");
            qa.stage = 255;
            if let Ok(window) = scene.windows.single() {
                commands.entity(window).despawn();
            }
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}

#[derive(Component)]
struct NavigationShot(usize);

fn capture_travel_after_settle(
    commands: &mut Commands,
    qa: &mut NavigationQa,
    exit: &mut MessageWriter<AppExit>,
    index: usize,
    position: Vec3,
    tick: u64,
    target: Option<&MovementTarget>,
    route: Option<&MovementRoute>,
    minimap: &serde_json::Value,
) -> bool {
    // Preserve first-frame acceptance telemetry, then allow the renderer to
    // lay out the minimap route before reading it back during real travel.
    if qa.frames > 8 {
        return true;
    }
    let Some(route) = route.filter(|route| target.is_some() && !route.waypoints.is_empty()) else {
        fail(qa, exit, "route ended before the delayed travel screenshot");
        return false;
    };
    if qa.frames == 8 {
        if minimap["route_segments"]
            .as_array()
            .is_none_or(|segments| segments.is_empty())
            || minimap["route_destination"].is_null()
        {
            fail(
                qa,
                exit,
                "active route has no computed minimap line/destination",
            );
            return false;
        }
        qa.event("travel_capture_after_settle", position, tick, serde_json::json!({
            "file":FILES[index], "frames_after_acceptance":8,
            "remaining_waypoints":route.waypoints.iter().map(|point|point.to_array()).collect::<Vec<_>>(),
            "destination":route.destination.to_array(), "movement_target_present":true, "minimap":minimap,
        }));
        capture(commands, qa, index, position, tick);
    }
    true
}

fn capture(
    commands: &mut Commands,
    qa: &mut NavigationQa,
    index: usize,
    position: Vec3,
    tick: u64,
) {
    if std::fs::create_dir_all(&qa.directory).is_err() {
        return;
    }
    qa.captures.push(serde_json::json!({"file":FILES[index],"position":position.to_array(),"snapshot_tick":tick,"scripted_input":true}));
    commands
        .spawn((Screenshot::primary_window(), NavigationShot(index)))
        .observe(save_to_disk(qa.directory.join(FILES[index])))
        .observe(readback);
}

fn readback(
    captured: On<ScreenshotCaptured>,
    shots: Query<&NavigationShot>,
    mut qa: ResMut<NavigationQa>,
) {
    if let Ok(shot) = shots.get(captured.entity) {
        if captured.image.width() == qa.width && captured.image.height() == qa.height {
            qa.readbacks.push(shot.0);
            info!("NAVIGATION_QA readback_complete index={}", shot.0);
        }
    }
}

fn fail(qa: &mut NavigationQa, exit: &mut MessageWriter<AppExit>, reason: &str) {
    error!("NAVIGATION_QA failed stage={}: {reason}", qa.stage);
    let _ = std::fs::create_dir_all(&qa.directory);
    let _ = std::fs::write(qa.directory.join("qa-failure.json"), serde_json::to_vec_pretty(&serde_json::json!({"stage":qa.stage,"reason":reason,"events":qa.events,"samples":qa.samples})).unwrap());
    qa.stage = 255;
    exit.write(AppExit::error());
}
