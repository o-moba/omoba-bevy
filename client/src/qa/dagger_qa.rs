//! One phone viewport: real offline casts plus an explicitly labelled rare-receipt
//! presentation fixture. No controllable authority RNG or physical-device claim.
use crate::{
    combat::CombatStats,
    frontend::AppScreen,
    net::{
        ClientSession, GameStateSnapshot, NetworkCommand, NetworkHeroClass, NetworkPlayerId,
        PlayerCosmeticAction, PlayerLoadout,
    },
    player::{MovementRoute, MovementTarget, Player},
};
use bevy::{
    app::AppExit,
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    window::PrimaryWindow,
};
use shared::combat::{CombatEntity, CombatEntityKind, CombatEvent, ProjectileStyle};
use std::{
    collections::HashSet,
    path::PathBuf,
    time::{Duration, Instant},
};

pub(crate) struct DaggerQaPlugin;
#[derive(Resource)]
struct Qa {
    dir: PathBuf,
    combat_location: Vec3,
    started: Instant,
    since: Instant,
    phase: u8,
    done: HashSet<String>,
    frames: Vec<serde_json::Value>,
    casts: Vec<serde_json::Value>,
    target: Option<Entity>,
    target_id: u64,
    own_id: u64,
    ack: u64,
    hit_id: u64,
    synthetic: Option<CombatEvent>,
}
#[derive(Component)]
struct Label;
const COMBAT_LOCATION: Vec3 = Vec3::new(-12.0, 0.5, -12.0);
fn parse_combat_location(value: &str) -> Option<Vec3> {
    let (x, z) = value.split_once(',')?;
    let position = Vec3::new(x.trim().parse().ok()?, 0.5, z.trim().parse().ok()?);
    (position.is_finite() && position.xz().abs().max_element() <= 90.0).then_some(position)
}
impl Plugin for DaggerQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(dir) = std::env::var_os("OMOBA_DAGGER_QA_DIR").map(PathBuf::from) else {
            return;
        };
        let combat_location =
            std::env::var("OMOBA_DAGGER_QA_POSITION").map_or(COMBAT_LOCATION, |value| {
                parse_combat_location(&value)
                    .expect("OMOBA_DAGGER_QA_POSITION must be finite x,z within -90..90")
            });
        std::fs::create_dir_all(&dir).expect("dagger QA directory");
        app.insert_resource(Qa {
            dir,
            combat_location,
            started: Instant::now(),
            since: Instant::now(),
            phase: 0,
            done: HashSet::new(),
            frames: Vec::new(),
            casts: Vec::new(),
            target: None,
            target_id: 0,
            own_id: 0,
            ack: 0,
            hit_id: 0,
            synthetic: None,
        })
        .insert_resource(bevy::winit::WinitSettings::continuous())
        .add_systems(Startup, label)
        .add_systems(PreUpdate, super::combat_qa::focus_capture_window)
        .add_systems(
            Update,
            window.before(crate::mobile_controls::MobileControlsSet::Layout),
        )
        .add_systems(
            Update,
            drive
                .after(crate::net::ClientNetPipeline::ApplySnapshot)
                .before(crate::combat_feedback::CollectCombatFeedback),
        )
        .add_systems(Last, (capture_ready, fixture_pacing));
    }
}
fn label(mut commands: Commands) {
    commands.spawn((
        Label,
        Text::new("QA · Adventurer · scripted offline casts"),
        TextFont {
            font_size: 9.0,
            ..default()
        },
        TextColor(Color::WHITE),
        BackgroundColor(Color::BLACK),
        Pickable::IGNORE,
        GlobalZIndex(500),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(0.0),
            left: Val::Px(240.0),
            ..default()
        },
    ));
}
fn fixture_pacing(mut pacing: ResMut<bevy::winit::WinitSettings>) {
    if std::env::var("OMOBA_QA_SYNTHETIC_FOCUS").as_deref() == Ok("1") {
        *pacing = bevy::winit::WinitSettings::continuous();
    }
}
fn window(
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut mobile: ResMut<crate::mobile_controls::MobileControls>,
) {
    for mut w in &mut windows {
        w.resolution.set_scale_factor_override(Some(1.0));
        if w.physical_width() != 852 || w.physical_height() != 393 {
            w.resolution.set_physical_resolution(852, 393);
        }
        if std::env::var("OMOBA_QA_SYNTHETIC_FOCUS").as_deref() == Ok("1") {
            w.focused = true;
            mobile.focused = true;
        }
    }
}
fn press(world: &mut World, id: &str) -> bool {
    let target = world
        .query::<(Entity, &crate::ui::TestId)>()
        .iter(world)
        .find(|(_, test)| test.as_str() == id)
        .map(|(entity, _)| entity);
    if let Some(entity) = target {
        world.write_message(crate::ui::SyntheticPress(entity));
    }
    target.is_some()
}
fn advance(qa: &mut Qa, phase: u8) {
    info!("Dagger QA phase {} -> {phase}", qa.phase);
    qa.phase = phase;
    qa.since = Instant::now();
}
fn cast(world: &mut World, qa: &mut Qa, slot: u8) {
    let Some(target) = qa
        .target
        .and_then(|e| world.get::<Transform>(e))
        .map(|t| t.translation.xz())
    else {
        return;
    };
    qa.ack = world
        .query_filtered::<&PlayerLoadout, With<Player>>()
        .single(world)
        .ok()
        .and_then(|s| s.0.as_ref())
        .map_or(0, |s| s.cast_request_id);
    world.write_message(NetworkCommand::CastSkill { slot, aim: target });
}
fn drive(world: &mut World) {
    let Some(mut qa) = world.remove_resource::<Qa>() else {
        return;
    };
    if qa.phase == 255 {
        world.insert_resource(qa);
        return;
    }
    if qa.started.elapsed() > Duration::from_secs(150) {
        let screen = format!("{:?}", world.resource::<State<AppScreen>>().get());
        let session = format!("{:?}", world.resource::<ClientSession>().state());
        let modal = format!("{:?}", world.resource::<crate::ui::ModalStack>().top());
        let context = format!(
            "{:?}",
            world.resource::<crate::input_context::GameplayInputContext>()
        );
        let windows = world
            .query::<&Window>()
            .iter(world)
            .map(|w| {
                serde_json::json!({
            "pixels":[w.physical_width(),w.physical_height()],"focused":w.focused})
            })
            .collect::<Vec<_>>();
        let controls = world
            .query::<(
                &crate::ui::TestId,
                &ComputedNode,
                Option<&InheritedVisibility>,
            )>()
            .iter(world)
            .filter(|(_, node, visible)| {
                node.size().min_element() > 0.0 && visible.is_none_or(|v| v.get())
            })
            .take(80)
            .map(
                |(id, node, _)| serde_json::json!({"id":id.as_str(),"size":node.size().to_array()}),
            )
            .collect::<Vec<_>>();
        std::fs::write(
            qa.dir.join("result.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
            "pass":false,"phase":qa.phase,"casts":qa.casts,"frames":qa.frames,
            "combat_location":qa.combat_location.to_array(),
            "screen":screen,"session":session,"modal":modal,"context":context,"windows":windows,"controls":controls }))
            .unwrap(),
        )
        .unwrap();
        qa.phase = 255;
        world.write_message(AppExit::error());
        world.insert_resource(qa);
        return;
    }
    let screen = *world.resource::<State<AppScreen>>().get();
    if world
        .resource::<crate::help_overlay::HelpOverlayVisible>()
        .0
    {
        press(world, "HelpDismissButton");
    }
    let age = qa.since.elapsed().as_secs_f32();
    let loadout = world
        .query_filtered::<&PlayerLoadout, With<Player>>()
        .single(world)
        .ok()
        .and_then(|s| s.0.clone());
    let acknowledged = loadout.as_ref().is_some_and(|s| s.cast_request_id > qa.ack);
    // The wire's skill_recovery_remaining_secs is applied to this component,
    // not LoadoutState. An HP receipt can precede the shared recovery deadline.
    let recovery_ready = world
        .query_filtered::<&crate::net::PlayerSkillCooldowns, With<Player>>()
        .single(world)
        .is_ok_and(|cooldowns| cooldowns.recovery_secs <= 0.0);
    // Selecting a class can rebuild the picker once. Keep the real scroll
    // component at the appended row until the capture verifies its clip rect.
    if matches!(qa.phase, 1 | 2) {
        for (name, node, mut scroll) in world
            .query::<(&Name, &ComputedNode, &mut ScrollPosition)>()
            .iter_mut(world)
        {
            if name.as_str() == "ClassButtonsRow" {
                scroll.y = crate::ui::scroll::max_offset(node);
            }
        }
    }
    match qa.phase {
        0 if screen == AppScreen::Home && age > 2.0 => {
            if press(world, "HomeOfflinePractice") {
                advance(&mut qa, 1);
            }
        }
        1 if screen == AppScreen::HeroSelect && age > 1.0 => {
            {
                let mut selection = world.resource_mut::<crate::team::TeamSelection>();
                if selection.avatar.as_deref() != Some("agnes") {
                    selection.character = crate::team::CharacterChoice::Ipfs;
                    selection.avatar = Some("agnes".into());
                }
            }
            if world.resource::<crate::team::TeamSelection>().hero_class
                == shared::HeroClass::Adventurer
            {
                advance(&mut qa, 2);
            } else {
                press(world, "ClassButton-adventurer");
            }
        }
        3 if qa.done.contains("01-adventurer-preview.png") => {
            if press(world, "FindMatchButton") {
                advance(&mut qa, 4);
            }
        }
        4 if screen == AppScreen::InMatch
            && world.resource::<ClientSession>().is_offline()
            && world
                .resource::<crate::input_context::GameplayInputContext>()
                .gameplay_allowed() =>
        {
            if let Ok((id, class)) = world
                .query_filtered::<(&NetworkPlayerId, &NetworkHeroClass), With<Player>>()
                .single(world)
                && class.0 == shared::HeroClass::Adventurer
            {
                qa.own_id = id.0;
                world.write_message(NetworkCommand::Debug(
                    shared::debug::DebugCommand::Practice(
                        shared::practice::PracticeCommand::ClearBots,
                    ),
                ));
                if let Ok(actor) = world.query_filtered::<Entity, With<Player>>().single(world) {
                    world.entity_mut(actor).insert(MovementTarget {
                        target: qa.combat_location,
                    });
                    // Existing user zoom range; no camera/HUD element is hidden.
                    world.resource_mut::<crate::camera::CameraSettings>().zoom =
                        crate::camera::CAMERA_MIN_ZOOM;
                    world.resource_mut::<crate::camera::CameraState>().zoom =
                        crate::camera::CAMERA_MIN_ZOOM;
                    advance(&mut qa, 40);
                }
            }
        }
        40 => {
            if let Ok(pose) = world
                .query_filtered::<&Transform, With<Player>>()
                .single(world)
                && pose.translation.xz().distance(qa.combat_location.xz()) < 0.7
            {
                world.write_message(NetworkCommand::Debug(
                    shared::debug::DebugCommand::Practice(
                        shared::practice::PracticeCommand::SpawnDummy,
                    ),
                ));
                advance(&mut qa, 5);
            }
        }
        5 if age > 0.3 => {
            let target = world
                .query::<(Entity, &NetworkPlayerId, &Transform, &CombatStats)>()
                .iter(world)
                .find(|(_, id, _, stats)| {
                    id.0 != qa.own_id && stats.max_hp == shared::debug::DUMMY_MAX_HP
                })
                .map(|(e, id, t, _)| (e, id.0, t.translation));
            if let Some((entity, id, point)) = target
                && let Ok((actor, pose)) = world
                    .query_filtered::<(Entity, &Transform), With<Player>>()
                    .single(world)
            {
                // Mostly camera-depth separation keeps both 2.1m bodies and
                // their overhead bars readable while remaining in melee range.
                let destination = Vec3::new(point.x - 2.15, pose.translation.y, point.z - 0.5);
                world.entity_mut(actor).insert(MovementTarget {
                    target: destination,
                });
                qa.target = Some(entity);
                qa.target_id = id;
                advance(&mut qa, 6);
            }
        }
        6 if recovery_ready => {
            let target = qa
                .target
                .and_then(|e| world.get::<Transform>(e))
                .map(|t| t.translation);
            if let Some(target) = target
                && let Ok((actor, pose)) = world
                    .query_filtered::<(Entity, &Transform), With<Player>>()
                    .single(world)
                && pose
                    .translation
                    .xz()
                    .distance(Vec2::new(target.x - 2.15, target.z - 0.5))
                    < 0.2
                && shared::vision::brush_at(pose.translation.xz().to_array()).is_none()
                && shared::vision::brush_at(target.xz().to_array()).is_none()
            {
                world
                    .entity_mut(actor)
                    .remove::<(MovementTarget, MovementRoute)>();
                cast(world, &mut qa, 0);
                advance(&mut qa, 7);
            }
        }
        7 | 11 | 14 if acknowledged && (qa.phase != 7 || recovery_ready) => {
            let slot = match qa.phase {
                7 => 0,
                11 => 2,
                _ => 3,
            };
            let receipt = world
                .resource::<GameStateSnapshot>()
                .combat_events
                .iter()
                .find(|e| {
                    e.source.id == qa.own_id
                        && e.target.id == qa.target_id
                        && e.action_slot == Some(slot)
                        && e.amount > 0.0
                })
                .cloned();
            if let Some(receipt) = receipt {
                qa.hit_id = receipt.id;
                qa.casts
                    .push(serde_json::json!({"slot":slot,"ack":loadout,"receipt":receipt}));
                if qa.phase == 7 {
                    cast(world, &mut qa, 1);
                    advance(&mut qa, 8);
                } else {
                    let phase = qa.phase + 1;
                    advance(&mut qa, phase);
                }
            }
        }
        8 if acknowledged => {
            let stun = qa
                .target
                .and_then(|e| world.get::<PlayerLoadout>(e))
                .and_then(|s| s.0.as_ref())
                .map_or(0.0, |s| s.stun_remaining_secs);
            if stun > 0.0 {
                qa.casts
                    .push(serde_json::json!({"slot":1,"ack":loadout,"authoritative_stun":stun}));
                advance(&mut qa, 9);
            }
        }
        10 if qa.done.contains("02-bluff.png") && recovery_ready => {
            cast(world, &mut qa, 2);
            advance(&mut qa, 11);
        }
        13 if qa.done.contains("03-backstab.png") && recovery_ready => {
            cast(world, &mut qa, 3);
            advance(&mut qa, 14);
        }
        16 if qa.done.contains("04-lethal-blow.png") => {
            if let Some((position, hp)) = qa.target.and_then(|e| {
                Some((
                    world.get::<Transform>(e)?.translation,
                    world.get::<CombatStats>(e)?.hp,
                ))
            }) {
                let id = world
                    .resource::<GameStateSnapshot>()
                    .combat_events
                    .iter()
                    .map(|e| e.id)
                    .max()
                    .unwrap_or(0)
                    + 1000;
                qa.synthetic = Some(CombatEvent {
                    id,
                    source: CombatEntity {
                        kind: CombatEntityKind::Player,
                        id: qa.own_id,
                    },
                    target: CombatEntity {
                        kind: CombatEntityKind::Player,
                        id: qa.target_id,
                    },
                    x: position.x,
                    y: position.y,
                    z: position.z,
                    amount: (hp - 1.0).max(1.0),
                    near_lethal: true,
                    action_slot: Some(2),
                    style: ProjectileStyle::Standard,
                    ..default()
                });
                qa.hit_id = id;
                advance(&mut qa, 17);
            }
        }
        18 if qa.done.contains("05-vital-break-receipt-fixture.png") => {
            let report = serde_json::json!({"pass":qa.casts.len()==4 && qa.frames.len()==5,"viewport":[852,393],"locale":"en",
                "physical_device_verified":false,"manual_input_verified":false,
                "synthetic_window_focus":std::env::var("OMOBA_QA_SYNTHETIC_FOCUS").as_deref()==Ok("1"),
                "combat_location":qa.combat_location.to_array(),
                "method":"Class selection via UI; socket-free local authority normal Q/W/E/R casts. Last frame injects a labelled presentation-only rare CombatEvent; RNG mechanics are unit tested separately.",
                "casts":qa.casts,"frames":qa.frames});
            std::fs::write(
                qa.dir.join("result.json"),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .unwrap();
            qa.phase = 255;
            world.write_message(AppExit::Success);
        }
        _ => {}
    }
    if matches!(qa.phase, 17 | 18)
        && let Some(receipt) = qa.synthetic.clone()
    {
        world
            .resource_mut::<GameStateSnapshot>()
            .combat_events
            .push(receipt);
        if let Some(mut stats) = qa.target.and_then(|e| world.get_mut::<CombatStats>(e)) {
            stats.hp = 1.0;
        }
        for mut text in world
            .query_filtered::<&mut Text, With<Label>>()
            .iter_mut(world)
        {
            text.0 = "QA · SYNTHETIC VITAL-BREAK RECEIPT · presentation only".into();
        }
    }
    world.insert_resource(qa);
}

fn handhelds(world: &mut World) -> Vec<serde_json::Value> {
    world.query::<(&crate::held_weapons::HeldWeapon,&GlobalTransform,&Transform,&SceneRoot)>().iter(world)
        .filter(|(w,..)|w.id=="dagger")
        .map(|(w,pose,local,scene)|serde_json::json!({"id":w.id,"owner":w.owner.to_bits(),
            "loaded":world.resource::<AssetServer>().is_loaded_with_dependencies(scene.0.id()),
            "position":pose.translation().to_array(),"attachment_error":world.get::<GlobalTransform>(w.hand)
                .map(|hand|hand.mul_transform(*local).translation().distance(pose.translation()))})).collect()
}
fn selected_class_rect(world: &mut World) -> Option<Rect> {
    world
        .query::<(
            &crate::ui::TestId,
            &ComputedNode,
            &UiGlobalTransform,
            Option<&CalculatedClip>,
        )>()
        .iter(world)
        .find(|(id, ..)| id.as_str() == "ClassButton-adventurer")
        .map(|(_, node, transform, clip)| {
            crate::ui::gesture::logical_ui_rect(
                node,
                transform,
                clip,
                1.0 / node.inverse_scale_factor(),
            )
        })
        .filter(|rect| {
            rect.height() >= 44.0
                && rect.width() >= 44.0
                && rect.min.cmpge(Vec2::ZERO).all()
                && rect.max.cmple(Vec2::new(852.0, 393.0)).all()
        })
}
fn capture_ready(world: &mut World) {
    let Some(mut qa) = world.remove_resource::<Qa>() else {
        return;
    };
    let file = match qa.phase {
        2 => "01-adventurer-preview.png",
        9 => "02-bluff.png",
        12 => "03-backstab.png",
        15 => "04-lethal-blow.png",
        17 => "05-vital-break-receipt-fixture.png",
        _ => {
            world.insert_resource(qa);
            return;
        }
    };
    let held = handhelds(world);
    let class_rect = selected_class_rect(world);
    let ready = held
        .iter()
        .any(|w| w["loaded"] == true && w["attachment_error"].as_f64().is_some_and(|e| e < 0.002));
    let action = world
        .query_filtered::<&PlayerCosmeticAction, With<Player>>()
        .single(world)
        .ok()
        .copied();
    let expected = if qa.phase == 9 {
        action.map_or(0, |a| a.sequence)
    } else {
        qa.hit_id
    };
    let particles = world
        .query::<(&crate::game_vfx::ParticleSlot, &InheritedVisibility)>()
        .iter(world)
        .filter(|(p, v)| {
            v.get()
                && p.sample()
                    .is_some_and(|(id, age)| id == expected && (0.12..0.45).contains(&age))
        })
        .count();
    if !ready
        || (qa.phase == 2 && class_rect.is_none())
        || (qa.phase == 2 && qa.since.elapsed().as_secs_f32() < 3.0)
        // Cast sequences and impact receipts use separate counters. A matching
        // particle number alone must not let the previous action capture this
        // stage before its own animation/effect has visibly advanced.
        || (qa.phase != 2 && (particles == 0 || qa.since.elapsed().as_secs_f32() < 0.15))
    {
        world.insert_resource(qa);
        return;
    }
    let animation = world
        .get_resource::<crate::sandbox::AnimationReadout>()
        .map(|a| serde_json::json!(a.0));
    let target=qa.target.and_then(|e|Some(serde_json::json!({"hp":world.get::<CombatStats>(e)?.hp,
        "yaw":world.get::<Transform>(e)?.rotation.to_euler(EulerRot::YXZ).0,
        "stun":world.get::<PlayerLoadout>(e).and_then(|l|l.0.as_ref()).map_or(0.0,|l|l.stun_remaining_secs)})));
    let local_position = world
        .query_filtered::<&Transform, With<Player>>()
        .single(world)
        .ok()
        .map(|p| p.translation.to_array());
    let target_position = qa
        .target
        .and_then(|e| world.get::<Transform>(e))
        .map(|p| p.translation.to_array());
    let zoom = world.resource::<crate::camera::CameraState>().zoom;
    qa.frames.push(serde_json::json!({"file":file,"handhelds":held,"particles":particles,"particle_event_id":expected,
        "animations":animation,"target":target,"local_position":local_position,"target_position":target_position,"camera_zoom":zoom,
        "phase_elapsed_secs":qa.since.elapsed().as_secs_f32(),"particle_counter_is_untyped":true,
        "selected_class_rect":class_rect.map(|r|[r.min.to_array(),r.max.to_array()]),
        "synthetic_receipt":if qa.phase==17 {qa.synthetic.as_ref()} else {None}}));
    let path = qa.dir.join(file);
    world
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path))
        .observe(move |event: On<ScreenshotCaptured>, mut qa: ResMut<Qa>| {
            if event.image.width() == 852 && event.image.height() == 393 {
                qa.done.insert(file.into());
            }
        });
    let next = qa.phase + 1;
    advance(&mut qa, next);
    world.insert_resource(qa);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_location_requires_finite_bounded_xz() {
        assert_eq!(parse_combat_location(" -12, -12 "), Some(COMBAT_LOCATION));
        for invalid in ["NaN,0", "0,inf", "91,0", "0,-91", "1,2,3", "1"] {
            assert_eq!(parse_combat_location(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn default_fixture_and_melee_positions_are_outside_brush() {
        let anchor = COMBAT_LOCATION.xz() + Vec2::splat(4.5 / 2.0_f32.sqrt());
        for point in [COMBAT_LOCATION.xz(), anchor, anchor - Vec2::new(2.15, 0.5)] {
            assert!(shared::vision::brush_at(point.to_array()).is_none());
        }
    }
}
