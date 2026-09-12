//! Shared procedural projectile geometry and short, authoritative-motion trails.
//! Disappearance is cleanup only: impacts come from confirmed combat events.
use std::collections::{HashMap, VecDeque};

use bevy::prelude::*;
use shared::combat::CombatEntityKind;

use crate::{
    combat_visuals::{
        CombatVisualProfile, CombatVisualRegistry, ProjectilePresentationRoot, ProjectileShape,
    },
    net::{
        NetworkAvatar, NetworkHeroClass, NetworkPlayerId, NetworkProjectile, NetworkSpriteCharacter,
    },
    sprite::PlayerVisualMode,
    team::Team,
    world2d::{layer, simulation_xz_to_render_xy},
};

const MAX_VISUALS: usize = 384;

pub struct ProjectileVisualsPlugin;
impl Plugin for ProjectileVisualsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_assets).add_systems(
            PostUpdate,
            (attach_visuals, update_visuals, draw_trails)
                .chain()
                .before(bevy::transform::TransformSystems::Propagate),
        );
    }
}

#[derive(Resource)]
struct ProjectileAssets {
    block: Handle<Mesh>,
    crystal: Handle<Mesh>,
    cone: Handle<Mesh>,
    white: Handle<StandardMaterial>,
    green: Handle<StandardMaterial>,
    blue: Handle<StandardMaterial>,
    profiles: HashMap<(u64, String), (Handle<StandardMaterial>, Option<Handle<Scene>>)>,
}

fn material(color: Color) -> StandardMaterial {
    StandardMaterial {
        base_color: color,
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    }
}

fn setup_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(ProjectileAssets {
        block: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        crystal: meshes.add(Sphere::new(1.0).mesh().ico(0).expect("icosahedron")),
        cone: meshes.add(Cone {
            radius: 1.0,
            height: 1.0,
        }),
        white: materials.add(material(Color::srgb(1.0, 0.97, 0.85))),
        green: materials.add(material(Color::srgb(0.15, 1.0, 0.35))),
        blue: materials.add(material(Color::srgb(0.2, 0.55, 1.0))),
        profiles: HashMap::new(),
    });
}

#[derive(Clone, Copy)]
struct TrailPoint {
    position: Vec3,
    age: f32,
}

#[derive(Component)]
struct ProjectileVisual {
    profile: CombatVisualProfile,
    previous: Vec3,
    age: f32,
    trail: VecDeque<TrailPoint>,
    facing: Entity,
    fallback: Entity,
    model: Option<(Entity, Handle<Scene>)>,
}

fn spawn_part(
    parent: &mut ChildSpawnerCommands,
    mesh: &Handle<Mesh>,
    material: &Handle<StandardMaterial>,
    position: Vec3,
    scale: Vec3,
    rotation: Quat,
    name: &str,
) {
    parent.spawn((
        Mesh3d(mesh.clone()),
        MeshMaterial3d(material.clone()),
        Transform::from_translation(position)
            .with_scale(scale)
            .with_rotation(rotation),
        Name::new(format!("Projectile-{name}")),
    ));
}

fn spawn_shape(
    parent: &mut ChildSpawnerCommands,
    shape: ProjectileShape,
    assets: &ProjectileAssets,
    tint: &Handle<StandardMaterial>,
    team: Team,
) {
    let block = &assets.block;
    match shape {
        ProjectileShape::Arrow => {
            spawn_part(
                parent,
                block,
                &assets.white,
                Vec3::new(0.0, 0.0, -0.15),
                Vec3::new(0.085, 0.085, 1.25),
                Quat::IDENTITY,
                "Arrow-Shaft",
            );
            spawn_part(
                parent,
                &assets.cone,
                tint,
                Vec3::new(0.0, 0.0, 0.65),
                Vec3::new(0.23, 0.55, 0.23),
                Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
                "Arrow-Head",
            );
            for angle in [0.0, std::f32::consts::FRAC_PI_2] {
                spawn_part(
                    parent,
                    block,
                    tint,
                    Vec3::new(0.0, 0.0, -0.58),
                    Vec3::new(0.48, 0.05, 0.33),
                    Quat::from_rotation_z(angle),
                    "Arrow-Fletching",
                );
            }
        }
        ProjectileShape::Arcane => {
            spawn_part(
                parent,
                &assets.crystal,
                tint,
                Vec3::ZERO,
                Vec3::new(0.33, 0.33, 0.74),
                Quat::IDENTITY,
                "Arcane-Core",
            );
            for angle in [0.0_f32, 2.094, 4.189] {
                spawn_part(
                    parent,
                    &assets.crystal,
                    &assets.white,
                    Vec3::new(angle.cos() * 0.45, angle.sin() * 0.45, -0.32),
                    Vec3::new(0.09, 0.09, 0.24),
                    Quat::IDENTITY,
                    "Arcane-Spark",
                );
            }
        }
        ProjectileShape::Holy => {
            spawn_part(
                parent,
                &assets.crystal,
                tint,
                Vec3::ZERO,
                Vec3::new(0.28, 0.42, 0.55),
                Quat::IDENTITY,
                "Holy-Diamond",
            );
            for index in 0..12 {
                let angle = index as f32 * std::f32::consts::TAU / 12.0;
                spawn_part(
                    parent,
                    block,
                    &assets.white,
                    Vec3::new(angle.cos() * 0.50, angle.sin() * 0.50, -0.18),
                    Vec3::new(0.28, 0.06, 0.065),
                    Quat::from_rotation_z(angle + std::f32::consts::FRAC_PI_2),
                    "Holy-Halo",
                );
            }
        }
        ProjectileShape::Crescent => {
            for index in 0..9 {
                let angle = -1.15 + index as f32 * 2.30 / 8.0;
                let taper = 1.0 - (angle / 1.5).abs() * 0.5;
                spawn_part(
                    parent,
                    block,
                    tint,
                    Vec3::new(angle.sin() * 0.95, 0.0, angle.cos() * 0.70 - 0.3),
                    Vec3::new(0.31, 0.075, 0.25 * taper),
                    Quat::from_rotation_y(angle),
                    "Warrior-Crescent",
                );
                spawn_part(
                    parent,
                    block,
                    &assets.white,
                    Vec3::new(angle.sin() * 1.0, 0.045, angle.cos() * 0.78 - 0.3),
                    Vec3::new(0.30, 0.03, 0.045),
                    Quat::from_rotation_y(angle),
                    "Warrior-Edge",
                );
            }
        }
        ProjectileShape::Bolt => {
            spawn_part(
                parent,
                &assets.crystal,
                tint,
                Vec3::ZERO,
                Vec3::new(0.25, 0.25, 0.70),
                Quat::IDENTITY,
                "Bolt-Core",
            );
            spawn_part(
                parent,
                block,
                &assets.white,
                Vec3::new(0.0, 0.0, -0.3),
                Vec3::new(0.10, 0.10, 0.9),
                Quat::IDENTITY,
                "Bolt-Streak",
            );
        }
    }
    spawn_part(
        parent,
        &assets.crystal,
        match team {
            Team::Green => &assets.green,
            Team::Blue => &assets.blue,
        },
        Vec3::new(0.0, 0.1, -0.6),
        Vec3::splat(0.15),
        Quat::IDENTITY,
        "Team-Cue",
    );
}

#[allow(clippy::too_many_arguments)]
fn attach_visuals(
    mut commands: Commands,
    mode: Res<PlayerVisualMode>,
    registry: Res<CombatVisualRegistry>,
    server: Option<Res<AssetServer>>,
    mut assets: ResMut<ProjectileAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    roots: Query<(Entity, &Transform, &NetworkProjectile), Without<ProjectileVisual>>,
    existing: Query<(), With<ProjectileVisual>>,
    owners: Query<(
        &NetworkPlayerId,
        Option<&NetworkHeroClass>,
        Option<&NetworkAvatar>,
        Option<&NetworkSpriteCharacter>,
    )>,
) {
    if *mode != PlayerVisualMode::Models3d {
        return;
    }
    let mut capacity = MAX_VISUALS.saturating_sub(existing.iter().count());
    for (owner, transform, projectile) in &roots {
        if capacity == 0 {
            break;
        }
        capacity -= 1;
        let identity = matches!(
            projectile.source_kind,
            CombatEntityKind::Player | CombatEntityKind::Unknown
        )
        .then(|| {
            owners
                .iter()
                .find(|(id, _, _, _)| id.0 == projectile.owner_id)
        })
        .flatten();
        let class = identity.and_then(|(_, class, _, _)| class.map(|class| class.0));
        let avatar =
            identity.and_then(|(_, _, avatar, _)| avatar.and_then(|avatar| avatar.0.as_deref()));
        let sprite =
            identity.and_then(|(_, _, _, sprite)| sprite.and_then(|sprite| sprite.0.as_deref()));
        let profile = registry
            .resolve(
                class,
                projectile.style,
                projectile.action_slot,
                avatar,
                sprite,
            )
            .clone();
        let key = (registry.revision(), profile.id.clone());
        let (tint, scene) = assets
            .profiles
            .entry(key)
            .or_insert_with(|| {
                (
                    materials.add(material(profile.color())),
                    profile.model.as_ref().and_then(|model| {
                        server.as_ref().map(|server| {
                            server.load(format!("{}#Scene{}", model.path, model.scene))
                        })
                    }),
                )
            })
            .clone();
        let facing = commands
            .spawn((
                Transform::default().with_scale(Vec3::splat(profile.scale)),
                Visibility::default(),
                ChildOf(owner),
                ProjectilePresentationRoot { owner },
                Name::new("Projectile-Facing"),
            ))
            .id();
        let fallback = commands
            .spawn((
                Transform::default(),
                Visibility::default(),
                ChildOf(facing),
                Name::new("Projectile-Procedural"),
            ))
            .with_children(|parent| {
                spawn_shape(parent, profile.shape, &assets, &tint, projectile.owner_team)
            })
            .id();
        let model = profile.model.as_ref().zip(scene).map(|(model, scene)| {
            let [x, y, z] = model.rotation_degrees.map(f32::to_radians);
            let entity = commands
                .spawn((
                    SceneRoot(scene.clone()),
                    Visibility::Hidden,
                    Transform::from_scale(Vec3::splat(model.scale))
                        .with_rotation(Quat::from_euler(EulerRot::XYZ, x, y, z)),
                    ChildOf(facing),
                    Name::new("Projectile-PackagedModel"),
                ))
                .id();
            (entity, scene)
        });
        commands.entity(owner).insert(ProjectileVisual {
            profile,
            previous: transform.translation,
            age: 0.0,
            trail: VecDeque::new(),
            facing,
            fallback,
            model,
        });
    }
}

fn facing_rotation(direction: Vec3, displacement: Vec3) -> Quat {
    let direction = [direction, displacement, Vec3::Z]
        .into_iter()
        .find_map(|value| value.is_finite().then(|| value.try_normalize()).flatten())
        .unwrap_or(Vec3::Z);
    Quat::from_rotation_arc(Vec3::Z, direction)
}

fn advance_trail(visual: &mut ProjectileVisual, position: Vec3, delta: f32) {
    visual.age += delta.max(0.0);
    for point in &mut visual.trail {
        point.age += delta.max(0.0);
    }
    visual
        .trail
        .retain(|point| point.age < visual.profile.trail.seconds);
    if position.is_finite() && position.distance_squared(visual.previous) > 0.000_001 {
        // Teleports/reconnect corrections are not a beam across the map.
        if position.distance_squared(visual.previous) > 30.0_f32.powi(2) {
            visual.trail.clear();
        } else {
            visual.trail.push_back(TrailPoint {
                position: visual.previous,
                age: 0.0,
            });
        }
        while visual.trail.len() > visual.profile.trail.samples {
            visual.trail.pop_front();
        }
    }
    visual.previous = position;
}

fn update_visuals(
    time: Res<Time>,
    server: Option<Res<AssetServer>>,
    scenes: Option<Res<Assets<Scene>>>,
    mut roots: Query<(&Transform, &NetworkProjectile, &mut ProjectileVisual)>,
    mut transforms: Query<&mut Transform, Without<NetworkProjectile>>,
    mut visibility: Query<&mut Visibility>,
) {
    for (transform, projectile, mut visual) in &mut roots {
        let rotation = facing_rotation(
            projectile.direction,
            transform.translation - visual.previous,
        );
        if let Ok(mut facing) = transforms.get_mut(visual.facing) {
            facing.rotation = rotation;
        }
        advance_trail(&mut visual, transform.translation, time.delta_secs());
        if let Some((entity, handle)) = &visual.model {
            let ready = server
                .as_ref()
                .is_some_and(|server| server.is_loaded_with_dependencies(handle.id()))
                && scenes.as_ref().is_some_and(|scenes| {
                    scenes.get(handle).is_some_and(|scene| {
                        scene
                            .world
                            .components()
                            .component_id::<Mesh3d>()
                            .is_some_and(|id| {
                                scene.world.archetypes().iter().any(|archetype| {
                                    !archetype.is_empty() && archetype.contains(id)
                                })
                            })
                    })
                });
            if let Ok(mut state) = visibility.get_mut(*entity) {
                *state = if ready {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
            }
            if let Ok(mut state) = visibility.get_mut(visual.fallback) {
                *state = if ready {
                    Visibility::Hidden
                } else {
                    Visibility::Inherited
                };
            }
        }
    }
}

fn draw_trails(
    mut gizmos: Gizmos,
    mode: Res<PlayerVisualMode>,
    roots: Query<(&Transform, &ProjectileVisual)>,
) {
    for (transform, visual) in &roots {
        let points: Vec<_> = visual
            .trail
            .iter()
            .copied()
            .chain(std::iter::once(TrailPoint {
                position: transform.translation,
                age: 0.0,
            }))
            .collect();
        for pair in points.windows(2) {
            let alpha =
                (1.0 - pair[0].age / visual.profile.trail.seconds.max(0.001)).clamp(0.0, 1.0) * 0.7;
            let color = visual.profile.color().with_alpha(alpha);
            let side = (pair[1].position - pair[0].position)
                .cross(Vec3::Y)
                .normalize_or_zero()
                * visual.profile.trail.width;
            for offset in [-1.0, 0.0, 1.0] {
                let start = pair[0].position + side * offset;
                let end = pair[1].position + side * offset;
                if *mode == PlayerVisualMode::Models3d {
                    gizmos.line(start, end, color);
                } else {
                    gizmos.line(
                        simulation_xz_to_render_xy(start).extend(layer::PROJECTILE),
                        simulation_xz_to_render_xy(end).extend(layer::PROJECTILE),
                        color,
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::combat::ProjectileStyle;
    #[test]
    fn homing_direction_and_invalid_inputs_produce_finite_forward_orientation() {
        for direction in [Vec3::X, Vec3::NEG_Z, Vec3::Y, Vec3::new(1.0, 0.4, 0.2)] {
            assert!(
                (facing_rotation(direction, Vec3::ZERO) * Vec3::Z).distance(direction.normalize())
                    < 0.0001
            );
        }
        assert!((facing_rotation(Vec3::NAN, Vec3::X) * Vec3::Z).distance(Vec3::X) < 0.0001);
        assert_eq!(facing_rotation(Vec3::ZERO, Vec3::ZERO), Quat::IDENTITY);
    }
    #[test]
    fn trail_is_bounded_expires_and_does_not_bridge_teleports() {
        let profile = CombatVisualRegistry::default()
            .resolve_style(ProjectileStyle::Arrow)
            .clone();
        let mut visual = ProjectileVisual {
            profile,
            previous: Vec3::ZERO,
            age: 0.0,
            trail: VecDeque::new(),
            facing: Entity::PLACEHOLDER,
            fallback: Entity::PLACEHOLDER,
            model: None,
        };
        for index in 1..100 {
            advance_trail(&mut visual, Vec3::X * index as f32, 0.01);
        }
        assert!(visual.trail.len() <= visual.profile.trail.samples);
        advance_trail(&mut visual, Vec3::X * 1000.0, 0.01);
        assert!(visual.trail.is_empty());
        advance_trail(&mut visual, Vec3::X * 1001.0, 0.01);
        advance_trail(&mut visual, Vec3::X * 1001.0, 1.0);
        assert!(visual.trail.is_empty());
    }
    #[test]
    fn spawn_despawn_reuses_meshes_and_materials_and_cleans_children() {
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<CombatVisualRegistry>()
            .insert_resource(PlayerVisualMode::Models3d)
            .add_systems(Startup, setup_assets)
            .add_systems(Update, attach_visuals);
        app.update();
        let mut counts = None;
        for _ in 0..3 {
            let entities: Vec<_> = [
                ProjectileStyle::Arrow,
                ProjectileStyle::Arcane,
                ProjectileStyle::Holy,
                ProjectileStyle::Crescent,
            ]
            .into_iter()
            .map(|style| {
                app.world_mut()
                    .spawn((
                        Transform::default(),
                        NetworkProjectile {
                            id: 1,
                            owner_id: 1,
                            owner_team: Team::Green,
                            source_kind: CombatEntityKind::Unknown,
                            style,
                            action_slot: None,
                            direction: Vec3::Z,
                        },
                    ))
                    .id()
            })
            .collect();
            app.update();
            let current = (
                app.world().resource::<Assets<Mesh>>().len(),
                app.world().resource::<Assets<StandardMaterial>>().len(),
            );
            if let Some(expected) = counts {
                assert_eq!(current, expected);
            } else {
                counts = Some(current);
            }
            for entity in entities {
                app.world_mut().entity_mut(entity).despawn();
            }
            app.update();
            assert_eq!(
                app.world_mut().query::<&Mesh3d>().iter(app.world()).count(),
                0
            );
        }
        assert_eq!(counts.unwrap().0, 3);
    }
}
