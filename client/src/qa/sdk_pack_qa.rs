//! Local developer catalog evidence through the ordinary SDK/preview/match paths.
use crate::{
    frontend::{
        AppScreen, ScreenDriverPaused,
        preview::{AvatarPreview, PreviewStatus},
    },
    net::{ClientSession, GameStateSnapshot, NetworkAvatar, NetworkCommand},
    player::Player,
    team::{CharacterChoice, Team, TeamSelection},
};
use bevy::{
    app::AppExit,
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    window::PrimaryWindow,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

pub(crate) struct SdkPackQaPlugin;
#[derive(serde::Deserialize)]
struct Pack {
    items: Vec<Item>,
}
#[derive(serde::Deserialize)]
struct Item {
    name: String,
    slug: String,
}
#[derive(Resource)]
struct Qa {
    directory: PathBuf,
    items: Vec<Item>,
    index: usize,
    stage: u8,
    frames: u32,
    match_index: usize,
    started: Instant,
    readback: bool,
    captures: Vec<serde_json::Value>,
    start: Option<Vec3>,
    preview_pose: Option<(Entity, Vec<Quat>, f32)>,
    remote_pose: Option<(Entity, Vec3, Vec<Quat>, f32)>,
    collection_step: u8,
}
#[derive(Component)]
struct Shot;
impl Plugin for SdkPackQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(directory) = std::env::var_os("OMOBA_SDK_PACK_QA_OUTPUT").map(PathBuf::from)
        else {
            return;
        };
        let pack: Pack = serde_json::from_slice(
            &std::fs::read(std::env::var_os("OMOBA_SDK_PACK_QA_MANIFEST").expect("pack manifest"))
                .expect("read pack"),
        )
        .expect("valid pack");
        let match_index = std::env::var("OMOBA_SDK_PACK_QA_MATCH")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        assert!(match_index < pack.items.len());
        let stage = if std::env::var_os("OMOBA_SDK_PACK_QA_MATCH_ONLY").is_some() {
            10
        } else {
            0
        };
        app.insert_resource(Qa {
            directory,
            items: pack.items,
            index: 0,
            stage,
            frames: 0,
            match_index,
            started: Instant::now(),
            readback: false,
            captures: vec![],
            start: None,
            preview_pose: None,
            remote_pose: None,
            collection_step: 0,
        })
        .insert_resource(ScreenDriverPaused(true))
        .insert_resource(bevy::winit::WinitSettings::continuous())
        .add_systems(PreUpdate, prepare.after(bevy::input::InputSystems))
        .add_systems(
            PostUpdate,
            observe.after(bevy::transform::TransformSystems::Propagate),
        );
    }
}
fn prepare(
    mut qa: ResMut<Qa>,
    mut next: ResMut<NextState<AppScreen>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut buttons: super::TestIdPresses,
    preview: Res<AvatarPreview>,
    selection: Res<TeamSelection>,
    help: Res<crate::help_overlay::HelpOverlayVisible>,
    mut camera: ResMut<crate::camera::CameraState>,
    mut settings: ResMut<crate::camera::CameraSettings>,
) {
    if let Ok(mut w) = windows.single_mut() {
        w.resolution.set_scale_factor_override(Some(1.));
        w.resolution.set_physical_resolution(1280, 720);
    }
    if help.0 {
        buttons.press("HelpDismissButton");
    }
    qa.frames += 1;
    if qa.stage < 10 {
        // Bevy 0.18 re-enters identical states with set(), resetting the preview.
        next.as_mut().set_if_neq(AppScreen::Collection);
    }
    if qa.stage == 0 {
        if preview.slug.as_deref() == Some(&qa.items[qa.index].slug) {
            qa.stage = 1;
            qa.frames = 0;
        } else if qa.frames.is_multiple_of(15) {
            // Exercise the ordinary equipment buttons, then the visible Studio
            // filter; its avatar tile is below the included roster in All.
            let weapon = std::env::var("OMOBA_SDK_PACK_QA_WEAPON").ok();
            match (qa.collection_step, weapon.as_deref()) {
                (0, Some(_)) => {
                    if buttons.press("CollectionWeapons") {
                        qa.collection_step = 1;
                    }
                }
                (1, Some(id)) => {
                    if selection.handheld == shared::handheld::HandheldSelection::Item(id.into()) {
                        qa.collection_step = 2;
                    } else {
                        buttons.press(&format!("Handheld-{id}"));
                    }
                }
                (0 | 2, _) => {
                    if buttons.press("CollectionStudio") {
                        qa.collection_step = 3;
                    }
                }
                _ => {
                    buttons.press(&format!("CollectionTile-{}", qa.items[qa.index].slug));
                }
            }
        }
    }
    if qa.stage >= 10 {
        *camera = crate::camera::CameraState {
            zoom: crate::camera::CAMERA_MIN_ZOOM,
            ..default()
        };
        settings.zoom = crate::camera::CAMERA_MIN_ZOOM;
    }
}
fn observe(
    mut commands: Commands,
    mut qa: ResMut<Qa>,
    preview: Res<AvatarPreview>,
    assets: Res<AssetServer>,
    (session, snapshot): (Res<ClientSession>, Res<GameStateSnapshot>),
    mut outgoing: MessageWriter<NetworkCommand>,
    mut selection: ResMut<TeamSelection>,
    mut next: ResMut<NextState<AppScreen>>,
    mut exit: MessageWriter<AppExit>,
    preview_rigs: Query<(
        Entity,
        &Name,
        &crate::humanoid::RuntimeHumanoidPlayer,
        &AnimationPlayer,
    )>,
    humanoids: Res<crate::humanoid::HumanoidRuntimeLibrary>,
    remote_actors: Query<
        (
            Entity,
            &Transform,
            &NetworkAvatar,
            &crate::model_scale::ModelScaleSource,
            &crate::humanoid::RuntimeHumanoidPlayer,
            &crate::net::PlayerHandheld,
            &crate::net::NetworkPlayerId,
            &AnimationPlayer,
        ),
        With<crate::net::RemotePlayer>,
    >,
    (models, actors, held, bones, animations): (
        Query<(&Name, &crate::model_scale::ModelScaleSource, &SceneRoot)>,
        Query<
            (
                Entity,
                &Transform,
                &NetworkAvatar,
                &crate::model_scale::ModelScaleSource,
                &crate::humanoid::RuntimeHumanoidPlayer,
            ),
            With<Player>,
        >,
        Query<(
            &crate::held_weapons::HeldWeapon,
            &Transform,
            &GlobalTransform,
            &SceneRoot,
        )>,
        Query<&GlobalTransform>,
        Res<crate::sandbox::AnimationReadout>,
    ),
) {
    if qa.stage == 255 {
        return;
    }
    if qa.frames.is_multiple_of(120) {
        let progress = serde_json::json!({"stage":qa.stage,"index":qa.index,"preview":preview.slug,
            "clips":preview.clips.iter().map(|c|&c.name).collect::<Vec<_>>(),
            "player_id":snapshot.your_id,"animation":animations.0.get(&snapshot.your_id),
            "distance":actors.single().ok().and_then(|(_,pose,_,_,_)|qa.start.map(|start|pose.translation.distance(start)))});
        let _ = std::fs::write(qa.directory.join("progress.json"), progress.to_string());
    }
    if qa.started.elapsed() > Duration::from_secs(240) {
        let _ = std::fs::write(
            qa.directory.join("failure.json"),
            format!(
                "{{\"stage\":{},\"index\":{},\"error\":\"timeout\"}}",
                qa.stage, qa.index
            ),
        );
        qa.stage = 255;
        exit.write(AppExit::error());
        return;
    }
    let loaded = |scene: &SceneRoot| {
        matches!(
            assets.get_recursive_dependency_load_state(scene.0.id()),
            Some(bevy::asset::RecursiveDependencyLoadState::Loaded)
        )
    };
    // The faster client keeps moving through its readback/overlap period so
    // the other rendered client can still measure replicated skeletal motion.
    if matches!(qa.stage, 12 | 14 | 15) && qa.frames.is_multiple_of(60) {
        if let Ok((entity, pose, _, _, _)) = actors.single() {
            commands
                .entity(entity)
                .insert(crate::player::MovementTarget {
                    target: pose.translation
                        + Vec3::new(if (qa.frames / 60) % 2 == 0 { 2.0 } else { -2.0 }, 0.0, 0.0),
                });
        }
    }
    match qa.stage {
        1 if qa.frames >= 45 && preview.status == PreviewStatus::Ready => {
            let item = &qa.items[qa.index];
            let expected = omoba_passport::store::model_asset_path(&item.slug);
            if preview.slug.as_deref() != Some(&item.slug)
                || omoba_passport::store::model_state(&item.slug)
                    != omoba_passport::store::ModelState::Ready
            {
                return;
            }
            if !models.iter().any(|(name, source, scene)| {
                name.as_str() == format!("AvatarPreviewModel-{}", item.slug)
                    && assets
                        .get_path(source.gltf.id())
                        .is_some_and(|p| p.to_string() == expected)
                    && loaded(scene)
            }) {
                return;
            }
            let Some((entity, _, rig, player)) = preview_rigs.iter().find(|(_, name, _, _)| {
                name.as_str() == format!("AvatarPreviewModel-{}", item.slug)
            }) else {
                return;
            };
            let Some(node) = preview.selected_clip().map(|clip| clip.node) else {
                return;
            };
            let Some(animation) = player.animation(node) else {
                return;
            };
            let elapsed = animation.elapsed();
            let Some((_, source, _)) = models
                .iter()
                .find(|(name, _, _)| name.as_str() == format!("AvatarPreviewModel-{}", item.slug))
            else {
                return;
            };
            let Some(semantic) = humanoids.semantic_nodes(&source.gltf) else {
                return;
            };
            let pose: Vec<_> = [
                "hips",
                "leftUpperArm",
                "rightUpperArm",
                "leftUpperLeg",
                "rightUpperLeg",
            ]
            .iter()
            .filter_map(|name| rig.joint(*semantic.get(*name)?))
            .filter_map(|joint| {
                bones
                    .get(joint)
                    .ok()
                    .map(|t| t.compute_transform().rotation)
            })
            .collect();
            if pose.len() != 5 {
                return;
            }
            let Some((old_entity, old_pose, old_elapsed)) = qa.preview_pose.as_ref() else {
                qa.preview_pose = Some((entity, pose, elapsed));
                return;
            };
            if *old_entity != entity {
                qa.preview_pose = Some((entity, pose, elapsed));
                return;
            }
            let advance = elapsed - old_elapsed;
            let rotation_delta = pose
                .iter()
                .zip(old_pose)
                .map(|(a, b)| a.angle_between(*b).abs())
                .sum::<f32>();
            if advance < 0.15 || rotation_delta < 0.0001 {
                return;
            }
            let clips: Vec<_> = preview.clips.iter().map(|c| c.name.clone()).collect();
            if !["idle", "walk", "attack", "cast", "death"]
                .iter()
                .all(|n| clips.iter().any(|c| c == n))
            {
                return;
            }
            let file = format!(
                "{:02}-{}.png",
                qa.index + 1,
                item.name.to_lowercase().replace(' ', "-")
            );
            let frame = serde_json::json!({"file":file,"name":item.name,"slug":item.slug,"model":expected,"clips":clips,"store_ready":true,"scene_loaded":true,"preview_bound":true,"animation_advance_secs":advance,"bone_rotation_delta":rotation_delta,"equipment_selected_through_ui":qa.collection_step==3,"handheld":selection.handheld});
            capture(&mut commands, &mut qa, &file, frame);
            qa.stage = 2;
        }
        2 if qa.readback => {
            qa.index += 1;
            qa.preview_pose = None;
            qa.frames = 0;
            qa.stage = if qa.index == qa.items.len() { 10 } else { 0 };
        }
        10 if session.is_connected() => {
            let slug = qa.items[qa.match_index].slug.clone();
            if omoba_passport::store::model_state(&slug) != omoba_passport::store::ModelState::Ready
            {
                return;
            }
            if let Ok(id) = std::env::var("OMOBA_SDK_PACK_QA_WEAPON") {
                if omoba_passport::weapon_store::model_state(&id)
                    != omoba_passport::store::ModelState::Ready
                {
                    return;
                }
                selection.handheld = shared::handheld::HandheldSelection::Item(id);
            }
            selection.hero_class = shared::HeroClass::Warrior;
            selection.avatar = Some(slug.clone());
            selection.character = CharacterChoice::Cube;
            outgoing.write(NetworkCommand::Join {
                handheld: selection.handheld.clone(),
                team: Team::Green,
                character: CharacterChoice::Cube,
                hero_class: shared::HeroClass::Warrior,
                avatar: Some(slug),
                sprite_character: None,
            });
            next.set(AppScreen::InMatch);
            qa.stage = 11;
            qa.frames = 0;
        }
        11 if session.join_confirmed() && qa.frames >= 90 => {
            let Ok((entity, pose, avatar, source, rig)) = actors.single() else {
                return;
            };
            let slug = &qa.items[qa.match_index].slug;
            if avatar.0.as_ref() != Some(slug)
                || rig.model != source.gltf.id()
                || assets
                    .get_path(source.gltf.id())
                    .is_none_or(|p| p.to_string() != omoba_passport::store::model_asset_path(slug))
            {
                return;
            }
            if !held.iter().any(|(w, _, _, scene)| {
                w.owner == entity
                    && w.id
                        == std::env::var("OMOBA_SDK_PACK_QA_WEAPON")
                            .unwrap_or_else(|_| "forge-sword".into())
                    && loaded(scene)
            }) {
                return;
            }
            qa.start = Some(pose.translation);
            commands
                .entity(entity)
                .insert(crate::player::MovementTarget {
                    target: pose.translation
                        + Vec3::new(3., 0., if snapshot.your_id % 2 == 0 { -3. } else { 3. }),
                });
            qa.stage = 12;
            qa.frames = 0;
        }
        12 if qa.frames >= 12 => {
            let Ok((entity, pose, avatar, source, rig)) = actors.single() else {
                return;
            };
            if animations
                .0
                .get(&snapshot.your_id)
                .is_none_or(|(name, _)| name != "Run")
                || pose.translation.distance(qa.start.unwrap()) < 0.2
            {
                return;
            }
            let Some((weapon, local, world, scene)) =
                held.iter().find(|(w, _, _, _)| w.owner == entity)
            else {
                return;
            };
            let Ok(hand) = bones.get(weapon.hand) else {
                return;
            };
            let error = hand
                .mul_transform(*local)
                .translation()
                .distance(world.translation());
            let frame = serde_json::json!({"file":"21-gameplay-running.png","name":qa.items[qa.match_index].name,"slug":avatar.0,
                "model":assets.get_path(source.gltf.id()).map(|p|p.to_string()),"bound_to_model":rig.model==source.gltf.id(),
                "animation":"Run","distance":pose.translation.distance(qa.start.unwrap()),"weapon":weapon.id,
                "attachment_error":error,"scene_loaded":loaded(scene),"server_admitted":session.join_confirmed(),"player_id":snapshot.your_id});
            capture(&mut commands, &mut qa, "21-gameplay-running.png", frame);
            if std::env::var_os("OMOBA_SDK_PACK_QA_REMOTE_WEAPON").is_none() {
                commands
                    .entity(entity)
                    .remove::<(crate::player::MovementTarget, crate::player::MovementRoute)>();
            }
            qa.stage = 13;
        }
        13 if qa.readback && std::env::var_os("OMOBA_SDK_PACK_QA_REMOTE_WEAPON").is_some() => {
            qa.stage = 14;
            qa.frames = 0;
        }
        14 => {
            let Ok(expected_weapon) = std::env::var("OMOBA_SDK_PACK_QA_REMOTE_WEAPON") else {
                return;
            };
            let Some((entity,pose,avatar,source,rig,selection,id,player))=remote_actors.iter().find(|(_,_,_,_,_,selection,_,_)|matches!(&selection.0,shared::handheld::HandheldSelection::Item(value) if *value==expected_weapon)) else{return};
            let Some(slug) = avatar.0.as_deref() else {
                return;
            };
            if omoba_passport::store::model_state(slug) != omoba_passport::store::ModelState::Ready
                || rig.model != source.gltf.id()
                || assets
                    .get_path(source.gltf.id())
                    .is_none_or(|p| p.to_string() != omoba_passport::store::model_asset_path(slug))
            {
                return;
            }
            let Some((weapon, local, world, scene)) = held.iter().find(|(weapon, _, _, scene)| {
                weapon.owner == entity && weapon.id == expected_weapon && loaded(scene)
            }) else {
                return;
            };
            let Some(definition) = omoba_passport::weapon_store::definition(&expected_weapon)
            else {
                return;
            };
            if !definition.model.starts_with("ekza://weapons/")
                || assets
                    .get_path(scene.0.id())
                    .is_none_or(|p| p.to_string() != format!("{}#Scene0", definition.model))
            {
                return;
            }
            let Some(semantic) = humanoids.semantic_nodes(&source.gltf) else {
                return;
            };
            let rotations: Vec<_> = [
                "hips",
                "leftUpperArm",
                "rightUpperArm",
                "leftUpperLeg",
                "rightUpperLeg",
            ]
            .iter()
            .filter_map(|name| rig.joint(*semantic.get(*name)?))
            .filter_map(|joint| {
                bones
                    .get(joint)
                    .ok()
                    .map(|t| t.compute_transform().rotation)
            })
            .collect();
            if rotations.len() != 5 {
                return;
            }
            let elapsed = player
                .playing_animations()
                .map(|(_, animation)| animation.elapsed())
                .fold(0.0_f32, f32::max);
            let Some((old_entity, old_position, old_rotation, old_elapsed)) =
                qa.remote_pose.as_ref()
            else {
                qa.remote_pose = Some((entity, pose.translation, rotations, elapsed));
                return;
            };
            if *old_entity != entity {
                qa.remote_pose = Some((entity, pose.translation, rotations, elapsed));
                return;
            }
            let distance = pose.translation.distance(*old_position);
            let rotation_delta = rotations
                .iter()
                .zip(old_rotation)
                .map(|(a, b)| a.angle_between(*b).abs())
                .sum::<f32>();
            if elapsed < *old_elapsed {
                qa.remote_pose = Some((entity, pose.translation, rotations, elapsed));
                return;
            }
            let advance = elapsed - old_elapsed;
            if advance < 0.15 || distance < 0.1 || rotation_delta < 0.0001 {
                return;
            }
            let Ok(hand) = bones.get(weapon.hand) else {
                return;
            };
            let error = hand
                .mul_transform(*local)
                .translation()
                .distance(world.translation());
            if error > 0.0001 {
                return;
            }
            if id.0 == snapshot.your_id
                || selection.0 != shared::handheld::HandheldSelection::Item(expected_weapon.clone())
            {
                return;
            }
            let frame = serde_json::json!({"file":"22-remote-equipped.png","remote_player_id":id.0,"local_player_id":snapshot.your_id,"slug":slug,"model":omoba_passport::store::model_asset_path(slug),"weapon":weapon.id,"weapon_model":definition.model,"scene_loaded":true,"bound_to_model":true,"replicated_selection_verified":true,"distance":distance,"animation_advance_secs":advance,"bone_rotation_delta":rotation_delta,"attachment_error":error});
            capture(&mut commands, &mut qa, "22-remote-equipped.png", frame);
            qa.stage = 15;
            qa.frames = 0;
        }
        13 | 15 if qa.readback && (qa.stage == 13 || qa.frames >= 120) => {
            let public_studio =
                std::env::var("OMOBA_SDK_PACK_QA_LIVE_REGISTRY").is_ok_and(|value| value == "1");
            let report = serde_json::json!({"pass":true,"local_developer_catalog":!public_studio,"public_studio":public_studio,"locale":"en","pixels":[1280,720],"captures":qa.captures});
            let summary = qa.directory.join("qa-summary.json");
            if !summary.exists() {
                std::fs::write(&summary, serde_json::to_vec_pretty(&report).unwrap())
                    .expect("save evidence");
            }
            // Keep the real actor alive and moving until the independent peer
            // has finished its own rendering assertions. Fixed frame overlap
            // races network downloads and slow rendering on the second client.
            if qa.stage == 15
                && std::env::var_os("OMOBA_SDK_PACK_QA_HOLD_FOR_PEER").is_some()
                && !qa.directory.join("peer-complete").exists()
            {
                return;
            }
            qa.stage = 255;
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}
fn capture(commands: &mut Commands, qa: &mut Qa, file: &str, frame: serde_json::Value) {
    qa.readback = false;
    qa.captures.push(frame);
    commands
        .spawn((Screenshot::primary_window(), Shot))
        .observe(save_to_disk(qa.directory.join(file)))
        .observe(
            |event: On<ScreenshotCaptured>, shots: Query<(), With<Shot>>, mut qa: ResMut<Qa>| {
                if shots.contains(event.entity)
                    && event.image.width() == 1280
                    && event.image.height() == 720
                {
                    qa.readback = true;
                }
            },
        );
}
