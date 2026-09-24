//! Opt-in native pickup lifecycle proof using normal movement and authoritative HP.
use crate::{
    camera::MainCamera,
    combat::CombatStats,
    game_vfx::{ButterflyWing, ParticleSlot},
    help_overlay::HelpOverlayVisible,
    mobile_controls::MobileControls,
    net::{ClientSession, GameState, GameStateSnapshot, NetworkCommand, NetworkPlayerId},
    player::{MovementTarget, Player},
    sprite::PlayerVisualMode,
    team::{CharacterChoice, Team, TeamSelection},
};
use bevy::{
    app::AppExit,
    ecs::system::{NonSendMarker, SystemParam},
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    window::PrimaryWindow,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

const FILES: [&str; 4] = [
    "01-butterfly-available.png",
    "02-butterfly-flapping.png",
    "03-butterfly-collected.png",
    "04-butterfly-respawned.png",
];
pub(crate) struct ForestPickupQaPlugin;
impl Plugin for ForestPickupQaPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var("OMOBA_VISUAL_QA_SCENARIO").as_deref() != Ok("forest-pickups") {
            return;
        }
        let Some(directory) = std::env::var_os("OMOBA_VISUAL_QA_DIR").map(PathBuf::from) else {
            return;
        };
        let number = |key: &str, fallback| {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(fallback)
        };
        app.insert_resource(PickupQa {
            directory,
            started: Instant::now(),
            stage_at: Instant::now(),
            timeout: Duration::from_secs(
                number("OMOBA_VISUAL_QA_TIMEOUT", 120).clamp(45, 600) as u64
            ),
            pixels: UVec2::new(
                number("OMOBA_QA_WIDTH", 1280),
                number("OMOBA_QA_HEIGHT", 720),
            ),
            stage: 0,
            captures: vec![],
            requests: vec![],
            readbacks: vec![],
            own_id: 0,
        })
        .add_systems(
            PreUpdate,
            (focus_window, prepare.after(bevy::ui::UiSystems::Focus)),
        )
        .add_systems(
            PostUpdate,
            observe
                .after(bevy::transform::TransformSystems::Propagate)
                .after(bevy::ui::UiSystems::Layout)
                .after(crate::game_vfx::VfxPresentation)
                .after(bevy::camera::visibility::VisibilitySystems::CheckVisibility),
        );
    }
}
#[derive(Resource)]
struct PickupQa {
    directory: PathBuf,
    started: Instant,
    stage_at: Instant,
    timeout: Duration,
    pixels: UVec2,
    stage: u8,
    captures: Vec<serde_json::Value>,
    requests: Vec<serde_json::Value>,
    readbacks: Vec<usize>,
    own_id: u64,
}
fn focus_window(
    windows: Query<Entity, With<PrimaryWindow>>,
    mut retry: Local<(u8, Option<Instant>)>,
    _main: NonSendMarker,
) {
    if retry.0 >= 3
        || retry
            .1
            .is_some_and(|at| at.elapsed() < Duration::from_secs(2))
    {
        return;
    }
    let Ok(entity) = windows.single() else {
        return;
    };
    bevy::winit::WINIT_WINDOWS.with_borrow(|windows| {
        if let Some(window) = windows.get_window(entity)
            && !window.has_focus()
        {
            window.focus_window();
            retry.0 += 1;
            retry.1 = Some(Instant::now());
        }
    });
}
fn prepare(
    qa: Res<PickupQa>,
    session: Res<ClientSession>,
    mut selection: ResMut<TeamSelection>,
    mut outgoing: MessageWriter<NetworkCommand>,
    mut joined: Local<bool>,
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
    if session.is_connected() && !session.join_confirmed() && !*joined {
        *joined = true;
        selection.team = Some(Team::Green);
        selection.hero_class = shared::HeroClass::Warrior;
        selection.character = CharacterChoice::Cube;
        selection.avatar = Some("agnes".into());
        outgoing.write(NetworkCommand::Join {
            team: Team::Green,
            character: CharacterChoice::Cube,
            hero_class: shared::HeroClass::Warrior,
            avatar: Some("agnes".into()),
            sprite_character: None,
        });
    }
    for (name, mut interaction) in &mut buttons {
        if session.join_confirmed() && help.0 && name.as_str() == "HelpDismissButton" {
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
        ),
        With<Player>,
    >,
    wings: Query<
        'w,
        's,
        (
            Entity,
            &'static ButterflyWing,
            &'static GlobalTransform,
            &'static ViewVisibility,
            Option<&'static Mesh3d>,
            Option<&'static Mesh2d>,
        ),
    >,
    particles: Query<'w, 's, (&'static ParticleSlot, &'static ViewVisibility)>,
    cameras: Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<MainCamera>>,
    nodes: Query<
        'w,
        's,
        (
            &'static Name,
            &'static ComputedNode,
            &'static InheritedVisibility,
        ),
    >,
    windows: Query<'w, 's, Entity, With<PrimaryWindow>>,
}
fn navigate(
    commands: &mut Commands,
    qa: &mut PickupQa,
    entity: Entity,
    destination: Vec3,
    tick: u64,
) {
    commands.entity(entity).insert(MovementTarget {
        target: destination,
    });
    qa.requests.push(serde_json::json!({"kind":"navigation", "destination":destination.to_array(), "snapshot_tick":tick}));
}
#[allow(clippy::too_many_arguments)]
fn observe(
    mut commands: Commands,
    mut qa: ResMut<PickupQa>,
    scene: Scene,
    snapshot: Res<GameStateSnapshot>,
    session: Res<ClientSession>,
    help: Res<HelpOverlayVisible>,
    mobile: Res<MobileControls>,
    mode: Res<PlayerVisualMode>,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.stage == 255 {
        return;
    }
    if qa.started.elapsed() > qa.timeout {
        fail(&mut qa, &mut exit, "bounded pickup scenario timed out");
        return;
    }
    let Ok((entity, pose, stats, own)) = scene.local.single() else {
        return;
    };
    if !session.join_confirmed()
        || !matches!(snapshot.state, GameState::Running)
        || help.0
        || stats.hp <= 0.0
    {
        return;
    }
    let Some(pickup) = snapshot.forest_pickups.first() else {
        return;
    };
    let anchor = Vec3::new(pickup.position[0], pose.translation.y, pickup.position[1]);
    let outside = anchor + Vec3::X * 3.0;
    let wings: Vec<_> = scene.wings.iter().filter(|(_, wing, _, _, _, _)| wing.pickup_id == pickup.id)
        .map(|(entity, _, transform, visibility, mesh3d, mesh2d)| {
            let screen = scene.cameras.single().ok().and_then(|(camera, pose)| camera.world_to_viewport(pose, transform.translation()).ok());
            let on_screen = screen.is_some_and(|p| p.x > 0.0 && p.y > 0.0 && p.x < qa.pixels.x as f32 && p.y < qa.pixels.y as f32);
            serde_json::json!({"entity":format!("{entity:?}"), "pickup_id":pickup.id,
                "position":transform.translation().to_array(), "rotation":transform.rotation().to_array(),
                "screen":screen.map(|p|p.to_array()), "visible":visibility.get(), "on_screen":on_screen,
                "drawable":if *mode == PlayerVisualMode::Models3d { mesh3d.is_some() } else { mesh2d.is_some() }})
        }).collect();
    let visible = wings
        .iter()
        .filter(|w| w["visible"] == true && w["on_screen"] == true && w["drawable"] == true)
        .count();
    let particles: Vec<_> = scene
        .particles
        .iter()
        .filter(|(_, v)| v.get())
        .filter_map(|(p, _)| p.sample())
        .filter(|(id, _)| *id == u64::MAX - pickup.id)
        .map(|(id, age)| serde_json::json!({"event_id":id,"age":age}))
        .collect();
    let nodes: Vec<_> = scene.nodes.iter().filter(|(n,_,_)| n.as_str().starts_with("Mobile"))
        .map(|(n, node, visibility)|serde_json::json!({"name":n.as_str(),"size":node.size().to_array(),"visible":visibility.get()})).collect();
    let frame = serde_json::json!({"snapshot_tick":snapshot.meta.snapshot_tick,"server_epoch":snapshot.meta.server_epoch,"match_id":snapshot.meta.match_id,
        "player_id":own.0,"player_position":pose.translation.to_array(),"hp":stats.hp,"max_hp":stats.max_hp,
        "pickup":pickup,"wings":wings,"particles":particles,"nodes":nodes,"visual_mode":format!("{:?}",*mode),"mobile_controls":mobile.enabled,
        "elapsed_seconds":qa.started.elapsed().as_secs_f64()});
    match qa.stage {
        0 if stats.hp < stats.max_hp => {
            qa.own_id = own.0;
            navigate(
                &mut commands,
                &mut qa,
                entity,
                outside,
                snapshot.meta.snapshot_tick,
            );
            qa.stage = 1;
        }
        1 if pose.translation.xz().distance(outside.xz()) < 0.35
            && visible >= 2
            && pickup.available =>
        {
            // Camera and downloaded avatar have several seconds to settle after the production route.
            qa.stage_at = Instant::now();
            qa.stage = 2;
        }
        2 if qa.stage_at.elapsed() > Duration::from_secs(3) && visible >= 2 => {
            capture(&mut commands, &mut qa, 0, frame);
            qa.stage_at = Instant::now();
            qa.stage = 3;
        }
        3 if qa.readbacks.contains(&0)
            && qa.stage_at.elapsed() > Duration::from_millis(370)
            && visible >= 2 =>
        {
            capture(&mut commands, &mut qa, 1, frame);
            qa.stage = 4;
        }
        4 if qa.readbacks.contains(&1) => {
            navigate(
                &mut commands,
                &mut qa,
                entity,
                anchor,
                snapshot.meta.snapshot_tick,
            );
            qa.stage = 5;
        }
        5 if !pickup.available
            && pickup.last_collector_id == Some(own.0)
            && visible == 0
            && particles
                .iter()
                .any(|p| p["age"].as_f64().is_some_and(|age| age >= 0.06)) =>
        {
            capture(&mut commands, &mut qa, 2, frame);
            qa.stage = 6;
        }
        6 if qa.readbacks.contains(&2) => {
            navigate(
                &mut commands,
                &mut qa,
                entity,
                outside,
                snapshot.meta.snapshot_tick,
            );
            qa.stage = 7;
        }
        7 if pickup.available
            && visible >= 2
            && pose.translation.xz().distance(outside.xz()) < 0.35 =>
        {
            capture(&mut commands, &mut qa, 3, frame);
            qa.stage = 8;
        }
        8 if qa.readbacks.len() == FILES.len()
            && FILES
                .iter()
                .all(|f| qa.directory.join(f).metadata().is_ok_and(|m| m.len() > 32)) =>
        {
            let summary = serde_json::json!({"scenario":"forest-pickups","pass":true,"version":env!("CARGO_PKG_VERSION"),
                "scripted_commands":true,"synthetic_damage":false,"synthetic_healing":false,"manual_interaction_verified":false,"physical_device_verified":false,
                "setup_fixture":"development server initial player placement; ambient AI disabled", "player_id":qa.own_id,
                "pixels":qa.pixels.to_array(),"captures":qa.captures,"requests":qa.requests,"readbacks":qa.readbacks});
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
fn capture(commands: &mut Commands, qa: &mut PickupQa, index: usize, mut frame: serde_json::Value) {
    frame["stage"] = index.into();
    frame["file"] = FILES[index].into();
    frame["pixels"] = serde_json::json!(qa.pixels.to_array());
    qa.captures.push(frame);
    commands
        .spawn((Screenshot::primary_window(), Shot(index)))
        .observe(save_to_disk(qa.directory.join(FILES[index])))
        .observe(readback);
}
fn readback(captured: On<ScreenshotCaptured>, shots: Query<&Shot>, mut qa: ResMut<PickupQa>) {
    if let Ok(shot) = shots.get(captured.entity)
        && captured.image.width() == qa.pixels.x
        && captured.image.height() == qa.pixels.y
        && !qa.readbacks.contains(&shot.0)
    {
        qa.readbacks.push(shot.0);
    }
}
fn fail(qa: &mut PickupQa, exit: &mut MessageWriter<AppExit>, reason: &str) {
    let _ = std::fs::write(qa.directory.join("qa-failure.json"),serde_json::to_vec_pretty(&serde_json::json!({"stage":qa.stage,"reason":reason,"captures":qa.captures,"requests":qa.requests})).unwrap());
    error!("FOREST_PICKUP_QA failed stage={}: {reason}", qa.stage);
    qa.stage = 255;
    exit.write(AppExit::error());
}
