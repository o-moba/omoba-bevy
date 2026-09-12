//! Opt-in native proof: ordinary scripted commands, live projectiles and
//! confirmed server events. No synthetic hits, paused VFX or device claims.
use crate::{
    camera::MainCamera,
    combat::CombatStats,
    combat_feedback::DamageNumber,
    combat_visuals::ProjectilePresentationRoot,
    help_overlay::HelpOverlayVisible,
    mobile_controls::MobileControls,
    net::{
        ClientSession, GameState, GameStateSnapshot, NetworkCommand, NetworkHeroClass,
        NetworkMinionAction, NetworkMinionId, NetworkMinionKind, NetworkPlayerId,
        NetworkProjectile, TargetId, TargetKind,
    },
    player::{MovementTarget, Player},
    sprite::PlayerVisualMode,
    team::{Team, TeamSelection},
    world2d::{layer, simulation_xz_to_render_xy},
};
use bevy::{
    app::AppExit,
    asset::RecursiveDependencyLoadState,
    ecs::system::SystemParam,
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    scene::{SceneInstance, SceneSpawner},
    ui::FocusPolicy,
    window::PrimaryWindow,
};
use shared::{
    HeroClass,
    combat::{CombatEntityKind, ProjectileStyle},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

const FILES: [&str; 3] = [
    "01-combat-ready.png",
    "02-projectile-flight.png",
    "03-confirmed-impact.png",
];
pub(crate) struct CombatQaPlugin;
impl Plugin for CombatQaPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var("OMOBA_VISUAL_QA_SCENARIO").as_deref() != Ok("combat") {
            return;
        }
        let Some(directory) = std::env::var_os("OMOBA_VISUAL_QA_DIR").map(PathBuf::from) else {
            return;
        };
        let class = std::env::var("OMOBA_COMBAT_QA_CLASS")
            .ok()
            .and_then(|v| HeroClass::from_id(&v))
            .unwrap_or(HeroClass::Ranger);
        let pixels = |key: &str, fallback: u32| {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(fallback)
        };
        app.insert_resource(CombatQa {
            directory,
            class,
            waves: std::env::var("OMOBA_COMBAT_QA_WAVES").as_deref() == Ok("1"),
            started: Instant::now(),
            timeout: Duration::from_secs(
                pixels("OMOBA_VISUAL_QA_TIMEOUT", 120).clamp(30, 600) as u64
            ),
            pixels: UVec2::new(
                pixels("OMOBA_QA_WIDTH", 1280),
                pixels("OMOBA_QA_HEIGHT", 720),
            ),
            stage: 0,
            settled: 0,
            own_id: 0,
            target_id: 0,
            captures: Vec::new(),
            readbacks: Vec::new(),
            requests: Vec::new(),
        })
        .add_systems(Startup, label)
        .add_systems(PreUpdate, prepare.after(bevy::ui::UiSystems::Focus))
        .add_systems(
            PostUpdate,
            observe
                .after(bevy::transform::TransformSystems::Propagate)
                .after(bevy::ui::UiSystems::Layout),
        );
    }
}

#[derive(Resource)]
struct CombatQa {
    directory: PathBuf,
    class: HeroClass,
    waves: bool,
    started: Instant,
    timeout: Duration,
    pixels: UVec2,
    stage: u8,
    settled: u32,
    own_id: u64,
    target_id: u64,
    captures: Vec<serde_json::Value>,
    readbacks: Vec<usize>,
    requests: Vec<serde_json::Value>,
}
fn label(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(33.0),
            bottom: Val::Px(3.0),
            ..default()
        },
        Text::new("QA: scripted commands · live server combat"),
        TextFont {
            font_size: 10.0,
            ..default()
        },
        TextColor(Color::WHITE),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.75)),
        FocusPolicy::Pass,
        ZIndex(200),
    ));
}
fn prepare(
    qa: Res<CombatQa>,
    session: Res<ClientSession>,
    selection: Res<TeamSelection>,
    help: Res<HelpOverlayVisible>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut buttons: Query<(&Name, &mut Interaction), With<Button>>,
) {
    if let Ok(mut window) = windows.single_mut() {
        window.resolution.set_scale_factor_override(Some(1.0));
        if window.physical_width() != qa.pixels.x || window.physical_height() != qa.pixels.y {
            window
                .resolution
                .set_physical_resolution(qa.pixels.x, qa.pixels.y);
        }
    }
    if qa.stage != 0 {
        return;
    }
    let class_button = format!("ClassButton-{}", qa.class.id());
    for (name, mut interaction) in &mut buttons {
        if (session.is_connected()
            && !session.join_confirmed()
            && name.as_str()
                == if selection.hero_class == qa.class {
                    "TeamGreenButton"
                } else {
                    class_button.as_str()
                })
            || (session.join_confirmed() && help.0 && name.as_str() == "HelpDismissButton")
        {
            *interaction = Interaction::Pressed;
        }
    }
}

#[derive(SystemParam)]
struct Scene<'w, 's> {
    local: Query<
        'w,
        's,
        (
            Entity,
            &'static Transform,
            &'static CombatStats,
            &'static NetworkPlayerId,
            &'static NetworkHeroClass,
        ),
        With<Player>,
    >,
    enemies: Query<
        'w,
        's,
        (
            &'static Transform,
            &'static CombatStats,
            &'static NetworkPlayerId,
            &'static Team,
        ),
        Without<Player>,
    >,
    projectiles: Query<'w, 's, (Entity, &'static Transform, &'static NetworkProjectile)>,
    presentations: Query<'w, 's, (Entity, &'static ProjectilePresentationRoot)>,
    children: Query<'w, 's, &'static Children>,
    drawables: Query<
        'w,
        's,
        (
            &'static GlobalTransform,
            &'static InheritedVisibility,
            Option<&'static Mesh3d>,
            Option<&'static Sprite>,
        ),
    >,
    minions: Query<
        'w,
        's,
        (
            &'static Transform,
            &'static NetworkMinionId,
            &'static NetworkMinionKind,
            &'static NetworkMinionAction,
            &'static Team,
        ),
    >,
    numbers: Query<
        'w,
        's,
        (
            &'static DamageNumber,
            &'static Text,
            &'static ComputedNode,
            &'static UiGlobalTransform,
            &'static InheritedVisibility,
        ),
    >,
    nodes: Query<
        'w,
        's,
        (
            &'static Name,
            &'static ComputedNode,
            &'static InheritedVisibility,
        ),
    >,
    cameras: Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<MainCamera>>,
    scenes: Query<'w, 's, (&'static SceneRoot, Option<&'static SceneInstance>)>,
    sprites: Query<'w, 's, &'static Sprite>,
    windows: Query<'w, 's, Entity, With<PrimaryWindow>>,
}
fn on_screen(scene: &Scene, mode: PlayerVisualMode, position: Vec3) -> Option<Vec2> {
    let (camera, transform) = scene.cameras.single().ok()?;
    let position = if mode == PlayerVisualMode::Sprite2d {
        simulation_xz_to_render_xy(position).extend(layer::PROJECTILE)
    } else {
        position
    };
    let p = camera.world_to_viewport(transform, position).ok()?;
    let size = camera.logical_viewport_size()?;
    (p.x > 8.0 && p.y > 8.0 && p.x < size.x - 8.0 && p.y < size.y - 8.0).then_some(p)
}

fn visible_projectile_parts(scene: &Scene, owner: Entity) -> usize {
    let Some((root, _)) = scene
        .presentations
        .iter()
        .find(|(_, presentation)| presentation.owner == owner)
    else {
        return 0;
    };
    let Ok((camera, camera_transform)) = scene.cameras.single() else {
        return 0;
    };
    let Some(viewport) = camera.logical_viewport_size() else {
        return 0;
    };
    std::iter::once(root)
        .chain(scene.children.iter_descendants(root))
        .filter(|entity| {
            let Ok((transform, visible, mesh, sprite)) = scene.drawables.get(*entity) else {
                return false;
            };
            if !visible.get() || (mesh.is_none() && !sprite.is_some_and(|s| s.color.alpha() > 0.0))
            {
                return false;
            }
            camera
                .world_to_viewport(camera_transform, transform.translation())
                .is_ok_and(|p| p.x > 0.0 && p.y > 0.0 && p.x < viewport.x && p.y < viewport.y)
        })
        .count()
}

#[allow(clippy::too_many_arguments)]
fn observe(
    mut commands: Commands,
    mut qa: ResMut<CombatQa>,
    scene: Scene,
    snapshot: Res<GameStateSnapshot>,
    session: Res<ClientSession>,
    help: Res<HelpOverlayVisible>,
    mobile: Res<MobileControls>,
    mode: Res<PlayerVisualMode>,
    assets: Res<AssetServer>,
    spawner: Res<SceneSpawner>,
    mut outgoing: MessageWriter<NetworkCommand>,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.stage == 255 {
        return;
    }
    if qa.started.elapsed() > qa.timeout {
        fail(&mut qa, &mut exit, "bounded combat scenario timed out");
        return;
    }
    let Ok((entity, position, stats, own_id, class)) = scene.local.single() else {
        return;
    };
    if !session.join_confirmed()
        || !matches!(snapshot.state, GameState::Running)
        || help.0
        || stats.hp <= 0.0
    {
        return;
    }
    if class.0 != qa.class {
        fail(
            &mut qa,
            &mut exit,
            "joined class differs from requested class",
        );
        return;
    }
    let expected = if qa.waves {
        ProjectileStyle::CasterBolt
    } else {
        ProjectileStyle::for_class(qa.class)
    };
    let minions: Vec<_> = scene.minions.iter().filter_map(|(t, id, kind, action, team)| {
        on_screen(&scene, *mode, t.translation).map(|p| serde_json::json!({"id":id.0,"kind":kind.0,"team":team,"attack_sequence":action.0,"screen":p.to_array(),"position":t.translation.to_array()}))
    }).collect();
    let projectiles: Vec<_> = scene.projectiles.iter().filter(|(_, _, projectile)| projectile.style == expected &&
        if qa.waves { projectile.source_kind == CombatEntityKind::Minion } else { projectile.source_kind == CombatEntityKind::Player && projectile.owner_id == own_id.0 })
        .filter(|(entity, _, _)| visible_projectile_parts(&scene, *entity) > 0)
        .filter_map(|(entity, t, projectile)| on_screen(&scene, *mode, t.translation).map(|p| serde_json::json!({
            "entity":format!("{entity:?}"),"rendered_drawables":visible_projectile_parts(&scene, entity),"id":projectile.id,"owner_id":projectile.owner_id,"source_kind":projectile.source_kind,
            "style":projectile.style,"action_slot":projectile.action_slot,"position":t.translation.to_array(),"screen":p.to_array()}))).collect();
    let events: Vec<_> = snapshot
        .combat_events
        .iter()
        .filter(|event| {
            event.amount > 0.0
                && event.style == expected
                && if qa.waves {
                    event.source.kind == CombatEntityKind::Minion
                } else {
                    event.source.kind == CombatEntityKind::Player
                        && event.source.id == own_id.0
                        && event.target.id == qa.target_id
                        && event.target.kind == CombatEntityKind::Player
                }
        })
        .cloned()
        .collect();
    let numbers: Vec<_> = scene.numbers.iter().filter(|(number, _, node, _, visible)| visible.get() && node.size().min_element() > 0.0 && events.iter().any(|event| event.id == number.event_id))
        .map(|(number, text, node, transform, _)| serde_json::json!({"event_id":number.event_id,"text":text.0,"size":node.size().to_array(),"center":transform.translation.to_array(),"visible":true})).collect();
    let nodes: Vec<_> = scene.nodes.iter().filter(|(name,_,_)| matches!(name.as_str(),"MobileJoystick"|"MobileAttack"|"MobileAbility-0"|"MobileAbility-1"|"MobileAbility-2"|"MobileAbility-3"|"ShopOpenButton"))
        .map(|(name,node,visible)| serde_json::json!({"name":name.as_str(),"size":node.size().to_array(),"visible":visible.get()})).collect();
    let frame = serde_json::json!({"snapshot_tick":snapshot.meta.snapshot_tick,"server_epoch":snapshot.meta.server_epoch,"match_id":snapshot.meta.match_id,
        "mobile_controls":mobile.enabled,"visual_mode":format!("{:?}", *mode),"player_position":position.translation.to_array(),
        "projectiles":projectiles,"combat_events":events,"damage_numbers":numbers,"minions":minions,"nodes":nodes});
    match qa.stage {
        0 => {
            let models_ready = *mode == PlayerVisualMode::Sprite2d
                || (!scene.scenes.is_empty()
                    && scene.scenes.iter().all(|(root, instance)| {
                        matches!(
                            assets.get_recursive_dependency_load_state(root.0.id()),
                            Some(RecursiveDependencyLoadState::Loaded)
                        ) && instance.is_some_and(|instance| spawner.instance_is_ready(**instance))
                    }));
            let sprites_ready = scene.sprites.iter().all(|sprite| {
                sprite.image == Handle::default()
                    || matches!(
                        assets.get_recursive_dependency_load_state(sprite.image.id()),
                        Some(RecursiveDependencyLoadState::Loaded)
                    )
            });
            if !models_ready || !sprites_ready {
                qa.settled = 0;
                return;
            }
            qa.settled += 1;
            if qa.settled < 45 {
                return;
            }
            qa.own_id = own_id.0;
            if qa.waves {
                if !minions.iter().any(|m| m["kind"] == "melee")
                    || !minions.iter().any(|m| m["kind"] == "caster")
                {
                    return;
                }
            } else {
                let Some((_, _, target_id, _)) = scene.enemies.iter().find(|(t, s, _, team)| {
                    **team == Team::Blue
                        && s.hp > 0.0
                        && t.translation.distance(position.translation) < 9.5
                }) else {
                    return;
                };
                qa.target_id = target_id.0;
            }
            capture(&mut commands, &mut qa, 0, frame);
            qa.stage = 1;
        }
        1 => {
            if !qa.readbacks.contains(&0) {
                return;
            }
            if qa.waves {
                commands
                    .entity(entity)
                    .insert(MovementTarget { target: Vec3::ZERO });
                qa.requests.push(serde_json::json!({"kind":"navigation","destination":[0,0,0],"snapshot_tick":snapshot.meta.snapshot_tick}));
            } else {
                let target = TargetId {
                    kind: TargetKind::Player,
                    id: qa.target_id,
                };
                outgoing.write(NetworkCommand::Cast { target, slot: 0 });
                qa.requests.push(serde_json::json!({"kind":"cast","slot":0,"target":target,"snapshot_tick":snapshot.meta.snapshot_tick}));
            }
            qa.stage = 2;
        }
        2 if !projectiles.is_empty() => {
            capture(&mut commands, &mut qa, 1, frame);
            qa.stage = 3;
        }
        3 if !numbers.is_empty() => {
            capture(&mut commands, &mut qa, 2, frame);
            qa.stage = 4;
        }
        4 if qa.readbacks.len() == FILES.len()
            && FILES
                .iter()
                .all(|f| qa.directory.join(f).metadata().is_ok_and(|m| m.len() > 32)) =>
        {
            let summary = serde_json::json!({"scenario":"combat","version":env!("CARGO_PKG_VERSION"),"pass":true,"class":qa.class.id(),"waves":qa.waves,
                "scripted_commands":true,"synthetic_damage":false,"manual_interaction_verified":false,"physical_device_verified":false,
                "setup_fixture":if qa.waves {"normal development waves; production route to midlane"} else {"development server initial player placement; ambient AI disabled"},
                "player_id":qa.own_id,"target_id":qa.target_id,"pixels":qa.pixels.to_array(),"captures":qa.captures,"requests":qa.requests});
            if std::fs::write(
                qa.directory.join("qa-summary.json"),
                serde_json::to_vec_pretty(&summary).unwrap(),
            )
            .is_err()
            {
                fail(&mut qa, &mut exit, "cannot write summary");
                return;
            }
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
fn capture(commands: &mut Commands, qa: &mut CombatQa, index: usize, mut frame: serde_json::Value) {
    frame["stage"] = index.into();
    frame["file"] = FILES[index].into();
    frame["pixels"] = serde_json::json!(qa.pixels.to_array());
    qa.captures.push(frame);
    commands
        .spawn((Screenshot::primary_window(), Shot(index)))
        .observe(save_to_disk(qa.directory.join(FILES[index])))
        .observe(readback);
}
fn readback(captured: On<ScreenshotCaptured>, shots: Query<&Shot>, mut qa: ResMut<CombatQa>) {
    if let Ok(shot) = shots.get(captured.entity)
        && captured.image.width() == qa.pixels.x
        && captured.image.height() == qa.pixels.y
        && !qa.readbacks.contains(&shot.0)
    {
        qa.readbacks.push(shot.0);
    }
}
fn fail(qa: &mut CombatQa, exit: &mut MessageWriter<AppExit>, reason: &str) {
    let _ = std::fs::write(qa.directory.join("qa-failure.json"),serde_json::to_vec_pretty(&serde_json::json!({"stage":qa.stage,"reason":reason,"captures":qa.captures,"requests":qa.requests})).unwrap());
    error!("COMBAT_QA failed stage={}: {reason}", qa.stage);
    qa.stage = 255;
    exit.write(AppExit::error());
}
