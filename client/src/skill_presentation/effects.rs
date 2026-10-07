//! Persistent, bounded drawables for authoritative skill objects. Despawn is silent.
//! An effect whose row gives it a `body` is drawn from that block (`bodies.rs`); every
//! other effect keeps the legacy style of its row or of its kind.
use super::accents::Palette;
use super::bodies::{self, Load, Paint, PartSlot, Role, Seen, VfxMeshes};
use super::cast::CastKey;
use super::category::SkillKey;
use super::geometry::GeoShape;
use super::schema::Body;
use super::stage::{self, EffectKey, EffectMemory};
use super::vocab::{Model, PaletteSlot};
use super::{EffectStyle, SkillPresentation};
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
                (sync, sync_bodies, finish)
                    .chain()
                    .before(bevy::transform::TransformSystems::Propagate),
            );
    }
}

#[derive(Resource)]
struct Geometry {
    ball: Handle<Mesh>,
    cone: Handle<Mesh>,
    block: Handle<Mesh>,
    ring: Handle<Mesh>,
    disc: Handle<Mesh>,
    white: Handle<StandardMaterial>,
    friendly: Handle<StandardMaterial>,
    hostile: Handle<StandardMaterial>,
    colors: HashMap<String, Handle<StandardMaterial>>,
    fills: HashMap<String, Handle<StandardMaterial>>,
    rocket: Handle<WorldAsset>,
    trap: Handle<WorldAsset>,
    props: HashMap<&'static str, Handle<WorldAsset>>,
}

impl Geometry {
    /// The packaged scene of a prop a body carries.
    fn model(&self, model: Model) -> Option<Handle<WorldAsset>> {
        match model {
            Model::Rocket => Some(self.rocket.clone()),
            Model::Trap => Some(self.trap.clone()),
            Model::Hook | Model::Lantern | Model::Orb => self.props.get(model.id()).cloned(),
        }
    }
}

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

fn material(color: Color) -> StandardMaterial {
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
fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    server: Res<AssetServer>,
) {
    commands.init_resource::<BodyPaints>();
    commands.insert_resource(Geometry {
        ball: meshes.add(Sphere::new(1.0).mesh().ico(1).unwrap()),
        cone: meshes.add(Cone::new(1.0, 1.0).mesh().resolution(5)),
        block: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        ring: meshes.add(Annulus::new(0.94, 1.0)),
        disc: meshes.add(Circle::new(1.0)),
        white: materials.add(material(hdr_tint(Color::srgb(1.0, 0.97, 0.82), 5.0))),
        friendly: materials.add(material(hdr_tint(Color::srgb(0.12, 0.95, 0.60), 2.0))),
        hostile: materials.add(material(hdr_tint(Color::srgb(1.0, 0.12, 0.08), 2.0))),
        colors: HashMap::new(),
        fills: HashMap::new(),
        rocket: server.load("cosmetics/standard/rocket.glb#Scene0"),
        trap: server.load("cosmetics/standard/trap.glb#Scene0"),
        props: ["hook", "lantern", "orb", "pillar", "colossus"]
            .into_iter()
            .map(|name| {
                (
                    name,
                    server.load(format!("cosmetics/standard/{name}.glb#Scene0")),
                )
            })
            .collect(),
    });
}

#[derive(Clone, Copy)]
enum Part {
    Core,
    Boundary,
    Inner,
    Satellite(u8),
    Streak(u8),
    Model,
    GroundGlow,
}
struct Instance {
    root: Entity,
    owner: u64,
    parts: Vec<(Entity, Part)>,
    skill: shared::loadout::SkillId,
    kind: EffectVisualKind,
    style: EffectStyle,
    friendly: bool,
    model: Option<Handle<WorldAsset>>,
    /// Where the root stands, for the choice of the nearest lights.
    at: Vec3,
    light: Option<Entity>,
    /// The meshes of the packaged scene no longer cast or receive shadows.
    unshadowed: bool,
}
#[derive(Resource, Default)]
struct Instances {
    round: Option<(u64, u64)>,
    /// Effects drawn through a legacy style, by replicated id.
    objects: HashMap<u64, Instance>,
    /// Effects drawn through the `body` of their row.
    bodies: HashMap<EffectKey, BodyInstance>,
}

pub(crate) fn valid_effect(e: &SkillEffectState) -> bool {
    e.position.into_iter().chain(e.end).all(f32::is_finite)
        && e.radius.is_finite()
        && (0.0..=256.0).contains(&e.radius)
        && e.remaining_secs.is_finite()
        && e.remaining_secs >= 0.0
}

fn spawn_instance(
    commands: &mut Commands,
    e: &SkillEffectState,
    style: EffectStyle,
    friendly: bool,
    geometry: &Geometry,
    color: Handle<StandardMaterial>,
    fill: Handle<StandardMaterial>,
) -> Instance {
    let root = commands
        .spawn((
            Transform::default(),
            Visibility::Hidden,
            Name::new(format!("SkillVfx-{}-{}", e.skill.id(), e.id)),
        ))
        .id();
    let mut parts = Vec::new();
    let boundary = if friendly {
        &geometry.friendly
    } else {
        &geometry.hostile
    };
    let mut part = |role, mesh: &Handle<Mesh>, mat: &Handle<StandardMaterial>| {
        let entity = commands
            .spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(mat.clone()),
                Transform::default(),
                Visibility::default(),
                NotShadowCaster,
                NotShadowReceiver,
                ChildOf(root),
            ))
            .id();
        parts.push((entity, role));
    };
    match style {
        EffectStyle::Cage | EffectStyle::Wall | EffectStyle::Fissure | EffectStyle::Cone => {
            let count = if style == EffectStyle::Cone {
                18
            } else if style == EffectStyle::Fissure {
                12
            } else if style == EffectStyle::Wall {
                11
            } else {
                10
            };
            for i in 0..count {
                part(
                    Part::Satellite(i),
                    if matches!(style, EffectStyle::Wall | EffectStyle::Fissure) && i < 10 {
                        &geometry.cone
                    } else if style == EffectStyle::Cone && i >= 10 {
                        &geometry.ball
                    } else {
                        &geometry.block
                    },
                    if (style == EffectStyle::Cage && i < 5)
                        || (style == EffectStyle::Cone && i < 10)
                        || (matches!(style, EffectStyle::Fissure | EffectStyle::Wall) && i >= 10)
                    {
                        boundary
                    } else {
                        &color
                    },
                );
            }
        }
        EffectStyle::Orb
        | EffectStyle::Lantern
        | EffectStyle::Pillar
        | EffectStyle::Colossus
        | EffectStyle::Hook => {
            part(Part::Core, &geometry.ball, &color);
            part(Part::Boundary, &geometry.ring, boundary);
            part(Part::Inner, &geometry.ring, &color);
        }
        EffectStyle::Field | EffectStyle::Pulse => {
            part(Part::Core, &geometry.disc, &fill);
            part(Part::Boundary, &geometry.ring, boundary);
            part(Part::Inner, &geometry.ring, &color);
            for i in 0..6 {
                part(Part::Satellite(i), &geometry.ball, &color);
            }
        }
        EffectStyle::Beam => {
            part(Part::Core, &geometry.block, &fill);
            for i in 0..2 {
                part(Part::Streak(i), &geometry.block, boundary);
            }
            part(Part::Streak(2), &geometry.block, &geometry.white);
            part(Part::Inner, &geometry.ring, &color);
        }
        EffectStyle::Trap => {
            part(Part::Core, &geometry.ball, &color);
            part(Part::Boundary, &geometry.ring, boundary);
            for i in 0..6 {
                part(Part::Satellite(i), &geometry.cone, &color);
            }
        }
        EffectStyle::Aegis => {
            part(Part::Core, &geometry.ball, &geometry.white);
            part(Part::Boundary, &geometry.ring, boundary);
            part(Part::Inner, &geometry.ring, &color);
        }
        _ => {
            part(Part::Core, &geometry.ball, &color);
            part(Part::Inner, &geometry.ball, &geometry.white);
            part(Part::Boundary, &geometry.ring, boundary);
            for i in 0..3 {
                part(
                    Part::Streak(i),
                    &geometry.block,
                    if i == 0 { &geometry.white } else { &color },
                );
            }
        }
    }
    if style == EffectStyle::Rocket {
        part(Part::GroundGlow, &geometry.disc, &fill);
    }
    let model = match style {
        EffectStyle::Rocket => Some(geometry.rocket.clone()),
        EffectStyle::Trap => Some(geometry.trap.clone()),
        EffectStyle::Hook => geometry.props.get("hook").cloned(),
        EffectStyle::Lantern => geometry.props.get("lantern").cloned(),
        EffectStyle::Orb => geometry.props.get("orb").cloned(),
        EffectStyle::Pillar => geometry.props.get("pillar").cloned(),
        EffectStyle::Colossus => geometry.props.get("colossus").cloned(),
        _ => None,
    };
    if let Some(scene) = &model {
        let entity = commands
            .spawn((
                WorldAssetRoot(scene.clone()),
                Transform::default(),
                Visibility::Hidden,
                ChildOf(root),
            ))
            .id();
        parts.push((entity, Part::Model));
    }
    Instance {
        root,
        owner: e.owner_id,
        parts,
        skill: e.skill,
        kind: e.kind,
        style,
        friendly,
        model,
        at: Vec3::ZERO,
        light: None,
        unshadowed: false,
    }
}

/// Kind describes the actual replicated object; e.g. a hook's soul is not another hook.
fn kind_style(e: &SkillEffectState) -> Option<EffectStyle> {
    Some(match e.kind {
        EffectVisualKind::Orb => EffectStyle::Orb,
        EffectVisualKind::Soul => EffectStyle::Ember,
        EffectVisualKind::Anchor => EffectStyle::Aegis,
        EffectVisualKind::Healing => EffectStyle::Field,
        EffectVisualKind::Lantern => EffectStyle::Lantern,
        EffectVisualKind::Cage => EffectStyle::Cage,
        EffectVisualKind::ShieldWall => EffectStyle::Wall,
        EffectVisualKind::BeamWarning if e.skill == shared::loadout::SkillId::HorizonWave => {
            EffectStyle::Beam
        }
        _ => return None,
    })
}
fn effect_style(e: &SkillEffectState, configured: EffectStyle) -> EffectStyle {
    kind_style(e).unwrap_or(configured)
}
fn bar(a: Vec3, b: Vec3, width: f32, height: f32) -> Transform {
    let d = b - a;
    Transform::from_translation((a + b) * 0.5)
        .with_rotation(Quat::from_rotation_y(d.x.atan2(d.z)))
        .with_scale(Vec3::new(width, height, d.length().max(0.01)))
}
/// Boundary geometry is server-owned; embellishment never expands the hit volume.
fn structure_part(
    e: &SkillEffectState,
    style: EffectStyle,
    i: u8,
    now: f32,
) -> (Transform, Visibility) {
    let r = e.radius.max(0.05);
    let length = Vec2::from_array(e.position).distance(Vec2::from_array(e.end));
    let mut visible = Visibility::Inherited;
    let t = match style {
        EffectStyle::Cage => {
            let side = i % 5;
            if e.consumed_segments & (1 << side) != 0 {
                visible = Visibility::Hidden;
            }
            let a = side as f32 * std::f32::consts::TAU / 5.0;
            let b = (side + 1) as f32 * std::f32::consts::TAU / 5.0;
            let h = if i < 5 { 0.10 } else { 0.85 };
            bar(
                Vec3::new(a.cos() * r, h, a.sin() * r),
                Vec3::new(b.cos() * r, h, b.sin() * r),
                0.10,
                if i < 5 { 0.12 } else { 0.09 },
            )
        }
        EffectStyle::Wall if i == 10 => bar(
            Vec3::new(-r, 0.02, 1.0),
            Vec3::new(r, 0.02, 1.0),
            0.06,
            0.07,
        ),
        EffectStyle::Wall => {
            // Interception is centered exactly one metre ahead of its owner.
            let x = (i as f32 / 9.0 * 2.0 - 1.0) * r * 0.915;
            Transform::from_xyz(x, 0.65, 1.0).with_scale(Vec3::new(
                r * 0.085,
                1.25 + 0.18 * (i as f32).sin(),
                0.15,
            ))
        }
        EffectStyle::Fissure if i >= 10 => {
            let x = if i == 10 { -r } else { r };
            bar(
                Vec3::new(x, 0.02, 0.0),
                Vec3::new(x, 0.02, length),
                0.055,
                0.05,
            )
        }
        EffectStyle::Fissure => {
            let z = (i as f32 + 0.5) / 10.0 * length;
            Transform::from_xyz(
                if i.is_multiple_of(2) {
                    -r * 0.55
                } else {
                    r * 0.55
                },
                0.25,
                z,
            )
            .with_rotation(Quat::from_rotation_z(if i.is_multiple_of(2) {
                -0.3
            } else {
                0.3
            }))
            .with_scale(Vec3::new(
                r * 0.24,
                0.8 + 0.15 * (now * 3.0 + i as f32).sin(),
                length / 24.0,
            ))
        }
        EffectStyle::Cone => {
            let half = 0.6_f32.acos();
            let point = |angle: f32| Vec3::new(angle.sin() * length, 0.04, angle.cos() * length);
            if i >= 10 {
                let phase = (now * 1.8 + (i - 10) as f32 / 8.0).fract();
                let angle = ((i % 3) as f32 - 1.0) * half * 0.55;
                Transform::from_translation(point(angle) * phase + Vec3::Y * (0.25 + phase * 0.3))
                    .with_scale(Vec3::splat(0.10 + phase * 0.25))
            } else if i < 8 {
                bar(
                    point(-half + 2.0 * half * i as f32 / 8.0),
                    point(-half + 2.0 * half * (i + 1) as f32 / 8.0),
                    0.07,
                    0.06,
                )
            } else {
                bar(
                    Vec3::Y * 0.04,
                    point(if i == 8 { -half } else { half }),
                    0.07,
                    0.06,
                )
            }
        }
        _ => {
            visible = Visibility::Hidden;
            Transform::default()
        }
    };
    (t, visible)
}

/// Uses received (possibly fog-clipped) geometry, never reconstructs a hidden origin.
fn sync(
    mut commands: Commands,
    game: Option<Res<GameStateSnapshot>>,
    mode: Res<PlayerVisualMode>,
    profiles: Res<SkillPresentation>,
    clock: Res<crate::vfx_clock::VfxClock>,
    map: Option<Res<crate::maps::MapLayout>>,
    local: Query<&crate::team::Team, With<crate::player::Player>>,
    mut instances: ResMut<Instances>,
    mut geometry: ResMut<Geometry>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    server: Res<AssetServer>,
    mut poses: Poses,
    scene_instances: Query<&WorldInstance>,
    scene_spawner: Option<Res<WorldInstanceSpawner>>,
) {
    let round = game
        .as_ref()
        .map(|g| (g.meta.server_epoch, g.meta.match_id));
    if round != instances.round || *mode != PlayerVisualMode::Models3d {
        for (_, instance) in instances.objects.drain() {
            commands.entity(instance.root).despawn();
        }
        for (_, instance) in instances.bodies.drain() {
            commands.entity(instance.root).despawn();
        }
        instances.round = round;
    }
    let Some(game) = game.filter(|_| *mode == PlayerVisualMode::Models3d) else {
        return;
    };
    let now = clock.now as f32;
    let mut live = HashSet::new();
    for e in game
        .skill_effects
        .iter()
        .take(shared::loadout::MAX_ACTIVE_EFFECTS)
    {
        if !valid_effect(e) {
            continue;
        }
        let Some(profile) = profiles.profile(e.skill) else {
            continue;
        };
        // One path draws an effect: the body of its row, when there is one.
        if profiles.body_for(e).is_some() {
            continue;
        }
        // A row that dropped its legacy style still shows the objects whose kind names
        // their look; its other effects wait for the row's `body`.
        let Some(style) = profile
            .effect
            .map(|configured| effect_style(e, configured))
            .or_else(|| kind_style(e))
        else {
            continue;
        };
        let friendly = local.single().is_ok_and(|t| *t == e.owner_team);
        if instances.objects.get(&e.id).is_some_and(|i| {
            i.owner != e.owner_id
                || i.skill != e.skill
                || i.kind != e.kind
                || i.style != style
                || i.friendly != friendly
        }) {
            let previous = instances.objects.remove(&e.id).unwrap();
            commands.entity(previous.root).despawn();
        }
        live.insert(e.id);
        let color_key = format!("{}-{:?}-{}", e.skill.id(), profile.color, profile.hdr_gain);
        let color = geometry
            .colors
            .entry(color_key.clone())
            .or_insert_with(|| {
                materials.add(material(hdr_tint(
                    Color::srgb_from_array(profile.color),
                    profile.hdr_gain,
                )))
            })
            .clone();
        let fill = geometry
            .fills
            .entry(color_key)
            .or_insert_with(|| {
                materials.add(fill_material(
                    Color::srgb_from_array(profile.color),
                    FILL_STRENGTH,
                ))
            })
            .clone();
        let instance = instances.objects.entry(e.id).or_insert_with(|| {
            spawn_instance(&mut commands, e, style, friendly, &geometry, color, fill)
        });
        let p = Vec2::from_array(e.position);
        let end = Vec2::from_array(e.end);
        let direction = (end - p).normalize_or(Vec2::Y);
        let rotation = Quat::from_rotation_y(direction.x.atan2(direction.y));
        let ground = map.as_ref().map_or(0.0, |m| m.terrain_height_3d(p.x, p.y));
        let planar = matches!(
            style,
            EffectStyle::Field
                | EffectStyle::Trap
                | EffectStyle::Beam
                | EffectStyle::Pulse
                | EffectStyle::Pillar
                | EffectStyle::Lantern
                | EffectStyle::Cage
                | EffectStyle::Wall
                | EffectStyle::Fissure
                | EffectStyle::Cone
                | EffectStyle::Colossus
        );
        let root_pose = Transform::from_xyz(p.x, ground + if planar { 0.15 } else { 0.95 }, p.y)
            .with_rotation(rotation);
        instance.at = root_pose.translation;
        place(
            &mut commands,
            &mut poses,
            instance.root,
            root_pose,
            Visibility::Inherited,
        );
        let radius = e.radius.max(0.05);
        let model_ready = instance.model.as_ref().is_some_and(|scene| {
            instance.parts.iter().any(|(entity, role)| {
                matches!(role, Part::Model)
                    && scene_ready(&server, scene, *entity, &scene_instances, &scene_spawner)
            })
        });
        let ground_ring = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
        #[cfg(feature = "qa")]
        commands
            .entity(instance.root)
            .insert(super::SkillEffectVisual {
                id: e.id,
                model_ready,
            });
        for &(entity, role) in &instance.parts {
            let mut visibility = Visibility::Inherited;
            let mut t = Transform::default();
            match (style, role) {
                (EffectStyle::Rocket, Part::GroundGlow) => {
                    // Keep the glow above the existing ground telegraph plane. It marks the
                    // replicated radius; the blast radius is not replicated.
                    t.translation.y = -0.65;
                    t.rotation = ground_ring;
                    t.scale = Vec3::splat(radius);
                }
                (EffectStyle::Rocket, Part::Streak(i)) => {
                    t.translation.z = -1.1 - i as f32 * 0.85;
                    let width = 0.3 - i as f32 * 0.06;
                    t.scale = Vec3::new(width, width, 1.6);
                }
                (_, Part::Model) => {
                    visibility = if model_ready {
                        Visibility::Inherited
                    } else {
                        Visibility::Hidden
                    };
                    t.scale = Vec3::splat(match style {
                        EffectStyle::Trap => radius * 1.6,
                        EffectStyle::Pillar => radius,
                        EffectStyle::Colossus => radius * 0.8,
                        EffectStyle::Orb => 1.25,
                        _ => 1.15,
                    });
                    if style == EffectStyle::Pillar && !e.armed {
                        visibility = Visibility::Hidden;
                    }
                    if style == EffectStyle::Orb {
                        t.rotation = Quat::from_rotation_y(now * 1.5);
                    }
                    if style == EffectStyle::Trap {
                        t.rotation = Quat::from_rotation_y(now * if e.armed { 0.0 } else { 3.0 });
                    }
                }
                (EffectStyle::Field | EffectStyle::Pulse, Part::Core) => {
                    t.scale = Vec3::splat(radius);
                    t.rotation = ground_ring;
                }
                (EffectStyle::Field | EffectStyle::Pulse, Part::Boundary) => {
                    t.scale = Vec3::splat(radius);
                    t.rotation = ground_ring;
                }
                (EffectStyle::Field | EffectStyle::Pulse, Part::Inner) => {
                    t.scale = Vec3::splat(
                        radius
                            * if style == EffectStyle::Pulse {
                                0.2 + 0.7 * (e.remaining_secs * 1.4).clamp(0.0, 1.0)
                            } else {
                                0.65 + 0.03 * (now * 3.0).sin()
                            },
                    );
                    t.rotation = ground_ring;
                    t.translation.y = 0.03;
                }
                (EffectStyle::Field | EffectStyle::Pulse, Part::Satellite(i)) => {
                    let angle = now * 0.5 + i as f32 * std::f32::consts::TAU / 6.0;
                    t.translation = Vec3::new(
                        angle.cos() * radius * 0.82,
                        0.25 + (now * 2.0 + angle).sin() * 0.1,
                        angle.sin() * radius * 0.82,
                    );
                    t.scale = Vec3::splat(0.1);
                }
                (EffectStyle::Beam, Part::Core) => {
                    let length = p.distance(end);
                    t.translation.z = length * 0.5;
                    t.scale = Vec3::new(
                        radius * 2.0,
                        if e.kind == EffectVisualKind::BeamWarning {
                            0.025
                        } else {
                            0.25
                        },
                        length.max(0.01),
                    );
                }
                (EffectStyle::Beam, Part::Streak(i)) => {
                    let length = p.distance(end);
                    if i == 2 {
                        t.translation = Vec3::new(0.0, 0.16, length * 0.5);
                        t.scale = Vec3::new(radius * 0.55, 0.16, length.max(0.01));
                        if e.kind == EffectVisualKind::BeamWarning {
                            visibility = Visibility::Hidden;
                        }
                    } else {
                        t.translation =
                            Vec3::new(if i == 0 { -radius } else { radius }, 0.07, length * 0.5);
                        t.scale = Vec3::new(0.055, 0.07, length.max(0.01));
                    }
                }
                (EffectStyle::Beam, Part::Inner) => {
                    t.rotation = ground_ring;
                    t.scale = Vec3::splat(radius.max(0.5) * (1.0 + 0.1 * (now * 9.0).sin()));
                }
                (EffectStyle::Trap, Part::Boundary) => {
                    t.rotation = ground_ring;
                    t.scale = Vec3::splat(radius);
                }
                (EffectStyle::Trap, Part::Satellite(i)) => {
                    let a = i as f32 * std::f32::consts::TAU / 6.0;
                    t.translation = Vec3::new(a.cos() * radius * 0.6, 0.1, a.sin() * radius * 0.6);
                    t.scale = Vec3::new(0.13, if e.armed { 0.4 } else { 0.1 }, 0.13);
                }
                (EffectStyle::Trap, Part::Core) => {
                    t.translation.y = 0.15;
                    t.scale = Vec3::splat(if e.armed { 0.16 } else { 0.08 });
                }
                (
                    EffectStyle::Cage
                    | EffectStyle::Wall
                    | EffectStyle::Fissure
                    | EffectStyle::Cone,
                    Part::Satellite(i),
                ) => {
                    (t, visibility) = structure_part(e, style, i, now);
                }
                (
                    EffectStyle::Orb
                    | EffectStyle::Lantern
                    | EffectStyle::Pillar
                    | EffectStyle::Colossus
                    | EffectStyle::Hook,
                    Part::Core,
                ) => {
                    t.scale = Vec3::splat(radius * 0.45);
                    if model_ready || (style == EffectStyle::Pillar && !e.armed) {
                        visibility = Visibility::Hidden;
                    }
                }
                (
                    EffectStyle::Orb
                    | EffectStyle::Lantern
                    | EffectStyle::Pillar
                    | EffectStyle::Colossus
                    | EffectStyle::Hook,
                    Part::Inner,
                ) => {
                    t.scale = Vec3::splat(if style == EffectStyle::Lantern {
                        0.5
                    } else {
                        radius * 0.7
                    });
                    t.rotation = if style == EffectStyle::Orb {
                        Quat::from_rotation_x(now)
                    } else {
                        ground_ring
                    };
                    t.translation.y = 0.12;
                }
                (EffectStyle::Aegis, Part::Core) => {
                    t.scale = Vec3::splat(radius * 0.35);
                }
                (EffectStyle::Aegis, Part::Boundary) => {
                    t.scale = Vec3::splat(radius);
                    t.rotation = Quat::from_rotation_z(now * 3.0);
                }
                (EffectStyle::Aegis, Part::Inner) => {
                    t.scale = Vec3::splat(radius * 0.7);
                    t.rotation = Quat::from_rotation_y(now * 4.0);
                }
                (_, Part::Core) => {
                    t.scale = match style {
                        EffectStyle::Needle => {
                            Vec3::new(radius * 0.35, radius * 0.35, radius * 2.8)
                        }
                        EffectStyle::Ember => {
                            Vec3::splat(radius * (0.85 + 0.12 * (now * 12.0).sin()))
                        }
                        EffectStyle::Shock => Vec3::new(radius * 1.4, radius * 0.4, radius),
                        _ => Vec3::new(radius * 0.75, radius * 0.75, radius * 2.0),
                    };
                    if model_ready {
                        visibility = Visibility::Hidden;
                    }
                }
                (_, Part::Inner) => {
                    t.scale = if style == EffectStyle::Shock {
                        Vec3::new(radius * 0.65, radius * 0.25, radius * 0.55)
                    } else {
                        Vec3::splat(radius * 0.3)
                    };
                    t.translation.z = radius * 0.45;
                    if model_ready {
                        visibility = Visibility::Hidden;
                    }
                }
                (_, Part::Boundary) => {
                    t.scale = Vec3::splat(radius);
                    t.rotation = ground_ring;
                    t.translation.y = if planar { 0.02 } else { -0.75 };
                }
                (_, Part::Streak(i)) => {
                    t.translation.z = -0.5 - i as f32 * 0.55;
                    let width = (0.11 - i as f32 * 0.025).max(0.025);
                    t.scale = Vec3::new(width, width, 0.8);
                }
                _ => {}
            }
            if model_ready
                && style == EffectStyle::Trap
                && matches!(role, Part::Core | Part::Satellite(_))
            {
                visibility = Visibility::Hidden;
            }
            place(&mut commands, &mut poses, entity, t, visibility);
        }
    }
    instances.objects.retain(|id, instance| {
        if live.contains(id) {
            true
        } else {
            commands.entity(instance.root).despawn();
            false
        }
    });
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
/// how strongly it is applied while the effect is live and while it is a telegraph.
const FILL_LIGHTNESS: f32 = 0.24;
const FILL_SATURATION: f32 = 0.6;
const FILL_STRENGTH: f32 = 0.92;
const FILL_DIM_STRENGTH: f32 = 0.6;
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
                    PaintKind::Fill => fill_material(color, FILL_STRENGTH),
                    PaintKind::FillDim => fill_material(color, FILL_DIM_STRENGTH),
                })
            })
            .clone()
    }

    /// The handles one body draws with: the colours of its row and the two sides.
    fn resolve(&mut self, materials: &mut Assets<StandardMaterial>, palette: &Palette) -> Paints {
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
            fill: made(PaintKind::Fill, primary.color, 0.0),
            fill_dim: made(PaintKind::FillDim, primary.color, 0.0),
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
    geometry: &Geometry,
    paints: Paints,
    friendly: bool,
) -> BodyInstance {
    let root = commands
        .spawn((Transform::default(), Visibility::Hidden, body_name(e)))
        .id();
    let model = body.model.and_then(|model| geometry.model(model));
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

/// Draws every effect whose row gives it a body. Poses use the received geometry and what
/// the stage tracker observed of the instance; nothing is predicted.
fn sync_bodies(
    mut commands: Commands,
    frame: BodyFrame,
    mut instances: ResMut<Instances>,
    meshes: Res<VfxMeshes>,
    geometry: Res<Geometry>,
    mut paints: ResMut<BodyPaints>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    server: Res<AssetServer>,
    mut poses: Poses,
    scene_instances: Query<&WorldInstance>,
    scene_spawner: Option<Res<WorldInstanceSpawner>>,
) {
    // `sync` has already dropped every instance of another round or view.
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
        // The legacy path drew this effect.
        if instances.objects.contains_key(&e.id) {
            continue;
        }
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
            let resolved = paints.resolve(&mut materials, &look.palette);
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
                        &geometry,
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
    let legacy = instances
        .objects
        .values()
        .map(|instance| instance.parts.len())
        .sum();
    let over = bodies::over_budget(&loads, legacy, bodies::PART_BUDGET);
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
) {
    let eye = camera
        .single()
        .map_or(Vec3::ZERO, |pose| pose.translation());
    let instances = &mut *instances;
    // Per rocket: distance, root, height of the light above the root, and its light.
    let mut rockets: Vec<(f32, Entity, f32, &mut Option<Entity>)> = Vec::new();
    let mut props: Vec<(Entity, &mut bool)> = Vec::new();
    for instance in instances.objects.values_mut() {
        if instance.style == EffectStyle::Rocket {
            rockets.push((
                instance.at.distance(eye),
                instance.root,
                0.6,
                &mut instance.light,
            ));
        }
        let scene = instance
            .parts
            .iter()
            .find(|(_, role)| matches!(role, Part::Model));
        if let Some((scene, _)) = scene {
            props.push((*scene, &mut instance.unshadowed));
        }
    }
    for instance in instances.bodies.values_mut() {
        if instance.body.model == Some(Model::Rocket) {
            rockets.push((
                instance.at.distance(eye),
                instance.root,
                bodies::LIGHT_HEIGHT,
                &mut instance.light,
            ));
        }
        let scene = instance
            .parts
            .iter()
            .find(|part| part.slot.role == Role::Model);
        if let Some(scene) = scene {
            props.push((scene.entity, &mut instance.unshadowed));
        }
    }
    rockets.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (rank, (_, root, height, light)) in rockets.into_iter().enumerate() {
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
                        Transform::from_xyz(0.0, height, -0.6),
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
    #[test]
    fn cage_consumption_and_auxiliary_silhouettes_follow_replicated_state() {
        let mut e = SkillEffectState {
            id: 1,
            owner_id: 7,
            owner_team: shared::map::Team::Green,
            skill: shared::loadout::SkillId::IronHook,
            kind: EffectVisualKind::Soul,
            position: [0.0; 2],
            end: [0.0; 2],
            radius: 5.0,
            remaining_secs: 2.0,
            armed: true,
            consumed_segments: 0b00101,
        };
        assert_eq!(effect_style(&e, EffectStyle::Hook), EffectStyle::Ember);
        e.kind = EffectVisualKind::Orb;
        assert_eq!(effect_style(&e, EffectStyle::Aegis), EffectStyle::Orb);
        for side in 0..5 {
            for layer in [0, 5] {
                let (pose, visibility) = structure_part(&e, EffectStyle::Cage, side + layer, 0.0);
                assert_eq!(visibility == Visibility::Hidden, side == 0 || side == 2);
                assert!(pose.translation.xz().length() <= e.radius);
            }
        }
        e.end = [0.0, 12.0];
        for i in 10..12 {
            let (pose, visibility) = structure_part(&e, EffectStyle::Fissure, i, 0.0);
            assert_eq!(visibility, Visibility::Inherited);
            assert_eq!(pose.translation.x.abs(), e.radius);
            assert_eq!(pose.scale.z, 12.0);
        }
    }
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
    fn an_effect_is_drawn_by_the_body_of_its_row_or_by_a_legacy_style_never_by_both() {
        use super::super::tests::schema_rules;
        use super::super::vocab::Archetype;
        // The sample registry gives `winter_shard`, the orb and `dawn_ray` a body and
        // leaves `iron_hook` and `horizon_wave` as packaged.
        let registry = schema_rules::parse(&schema_rules::samples()).unwrap();
        for id in ["winter_shard", "orbital_command", "dawn_ray"] {
            assert!(registry.row(id).unwrap().effect.is_none());
        }
        let mut app = app(registry);
        show(
            &mut app,
            vec![
                replicated(1, SkillId::IronHook, EffectVisualKind::Bolt),
                replicated(2, SkillId::WinterShard, EffectVisualKind::Bolt),
                replicated(3, SkillId::OrbitalCommand, EffectVisualKind::Orb),
                replicated(4, SkillId::DawnRay, EffectVisualKind::BeamWarning),
                replicated(5, SkillId::HorizonWave, EffectVisualKind::BeamWarning),
            ],
        );
        let instances = app.world().resource::<Instances>();
        assert_eq!(instances.objects[&1].style, EffectStyle::Hook);
        assert_eq!(instances.objects[&5].style, EffectStyle::Beam);
        assert_eq!(instances.objects.len(), 2);
        let shard = &instances.bodies[&EffectKey::Runtime(2)];
        let orb = &instances.bodies[&ORB];
        let ray = &instances.bodies[&EffectKey::Runtime(4)];
        assert_eq!(
            (shard.body.archetype, orb.body.archetype, ray.body.archetype),
            (Archetype::Traveller, Archetype::Orbiter, Archetype::Lane)
        );
        assert_eq!(instances.bodies.len(), 3);
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
        // Every mesh part is kept out of the shadow pass, in both paths.
        let legacy = instances.objects.values().flat_map(|instance| {
            instance
                .parts
                .iter()
                .filter(|(_, role)| !matches!(role, Part::Model))
                .map(|(entity, _)| *entity)
        });
        let staged = instances.bodies.values().flat_map(|instance| {
            instance
                .parts
                .iter()
                .filter(|part| part.slot.mesh.is_some())
                .map(|part| part.entity)
        });
        let meshes: Vec<Entity> = legacy.chain(staged).collect();
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
            // A legacy instance carries no body evidence.
            assert!(
                app.world()
                    .get::<super::super::SkillBodyVisual>(instances.objects[&1].root)
                    .is_none()
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
        assert_eq!(alpha(&instance.paints.fill), FILL_STRENGTH);
        assert_eq!(alpha(&instance.paints.fill_dim), FILL_DIM_STRENGTH);
        for fill in [&instance.paints.fill, &instance.paints.fill_dim] {
            assert_eq!(materials.get(fill).unwrap().alpha_mode, AlphaMode::Multiply);
        }
        assert_eq!(alpha(&instance.paints.primary), 1.0);
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
        // The packaged row is drawn by the legacy style, the final row by its body.
        for (registry, staged) in [
            (SkillPresentation::packaged(), false),
            (SkillPresentation::target(), true),
        ] {
            let mut app = app(registry);
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
                let mut ids: Vec<u64> = (1..=3)
                    .filter(|id| {
                        let root = if staged {
                            instances
                                .bodies
                                .get(&EffectKey::Runtime(*id))
                                .map(|i| i.root)
                        } else {
                            instances.objects.get(id).map(|i| i.root)
                        };
                        root.is_some_and(|root| roots.contains(&root))
                    })
                    .collect();
                assert_eq!(
                    ids.len(),
                    roots.len(),
                    "every light belongs to a live rocket"
                );
                ids.sort_unstable();
                ids
            };
            show(&mut app, rockets(&[1, 2, 3]));
            assert_eq!(
                app.world().resource::<Instances>().bodies.len(),
                if staged { 3 } else { 0 }
            );
            assert_eq!(lit(&mut app), [1, 2], "staged: {staged}");
            // The nearest rocket is gone: the next one is lit, and still only two.
            show(&mut app, rockets(&[2, 3]));
            assert_eq!(lit(&mut app), [2, 3]);
            // A nearer rocket arrives: the farthest gives its light up.
            show(&mut app, rockets(&[1, 2, 3]));
            assert_eq!(lit(&mut app), [1, 2]);
            show(&mut app, Vec::new());
            assert!(lit(&mut app).is_empty());
        }
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

    /// The two fixes the packaged build shows: the rocket's ground mark has the replicated
    /// radius, and effect parts stay out of the shadow pass.
    #[test]
    fn the_legacy_rocket_marks_its_replicated_radius() {
        let mut app = app(SkillPresentation::packaged());
        let rocket = replicated(9, SkillId::WildRocket, EffectVisualKind::Rocket);
        assert_eq!(rocket.radius, 1.05);
        show(&mut app, vec![rocket]);
        let instance = &app.world().resource::<Instances>().objects[&9];
        assert_eq!(instance.style, EffectStyle::Rocket);
        let scale_of = |wanted: fn(&Part) -> bool| {
            let (entity, _) = instance
                .parts
                .iter()
                .find(|(_, role)| wanted(role))
                .unwrap();
            app.world().get::<Transform>(*entity).unwrap().scale
        };
        // Both are unit-radius meshes of the legacy path.
        assert_eq!(
            scale_of(|role| matches!(role, Part::GroundGlow)),
            Vec3::splat(1.05)
        );
        assert_eq!(
            scale_of(|role| matches!(role, Part::Boundary)),
            Vec3::splat(1.05)
        );
        for (entity, role) in &instance.parts {
            let part = app.world().entity(*entity);
            assert_eq!(
                part.contains::<NotShadowCaster>() && part.contains::<NotShadowReceiver>(),
                !matches!(role, Part::Model)
            );
        }
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
        let root = app.world().resource::<Instances>().objects[&9].root;
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
        assert_eq!(app.world().resource::<Instances>().objects[&9].root, root);
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
        assert!(app.world().resource::<Instances>().objects.is_empty());
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .skill_effects
            .push(e);
        app.update();
        let old = app.world().resource::<Instances>().objects[&9].root;
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
        assert!(app.world().resource::<Instances>().objects.is_empty());
    }
}
