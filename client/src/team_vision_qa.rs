//! Opt-in native visibility proof using real movement and authoritative snapshots.
use crate::{
    camera::MainCamera,
    combat::TargetState,
    help_overlay::HelpOverlayVisible,
    minimap::MinimapQaScene,
    mobile_controls::MobileControls,
    net::{
        ClientSession, GameStateSnapshot, NetworkCommand, NetworkPlayerId, TargetId, TargetKind,
    },
    persistence::ClientSessionId,
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

const FILES: [&str; 6] = [
    "01-visible-outside-brush.png",
    "02-hidden-in-brush.png",
    "03-same-brush-reveal.png",
    "04-concealed-again.png",
    "05-exited-brush.png",
    "06-outside-team-sight.png",
];
pub(crate) struct TeamVisionQaPlugin;
impl Plugin for TeamVisionQaPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var("OMOBA_VISUAL_QA_SCENARIO").as_deref() != Ok("team-vision") {
            return;
        }
        let Some(directory) = std::env::var_os("OMOBA_VISUAL_QA_DIR").map(PathBuf::from) else {
            return;
        };
        let value = |key: &str, fallback| {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(fallback)
        };
        app.insert_resource(VisionQa {
            directory,
            started: Instant::now(),
            timeout: Duration::from_secs(value("OMOBA_VISUAL_QA_TIMEOUT", 110) as u64),
            pixels: UVec2::new(value("OMOBA_QA_WIDTH", 1280), value("OMOBA_QA_HEIGHT", 720)),
            stage: 0,
            settled: 0,
            target_id: 0,
            old_descendants: Vec::new(),
            captures: Vec::new(),
            readbacks: Vec::new(),
        })
        .add_systems(
            PreUpdate,
            (
                focus_capture_window,
                prepare.after(bevy::ui::UiSystems::Focus),
            ),
        )
        .add_systems(
            PostUpdate,
            observe
                .after(bevy::transform::TransformSystems::Propagate)
                .after(bevy::ui::UiSystems::Layout)
                .after(bevy::camera::visibility::VisibilitySystems::CheckVisibility),
        );
    }
}
#[derive(Resource)]
struct VisionQa {
    directory: PathBuf,
    started: Instant,
    timeout: Duration,
    pixels: UVec2,
    stage: usize,
    settled: u32,
    target_id: u64,
    old_descendants: Vec<Entity>,
    captures: Vec<serde_json::Value>,
    readbacks: Vec<usize>,
}
fn prepare(
    qa: Res<VisionQa>,
    session: Res<ClientSession>,
    mut identity: ResMut<ClientSessionId>,
    mut selection: ResMut<TeamSelection>,
    mut outgoing: MessageWriter<NetworkCommand>,
    mut joined: Local<bool>,
    help: Res<HelpOverlayVisible>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut buttons: Query<(&Name, &mut Interaction), With<Button>>,
) {
    if let Ok(mut w) = windows.single_mut() {
        w.resolution.set_scale_factor_override(Some(1.0));
        if w.physical_width() != qa.pixels.x || w.physical_height() != qa.pixels.y {
            w.resolution
                .set_physical_resolution(qa.pixels.x, qa.pixels.y);
        }
    }
    if session.is_connected() && !session.join_confirmed() && !*joined {
        *joined = true;
        identity.0 = "vision-qa-local".into();
        selection.team = Some(Team::Green);
        selection.hero_class = shared::HeroClass::Ranger;
        selection.character = CharacterChoice::Cube;
        selection.avatar = Some("agnes".into());
        outgoing.write(NetworkCommand::Join {
            team: Team::Green,
            character: CharacterChoice::Cube,
            hero_class: shared::HeroClass::Ranger,
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
// Focus the real OS window; never bypass gameplay focus checks.
fn focus_capture_window(
    windows: Query<Entity, With<PrimaryWindow>>,
    mut retry: Local<(u8, Option<Instant>)>,
    _main: NonSendMarker,
) {
    if retry.0 >= 3
        || retry
            .1
            .is_some_and(|t| t.elapsed() < Duration::from_secs(2))
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
#[derive(SystemParam)]
struct Scene<'w, 's> {
    heroes: Query<
        'w,
        's,
        (
            Entity,
            &'static NetworkPlayerId,
            &'static Transform,
            &'static Team,
            Option<&'static Player>,
        ),
    >,
    bars: Query<'w, 's, (Entity, &'static crate::combat::CombatBarAnchor)>,
    children: Query<'w, 's, &'static Children>,
    entities: Query<'w, 's, Entity>,
    drawables:
        Query<'w, 's, (&'static GlobalTransform, &'static InheritedVisibility), With<Mesh3d>>,
    nodes: Query<
        'w,
        's,
        (
            &'static Name,
            Option<&'static ComputedNode>,
            Option<&'static InheritedVisibility>,
            Option<&'static Text>,
        ),
    >,
    cameras: Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<MainCamera>>,
    windows: Query<'w, 's, Entity, With<PrimaryWindow>>,
    minimap: MinimapQaScene<'w, 's>,
}
fn observe(
    mut commands: Commands,
    mut qa: ResMut<VisionQa>,
    scene: Scene,
    snapshot: Res<GameStateSnapshot>,
    session: Res<ClientSession>,
    help: Res<HelpOverlayVisible>,
    mobile: Res<MobileControls>,
    mode: Res<PlayerVisualMode>,
    fog: Res<crate::team_vision::VisionPresentation>,
    mut target: ResMut<TargetState>,
    meshes: Res<Assets<Mesh>>,
    materials: Res<Assets<StandardMaterial>>,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.stage == 255 {
        return;
    }
    if qa.started.elapsed() > qa.timeout {
        fail(
            &mut qa,
            &mut exit,
            "bounded native vision scenario timed out",
        );
        return;
    }
    if !session.join_confirmed() || help.0 {
        return;
    }
    if *mode != PlayerVisualMode::Models3d {
        fail(&mut qa, &mut exit, "scenario requires actual Models3d");
        return;
    }
    let Some((local, own_id, position, _, _)) =
        scene.heroes.iter().find(|(_, _, _, _, p)| p.is_some())
    else {
        return;
    };
    let Some(vision) = snapshot.vision.as_ref() else {
        return;
    };
    if qa.stage == FILES.len() {
        if qa.readbacks.len() != FILES.len()
            || !FILES
                .iter()
                .all(|f| qa.directory.join(f).metadata().is_ok_and(|m| m.len() > 32))
        {
            return;
        }
        let summary = serde_json::json!({"scenario":"team-vision","pass":true,"version":env!("CARGO_PKG_VERSION"),
            "scripted_commands":true,"synthetic_visibility":false,"manual_interaction_verified":false,"physical_device_verified":false,
            "setup_fixture":"development placement around authored brush 1; ambient AI disabled",
            "player_id":own_id.0,"target_id":qa.target_id,"pixels":qa.pixels.to_array(),"captures":qa.captures});
        if std::fs::write(
            qa.directory.join("qa-summary.json"),
            serde_json::to_vec_pretty(&summary).unwrap(),
        )
        .is_err()
        {
            fail(&mut qa, &mut exit, "cannot save summary");
            return;
        }
        qa.stage = 255;
        if let Ok(window) = scene.windows.single() {
            commands.entity(window).despawn();
        }
        exit.write(AppExit::Success);
        return;
    }
    if qa.stage > 0 && !qa.readbacks.contains(&(qa.stage - 1)) {
        return;
    }
    let enemy = scene.heroes.iter().find(|(_, id, _, team, _)| {
        **team == Team::Blue && (qa.target_id == 0 || id.0 == qa.target_id)
    });
    let rendered = enemy.map_or(0, |(entity, _, _, _, _)| {
        std::iter::once(entity)
            .chain(scene.children.iter_descendants(entity))
            .filter(|e| {
                let Ok((t, v)) = scene.drawables.get(*e) else {
                    return false;
                };
                let Ok((camera, transform)) = scene.cameras.single() else {
                    return false;
                };
                let Some(size) = camera.logical_viewport_size() else {
                    return false;
                };
                v.get()
                    && camera
                        .world_to_viewport(transform, t.translation())
                        .is_ok_and(|p| p.x > 0.0 && p.y > 0.0 && p.x < size.x && p.y < size.y)
            })
            .count()
    });
    let minimap = scene.minimap.diagnostics();
    let hidden = matches!(qa.stage, 1 | 3 | 5);
    let old_alive = qa
        .old_descendants
        .iter()
        .filter(|e| scene.entities.contains(**e))
        .count();
    let remaining_bars = scene
        .bars
        .iter()
        .filter(|(_, bar)| qa.old_descendants.contains(&bar.target))
        .count();
    let target_cleared = target.selected_entity.is_none() && target.selected_target.is_none();
    let state_ok = if hidden {
        enemy.is_none()
            && old_alive == 0
            && remaining_bars == 0
            && target_cleared
            && minimap["hero_markers"]["enemy"].as_u64() == Some(0)
    } else {
        enemy.is_some() && rendered > 0
    };
    let brush = shared::vision::brush_layout()[0];
    let center = Vec2::from_array(brush.center);
    // The admitted allied observer occupies the return destination. Normal
    // player collision keeps our hero PLAYER_SIZE away; do not force overlap.
    let return_tolerance = crate::player::PLAYER_SIZE + 0.05;
    let location_ok = match qa.stage {
        2 => {
            vision.local_brush == Some(brush.id)
                && position.translation.xz().distance(center + Vec2::X * 2.0) < 0.6
        }
        3 => {
            vision.local_brush.is_none()
                && position.translation.xz().distance(center + Vec2::X * 8.0) < return_tolerance
        }
        _ => true,
    };
    // The driver acknowledges a reached destination using its own blue-team wire snapshot.
    let peer_ready = qa.stage == 0
        || std::fs::read_to_string(qa.directory.join("peer-ready.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .is_some_and(|v| v["stage"].as_u64() == Some(qa.stage as u64));
    if !state_ok || !location_ok || !peer_ready {
        qa.settled = 0;
        return;
    }
    qa.settled += 1;
    if qa.settled < 45 {
        return;
    }
    if let Some((entity, id, _, _, _)) = enemy {
        qa.target_id = id.0;
        target.selected_entity = Some(entity);
        target.selected_target = Some(TargetId {
            kind: TargetKind::Player,
            id: id.0,
        });
        qa.old_descendants = std::iter::once(entity)
            .chain(scene.children.iter_descendants(entity))
            .collect();
    }
    let nodes:Vec<_>=scene.nodes.iter().filter(|(n,_,_,_)| n.as_str().starts_with("Mobile") || n.as_str().starts_with("Gameplay brush") || n.as_str().contains("Brush") || n.as_str().contains("fog") || n.as_str()=="TargetMarker")
        .map(|(name,node,visible,text)| serde_json::json!({"name":name.as_str(),"size":node.map(|n|n.size().to_array()),"visible":visible.is_some_and(|v|v.get()),"text":text.map(|t|t.0.as_str())})).collect();
    let frame = serde_json::json!({"stage":qa.stage,"file":FILES[qa.stage],"pixels":qa.pixels.to_array(),
        "snapshot_tick":snapshot.meta.snapshot_tick,"server_epoch":snapshot.meta.server_epoch,"match_id":snapshot.meta.match_id,
        "visual_mode":format!("{:?}",*mode),"mobile_controls":mobile.enabled,"player_position":position.translation.to_array(),
        "brush_fixture":{"id":brush.id,"center":brush.center,"radius":brush.radius},
        "vision":vision,"target_id":qa.target_id,"enemy_present":enemy.is_some(),"rendered_enemy_drawables":rendered,
        "enemy_entities":scene.heroes.iter().filter(|(_,id,_,_,_)|id.0==qa.target_id).count(),
        "previous_actor_entities_remaining":old_alive,"target_cleared":target_cleared,"minimap":minimap,"nodes":nodes,
        "enemy_combat_bars":remaining_bars,"fog":{"active":fog.active,"updates":fog.updates,"darkest":fog.darkest,"clearest":fog.clearest},
        "mesh_assets":meshes.len(),"material_assets":materials.len(),"total_entities":scene.entities.iter().count()});
    let stage = qa.stage;
    qa.captures.push(frame);
    let _ = std::fs::write(
        qa.directory.join(format!("stage-{stage}.json")),
        serde_json::to_vec_pretty(qa.captures.last().unwrap()).unwrap(),
    );
    commands
        .spawn((Screenshot::primary_window(), Shot(stage)))
        .observe(save_to_disk(qa.directory.join(FILES[stage])))
        .observe(readback);
    qa.stage += 1;
    qa.settled = 0;
    let next = qa.stage;
    let enemy_destination = match next {
        1..=3 => Some(center),
        4 => Some(center + Vec2::X * 4.0),
        5 => Some(center + Vec2::X * 44.0),
        _ => None,
    };
    let _ = std::fs::write(
        qa.directory.join("stage-command.json"),
        serde_json::to_vec(&serde_json::json!({
            "stage":next,"enemy_destination":enemy_destination.map(|p| p.to_array()),
            "brush":{"id":brush.id,"center":brush.center,"radius":brush.radius}
        }))
        .unwrap(),
    );
    if next == 2 || next == 3 {
        let destination = center + Vec2::X * if next == 2 { 2.0 } else { 8.0 };
        commands.entity(local).insert(MovementTarget {
            target: Vec3::new(destination.x, position.translation.y, destination.y),
        });
    }
}
#[derive(Component)]
struct Shot(usize);
fn readback(captured: On<ScreenshotCaptured>, shots: Query<&Shot>, mut qa: ResMut<VisionQa>) {
    if let Ok(shot) = shots.get(captured.entity)
        && captured.image.width() == qa.pixels.x
        && captured.image.height() == qa.pixels.y
        && !qa.readbacks.contains(&shot.0)
    {
        qa.readbacks.push(shot.0);
    }
}
fn fail(qa: &mut VisionQa, exit: &mut MessageWriter<AppExit>, reason: &str) {
    let _ = std::fs::write(
        qa.directory.join("qa-failure.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"stage":qa.stage,"reason":reason,"captures":qa.captures}),
        )
        .unwrap(),
    );
    error!("TEAM_VISION_QA failed stage={}: {reason}", qa.stage);
    qa.stage = 255;
    exit.write(AppExit::error());
}
