//! Bounded native targeting proof: real mouse/TouchInput and live server damage.
//! The opt-in server fixture only places admitted players and disables ambient AI.
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use bevy::{
    app::AppExit,
    asset::RecursiveDependencyLoadState,
    ecs::system::SystemParam,
    input::touch::{TouchInput, TouchPhase},
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    scene::{SceneInstance, SceneSpawner},
    ui::FocusPolicy,
    window::PrimaryWindow,
};

use crate::{
    camera::MainCamera,
    combat::{CombatStats, LocalCastCooldown, TargetState},
    help_overlay::HelpOverlayVisible,
    input_context::GameplayInputContext,
    mobile_controls::MobileControls,
    net::{
        ClientSession, GameState, GameStateSnapshot, NetworkCommand, NetworkHeroClass,
        NetworkPlayerId, TargetId, TargetKind,
    },
    player::{MovementTarget, Player},
    targeting::{BasicAttackState, TargetAimPreview},
    team::{Team, TeamSelection},
    verdant3d::VerdantEnvironment,
};

// Materialize diagnostic arguments before borrowing the ResMut receiver.
macro_rules! event {
    ($qa:expr, $name:expr, $tick:expr, $detail:expr) => {{
        let detail = $detail;
        $qa.event($name, $tick, detail);
    }};
}

const FILES: [&str; 4] = [
    "01-target-ready.png",
    "02-target-preview.png",
    "03-basic-attack.png",
    "04-target-canceled.png",
];

pub(crate) struct TargetingQaPlugin;
impl Plugin for TargetingQaPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var("OMOBA_VISUAL_QA_SCENARIO").as_deref() != Ok("targeting") {
            return;
        }
        let Some(directory) = std::env::var_os("OMOBA_VISUAL_QA_DIR").map(PathBuf::from) else {
            return;
        };
        app.insert_resource(TargetingQa::new(directory))
            .add_systems(Startup, label)
            .add_systems(PreUpdate, inject_touch.before(bevy::input::InputSystems))
            .add_systems(
                PreUpdate,
                inject_mouse
                    .after(bevy::input::InputSystems)
                    .before(bevy::ui::UiSystems::Focus),
            )
            .add_systems(PreUpdate, admission.after(bevy::ui::UiSystems::Focus))
            .add_systems(
                PostUpdate,
                (observe_commands, observe_targeting)
                    .chain()
                    .after(bevy::ui::UiSystems::Layout)
                    .after(bevy::transform::TransformSystems::Propagate),
            );
    }
}

fn label(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(35.0),
            bottom: Val::Px(3.0),
            padding: UiRect::all(Val::Px(2.0)),
            ..default()
        },
        Text::new("QA: scripted input · server placement fixture"),
        TextFont {
            font_size: 10.0,
            ..default()
        },
        TextColor(Color::WHITE),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
        FocusPolicy::Pass,
        ZIndex(200),
    ));
}

#[derive(Clone, Copy)]
enum MouseInput {
    Click(Vec2, MouseButton, bool),
    Key(KeyCode),
}

#[derive(Resource)]
struct TargetingQa {
    directory: PathBuf,
    started: Instant,
    stage_started: Instant,
    timeout: Duration,
    stage: u8,
    frames: u32,
    width: u32,
    height: u32,
    mouse: Option<MouseInput>,
    touches: Vec<(u64, TouchPhase, Vec2)>,
    player_id: u64,
    target_id: u64,
    other_target_ids: Vec<u64>,
    start: Vec3,
    target_hp: f32,
    damage_count: usize,
    latest_hp: f32,
    drag_end: Vec2,
    events: Vec<serde_json::Value>,
    commands: Vec<serde_json::Value>,
    captures: Vec<serde_json::Value>,
    readbacks: Vec<usize>,
    focus_requested: bool,
}

impl TargetingQa {
    fn new(directory: PathBuf) -> Self {
        let pixels = |key, fallback| {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(fallback)
        };
        Self {
            directory,
            started: Instant::now(),
            stage_started: Instant::now(),
            timeout: Duration::from_secs(
                pixels("OMOBA_VISUAL_QA_TIMEOUT", 90).clamp(30, 240) as u64
            ),
            stage: 0,
            frames: 0,
            width: pixels("OMOBA_QA_WIDTH", 1280),
            height: pixels("OMOBA_QA_HEIGHT", 720),
            mouse: None,
            touches: Vec::new(),
            player_id: 0,
            target_id: 0,
            other_target_ids: Vec::new(),
            start: Vec3::ZERO,
            target_hp: 0.0,
            damage_count: 0,
            latest_hp: 0.0,
            drag_end: Vec2::ZERO,
            events: Vec::new(),
            commands: Vec::new(),
            captures: Vec::new(),
            readbacks: Vec::new(),
            focus_requested: false,
        }
    }
    fn advance(&mut self, stage: u8) {
        self.stage = stage;
        self.frames = 0;
        self.stage_started = Instant::now();
    }
    fn event(&mut self, event: &str, tick: u64, detail: serde_json::Value) {
        let value = serde_json::json!({"event":event,"snapshot_tick":tick,
            "elapsed_seconds":self.started.elapsed().as_secs_f64(),"detail":detail});
        info!("TARGETING_QA event={value}");
        self.events.push(value);
    }
    fn attacks(&self) -> usize {
        self.commands
            .iter()
            .filter(|c| c["kind"] == "basic_attack")
            .count()
    }
    fn click(&mut self, p: Vec2, button: MouseButton, alt: bool) {
        self.mouse = Some(MouseInput::Click(p, button, alt));
    }
    fn touch(&mut self, id: u64, phase: TouchPhase, p: Vec2) {
        self.touches.push((id, phase, p));
    }
}

fn inject_touch(
    mut qa: ResMut<TargetingQa>,
    windows: Query<Entity, With<PrimaryWindow>>,
    mut events: MessageWriter<TouchInput>,
) {
    if let Ok(window) = windows.single() {
        for (id, phase, position) in qa.touches.drain(..) {
            events.write(TouchInput {
                phase,
                position,
                window,
                force: None,
                id,
            });
        }
    }
}

fn inject_mouse(
    mut qa: ResMut<TargetingQa>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    mouse.release(MouseButton::Left);
    mouse.release(MouseButton::Right);
    for key in [KeyCode::AltLeft, KeyCode::KeyS, KeyCode::Escape] {
        keys.release(key);
    }
    if let Ok(mut window) = windows.single_mut() {
        if !qa.focus_requested {
            window.focused = true;
            qa.focus_requested = true;
        }
        window.resolution.set_scale_factor_override(Some(1.0));
        if window.physical_width() != qa.width || window.physical_height() != qa.height {
            window
                .resolution
                .set_physical_resolution(qa.width, qa.height);
        }
        if let Some(input) = qa.mouse.take() {
            match input {
                MouseInput::Click(p, b, alt) => {
                    window.set_cursor_position(Some(p));
                    if alt {
                        keys.press(KeyCode::AltLeft);
                    }
                    mouse.press(b);
                }
                MouseInput::Key(k) => {
                    keys.press(k);
                }
            }
        }
    }
}

fn admission(
    qa: Res<TargetingQa>,
    session: Res<ClientSession>,
    selection: Res<TeamSelection>,
    help: Res<HelpOverlayVisible>,
    mut buttons: Query<(&Name, &mut Interaction), With<Button>>,
) {
    if qa.stage != 0 {
        return;
    }
    for (name, mut interaction) in &mut buttons {
        let press = (session.is_connected()
            && !session.join_confirmed()
            && name.as_str()
                == if selection.hero_class == shared::HeroClass::Mage {
                    "TeamGreenButton"
                } else {
                    "ClassButton-mage"
                })
            || (session.join_confirmed() && help.0 && name.as_str() == "HelpDismissButton");
        if press {
            *interaction = Interaction::Pressed;
        }
    }
}

fn observe_commands(
    mut qa: ResMut<TargetingQa>,
    game: Res<GameStateSnapshot>,
    mut outgoing: MessageReader<NetworkCommand>,
) {
    for command in outgoing.read() {
        let value = match command {
            NetworkCommand::BasicAttack { target } => {
                serde_json::json!({"kind":"basic_attack","target":target})
            }
            NetworkCommand::Cast { target, slot } => {
                serde_json::json!({"kind":"cast","target":target,"slot":slot})
            }
            _ => continue,
        };
        let mut value = value;
        value["snapshot_tick"] = game.meta.snapshot_tick.into();
        value["elapsed_seconds"] = qa.started.elapsed().as_secs_f64().into();
        value["stage"] = qa.stage.into();
        qa.commands.push(value);
    }
}

#[derive(SystemParam)]
struct Scene<'w, 's> {
    player: Query<
        'w,
        's,
        (
            &'static Transform,
            &'static CombatStats,
            &'static Team,
            &'static NetworkPlayerId,
            &'static NetworkHeroClass,
            Option<&'static MovementTarget>,
        ),
        With<Player>,
    >,
    enemies: Query<
        'w,
        's,
        (
            &'static Transform,
            &'static CombatStats,
            &'static Team,
            &'static NetworkPlayerId,
        ),
        Without<Player>,
    >,
    cameras: Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<MainCamera>>,
    environment: Query<'w, 's, Entity, With<VerdantEnvironment>>,
    scenes: Query<'w, 's, (&'static SceneRoot, Option<&'static SceneInstance>)>,
    windows: Query<'w, 's, Entity, With<PrimaryWindow>>,
    window_state: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    mouse: Res<'w, ButtonInput<MouseButton>>,
    pointer: Res<'w, crate::combat::WorldPointerState>,
    minimap: Res<'w, crate::minimap::MinimapNavigationState>,
    buttons: Query<
        'w,
        's,
        (
            Option<&'static Name>,
            &'static Interaction,
            Option<&'static ComputedNode>,
            Option<&'static InheritedVisibility>,
        ),
        With<Button>,
    >,
    candidate_state: Query<
        'w,
        's,
        (
            Entity,
            &'static Transform,
            &'static NetworkPlayerId,
            &'static CombatStats,
            &'static Team,
            Option<&'static InheritedVisibility>,
            Option<&'static Visibility>,
        ),
        Without<Player>,
    >,
    nodes: Query<
        'w,
        's,
        (
            &'static Name,
            &'static ComputedNode,
            &'static UiGlobalTransform,
            &'static InheritedVisibility,
        ),
    >,
}

fn project(scene: &Scene, p: Vec3) -> Option<Vec2> {
    let (camera, transform) = scene.cameras.single().ok()?;
    camera.world_to_viewport(transform, p).ok()
}

/// Observes production gates without changing them or repeating an input.
fn selection_gates(
    scene: &Scene,
    context: &GameplayInputContext,
    mobile: &MobileControls,
    target: &TargetState,
) -> serde_json::Value {
    let window = scene.window_state.single().ok();
    let cursor = window.and_then(Window::cursor_position);
    let buttons: Vec<_> = scene
        .buttons
        .iter()
        .filter(|(_, interaction, _, _)| **interaction != Interaction::None)
        .map(|(name, interaction, node, visible)| {
            serde_json::json!({
                "name":name.map(Name::as_str),"interaction":format!("{interaction:?}"),
                "size":node.map(|n|n.size().to_array()),"inherited_visible":visible.map(|v| v.get())
            })
        })
        .collect();
    let candidates: Vec<_> = scene.candidate_state.iter().map(|(entity,transform,id,stats,team,visible,visibility)| {
        let screen=project(scene,transform.translation);
        serde_json::json!({"entity":format!("{entity:?}"),"id":id.0,"hp":stats.hp,"team":team,
            "inherited_visible":visible.map(|v| v.get()),"visibility":visibility.map(|v|format!("{v:?}")),
            "position":transform.translation.to_array(),"screen":screen.map(|p|p.to_array()),
            "pointer_distance":screen.zip(cursor).map(|(a,b)|a.distance(b))})
    }).collect();
    serde_json::json!({"window_focused":window.map(|w|w.focused),"cursor":cursor.map(|p|p.to_array()),
        "left_just_pressed":scene.mouse.just_pressed(MouseButton::Left),"left_pressed":scene.mouse.pressed(MouseButton::Left),
        "right_just_pressed":scene.mouse.just_pressed(MouseButton::Right),"gameplay_allowed":context.gameplay_allowed(),
        "modal_open":context.modal_open,"mobile_enabled":mobile.enabled,"mobile_focused":mobile.focused,
        "consumed_primary":scene.pointer.consumed_primary_press,"consumed_secondary":scene.pointer.consumed_secondary_press,
        "minimap_consumed_primary":scene.minimap.consumed_primary_click,"blocking_buttons":buttons,"candidates":candidates,
        "selected_target":target.selected_target,"selected_entity":target.selected_entity.map(|e|format!("{e:?}")),
        "camera":scene.cameras.single().ok().map(|(_,transform)|serde_json::json!({"position":transform.translation().to_array(),"rotation":transform.rotation().to_array()}))})
}

#[allow(clippy::too_many_arguments)]
fn observe_targeting(
    mut commands: Commands,
    mut qa: ResMut<TargetingQa>,
    scene: Scene,
    session: Res<ClientSession>,
    game: Res<GameStateSnapshot>,
    help: Res<HelpOverlayVisible>,
    context: Res<GameplayInputContext>,
    mobile: Res<MobileControls>,
    target: Res<TargetState>,
    basic: Res<BasicAttackState>,
    preview: Res<TargetAimPreview>,
    cooldown: Res<LocalCastCooldown>,
    assets: Res<AssetServer>,
    spawner: Res<SceneSpawner>,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.stage == 255 {
        return;
    }
    if qa.started.elapsed() > qa.timeout {
        fail(&mut qa, &mut exit, "bounded scenario timed out");
        return;
    }
    qa.frames += 1;
    let Ok((transform, stats, team, id, class, movement)) = scene.player.single() else {
        return;
    };
    let position = transform.translation;
    let tick = game.meta.snapshot_tick;
    let mut enemies: Vec<_> = scene
        .enemies
        .iter()
        .filter(|(_, s, t, _)| s.is_alive() && *t != team)
        .collect();
    enemies.sort_by_key(|(_, _, _, id)| id.0);
    if qa.stage > 0
        && (!session.join_confirmed()
            || !stats.is_alive()
            || !matches!(game.state, GameState::Running))
    {
        fail(&mut qa, &mut exit, "live match/player precondition lost");
        return;
    }
    if qa.commands.iter().any(|c| c["kind"] == "cast")
        || cooldown.remaining_secs.iter().any(|v| *v > 0.001)
    {
        fail(
            &mut qa,
            &mut exit,
            "basic-only input emitted a skill or changed a skill cooldown",
        );
        return;
    }
    let enemy = enemies
        .iter()
        .find(|(_, _, _, id)| id.0 == qa.target_id)
        .copied();
    if let Some((_, hp, _, _)) = enemy {
        if hp.hp < qa.latest_hp - 0.01 {
            qa.damage_count += 1;
        }
        qa.latest_hp = hp.hp;
    }
    match qa.stage {
        0 => {
            let ready = session.join_confirmed()
                && !help.0
                && context.gameplay_allowed()
                && matches!(game.state, GameState::Running)
                && enemies.len() == 2
                && class.0 == shared::HeroClass::Mage
                && !scene.environment.is_empty()
                && !scene.scenes.is_empty()
                && scene.scenes.iter().all(|(root, instance)| {
                    instance.is_some_and(|i| spawner.instance_is_ready(**i))
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
            qa.player_id = id.0;
            qa.start = position;
            qa.target_id = enemies[0].3.0;
            qa.other_target_ids = enemies.iter().skip(1).map(|(_, _, _, id)| id.0).collect();
            qa.target_hp = enemies[0].1.hp;
            qa.latest_hp = qa.target_hp;
            event!(
                qa,
                "ready",
                tick,
                serde_json::json!({"position":position.to_array(),"enemy_positions":enemies.iter().map(|(t,_,_,id)|serde_json::json!({"id":id.0,"position":t.translation.to_array()})).collect::<Vec<_>>() })
            );
            capture(
                &mut commands,
                &mut qa,
                0,
                &scene,
                &mobile,
                &target,
                &preview,
                &basic,
                tick,
                position,
            );
            qa.advance(1);
        }
        1 => {
            if !qa.readbacks.contains(&0) {
                return;
            }
            let Some((enemy, _, _, _)) = enemy else {
                return;
            };
            let Some(screen) = project(&scene, enemy.translation) else {
                return;
            };
            if mobile.enabled {
                let Some(origin) = project(&scene, position) else {
                    return;
                };
                let vector = screen - origin;
                let reach = (mobile.viewport.y * 0.70).min(mobile.viewport.x * 0.45);
                let extent = (vector.length() / reach).clamp(0.01, 1.0);
                let drag_end = mobile.layout().attack_center
                    + vector.normalize_or_zero() * (12.0 + 84.0 * extent) * mobile.scale();
                qa.drag_end = drag_end;
                qa.touch(11, TouchPhase::Started, mobile.layout().attack_center);
                qa.touch(11, TouchPhase::Moved, drag_end);
            } else {
                qa.click(screen, MouseButton::Left, false);
                event!(
                    qa,
                    "left_select",
                    tick,
                    serde_json::json!({"cursor":screen.to_array()})
                );
            }
            qa.advance(2);
        }
        2 => {
            if qa.frames == 1 {
                event!(
                    qa,
                    "selection_gate_readback",
                    tick,
                    selection_gates(&scene, &context, &mobile, &target)
                );
            }
            if qa.attacks() != 0
                || movement.is_some()
                || position.xz().distance(qa.start.xz()) > 0.1
            {
                fail(
                    &mut qa,
                    &mut exit,
                    "selection or drag preview leaked attack/movement",
                );
                return;
            }
            let expected = Some(TargetId {
                kind: TargetKind::Player,
                id: qa.target_id,
            });
            if mobile.enabled {
                if !preview.active || preview.target != expected {
                    fail(
                        &mut qa,
                        &mut exit,
                        "drag did not preview the exact intended target",
                    );
                    return;
                }
            } else if target.selected_target != expected {
                fail(&mut qa, &mut exit, "LMB did not select the exact target");
                return;
            }
            if qa.stage_started.elapsed() < Duration::from_millis(500) {
                return;
            }
            event!(
                qa,
                if mobile.enabled {
                    "drag_preview"
                } else {
                    "selection_no_attack"
                },
                tick,
                serde_json::json!({"exact_target":expected,"outgoing_attacks":qa.attacks(),"movement_absent":true})
            );
            capture(
                &mut commands,
                &mut qa,
                1,
                &scene,
                &mobile,
                &target,
                &preview,
                &basic,
                tick,
                position,
            );
            qa.advance(3);
        }
        3 => {
            if !qa.readbacks.contains(&1) {
                return;
            }
            event!(
                qa,
                "basic_start",
                tick,
                serde_json::json!({"target_id":qa.target_id,"hp":qa.latest_hp})
            );
            if mobile.enabled {
                let end = qa.drag_end;
                qa.touch(11, TouchPhase::Ended, end);
                event!(
                    qa,
                    "drag_release",
                    tick,
                    serde_json::json!({"touch_id":11,"position":end.to_array()})
                );
            } else {
                let Some((enemy, _, _, _)) = enemy else {
                    return;
                };
                let Some(screen) = project(&scene, enemy.translation) else {
                    return;
                };
                qa.click(screen, MouseButton::Right, false);
                event!(
                    qa,
                    "right_attack",
                    tick,
                    serde_json::json!({"cursor":screen.to_array()})
                );
            }
            qa.advance(4);
        }
        4 => {
            let expected = serde_json::json!({"kind":"player","id":qa.target_id});
            if qa
                .commands
                .iter()
                .any(|c| c["kind"] == "basic_attack" && c["target"] != expected)
            {
                fail(
                    &mut qa,
                    &mut exit,
                    "basic command changed exact target identity",
                );
                return;
            }
            if mobile.enabled && movement.is_some() {
                fail(
                    &mut qa,
                    &mut exit,
                    "phone basic attack started automatic chasing",
                );
                return;
            }
            let required = if mobile.enabled { 1 } else { 2 };
            if qa.damage_count < required || qa.attacks() < required {
                return;
            }
            if target.selected_target
                != Some(TargetId {
                    kind: TargetKind::Player,
                    id: qa.target_id,
                })
            {
                fail(&mut qa, &mut exit, "accepted attack lost selected target");
                return;
            }
            event!(
                qa,
                "basic_damage",
                tick,
                serde_json::json!({"hp_before":qa.target_hp,"hp_after":qa.latest_hp,
                "client_requests":qa.attacks(),"snapshot_hp_drops":qa.damage_count})
            );
            capture(
                &mut commands,
                &mut qa,
                2,
                &scene,
                &mobile,
                &target,
                &preview,
                &basic,
                tick,
                position,
            );
            if mobile.enabled {
                let center = mobile.layout().joystick_center;
                qa.touch(22, TouchPhase::Started, center);
                qa.touch(
                    22,
                    TouchPhase::Moved,
                    center + Vec2::new(-mobile.layout().joystick_radius * 0.6, 0.0),
                );
            } else {
                let Some(p) = project(&scene, position + Vec3::new(-2.5, 0.0, -1.5)) else {
                    return;
                };
                qa.click(p, MouseButton::Right, false);
            }
            qa.advance(5);
        }
        5 => {
            if basic.order.is_some() {
                fail(
                    &mut qa,
                    &mut exit,
                    "explicit movement left basic order active",
                );
                return;
            }
            event!(
                qa,
                "basic_stop",
                tick,
                serde_json::json!({"order_absent":true,"attacks":qa.attacks()})
            );
            event!(
                qa,
                if mobile.enabled {
                    "independent_movement"
                } else {
                    "ground_cancel"
                },
                tick,
                serde_json::json!({"joystick":mobile.movement.to_array(),"movement_target":movement.map(|m|m.target.to_array())})
            );
            if mobile.enabled {
                if mobile.movement.length() < 0.1 {
                    fail(&mut qa, &mut exit, "second finger did not produce movement");
                    return;
                }
                // Capture a different attack finger then cancel it while the movement finger remains owned.
                qa.touch(33, TouchPhase::Started, mobile.layout().attack_center);
                qa.touch(
                    33,
                    TouchPhase::Moved,
                    mobile.layout().attack_center + Vec2::new(-45.0, 0.0),
                );
                qa.advance(6);
            } else {
                qa.mouse = Some(MouseInput::Key(KeyCode::KeyS));
                qa.advance(10);
            }
        }
        6 => {
            if !preview.active || mobile.movement.length() < 0.1 {
                fail(
                    &mut qa,
                    &mut exit,
                    "independent movement/attack fingers lost ownership",
                );
                return;
            }
            qa.touch(
                33,
                TouchPhase::Canceled,
                mobile.layout().attack_center + Vec2::new(-45.0, 0.0),
            );
            qa.touch(22, TouchPhase::Ended, mobile.layout().joystick_center);
            qa.advance(7);
        }
        7 => {
            if preview.active || basic.order.is_some() || mobile.movement != Vec2::ZERO {
                fail(
                    &mut qa,
                    &mut exit,
                    "TouchPhase::Canceled left an active gesture/order",
                );
                return;
            }
            event!(
                qa,
                "touch_canceled",
                tick,
                serde_json::json!({"phase":"Canceled","gesture_absent":true})
            );
            qa.touch(44, TouchPhase::Started, mobile.layout().attack_center);
            qa.touch(
                44,
                TouchPhase::Moved,
                mobile.layout().attack_center + Vec2::new(-90.0, 0.0) * mobile.scale(),
            );
            qa.advance(8);
        }
        8 => {
            if !preview.active || preview.candidate.is_some() {
                fail(
                    &mut qa,
                    &mut exit,
                    "empty directional fixture unexpectedly has a candidate",
                );
                return;
            }
            qa.touch(
                44,
                TouchPhase::Ended,
                mobile.layout().attack_center + Vec2::new(-90.0, 0.0) * mobile.scale(),
            );
            qa.advance(9);
        }
        9 => {
            if preview.active || basic.order.is_some() {
                fail(
                    &mut qa,
                    &mut exit,
                    "empty release left preview or basic order",
                );
                return;
            }
            event!(
                qa,
                "empty_aim_released",
                tick,
                serde_json::json!({"fallback_absent":true})
            );
            qa.advance(15);
        }
        10 => {
            if movement.is_some() {
                return;
            }
            let Some((enemy, _, _, _)) = enemy else {
                return;
            };
            let Some(screen) = project(&scene, enemy.translation) else {
                return;
            };
            qa.click(screen, MouseButton::Right, true);
            qa.advance(11);
        }
        11 => {
            if basic.order.is_some() || movement.is_some() {
                fail(&mut qa, &mut exit, "Alt+RMB leaked attack or movement");
                return;
            }
            event!(
                qa,
                "alt_click_blocked",
                tick,
                serde_json::json!({"order_absent":true,"movement_absent":true})
            );
            let Some((_, node, transform, visible)) = scene
                .nodes
                .iter()
                .find(|(name, _, _, _)| name.as_str() == "ShopOpenButton")
            else {
                return;
            };
            if !visible.get() || node.size().min_element() <= 0.0 {
                return;
            }
            qa.click(transform.translation, MouseButton::Right, false);
            qa.advance(12);
        }
        12 => {
            if basic.order.is_some() || movement.is_some() {
                fail(&mut qa, &mut exit, "UI RMB leaked attack or movement");
                return;
            }
            event!(
                qa,
                "ui_click_blocked",
                tick,
                serde_json::json!({"button":"ShopOpenButton","order_absent":true,"movement_absent":true})
            );
            qa.advance(15);
        }
        15 => {
            if qa.stage_started.elapsed() < Duration::from_secs(2) {
                return;
            }
            let stop = qa
                .events
                .iter()
                .find(|e| e["event"] == "basic_stop")
                .unwrap();
            if qa.attacks() != stop["detail"]["attacks"].as_u64().unwrap() as usize
                || preview.active
                || basic.order.is_some()
            {
                fail(
                    &mut qa,
                    &mut exit,
                    "attack/cancel gesture escaped its permitted input interval",
                );
                return;
            }
            event!(
                qa,
                "cancellation_verified",
                tick,
                serde_json::json!({"order_absent":true,"preview_absent":true,"attacks":qa.attacks()})
            );
            capture(
                &mut commands,
                &mut qa,
                3,
                &scene,
                &mobile,
                &target,
                &preview,
                &basic,
                tick,
                position,
            );
            qa.advance(16);
        }
        16 => {
            if qa.readbacks.len() != FILES.len()
                || FILES
                    .iter()
                    .any(|f| !qa.directory.join(f).metadata().is_ok_and(|m| m.len() > 32))
            {
                return;
            }
            let summary = serde_json::json!({"scenario":"targeting","version":env!("CARGO_PKG_VERSION"),"pass":true,
                "scripted_input":true,"manual_interaction_verified":false,"synthetic_damage":false,
                "setup_fixture":"development server target placement and ambient AI disabled",
                "input_method":if mobile.enabled {"real TouchInput phase messages before InputSystems"}else{"ButtonInput mouse press/release with actual logical window cursor"},
                "admission":"ordinary Mage class/team/help button handlers", "player_id":qa.player_id,
                "target_id":qa.target_id,"other_target_ids":qa.other_target_ids,"events":qa.events,"commands":qa.commands,
                "skill_cooldowns":cooldown.remaining_secs,"captures":qa.captures,"pixels":[qa.width,qa.height]});
            if std::fs::write(
                qa.directory.join("qa-summary.json"),
                serde_json::to_vec_pretty(&summary).unwrap(),
            )
            .is_err()
            {
                fail(&mut qa, &mut exit, "cannot write summary");
                return;
            }
            info!("TARGETING_QA completed");
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
struct Shot(usize);
#[allow(clippy::too_many_arguments)]
fn capture(
    commands: &mut Commands,
    qa: &mut TargetingQa,
    index: usize,
    scene: &Scene,
    mobile: &MobileControls,
    target: &TargetState,
    preview: &TargetAimPreview,
    basic: &BasicAttackState,
    tick: u64,
    position: Vec3,
) {
    let nodes: Vec<_> = scene
        .nodes
        .iter()
        .filter(|(name, _, _, _)| {
            matches!(
                name.as_str(),
                "MobileAttack"
                    | "MobileAbility-0"
                    | "MobileAbility-1"
                    | "MobileAbility-2"
                    | "MobileAbility-3"
                    | "TargetAimVector"
                    | "TargetAimReticle"
                    | "TargetAimCandidate"
                    | "LockedTargetIndicator"
            )
        })
        .map(|(name, node, transform, visible)| {
            serde_json::json!({"name":name.as_str(),"size":node.size().to_array(),
            "center":transform.translation.to_array(),"visible":visible.get()})
        })
        .collect();
    qa.captures.push(serde_json::json!({"stage":index,"file":FILES[index],"pixels":[qa.width,qa.height],
        "mobile_controls":mobile.enabled,"snapshot_tick":tick,"position":position.to_array(),
        "selected_target":target.selected_target,"basic_order":basic.order.map(|o|o.target),
        "preview":{"active":preview.active,"target":preview.target,"origin":preview.origin.to_array(),
            "cursor":preview.cursor.to_array(),"candidate_screen":preview.candidate_screen.map(|p|p.to_array())},"nodes":nodes}));
    commands
        .spawn((Screenshot::primary_window(), Shot(index)))
        .observe(save_to_disk(qa.directory.join(FILES[index])))
        .observe(readback);
}
fn readback(captured: On<ScreenshotCaptured>, shots: Query<&Shot>, mut qa: ResMut<TargetingQa>) {
    if let Ok(shot) = shots.get(captured.entity) {
        if captured.image.width() == qa.width
            && captured.image.height() == qa.height
            && !qa.readbacks.contains(&shot.0)
        {
            qa.readbacks.push(shot.0);
        }
    }
}
fn fail(qa: &mut TargetingQa, exit: &mut MessageWriter<AppExit>, reason: &str) {
    error!("TARGETING_QA failed stage={}: {reason}", qa.stage);
    let _ = std::fs::write(
        qa.directory.join("qa-failure.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
        "stage":qa.stage,"reason":reason,"events":qa.events,"commands":qa.commands}))
        .unwrap(),
    );
    qa.stage = 255;
    exit.write(AppExit::error());
}
