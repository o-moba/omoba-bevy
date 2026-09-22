use super::*;
use bevy::{
    animation::AnimatedBy,
    asset::AssetApp,
    gltf::{GltfNode, GltfSkin},
    mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    time::TimeUpdateStrategy,
};
use omoba_passport::humanoid::VrmVersion;
use std::time::Duration;

fn motion() -> SharedHumanoidMotion {
    SharedHumanoidMotion::parse(include_str!(
        "../../assets/animations/humanoid-motion-v1.json"
    ))
    .unwrap()
}

fn rig(slug: &str) -> HumanoidRig {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets/avatars")
        .join(format!("{slug}.glb"));
    HumanoidRig::from_glb(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn all_fifteen_shipped_rigs_retarget_actual_run_with_separate_walk_and_all_states() {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../../assets/avatars/manifest.json")).unwrap();
    let motion = motion();
    assert_eq!(motion.clips["run"].source_clip, "Sprint_Loop");
    assert_eq!(motion.clips["walk"].source_clip, "Walk_Loop");
    let avatars = manifest["avatars"].as_array().unwrap();
    assert_eq!(avatars.len(), 15);
    for avatar in avatars {
        let slug = avatar["slug"].as_str().unwrap();
        let rig = rig(slug);
        let (clips, animated_nodes) = retarget::retarget_all(&rig, &motion).unwrap();
        assert_eq!(clips.len(), 6, "{slug}");
        assert_eq!(clips["run"].duration(), motion.clips["run"].duration);
        let first = retarget::sample_frame(&rig, &motion, "run", 0).unwrap();
        let last =
            retarget::sample_frame(&rig, &motion, "run", motion.clips["run"].times.len() - 1)
                .unwrap();
        assert!(
            first.1.distance(last.1) < 0.00001,
            "{slug}: run hips loop seam"
        );
        for bone in [
            "hips",
            "leftUpperLeg",
            "leftLowerLeg",
            "leftFoot",
            "rightUpperLeg",
            "rightLowerLeg",
            "rightFoot",
        ] {
            let node = rig.bones[bone];
            assert!(animated_nodes.contains(&node));
            let mut max_change = 0.0_f32;
            for sample in 1..motion.clips["run"].times.len() {
                let frame = retarget::sample_frame(&rig, &motion, "run", sample).unwrap();
                assert!(frame.0[node].is_finite());
                assert!(frame.1.is_finite());
                max_change = max_change.max(frame.0[node].angle_between(first.0[node]).abs());
            }
            assert!(
                max_change > 0.005,
                "{slug}/{bone}: static run joint ({max_change})"
            );
            assert!(
                first.0[node].angle_between(last.0[node]).abs() < 0.001,
                "{slug}/{bone}: loop seam"
            );
        }
        assert!(
            clips["run"]
                .curves()
                .contains_key(&humanoid_target_id(rig.bones["hips"]))
        );
    }
}

#[test]
fn retarget_preserves_local_pose_under_rotated_scaled_translated_ancestry() {
    let base = rig("agnes");
    let motion = motion();
    let mut transformed = base.clone();
    // Prepend a parent with arbitrary rotation, uniform scale and translation.
    let extra = transformed.nodes.len();
    for root in transformed.scene_roots.clone() {
        transformed.nodes[root].parent = Some(extra);
    }
    transformed.nodes.push(omoba_passport::humanoid::RigNode {
        parent: None,
        translation: [11.0, -4.0, 7.0],
        rotation: Quat::from_euler(EulerRot::XYZ, 0.31, 0.73, -0.22).to_array(),
        scale: [2.5; 3],
    });
    transformed.scene_roots = vec![extra];
    transformed.topological_order.insert(0, extra);
    for sample in [0, 4, 9, 15] {
        let original = retarget::sample_frame(&base, &motion, "run", sample).unwrap();
        let changed = retarget::sample_frame(&transformed, &motion, "run", sample).unwrap();
        for &node in base.bones.values() {
            assert!(
                original.0[node].angle_between(changed.0[node]).abs() < 0.001,
                "node {node}"
            );
        }
        assert!(original.1.distance(changed.1) < 0.0001);
    }
}

#[test]
fn optional_bones_and_vrm1_thumb_metadata_use_correct_motion_streams() {
    let mut vrm0 = rig("agnes");
    let motion = motion();
    let first = retarget::sample_frame(&vrm0, &motion, "run", 5).unwrap();
    let mut vrm1 = vrm0.clone();
    vrm1.version = VrmVersion::Vrm1;
    for side in ["left", "right"] {
        let proximal = vrm1.bones.remove(&format!("{side}ThumbProximal"));
        let intermediate = vrm1.bones.remove(&format!("{side}ThumbIntermediate"));
        if let Some(node) = proximal {
            vrm1.bones.insert(format!("{side}ThumbMetacarpal"), node);
        }
        if let Some(node) = intermediate {
            vrm1.bones.insert(format!("{side}ThumbProximal"), node);
        }
    }
    let second = retarget::sample_frame(&vrm1, &motion, "run", 5).unwrap();
    for &node in vrm0.bones.values() {
        assert!(first.0[node].angle_between(second.0[node]).abs() < 0.001);
    }
    for optional in [
        "upperChest",
        "neck",
        "leftShoulder",
        "rightShoulder",
        "leftToes",
        "rightToes",
    ] {
        vrm0.bones.remove(optional);
    }
    assert!(retarget::retarget_all(&vrm0, &motion).is_ok());
}

fn empty_gltf(nodes: Vec<Handle<GltfNode>>, skin: Handle<GltfSkin>) -> Gltf {
    Gltf {
        scenes: vec![],
        named_scenes: default(),
        meshes: vec![],
        named_meshes: default(),
        materials: vec![],
        named_materials: default(),
        nodes,
        named_nodes: default(),
        skins: vec![skin],
        named_skins: default(),
        default_scene: None,
        animations: vec![],
        named_animations: default(),
        source: None,
    }
}

fn spawn_instance(
    world: &mut World,
    model: &Handle<Gltf>,
    rig: &HumanoidRig,
    inverse: Handle<SkinnedMeshInverseBindposes>,
) -> (Entity, Vec<Entity>) {
    let root = world
        .spawn((
            Transform::from_xyz(17.0, 0.0, -9.0),
            RuntimeHumanoidRequest {
                model: model.clone(),
            },
        ))
        .id();
    let entities: Vec<_> = rig
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            world
                .spawn((
                    // Deliberately unrelated names: only node indices/skin identities count.
                    Name::new(format!("unrecognizable node {index}")),
                    Transform {
                        translation: Vec3::from_array(node.translation),
                        rotation: Quat::from_array(node.rotation),
                        scale: Vec3::from_array(node.scale),
                    },
                ))
                .id()
        })
        .collect();
    for (index, node) in rig.nodes.iter().enumerate() {
        world
            .entity_mut(entities[index])
            .insert(ChildOf(node.parent.map_or(root, |parent| entities[parent])));
    }
    world.spawn((
        Transform::default(),
        ChildOf(root),
        SkinnedMesh {
            inverse_bindposes: inverse,
            joints: entities.clone(),
        },
    ));
    (root, entities)
}

#[test]
fn actual_ecs_clipless_binding_survives_async_readiness_scene_refresh_and_despawn() {
    let rig = rig("agnes");
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        TransformPlugin,
        AnimationPlugin,
    ))
    .init_asset::<Gltf>()
    .init_asset::<GltfNode>()
    .init_asset::<GltfSkin>()
    .init_asset::<SkinnedMeshInverseBindposes>()
    .init_resource::<HumanoidRuntimeLibrary>()
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
        1.0 / 60.0,
    )))
    .add_systems(
        PostUpdate,
        bind_runtime_humanoids.before(bevy::app::AnimationSystems),
    );
    let inverse = app
        .world_mut()
        .resource_mut::<Assets<SkinnedMeshInverseBindposes>>()
        .add(vec![Mat4::IDENTITY; rig.nodes.len()]);
    let node_handles: Vec<_> = rig
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            app.world_mut()
                .resource_mut::<Assets<GltfNode>>()
                .add(GltfNode {
                    index,
                    name: format!("asset node {index}"),
                    children: vec![],
                    mesh: None,
                    skin: None,
                    transform: Transform {
                        translation: Vec3::from_array(node.translation),
                        rotation: Quat::from_array(node.rotation),
                        scale: Vec3::from_array(node.scale),
                    },
                    is_animation_root: false,
                    extras: None,
                })
        })
        .collect();
    let skin = app
        .world_mut()
        .resource_mut::<Assets<GltfSkin>>()
        .add(GltfSkin {
            index: 0,
            name: "skin".into(),
            joints: node_handles.clone(),
            inverse_bind_matrices: inverse.clone(),
            extras: None,
        });
    let model = app
        .world_mut()
        .resource_mut::<Assets<Gltf>>()
        .add(empty_gltf(node_handles, skin));
    let (first, first_joints) = spawn_instance(app.world_mut(), &model, &rig, inverse.clone());
    let (second, second_joints) = spawn_instance(app.world_mut(), &model, &rig, inverse);
    app.update(); // Scene may exist before its shared motion asset is prepared.
    assert!(app.world().get::<RuntimeHumanoidPlayer>(first).is_none());
    let (clips, animated_nodes) = retarget::retarget_all(&rig, &motion()).unwrap();
    let mut handles: HashMap<_, _> = clips
        .into_iter()
        .map(|(name, clip)| {
            (
                name,
                app.world_mut()
                    .resource_mut::<Assets<AnimationClip>>()
                    .add(clip),
            )
        })
        .collect();
    let runtime_clips = RuntimeHumanoidClips {
        idle: handles.remove("idle").unwrap(),
        walk: handles.remove("walk").unwrap(),
        run: handles.remove("run").unwrap(),
        attack: handles.remove("attack").unwrap(),
        cast: handles.remove("cast").unwrap(),
        death: handles.remove("death").unwrap(),
    };
    app.world_mut()
        .resource_mut::<HumanoidRuntimeLibrary>()
        .models
        .insert(
            model.id(),
            RuntimeRig {
                clips: runtime_clips.clone(),
                rig: rig.clone(),
                animated_nodes: animated_nodes.clone(),
            },
        );
    app.update();
    assert!(app.world().get::<RuntimeHumanoidPlayer>(first).is_some());
    for &node in rig.bones.values() {
        if app
            .world()
            .get::<AnimationTargetId>(first_joints[node])
            .is_some()
        {
            assert_eq!(
                app.world().get::<AnimatedBy>(first_joints[node]).unwrap().0,
                first
            );
            assert_eq!(
                app.world()
                    .get::<AnimatedBy>(second_joints[node])
                    .unwrap()
                    .0,
                second
            );
        }
    }
    let (graph, indices) = AnimationGraph::from_clips([runtime_clips.run, runtime_clips.idle]);
    let graph = app
        .world_mut()
        .resource_mut::<Assets<AnimationGraph>>()
        .add(graph);
    app.world_mut()
        .entity_mut(first)
        .insert(AnimationGraphHandle(graph.clone()));
    app.world_mut()
        .entity_mut(second)
        .insert(AnimationGraphHandle(graph.clone()));
    app.world_mut()
        .get_mut::<AnimationPlayer>(first)
        .unwrap()
        .play(indices[0])
        .repeat();
    app.world_mut()
        .get_mut::<AnimationPlayer>(second)
        .unwrap()
        .play(indices[1])
        .repeat();
    let leg = rig.bones["leftLowerLeg"];
    let before = app
        .world()
        .get::<Transform>(first_joints[leg])
        .unwrap()
        .rotation;
    for _ in 0..8 {
        app.update();
    }
    let running = app
        .world()
        .get::<Transform>(first_joints[leg])
        .unwrap()
        .rotation;
    let idle = app
        .world()
        .get::<Transform>(second_joints[leg])
        .unwrap()
        .rotation;
    assert!(running.angle_between(before).abs() > 0.01);
    assert!(running.angle_between(idle).abs() > 0.01);
    assert_eq!(
        app.world().get::<Transform>(first).unwrap().translation,
        Vec3::new(17.0, 0.0, -9.0)
    );
    // A scene refresh may restore imported target IDs/owners after Update,
    // while the external root and its active runtime Run still exist.
    #[derive(Resource)]
    struct ImportedTargetRefresh {
        targets: Vec<Entity>,
        owner: Entity,
        id: AnimationTargetId,
        damage: u8,
    }
    app.add_systems(bevy::app::SpawnScene, |world: &mut World| {
        let Some(refresh) = world.remove_resource::<ImportedTargetRefresh>() else {
            return;
        };
        for entity in refresh.targets {
            let mut target = world.entity_mut(entity);
            match refresh.damage {
                0 => {
                    target.insert((refresh.id, AnimatedBy(refresh.owner)));
                }
                1 => {
                    target.remove::<AnimationTargetId>();
                }
                _ => {
                    target.remove::<AnimatedBy>();
                }
            }
        }
    });
    let imported_player = app
        .world_mut()
        .spawn((
            Transform::default(),
            ChildOf(first),
            AnimationPlayer::default(),
        ))
        .id();
    let imported_id = AnimationTargetId::from_name(&Name::new("imported leg"));
    for damage in 0..3 {
        let elapsed = app
            .world()
            .get::<AnimationPlayer>(first)
            .unwrap()
            .animation(indices[0])
            .unwrap()
            .elapsed();
        app.world_mut()
            .entity_mut(imported_player)
            .insert(AnimationGraphHandle(graph.clone()));
        app.world_mut()
            .get_mut::<AnimationPlayer>(imported_player)
            .unwrap()
            .play(indices[0])
            .repeat();
        app.world_mut().insert_resource(ImportedTargetRefresh {
            targets: animated_nodes
                .iter()
                .map(|&node| first_joints[node])
                .collect(),
            owner: imported_player,
            id: imported_id,
            damage,
        });
        app.update();
        for &node in &animated_nodes {
            assert_eq!(
                app.world().get::<AnimatedBy>(first_joints[node]).unwrap().0,
                first
            );
            assert_eq!(
                *app.world()
                    .get::<AnimationTargetId>(first_joints[node])
                    .unwrap(),
                humanoid_target_id(node)
            );
            assert_eq!(
                app.world()
                    .get::<AnimatedBy>(second_joints[node])
                    .unwrap()
                    .0,
                second
            );
        }
        assert_eq!(
            app.world().get::<AnimationGraphHandle>(first).unwrap().0,
            graph
        );
        assert!(
            app.world()
                .get::<AnimationPlayer>(first)
                .unwrap()
                .animation(indices[0])
                .unwrap()
                .elapsed()
                > elapsed,
            "binding repair must preserve and advance the active Run"
        );
        assert_eq!(
            app.world()
                .get::<AnimationPlayer>(imported_player)
                .unwrap()
                .playing_animations()
                .count(),
            0
        );
        assert!(
            app.world()
                .get::<AnimationGraphHandle>(imported_player)
                .is_none()
        );
        assert!(
            app.world()
                .get::<AnimationPlayer>(second)
                .unwrap()
                .is_playing_animation(indices[1])
        );
    }
    for _ in 0..8 {
        app.update();
    }
    assert!(
        app.world()
            .get::<Transform>(first_joints[leg])
            .unwrap()
            .rotation
            .angle_between(running)
            .abs()
            > 0.01,
        "Run must keep changing the pose after scene refresh"
    );
    app.world_mut().entity_mut(first).despawn();
    app.update();
    assert!(app.world().get_entity(first_joints[leg]).is_err());
    assert!(app.world().get::<RuntimeHumanoidPlayer>(second).is_some());
    // Replacement on an existing owner must stop its old pose even while the
    // new model is not ready. Equal node indices do not imply equal models.
    let new_model = app
        .world_mut()
        .resource_mut::<Assets<Gltf>>()
        .reserve_handle();
    app.world_mut()
        .entity_mut(second)
        .insert(RuntimeHumanoidRequest { model: new_model });
    app.update();
    assert!(app.world().get::<RuntimeHumanoidPlayer>(second).is_none());
    assert!(app.world().get::<AnimatedBy>(second_joints[leg]).is_none());
    assert!(app.world().get::<AnimationGraphHandle>(second).is_none());
}

#[test]
fn malformed_shared_motion_is_rejected_before_runtime_assets_exist() {
    let mut value: serde_json::Value = serde_json::from_str(include_str!(
        "../../assets/animations/humanoid-motion-v1.json"
    ))
    .unwrap();
    value["clips"]["run"]["times"][1] = serde_json::json!(0);
    assert!(
        SharedHumanoidMotion::parse(&value.to_string())
            .unwrap_err()
            .contains("sample times")
    );
}

#[test]
fn embedded_alias_preserves_curves_with_safe_runtime_ids_and_rejects_partial_remaps() {
    use bevy::{animation::animated_field, math::curve::UnevenSampleAutoCurve};
    let rig = rig("agnes");
    let motion = motion();
    let (_, animated_nodes) = retarget::retarget_all(&rig, &motion).unwrap();
    let names: Vec<_> = (0..rig.nodes.len())
        .map(|node| format!("GltfNode{node}"))
        .collect();
    let leg = rig.bones["leftLowerLeg"];
    let mut path = vec![names[leg].as_str()];
    let mut cursor = leg;
    while let Some(parent) = rig.nodes[cursor].parent {
        path.push(names[parent].as_str());
        cursor = parent;
    }
    path.reverse();
    let original_id: AnimationTargetId = path.into_iter().collect();
    let mut original = AnimationClip::default();
    original.add_curve_to_target(
        original_id,
        AnimatableCurve::new(
            animated_field!(Transform::rotation),
            UnevenSampleAutoCurve::new([(0.0, Quat::IDENTITY), (0.7, Quat::from_rotation_x(0.8))])
                .unwrap(),
        ),
    );
    let remapped = embedded::remap(&rig, &animated_nodes, &names, &original).unwrap();
    assert_eq!(remapped.duration(), original.duration());
    assert_eq!(remapped.curves().len(), 1);
    assert!(remapped.curves().contains_key(&humanoid_target_id(leg)));
    assert!(!remapped.curves().contains_key(&original_id));
    original.add_curve_to_target(
        AnimationTargetId::from_name(&Name::new("unsupported accessory")),
        AnimatableCurve::new(
            animated_field!(Transform::translation),
            UnevenSampleAutoCurve::new([(0.0, Vec3::ZERO), (0.7, Vec3::X)]).unwrap(),
        ),
    );
    assert!(embedded::remap(&rig, &animated_nodes, &names, &original).is_err());
    assert_eq!(original.curves().len(), 2); // Immutable original clip is retained.
}
