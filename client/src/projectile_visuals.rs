//! Shared procedural projectile geometry and short, authoritative-motion trails.
//! Disappearance is cleanup only: impacts come from confirmed combat events.
use std::collections::{HashMap, VecDeque};

use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::prelude::*;
use shared::combat::CombatEntityKind;

use crate::{
    combat_visuals::{
        CombatVisualProfile, CombatVisualRegistry, FlightBody, FormPaint, FormPart,
        ProjectilePresentationRoot, ProjectileShape, known_basic,
    },
    net::{
        NetworkAvatar, NetworkHeroClass, NetworkPlayerId, NetworkProjectile, NetworkSpriteCharacter,
    },
    skill_presentation::{
        bodies::{PartMesh, VfxMeshes},
        vocab::ProjectilePresentation,
    },
    sprite::{PlayerVisualMode, in_models3d},
    team::Team,
    world2d::{layer, simulation_xz_to_render_xy},
};

pub(crate) const MAX_VISUALS: usize = 384;
/// Gain of the light parts of a form and of its white-hot core. The light is brighter
/// than a `shape` body; far more gain turns a saturated profile colour pale.
const FORM_GAIN: f32 = 3.6;
/// Length of a long body that is drawn at once behind the place its projectile was first
/// seen at. A projectile is first seen about half a unit ahead of its hero, so this much
/// of it ends inside the hero that threw it and not behind its back.
const FORM_START: f32 = 0.7;
/// A body that turns about its position or lies across it is first seen at this share of
/// its size and is whole after this distance of flight: on the frame of the cast it is a
/// thing in the hand, not a plate over the head of the hero that threw it.
const FORM_SEED: f32 = 0.4;
const FORM_GROWTH: f32 = 0.9;
const CORE_GAIN: f32 = 5.0;
/// Lightness and opacity of the deep shade of a form's colour.
const ECHO_LIGHTNESS: f32 = 0.36;
const ECHO_ALPHA: f32 = 0.9;
/// Height above the terrain of a body that slides along the ground.
const GROUND_LIFT: f32 = 0.12;

pub struct ProjectileVisualsPlugin;
impl Plugin for ProjectileVisualsPlugin {
    fn build(&self, app: &mut App) {
        crate::vfx_clock::ensure(app);
        app.add_systems(Startup, setup_assets.run_if(in_models3d()))
            .add_systems(
                PostUpdate,
                (attach_visuals, update_visuals, animate_orbits)
                    .chain()
                    .before(bevy::transform::TransformSystems::Propagate)
                    .run_if(in_models3d()),
            )
            .add_systems(
                PostUpdate,
                draw_trails
                    .after(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate)
                    .after(bevy::transform::TransformSystems::Propagate),
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
    /// The white-hot paint of a form.
    core: Handle<StandardMaterial>,
    profiles: HashMap<(u64, String), (Handle<StandardMaterial>, Option<Handle<WorldAsset>>)>,
    /// The light and the deep shade of each profile that is drawn as a form.
    forms: HashMap<(u64, String), (Handle<StandardMaterial>, Handle<StandardMaterial>)>,
}

fn material(color: Color) -> StandardMaterial {
    StandardMaterial {
        base_color: crate::game_vfx::hdr_tint(color, 2.8),
        fog_enabled: false,
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    }
}

/// A part of a form: a flat silhouette is seen from both sides.
fn form_material(color: Color, alpha_mode: AlphaMode) -> StandardMaterial {
    StandardMaterial {
        base_color: color,
        fog_enabled: false,
        unlit: true,
        double_sided: true,
        cull_mode: None,
        alpha_mode,
        ..default()
    }
}

/// The deep shade of a colour: its hue and saturation, darker than any ground.
fn echo(color: Color) -> Color {
    let hsl = Hsla::from(color);
    Hsla::new(hsl.hue, hsl.saturation, ECHO_LIGHTNESS, ECHO_ALPHA).into()
}

fn setup_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(ProjectileAssets {
        block: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        crystal: meshes.add(Sphere::new(1.0).mesh().ico(1).expect("icosphere")),
        cone: meshes.add(Cone {
            radius: 1.0,
            height: 1.0,
        }),
        white: materials.add(material(Color::srgb(0.94, 1.0, 1.0))),
        green: materials.add(material(Color::srgb(0.15, 1.0, 0.35))),
        blue: materials.add(material(Color::srgb(0.2, 0.55, 1.0))),
        core: materials.add(form_material(
            crate::game_vfx::hdr_tint(Color::srgb(1.0, 0.97, 0.82), CORE_GAIN),
            AlphaMode::Opaque,
        )),
        profiles: HashMap::new(),
        forms: HashMap::new(),
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
    /// Distance flown since the projectile was first seen.
    travelled: f32,
    age: f32,
    trail: VecDeque<TrailPoint>,
    facing: Entity,
    fallback: Entity,
    model: Option<(Entity, Handle<WorldAsset>)>,
}

/// The parts of a form or of a reach streak, moved by their transforms alone.
#[derive(Component)]
struct FormBody {
    parts: Vec<(Entity, FormPart)>,
    /// Keeps two projectiles from moving in step.
    phase: f32,
    /// The body slides along the terrain under the replicated position.
    hugs_ground: bool,
    /// `combat_visuals::form_heading_span` of a body laid out along its path.
    span: Option<(f32, f32)>,
    /// A form that stays centred on its projectile: it grows over its first stretch of
    /// flight. The reach streak of a melee contact is whole at once.
    centred: bool,
}

/// Capture evidence: what stands for a projectile.
#[cfg(feature = "qa")]
#[derive(Component, Clone, Debug)]
pub(crate) struct ProjectileBodyVisual {
    pub profile: String,
    pub body: FlightBody,
    pub parts: usize,
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
    let mut part = parent.spawn((
        Mesh3d(mesh.clone()),
        MeshMaterial3d(material.clone()),
        Transform::from_translation(position)
            .with_scale(scale)
            .with_rotation(rotation),
        NotShadowCaster,
        NotShadowReceiver,
        Name::new(format!("Projectile-{name}")),
    ));
    if name == "Arcane-Spark" {
        part.insert(OrbitingSpark(position.y.atan2(position.x)));
    }
}
#[derive(Component)]
struct OrbitingSpark(f32);
fn animate_orbits(
    clock: Res<crate::vfx_clock::VfxClock>,
    mut sparks: Query<(&OrbitingSpark, &mut Transform)>,
) {
    // The presentation clock: a paused frame shows the sparks where the body is.
    let turn = (clock.now * 8.0).rem_euclid(std::f64::consts::TAU) as f32;
    for (phase, mut pose) in &mut sparks {
        let angle = phase.0 + turn;
        pose.translation = Vec3::new(angle.cos() * 0.62, angle.sin() * 0.62, -0.1);
    }
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
                Vec3::splat(0.46),
                Quat::IDENTITY,
                "Arcane-Core",
            );
            for angle in [0.0_f32, 2.094, 4.189] {
                spawn_part(
                    parent,
                    &assets.crystal,
                    &assets.white,
                    Vec3::new(angle.cos() * 0.45, angle.sin() * 0.45, -0.32),
                    Vec3::splat(0.11),
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
            // A compact silver thrown blade with a luminous guard, not chunky yellow fruit.
            spawn_part(
                parent,
                block,
                &assets.white,
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(0.19, 0.09, 1.30),
                Quat::IDENTITY,
                "Warrior-Blade",
            );
            spawn_part(
                parent,
                &assets.cone,
                &assets.white,
                Vec3::new(0.0, 0.0, 0.72),
                Vec3::new(0.19, 0.50, 0.09),
                Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
                "Warrior-Point",
            );
            spawn_part(
                parent,
                block,
                tint,
                Vec3::new(0.0, 0.0, -0.48),
                Vec3::new(0.65, 0.13, 0.15),
                Quat::IDENTITY,
                "Warrior-Guard",
            );
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

fn attach_visuals(
    mut commands: Commands,
    mode: Res<PlayerVisualMode>,
    registry: Res<CombatVisualRegistry>,
    server: Option<Res<AssetServer>>,
    library: Option<Res<VfxMeshes>>,
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
        let body = profile.flight_body(
            library.is_some(),
            known_basic(class, projectile.action_slot),
        );
        let facing = commands
            .spawn((
                Transform::default().with_scale(Vec3::splat(profile.scale)),
                Visibility::default(),
                ChildOf(owner),
                ProjectilePresentationRoot {
                    #[cfg(any(test, feature = "qa"))]
                    owner,
                },
                Name::new("Projectile-Facing"),
            ))
            .id();
        #[cfg(feature = "qa")]
        commands.entity(owner).insert(ProjectileBodyVisual {
            profile: profile.id.clone(),
            body,
            parts: body.parts().len(),
        });
        // Any body but the `shape` is built from the shared meshes.
        if let Some(library) = library.as_ref().filter(|_| body != FlightBody::Shape) {
            let (tint, deep) = assets
                .forms
                .entry(key)
                .or_insert_with(|| {
                    (
                        materials.add(form_material(
                            crate::game_vfx::hdr_tint(profile.color(), FORM_GAIN),
                            AlphaMode::Opaque,
                        )),
                        materials.add(form_material(echo(profile.color()), AlphaMode::Blend)),
                    )
                })
                .clone();
            let container = commands
                .spawn((
                    Transform::default(),
                    Visibility::default(),
                    ChildOf(facing),
                    Name::new("Projectile-Form"),
                ))
                .id();
            let parts = body
                .parts()
                .into_iter()
                .map(|part| {
                    let paint = match part.paint {
                        FormPaint::Tint => &tint,
                        FormPaint::Echo => &deep,
                        FormPaint::Core => &assets.core,
                        FormPaint::Team => match projectile.owner_team {
                            Team::Green => &assets.green,
                            Team::Blue => &assets.blue,
                        },
                    };
                    let entity = commands
                        .spawn((
                            Mesh3d(library.handle(PartMesh::Silhouette(part.mesh))),
                            MeshMaterial3d(paint.clone()),
                            part.pose(0.0),
                            NotShadowCaster,
                            NotShadowReceiver,
                            ChildOf(container),
                            Name::new("Projectile-FormPart"),
                        ))
                        .id();
                    (entity, part)
                })
                .collect();
            let hugs_ground = profile.hugs_ground(body);
            commands.entity(owner).insert((
                ProjectileVisual {
                    profile,
                    previous: transform.translation,
                    travelled: 0.0,
                    age: 0.0,
                    trail: VecDeque::new(),
                    facing,
                    fallback: container,
                    model: None,
                },
                FormBody {
                    parts,
                    // An irrational step spreads the ids over the turn.
                    phase: (projectile.id % 97) as f32 * 0.618,
                    hugs_ground,
                    span: match body {
                        FlightBody::Form(form, mesh) => {
                            crate::combat_visuals::form_heading_span(form, mesh)
                        }
                        _ => None,
                    },
                    centred: matches!(
                        body,
                        FlightBody::Form(form, mesh)
                            if crate::combat_visuals::form_heading_span(form, mesh).is_none()
                    ),
                },
            ));
            continue;
        }
        // A wave carries no weapon model, whatever stands for it.
        let packaged = profile
            .model
            .as_ref()
            .filter(|_| profile.presentation != ProjectilePresentation::Wave);
        let (tint, scene) = assets
            .profiles
            .entry(key)
            .or_insert_with(|| {
                (
                    materials.add(material(profile.color())),
                    packaged.and_then(|model| {
                        server.as_ref().map(|server| {
                            server.load(format!("{}#Scene{}", model.path, model.scene))
                        })
                    }),
                )
            })
            .clone();
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
        let model = packaged.zip(scene).map(|(model, scene)| {
            let [x, y, z] = model.rotation_degrees.map(f32::to_radians);
            let entity = commands
                .spawn((
                    WorldAssetRoot(scene.clone()),
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
            travelled: 0.0,
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
    // Without a trail no sample is kept, so not even the segment from the previous
    // position to the newest one is drawn.
    if !visual.profile.trails() {
        visual.previous = position;
        return;
    }
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
    clock: Res<crate::vfx_clock::VfxClock>,
    server: Option<Res<AssetServer>>,
    scenes: Option<Res<Assets<WorldAsset>>>,
    map: Option<Res<crate::maps::MapLayout>>,
    mut roots: Query<(
        &Transform,
        &NetworkProjectile,
        &mut ProjectileVisual,
        Option<&FormBody>,
    )>,
    mut transforms: Query<&mut Transform, Without<NetworkProjectile>>,
    mut visibility: Query<&mut Visibility>,
) {
    for (transform, projectile, mut visual, form) in &mut roots {
        let at = transform.translation;
        let hugs_ground = form.is_some_and(|form| form.hugs_ground);
        // A body on the ground keeps level whatever height the projectile aims at.
        let level = if hugs_ground {
            Vec3::new(1.0, 0.0, 1.0)
        } else {
            Vec3::ONE
        };
        let rotation =
            facing_rotation(projectile.direction * level, (at - visual.previous) * level);
        if let Ok(mut facing) = transforms.get_mut(visual.facing) {
            facing.rotation = rotation;
            if hugs_ground {
                let ground = map
                    .as_ref()
                    .map_or(0.0, |map| map.terrain_height_3d(at.x, at.z));
                facing.translation.y = ground + GROUND_LIFT - at.y;
            }
        }
        let step = at.distance(visual.previous);
        // A correction across the map is no flight.
        if step.is_finite() && step < 30.0 {
            visual.travelled += step;
        }
        advance_trail(&mut visual, at, clock.delta);
        if let Some(form) = form {
            for (entity, part) in &form.parts {
                if let Ok(mut pose) = transforms.get_mut(*entity) {
                    *pose = part.pose(visual.age + form.phase);
                }
            }
            // The nose of a long body is the replicated position, and the body grows out
            // of the place the projectile was first seen at: nothing of it runs ahead of
            // the projectile, and no more than an arm's length of it lies behind its start.
            if let Some((tail, nose)) = form.span {
                let length = (nose - tail) * visual.profile.scale;
                let shown = ((visual.travelled + FORM_START) / length).clamp(0.05, 1.0);
                if let Ok(mut pose) = transforms.get_mut(visual.fallback) {
                    // It is thinner while it is short, so that it keeps an outline that
                    // points along its path.
                    let across = shown.powf(0.4);
                    pose.translation.z = -nose * shown;
                    pose.scale = Vec3::new(across, across, shown);
                }
            } else if form.centred {
                // Its middle stays on the projectile; no delay is added and it is never
                // larger than its profile makes it.
                let grown = FORM_SEED + (1.0 - FORM_SEED) * visual.travelled / FORM_GROWTH;
                if let Ok(mut pose) = transforms.get_mut(visual.fallback) {
                    pose.scale = Vec3::splat(grown.clamp(FORM_SEED, 1.0));
                }
            }
        }
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
    mut roots: Query<(&Transform, &mut ProjectileVisual, &InheritedVisibility)>,
) {
    for (transform, mut visual, inherited) in &mut roots {
        if !inherited.get() {
            visual.trail.clear();
            continue;
        }
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
            travelled: 0.0,
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
        // Rule T0: a trail that lasts no time keeps no sample, not even the newest step.
        visual.profile.trail.seconds = 0.0;
        advance_trail(&mut visual, Vec3::X * 1002.0, 0.01);
        assert!(visual.trail.is_empty() && visual.previous == Vec3::X * 1002.0);
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

    use crate::skill_presentation::vocab::{ProjectileForm, Silhouette};
    use shared::HeroClass;

    const TARGET: &str = include_str!("skill_presentation/fixtures/target_combat_visuals.json");
    const WARRIOR: u64 = 1;
    const WILDSPARK: u64 = 2;
    const CLERIC: u64 = 3;
    const STEP: f32 = 0.05;

    /// An app that draws projectiles of the final profiles for three visible heroes;
    /// `library` adds the shared meshes a shipping client always has.
    fn drawing(library: bool) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_asset::<WorldAsset>()
            .insert_resource(CombatVisualRegistry::from_json(TARGET).unwrap())
            .insert_resource(PlayerVisualMode::Models3d)
            .insert_resource(crate::vfx_clock::VfxClock {
                now: 0.0,
                delta: STEP,
            })
            .add_systems(Startup, setup_assets)
            .add_systems(Update, (attach_visuals, update_visuals).chain());
        if library {
            app.add_systems(Startup, crate::skill_presentation::bodies::setup_meshes);
        }
        for (id, class) in [
            (WARRIOR, HeroClass::Warrior),
            (WILDSPARK, HeroClass::Wildspark),
            (CLERIC, HeroClass::Cleric),
        ] {
            app.world_mut()
                .spawn((NetworkPlayerId(id), NetworkHeroClass(class)));
        }
        app.update();
        app
    }

    /// A replicated projectile at chest height that aims a little upward.
    fn shoot(
        app: &mut App,
        owner_id: u64,
        style: ProjectileStyle,
        action_slot: Option<u8>,
    ) -> Entity {
        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(3.0, 0.85, 4.0),
                NetworkProjectile {
                    id: 40 + owner_id,
                    owner_id,
                    owner_team: Team::Blue,
                    source_kind: CombatEntityKind::Player,
                    style,
                    action_slot,
                    direction: Vec3::new(0.8, 0.6, 0.0),
                },
            ))
            .id();
        app.update();
        entity
    }

    /// Names of the mesh parts under a projectile root.
    fn part_names(app: &mut App, root: Entity) -> Vec<String> {
        let world = app.world_mut();
        let mut parts = world.query_filtered::<(Entity, &Name), With<Mesh3d>>();
        let found: Vec<(Entity, String)> = parts
            .iter(world)
            .map(|(entity, name)| (entity, name.to_string()))
            .collect();
        let under_root = |mut entity: Entity| {
            while let Some(parent) = world.get::<ChildOf>(entity) {
                entity = parent.parent();
                if entity == root {
                    return true;
                }
            }
            false
        };
        found
            .into_iter()
            .filter(|(entity, _)| under_root(*entity))
            .map(|(_, name)| name)
            .collect()
    }

    fn facing(app: &App, root: Entity) -> Transform {
        let visual = app.world().get::<ProjectileVisual>(root).unwrap();
        *app.world().get::<Transform>(visual.facing).unwrap()
    }

    /// A body that is laid out along its path has its nose on the replicated position and
    /// grows out of the place the projectile was first seen at; a body that turns about
    /// its position stays centred on it.
    #[test]
    fn a_long_body_ends_at_the_projectile_and_grows_from_its_start() {
        use crate::combat_visuals::form_heading_span;
        let mut app = drawing(true);
        let bullet = shoot(&mut app, WILDSPARK, ProjectileStyle::Bullet, None);
        let (profile, container) = {
            let visual = app.world().get::<ProjectileVisual>(bullet).unwrap();
            (visual.profile.clone(), visual.fallback)
        };
        assert_eq!(profile.form, Some(ProjectileForm::Dart));
        let (tail, nose) =
            form_heading_span(ProjectileForm::Dart, profile.silhouette.unwrap()).unwrap();
        assert!(tail < -1.0 && nose > 1.0, "{tail} {nose}");
        let length = (nose - tail) * profile.scale;
        let pose = |app: &App, container: Entity| *app.world().get::<Transform>(container).unwrap();
        // The farthest point of the body ahead of and behind the projectile, in units.
        let ends = |app: &App| {
            let pose = pose(app, container);
            (
                (pose.translation.z + tail * pose.scale.z) * profile.scale,
                (pose.translation.z + nose * pose.scale.z) * profile.scale,
            )
        };
        // First seen: its first stretch, none of it ahead.
        let (behind, ahead) = ends(&app);
        assert!(ahead.abs() < 1e-5, "{ahead}");
        assert!((behind + FORM_START).abs() < 1e-4, "{behind}");
        let start = pose(&app, container).scale;
        assert!(start.z < start.x && start.x < 1.0 && start.x == start.y);
        // In flight it is never longer than the distance flown and that first stretch, and
        // its nose stays on the projectile.
        let mut flown = 0.0;
        while flown < length + 1.0 {
            app.world_mut()
                .get_mut::<Transform>(bullet)
                .unwrap()
                .translation
                .x += 0.5;
            flown += 0.5;
            app.update();
            let (behind, ahead) = ends(&app);
            assert!(ahead.abs() < 1e-5, "{ahead} after {flown}");
            assert!(
                -behind <= flown + FORM_START + 1e-4,
                "{behind} after {flown}"
            );
            assert!((-behind - (flown + FORM_START).min(length)).abs() < 1e-4);
        }
        assert_eq!(pose(&app, container).scale, Vec3::ONE);
        // A plate that spins about its position keeps its middle there, and grows to its
        // size over its first stretch of flight.
        assert_eq!(
            form_heading_span(ProjectileForm::DiscSkim, Silhouette::Kite),
            None
        );
        for form in [
            ProjectileForm::Tumbler,
            ProjectileForm::TwinHelix,
            ProjectileForm::Wavefront,
        ] {
            assert_eq!(form_heading_span(form, Silhouette::Diamond), None);
        }
        let plate = shoot(&mut app, WARRIOR, ProjectileStyle::Crescent, Some(0));
        let visual = app.world().get::<ProjectileVisual>(plate).unwrap();
        assert_eq!(visual.profile.form, Some(ProjectileForm::DiscSkim));
        let container = visual.fallback;
        let seen = pose(&app, container);
        assert_eq!(
            (seen.translation, seen.scale),
            (Vec3::ZERO, Vec3::splat(FORM_SEED))
        );
        let mut last = FORM_SEED;
        for _ in 0..4 {
            app.world_mut()
                .get_mut::<Transform>(plate)
                .unwrap()
                .translation
                .x += 0.3;
            app.update();
            let grown = pose(&app, container);
            assert_eq!(grown.translation, Vec3::ZERO);
            assert!(grown.scale.x >= last && grown.scale.x <= 1.0);
            assert!(grown.scale.x == grown.scale.y && grown.scale.y == grown.scale.z);
            last = grown.scale.x;
        }
        assert_eq!(pose(&app, container).scale, Vec3::ONE);
    }

    #[test]
    fn form_beats_model_when_meshes_exist() {
        // The launcher round of the Repeater names a form and the rocket model.
        let mut app = drawing(true);
        let rocket = shoot(&mut app, WILDSPARK, ProjectileStyle::Rocket, Some(255));
        assert_eq!(part_names(&mut app, rocket), ["Projectile-FormPart"; 4]);
        let models = |app: &mut App| {
            let world = app.world_mut();
            world.query::<&WorldAssetRoot>().iter(world).count()
        };
        assert_eq!(models(&mut app), 0);
        // The model is not even requested.
        let assets = app.world().resource::<ProjectileAssets>();
        assert!(assets.profiles.is_empty() && assets.forms.len() == 1);
        #[cfg(feature = "qa")]
        assert_eq!(
            app.world()
                .get::<ProjectileBodyVisual>(rocket)
                .unwrap()
                .body,
            FlightBody::Form(ProjectileForm::Tumbler, Silhouette::Block)
        );

        // Without the shared meshes the same profile is its `shape` and brings its model.
        let mut app = drawing(false);
        let rocket = shoot(&mut app, WILDSPARK, ProjectileStyle::Rocket, Some(255));
        assert_eq!(
            part_names(&mut app, rocket),
            [
                "Projectile-Bolt-Core",
                "Projectile-Bolt-Streak",
                "Projectile-Team-Cue"
            ]
        );
        assert_eq!(models(&mut app), 1);
        assert!(app.world().get::<FormBody>(rocket).is_none());
    }

    #[test]
    fn a_melee_contact_throws_no_blade_and_a_wave_slides_on_the_ground() {
        let mut app = drawing(true);
        // The Warrior's basic attack: two low slivers, level on the ground under the
        // replicated position, whatever height the projectile aims at.
        let basic = shoot(&mut app, WARRIOR, ProjectileStyle::Crescent, None);
        assert_eq!(part_names(&mut app, basic), ["Projectile-FormPart"; 2]);
        let pose = facing(&app, basic);
        assert!((pose.translation.y - (GROUND_LIFT - 0.85)).abs() < 1e-6);
        assert!((pose.rotation * Vec3::Z).distance(Vec3::X) < 1e-5);
        // Heroic Strike: the four parts of the wave, on the ground as well.
        let wave = shoot(&mut app, WARRIOR, ProjectileStyle::Crescent, Some(2));
        assert_eq!(part_names(&mut app, wave), ["Projectile-FormPart"; 4]);
        let pose = facing(&app, wave);
        assert!((pose.translation.y - (GROUND_LIFT - 0.85)).abs() < 1e-6);
        assert!((pose.rotation * Vec3::Z).distance(Vec3::X) < 1e-5);
        assert_eq!(pose.scale, Vec3::splat(1.3));
        // Shield Bash is thrown: it flies at the replicated height along the replicated
        // direction.
        let plate = shoot(&mut app, WARRIOR, ProjectileStyle::Crescent, Some(0));
        assert_eq!(part_names(&mut app, plate), ["Projectile-FormPart"; 3]);
        let pose = facing(&app, plate);
        assert_eq!(pose.translation, Vec3::ZERO);
        assert!((pose.rotation * Vec3::Z).distance(Vec3::new(0.8, 0.6, 0.0)) < 1e-5);
        // Neither a contact nor a wave, nor a body whose trail lasts no time, leaves a
        // trail sample while it moves.
        for step in 1..6 {
            for root in [basic, wave, plate] {
                app.world_mut()
                    .get_mut::<Transform>(root)
                    .unwrap()
                    .translation
                    .x += step as f32;
            }
            app.update();
        }
        for root in [basic, wave, plate] {
            assert!(
                app.world()
                    .get::<ProjectileVisual>(root)
                    .unwrap()
                    .trail
                    .is_empty()
            );
        }
        // A crescent whose owner the client cannot name resolves to the same profile as
        // the Warrior's basic attack. It may be an ability on a long flight: it keeps the
        // thrown body.
        let unknown = shoot(&mut app, 99, ProjectileStyle::Crescent, None);
        let names = part_names(&mut app, unknown);
        assert!(
            names.contains(&"Projectile-Warrior-Blade".to_string()),
            "{names:?}"
        );
        assert_eq!(facing(&app, unknown).translation, Vec3::ZERO);
        assert!(app.world().get::<FormBody>(unknown).is_none());
        // Smite of a known Cleric is a wave; no weapon model and no blade either.
        let seal = shoot(&mut app, CLERIC, ProjectileStyle::Holy, Some(0));
        assert_eq!(part_names(&mut app, seal), ["Projectile-FormPart"; 3]);
        assert!(app.world().get::<FormBody>(seal).unwrap().hugs_ground);
    }

    #[test]
    fn form_parts_share_meshes_and_materials_cast_no_shadow_and_follow_the_clock() {
        let mut app = drawing(true);
        let library: Vec<_> = {
            let meshes = app.world().resource::<VfxMeshes>();
            Silhouette::ALL
                .iter()
                .map(|mesh| meshes.handle(PartMesh::Silhouette(*mesh)).id())
                .collect()
        };
        let mut counts = None;
        for round in 0..3 {
            let roots = [
                shoot(&mut app, WARRIOR, ProjectileStyle::Crescent, None),
                shoot(&mut app, WARRIOR, ProjectileStyle::Crescent, Some(0)),
                shoot(&mut app, WARRIOR, ProjectileStyle::Crescent, Some(2)),
                shoot(&mut app, WARRIOR, ProjectileStyle::Crescent, Some(3)),
                shoot(&mut app, CLERIC, ProjectileStyle::Holy, Some(255)),
            ];
            let current = (
                app.world().resource::<Assets<Mesh>>().len(),
                app.world().resource::<Assets<StandardMaterial>>().len(),
            );
            // No projectile adds a mesh, and after the first round none adds a material.
            assert_eq!(*counts.get_or_insert(current), current, "round {round}");
            let world = app.world_mut();
            let mut parts =
                world.query::<(&Mesh3d, Has<NotShadowCaster>, Has<NotShadowReceiver>)>();
            let drawn: Vec<_> = parts.iter(world).collect();
            assert_eq!(drawn.len(), 2 + 3 + 4 + 4 + 6);
            for (mesh, caster, receiver) in drawn {
                assert!(library.contains(&mesh.0.id()) && caster && receiver);
            }
            for root in roots {
                app.world_mut().entity_mut(root).despawn();
            }
            app.update();
            let world = app.world_mut();
            assert_eq!(world.query::<&Mesh3d>().iter(world).count(), 0);
        }
        // The three projectile meshes of the `shape` bodies, the shared silhouettes and the
        // four meshes the library keeps for interior layers and boundaries.
        let shared = app.world().resource::<Assets<Mesh>>().len();
        assert_eq!(counts.unwrap().0, shared);
        assert_eq!(shared, 3 + Silhouette::ALL.len() + 4);

        // Every part is where the form table puts it at the age of the flight; a paused
        // clock holds the whole body still.
        let plate = shoot(&mut app, WARRIOR, ProjectileStyle::Crescent, Some(0));
        let poses = |app: &App| -> Vec<Transform> {
            let form = app.world().get::<FormBody>(plate).unwrap();
            let age = app.world().get::<ProjectileVisual>(plate).unwrap().age;
            form.parts
                .iter()
                .map(|(entity, part)| {
                    let pose = *app.world().get::<Transform>(*entity).unwrap();
                    assert_eq!(pose, part.pose(age + form.phase));
                    pose
                })
                .collect()
        };
        let first = poses(&app);
        app.update();
        let second = poses(&app);
        assert!(first[0].rotation.angle_between(second[0].rotation) > 0.1);
        app.world_mut()
            .resource_mut::<crate::vfx_clock::VfxClock>()
            .delta = 0.0;
        app.update();
        assert_eq!(poses(&app), second);
        // The shape bodies cast no shadow either.
        let mut app = drawing(false);
        shoot(&mut app, WARRIOR, ProjectileStyle::Crescent, None);
        let world = app.world_mut();
        let mut parts =
            world.query_filtered::<(Has<NotShadowCaster>, Has<NotShadowReceiver>), With<Mesh3d>>();
        assert_eq!(parts.iter(world).count(), 4);
        assert!(parts.iter(world).all(|flags| flags == (true, true)));
    }
}
