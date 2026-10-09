//! Persistent, bounded drawables for authoritative skill objects. Despawn is silent.
//! An effect is drawn from the `body` its row gives its kind (`bodies.rs`); an effect
//! without one is not drawn.
use super::SkillPresentation;
use super::accents::Palette;
use super::bodies::{self, Load, Paint, PartSlot, Role, Seen, VfxMeshes};
use super::cast::CastKey;
use super::category::SkillKey;
use super::geometry::GeoShape;
use super::schema::Body;
use super::stage::{self, EffectKey, EffectMemory};
use super::vocab::{Model, PaletteSlot};
use crate::game_vfx::hdr_tint;
use crate::net::NetworkPlayerId;
use crate::{net::GameStateSnapshot, sprite::PlayerVisualMode};
use bevy::ecs::system::SystemParam;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::prelude::*;
use bevy::world_serialization::{WorldInstance, WorldInstanceSpawner};
use shared::loadout::{EffectVisualKind, SkillEffectState, SkillId};
use std::collections::{HashMap, HashSet};

pub(super) struct SkillEffectsPlugin;
impl Plugin for SkillEffectsPlugin {
    fn build(&self, app: &mut App) {
        crate::vfx_clock::ensure(app);
        app.init_resource::<Instances>()
            .add_systems(Startup, (bodies::setup_meshes, setup))
            .add_systems(
                PostUpdate,
                (sync_bodies, finish)
                    .chain()
                    .before(bevy::transform::TransformSystems::Propagate),
            );
    }
}

/// The packaged scenes of the props a body may carry.
#[derive(Resource)]
struct Props(HashMap<Model, Handle<WorldAsset>>);

/// The interior of an area effect darkens and tints the ground under it in the hue of the
/// skill colour, by `strength`. It sets the bright boundary and core off against pale
/// ground and keeps the ground's own detail, as the shadow of the fill disc did while
/// effect parts still cast shadows.
fn fill_material(color: Color, strength: f32) -> StandardMaterial {
    let hsl = Hsla::from(color);
    // A colour without a hue only darkens.
    let saturation = hsl.saturation.max(FILL_SATURATION) * f32::from(hsl.saturation > 0.02);
    StandardMaterial {
        alpha_mode: AlphaMode::Multiply,
        // Drawn before the lines and parts that lie in it, so that it tints the ground
        // and not them.
        depth_bias: FILL_DEPTH_BIAS,
        ..material(Hsla::new(hsl.hue, saturation, FILL_LIGHTNESS, strength).into())
    }
}

/// An unlit, two-sided colour that the scene fog does not dim.
pub(super) fn material(color: Color) -> StandardMaterial {
    StandardMaterial {
        base_color: color,
        unlit: true,
        fog_enabled: false,
        double_sided: true,
        cull_mode: None,
        alpha_mode: AlphaMode::Blend,
        ..default()
    }
}
fn setup(mut commands: Commands, server: Res<AssetServer>) {
    commands.init_resource::<BodyPaints>();
    commands.insert_resource(Props(
        Model::ALL
            .iter()
            .map(|model| {
                (
                    *model,
                    server.load(format!("cosmetics/standard/{}.glb#Scene0", model.id())),
                )
            })
            .collect(),
    ));
}

#[derive(Resource, Default)]
struct Instances {
    round: Option<(u64, u64)>,
    /// One body for each effect that is drawn.
    bodies: HashMap<EffectKey, BodyInstance>,
}

pub(crate) fn valid_effect(e: &SkillEffectState) -> bool {
    e.position.into_iter().chain(e.end).all(f32::is_finite)
        && e.radius.is_finite()
        && (0.0..=256.0).contains(&e.radius)
        && e.remaining_secs.is_finite()
        && e.remaining_secs >= 0.0
}

type Poses<'w, 's> =
    Query<'w, 's, (&'static mut Transform, &'static mut Visibility), Without<NetworkPlayerId>>;

/// Poses one entity of an effect in this frame.
fn place(
    commands: &mut Commands,
    poses: &mut Poses,
    entity: Entity,
    pose: Transform,
    visibility: Visibility,
) {
    if let Ok((mut current, mut visible)) = poses.get_mut(entity) {
        *current = pose;
        *visible = visibility;
    } else {
        // Deferred spawns receive their final pose before transform propagation.
        // A one-snapshot beam must render on this frame, not one frame later.
        commands.entity(entity).insert((pose, visibility));
    }
}

/// Whether the packaged scene under `entity` is loaded and spawned.
fn scene_ready(
    server: &AssetServer,
    scene: &Handle<WorldAsset>,
    entity: Entity,
    scene_instances: &Query<&WorldInstance>,
    scene_spawner: &Option<Res<WorldInstanceSpawner>>,
) -> bool {
    matches!(
        server.get_recursive_dependency_load_state(scene.id()),
        Some(bevy::asset::RecursiveDependencyLoadState::Loaded)
    ) && scene_instances.get(entity).is_ok_and(|instance| {
        scene_spawner
            .as_ref()
            .is_some_and(|spawner| spawner.instance_is_ready(**instance))
    })
}

/// HDR gain of the spark colour on a body part. It is one value for every skill, so that
/// bodies share one material for each spark colour.
const ACCENT_GAIN: f32 = 4.0;
/// Lightness and least saturation of the tint of the interior layer of an area effect, and
/// the share of its strength (`bodies::fill_strength`) that a telegraph is tinted with.
const FILL_LIGHTNESS: f32 = 0.24;
const FILL_SATURATION: f32 = 0.6;
const FILL_DIM_SHARE: f32 = 0.6 / bodies::FILL_STRENGTH;
/// Sorting offset of the interior layer among the translucent parts of an effect.
const FILL_DEPTH_BIAS: f32 = -4.0;
/// Share of its own colour a matter part glows with, so that its shaded side is not black.
const MATTER_GLOW: f32 = 0.3;
/// Depth bias of light materials: enough to win against a matter part in the same plane.
const LIGHT_DEPTH_BIAS: f32 = 32.0;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum PaintKind {
    /// Light: unlit, with HDR gain.
    Lit,
    /// Matter: shaded by the scene lights, so that a solid shows its form.
    Matter,
    Fill,
    FillDim,
}

/// The materials of staged bodies, shared by colour value and kind and never changed after
/// they are made.
#[derive(Resource, Default)]
struct BodyPaints {
    made: HashMap<(PaintKind, [u32; 3], u32), Handle<StandardMaterial>>,
}

impl BodyPaints {
    fn handle(
        &mut self,
        materials: &mut Assets<StandardMaterial>,
        kind: PaintKind,
        color: Color,
        gain: f32,
    ) -> Handle<StandardMaterial> {
        let srgb = color.to_srgba();
        let key = (
            kind,
            [srgb.red, srgb.green, srgb.blue].map(f32::to_bits),
            gain.to_bits(),
        );
        self.made
            .entry(key)
            .or_insert_with(|| {
                materials.add(match kind {
                    // A solid part hides what is behind it, so its outline stays sharp. A
                    // flat light part in the plane of a matter part is drawn over it from
                    // both sides: the plate shows on its rim wherever the camera stands.
                    PaintKind::Lit => StandardMaterial {
                        alpha_mode: AlphaMode::Opaque,
                        depth_bias: LIGHT_DEPTH_BIAS,
                        ..material(hdr_tint(color, gain))
                    },
                    PaintKind::Matter => StandardMaterial {
                        alpha_mode: AlphaMode::Opaque,
                        unlit: false,
                        emissive: color.to_linear() * MATTER_GLOW,
                        perceptual_roughness: 0.7,
                        ..material(color)
                    },
                    // The interior layers carry their strength where a light has its gain.
                    PaintKind::Fill => fill_material(color, gain),
                    PaintKind::FillDim => fill_material(color, gain * FILL_DIM_SHARE),
                })
            })
            .clone()
    }

    /// The handles one body draws with: the colours of its row and the two sides. `fill`
    /// is how strongly its interior layer tints the ground.
    fn resolve(
        &mut self,
        materials: &mut Assets<StandardMaterial>,
        palette: &Palette,
        fill: f32,
    ) -> Paints {
        let primary = palette.slot(PaletteSlot::Primary);
        let mut made = |kind, color: Color, gain: f32| self.handle(materials, kind, color, gain);
        Paints {
            friend: made(PaintKind::Lit, Color::srgb(0.12, 0.95, 0.60), 2.0),
            foe: made(PaintKind::Lit, Color::srgb(1.0, 0.12, 0.08), 2.0),
            primary: made(PaintKind::Lit, primary.color, primary.gain),
            secondary: made(
                PaintKind::Matter,
                palette.slot(PaletteSlot::Secondary).color,
                1.0,
            ),
            accent: made(
                PaintKind::Lit,
                palette.slot(PaletteSlot::Accent).color,
                ACCENT_GAIN,
            ),
            white: made(PaintKind::Lit, Color::srgb(1.0, 0.97, 0.82), 5.0),
            fill: made(PaintKind::Fill, primary.color, fill),
            fill_dim: made(PaintKind::FillDim, primary.color, fill),
        }
    }
}

/// Material handles of one body, resolved when it is spawned or its colours change.
struct Paints {
    friend: Handle<StandardMaterial>,
    foe: Handle<StandardMaterial>,
    primary: Handle<StandardMaterial>,
    secondary: Handle<StandardMaterial>,
    accent: Handle<StandardMaterial>,
    white: Handle<StandardMaterial>,
    fill: Handle<StandardMaterial>,
    fill_dim: Handle<StandardMaterial>,
}

impl Paints {
    fn of(&self, paint: Paint, friendly: bool) -> &Handle<StandardMaterial> {
        match paint {
            Paint::Team if friendly => &self.friend,
            Paint::Team => &self.foe,
            Paint::Slot(PaletteSlot::Primary) => &self.primary,
            Paint::Slot(PaletteSlot::Secondary) => &self.secondary,
            Paint::Slot(PaletteSlot::Accent) => &self.accent,
            Paint::Slot(PaletteSlot::White) => &self.white,
            Paint::Fill => &self.fill,
            Paint::FillDim => &self.fill_dim,
        }
    }
}

struct BodyPart {
    entity: Entity,
    slot: PartSlot,
    /// The material the entity has now; `None` for the packaged model.
    paint: Option<Paint>,
    /// The handle behind `paint` changed.
    repaint: bool,
}

/// One effect drawn through the `body` of its row.
struct BodyInstance {
    root: Entity,
    owner: u64,
    skill: SkillId,
    kind: EffectVisualKind,
    friendly: bool,
    /// The block the parts were built from, and the family of boundary they were built
    /// for: the fog can cut a cone down to a line, and a wall can lose its heading.
    body: Body,
    shape: std::mem::Discriminant<GeoShape>,
    parts: Vec<BodyPart>,
    paints: Paints,
    model: Option<Handle<WorldAsset>>,
    at: Vec3,
    light: Option<Entity>,
    unshadowed: bool,
    jaw_fold: f32,
}

/// The name of a body: the skill and the replicated id of the effect it draws.
fn body_name(e: &SkillEffectState) -> Name {
    Name::new(format!("SkillVfx-{}-{}", e.skill.id(), e.id))
}

fn spawn_body(
    commands: &mut Commands,
    e: &SkillEffectState,
    body: &Body,
    seen: &Seen,
    meshes: &VfxMeshes,
    props: &Props,
    paints: Paints,
    friendly: bool,
) -> BodyInstance {
    let root = commands
        .spawn((Transform::default(), Visibility::Hidden, body_name(e)))
        .id();
    let model = body.model.and_then(|model| props.0.get(&model).cloned());
    let parts = bodies::part_list(body, &seen.shape)
        .into_iter()
        .filter_map(|slot| {
            let paint = bodies::part_paint(&slot, seen);
            let entity = match (slot.mesh, paint) {
                (Some(mesh), Some(paint)) => commands
                    .spawn((
                        Mesh3d(meshes.handle(mesh)),
                        MeshMaterial3d(paints.of(paint, friendly).clone()),
                        Transform::default(),
                        Visibility::Hidden,
                        NotShadowCaster,
                        NotShadowReceiver,
                        ChildOf(root),
                    ))
                    .id(),
                _ => commands
                    .spawn((
                        WorldAssetRoot(model.clone()?),
                        Transform::default(),
                        Visibility::Hidden,
                        ChildOf(root),
                    ))
                    .id(),
            };
            Some(BodyPart {
                entity,
                slot,
                paint,
                repaint: false,
            })
        })
        .collect();
    BodyInstance {
        root,
        owner: e.owner_id,
        skill: e.skill,
        kind: e.kind,
        friendly,
        body: body.clone(),
        shape: std::mem::discriminant(&seen.shape),
        parts,
        paints,
        model,
        at: Vec3::ZERO,
        light: None,
        unshadowed: false,
        jaw_fold: 0.0,
    }
}

/// What the body renderer reads of the frame.
#[derive(SystemParam)]
struct BodyFrame<'w, 's> {
    game: Option<Res<'w, GameStateSnapshot>>,
    mode: Res<'w, PlayerVisualMode>,
    profiles: Res<'w, SkillPresentation>,
    clock: Res<'w, crate::vfx_clock::VfxClock>,
    map: Option<Res<'w, crate::maps::MapLayout>>,
    memory: Option<Res<'w, EffectMemory>>,
    local: Query<'w, 's, &'static crate::team::Team, With<crate::player::Player>>,
    heroes: Query<
        'w,
        's,
        (
            &'static NetworkPlayerId,
            &'static Transform,
            &'static InheritedVisibility,
        ),
    >,
    camera: Query<'w, 's, &'static GlobalTransform, With<crate::camera::MainCamera>>,
}

/// Height of a hero's chest above its feet, where a tether ends.
const OWNER_CHEST: f32 = 1.0;

/// Draws every effect whose row gives it a body. Poses use the received (possibly
/// fog-clipped) geometry and what the stage tracker observed of the instance; nothing is
/// predicted and a hidden origin is never reconstructed.
fn sync_bodies(
    mut commands: Commands,
    frame: BodyFrame,
    mut instances: ResMut<Instances>,
    meshes: Res<VfxMeshes>,
    props: Res<Props>,
    mut paints: ResMut<BodyPaints>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    server: Res<AssetServer>,
    mut poses: Poses,
    scene_instances: Query<&WorldInstance>,
    scene_spawner: Option<Res<WorldInstanceSpawner>>,
) {
    // A body belongs to one round and to the 3D view.
    let round = frame
        .game
        .as_ref()
        .map(|game| (game.meta.server_epoch, game.meta.match_id));
    if round != instances.round || *frame.mode != PlayerVisualMode::Models3d {
        for (_, instance) in instances.bodies.drain() {
            commands.entity(instance.root).despawn();
        }
        instances.round = round;
    }
    let Some(game) = frame
        .game
        .as_ref()
        .filter(|_| *frame.mode == PlayerVisualMode::Models3d)
    else {
        return;
    };
    let profiles = &*frame.profiles;
    let now = frame.clock.now as f32;
    let eye = frame
        .camera
        .single()
        .map_or(Vec3::ZERO, |pose| pose.translation());
    let recolored = frame.profiles.is_changed();
    let instances = &mut *instances;
    let mut live = HashSet::new();
    let mut drawn = Vec::new();
    for (key, e) in stage::keyed(&game.skill_effects) {
        let friendly = frame.local.single().is_ok_and(|team| *team == e.owner_team);
        let stands = bodies::root_at(e);
        let ground = frame
            .map
            .as_ref()
            .map_or(0.0, |map| map.terrain_height_3d(stands.x, stands.y));
        let owner = frame
            .heroes
            .iter()
            .find(|(id, _, visible)| id.0 == e.owner_id && visible.get())
            .map(|(_, pose, _)| pose.translation + Vec3::Y * OWNER_CHEST);
        let memory = frame.memory.as_ref().and_then(|memory| memory.get(key));
        let seen = Seen::of(e, memory, owner, ground, now);
        // An instance whose skill, kind, owner and family of boundary stand under an
        // unchanged registry is what it was: its row is not looked up again.
        let shape = std::mem::discriminant(&seen.shape);
        let known = !recolored
            && instances.bodies.get(&key).is_some_and(|instance| {
                (
                    instance.owner,
                    instance.kind,
                    instance.skill,
                    instance.shape,
                ) == (e.owner_id, e.kind, e.skill, shape)
            });
        if !known {
            let Some((body, look)) = profiles
                .body_for(e)
                .zip(profiles.look(CastKey::Skill(SkillKey::Modular(e.skill))))
            else {
                continue;
            };
            // An auxiliary object keeps its entities when only the skill that orders it
            // changes; any other change of what the instance is builds it anew.
            let rebuilt = instances.bodies.get(&key).is_some_and(|instance| {
                instance.owner != e.owner_id
                    || instance.kind != e.kind
                    || instance.body != *body
                    || instance.shape != shape
                    || (instance.skill != e.skill && matches!(key, EffectKey::Runtime(_)))
            });
            if rebuilt && let Some(previous) = instances.bodies.remove(&key) {
                commands.entity(previous.root).despawn();
            }
            let resolved =
                paints.resolve(&mut materials, &look.palette, bodies::fill_strength(body));
            match instances.bodies.get_mut(&key) {
                Some(instance) => {
                    instance.paints = resolved;
                    if instance.skill != e.skill {
                        commands.entity(instance.root).insert(body_name(e));
                        instance.skill = e.skill;
                    }
                    // The model keeps its own materials.
                    for part in instance
                        .parts
                        .iter_mut()
                        .filter(|part| part.paint.is_some())
                    {
                        part.repaint = true;
                    }
                }
                None => {
                    let spawned = spawn_body(
                        &mut commands,
                        e,
                        body,
                        &seen,
                        &meshes,
                        &props,
                        resolved,
                        friendly,
                    );
                    instances.bodies.insert(key, spawned);
                }
            }
        }
        let Some(instance) = instances.bodies.get_mut(&key) else {
            continue;
        };
        live.insert(key);
        if instance.friendly != friendly {
            instance.friendly = friendly;
            for part in &mut instance.parts {
                part.repaint |= part.paint == Some(Paint::Team);
            }
        }
        let root = bodies::root_pose(&seen);
        instance.at = root.translation;
        if let shared::loadout::SkillEffect::TrapLine {
            arm_secs,
            duration_secs,
            ..
        } = shared::loadout::skill(e.skill).effect
        {
            instance.jaw_fold = if e.armed {
                0.0
            } else {
                ((e.remaining_secs - (duration_secs - arm_secs)) / arm_secs).clamp(0.0, 1.0)
            };
        }
        let model_ready = instance.model.as_ref().is_some_and(|scene| {
            instance.parts.iter().any(|part| {
                part.slot.role == Role::Model
                    && scene_ready(
                        &server,
                        scene,
                        part.entity,
                        &scene_instances,
                        &scene_spawner,
                    )
            })
        });
        let mut parts = Vec::with_capacity(instance.parts.len());
        let mut load = Load {
            key,
            distance: root.translation.distance(eye),
            kept: 0,
            authored: 0,
        };
        for part in &mut instance.parts {
            let (pose, mut visibility) = bodies::part_pose(&part.slot, &seen);
            if part.slot.role == Role::Model && !model_ready {
                visibility = Visibility::Hidden;
            }
            let paint = bodies::part_paint(&part.slot, &seen);
            if (part.repaint || paint != part.paint)
                && let Some(paint) = paint
            {
                commands.entity(part.entity).insert(MeshMaterial3d(
                    instance.paints.of(paint, instance.friendly).clone(),
                ));
                part.paint = Some(paint);
                part.repaint = false;
            }
            if visibility != Visibility::Hidden {
                if part.slot.role.authored() {
                    load.authored += 1;
                } else {
                    load.kept += 1;
                }
            }
            parts.push((part.entity, part.slot.role, pose, visibility));
        }
        #[cfg(feature = "qa")]
        let evidence = (
            super::SkillEffectVisual {
                id: e.id,
                model_ready,
            },
            super::SkillBodyVisual {
                archetype: instance.body.archetype,
                boundary: seen.shape,
                engine: count_roles(&parts, |role| matches!(role, Role::Boundary(_))),
                authored: instance
                    .parts
                    .iter()
                    .filter(|part| part.slot.role.authored())
                    .count(),
                trail: count_roles(&parts, |role| matches!(role, Role::Trail(_))),
                budget_hidden: 0,
            },
        );
        drawn.push((
            load,
            instance.root,
            root,
            parts,
            #[cfg(feature = "qa")]
            evidence,
        ));
    }
    instances.bodies.retain(|key, instance| {
        if live.contains(key) {
            true
        } else {
            commands.entity(instance.root).despawn();
            false
        }
    });

    let loads: Vec<_> = drawn.iter().map(|body| body.0).collect();
    let over = bodies::over_budget(&loads, bodies::PART_BUDGET);
    for body in drawn {
        let (load, root, pose, parts) = (body.0, body.1, body.2, body.3);
        let hide = over.contains(&load.key);
        place(&mut commands, &mut poses, root, pose, Visibility::Inherited);
        #[cfg(feature = "qa")]
        {
            let (visual, mut evidence) = body.4;
            if hide {
                evidence.budget_hidden = load.authored;
            }
            commands.entity(root).insert((visual, evidence));
        }
        for (entity, role, pose, visibility) in parts {
            let visibility = if hide && role.authored() {
                Visibility::Hidden
            } else {
                visibility
            };
            place(&mut commands, &mut poses, entity, pose, visibility);
        }
    }
}

/// Visible parts of one body with a role that `wanted` picks.
#[cfg(feature = "qa")]
fn count_roles(
    parts: &[(Entity, Role, Transform, Visibility)],
    wanted: impl Fn(Role) -> bool,
) -> usize {
    parts
        .iter()
        .filter(|(_, role, _, visibility)| wanted(*role) && *visibility != Visibility::Hidden)
        .count()
}

/// Keeps the flight light of the rockets nearest to the camera and takes the meshes of
/// packaged props out of the shadow pass once their scene is spawned.
fn finish(
    mut commands: Commands,
    mut instances: ResMut<Instances>,
    camera: Query<&GlobalTransform, With<crate::camera::MainCamera>>,
    children: Query<&Children>,
    shadowed: Query<(), (With<Mesh3d>, Without<NotShadowCaster>)>,
    mut joints: Query<(&Name, &mut Transform)>,
) {
    let eye = camera
        .single()
        .map_or(Vec3::ZERO, |pose| pose.translation());
    let instances = &mut *instances;
    // Per rocket: distance, root and its light.
    let mut rockets: Vec<(f32, Entity, &mut Option<Entity>)> = Vec::new();
    let mut props: Vec<(Entity, &mut bool)> = Vec::new();
    for instance in instances.bodies.values_mut() {
        if instance.body.model == Some(Model::Rocket) {
            rockets.push((
                instance.at.distance(eye),
                instance.root,
                &mut instance.light,
            ));
        }
        let scene = instance
            .parts
            .iter()
            .find(|part| part.slot.role == Role::Model);
        if let Some(scene) = scene {
            if instance.body.model == Some(Model::Trap) {
                for child in children.iter_descendants(scene.entity) {
                    if let Ok((name, mut pose)) = joints.get_mut(child) {
                        let side = match name.as_str() {
                            "WildsparkJawLeft" => -1.0,
                            "WildsparkJawRight" => 1.0,
                            _ => continue,
                        };
                        pose.rotation = Quat::from_rotation_z(side * 1.15 * instance.jaw_fold);
                    }
                }
            }
            props.push((scene.entity, &mut instance.unshadowed));
        }
    }
    rockets.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (rank, (_, root, light)) in rockets.into_iter().enumerate() {
        if rank >= bodies::MAX_EFFECT_LIGHTS {
            if let Some(light) = light.take() {
                commands.entity(light).despawn();
            }
        } else if light.is_none() {
            *light = Some(
                commands
                    .spawn((
                        PointLight {
                            color: Color::srgb(1.0, 0.48, 0.12),
                            intensity: 65_000.0,
                            range: 8.0,
                            shadow_maps_enabled: false,
                            ..default()
                        },
                        Transform::from_xyz(0.0, bodies::LIGHT_HEIGHT, -0.6),
                        ChildOf(root),
                        Name::new("RocketFlightLight"),
                    ))
                    .id(),
            );
        }
    }
    for (scene, unshadowed) in props {
        if *unshadowed {
            continue;
        }
        for mesh in children.iter_descendants(scene) {
            if shadowed.contains(mesh) {
                commands
                    .entity(mesh)
                    .insert((NotShadowCaster, NotShadowReceiver));
                *unshadowed = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app(registry: SkillPresentation) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_asset::<WorldAsset>()
            .insert_resource(PlayerVisualMode::Models3d)
            .init_resource::<GameStateSnapshot>()
            .insert_resource(registry)
            .add_plugins(SkillEffectsPlugin);
        app
    }
    /// An effect of `skill` as the server replicates it, heading along +X: `end` is one
    /// unit ahead for a kind with a heading, the far end of a strip or a cone, the
    /// half-length of a wall ahead, and the place itself for every other.
    fn replicated(id: u64, skill: SkillId, kind: EffectVisualKind) -> SkillEffectState {
        use super::super::geometry::{self, GeoClass};
        let radius = geometry::replicated_radius(skill, kind);
        let ahead = match geometry::boundary_class(skill, kind) {
            _ if super::super::category::heading_only(kind) => 1.0,
            GeoClass::Capsule | GeoClass::Lane | GeoClass::Sector => {
                shared::loadout::skill(skill).ability.cast_range.min(45.0)
            }
            GeoClass::Bar => radius,
            GeoClass::Ring | GeoClass::Pentagon | GeoClass::None => 0.0,
        };
        SkillEffectState {
            id,
            owner_id: 7,
            owner_team: shared::map::Team::Green,
            skill,
            kind,
            position: [4.0, 5.0],
            end: [4.0 + ahead, 5.0],
            radius,
            remaining_secs: 1.0,
            armed: true,
            consumed_segments: 0,
        }
    }
    fn show(app: &mut App, effects: Vec<SkillEffectState>) {
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .skill_effects = effects;
        app.update();
    }
    fn bodies(app: &App) -> &HashMap<EffectKey, BodyInstance> {
        &app.world().resource::<Instances>().bodies
    }
    fn material(app: &App, entity: Entity) -> Handle<StandardMaterial> {
        app.world()
            .get::<MeshMaterial3d<StandardMaterial>>(entity)
            .unwrap()
            .0
            .clone()
    }
    fn part(instance: &BodyInstance, role: Role) -> Entity {
        instance
            .parts
            .iter()
            .find(|part| part.slot.role == role)
            .unwrap()
            .entity
    }
    /// Body parts that are drawn, by whether a budget may hide them.
    fn visible_parts(app: &App) -> (usize, usize) {
        let mut counts = (0, 0);
        for instance in bodies(app).values() {
            for part in &instance.parts {
                if *app.world().get::<Visibility>(part.entity).unwrap() != Visibility::Hidden {
                    if part.slot.role.authored() {
                        counts.1 += 1;
                    } else {
                        counts.0 += 1;
                    }
                }
            }
        }
        counts
    }
    const ORB: EffectKey = EffectKey::Aux {
        owner: 7,
        kind: EffectVisualKind::Orb,
        ordinal: 0,
    };

    #[test]
    fn an_effect_is_drawn_by_the_body_its_row_gives_its_kind() {
        use super::super::tests::schema_rules;
        use super::super::vocab::Archetype;
        let mut app = app(schema_rules::parse(&schema_rules::samples()).unwrap());
        // A cage is no object of Winter Shard: its row gives that kind no body.
        let stray = replicated(6, SkillId::WinterShard, EffectVisualKind::Cage);
        assert!(valid_effect(&stray));
        show(
            &mut app,
            vec![
                replicated(1, SkillId::IronHook, EffectVisualKind::Bolt),
                replicated(2, SkillId::WinterShard, EffectVisualKind::Bolt),
                replicated(3, SkillId::OrbitalCommand, EffectVisualKind::Orb),
                replicated(4, SkillId::DawnRay, EffectVisualKind::BeamWarning),
                replicated(5, SkillId::HorizonWave, EffectVisualKind::BeamWarning),
                stray,
            ],
        );
        let instances = app.world().resource::<Instances>();
        let shard = &instances.bodies[&EffectKey::Runtime(2)];
        let orb = &instances.bodies[&ORB];
        let ray = &instances.bodies[&EffectKey::Runtime(4)];
        assert_eq!(
            [1, 2, 4, 5].map(|id| instances.bodies[&EffectKey::Runtime(id)].body.archetype),
            [
                Archetype::Traveller,
                Archetype::Traveller,
                Archetype::Lane,
                Archetype::Lane
            ]
        );
        assert_eq!(orb.body.archetype, Archetype::Orbiter);
        // The effect without a body is not drawn at all.
        assert_eq!(instances.bodies.len(), 5);
        assert_eq!(
            app.world()
                .iter_entities()
                .filter(|entity| entity
                    .get::<Name>()
                    .is_some_and(|name| name.as_str().starts_with("SkillVfx-")))
                .count(),
            5
        );
        // The strip has its two edges and its two round ends.
        let outline = ray
            .parts
            .iter()
            .filter(|part| matches!(part.slot.role, Role::Boundary(_)))
            .count();
        assert_eq!(outline, 4);

        // The root stands on the replicated position, turned along the heading.
        let root = app.world().get::<Transform>(shard.root).unwrap();
        assert_eq!(root.translation, Vec3::new(4.0, 0.0, 5.0));
        assert!((root.rotation * Vec3::Z - Vec3::X).length() < 1e-5);
        assert_eq!(
            *app.world().get::<Visibility>(shard.root).unwrap(),
            Visibility::Inherited
        );
        // The shard has no exhaust bars: a ring, a core and three satellites, and no trail
        // part on its first frame.
        let roles: Vec<Role> = shard.parts.iter().map(|part| part.slot.role).collect();
        assert_eq!(
            roles[..5],
            [
                Role::Boundary(0),
                Role::Core,
                Role::Satellite(0),
                Role::Satellite(1),
                Role::Satellite(2)
            ]
        );
        assert!(roles[5..].iter().all(|role| matches!(role, Role::Trail(_))));
        // Every mesh part is kept out of the shadow pass.
        let meshes: Vec<Entity> = instances
            .bodies
            .values()
            .flat_map(|instance| {
                instance
                    .parts
                    .iter()
                    .filter(|part| part.slot.mesh.is_some())
                    .map(|part| part.entity)
            })
            .collect();
        assert!(meshes.len() > 10);
        for entity in meshes {
            let part = app.world().entity(entity);
            assert!(part.contains::<Mesh3d>());
            assert!(part.contains::<NotShadowCaster>() && part.contains::<NotShadowReceiver>());
        }
        #[cfg(feature = "qa")]
        {
            let evidence = app
                .world()
                .get::<super::super::SkillBodyVisual>(shard.root)
                .unwrap();
            assert_eq!(evidence.archetype, Archetype::Traveller);
            assert_eq!(
                evidence.boundary,
                super::super::geometry::GeoShape::Ring {
                    center: Vec2::new(4.0, 5.0),
                    radius: 0.6
                }
            );
            assert_eq!(
                (evidence.engine, evidence.trail, evidence.budget_hidden),
                (1, 0, 0)
            );
            assert_eq!(
                app.world()
                    .get::<super::super::SkillEffectVisual>(shard.root)
                    .unwrap()
                    .id,
                2
            );
        }
    }

    /// Rule E-12: the orb is one object ordered by two skills.
    #[test]
    fn an_auxiliary_body_keeps_its_entities_when_only_its_skill_changes() {
        let mut app = app(SkillPresentation::target());
        show(
            &mut app,
            vec![
                replicated(u64::MAX, SkillId::OrbitalCommand, EffectVisualKind::Orb),
                replicated(8, SkillId::WinterShard, EffectVisualKind::Bolt),
            ],
        );
        let orb = &bodies(&app)[&ORB];
        let (root, shell, core, ring) = (
            orb.root,
            part(orb, Role::Shell),
            part(orb, Role::Core),
            part(orb, Role::Boundary(0)),
        );
        let before = [shell, core, ring].map(|entity| material(&app, entity));
        let shard = bodies(&app)[&EffectKey::Runtime(8)].root;
        let made = app.world().resource::<Assets<StandardMaterial>>().len();

        // The other order takes the orb over: same entities, the colours of its row.
        show(
            &mut app,
            vec![
                replicated(u64::MAX, SkillId::OrbitalGuard, EffectVisualKind::Orb),
                replicated(8, SkillId::WinterShard, EffectVisualKind::Bolt),
            ],
        );
        let orb = &bodies(&app)[&ORB];
        assert_eq!(orb.root, root);
        assert_eq!(orb.skill, SkillId::OrbitalGuard);
        assert_eq!(
            (part(orb, Role::Shell), part(orb, Role::Core)),
            (shell, core)
        );
        let after = [shell, core, ring].map(|entity| material(&app, entity));
        assert_ne!(after[0], before[0], "the shell takes the new skill colour");
        assert_eq!(after[1], before[1], "the class matter colour is shared");
        assert_eq!(after[2], before[2], "the side did not change");
        assert_eq!(bodies(&app)[&EffectKey::Runtime(8)].root, shard);

        // Back again: the handles of the first row are reused, no material is made.
        let both = app.world().resource::<Assets<StandardMaterial>>().len();
        assert!(both > made);
        show(
            &mut app,
            vec![
                replicated(u64::MAX, SkillId::OrbitalCommand, EffectVisualKind::Orb),
                replicated(8, SkillId::WinterShard, EffectVisualKind::Bolt),
                replicated(9, SkillId::WinterShard, EffectVisualKind::Bolt),
            ],
        );
        assert_eq!(material(&app, shell), before[0]);
        assert_eq!(
            app.world().resource::<Assets<StandardMaterial>>().len(),
            both,
            "a second instance of a row shares its materials"
        );
        let twin = &bodies(&app)[&EffectKey::Runtime(9)];
        assert_eq!(
            material(&app, part(twin, Role::Core)),
            material(
                &app,
                part(&bodies(&app)[&EffectKey::Runtime(8)], Role::Core)
            )
        );

        // The viewer turns out to be on the owner's side: the ring changes, nothing respawns.
        app.world_mut()
            .spawn((crate::team::Team::Green, crate::player::Player));
        app.update();
        assert_eq!(bodies(&app)[&ORB].root, root);
        assert_ne!(material(&app, ring), before[2]);
        assert_eq!(material(&app, shell), before[0]);

        // A replicated id is one object: another skill under it is another body.
        show(
            &mut app,
            vec![replicated(8, SkillId::DawnBind, EffectVisualKind::Bolt)],
        );
        let rebuilt = &bodies(&app)[&EffectKey::Runtime(8)];
        assert_ne!(rebuilt.root, shard);
        assert!(app.world().get_entity(shard).is_err());
        assert!(
            app.world().get_entity(root).is_err(),
            "the orb left the snapshot"
        );
        assert_eq!(bodies(&app).len(), 1);
    }

    #[test]
    fn the_fill_of_a_telegraph_is_dim_and_swaps_when_the_effect_arms() {
        let mut app = app(SkillPresentation::target());
        let mut trap = replicated(3, SkillId::WildTraps, EffectVisualKind::Trap);
        trap.armed = false;
        show(&mut app, vec![trap.clone()]);
        let instance = &bodies(&app)[&EffectKey::Runtime(3)];
        let (fill, pip) = (part(instance, Role::Fill), part(instance, Role::Marker(0)));
        assert_eq!(material(&app, fill), instance.paints.fill_dim);
        assert_eq!(material(&app, pip), instance.paints.secondary);
        let made = app.world().resource::<Assets<StandardMaterial>>().len();
        trap.armed = true;
        show(&mut app, vec![trap]);
        let instance = &bodies(&app)[&EffectKey::Runtime(3)];
        assert_eq!(material(&app, fill), instance.paints.fill);
        assert_eq!(material(&app, pip), instance.paints.primary);
        assert_ne!(instance.paints.fill, instance.paints.fill_dim);
        // The step is a change of handle; no material is made or edited for it.
        assert_eq!(
            app.world().resource::<Assets<StandardMaterial>>().len(),
            made
        );
        let materials = app.world().resource::<Assets<StandardMaterial>>();
        let alpha =
            |handle: &Handle<StandardMaterial>| materials.get(handle).unwrap().base_color.alpha();
        // The strength of a row that names none, and the dimmer layer of a telegraph.
        assert_eq!(alpha(&instance.paints.fill), bodies::FILL_STRENGTH);
        assert!((alpha(&instance.paints.fill_dim) - 0.6).abs() < 1e-6);
        for fill in [&instance.paints.fill, &instance.paints.fill_dim] {
            assert_eq!(materials.get(fill).unwrap().alpha_mode, AlphaMode::Multiply);
        }
        assert_eq!(alpha(&instance.paints.primary), 1.0);

        // A row may name the strength; the telegraph keeps its share of it.
        let mut gentle = self::app(SkillPresentation::target_with(|config| {
            config["skills"]["wild_traps"]["body"]["fill_strength"] = serde_json::json!(0.5);
        }));
        show(
            &mut gentle,
            vec![replicated(3, SkillId::WildTraps, EffectVisualKind::Trap)],
        );
        let instance = &bodies(&gentle)[&EffectKey::Runtime(3)];
        let materials = gentle.world().resource::<Assets<StandardMaterial>>();
        let alpha =
            |handle: &Handle<StandardMaterial>| materials.get(handle).unwrap().base_color.alpha();
        assert_eq!(alpha(&instance.paints.fill), 0.5);
        let dim = 0.5 * 0.6 / bodies::FILL_STRENGTH;
        assert!((alpha(&instance.paints.fill_dim) - dim).abs() < 1e-6);
    }

    /// The parts of a body are those of the boundary it was built for. The fog can cut a
    /// cone down to a line and give it back whole, and each time the body is built anew.
    #[test]
    fn a_body_is_rebuilt_when_the_family_of_its_boundary_changes() {
        let mut app = app(SkillPresentation::target());
        // The root of the body of an effect, its boundary parts and its visible parts.
        let body = |app: &App, id: u64| -> (Entity, usize, usize) {
            let instance = &bodies(app)[&EffectKey::Runtime(id)];
            let boundary = instance
                .parts
                .iter()
                .filter(|part| matches!(part.slot.role, Role::Boundary(_)))
                .count();
            let visible = instance
                .parts
                .iter()
                .filter(|part| {
                    *app.world().get::<Visibility>(part.entity).unwrap() != Visibility::Hidden
                })
                .count();
            (instance.root, boundary, visible)
        };
        let cone = replicated(5, SkillId::FurnaceBreath, EffectVisualKind::BeamWarning);
        show(&mut app, vec![cone.clone()]);
        // Two edges and six chords, the dim fill and two flame tongues; the read-out has
        // not begun to grow.
        let (whole, bars, visible) = body(&app, 5);
        assert_eq!((bars, visible), (8, 11));
        let mut later = cone.clone();
        later.remaining_secs = 0.6;
        show(&mut app, vec![later.clone()]);
        assert_eq!(body(&app, 5), (whole, 8, 12), "the same entities");
        // The fog takes a part of the axis: one plain bar and nothing else.
        let mut cut = later.clone();
        cut.end = [8.0, 5.0];
        show(&mut app, vec![cut]);
        let (line, bars, visible) = body(&app, 5);
        assert_ne!(line, whole);
        assert!(app.world().get_entity(whole).is_err());
        assert_eq!((bars, visible), (1, 1));
        #[cfg(feature = "qa")]
        {
            let evidence = app
                .world()
                .get::<super::super::SkillBodyVisual>(line)
                .unwrap();
            assert_eq!(evidence.engine, 1);
            assert_eq!(
                evidence.boundary,
                GeoShape::Segment {
                    from: Vec2::new(4.0, 5.0),
                    to: Vec2::new(8.0, 5.0)
                }
            );
        }
        // The whole axis is in sight again.
        show(&mut app, vec![later]);
        let (again, bars, visible) = body(&app, 5);
        assert_ne!(again, line);
        assert_eq!((bars, visible), (8, 12));

        // A wall stands on its bar; without a heading there is no bar and nothing stands.
        let wall = replicated(6, SkillId::Northwall, EffectVisualKind::ShieldWall);
        show(&mut app, vec![wall.clone()]);
        let (standing, bars, visible) = body(&app, 6);
        assert_eq!((bars, visible), (1, 6));
        let mut lost = wall;
        lost.end = lost.position;
        show(&mut app, vec![lost]);
        let (fallen, bars, visible) = body(&app, 6);
        assert_ne!(fallen, standing);
        assert_eq!((bars, visible), (0, 0));

        // A cage keeps its entities while its sides break: a broken side is hidden.
        let cage = replicated(7, SkillId::IronBoundary, EffectVisualKind::Cage);
        show(&mut app, vec![cage.clone()]);
        let (fence, bars, visible) = body(&app, 7);
        assert_eq!((bars, visible), (10, 16));
        let mut broken = cage;
        broken.consumed_segments = 0b10010;
        show(&mut app, vec![broken]);
        assert_eq!(body(&app, 7), (fence, 10, 12));
        #[cfg(feature = "qa")]
        assert_eq!(
            app.world()
                .get::<super::super::SkillBodyVisual>(fence)
                .unwrap()
                .engine,
            6
        );
    }

    /// The cache makes what the parser's material rule counts: three materials for a skill
    /// colour, one for a matter and one for a spark colour, and the engine's own.
    #[test]
    fn every_staged_body_of_the_final_rows_is_drawn_from_shared_materials() {
        use std::collections::BTreeSet;
        let registry = SkillPresentation::target();
        let mut effects = Vec::new();
        let mut rows = BTreeSet::new();
        for (id, _) in registry.rows() {
            let Some(skill) = SkillId::from_id(id) else {
                continue;
            };
            let kinds = super::super::category::own_kinds(skill)
                .iter()
                .chain(super::super::category::aux_kinds(skill));
            for kind in kinds {
                let effect = replicated(effects.len() as u64 + 1, skill, *kind);
                if registry.body_for(&effect).is_some() {
                    rows.insert(id.to_string());
                    // Two instances of each: the second makes no material.
                    let mut twin = effect.clone();
                    twin.id += 100;
                    twin.owner_id = 9;
                    effects.extend([effect, twin]);
                }
            }
        }
        assert_eq!(effects.len(), 33 * 2);
        let bits = |color: [f32; 3]| color.map(f32::to_bits);
        let (mut primaries, mut secondaries, mut accents) =
            (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
        for id in &rows {
            let profile = registry.row(id).unwrap();
            let theme = registry
                .theme(shared::HeroClass::from_id(&profile.home).unwrap())
                .unwrap();
            primaries.insert((bits(profile.color), profile.hdr_gain.to_bits()));
            secondaries.insert(bits(profile.secondary.unwrap_or(theme.secondary)));
            accents.insert(bits(profile.accent.unwrap_or(theme.accent)));
        }
        let mut app = app(registry);
        show(&mut app, effects);
        assert_eq!(bodies(&app).len(), 33 * 2);
        let made = app.world().resource::<BodyPaints>().made.len();
        // White and the two team colours are the engine's share here.
        let most = 3 * primaries.len() + secondaries.len() + accents.len() + 3;
        assert!(made <= most, "{made} of {most}");
        assert!(made >= 2 * primaries.len() + 3, "{made}");
        assert!(most <= bodies::MATERIAL_BUDGET);
        // Frames that follow make none.
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(app.world().resource::<BodyPaints>().made.len(), made);
        // Every pose the app wrote is finite and every body is shown.
        for instance in bodies(&app).values() {
            assert_eq!(
                *app.world().get::<Visibility>(instance.root).unwrap(),
                Visibility::Inherited
            );
            for part in &instance.parts {
                assert!(
                    app.world()
                        .get::<Transform>(part.entity)
                        .unwrap()
                        .is_finite()
                );
            }
        }
    }

    #[test]
    fn at_most_two_effect_lights_are_alive_and_the_nearest_rockets_keep_them() {
        // Rockets fly at x = 4, 14 and 24; the camera stands at the origin.
        let rockets = |ids: &[u64]| -> Vec<SkillEffectState> {
            ids.iter()
                .map(|id| {
                    let mut rocket = replicated(*id, SkillId::WildRocket, EffectVisualKind::Rocket);
                    rocket.position[0] = 4.0 + (*id as f32 - 1.0) * 10.0;
                    rocket.end[0] = rocket.position[0] + 1.0;
                    rocket
                })
                .collect()
        };
        let mut app = app(SkillPresentation::target());
        app.world_mut()
            .spawn((crate::camera::MainCamera, GlobalTransform::default()));
        let lit = |app: &mut App| -> Vec<u64> {
            let roots: Vec<Entity> = app
                .world_mut()
                .query_filtered::<&ChildOf, With<PointLight>>()
                .iter(app.world())
                .map(ChildOf::parent)
                .collect();
            let instances = app.world().resource::<Instances>();
            let ids: Vec<u64> = (1..=3)
                .filter(|id| {
                    instances
                        .bodies
                        .get(&EffectKey::Runtime(*id))
                        .is_some_and(|instance| roots.contains(&instance.root))
                })
                .collect();
            assert_eq!(
                ids.len(),
                roots.len(),
                "every light belongs to a live rocket"
            );
            ids
        };
        show(&mut app, rockets(&[1, 2, 3]));
        assert_eq!(app.world().resource::<Instances>().bodies.len(), 3);
        assert_eq!(lit(&mut app), [1, 2]);
        // The nearest rocket is gone: the next one is lit, and still only two.
        show(&mut app, rockets(&[2, 3]));
        assert_eq!(lit(&mut app), [2, 3]);
        // A nearer rocket arrives: the farthest gives its light up.
        show(&mut app, rockets(&[1, 2, 3]));
        assert_eq!(lit(&mut app), [1, 2]);
        show(&mut app, Vec::new());
        assert!(lit(&mut app).is_empty());
    }

    #[test]
    fn the_part_budget_keeps_every_boundary_and_the_nearest_looks() {
        let mut app = app(SkillPresentation::target());
        app.world_mut()
            .spawn((crate::camera::MainCamera, GlobalTransform::default()));
        // Twelve parts each: a ring, a fill and a marker, a core and eight satellites.
        let zones = |count: u64| -> Vec<SkillEffectState> {
            (1..=count)
                .map(|id| {
                    let mut zone = replicated(id, SkillId::DawnField, EffectVisualKind::Field);
                    zone.position = [id as f32 * 7.0, 0.0];
                    zone.end = zone.position;
                    zone
                })
                .collect()
        };
        show(&mut app, zones(33));
        assert_eq!(visible_parts(&app), (33 * 3, 33 * 9));
        // 34 zones would show 408 parts: the farthest gives up its authored parts.
        show(&mut app, zones(34));
        assert_eq!(visible_parts(&app), (34 * 3, 33 * 9));
        let hidden: Vec<u64> = (1..=34)
            .filter(|id| {
                let instance = &bodies(&app)[&EffectKey::Runtime(*id)];
                *app.world()
                    .get::<Visibility>(part(instance, Role::Core))
                    .unwrap()
                    == Visibility::Hidden
            })
            .collect();
        assert_eq!(hidden, [34]);
        // The ceiling of the server: every boundary, fill and marker is still drawn.
        show(&mut app, zones(shared::loadout::MAX_ACTIVE_EFFECTS as u64));
        let (kept, authored) = visible_parts(&app);
        assert_eq!(kept, 128 * 3);
        assert_eq!(authored, 9);
        assert!(kept + authored <= bodies::PART_BUDGET);
        let nearest = &bodies(&app)[&EffectKey::Runtime(1)];
        assert_eq!(
            *app.world()
                .get::<Visibility>(part(nearest, Role::Core))
                .unwrap(),
            Visibility::Inherited
        );
        #[cfg(feature = "qa")]
        {
            let far = &bodies(&app)[&EffectKey::Runtime(128)];
            let evidence = |root| {
                app.world()
                    .get::<super::super::SkillBodyVisual>(root)
                    .unwrap()
                    .budget_hidden
            };
            assert_eq!((evidence(nearest.root), evidence(far.root)), (0, 9));
        }
        // With room again the looks return.
        show(&mut app, zones(5));
        assert_eq!(visible_parts(&app), (5 * 3, 5 * 9));
    }

    #[test]
    fn staged_bodies_clear_on_fog_round_or_2d_switch_and_wait_for_their_model() {
        let mut app = app(SkillPresentation::target());
        let hook = replicated(6, SkillId::IronHook, EffectVisualKind::Bolt);
        show(&mut app, vec![hook.clone()]);
        let instance = &bodies(&app)[&EffectKey::Runtime(6)];
        let (root, model) = (instance.root, part(instance, Role::Model));
        // The packaged scene is not loaded in this app: the prop stays hidden, the rest of
        // the body is drawn.
        assert_eq!(
            *app.world().get::<Visibility>(model).unwrap(),
            Visibility::Hidden
        );
        assert!(app.world().entity(model).contains::<WorldAssetRoot>());
        assert_eq!(
            *app.world()
                .get::<Visibility>(part(instance, Role::Shell))
                .unwrap(),
            Visibility::Inherited
        );
        // The same object in the next snapshot keeps its entities and follows the position.
        let mut moved = hook.clone();
        moved.position = [5.1, 5.0];
        moved.end = [6.1, 5.0];
        show(&mut app, vec![moved]);
        assert_eq!(bodies(&app)[&EffectKey::Runtime(6)].root, root);
        assert_eq!(
            app.world().get::<Transform>(root).unwrap().translation.x,
            5.1
        );
        // Omitted by fog: the body and its children are gone, without a one-shot.
        show(&mut app, Vec::new());
        assert!(app.world().get_entity(root).is_err());
        assert!(app.world().get_entity(model).is_err());
        assert!(bodies(&app).is_empty());
        // A reused id of another round is another object.
        show(&mut app, vec![hook.clone()]);
        let old = bodies(&app)[&EffectKey::Runtime(6)].root;
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .meta
            .match_id += 1;
        app.update();
        assert!(app.world().get_entity(old).is_err());
        assert_ne!(bodies(&app)[&EffectKey::Runtime(6)].root, old);
        // The flat view draws no 3D body.
        *app.world_mut().resource_mut::<PlayerVisualMode>() = PlayerVisualMode::Sprite2d;
        app.update();
        assert!(bodies(&app).is_empty());
        assert_eq!(
            app.world_mut().query::<&Mesh3d>().iter(app.world()).count(),
            0
        );
    }

    #[test]
    fn live_objects_reuse_identity_and_clear_on_fog_round_or_2d_switch() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_asset::<WorldAsset>()
            .insert_resource(PlayerVisualMode::Models3d)
            .init_resource::<GameStateSnapshot>()
            .insert_resource(
                SkillPresentation::parse(include_str!("../../assets/config/skills.skillfx"))
                    .unwrap(),
            )
            .add_plugins(SkillEffectsPlugin);
        let mut e = SkillEffectState {
            id: 9,
            owner_id: 7,
            owner_team: shared::map::Team::Green,
            skill: shared::loadout::SkillId::WildRocket,
            kind: EffectVisualKind::Rocket,
            position: [4.0, 5.0],
            end: [10.0, 5.0],
            radius: 0.8,
            remaining_secs: 1.0,
            armed: true,
            consumed_segments: 0,
        };
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .skill_effects
            .push(e.clone());
        app.update();
        let root = app.world().resource::<Instances>().bodies[&EffectKey::Runtime(9)].root;
        assert_eq!(
            *app.world().get::<Visibility>(root).unwrap(),
            Visibility::Inherited
        );
        assert_eq!(
            app.world().get::<Transform>(root).unwrap().translation.x,
            4.0
        );
        e.position[0] = 6.0;
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .skill_effects[0] = e.clone();
        app.update();
        assert_eq!(
            app.world().resource::<Instances>().bodies[&EffectKey::Runtime(9)].root,
            root
        );
        assert_eq!(
            app.world().get::<Transform>(root).unwrap().translation.x,
            6.0
        );
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .skill_effects
            .clear();
        app.update();
        assert!(
            app.world().get_entity(root).is_err(),
            "fog omission removes visual and children"
        );
        assert!(app.world().resource::<Instances>().bodies.is_empty());
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .skill_effects
            .push(e);
        app.update();
        let old = app.world().resource::<Instances>().bodies[&EffectKey::Runtime(9)].root;
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .meta
            .match_id += 1;
        app.update();
        assert!(
            app.world().get_entity(old).is_err(),
            "reused IDs do not survive round changes"
        );
        *app.world_mut().resource_mut::<PlayerVisualMode>() = PlayerVisualMode::Sprite2d;
        app.update();
        assert!(app.world().resource::<Instances>().bodies.is_empty());
    }
}
