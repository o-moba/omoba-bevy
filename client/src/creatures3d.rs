//! Original Verdant stone/crystal creatures. Shared geometry and materials
//! survive waves/respawns; articulated parts belong to each actor root.

use bevy::prelude::*;

use crate::model_scale::NormalizeModelScale;
use crate::net::{MinionBrainState, NetworkMinionBrainState, NeutralAiState, NeutralAiStateTag};
use crate::sprite::PlayerVisualMode;
use crate::team::Team;

#[derive(Resource)]
pub(crate) struct CreatureAssets {
    block: Handle<Mesh>,
    stone: Handle<Mesh>,
    crystal: Handle<Mesh>,
    ivory: Handle<StandardMaterial>,
    brass: Handle<StandardMaterial>,
    jade: Handle<StandardMaterial>,
    azure: Handle<StandardMaterial>,
    aether: Handle<StandardMaterial>,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProceduralCreature {
    Minion(Team),
    WendigoGuardian,
}

#[derive(Component)]
pub(crate) struct CreaturePart {
    owner: Entity,
    rest: Transform,
    motion: PartMotion,
}

#[derive(Clone, Copy)]
enum PartMotion {
    Still,
    Leg(f32),
    Arm(f32),
}

/// A 2D startup allocates no procedural 3D resources or model loads.
pub(crate) fn setup_creature_assets(
    mut commands: Commands,
    mode: Res<PlayerVisualMode>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if *mode != PlayerVisualMode::Models3d {
        return;
    }
    let mut stone = Sphere::new(0.5)
        .mesh()
        .ico(1)
        .expect("fixed one-subdivision icosphere is valid");
    stone.duplicate_vertices();
    stone.compute_flat_normals();
    let mut material = |base_color, emissive, metallic| {
        materials.add(StandardMaterial {
            base_color,
            emissive,
            metallic,
            perceptual_roughness: if metallic > 0.0 { 0.43 } else { 0.82 },
            ..default()
        })
    };
    commands.insert_resource(CreatureAssets {
        block: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        stone: meshes.add(stone),
        crystal: meshes.add(Cone::new(0.5, 1.0).mesh().resolution(4)),
        ivory: material(Color::srgb(0.67, 0.72, 0.58), LinearRgba::BLACK, 0.0),
        brass: material(Color::srgb(0.58, 0.39, 0.13), LinearRgba::BLACK, 0.7),
        jade: material(
            Color::srgb(0.18, 0.65, 0.35),
            LinearRgba::new(0.04, 0.22, 0.07, 1.0),
            0.1,
        ),
        azure: material(
            Color::srgb(0.16, 0.46, 0.9),
            LinearRgba::new(0.03, 0.10, 0.32, 1.0),
            0.1,
        ),
        aether: material(
            Color::srgb(0.64, 0.36, 0.83),
            LinearRgba::new(0.15, 0.05, 0.22, 1.0),
            0.1,
        ),
    });
}

/// One hand-authored sentinel: +Z forward matches server yaw, rest soles Y=0.
pub(crate) fn spawn_creature(
    commands: &mut Commands,
    owner: Entity,
    kind: ProceduralCreature,
    assets: &CreatureAssets,
) {
    let accent = match kind {
        ProceduralCreature::Minion(Team::Green) => &assets.jade,
        ProceduralCreature::Minion(Team::Blue) => &assets.azure,
        ProceduralCreature::WendigoGuardian => &assets.aether,
    };
    let guardian = kind == ProceduralCreature::WendigoGuardian;
    commands.entity(owner).insert(kind).with_children(|parent| {
        let mut part = |name: &str,
                        mesh: &Handle<Mesh>,
                        material: &Handle<StandardMaterial>,
                        translation: Vec3,
                        scale: Vec3,
                        roll: f32,
                        motion: PartMotion| {
            let rest = Transform::from_translation(translation)
                .with_scale(scale)
                .with_rotation(Quat::from_rotation_z(roll));
            parent.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                rest,
                Visibility::default(),
                CreaturePart {
                    owner,
                    rest,
                    motion,
                },
                Name::new(format!("VerdantCreature-{name}")),
            ));
        };
        part(
            "torso",
            &assets.stone,
            &assets.ivory,
            Vec3::new(0.0, 0.87, 0.0),
            Vec3::new(if guardian { 1.22 } else { 0.94 }, 1.05, 0.68),
            0.0,
            PartMotion::Still,
        );
        part(
            "head",
            &assets.stone,
            &assets.ivory,
            Vec3::new(0.0, 1.49, 0.06),
            Vec3::new(0.61, 0.55, 0.52),
            0.0,
            PartMotion::Still,
        );
        part(
            "brow",
            &assets.block,
            &assets.brass,
            Vec3::new(0.0, 1.6, 0.3),
            Vec3::new(0.59, 0.10, 0.09),
            0.0,
            PartMotion::Still,
        );
        part(
            "heart-crystal",
            &assets.crystal,
            accent,
            Vec3::new(0.0, 0.94, 0.36),
            Vec3::new(0.42, 0.56, 0.20),
            0.0,
            PartMotion::Still,
        );
        for side in [-1.0, 1.0] {
            part(
                "leg",
                &assets.block,
                &assets.ivory,
                Vec3::new(side * 0.22, 0.25, 0.0),
                Vec3::new(0.25, 0.50, 0.35),
                0.0,
                PartMotion::Leg(side),
            );
            part(
                "shoulder",
                &assets.stone,
                accent,
                Vec3::new(side * 0.51, 1.14, 0.0),
                Vec3::splat(if guardian { 0.52 } else { 0.39 }),
                0.0,
                PartMotion::Still,
            );
            part(
                "arm",
                &assets.block,
                &assets.ivory,
                Vec3::new(side * 0.59, 0.77, 0.04),
                Vec3::new(0.26, 0.65, 0.28),
                side * 0.12,
                PartMotion::Arm(side),
            );
            part(
                "eye",
                &assets.block,
                accent,
                Vec3::new(side * 0.13, 1.48, 0.325),
                Vec3::new(0.10, 0.09, 0.05),
                0.0,
                PartMotion::Still,
            );
            if guardian {
                part(
                    "antler-branch",
                    &assets.block,
                    &assets.brass,
                    Vec3::new(side * 0.4, 1.81, 0.0),
                    Vec3::new(0.16, 0.66, 0.17),
                    -side * 0.62,
                    PartMotion::Still,
                );
                for (x, height) in [(0.36, 2.12), (0.64, 2.03), (0.87, 1.84)] {
                    part(
                        "antler-crystal",
                        &assets.crystal,
                        &assets.ivory,
                        Vec3::new(side * x, height, 0.0),
                        Vec3::new(0.16, 0.59, 0.16),
                        -side * 0.27,
                        PartMotion::Still,
                    );
                }
            }
        }
        match kind {
            ProceduralCreature::Minion(Team::Green) => part(
                "jade-crown",
                &assets.crystal,
                accent,
                Vec3::new(0.0, 1.90, 0.03),
                Vec3::new(0.40, 0.58, 0.38),
                0.0,
                PartMotion::Still,
            ),
            ProceduralCreature::Minion(Team::Blue) => {
                for side in [-1.0, 1.0] {
                    part(
                        "azure-twin-crown",
                        &assets.crystal,
                        accent,
                        Vec3::new(side * 0.2, 1.83, 0.03),
                        Vec3::new(0.28, 0.50, 0.30),
                        -side * 0.25,
                        PartMotion::Still,
                    );
                }
            }
            ProceduralCreature::WendigoGuardian => {}
        }
    });
}

/// Bounded state-driven motion creates no entities/assets. Wait for a measured
/// rest pose so normalization never depends on an arbitrary walking frame.
pub(crate) fn animate_creatures(
    time: Res<Time>,
    owners: Query<(
        &NormalizeModelScale,
        Option<&NetworkMinionBrainState>,
        Option<&NeutralAiStateTag>,
    )>,
    mut parts: Query<(&CreaturePart, &mut Transform)>,
) {
    for (part, mut transform) in &mut parts {
        let Ok((scale, minion, neutral)) = owners.get(part.owner) else {
            continue;
        };
        if scale.head_local_y.is_none() {
            continue;
        }
        let walking = minion.is_some_and(|state| {
            matches!(
                state.0,
                MinionBrainState::Marching | MinionBrainState::Chasing
            )
        }) || neutral.is_some_and(|state| state.0 == NeutralAiState::Aggro);
        let attacking = minion.is_some_and(|state| state.0 == MinionBrainState::Attacking);
        *transform = part.rest;
        let phase = time.elapsed_secs() * 8.0 + part.owner.index_u32() as f32 * 0.7;
        match part.motion {
            PartMotion::Leg(side) if walking => {
                // Feet only lift; the rest-ground plane is never penetrated.
                transform.translation.y += (phase.sin() * side).max(0.0) * 0.10;
                transform.translation.z += phase.sin() * side * 0.14;
            }
            PartMotion::Arm(side) if walking || attacking => {
                let angle = if attacking {
                    -0.65 - (phase * 1.4).sin() * 0.55
                } else {
                    phase.sin() * side * 0.28
                };
                transform.rotation = part.rest.rotation * Quat::from_rotation_x(angle);
                if attacking {
                    transform.translation.z += 0.13;
                }
            }
            _ => {}
        }
    }
}

/// Headless production-system fixture: real primitive bounds and transform
/// propagation, without a renderer, AssetServer, downloads, or a GPU.
#[cfg(test)]
pub(crate) fn test_app(mode: PlayerVisualMode) -> App {
    use bevy::camera::primitives::{Aabb, MeshAabb};
    use bevy::gltf::{Gltf, GltfMesh, GltfNode};
    use bevy::time::TimeUpdateStrategy;
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin))
        .insert_resource(mode)
        .insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0 / 60.0),
        ))
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .init_resource::<Assets<Gltf>>()
        .init_resource::<Assets<GltfNode>>()
        .init_resource::<Assets<GltfMesh>>()
        .add_plugins((
            crate::minions::MinionVisualsPlugin,
            crate::model_scale::ModelScalePlugin,
        ))
        .add_systems(
            PostUpdate,
            |mut commands: Commands,
             meshes: Res<Assets<Mesh>>,
             parts: Query<(Entity, &Mesh3d), Without<Aabb>>| {
                for (entity, mesh) in &parts {
                    if let Some(bounds) = meshes.get(&mesh.0).and_then(MeshAabb::compute_aabb) {
                        commands.entity(entity).insert(bounds);
                    }
                }
            },
        );
    app
}
