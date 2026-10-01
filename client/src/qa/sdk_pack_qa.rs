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
            buttons.press(&format!("CollectionTile-{}", qa.items[qa.index].slug));
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
            "clips":preview.clips.iter().map(|c|&c.name).collect::<Vec<_>>()});
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
            let frame = serde_json::json!({"file":file,"name":item.name,"slug":item.slug,"model":expected,"clips":clips,"store_ready":true,"scene_loaded":true});
            capture(&mut commands, &mut qa, &file, frame);
            qa.stage = 2;
        }
        2 if qa.readback => {
            qa.index += 1;
            qa.frames = 0;
            qa.stage = if qa.index == qa.items.len() { 10 } else { 0 };
        }
        10 if session.is_connected() => {
            let slug = qa.items[qa.match_index].slug.clone();
            if omoba_passport::store::model_state(&slug) != omoba_passport::store::ModelState::Ready
            {
                return;
            }
            selection.hero_class = shared::HeroClass::Warrior;
            selection.avatar = Some(slug.clone());
            selection.character = CharacterChoice::Cube;
            outgoing.write(NetworkCommand::Join {
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
            if !held
                .iter()
                .any(|(w, _, _, scene)| w.owner == entity && w.id == "forge-sword" && loaded(scene))
            {
                return;
            }
            qa.start = Some(pose.translation);
            commands
                .entity(entity)
                .insert(crate::player::MovementTarget {
                    target: pose.translation + Vec3::new(3., 0., 3.),
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
            commands
                .entity(entity)
                .remove::<(crate::player::MovementTarget, crate::player::MovementRoute)>();
            qa.stage = 13;
        }
        13 if qa.readback => {
            let public_studio =
                std::env::var("OMOBA_SDK_PACK_QA_LIVE_REGISTRY").is_ok_and(|value| value == "1");
            let report = serde_json::json!({"pass":true,"local_developer_catalog":!public_studio,"public_studio":public_studio,"locale":"en","pixels":[1280,720],"captures":qa.captures});
            std::fs::write(
                qa.directory.join("qa-summary.json"),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .expect("save evidence");
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
