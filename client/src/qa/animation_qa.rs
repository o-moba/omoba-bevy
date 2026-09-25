//! Opt-in native motion audit: actual scenes, production animation pipeline,
//! labeled deterministic movement fixtures, and measured joint transforms.
use crate::{
    combat::CombatStats,
    humanoid::RuntimeHumanoidPlayer,
    model_scale::{ModelScalePlugin, ModelScaleSettings, ModelScaleSource, NormalizeModelScale},
    net::{NetworkAvatar, NetworkCharacterChoice, PlayerCosmeticAction, RemotePlayer},
    player::{MovementTarget, Player, PlayerBody},
    team::CharacterChoice,
    world::AvatarAssetCache,
};
use bevy::{
    animation::{AnimatedBy, AnimationTargetId},
    camera::ScalingMode,
    gltf::{Gltf, GltfLoaderSettings},
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    window::{PrimaryWindow, WindowResolution},
};
use std::{path::PathBuf, time::Instant};

#[derive(Resource)]
struct Audit {
    directory: PathBuf,
    fixtures: bool,
    frame: usize,
    stage: usize,
    waiting: bool,
    readbacks: Vec<usize>,
    samples: Vec<serde_json::Value>,
    started: Instant,
    finished: bool,
}
#[derive(Component)]
struct AuditHero {
    slug: String,
    origin: Vec3,
    local: bool,
}
#[derive(Component)]
struct Shot(usize);
const CAPTURE_FRAMES: [usize; 8] = [2, 5, 8, 11, 14, 17, 20, 23];

/// Starts an isolated render harness; never contacts the game or Studio service.
pub(crate) fn run(directory: PathBuf) {
    let fixtures = std::env::var("OMOBA_ANIMATION_QA_FIXTURES").is_ok_and(|v| v == "1");
    std::fs::create_dir_all(&directory).expect("QA output directory");
    let mut app = App::new();
    if let Ok(fixture_root) = std::env::var("OMOBA_ANIMATION_QA_ASSETS") {
        app.register_asset_source(
            "motion-qa",
            bevy::asset::io::AssetSourceBuilder::platform_default(&fixture_root, None),
        );
    }
    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                file_path: omoba_passport::assets::client_asset_root()
                    .to_string_lossy()
                    .into_owned(),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "OMOBA | shared run motion audit".into(),
                    resolution: WindowResolution::new(1440, 960).with_scale_factor_override(1.0),
                    ..default()
                }),
                ..default()
            }),
    )
    .insert_resource(bevy::winit::WinitSettings::continuous())
    .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f64(1.0 / 30.0),
    ))
    .insert_resource(ClearColor(Color::srgb(0.018, 0.029, 0.034)))
    .insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 550.0,
        ..default()
    })
    .init_resource::<AvatarAssetCache>()
    .add_plugins(ModelScalePlugin)
    .insert_resource(ModelScaleSettings {
        target_height: 1.72,
    })
    .insert_resource(Audit {
        directory,
        fixtures,
        frame: 0,
        stage: 0,
        waiting: false,
        readbacks: vec![],
        samples: vec![],
        started: Instant::now(),
        finished: false,
    })
    .add_systems(Startup, setup)
    .add_systems(PreUpdate, move_heroes)
    .add_systems(Update, crate::world::force_vrm_models_double_sided)
    .add_systems(
        PostUpdate,
        capture.after(bevy::transform::TransformSystems::Propagate),
    );
    crate::player::register_hero_animation_systems(&mut app);
    // The public game entry point returns (), so propagate the audit's failure
    // explicitly after the native runner has completed its normal shutdown.
    if let AppExit::Error(code) = app.run() {
        std::process::exit(i32::from(code.get()));
    }
}

fn setup(mut commands: Commands, assets: Res<AssetServer>, qa: Res<Audit>) {
    commands.spawn((
        Camera3d::default(),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: 7.5,
            },
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_xyz(0.0, 0.3, 18.0).looking_at(Vec3::new(0.0, 0.3, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 10500.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_xyz(5.0, 8.0, 9.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(18.0),
            left: Val::Px(24.0),
            ..default()
        },
        Text::new(if qa.fixtures {
            "QA | CLIPLESS VRM 0 / VRM 1 | shared runtime motion"
        } else {
            "QA | ALL 15 SHIPPED HEROES | shared runtime run"
        }),
        TextFont {
            font_size: 22.0,
            ..default()
        },
        TextColor(Color::srgb(0.76, 0.87, 0.82)),
    ));
    let entries: Vec<(String, String)> = if qa.fixtures {
        vec![
            (
                "clipless-vrm0".into(),
                "motion-qa://clipless-vrm0.glb".into(),
            ),
            (
                "clipless-vrm1-renamed".into(),
                "motion-qa://clipless-vrm1.glb".into(),
            ),
        ]
    } else {
        omoba_passport::avatars::avatar_roster()
            .iter()
            .filter(|a| a.passport.is_none())
            .map(|a| (a.slug.clone(), format!("avatars/{}.glb", a.slug)))
            .collect()
    };
    for (i, (slug, path)) in entries.into_iter().enumerate() {
        let origin = if qa.fixtures {
            Vec3::new((i as f32 - 0.5) * 3.5, -1.0, 0.0)
        } else {
            Vec3::new(
                (i % 5) as f32 * 2.15 - 4.3,
                1.8 - (i / 5) as f32 * 2.25,
                0.0,
            )
        };
        let gltf: Handle<Gltf> = assets
            .load_with_settings(path.clone(), |s: &mut GltfLoaderSettings| {
                s.include_source = true
            });
        let scene: Handle<Scene> = assets
            .load_with_settings(format!("{path}#Scene0"), |s: &mut GltfLoaderSettings| {
                s.include_source = true
            });
        let local = i % 2 == 0;
        let mut hero = commands.spawn((
            SceneRoot(scene),
            Transform::from_translation(origin)
                .with_rotation(Quat::from_rotation_y(std::f32::consts::PI + 0.35)),
            Visibility::default(),
            PlayerBody,
            CombatStats::default(),
            NetworkCharacterChoice(CharacterChoice::Cube),
            NetworkAvatar(Some(slug.clone())),
            PlayerCosmeticAction::default(),
            NormalizeModelScale::for_player_model(),
            ModelScaleSource {
                gltf,
                key: slug.clone(),
            },
            AuditHero {
                slug: slug.clone(),
                origin,
                local,
            },
        ));
        if local {
            hero.insert((
                Player,
                MovementTarget {
                    target: origin + Vec3::Z * 10.0,
                },
            ));
        } else {
            hero.insert(RemotePlayer);
        }
        let (left, top) = if qa.fixtures {
            (360.0 + i as f32 * 448.0, 635.0)
        } else {
            (
                98.0 + (i % 5) as f32 * 275.0,
                312.0 + (i / 5) as f32 * 288.0,
            )
        };
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(left),
                top: Val::Px(top),
                ..default()
            },
            Text::new(format!(
                "{}\n{} | RUN",
                slug,
                if local { "local" } else { "remote" }
            )),
            TextFont {
                font_size: 15.0,
                ..default()
            },
            TextColor(Color::srgb(0.68, 0.77, 0.73)),
        ));
    }
}

fn move_heroes(qa: Res<Audit>, mut heroes: Query<(&AuditHero, &mut Transform)>) {
    // The local path uses actual movement intent; remote path uses position deltas,
    // exactly the two production state-machine inputs. No animation state override.
    for (hero, mut transform) in &mut heroes {
        transform.translation = hero.origin + Vec3::X * ((qa.frame as f32 * 0.045).sin() * 0.08);
    }
}

fn capture(
    mut commands: Commands,
    mut qa: ResMut<Audit>,
    heroes: Query<(
        Entity,
        &AuditHero,
        Option<&RuntimeHumanoidPlayer>,
        &AnimationPlayer,
        &crate::player::PlayerAnimationBinding,
        &ModelScaleSource,
    )>,
    joints: Query<(
        Entity,
        &AnimatedBy,
        &Transform,
        Option<&Name>,
        Option<&AnimationTargetId>,
    )>,
    library: Res<crate::humanoid::HumanoidRuntimeLibrary>,
    errors: Query<&crate::humanoid::RuntimeHumanoidBindingError>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.finished {
        return;
    }
    let expected = if qa.fixtures { 2 } else { 15 };
    if qa.started.elapsed().as_secs() > 120 {
        finish(
            &mut qa,
            false,
            &format!(
                "timeout waiting for actual rigs or screenshot readback; binding errors: {:?}",
                errors.iter().map(|e| e.0.as_str()).collect::<Vec<_>>()
            ),
            &mut exit,
        );
        return;
    }
    if qa.waiting {
        let stage = qa.stage;
        if qa.readbacks.contains(&stage) {
            qa.waiting = false;
            qa.stage += 1;
        } else {
            return;
        }
    }
    if heroes
        .iter()
        .filter(|(_, _, r, _, _, _)| r.is_some())
        .count()
        != expected
    {
        return;
    }
    if qa.stage == CAPTURE_FRAMES.len() {
        let pass = measure_motion(&qa.samples, expected);
        // Close the native window before exit, matching the frontend capture
        // harness and releasing its renderer surface in the normal schedule.
        if let Ok((window, _)) = windows.single() {
            commands.entity(window).despawn();
        }
        finish(
            &mut qa,
            pass,
            "actual production pipeline and changing joint transforms",
            &mut exit,
        );
        return;
    }
    qa.frame += 1;
    if qa.frame < CAPTURE_FRAMES[qa.stage] {
        return;
    }
    let mut records = vec![];
    for (entity, hero, runtime, player, binding, source) in &heroes {
        let semantics = library.semantic_nodes(&source.gltf);
        let pose:Vec<_>=joints.iter().filter(|(_,by,_,_,_)|by.0==entity).map(|(joint,_,transform,name,target)|serde_json::json!({
            "entity":joint.to_bits(),"name":name.map(Name::as_str),
            "semantic":semantics.and_then(|bones| bones.iter().find(|(_,node)| target.is_some_and(|id| *id==crate::humanoid::humanoid_target_id(**node))).map(|(name,_)| name)),
            "rotation":transform.rotation.to_array(),"translation":transform.translation.to_array(),
            "finite":transform.rotation.is_finite() && transform.translation.is_finite()
        })).collect();
        records.push(serde_json::json!({"slug":hero.slug,"local":hero.local,"runtime":runtime.is_some(),"running":binding.is_running(),"active_animations":player.playing_animations().count(),"pose":pose}));
    }
    let size = windows
        .single()
        .map(|(_, w)| [w.physical_width(), w.physical_height()])
        .unwrap_or_default();
    let stage = qa.stage;
    let frame = qa.frame;
    qa.samples
        .push(serde_json::json!({"stage":stage,"frame":frame,"size":size,"heroes":records}));
    commands
        .spawn((Screenshot::primary_window(), Shot(stage)))
        .observe(save_to_disk(
            qa.directory.join(format!("run-{stage:02}.png")),
        ))
        .observe(readback);
    qa.waiting = true;
}
fn readback(event: On<ScreenshotCaptured>, shots: Query<&Shot>, mut qa: ResMut<Audit>) {
    if let Ok(shot) = shots.get(event.entity) {
        qa.readbacks.push(shot.0);
    }
}
fn measure_motion(samples: &[serde_json::Value], expected: usize) -> bool {
    if samples.len() != 8 {
        return false;
    }
    let Some(first) = samples[0]["heroes"].as_array() else {
        return false;
    };
    // A scene refresh can replace imported animation ownership after a valid
    // first frame. Every captured phase must retain a complete, finite pose.
    let valid_frames = samples.iter().all(|sample| {
        sample["heroes"].as_array().is_some_and(|heroes| {
            heroes.len() == expected
                && heroes.iter().all(|hero| {
                    hero["runtime"] == true
                        && hero["running"] == true
                        && hero["active_animations"] == 1
                        && hero["pose"].as_array().is_some_and(|pose| {
                            pose.len() >= 15
                                && pose.iter().all(|bone| bone["finite"] == true)
                                && [
                                    "hips",
                                    "leftUpperLeg",
                                    "rightUpperLeg",
                                    "leftLowerLeg",
                                    "rightLowerLeg",
                                    "leftFoot",
                                    "rightFoot",
                                ]
                                .iter()
                                .all(|name| pose.iter().any(|bone| bone["semantic"] == *name))
                        })
                })
        })
    });
    valid_frames
        && first.iter().all(|hero| {
            let slug = &hero["slug"];
            let Some(pose) = hero["pose"].as_array() else {
                return false;
            };
            let changed = |semantic: &str, field: &str| {
                let Some(bone) = pose.iter().find(|b| b["semantic"] == semantic) else {
                    return false;
                };
                samples.iter().skip(1).any(|sample| {
                    sample["heroes"].as_array().is_some_and(|hs| {
                        hs.iter().any(|h| {
                            &h["slug"] == slug
                                && h["pose"].as_array().is_some_and(|ps| {
                                    ps.iter().any(|b| {
                                        b["entity"] == bone["entity"] && b[field] != bone[field]
                                    })
                                })
                        })
                    })
                })
            };
            pose.len() >= 15
                && hero["active_animations"] == 1
                && hero["running"] == true
                && pose.iter().all(|b| b["finite"] == true)
                && [
                    "leftUpperLeg",
                    "rightUpperLeg",
                    "leftLowerLeg",
                    "rightLowerLeg",
                    "leftFoot",
                    "rightFoot",
                ]
                .iter()
                .all(|s| changed(s, "rotation"))
                && changed("hips", "translation")
        })
}
fn finish(qa: &mut Audit, pass: bool, reason: &str, exit: &mut MessageWriter<AppExit>) {
    let result = serde_json::json!({"pass":pass,"reason":reason,"fixture":"scripted movement through actual production animation systems; no server/Studio claim","clipless":qa.fixtures,"samples":qa.samples,"readbacks":qa.readbacks});
    std::fs::write(
        qa.directory.join("motion-audit.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .expect("write QA report");
    qa.finished = true;
    exit.write(if pass {
        AppExit::Success
    } else {
        AppExit::error()
    });
}
