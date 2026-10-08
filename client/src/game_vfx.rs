//! Bounded combat particles and presentation of authoritative healing butterflies.
//! Neither trails nor pickup presentation can award damage or healing.
// i18n-strict
use crate::{
    camera::MainCamera,
    maps::MapLayout,
    net::{PlayerUtility, RemotePlayer},
    player::Player,
    skill_presentation::{
        accents,
        cast::{MoveObserved, SkillCastObserved, ThemedDashes, action_yaw},
        vocab::ParticleShape as Shape,
    },
    sprite::PlayerVisualMode,
    world2d::{layer, simulation_xz_to_render_xy},
};
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use shared::combat::ProjectileStyle;

const PARTICLE_BUDGET: usize = 256;
const PARTICLE_HDR_GAIN: f32 = 2.5;
const BUTTERFLY_BUDGET: usize = shared::forest_pickups::FOREST_PICKUP_COUNT * 3;
/// World distance a hasted hero travels between two speed streaks.
const HASTE_STREAK_SPACING: f32 = 0.42;
/// Seconds between ground pulses under a hasted hero.
const HASTE_PULSE_PERIOD: f32 = 0.55;
const DASH_COLOR: Color = Color::srgb(0.55, 0.9, 1.0);
const DASH_CORE_COLOR: Color = Color::srgb(0.92, 0.98, 1.0);
const HASTE_COLOR: Color = Color::srgb(1.0, 0.78, 0.28);
const HASTE_CORE_COLOR: Color = Color::srgb(1.0, 0.93, 0.7);

/// Unlit materials use base color directly, so HDR energy belongs in its RGB,
/// while alpha keeps controlling coverage and the particle's lifetime fade.
pub(crate) fn hdr_tint(color: Color, gain: f32) -> Color {
    let linear = color.to_linear();
    Color::linear_rgba(
        linear.red * gain,
        linear.green * gain,
        linear.blue * gain,
        linear.alpha,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BurstKind {
    Melee,
    Magic,
    Ranged,
    VitalBreak,
}
impl BurstKind {
    pub(crate) fn for_style(style: ProjectileStyle) -> Self {
        match style {
            ProjectileStyle::Arcane | ProjectileStyle::Holy | ProjectileStyle::CasterBolt => {
                Self::Magic
            }
            ProjectileStyle::Crescent | ProjectileStyle::Claw | ProjectileStyle::Standard => {
                Self::Melee
            }
            _ => Self::Ranged,
        }
    }
}
#[derive(Message, Clone)]
pub(crate) struct ImpactBurst {
    pub position: Vec3,
    pub direction: Vec2,
    pub color: Color,
    pub scale: f32,
    pub lifetime: f32,
    pub kind: BurstKind,
    pub seed: u64,
}
#[derive(Message)]
pub(crate) struct ClearCombatVfx;
/// Utility action presentation. Positions are simulation coordinates (XZ on
/// the ground plane) in both render modes, exactly like [`ImpactBurst`].
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub(crate) enum UtilityVfx {
    /// An accepted dash moved the hero `actor` from `from` to `to`.
    Dash {
        actor: u64,
        from: Vec3,
        to: Vec3,
        seed: u64,
    },
    /// One speed streak left behind a hasted hero travelling along `direction`.
    HasteStreak {
        position: Vec3,
        direction: Vec2,
        seed: u64,
    },
    /// A ground pulse under a hasted hero (buff start and while it lasts).
    HastePulse { position: Vec3, seed: u64 },
}
#[derive(Message)]
struct FlightParticles(Vec<Particle>);
/// Decorative particles of an accepted cast: accents, moves, links and stage one-shots.
/// They share the decorative half of the pool with flight trails and are admitted first.
#[derive(Message)]
pub(crate) struct SkillBurst(pub Vec<ParticleSpec>);
/// Particles of an accepted combat receipt. They are admitted with the wire-style bursts,
/// ahead of every decorative particle. The receipt collector is their only writer.
#[derive(Message)]
pub(crate) struct ConfirmedBurst(pub Vec<ParticleSpec>);

/// A colour and the HDR gain its 3D material is drawn with. Flat rendering ignores the gain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Tint {
    pub color: Color,
    pub gain: f32,
}
/// How the size of a particle changes over its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Curve {
    /// Shrinks to nothing.
    Shrink,
    /// Grows from 0.4 to twice its size.
    Grow,
    /// Swells to its size in the first quarter of its life, then holds.
    Pop,
    /// Keeps its size.
    Hold,
    /// Lengthens along its own axis to one and a half times its size while it thins.
    Stretch,
}
impl Curve {
    /// Scale along and across the particle's own axis at life fraction `t`.
    fn scale(self, t: f32) -> Vec2 {
        match self {
            Self::Shrink => Vec2::splat((1. - t).max(0.01)),
            Self::Grow => Vec2::splat(0.4 + t * 1.6),
            Self::Pop => Vec2::splat(1. - 0.6 * (1. - (t * 4.).min(1.)).powi(2)),
            Self::Hold => Vec2::ONE,
            Self::Stretch => Vec2::new(0.5 + t, 1. - t * 0.4),
        }
    }
    /// The largest factor the curve reaches along the particle's axis.
    pub(crate) const fn peak(self) -> f32 {
        match self {
            Self::Shrink | Self::Pop | Self::Hold => 1.,
            Self::Grow => 2.,
            Self::Stretch => 1.5,
        }
    }
}
/// The plane a particle is drawn in. `ParticleSpec::angle` is a ground heading in every case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Orient {
    /// Faces the camera; its axis follows the heading as seen on screen.
    Billboard,
    /// Lies in the horizontal plane with its axis along the heading.
    Ground,
    /// Faces the camera with its axis along its direction of travel; a particle that does
    /// not move keeps the heading.
    Velocity,
}
/// Admission class in the pool. Confirmations get first access and the reserved half;
/// `Skill` and `Trail` share the decorative half, `Skill` first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ParticleClass {
    Confirm,
    Skill,
    Trail,
}
/// What a pooled particle depicts. Capture evidence counts particles per source, because an
/// action sequence and a receipt id can be the same number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ParticleSource {
    /// Wire-style bursts, utility effects, flight puffs and accents of rows without `cast`.
    Engine,
    Accent,
    Move,
    Link,
    /// A classified transition or end of a replicated effect.
    Stage,
    /// The trap cue and the camp hit.
    Cue,
    Impact,
}
/// Half extent of the mesh of a shape at size 1. Every planar silhouette fits the unit
/// circle and is symmetric about its own axis, which points along +X.
pub(crate) const fn unit_radius(shape: Shape) -> f32 {
    match shape {
        Shape::Slash => 0.7,
        Shape::Streak => 0.85,
        _ => 0.5,
    }
}
/// How far ahead of its mesh origin the middle of a curved blade lies at size 1. Every
/// other shape is centred on its origin.
pub(crate) const fn blade_depth(shape: Shape) -> f32 {
    match shape {
        Shape::Slash => 0.62,
        Shape::Crescent => 0.38,
        Shape::Arc => 0.43,
        _ => 0.,
    }
}
/// Delayed particles hold a pool slot while hidden, so the wait is bounded.
const MAX_DELAY: f32 = 0.25;
/// Share of its life for which a generated particle is drawn at its whole coverage.
pub(crate) const HOLD_SHARE: f32 = 0.5;
/// Share of the radius of a dense glow that is covered by `DENSE_COVER` of its colour; the
/// rest fades to nothing at the rim. The core is narrower than the body of a hero and lets
/// a fifth of it through, so a unit under the flash of a hit keeps its outline.
pub(crate) const DENSE_CORE: f32 = 0.34;
const DENSE_COVER: f32 = 0.8;

/// One pooled particle as a pure generator describes it. Positions and velocities are
/// simulation coordinates in both render modes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ParticleSpec {
    /// The action sequence of a cast accent or the id of a combat receipt.
    pub event_id: u64,
    pub origin: Vec3,
    pub velocity: Vec3,
    pub lifetime: f32,
    /// Seconds the particle waits, hidden, before it starts (at most 0.25).
    pub delay: f32,
    pub size: f32,
    /// Ground heading of the particle's own axis, in radians.
    pub angle: f32,
    pub color: Tint,
    /// The colour the particle reaches at the end of its life.
    pub end_color: Option<Tint>,
    pub shape: Shape,
    /// Downward acceleration; a negative value rises.
    pub gravity: f32,
    /// Share of the velocity lost per second.
    pub drag: f32,
    /// Turn rate of the particle's own axis, in radians per second.
    pub spin: f32,
    pub curve: Curve,
    pub orient: Orient,
    /// Decorative bursts of one frame are admitted in ascending order: the local hero
    /// first, then by distance.
    pub sort_key: u32,
    pub source: ParticleSource,
    /// A glow drawn with the dense texture: covered in its colour over the inner
    /// `DENSE_CORE` of its radius, fading only from there to its rim.
    pub dense: bool,
}
impl ParticleSpec {
    pub(crate) const BASE: Self = Self {
        event_id: 0,
        origin: Vec3::ZERO,
        velocity: Vec3::ZERO,
        lifetime: 0.35,
        delay: 0.,
        size: 1.,
        angle: 0.,
        color: Tint {
            color: Color::WHITE,
            gain: PARTICLE_HDR_GAIN,
        },
        end_color: None,
        shape: Shape::Glow,
        gravity: 0.,
        drag: 0.,
        spin: 0.,
        curve: Curve::Shrink,
        orient: Orient::Billboard,
        sort_key: 0,
        source: ParticleSource::Engine,
        dense: false,
    };
    /// Seconds after the burst at which the particle is gone.
    pub(crate) fn end_secs(&self) -> f32 {
        self.delay + self.lifetime
    }
    /// Farthest the particle is drawn from `from` on the ground plane: the travel of its
    /// centre plus its own half extent. Drag only slows a particle along its line, so the
    /// centre is farthest at one end of its life.
    pub(crate) fn reach(&self, from: Vec3) -> f32 {
        let live = Particle::from_spec(self, ParticleClass::Skill);
        let centre = [0., self.lifetime]
            .map(|age| (self.origin + live.travel(age) - from).xz().length())
            .into_iter()
            .fold(0., f32::max);
        centre + unit_radius(self.shape) * self.size * self.curve.peak()
    }
    /// The pose `age` seconds after the particle started.
    #[cfg(test)]
    pub(crate) fn pose_at(&self, age: f32, flat: bool, facing: Quat) -> Transform {
        let mut live = Particle::from_spec(self, ParticleClass::Skill);
        live.age = age;
        live.pose(flat, facing)
    }
    /// Whether the pool can draw the particle: finite fields, a positive life and size, and
    /// a finite pose in both render modes for its whole life.
    pub(crate) fn is_sound(&self) -> bool {
        if !(self.lifetime.is_finite()
            && self.lifetime > 0.
            && self.size.is_finite()
            && self.size > 0.
            && self.delay.is_finite()
            && self.delay >= 0.)
        {
            return false;
        }
        let mut live = Particle::from_spec(self, ParticleClass::Skill);
        [0., 0.5, 1.].into_iter().all(|t| {
            live.age = self.lifetime * t;
            [false, true].into_iter().all(|flat| {
                let pose = live.pose(flat, Quat::IDENTITY);
                pose.translation.is_finite() && pose.scale.is_finite() && pose.rotation.is_finite()
            })
        })
    }
}

#[derive(Clone)]
struct Particle {
    event_id: u64,
    origin: Vec3,
    velocity: Vec3,
    age: f32,
    lifetime: f32,
    size: f32,
    angle: f32,
    color: Color,
    shape: Shape,
    gain: f32,
    end_color: Option<Tint>,
    gravity: f32,
    drag: f32,
    spin: f32,
    /// `None` keeps the built-in motion of the shape: the glow shrinks, the ring grows, the
    /// slash sweeps and the streak tapers.
    curve: Option<Curve>,
    orient: Orient,
    class: ParticleClass,
    #[cfg_attr(not(feature = "qa"), allow(dead_code))] // read by capture evidence
    source: ParticleSource,
    dense: bool,
}
impl Particle {
    /// The fields a wire-style burst, a utility effect and a flight puff leave alone.
    const BASE: Self = Self {
        event_id: 0,
        origin: Vec3::ZERO,
        velocity: Vec3::ZERO,
        age: 0.,
        lifetime: 1.,
        size: 1.,
        angle: 0.,
        color: Color::WHITE,
        shape: Shape::Glow,
        gain: PARTICLE_HDR_GAIN,
        end_color: None,
        gravity: 0.,
        drag: 0.,
        spin: 0.,
        curve: None,
        orient: Orient::Billboard,
        class: ParticleClass::Confirm,
        source: ParticleSource::Engine,
        dense: false,
    };
    fn from_spec(spec: &ParticleSpec, class: ParticleClass) -> Self {
        Self {
            event_id: spec.event_id,
            origin: spec.origin,
            velocity: spec.velocity,
            age: -spec.delay.clamp(0., MAX_DELAY),
            lifetime: spec.lifetime,
            size: spec.size,
            angle: spec.angle,
            color: spec.color.color,
            shape: spec.shape,
            gain: spec.color.gain,
            end_color: spec.end_color,
            gravity: spec.gravity,
            drag: spec.drag,
            spin: spec.spin,
            curve: Some(spec.curve),
            orient: spec.orient,
            class,
            source: spec.source,
            dense: spec.dense,
        }
    }
    /// Seconds of undamped flight that cover the same distance as `age` seconds with drag.
    fn glide(&self, age: f32) -> f32 {
        if self.drag > 1e-3 {
            (1. - (-self.drag * age).exp()) / self.drag
        } else {
            age
        }
    }
    /// Displacement after `age` seconds under drag and gravity.
    fn travel(&self, age: f32) -> Vec3 {
        let glide = self.glide(age);
        let fall = if self.drag > 1e-3 {
            (age - glide) / self.drag
        } else {
            0.5 * age * age
        };
        self.velocity * glide - Vec3::Y * (self.gravity * fall)
    }
    /// Coverage at life fraction `t`. A wire-style burst, a utility effect and a flight
    /// puff fade from their first frame. A generated particle keeps its whole coverage for
    /// `HOLD_SHARE` of its life and fades over the rest: a short accent or impact is drawn
    /// in its colour before it thins out over pale ground.
    fn opacity(&self, t: f32) -> f32 {
        let left = (1. - t).max(0.);
        match self.curve {
            Some(_) => (left / (1. - HOLD_SHARE)).min(1.),
            None => left,
        }
    }
    /// Colour and HDR gain at life fraction `t`.
    fn tint(&self, t: f32) -> Tint {
        match self.end_color {
            Some(end) => Tint {
                color: self.color.mix(&end.color, t),
                gain: self.gain + (end.gain - self.gain) * t,
            },
            None => Tint {
                color: self.color,
                gain: self.gain,
            },
        }
    }
    fn pose(&self, flat: bool, facing: Quat) -> Transform {
        let t = (self.age / self.lifetime).clamp(0., 1.);
        let p = self.origin + self.travel(self.age);
        let position = if flat {
            simulation_xz_to_render_xy(p).extend(layer::VFX + 0.1)
        } else {
            p
        };
        let (scale, sweep) = match self.curve {
            Some(curve) => (curve.scale(t), 0.),
            None => match self.shape {
                Shape::Glow => (Vec2::splat((1. - t).max(0.01)), 0.),
                Shape::Ringlet => (Vec2::splat(0.4 + t * 1.6), 0.),
                Shape::Slash => (Vec2::splat(0.75 + t * 0.5), t * 1.1),
                Shape::Streak => (Vec2::splat(1. - t * 0.45), 0.),
                _ => (Vec2::ONE, 0.),
            },
        };
        let age = self.age.max(0.);
        let turn = sweep + self.spin * age;
        let heading = Vec3::new(self.angle.cos(), 0., self.angle.sin());
        let moving =
            self.velocity * (-self.drag * age).exp() - Vec3::Y * (self.gravity * self.glide(age));
        let axis = if self.orient == Orient::Velocity && moving.length_squared() > 1e-6 {
            moving
        } else {
            heading
        };
        let rotation = if flat {
            // The flat view looks straight down, so only a horizontal direction can turn it.
            let seen = if axis.xz().length_squared() > 1e-6 {
                axis
            } else {
                heading
            };
            Quat::from_rotation_z(seen.z.atan2(seen.x) + turn)
        } else if self.orient == Orient::Ground {
            Quat::from_rotation_y(-(self.angle + turn))
                * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)
        } else {
            let projected = facing.inverse() * axis;
            facing * Quat::from_rotation_z(projected.y.atan2(projected.x) + turn)
        };
        let scale = scale * self.size;
        Transform::from_translation(position)
            .with_scale(scale.extend(scale.max_element()))
            .with_rotation(rotation)
    }
}
#[derive(Component)]
pub(crate) struct ParticleSlot {
    active: Option<Particle>,
    material: Handle<StandardMaterial>,
    flat: Handle<ColorMaterial>,
}
impl ParticleSlot {
    #[cfg(feature = "qa")]
    pub(crate) fn sample(&self) -> Option<(u64, f32)> {
        self.active.as_ref().map(|p| (p.event_id, p.age))
    }
    /// The admission class and the source of the live particle.
    #[cfg(feature = "qa")]
    pub(crate) fn source(&self) -> Option<(ParticleClass, ParticleSource)> {
        self.active.as_ref().map(|p| (p.class, p.source))
    }
}
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct VfxPresentation;
#[derive(Component)]
pub(crate) struct ButterflyWing {
    pub(crate) pickup_id: u64,
    anchor: Vec3,
    phase: f32,
    side: f32,
    material: Handle<StandardMaterial>,
    flat: Handle<ColorMaterial>,
}
#[derive(Component)]
struct ButterflyGlow {
    pickup_id: u64,
    anchor: Vec3,
    phase: f32,
    material: Handle<StandardMaterial>,
    flat: Handle<ColorMaterial>,
}
#[derive(Resource, Default)]
struct PickupReceipts {
    identity: Option<(u64, u64)>,
    sequences: std::collections::HashMap<u64, u64>,
}

#[derive(Resource)]
struct VfxAssets {
    glow_texture: Handle<Image>,
    /// The glow of a `dense` particle.
    dense_texture: Handle<Image>,
    /// One mesh per particle shape, in the order of `Shape::ALL`.
    shapes: Vec<Handle<Mesh>>,
    wing: Handle<Mesh>,
}
impl VfxAssets {
    fn mesh(&self, shape: Shape) -> Handle<Mesh> {
        self.shapes[shape as usize].clone()
    }
}
pub(crate) struct GameVfxPlugin;
impl Plugin for GameVfxPlugin {
    fn build(&self, app: &mut App) {
        crate::vfx_clock::ensure(app);
        app.init_resource::<PickupReceipts>()
            .init_resource::<ThemedDashes>()
            .add_message::<ImpactBurst>()
            .add_message::<ClearCombatVfx>()
            .add_message::<crate::game_audio::AudioCueRequest>()
            .add_message::<UtilityVfx>()
            .add_message::<FlightParticles>()
            .add_message::<SkillBurst>()
            .add_message::<ConfirmedBurst>()
            .add_message::<SkillCastObserved>()
            .add_message::<MoveObserved>()
            .add_message::<crate::skill_presentation::stage::StageEvent>()
            .add_message::<crate::combat_feedback::ConfirmedHit>()
            .add_systems(
                Startup,
                setup.after(crate::skill_presentation::bodies::setup_meshes),
            )
            .add_systems(
                Update,
                emit_haste_trails.after(crate::net::ClientNetPipeline::InterpolateRemotePlayers),
            )
            .add_systems(
                PostUpdate,
                (
                    (
                        pickup_feedback,
                        emit_projectile_particles,
                        emit_skill_cast_particles,
                        accents::emit_cast,
                        accents::emit_moves,
                        accents::emit_stage_oneshots,
                        accents::emit_links,
                        animate_particles,
                    )
                        .chain(),
                    animate_butterflies,
                    animate_butterfly_glows,
                )
                    .in_set(VfxPresentation)
                    .after(bevy::transform::TransformSystems::Propagate)
                    .after(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate)
                    .before(bevy::camera::visibility::VisibilitySystems::CheckVisibility),
            );
    }
}
/// Coverage of the dense glow at `r` radii from its centre: whole over its core, then an
/// even fall to nothing at the rim, which keeps more of its colour than the soft glow's.
fn dense_alpha(r: f32) -> f32 {
    let fade = ((1. - r) / (1. - DENSE_CORE)).clamp(0., 1.);
    DENSE_COVER * fade
}
fn dense_texture() -> Image {
    let mut pixels = Vec::with_capacity(64 * 64 * 4);
    for y in 0..64 {
        for x in 0..64 {
            let u = (x as f32 + 0.5) / 64.;
            let v = (y as f32 + 0.5) / 64.;
            let r = Vec2::new(u * 2. - 1., v * 2. - 1.).length();
            pixels.extend_from_slice(&[255, 255, 255, (dense_alpha(r) * 255.) as u8]);
        }
    }
    Image::new(
        Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}
fn texture(wing: bool) -> Image {
    let mut pixels = Vec::with_capacity(64 * 64 * 4);
    for y in 0..64 {
        for x in 0..64 {
            let u = (x as f32 + 0.5) / 64.;
            let v = (y as f32 + 0.5) / 64.;
            if wing {
                let edge = u.min(1. - u).min(v.min(1. - v));
                let vein = ((v - 0.5).atan2(u + 0.15) * 7.).sin().abs() < 0.075;
                let spot = ((u - 0.7).powi(2) + (v - 0.32).powi(2)).sqrt();
                let value = if edge < 0.08 || vein || (0.05..0.095).contains(&spot) {
                    35
                } else if spot < 0.05 {
                    255
                } else {
                    200
                };
                pixels.extend_from_slice(&[value, value, value, 255]);
            } else {
                let r = Vec2::new(u * 2. - 1., v * 2. - 1.).length();
                let alpha = (1. - r).max(0.).powf(1.6);
                pixels.extend_from_slice(&[255, 255, 255, (alpha * 255.) as u8]);
            }
        }
    }
    Image::new(
        Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}
/// A flat mesh in the particle's own XY plane. Particles are tinted, not textured, so
/// every vertex samples the middle of the glow texture.
fn planar_mesh(positions: Vec<[f32; 2]>, indices: Vec<u32>) -> Mesh {
    let count = positions.len();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        positions
            .into_iter()
            .map(|[x, y]| [x, y, 0.])
            .collect::<Vec<_>>(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 0., 1.]; count])
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.5, 0.5]; count])
    .with_inserted_indices(Indices::U32(indices))
}
/// Radiance of the outline of a shaded silhouette as a share of its middle. The middle
/// keeps the whole HDR gain of its colour and the outline falls to a deep shade of the
/// same hue, so a pale or white-hot shape still has an edge on pale ground.
pub(crate) const RIM_SHADE: f32 = 0.1;
/// Shades a flat mesh through its vertex colours: `1.0` where `lit` holds for the vertex
/// index, `RIM_SHADE` elsewhere. The material colour is multiplied by it in both render
/// modes, so the shading costs no draw and no pool slot.
fn rimmed(mesh: Mesh, lit: impl Fn(usize) -> bool) -> Mesh {
    let count = mesh.count_vertices();
    mesh.with_inserted_attribute(
        Mesh::ATTRIBUTE_COLOR,
        (0..count)
            .map(|index| {
                let shade = if lit(index) { 1. } else { RIM_SHADE };
                [shade, shade, shade, 1.]
            })
            .collect::<Vec<_>>(),
    )
}
/// A closed outline filled as a fan around `centre`: lit in the middle, shaded on the
/// outline.
fn fan_mesh(centre: [f32; 2], rim: &[[f32; 2]]) -> Mesh {
    let mut positions = vec![centre];
    positions.extend_from_slice(rim);
    let count = rim.len() as u32;
    let indices = (0..count)
        .flat_map(|i| [0, 1 + i, 1 + (i + 1) % count])
        .collect();
    rimmed(planar_mesh(positions, indices), |index| index == 0)
}
/// A band along an arc of `outer` radius between two angles; `width(t)` is its thickness
/// at the fraction `t` of the arc.
fn band_mesh(from: f32, to: f32, outer: f32, steps: u32, width: impl Fn(f32) -> f32) -> Mesh {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let a = from + t * (to - from);
        for radius in [outer - width(t), outer] {
            positions.push([a.cos() * radius, a.sin() * radius]);
        }
        if i < steps {
            let j = i * 2;
            indices.extend_from_slice(&[j, j + 1, j + 2, j + 1, j + 3, j + 2]);
        }
    }
    planar_mesh(positions, indices)
}
/// The same band with a lit line along its middle and both edges shaded. Where the band
/// tapers to a point, the point is shaded too.
fn rimmed_band(from: f32, to: f32, outer: f32, steps: u32, width: impl Fn(f32) -> f32) -> Mesh {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    let mut lit = Vec::new();
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let a = from + t * (to - from);
        for (row, radius) in [outer - width(t), outer - 0.5 * width(t), outer]
            .into_iter()
            .enumerate()
        {
            positions.push([a.cos() * radius, a.sin() * radius]);
            lit.push(row == 1 && width(t) > 1e-3);
        }
        if i < steps {
            let j = i * 3;
            for row in [j, j + 1] {
                indices.extend_from_slice(&[row, row + 1, row + 3, row + 1, row + 4, row + 3]);
            }
        }
    }
    rimmed(planar_mesh(positions, indices), |index| lit[index])
}
/// The mesh of one particle shape. Each fits `unit_radius`, is symmetric about its own
/// axis and points along +X, so a mirrored view of it looks the same.
pub(crate) fn shape_mesh(shape: Shape) -> Mesh {
    use std::f32::consts::{FRAC_PI_2, PI, TAU};
    match shape {
        Shape::Glow => Rectangle::new(1., 1.).into(),
        // The stroke is a fifth of the radius: a small ring is still a line that is seen.
        Shape::Ringlet => Annulus::new(0.4, 0.5).into(),
        Shape::Slash => band_mesh(-1.2, 1.2, 0.7, 20, |t| (PI * t).sin() * 0.16),
        Shape::Streak => Rectangle::new(1.7, 0.36).into(),
        Shape::Star => {
            let rim: Vec<_> = (0..8)
                .map(|i| {
                    let (radius, a) = (if i % 2 == 0 { 0.5 } else { 0.17 }, i as f32 * TAU / 8.);
                    [a.cos() * radius, a.sin() * radius]
                })
                .collect();
            fan_mesh([0., 0.], &rim)
        }
        // Two arms from the point. The inside of the point is lit and each arm deepens
        // toward its end and toward its outer edge: a mark with a bright corner, not a
        // bar with a highlight along it, however large it is drawn.
        Shape::Chevron => rimmed(
            planar_mesh(
                vec![
                    [0.5, 0.],
                    [-0.08, 0.49],
                    [-0.3, 0.4],
                    [0.16, 0.],
                    [-0.3, -0.4],
                    [-0.08, -0.49],
                    [0.33, 0.],
                    [-0.19, 0.445],
                    [-0.19, -0.445],
                ],
                vec![
                    0, 1, 7, 0, 7, 6, 6, 7, 2, 6, 2, 3, 6, 3, 4, 6, 4, 8, 0, 6, 8, 0, 8, 5,
                ],
            ),
            |index| index == 3 || index == 6,
        ),
        Shape::Diamond => fan_mesh([0., 0.], &[[0.5, 0.], [0., 0.3], [-0.5, 0.], [0., -0.3]]),
        Shape::Arc => band_mesh(-FRAC_PI_2, FRAC_PI_2, 0.5, 12, |_| 0.14),
        Shape::Drop => {
            // A round bulb behind a point.
            let mut rim = vec![[0.5, 0.]];
            rim.extend((0..=8).map(|i| {
                let a = 1.2 + i as f32 * (TAU - 2.4) / 8.;
                [-0.2 + a.cos() * 0.3, a.sin() * 0.3]
            }));
            fan_mesh([-0.2, 0.], &rim)
        }
        // A lit square in the middle and four arms that shade toward their ends.
        Shape::Cross => rimmed(
            planar_mesh(
                vec![
                    [-0.12, -0.12],
                    [0.12, -0.12],
                    [0.12, 0.12],
                    [-0.12, 0.12],
                    [0.485, -0.12],
                    [0.485, 0.12],
                    [0.12, 0.485],
                    [-0.12, 0.485],
                    [-0.485, 0.12],
                    [-0.485, -0.12],
                    [-0.12, -0.485],
                    [0.12, -0.485],
                ],
                vec![
                    0, 1, 2, 0, 2, 3, 1, 4, 5, 1, 5, 2, 2, 6, 7, 2, 7, 3, 3, 8, 9, 3, 9, 0, 0, 10,
                    11, 0, 11, 1,
                ],
            ),
            |index| index < 4,
        ),
        Shape::Crescent => rimmed_band(-1.3, 1.3, 0.5, 16, |t| (PI * t).sin() * 0.24),
        Shape::Claw => {
            // Three tines side by side, the middle one longest.
            let mut positions = Vec::new();
            let mut indices = Vec::new();
            for (y, back, tip) in [(-0.27, -0.36, 0.38), (0., -0.46, 0.5), (0.27, -0.36, 0.38)] {
                let first = positions.len() as u32;
                positions.extend_from_slice(&[
                    [back, y - 0.06],
                    [back, y + 0.06],
                    [0.1, y + 0.045],
                    [tip, y],
                    [0.1, y - 0.045],
                ]);
                indices.extend([0, 1, 2, 0, 2, 3, 0, 3, 4].map(|i| first + i));
            }
            planar_mesh(positions, indices)
        }
        // A heater shield: the flat top leads, the point trails.
        Shape::Kite => fan_mesh(
            [0.05, 0.],
            &[
                [0.37, 0.33],
                [0.05, 0.38],
                [-0.5, 0.],
                [0.05, -0.38],
                [0.37, -0.33],
            ],
        ),
    }
}
fn wing_mesh() -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [0., 0., 0.],
            [0.24, 0.17, 0.],
            [0.32, 0.05, 0.],
            [0.19, -0.14, 0.],
            [0.06, -0.11, 0.],
        ],
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 0., 1.]; 5])
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[0., 0.5], [0.75, 0.], [1., 0.4], [0.6, 1.], [0.2, 0.9]],
    )
    .with_inserted_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3, 0, 3, 4]))
}
fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut flat_materials: ResMut<Assets<ColorMaterial>>,
    mode: Res<PlayerVisualMode>,
    map: Res<MapLayout>,
    library: Option<Res<crate::skill_presentation::bodies::VfxMeshes>>,
) {
    let glow = images.add(texture(false));
    let wing_texture = images.add(texture(true));
    let assets = VfxAssets {
        glow_texture: glow.clone(),
        dense_texture: images.add(dense_texture()),
        // The flat silhouettes of skill bodies are these very meshes: one asset for both.
        shapes: Shape::ALL
            .iter()
            .map(|shape| {
                library
                    .as_ref()
                    .and_then(|library| library.particle(*shape))
                    .unwrap_or_else(|| meshes.add(shape_mesh(*shape)))
            })
            .collect(),
        wing: meshes.add(wing_mesh()),
    };
    for _ in 0..PARTICLE_BUDGET {
        let material = materials.add(StandardMaterial {
            unlit: true,
            fog_enabled: false,
            alpha_mode: AlphaMode::Add,
            base_color_texture: Some(glow.clone()),
            cull_mode: None,
            double_sided: true,
            ..default()
        });
        let flat = flat_materials.add(ColorMaterial {
            texture: Some(glow.clone()),
            ..default()
        });
        let mut e = commands.spawn((
            Name::new("Pooled cosmetic particle"),
            ParticleSlot {
                active: None,
                material: material.clone(),
                flat: flat.clone(),
            },
            Transform::default(),
            Visibility::Hidden,
            bevy::light::NotShadowCaster,
            bevy::light::NotShadowReceiver,
        ));
        if *mode == PlayerVisualMode::Sprite2d {
            e.insert((Mesh2d(assets.mesh(Shape::Glow)), MeshMaterial2d(flat)));
        } else {
            e.insert((Mesh3d(assets.mesh(Shape::Glow)), MeshMaterial3d(material)));
        }
    }
    let colors = [Color::srgb(0.55, 1., 0.78), Color::srgb(1., 0.88, 0.42)];
    let wing_materials = colors.map(|color| {
        materials.add(StandardMaterial {
            base_color: color,
            base_color_texture: Some(wing_texture.clone()),
            unlit: true,
            cull_mode: None,
            double_sided: true,
            ..default()
        })
    });
    let wing_flat = colors.map(|color| {
        flat_materials.add(ColorMaterial {
            color,
            texture: Some(wing_texture.clone()),
            ..default()
        })
    });
    let anchors = shared::forest_pickups::pickup_layout();
    let glow_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.22, 1.0, 0.60, 0.42),
        base_color_texture: Some(glow.clone()),
        unlit: true,
        alpha_mode: AlphaMode::Add,
        cull_mode: None,
        ..default()
    });
    let glow_flat = flat_materials.add(ColorMaterial {
        color: Color::srgba(0.22, 1.0, 0.60, 0.35),
        texture: Some(glow.clone()),
        ..default()
    });
    for i in 0..BUTTERFLY_BUDGET {
        let pickup_id = (i / 3 + 1) as u64;
        let anchor = Vec2::from_array(anchors[i / 3]);
        let phase = i as f32 * 2.399;
        let base = Vec3::new(
            anchor.x,
            map.terrain_height_3d(anchor.x, anchor.y),
            anchor.y,
        );
        let mut halo = commands.spawn((
            Name::new("Healing butterfly glow"),
            ButterflyGlow {
                pickup_id,
                anchor: base,
                phase,
                material: glow_material.clone(),
                flat: glow_flat.clone(),
            },
            Transform::default(),
            Visibility::Hidden,
            bevy::light::NotShadowCaster,
            bevy::light::NotShadowReceiver,
        ));
        if *mode == PlayerVisualMode::Sprite2d {
            halo.insert((
                Mesh2d(assets.mesh(Shape::Glow)),
                MeshMaterial2d(glow_flat.clone()),
            ));
        } else {
            halo.insert((
                Mesh3d(assets.mesh(Shape::Glow)),
                MeshMaterial3d(glow_material.clone()),
            ));
        }
        for side in [-1., 1.] {
            let mut e = commands.spawn((
                Name::new("Forest butterfly wing"),
                ButterflyWing {
                    pickup_id,
                    anchor: base,
                    phase,
                    side,
                    material: wing_materials[i % 2].clone(),
                    flat: wing_flat[i % 2].clone(),
                },
                Transform::default(),
                Visibility::Hidden,
                bevy::light::NotShadowCaster,
                bevy::light::NotShadowReceiver,
            ));
            if *mode == PlayerVisualMode::Sprite2d {
                e.insert((
                    Mesh2d(assets.wing.clone()),
                    MeshMaterial2d(wing_flat[i % 2].clone()),
                ));
            } else {
                e.insert((
                    Mesh3d(assets.wing.clone()),
                    MeshMaterial3d(wing_materials[i % 2].clone()),
                ));
            }
        }
    }
    commands.insert_resource(assets);
}
fn burst_particles(burst: &ImpactBurst) -> Vec<Particle> {
    if !burst.position.is_finite()
        || !burst.scale.is_finite()
        || !burst.lifetime.is_finite()
        || !burst.direction.is_finite()
    {
        return Vec::new();
    }
    let size = burst.scale.clamp(0.1, 2.);
    let lifetime = burst.lifetime.clamp(0.08, 1.2);
    let origin = burst.position + Vec3::Y * 0.75;
    let angle = burst.direction.y.atan2(burst.direction.x);
    let first = Particle {
        event_id: burst.seed,
        origin,
        velocity: Vec3::ZERO,
        age: 0.,
        lifetime,
        size: size * 2.4,
        angle,
        color: burst.color,
        shape: match burst.kind {
            BurstKind::Magic => Shape::Ringlet,
            BurstKind::Melee => Shape::Slash,
            BurstKind::Ranged => Shape::Glow,
            BurstKind::VitalBreak => Shape::Ringlet,
        },
        ..Particle::BASE
    };
    let mut particles = vec![
        first.clone(),
        Particle {
            shape: Shape::Glow,
            size: size * 2.0,
            lifetime: lifetime * 0.65,
            ..first.clone()
        },
    ];
    let count = match burst.kind {
        BurstKind::Magic => 10,
        BurstKind::Melee => 4,
        BurstKind::Ranged => 6,
        BurstKind::VitalBreak => 10,
    };
    if burst.kind == BurstKind::VitalBreak {
        for turn in [-0.65, 0.65] {
            particles.push(Particle {
                shape: Shape::Streak,
                angle: angle + turn,
                size: size * 2.7,
                color: Color::srgb(1.0, 0.83, 0.35),
                ..first.clone()
            });
        }
    }
    for i in 0..count {
        let phase =
            (burst.seed % 997) as f32 * 0.03 + i as f32 * std::f32::consts::TAU / count as f32;
        let speed = if burst.kind == BurstKind::Melee {
            3.8
        } else {
            3.6
        };
        particles.push(Particle {
            velocity: Vec3::new(
                phase.cos() * speed,
                0.4 + (i % 3) as f32 * 0.6,
                phase.sin() * speed,
            ),
            size: size * 0.85,
            shape: Shape::Glow,
            lifetime: lifetime * 1.4,
            ..first.clone()
        });
    }
    particles
}
/// Deterministic per-particle jitter in `[-1, 1]` from a seed and an index.
pub(crate) fn jitter(seed: u64, index: u64, salt: u64) -> f32 {
    let mut h = seed
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(index.wrapping_mul(0xBF58_476D_1CE4_E5B9))
        .wrapping_add(salt.wrapping_mul(0x94D0_49BB_1331_11EB));
    h ^= h >> 31;
    h = h.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    h ^= h >> 32;
    (h % 20_001) as f32 / 10_000. - 1.
}
/// Dash: a ring collapses at the origin, cyan afterimages trace the travelled
/// path with a short stagger, and the arrival pops with a flash, an expanding
/// ring and radial sparks. A fully blocked dash still flashes in place.
fn dash_particles(from: Vec3, to: Vec3, seed: u64) -> Vec<Particle> {
    if !from.is_finite() || !to.is_finite() {
        return Vec::new();
    }
    let delta = (to - from).with_y(0.);
    let length = delta.length();
    let direction = if length > 0.05 {
        delta / length
    } else {
        Vec3::X
    };
    let angle = direction.z.atan2(direction.x);
    let side = Vec3::new(-direction.z, 0., direction.x);
    let base = Particle {
        event_id: seed,
        origin: from + Vec3::Y * 0.8,
        velocity: Vec3::ZERO,
        age: 0.,
        lifetime: 0.35,
        size: 1.,
        angle,
        color: DASH_COLOR,
        shape: Shape::Glow,
        ..Particle::BASE
    };
    let mut particles = vec![
        Particle {
            shape: Shape::Ringlet,
            size: 2.2,
            lifetime: 0.4,
            ..base.clone()
        },
        Particle {
            size: 2.,
            lifetime: 0.28,
            color: DASH_CORE_COLOR,
            ..base.clone()
        },
    ];
    let afterimages = ((length / 0.6).round() as usize).clamp(0, 8);
    for i in 0..afterimages {
        let along = (i as f32 + 0.5) / afterimages as f32;
        let offset = side * jitter(seed, i as u64, 1) * 0.18;
        particles.push(Particle {
            origin: from + direction * (length * along) + Vec3::Y * 0.85 + offset,
            velocity: direction * 1.6 + Vec3::Y * 0.25,
            age: -0.018 * i as f32,
            lifetime: 0.42,
            size: 1.6,
            shape: Shape::Streak,
            ..base.clone()
        });
        if i % 2 == 0 {
            particles.push(Particle {
                origin: from + direction * (length * along) + Vec3::Y * 0.35,
                velocity: side * jitter(seed, i as u64, 2) * 1.8 + Vec3::Y * 1.4,
                age: -0.018 * i as f32,
                lifetime: 0.55,
                size: 0.32,
                color: DASH_CORE_COLOR,
                ..base.clone()
            });
        }
    }
    let arrival = to + Vec3::Y * 0.8;
    let delay = -0.04;
    particles.push(Particle {
        origin: arrival,
        age: delay,
        lifetime: 0.3,
        size: 3.2,
        color: DASH_CORE_COLOR,
        ..base.clone()
    });
    particles.push(Particle {
        origin: arrival,
        age: delay,
        lifetime: 0.55,
        size: 2.8,
        shape: Shape::Ringlet,
        ..base.clone()
    });
    for i in 0..8u64 {
        let phase = i as f32 * std::f32::consts::TAU / 8. + jitter(seed, i, 3) * 0.3;
        particles.push(Particle {
            origin: arrival - Vec3::Y * 0.3,
            velocity: Vec3::new(
                phase.cos() * 3.2,
                1.6 + jitter(seed, i, 4) * 0.5,
                phase.sin() * 3.2,
            ),
            age: delay,
            lifetime: 0.6,
            size: 0.36,
            ..base.clone()
        });
    }
    particles
}
/// One amber speed line trailing a hasted hero, plus an occasional ember.
fn haste_streak_particles(position: Vec3, direction: Vec2, seed: u64) -> Vec<Particle> {
    if !position.is_finite() || !direction.is_finite() {
        return Vec::new();
    }
    let dir = direction.normalize_or(Vec2::X);
    let back = Vec3::new(-dir.x, 0., -dir.y);
    let side = Vec3::new(-dir.y, 0., dir.x) * if seed.is_multiple_of(2) { 0.28 } else { -0.28 };
    let base = Particle {
        event_id: seed,
        origin: position + Vec3::Y * 0.55 + back * 0.35 + side,
        velocity: back * 1.4 + Vec3::Y * 0.2,
        age: 0.,
        lifetime: 0.42,
        size: 1.25,
        angle: dir.y.atan2(dir.x),
        color: HASTE_COLOR,
        shape: Shape::Streak,
        ..Particle::BASE
    };
    let mut particles = vec![base.clone()];
    if seed.is_multiple_of(3) {
        particles.push(Particle {
            origin: position + Vec3::Y * 0.2 + side * 1.5,
            velocity: back * 0.6 + Vec3::Y * (1.2 + jitter(seed, 0, 5) * 0.4),
            lifetime: 0.5,
            size: 0.28,
            color: HASTE_CORE_COLOR,
            shape: Shape::Glow,
            ..base
        });
    }
    particles
}
/// A ring pulse under a hasted hero with a few rising embers; visible even
/// while the hero stands still so the buff itself reads, not only the trail.
fn haste_pulse_particles(position: Vec3, seed: u64) -> Vec<Particle> {
    if !position.is_finite() {
        return Vec::new();
    }
    let base = Particle {
        event_id: seed,
        origin: position + Vec3::Y * 0.35,
        velocity: Vec3::ZERO,
        age: 0.,
        lifetime: 0.5,
        size: 1.7,
        angle: 0.,
        color: HASTE_COLOR,
        shape: Shape::Ringlet,
        ..Particle::BASE
    };
    let mut particles = vec![base.clone()];
    for i in 0..3u64 {
        let phase = i as f32 * std::f32::consts::TAU / 3. + jitter(seed, i, 6) * 0.5;
        particles.push(Particle {
            origin: position + Vec3::new(phase.cos() * 0.45, 0.1, phase.sin() * 0.45),
            velocity: Vec3::Y * (1.1 + jitter(seed, i, 7) * 0.3),
            lifetime: 0.6,
            size: 0.26,
            color: HASTE_CORE_COLOR,
            shape: Shape::Glow,
            ..base.clone()
        });
    }
    particles
}
fn utility_particles(vfx: &UtilityVfx) -> Vec<Particle> {
    match *vfx {
        UtilityVfx::Dash { from, to, seed, .. } => dash_particles(from, to, seed),
        UtilityVfx::HasteStreak {
            position,
            direction,
            seed,
        } => haste_streak_particles(position, direction, seed),
        UtilityVfx::HastePulse { position, seed } => haste_pulse_particles(position, seed),
    }
}
/// Per-hero haste trail bookkeeping; heroes are keyed by entity and forgotten
/// once they despawn or the buff ends.
#[derive(Default)]
pub(crate) struct HasteTrail {
    last_position: Vec3,
    travelled: f32,
    pulse_in: f32,
    emitted: u64,
}
/// A speed buff of a skill or a passive counts once the replicated movement multiplier
/// exceeds the replicated slow by this factor.
const SPEED_READ_FACTOR: f32 = 1.05;
/// Whether a hero shows the speed read: the utility haste, or a movement multiplier above
/// what its slows alone leave, on a living hero the client sees. Both values are
/// replicated; the read is the same amber for every cause.
fn speed_read(
    utility: &shared::utility::UtilityState,
    loadout: Option<&shared::loadout::LoadoutState>,
    visible: bool,
    alive: bool,
) -> bool {
    let buffed = loadout.is_some_and(|loadout| {
        loadout.movement_multiplier > loadout.slow_multiplier * SPEED_READ_FACTOR
    });
    visible && alive && (utility.haste_active_secs > 0.0 || buffed)
}
/// Follow every hasted hero (local, remote and bot) and pace streaks by
/// distance travelled, so a stationary hero only pulses and a sprinting one
/// leaves a continuous double trail regardless of frame rate.
pub(crate) fn emit_haste_trails(
    clock: Res<crate::vfx_clock::VfxClock>,
    heroes: Query<
        (
            Entity,
            &Transform,
            &PlayerUtility,
            &InheritedVisibility,
            Option<&crate::net::PlayerLoadout>,
            Option<&crate::combat::CombatStats>,
        ),
        Or<(With<Player>, With<RemotePlayer>)>,
    >,
    mut trails: Local<std::collections::HashMap<Entity, HasteTrail>>,
    mut out: MessageWriter<UtilityVfx>,
) {
    let mut seen = Vec::new();
    for (entity, transform, utility, visible, loadout, stats) in &heroes {
        let hasted = speed_read(
            &utility.state,
            loadout.and_then(|loadout| loadout.0.as_ref()),
            visible.get(),
            stats.is_none_or(|s| s.is_alive()),
        );
        if !hasted {
            trails.remove(&entity);
            continue;
        }
        seen.push(entity);
        let position = transform.translation;
        let trail = trails.entry(entity).or_insert_with(|| HasteTrail {
            last_position: position,
            travelled: 0.,
            // Pulse immediately so the buff start is unmistakable.
            pulse_in: 0.,
            emitted: 0,
        });
        let step = (position - trail.last_position).with_y(0.);
        let distance = step.length();
        // A teleport (dash while hasted, respawn) must not draw a streak fence.
        if distance > 6.0 {
            trail.last_position = position;
            trail.travelled = 0.;
            continue;
        }
        let seed_base = entity.to_bits() << 32;
        if distance > 1e-4 {
            let direction = step.xz() / distance;
            trail.travelled += distance;
            let mut emitted_this_frame = 0;
            while trail.travelled >= HASTE_STREAK_SPACING && emitted_this_frame < 4 {
                trail.travelled -= HASTE_STREAK_SPACING;
                emitted_this_frame += 1;
                trail.emitted += 1;
                // Place the streak where the hero was when it crossed the spacing mark.
                let behind = position - step * (trail.travelled / distance).clamp(0., 1.);
                out.write(UtilityVfx::HasteStreak {
                    position: behind,
                    direction,
                    seed: seed_base | trail.emitted,
                });
            }
            if emitted_this_frame == 4 {
                trail.travelled = 0.;
            }
        }
        trail.pulse_in -= clock.delta;
        if trail.pulse_in <= 0. {
            trail.pulse_in = HASTE_PULSE_PERIOD;
            trail.emitted += 1;
            out.write(UtilityVfx::HastePulse {
                position,
                seed: seed_base | trail.emitted,
            });
        }
        trail.last_position = position;
    }
    trails.retain(|entity, _| seen.contains(entity));
}
fn animate_particles(
    mut commands: Commands,
    clock: Res<crate::vfx_clock::VfxClock>,
    mode: Res<PlayerVisualMode>,
    assets: Res<VfxAssets>,
    mut bursts: MessageReader<ImpactBurst>,
    mut receipts: MessageReader<ConfirmedBurst>,
    mut utilities: MessageReader<UtilityVfx>,
    themed: Res<ThemedDashes>,
    mut casts: MessageReader<SkillBurst>,
    mut flight: MessageReader<FlightParticles>,
    mut resets: MessageReader<ClearCombatVfx>,
    cameras: Query<&GlobalTransform, (With<MainCamera>, Without<ParticleSlot>)>,
    mut slots: Query<(
        Entity,
        &mut ParticleSlot,
        &mut Transform,
        &mut GlobalTransform,
        &mut Visibility,
        &mut InheritedVisibility,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut flats: ResMut<Assets<ColorMaterial>>,
) {
    let flat = *mode == PlayerVisualMode::Sprite2d;
    let facing = cameras
        .single()
        .map(|p| p.to_scale_rotation_translation().1)
        .unwrap_or(Quat::IDENTITY);
    let clear = resets.read().next().is_some();
    for (_, mut slot, _, _, _, _) in &mut slots {
        if clear || mode.is_changed() {
            slot.active = None;
        }
        if let Some(p) = &mut slot.active {
            p.age += clock.delta;
            if p.age >= p.lifetime {
                slot.active = None;
            }
        }
    }
    // Confirmed impacts get first access; decorative particles cannot starve hits.
    let mut impacts = Vec::new();
    for burst in bursts.read() {
        if impacts.len() < 96 * 12 {
            impacts.extend(burst_particles(burst));
        }
    }
    for burst in receipts.read() {
        if impacts.len() < 96 * 12 {
            impacts.extend(live_particles(&burst.0, ParticleClass::Confirm));
        }
    }
    // Utility effects are confirmed server actions too (dash acknowledgment,
    // replicated haste), so they queue behind hits rather than with trails.
    // A relocation the choreography painted this frame replaces the generic dash.
    let utility: Vec<_> = utilities
        .read()
        .filter(|vfx| !matches!(vfx, UtilityVfx::Dash { actor, .. } if themed.0.contains(actor)))
        .take(96)
        .flat_map(utility_particles)
        .collect();
    // Decorative intake is bounded per frame. Skill bursts go first, the local hero's
    // before others, and whatever does not fit is dropped, never replayed.
    let mut skill: Vec<&ParticleSpec> = casts.read().flat_map(|burst| &burst.0).collect();
    skill.sort_by_key(|spec| spec.sort_key);
    let mut decorative: Vec<_> =
        live_particles(skill.into_iter().take(48), ParticleClass::Skill).collect();
    for batch in flight.read() {
        let room = 48usize.saturating_sub(decorative.len());
        decorative.extend(batch.0.iter().take(room).map(|p| Particle {
            class: if p.event_id == 0 {
                ParticleClass::Trail
            } else {
                ParticleClass::Skill
            },
            ..p.clone()
        }));
    }
    let mut decorative_count = slots
        .iter()
        .filter(|(_, slot, ..)| {
            slot.active
                .as_ref()
                .is_some_and(|p| p.class != ParticleClass::Confirm)
        })
        .count();
    for p in impacts.into_iter().chain(utility).chain(decorative) {
        // Reserve half the pool for combat confirmations, even during sustained fire.
        if p.class != ParticleClass::Confirm {
            if decorative_count >= PARTICLE_BUDGET / 2 {
                break;
            }
            decorative_count += 1;
        }
        let Some((entity, mut slot, _, _, _, _)) =
            slots.iter_mut().find(|(_, slot, ..)| slot.active.is_none())
        else {
            break;
        };
        let mesh = assets.mesh(p.shape);
        // Glows and streaks use the radial texture; every other shape needs a solid tint.
        let texture = match p.shape {
            Shape::Glow if p.dense => Some(assets.dense_texture.clone()),
            Shape::Glow | Shape::Streak => Some(assets.glow_texture.clone()),
            _ => None,
        };
        if let Some(mut m) = materials.get_mut(&slot.material) {
            m.base_color = hdr_tint(p.color, p.gain);
            m.base_color_texture = texture.clone();
            m.alpha_mode = AlphaMode::Blend;
        }
        if let Some(mut m) = flats.get_mut(&slot.flat) {
            m.color = p.color;
            m.texture = texture;
        }
        if flat {
            commands
                .entity(entity)
                .remove::<(Mesh3d, MeshMaterial3d<StandardMaterial>)>()
                .insert((Mesh2d(mesh), MeshMaterial2d(slot.flat.clone())));
        } else {
            commands
                .entity(entity)
                .remove::<(Mesh2d, MeshMaterial2d<ColorMaterial>)>()
                .insert((Mesh3d(mesh), MeshMaterial3d(slot.material.clone())));
        }
        slot.active = Some(p);
    }
    for (_, slot, mut transform, mut global, mut visibility, mut inherited) in &mut slots {
        // Staggered particles (negative age) hold their slot but stay hidden.
        let Some(p) = slot.active.as_ref().filter(|p| p.age >= 0.) else {
            *visibility = Visibility::Hidden;
            *inherited = InheritedVisibility::HIDDEN;
            continue;
        };
        *transform = p.pose(flat, facing);
        *global = GlobalTransform::from(*transform);
        *visibility = Visibility::Visible;
        *inherited = InheritedVisibility::VISIBLE;
        let life = p.age / p.lifetime;
        let opacity = p.opacity(life);
        let Tint { color, gain } = p.tint(life.clamp(0., 1.));
        if let Some(mut m) = materials.get_mut(&slot.material) {
            m.base_color = hdr_tint(color, gain).with_alpha(color.alpha() * opacity);
        }
        if let Some(mut m) = flats.get_mut(&slot.flat) {
            m.color = color.with_alpha(color.alpha() * opacity);
        }
    }
}
/// Generated particles the pool can draw; a malformed one is dropped, like a malformed burst.
fn live_particles<'a>(
    specs: impl IntoIterator<Item = &'a ParticleSpec>,
    class: ParticleClass,
) -> impl Iterator<Item = Particle> {
    specs
        .into_iter()
        .filter(|spec| spec.is_sound())
        .map(move |spec| Particle::from_spec(spec, class))
}
fn butterfly_position(anchor: Vec3, phase: f32, seconds: f64) -> Vec3 {
    let t = (seconds * 0.7).rem_euclid(std::f64::consts::TAU * 10.) as f32;
    anchor
        + Vec3::new(
            (t + phase).sin() * 0.68,
            1.35 + (t * 1.8 + phase).sin() * 0.30,
            (t * 0.8 + phase).cos() * 0.62,
        )
}
fn animate_butterflies(
    game: Option<Res<crate::net::GameStateSnapshot>>,
    mut commands: Commands,
    assets: Res<VfxAssets>,
    time: Res<Time>,
    mode: Res<PlayerVisualMode>,
    cameras: Query<(&Camera, &GlobalTransform), (With<MainCamera>, Without<ButterflyWing>)>,
    mut wings: Query<(
        Entity,
        &ButterflyWing,
        &mut Transform,
        &mut GlobalTransform,
        &mut Visibility,
        &mut InheritedVisibility,
    )>,
) {
    let camera = cameras.single().ok();
    let flat = *mode == PlayerVisualMode::Sprite2d;
    for (entity, wing, mut transform, mut global, mut visibility, mut inherited) in &mut wings {
        if mode.is_changed() {
            if flat {
                commands
                    .entity(entity)
                    .remove::<(Mesh3d, MeshMaterial3d<StandardMaterial>)>()
                    .insert((
                        Mesh2d(assets.wing.clone()),
                        MeshMaterial2d(wing.flat.clone()),
                    ));
            } else {
                commands
                    .entity(entity)
                    .remove::<(Mesh2d, MeshMaterial2d<ColorMaterial>)>()
                    .insert((
                        Mesh3d(assets.wing.clone()),
                        MeshMaterial3d(wing.material.clone()),
                    ));
            }
        }
        let anchor = pickup_anchor(game.as_deref(), wing.pickup_id, wing.anchor);
        let point = butterfly_position(anchor, wing.phase, time.elapsed_secs_f64());
        let position = if flat {
            simulation_xz_to_render_xy(point).extend(layer::VFX - 1.)
        } else {
            point
        };
        let available = pickup_available(game.as_deref(), wing.pickup_id);
        let visible = available
            && camera.is_some_and(|(cam, pose)| {
                cam.world_to_viewport(pose, position).is_ok_and(|p| {
                    cam.logical_viewport_size().is_some_and(|s| {
                        p.x >= -40. && p.y >= -40. && p.x <= s.x + 40. && p.y <= s.y + 40.
                    })
                })
            });
        *visibility = if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        *inherited = if visible {
            InheritedVisibility::VISIBLE
        } else {
            InheritedVisibility::HIDDEN
        };
        if !visible {
            continue;
        }
        let flap = (time.elapsed_secs() * 12. + wing.phase).sin() * 0.9;
        *transform =
            Transform::from_translation(position).with_scale(Vec3::new(wing.side * 2.2, 2.2, 2.2));
        if flat {
            transform.scale.x *= 0.2 + flap.cos() * 0.8;
            transform.rotation = Quat::from_rotation_z(wing.phase * 0.3);
        } else {
            transform.rotation = Quat::from_rotation_y(wing.phase * 0.3)
                * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)
                * Quat::from_rotation_y(flap * wing.side);
        }
        *global = GlobalTransform::from(*transform);
    }
}
/// Built-in cast accents of the rows that carry no `cast` block; a row with one is drawn
/// from its data by `accents::emit_cast`. Only a new accepted action emits them; a miss
/// still casts, but it never manufactures an impact on another character.
fn emit_skill_cast_particles(
    game: Option<Res<crate::net::GameStateSnapshot>>,
    profiles: Option<Res<crate::skill_presentation::SkillPresentation>>,
    actors: Query<(
        Entity,
        &Transform,
        &InheritedVisibility,
        &crate::net::PlayerCosmeticAction,
        &crate::net::PlayerActionFacing,
        &crate::net::NetworkHeroClass,
        Option<&crate::net::PlayerLoadout>,
        &crate::combat::CombatStats,
    )>,
    mut receipts: Local<(Option<(u64, u64)>, std::collections::HashMap<Entity, u64>)>,
    mut output: MessageWriter<FlightParticles>,
    mut audio: MessageWriter<crate::game_audio::AudioCueRequest>,
) {
    let (Some(game), Some(profiles)) = (game, profiles) else {
        return;
    };
    let round = Some((game.meta.server_epoch, game.meta.match_id));
    if receipts.0 != round {
        receipts.0 = round;
        receipts.1.clear();
    }
    receipts.1.retain(|entity, _| actors.contains(*entity));
    for (entity, pose, visibility, action, facing, class, loadout, stats) in &actors {
        let previous = receipts.1.get(&entity).copied();
        receipts
            .1
            .insert(entity, previous.unwrap_or(0).max(action.sequence));
        if !cast_is_new(previous, action.sequence)
            || !visibility.get()
            || !stats.is_alive()
            || !matches!(game.state, crate::net::GameState::Running)
        {
            continue;
        }
        let state = loadout.and_then(|l| l.0.as_ref());
        let themed = profiles.themed_cast(class.0, state, action.slot);
        // The accent points along the accepted action, and it is anchored at the hero's
        // simulation position in both render modes.
        let direction =
            accents::CastContext::aim(action_yaw(action, facing), pose.forward().as_vec3());
        let heading = direction.to_angle();
        if action.slot == shared::BASIC_ATTACK_ACTION_SLOT
            && crate::equipped_skills::resolve(class.0, loadout)
                .and_then(|skills| skills.resolved())
                .is_some_and(|skills| {
                    skills.attack_profile() == shared::loadout::AttackProfileId::Melee
                })
        {
            if !themed {
                let forward = Vec3::new(direction.x, 0.0, direction.y);
                output.write(FlightParticles(vec![Particle {
                    event_id: action.sequence,
                    origin: pose.translation + Vec3::Y * 0.8 + forward * 0.65,
                    velocity: Vec3::ZERO,
                    age: 0.0,
                    lifetime: 0.2,
                    size: 0.8,
                    angle: heading,
                    color: Color::srgb(0.75, 0.95, 1.0),
                    shape: Shape::Slash,
                    ..Particle::BASE
                }]));
            }
            continue;
        }
        let Some(profile) = profiles.action_profile(class.0, state, action.slot) else {
            continue;
        };
        // Long windups already have a server-owned warning; avoid implying immediate release.
        if profile.windup.is_some() {
            continue;
        }
        // A Bluff row that names its cast voice is voiced from that data by the audio
        // layer. A row without one keeps the cue it always had.
        if profile
            .sound
            .as_ref()
            .is_none_or(|sound| sound.cast.is_none())
            && crate::skill_presentation::equipped_skill(class.0, state, action.slot)
                == Some(shared::loadout::SkillId::DaggerBluff)
        {
            audio.write(crate::game_audio::AudioCueRequest(
                crate::game_audio::AudioCue::Bluff,
            ));
        }
        let origin = pose.translation + Vec3::Y * 0.8;
        use crate::skill_presentation::EffectStyle as S;
        // A row with a `cast` block is drawn from its data, never twice.
        let Some(effect) = profile.effect.filter(|_| !themed) else {
            continue;
        };
        let (shape, count, size) = match effect {
            S::Slash => (Shape::Slash, 5, 1.6),
            S::Needle | S::Lance | S::Shock | S::Repeater => (Shape::Streak, 4, 1.1),
            S::Aegis | S::Pulse | S::Field | S::Wall => (Shape::Ringlet, 7, 1.4),
            _ => (Shape::Glow, 7, 1.0),
        };
        let color = Color::srgb_from_array(profile.color);
        output.write(FlightParticles(
            (0..count)
                .map(|i| {
                    let angle = i as f32 * std::f32::consts::TAU / count as f32;
                    Particle {
                        event_id: action.sequence,
                        origin,
                        velocity: if i == 0 {
                            Vec3::ZERO
                        } else {
                            Vec3::new(angle.cos(), 0.3, angle.sin()) * 1.8
                        },
                        age: 0.0,
                        lifetime: if i == 0 { 0.4 } else { 0.28 },
                        size: if i == 0 { size } else { 0.24 },
                        angle: heading,
                        color,
                        shape: if i == 0 { shape } else { Shape::Glow },
                        ..Particle::BASE
                    }
                })
                .collect(),
        ));
    }
}
fn cast_is_new(previous: Option<u64>, current: u64) -> bool {
    previous.is_some_and(|previous| current > previous)
}

/// The glow puff and the two orbiting glows a projectile leaves behind in one tick. Only
/// a thrown `shape` body of a magic style leaves them: a form carries its own wake, and a
/// wave or a melee contact is no missile in either render mode. `forms` says whether the
/// projectile renderer draws forms (Models3d with the shared meshes).
fn flight_puffs(
    at: Vec3,
    projectile: &crate::net::NetworkProjectile,
    profile: Option<&crate::combat_visuals::CombatVisualProfile>,
    class: Option<shared::HeroClass>,
    forms: bool,
    flat: bool,
    now: f64,
) -> Vec<Particle> {
    let puffs = match profile {
        Some(profile) => profile.puffs(
            profile.flight_body(
                forms,
                crate::combat_visuals::known_basic(class, projectile.action_slot),
            ),
            projectile.style,
        ),
        None => matches!(
            projectile.style,
            ProjectileStyle::Arcane | ProjectileStyle::Holy
        ),
    };
    if !puffs {
        return Vec::new();
    }
    let color = profile
        .map_or(Color::srgb(0.65, 0.8, 1.0), |p| p.color())
        .with_alpha(0.85);
    let scale = profile.map_or(1.0, |p| p.scale).clamp(0.2, 2.0);
    let direction = projectile.direction.normalize_or_zero();
    let base = Particle {
        event_id: 0,
        origin: at,
        velocity: -direction * 1.0,
        age: 0.0,
        lifetime: 0.26,
        size: 1.5 * scale,
        angle: 0.0,
        color,
        shape: Shape::Glow,
        ..Particle::BASE
    };
    let phase = now as f32 * 9.0 + projectile.id as f32 % 100.0;
    let orbit = [phase, phase + std::f32::consts::PI].map(|angle| Particle {
        origin: at
            + if flat {
                Vec3::new(angle.cos() * 0.50, 0.0, angle.sin() * 0.50)
            } else {
                Vec3::new(angle.cos() * 0.50, angle.sin() * 0.42, 0.0)
            },
        size: 0.45,
        lifetime: 0.16,
        color: Color::srgba(0.90, 0.85, 1.0, 0.9),
        ..base.clone()
    });
    std::iter::once(base).chain(orbit).collect()
}

/// Short tails sample authoritative positions; no stationary projectile invents a hit.
fn emit_projectile_particles(
    vfx_clock: Res<crate::vfx_clock::VfxClock>,
    mode: Res<PlayerVisualMode>,
    game: Option<Res<crate::net::GameStateSnapshot>>,
    cameras: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    registry: Option<Res<crate::combat_visuals::CombatVisualRegistry>>,
    library: Option<Res<crate::skill_presentation::bodies::VfxMeshes>>,
    owners: Query<(
        &crate::net::NetworkPlayerId,
        Option<&crate::net::NetworkHeroClass>,
        Option<&crate::net::NetworkAvatar>,
        Option<&crate::net::NetworkSpriteCharacter>,
    )>,
    projectiles: Query<(
        &Transform,
        &crate::net::NetworkProjectile,
        &InheritedVisibility,
    )>,
    mut clock: Local<(Option<(u64, u64)>, f32)>,
    mut output: MessageWriter<FlightParticles>,
) {
    if game
        .as_ref()
        .is_some_and(|g| !matches!(g.state, crate::net::GameState::Running))
    {
        return;
    }
    let identity = game
        .as_ref()
        .map(|g| (g.meta.server_epoch, g.meta.match_id));
    if clock.0 != identity {
        clock.0 = identity;
        clock.1 = 0.0;
    }
    // Paced by the presentation clock: a paused or slowed simulation must not
    // pile trail particles onto a projectile that has not moved.
    clock.1 += vfx_clock.delta.min(0.1);
    if clock.1 < 0.045 {
        return;
    }
    clock.1 = 0.0;
    let Ok((camera, pose)) = cameras.single() else {
        return;
    };
    let Some(viewport) = camera.logical_viewport_size() else {
        return;
    };
    let flat = *mode == PlayerVisualMode::Sprite2d;
    let mut particles = Vec::new();
    for (transform, projectile, inherited) in &projectiles {
        if !inherited.get() || !transform.translation.is_finite() {
            continue;
        }
        let p = transform.translation;
        let render = if flat {
            simulation_xz_to_render_xy(p).extend(layer::VFX)
        } else {
            p
        };
        if !camera.world_to_viewport(pose, render).is_ok_and(|p| {
            p.x >= -10.0 && p.y >= -10.0 && p.x <= viewport.x + 10.0 && p.y <= viewport.y + 10.0
        }) {
            continue;
        }
        let owner = matches!(
            projectile.source_kind,
            shared::combat::CombatEntityKind::Player | shared::combat::CombatEntityKind::Unknown
        )
        .then(|| owners.iter().find(|(id, ..)| id.0 == projectile.owner_id))
        .flatten();
        let class = owner.and_then(|(_, c, _, _)| c.map(|c| c.0));
        let profile = registry.as_ref().map(|r| {
            r.resolve(
                class,
                projectile.style,
                projectile.action_slot,
                owner.and_then(|(_, _, a, _)| a.and_then(|a| a.0.as_deref())),
                owner.and_then(|(_, _, _, s)| s.and_then(|s| s.0.as_deref())),
            )
        });
        particles.extend(flight_puffs(
            p,
            projectile,
            profile,
            class,
            !flat && library.is_some(),
            flat,
            vfx_clock.now,
        ));
        if particles.len() >= 45 {
            break;
        }
    }
    if !particles.is_empty() {
        output.write(FlightParticles(particles));
    }
}

fn pickup_available(game: Option<&crate::net::GameStateSnapshot>, id: u64) -> bool {
    game.is_some_and(|g| {
        matches!(g.state, crate::net::GameState::Running)
            && g.forest_pickups.iter().any(|p| p.id == id && p.available)
    })
}
fn pickup_anchor(game: Option<&crate::net::GameStateSnapshot>, id: u64, fallback: Vec3) -> Vec3 {
    game.and_then(|g| g.forest_pickups.iter().find(|p| p.id == id))
        .filter(|p| p.position.iter().all(|v| v.is_finite()))
        .map_or(fallback, |p| {
            Vec3::new(p.position[0], fallback.y, p.position[1])
        })
}
fn animate_butterfly_glows(
    mut commands: Commands,
    game: Option<Res<crate::net::GameStateSnapshot>>,
    time: Res<Time>,
    mode: Res<PlayerVisualMode>,
    assets: Res<VfxAssets>,
    cameras: Query<(&Camera, &GlobalTransform), (With<MainCamera>, Without<ButterflyGlow>)>,
    mut glows: Query<(
        Entity,
        &ButterflyGlow,
        &mut Transform,
        &mut GlobalTransform,
        &mut Visibility,
        &mut InheritedVisibility,
    )>,
) {
    let flat = *mode == PlayerVisualMode::Sprite2d;
    let camera = cameras.single().ok();
    for (entity, glow, mut pose, mut global, mut visible, mut inherited) in &mut glows {
        if mode.is_changed() {
            if flat {
                commands
                    .entity(entity)
                    .remove::<(Mesh3d, MeshMaterial3d<StandardMaterial>)>()
                    .insert((
                        Mesh2d(assets.mesh(Shape::Glow)),
                        MeshMaterial2d(glow.flat.clone()),
                    ));
            } else {
                commands
                    .entity(entity)
                    .remove::<(Mesh2d, MeshMaterial2d<ColorMaterial>)>()
                    .insert((
                        Mesh3d(assets.mesh(Shape::Glow)),
                        MeshMaterial3d(glow.material.clone()),
                    ));
            }
        }
        let anchor = pickup_anchor(game.as_deref(), glow.pickup_id, glow.anchor);
        let point = butterfly_position(anchor, glow.phase, time.elapsed_secs_f64());
        let position = if flat {
            simulation_xz_to_render_xy(point).extend(layer::VFX - 1.2)
        } else {
            point - Vec3::Y * 0.04
        };
        let on_screen = camera.is_some_and(|(cam, transform)| {
            cam.world_to_viewport(transform, position).is_ok_and(|p| {
                cam.logical_viewport_size().is_some_and(|s| {
                    p.x >= -40.0 && p.y >= -40.0 && p.x <= s.x + 40.0 && p.y <= s.y + 40.0
                })
            })
        });
        let show = pickup_available(game.as_deref(), glow.pickup_id) && on_screen;
        *visible = if show {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        *inherited = if show {
            InheritedVisibility::VISIBLE
        } else {
            InheritedVisibility::HIDDEN
        };
        let facing = if flat {
            Quat::IDENTITY
        } else {
            camera.map_or(Quat::IDENTITY, |(_, p)| p.to_scale_rotation_translation().1)
        };
        *pose = Transform::from_translation(position)
            .with_rotation(facing)
            .with_scale(Vec3::splat(
                2.0 + 0.20 * (time.elapsed_secs() * 3.0 + glow.phase).sin(),
            ));
        *global = GlobalTransform::from(*pose);
    }
}
fn pickup_feedback(
    game: Option<Res<crate::net::GameStateSnapshot>>,
    mut receipts: ResMut<PickupReceipts>,
    actors: Query<(
        &crate::net::NetworkPlayerId,
        &Transform,
        Option<&InheritedVisibility>,
        &crate::combat::CombatStats,
    )>,
    mut bursts: MessageWriter<ImpactBurst>,
    mut audio: MessageWriter<crate::game_audio::AudioCueRequest>,
    mut feedback: Option<ResMut<crate::combat::ActionFeedback>>,
) {
    let Some(game) = game.filter(|g| matches!(g.state, crate::net::GameState::Running)) else {
        *receipts = PickupReceipts::default();
        return;
    };
    let identity = (game.meta.server_epoch, game.meta.match_id);
    let fresh = receipts.identity != Some(identity);
    if fresh {
        receipts.identity = Some(identity);
        receipts.sequences.clear();
    }
    for pickup in game
        .forest_pickups
        .iter()
        .filter(|p| (1..=shared::forest_pickups::FOREST_PICKUP_COUNT as u64).contains(&p.id))
    {
        let previous = receipts.sequences.get(&pickup.id).copied();
        receipts.sequences.insert(
            pickup.id,
            previous.unwrap_or(0).max(pickup.collection_sequence),
        );
        if fresh
            || previous.is_none_or(|old| pickup.collection_sequence <= old)
            || !pickup.healed_amount.is_finite()
            || pickup.healed_amount <= 0.0
        {
            continue;
        }
        let Some((_, pose, visibility, stats)) = actors
            .iter()
            .find(|(id, _, _, _)| Some(id.0) == pickup.last_collector_id)
        else {
            continue;
        };
        if !stats.is_alive() || visibility.is_some_and(|v| !v.get()) {
            continue;
        }
        bursts.write(ImpactBurst {
            position: pose.translation,
            direction: Vec2::Y,
            color: Color::srgb(0.35, 1.0, 0.65),
            scale: 1.3,
            lifetime: 0.65,
            kind: BurstKind::Magic,
            seed: u64::MAX - pickup.id,
        });
        if pickup.last_collector_id == Some(game.your_id) {
            audio.write(crate::game_audio::AudioCueRequest(
                crate::game_audio::AudioCue::Butterfly,
            ));
            if let Some(feedback) = feedback.as_deref_mut() {
                feedback.push_line(crate::i18n::trf(
                    "combat.feedback.butterfly",
                    &[("hp", &format!("{:.0}", pickup.healed_amount))],
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cast_receipts_do_not_replay_initial_duplicate_or_older_actions() {
        assert!(!cast_is_new(None, 12));
        assert!(!cast_is_new(Some(12), 12));
        assert!(!cast_is_new(Some(12), 11));
        assert!(cast_is_new(Some(12), 13));
    }
    #[test]
    fn pickup_receipts_seed_once_ignore_rollbacks_and_respect_hidden_collectors() {
        use crate::net::{GameState, GameStateSnapshot, NetworkPlayerId};
        let mut app = App::new();
        app.init_resource::<PickupReceipts>()
            .add_message::<ImpactBurst>()
            .add_message::<crate::game_audio::AudioCueRequest>()
            .add_systems(Update, pickup_feedback);
        let actor = app
            .world_mut()
            .spawn((
                NetworkPlayerId(7),
                Transform::default(),
                crate::combat::CombatStats::default(),
                InheritedVisibility::VISIBLE,
            ))
            .id();
        let mut game = GameStateSnapshot {
            state: GameState::Running,
            your_id: 7,
            ..Default::default()
        };
        game.forest_pickups
            .push(shared::forest_pickups::ForestPickupState {
                id: 1,
                position: [-66.0, 45.0],
                available: false,
                collection_sequence: 8,
                last_collector_id: Some(7),
                healed_amount: 40.0,
            });
        app.insert_resource(game);
        app.update(); // Connecting after a collection must not replay it.
        assert_eq!(app.world().resource::<Messages<ImpactBurst>>().len(), 0);
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .forest_pickups[0]
            .collection_sequence = 9;
        app.update();
        assert_eq!(app.world().resource::<Messages<ImpactBurst>>().len(), 1);
        assert_eq!(
            app.world()
                .resource::<Messages<crate::game_audio::AudioCueRequest>>()
                .len(),
            1
        );
        app.world_mut()
            .resource_mut::<Messages<crate::game_audio::AudioCueRequest>>()
            .clear();
        app.world_mut()
            .resource_mut::<Messages<ImpactBurst>>()
            .clear();
        for sequence in [9, 8, 9] {
            app.world_mut()
                .resource_mut::<GameStateSnapshot>()
                .forest_pickups[0]
                .collection_sequence = sequence;
            app.update();
            assert_eq!(app.world().resource::<Messages<ImpactBurst>>().len(), 0);
            assert_eq!(
                app.world()
                    .resource::<Messages<crate::game_audio::AudioCueRequest>>()
                    .len(),
                0
            );
        }
        app.world_mut()
            .entity_mut(actor)
            .insert(InheritedVisibility::HIDDEN);
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .forest_pickups[0]
            .collection_sequence = 10;
        app.update();
        assert_eq!(app.world().resource::<Messages<ImpactBurst>>().len(), 0);
        app.world_mut()
            .entity_mut(actor)
            .insert(InheritedVisibility::VISIBLE);
        app.update(); // Becoming visible cannot replay a hidden receipt.
        assert_eq!(app.world().resource::<Messages<ImpactBurst>>().len(), 0);
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .meta
            .match_id += 1;
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .forest_pickups[0]
            .collection_sequence = 11;
        app.update();
        assert_eq!(app.world().resource::<Messages<ImpactBurst>>().len(), 0);
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Lobby;
        app.update();
        assert!(
            app.world()
                .resource::<PickupReceipts>()
                .sequences
                .is_empty()
        );
    }
    #[test]
    fn pickup_visibility_follows_authority_and_drift_stays_collectible() {
        let mut game = crate::net::GameStateSnapshot {
            state: crate::net::GameState::Running,
            ..Default::default()
        };
        game.forest_pickups
            .push(shared::forest_pickups::ForestPickupState {
                id: 1,
                position: [-66.0, 45.0],
                available: true,
                collection_sequence: 0,
                last_collector_id: None,
                healed_amount: 0.0,
            });
        assert!(pickup_available(Some(&game), 1));
        assert!(!pickup_available(Some(&game), 2));
        assert!(!pickup_available(None, 1));
        let anchor = pickup_anchor(Some(&game), 1, Vec3::Y);
        assert_eq!(anchor, Vec3::new(-66.0, 1.0, 45.0));
        for frame in 0..100 {
            let point = butterfly_position(anchor, 0.7, frame as f64 * 0.1);
            assert!(point.xz().distance(anchor.xz()) < shared::forest_pickups::PICKUP_RADIUS);
            assert!(point.y > anchor.y + 1.0);
        }
        game.forest_pickups[0].available = false;
        assert!(!pickup_available(Some(&game), 1));
        game.forest_pickups[0].available = true;
        game.state = crate::net::GameState::Lobby;
        assert!(!pickup_available(Some(&game), 1));
    }
    fn test_burst() -> ImpactBurst {
        ImpactBurst {
            position: Vec3::ZERO,
            direction: Vec2::X,
            color: Color::WHITE,
            scale: 1.,
            lifetime: 1.,
            kind: BurstKind::Magic,
            seed: 1,
        }
    }
    #[test]
    fn fading_particles_keep_hdr_energy_without_changing_flat_color_or_alpha() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(16),
            ))
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<Image>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<Assets<ColorMaterial>>()
            .init_resource::<PlayerVisualMode>()
            .init_resource::<MapLayout>()
            .add_plugins(GameVfxPlugin);
        app.update();
        let color = Color::srgba(1.0, 0.5, 0.25, 0.4);
        for mode in [PlayerVisualMode::Models3d, PlayerVisualMode::Sprite2d] {
            app.world_mut().insert_resource(mode);
            app.world_mut()
                .write_message(FlightParticles(vec![Particle {
                    event_id: 0,
                    origin: Vec3::ZERO,
                    velocity: Vec3::ZERO,
                    age: 0.0,
                    lifetime: 10.0,
                    size: 1.0,
                    angle: 0.0,
                    color,
                    shape: Shape::Glow,
                    ..Particle::BASE
                }]));
            app.update();
            for mut slot in app
                .world_mut()
                .query::<&mut ParticleSlot>()
                .iter_mut(app.world_mut())
            {
                if let Some(particle) = &mut slot.active {
                    particle.age = 5.0;
                }
            }
            app.update();
            let (material, flat, age, is_3d, is_2d) = app
                .world_mut()
                .query::<(&ParticleSlot, Option<&Mesh3d>, Option<&Mesh2d>)>()
                .iter(app.world())
                .find_map(|(slot, mesh_3d, mesh_2d)| {
                    slot.active.as_ref().map(|particle| {
                        (
                            slot.material.clone(),
                            slot.flat.clone(),
                            particle.age,
                            mesh_3d.is_some(),
                            mesh_2d.is_some(),
                        )
                    })
                })
                .expect("The particle remains alive halfway through its fade");
            assert_eq!(is_3d, mode == PlayerVisualMode::Models3d);
            assert_eq!(is_2d, mode == PlayerVisualMode::Sprite2d);
            let expected_alpha = color.alpha() * (1.0 - age / 10.0);
            assert!((0.19..0.21).contains(&expected_alpha));
            let material = app
                .world()
                .resource::<Assets<StandardMaterial>>()
                .get(&material)
                .unwrap();
            let hdr = material.base_color.to_linear();
            assert!((hdr.red - 2.5).abs() < 0.0001);
            assert!((hdr.green - color.to_linear().green * 2.5).abs() < 0.0001);
            assert!((hdr.alpha - expected_alpha).abs() < 0.0001);
            assert!(!material.fog_enabled);
            let flat = app
                .world()
                .resource::<Assets<ColorMaterial>>()
                .get(&flat)
                .unwrap();
            assert_eq!(flat.color, color.with_alpha(expected_alpha));
        }
    }
    /// Budget of shared meshes: the particle pool and the skill bodies draw the nine flat
    /// silhouettes from one asset each.
    #[test]
    fn the_pool_shares_its_flat_meshes_with_the_body_library() {
        use crate::skill_presentation::bodies::{self, PartMesh, VfxMeshes};
        use crate::skill_presentation::vocab::Silhouette;
        let app = |library: bool| {
            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .init_resource::<Assets<Mesh>>()
                .init_resource::<Assets<Image>>()
                .init_resource::<Assets<StandardMaterial>>()
                .init_resource::<Assets<ColorMaterial>>()
                .init_resource::<PlayerVisualMode>()
                .init_resource::<MapLayout>()
                .add_plugins(GameVfxPlugin);
            if library {
                app.add_systems(Startup, bodies::setup_meshes);
            }
            app.update();
            app
        };
        // On its own the pool builds its thirteen shapes and the butterfly wing.
        assert_eq!(app(false).world().resource::<Assets<Mesh>>().len(), 14);
        let app = app(true);
        // Nineteen meshes of the library, four shapes only a particle has, and the wing.
        assert_eq!(app.world().resource::<Assets<Mesh>>().len(), 19 + 4 + 1);
        let library = app.world().resource::<VfxMeshes>();
        let pool = app.world().resource::<VfxAssets>();
        for (shape, silhouette) in [
            (Shape::Kite, Silhouette::Kite),
            (Shape::Star, Silhouette::Star),
            (Shape::Chevron, Silhouette::Chevron),
            (Shape::Diamond, Silhouette::Diamond),
            (Shape::Arc, Silhouette::Arc),
            (Shape::Drop, Silhouette::Drop),
            (Shape::Cross, Silhouette::Cross),
            (Shape::Crescent, Silhouette::Crescent),
            (Shape::Claw, Silhouette::Claw),
        ] {
            assert_eq!(
                pool.mesh(shape),
                library.handle(PartMesh::Silhouette(silhouette)),
                "{shape:?}"
            );
            assert_eq!(library.particle(shape), Some(pool.mesh(shape)));
        }
        for shape in [Shape::Glow, Shape::Ringlet, Shape::Slash, Shape::Streak] {
            assert_eq!(library.particle(shape), None);
        }
    }
    #[test]
    fn pool_saturates_resets_and_reuses_entities_across_render_modes() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<Image>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<Assets<ColorMaterial>>()
            .init_resource::<PlayerVisualMode>()
            .init_resource::<MapLayout>()
            .add_plugins(GameVfxPlugin);
        app.update();
        let count = app.world().entities().count_spawned();
        for _ in 0..96 {
            app.world_mut().write_message(test_burst());
        }
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&ParticleSlot>()
                .iter(app.world())
                .filter(|s| s.active.is_some())
                .count(),
            PARTICLE_BUDGET
        );
        assert_eq!(app.world().entities().count_spawned(), count);
        app.world_mut().write_message(ClearCombatVfx);
        app.update();
        assert!(
            app.world_mut()
                .query::<(&ParticleSlot, &Visibility)>()
                .iter(app.world())
                .all(|(s, v)| s.active.is_none() && *v == Visibility::Hidden)
        );
        app.world_mut().insert_resource(PlayerVisualMode::Sprite2d);
        app.world_mut().write_message(test_burst());
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&ParticleSlot>()
                .iter(app.world())
                .filter(|s| s.active.is_some())
                .count(),
            12
        );
        assert_eq!(
            app.world_mut()
                .query_filtered::<&Mesh2d, With<ButterflyWing>>()
                .iter(app.world())
                .count(),
            BUTTERFLY_BUDGET * 2
        );
        assert_eq!(app.world().entities().count_spawned(), count);
        // Expired particles free slots instead of growing the entity set.
        for mut slot in app
            .world_mut()
            .query::<&mut ParticleSlot>()
            .iter_mut(app.world_mut())
        {
            if let Some(p) = &mut slot.active {
                p.age = 10.;
            }
        }
        app.update();
        assert!(
            app.world_mut()
                .query::<&ParticleSlot>()
                .iter(app.world())
                .all(|s| s.active.is_none())
        );
    }
    #[test]
    fn sustained_trails_leave_reserved_capacity_and_never_replay_dropped_bursts() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<Image>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<Assets<ColorMaterial>>()
            .init_resource::<PlayerVisualMode>()
            .init_resource::<MapLayout>()
            .add_plugins(GameVfxPlugin);
        app.update();
        let mut trail = burst_particles(&test_burst())[1].clone();
        trail.event_id = 0;
        trail.lifetime = 100.0;
        for _ in 0..10 {
            app.world_mut()
                .write_message(FlightParticles(vec![trail.clone(); 100]));
            app.update();
        }
        let count = app
            .world_mut()
            .query::<&ParticleSlot>()
            .iter(app.world())
            .filter(|s| s.active.as_ref().is_some_and(|p| p.event_id == 0))
            .count();
        assert_eq!(count, PARTICLE_BUDGET / 2);
        for _ in 0..200 {
            app.world_mut().write_message(test_burst());
        }
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&ParticleSlot>()
                .iter(app.world())
                .filter(|s| s.active.as_ref().is_some_and(|p| p.event_id == 1))
                .count(),
            PARTICLE_BUDGET / 2
        );
        app.world_mut().write_message(ClearCombatVfx);
        app.update();
        app.update();
        assert!(
            app.world_mut()
                .query::<&ParticleSlot>()
                .iter(app.world())
                .all(|s| s.active.is_none())
        );
    }
    #[test]
    fn particles_age_on_the_presentation_clock_under_pause_step_and_slow_motion() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(16),
            ))
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<Image>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<Assets<ColorMaterial>>()
            .init_resource::<PlayerVisualMode>()
            .init_resource::<MapLayout>()
            .add_plugins(GameVfxPlugin);
        let mut paused = shared::sandbox::SandboxSnapshot {
            config: Default::default(),
            ack: None,
            last_request_id: 0,
            actors: Vec::new(),
            analytics: Default::default(),
            simulation_secs: 3.0,
            frame: 0,
        };
        paused.config.environment.paused = true;
        app.insert_resource(crate::net::GameStateSnapshot {
            meta: shared::protocol::SnapshotMeta::new(7, 1, 1),
            state: crate::net::GameState::Running,
            sandbox: Some(paused),
            ..Default::default()
        });
        app.update();
        let mut particle = burst_particles(&test_burst())[0].clone();
        particle.lifetime = 0.1;
        app.world_mut()
            .write_message(FlightParticles(vec![particle]));
        fn age(app: &mut App) -> Option<f32> {
            app.world_mut()
                .query::<&ParticleSlot>()
                .iter(app.world())
                .find_map(|slot| slot.active.as_ref().map(|particle| particle.age))
        }
        fn sandbox(app: &mut App) -> Mut<'_, shared::sandbox::SandboxSnapshot> {
            app.world_mut()
                .resource_mut::<crate::net::GameStateSnapshot>()
                .map_unchanged(|game| game.sandbox.as_mut().unwrap())
        }
        // Twenty frames are 0.32 s of wall time, three times the lifetime.
        for _ in 0..20 {
            app.update();
            assert_eq!(
                age(&mut app),
                Some(0.0),
                "a paused frame holds its particles"
            );
        }
        sandbox(&mut app).simulation_secs += 1.0 / 60.0;
        app.update();
        let stepped = age(&mut app).unwrap();
        assert!((stepped - 1.0 / 60.0).abs() < 1e-6);
        let mut environment = sandbox(&mut app);
        environment.config.environment.paused = false;
        environment.config.environment.time_scale = 0.25;
        app.update();
        assert!((age(&mut app).unwrap() - stepped - 0.016 * 0.25).abs() < 1e-6);
    }
    #[test]
    fn malformed_bursts_do_not_produce_particles() {
        let mut burst = test_burst();
        burst.direction.x = f32::NAN;
        assert!(burst_particles(&burst).is_empty());
        burst = test_burst();
        burst.position.y = f32::INFINITY;
        assert!(burst_particles(&burst).is_empty());
    }
    #[test]
    fn styles_dispatch_and_effect_lifetimes_are_bounded() {
        assert_eq!(
            BurstKind::for_style(ProjectileStyle::Arcane),
            BurstKind::Magic
        );
        assert_eq!(
            BurstKind::for_style(ProjectileStyle::Crescent),
            BurstKind::Melee
        );
        assert_eq!(
            BurstKind::for_style(ProjectileStyle::Arrow),
            BurstKind::Ranged
        );
        // A claw is a melee strike; it used to fall through to the ranged pop.
        assert_eq!(
            BurstKind::for_style(ProjectileStyle::Claw),
            BurstKind::Melee
        );
        for kind in [BurstKind::Melee, BurstKind::Magic, BurstKind::Ranged] {
            let burst = ImpactBurst {
                position: Vec3::ZERO,
                direction: Vec2::X,
                color: Color::WHITE,
                scale: 100.,
                lifetime: 100.,
                kind,
                seed: 42,
            };
            let particles = burst_particles(&burst);
            assert!(particles.len() <= 12);
            for p in particles {
                assert!(p.lifetime <= 1.7);
                for flat in [false, true] {
                    let pose = p.pose(flat, Quat::IDENTITY);
                    assert!(pose.translation.is_finite() && pose.scale.is_finite());
                }
            }
        }
        // The same bound holds for every authored impact kind and accent pattern, however
        // large a row asks them to be.
        use crate::skill_presentation::{accents, impacts, vocab};
        let palette = accents::Palette::of_class(&crate::skill_presentation::Theme {
            secondary: [0.5; 3],
            accent: [1.0; 3],
        });
        let bounded = |particles: Vec<ParticleSpec>, most: usize, secs: f32| {
            assert!(!particles.is_empty() && particles.len() <= most);
            for spec in particles {
                assert!(spec.is_sound() && spec.end_secs() <= secs);
            }
        };
        for kind in vocab::ImpactKind::ALL {
            let recipe = impacts::ImpactRecipe {
                kind: *kind,
                shape: None,
                count: Some(u8::MAX),
                scale: 100.,
                lifetime: 100.,
                slots: None,
            };
            for area_damage in [false, true] {
                let ctx = impacts::ImpactContext {
                    position: Vec3::ZERO,
                    ground: 0.,
                    direction: Vec2::X,
                    heading: None,
                    area_damage,
                    receipt: 42,
                    reserved: 0,
                };
                bounded(impacts::impact_particles(&recipe, &palette, &ctx), 12, 1.7);
            }
        }
        for pattern in vocab::AccentPattern::ALL {
            let accent = accents::CastAccent {
                count: Some(u8::MAX),
                scale: 100.,
                lifetime: 100.,
                ..accents::CastAccent::plain(*pattern)
            };
            let ctx = accents::CastContext {
                origin: Vec3::ZERO,
                direction: Vec2::X,
                recast: false,
                area: None,
                strike_to: Some(Vec3::X * 30.),
                sequence: 42,
            };
            let particles = accents::accent_particles(&accent, &palette, &ctx);
            if *pattern == vocab::AccentPattern::None {
                assert!(particles.is_empty());
            } else {
                bounded(particles, 8, 0.5);
            }
        }
    }
    fn pool() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(16),
            ))
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<Image>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<Assets<ColorMaterial>>()
            .init_resource::<PlayerVisualMode>()
            .init_resource::<MapLayout>()
            .add_plugins(GameVfxPlugin);
        app.update();
        app
    }
    fn live(app: &mut App, pick: impl Fn(&Particle) -> bool) -> usize {
        app.world_mut()
            .query::<&ParticleSlot>()
            .iter(app.world())
            .filter(|slot| slot.active.as_ref().is_some_and(&pick))
            .count()
    }
    #[test]
    fn skill_bursts_fill_the_decorative_half_in_order_and_receipts_keep_the_other() {
        let mut app = pool();
        let spec = |event_id: u64, sort_key: u32| ParticleSpec {
            event_id,
            sort_key,
            lifetime: 100.,
            source: ParticleSource::Accent,
            ..ParticleSpec::BASE
        };
        // A far hero's burst is written first, the local hero's second. One frame takes 48
        // decorative particles, the lowest sort key first, and drops the rest for good.
        app.world_mut()
            .write_message(SkillBurst(vec![spec(7, 5); 60]));
        app.world_mut()
            .write_message(SkillBurst(vec![spec(9, 0); 40]));
        app.update();
        assert_eq!(live(&mut app, |p| p.event_id == 9), 40);
        assert_eq!(live(&mut app, |p| p.event_id == 7), 8);
        app.update();
        assert_eq!(live(&mut app, |_| true), 48);
        // Skill bursts and trails share the intake and one half of the pool. Within a frame
        // the skill bursts go first, whatever was written first.
        let trail = Particle {
            lifetime: 100.,
            ..Particle::BASE
        };
        app.world_mut()
            .write_message(FlightParticles(vec![trail.clone(); 100]));
        app.world_mut()
            .write_message(SkillBurst(vec![spec(7, 5); 60]));
        app.update();
        assert_eq!(live(&mut app, |p| p.class == ParticleClass::Skill), 96);
        assert_eq!(live(&mut app, |p| p.class == ParticleClass::Trail), 0);
        app.world_mut()
            .write_message(FlightParticles(vec![trail; 100]));
        app.update();
        assert_eq!(live(&mut app, |p| p.class == ParticleClass::Trail), 32);
        assert_eq!(
            live(&mut app, |p| p.class != ParticleClass::Confirm),
            PARTICLE_BUDGET / 2
        );
        // A receipt is not decoration: it takes the reserved half however full the other
        // is, and a skill burst cannot take a reserved slot.
        let hit = ParticleSpec {
            source: ParticleSource::Impact,
            ..spec(3, 0)
        };
        app.world_mut()
            .write_message(SkillBurst(vec![spec(11, 0); 10]));
        app.world_mut()
            .write_message(ConfirmedBurst(vec![hit; 200]));
        app.update();
        assert_eq!(live(&mut app, |p| p.event_id == 3), PARTICLE_BUDGET / 2);
        assert_eq!(live(&mut app, |p| p.event_id == 11), 0);
        #[cfg(feature = "qa")]
        {
            let tags: Vec<_> = app
                .world_mut()
                .query::<&ParticleSlot>()
                .iter(app.world())
                .filter_map(ParticleSlot::source)
                .collect();
            let count = |tag| tags.iter().filter(|seen| **seen == tag).count();
            assert_eq!(
                count((ParticleClass::Confirm, ParticleSource::Impact)),
                PARTICLE_BUDGET / 2
            );
            assert_eq!(count((ParticleClass::Skill, ParticleSource::Accent)), 96);
            assert_eq!(count((ParticleClass::Trail, ParticleSource::Engine)), 32);
        }
    }
    #[test]
    fn flight_puffs_are_trails_and_legacy_accents_are_skill_particles() {
        let mut app = pool();
        app.world_mut().write_message(FlightParticles(vec![
            Particle {
                lifetime: 100.,
                ..Particle::BASE
            },
            Particle {
                event_id: 12,
                lifetime: 100.,
                ..Particle::BASE
            },
        ]));
        app.world_mut().write_message(test_burst());
        app.update();
        assert_eq!(
            live(&mut app, |p| p.event_id == 0
                && p.class == ParticleClass::Trail),
            1
        );
        assert_eq!(
            live(&mut app, |p| p.event_id == 12
                && p.class == ParticleClass::Skill),
            1
        );
        assert_eq!(live(&mut app, |p| p.class == ParticleClass::Confirm), 12);
    }
    #[test]
    fn a_relocation_the_choreography_painted_replaces_the_generic_dash() {
        use crate::skill_presentation::cast::MoveCause;
        let mut app = pool();
        let to = Vec3::X * 5.;
        let dash = |actor: u64| UtilityVfx::Dash {
            actor,
            from: Vec3::ZERO,
            to,
            seed: 70 + actor,
        };
        // Hero 7 was displaced by something else; hero 8 dashed on its own.
        app.world_mut().write_message(dash(7));
        app.world_mut().write_message(dash(8));
        app.world_mut().write_message(MoveObserved {
            actor_id: 7,
            from: Some(Vec3::ZERO),
            to: Some(to),
            cause: MoveCause::Forced,
            skill: None,
            seed: 501,
            local: false,
        });
        app.world_mut().write_message(MoveObserved {
            actor_id: 8,
            from: Some(Vec3::ZERO),
            to: Some(to),
            cause: MoveCause::UtilityDash,
            skill: None,
            seed: 502,
            local: false,
        });
        app.update();
        assert_eq!(live(&mut app, |p| p.event_id == 77), 0);
        assert_eq!(
            live(&mut app, |p| p.event_id == 501
                && p.source == ParticleSource::Move
                && p.class == ParticleClass::Skill),
            accents::drag_streak(Some(Vec3::ZERO), Some(to), 501).len()
        );
        let generic = dash_particles(Vec3::ZERO, to, 78).len();
        assert_eq!(live(&mut app, |p| p.event_id == 78), generic);
        assert_eq!(live(&mut app, |p| p.event_id == 502), 0);
        // The note lasts one frame: a later dash of the same hero is drawn again.
        app.world_mut().write_message(dash(7));
        app.update();
        assert_eq!(live(&mut app, |p| p.event_id == 77), generic);
    }
    /// The speed read follows two replicated facts and nothing else: the utility haste, or
    /// a movement multiplier above what the hero's slows alone leave.
    #[test]
    fn the_speed_read_shows_the_utility_haste_and_skill_speed_buffs() {
        use shared::loadout::LoadoutState;
        use shared::utility::UtilityState;
        let haste = UtilityState {
            haste_active_secs: 2.0,
            ..default()
        };
        let none = UtilityState::default();
        let moving = |movement: f32, slow: f32| LoadoutState {
            movement_multiplier: movement,
            slow_multiplier: slow,
            ..default()
        };
        // (utility, movement and slow multipliers, visible, alive) -> shown
        let table = [
            // No buff: a legacy hero, a modular hero at rest, and the 5 % margin.
            (none, None, true, true, false),
            (none, Some((1.0, 1.0)), true, true, false),
            (none, Some((1.05, 1.0)), true, true, false),
            (none, Some((1.06, 1.0)), true, true, true),
            // A skill or passive speed step (1.3, 1.4).
            (none, Some((1.3, 1.0)), true, true, true),
            (none, Some((1.4, 1.0)), true, true, true),
            // A slow without a buff, and a root or a stun (movement 0).
            (none, Some((0.6, 0.6)), true, true, false),
            (none, Some((0.0, 1.0)), true, true, false),
            (none, Some((0.0, 0.6)), true, true, false),
            (none, Some((0.0, 0.0)), true, true, false),
            // A buff under a slow still shows: 0.6 x 1.3.
            (none, Some((0.78, 0.6)), true, true, true),
            // The utility haste shows with and without replicated skill state.
            (haste, None, true, true, true),
            (haste, Some((1.0, 1.0)), true, true, true),
            (haste, Some((0.6, 0.6)), true, true, true),
            // Never on a hero the client does not see, nor on a dead one.
            (haste, None, false, true, false),
            (haste, None, true, false, false),
            (none, Some((1.4, 1.0)), false, true, false),
            (none, Some((1.4, 1.0)), true, false, false),
            // Values that are not numbers show nothing.
            (none, Some((f32::NAN, 1.0)), true, true, false),
            (none, Some((1.4, f32::NAN)), true, true, false),
        ];
        for (row, (utility, multipliers, visible, alive, shown)) in table.into_iter().enumerate() {
            let loadout = multipliers.map(|(movement, slow)| moving(movement, slow));
            assert_eq!(
                speed_read(&utility, loadout.as_ref(), visible, alive),
                shown,
                "row {row}"
            );
        }
        // The cause never matters: every other replicated field leaves the read alone.
        let kit = LoadoutState {
            recipe: Some(shared::loadout::CoreId::Wildspark.preset()),
            passive_remaining_secs: 6.0,
            passive_stacks: 5,
            energy: true,
            ..default()
        };
        assert!(!speed_read(&none, Some(&kit), true, true));

        // In the world: the buffed hero pulses at once and leaves streaks as it moves; the
        // slowed, the hidden and the dead one leave nothing.
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<UtilityVfx>()
            .add_systems(Update, emit_haste_trails);
        crate::vfx_clock::ensure(&mut app);
        let mut hero = |x: f32, loadout: LoadoutState, visible: bool, hp: f32| {
            app.world_mut()
                .spawn((
                    RemotePlayer,
                    Transform::from_xyz(x, 0.0, 0.0),
                    PlayerUtility::default(),
                    if visible {
                        InheritedVisibility::VISIBLE
                    } else {
                        InheritedVisibility::HIDDEN
                    },
                    crate::net::PlayerLoadout(Some(loadout)),
                    crate::combat::CombatStats { hp, ..default() },
                ))
                .id()
        };
        let buffed = hero(10.0, moving(1.3, 1.0), true, 50.0);
        hero(20.0, moving(0.6, 0.6), true, 50.0);
        hero(30.0, moving(1.3, 1.0), false, 50.0);
        hero(40.0, moving(1.3, 1.0), true, 0.0);
        app.update();
        let drain = |app: &mut App| -> Vec<UtilityVfx> {
            app.world_mut()
                .resource_mut::<Messages<UtilityVfx>>()
                .drain()
                .collect()
        };
        assert!(matches!(
            drain(&mut app)[..],
            [UtilityVfx::HastePulse { position, .. }] if position.x == 10.0
        ));
        app.world_mut()
            .get_mut::<Transform>(buffed)
            .unwrap()
            .translation
            .x += 1.0;
        app.update();
        let streaks = drain(&mut app);
        assert!(!streaks.is_empty());
        assert!(streaks.iter().all(|vfx| matches!(
            vfx,
            UtilityVfx::HasteStreak { position, direction, .. }
                if (10.0..=11.0).contains(&position.x) && *direction == Vec2::X
        )));
        // The buff ends with the replicated multiplier.
        app.world_mut()
            .get_mut::<crate::net::PlayerLoadout>(buffed)
            .unwrap()
            .0 = Some(moving(1.0, 1.0));
        app.world_mut()
            .get_mut::<Transform>(buffed)
            .unwrap()
            .translation
            .x += 1.0;
        app.update();
        assert!(drain(&mut app).is_empty());
    }
    #[test]
    fn legacy_accents_follow_the_accepted_yaw_at_the_simulation_position() {
        use crate::net::{NetworkHeroClass, PlayerActionFacing, PlayerCosmeticAction};
        use crate::skill_presentation::SkillPresentation;
        use shared::{HeroClass, PlayerActionKind};
        let at = Vec3::new(3., 0.5, -7.);
        let yaw = shared::math::hero_yaw_towards(0.6, 0.8);
        // The accents of one accepted action of `class` on `slot` under a registry.
        let accents_of = |registry: SkillPresentation, class: HeroClass, slot: u8| {
            let mut app = App::new();
            app.insert_resource(registry)
                .insert_resource(crate::net::GameStateSnapshot {
                    meta: shared::protocol::SnapshotMeta::new(7, 1, 1),
                    state: crate::net::GameState::Running,
                    ..Default::default()
                })
                .add_message::<FlightParticles>()
                .add_message::<crate::game_audio::AudioCueRequest>()
                .add_systems(Update, emit_skill_cast_particles);
            let hero = app
                .world_mut()
                .spawn((
                    // The model still looks the other way.
                    Transform::from_translation(at).looking_to(Vec3::new(-0.6, 0., -0.8), Vec3::Y),
                    InheritedVisibility::VISIBLE,
                    PlayerCosmeticAction::default(),
                    PlayerActionFacing::default(),
                    NetworkHeroClass(class),
                    crate::combat::CombatStats::default(),
                ))
                .id();
            app.update();
            app.world_mut().entity_mut(hero).insert((
                PlayerCosmeticAction {
                    sequence: 1,
                    kind: PlayerActionKind::Cast,
                    slot,
                },
                PlayerActionFacing {
                    sequence: 1,
                    yaw: Some(yaw),
                },
            ));
            app.update();
            app.world_mut()
                .resource_mut::<Messages<FlightParticles>>()
                .drain()
                .flat_map(|batch| batch.0)
                .collect::<Vec<_>>()
        };
        let accent = accents_of(SkillPresentation::unmigrated(), HeroClass::Warrior, 0);
        assert!(!accent.is_empty());
        for particle in &accent {
            // Simulation coordinates whatever the render mode: the system reads none.
            assert_eq!(particle.origin, at + Vec3::Y * 0.8);
            assert!(Vec2::from_angle(particle.angle).distance(Vec2::new(0.6, 0.8)) < 1e-5);
            assert_eq!(particle.event_id, 1);
        }
        // The melee swing of a basic attack is laid ahead along the same yaw.
        let swing = accents_of(
            SkillPresentation::unmigrated(),
            HeroClass::Stormfist,
            shared::BASIC_ATTACK_ACTION_SLOT,
        );
        assert_eq!(swing.len(), 1);
        assert!(
            swing[0]
                .origin
                .distance(at + Vec3::new(0.6, 0., 0.8) * 0.65 + Vec3::Y * 0.8)
                < 1e-5
        );
        assert!(Vec2::from_angle(swing[0].angle).distance(Vec2::new(0.6, 0.8)) < 1e-5);
        // A row with a `cast` block, and a basic attack with an accent, are drawn from
        // their data instead.
        assert!(accents_of(SkillPresentation::target(), HeroClass::Warrior, 0).is_empty());
        assert!(
            accents_of(
                SkillPresentation::target(),
                HeroClass::Stormfist,
                shared::BASIC_ATTACK_ACTION_SLOT,
            )
            .is_empty()
        );
    }
    #[test]
    fn the_bluff_cue_is_requested_only_while_its_row_names_no_cast_voice() {
        use crate::game_audio::{AudioCue, AudioCueRequest};
        use crate::net::{NetworkHeroClass, PlayerActionFacing, PlayerCosmeticAction};
        use crate::skill_presentation::SkillPresentation;
        use shared::{HeroClass, PlayerActionKind, loadout::SkillId};
        // The cue requests of one accepted action of an Adventurer on `slot`.
        let requests = |registry: SkillPresentation, slot: u8| {
            let mut app = App::new();
            app.insert_resource(registry)
                .insert_resource(crate::net::GameStateSnapshot {
                    meta: shared::protocol::SnapshotMeta::new(7, 1, 1),
                    state: crate::net::GameState::Running,
                    ..Default::default()
                })
                .add_message::<FlightParticles>()
                .add_message::<AudioCueRequest>()
                .add_systems(Update, emit_skill_cast_particles);
            let hero = app
                .world_mut()
                .spawn((
                    Transform::default(),
                    InheritedVisibility::VISIBLE,
                    PlayerCosmeticAction::default(),
                    PlayerActionFacing::default(),
                    NetworkHeroClass(HeroClass::Adventurer),
                    crate::combat::CombatStats::default(),
                ))
                .id();
            app.update();
            app.world_mut()
                .entity_mut(hero)
                .insert(PlayerCosmeticAction {
                    sequence: 1,
                    kind: PlayerActionKind::Cast,
                    slot,
                });
            app.update();
            app.world_mut()
                .resource_mut::<Messages<AudioCueRequest>>()
                .drain()
                .map(|request| request.0)
                .collect::<Vec<_>>()
        };
        let kit = shared::loadout::preset_for_class(HeroClass::Adventurer).unwrap();
        let bluff = kit
            .skills()
            .iter()
            .position(|skill| *skill == SkillId::DaggerBluff)
            .unwrap() as u8;
        // The unmigrated row names no voice, so Bluff keeps the cue it always had.
        assert_eq!(
            requests(SkillPresentation::unmigrated(), bluff),
            [AudioCue::Bluff]
        );
        // A row with `sound.cast` is voiced by the audio layer, once.
        assert!(requests(SkillPresentation::target(), bluff).is_empty());
        for slot in (0..4).filter(|slot| *slot != bluff) {
            assert!(requests(SkillPresentation::unmigrated(), slot).is_empty());
            assert!(requests(SkillPresentation::target(), slot).is_empty());
        }
    }
    /// The arcane style default draws the hit of a Mage the client cannot see. It strikes
    /// one unit, so its ring and its sparks stay at that unit like a recipe would.
    #[test]
    fn the_arcane_default_burst_stays_at_the_unit_it_struck() {
        use crate::combat_visuals::CombatVisualRegistry;
        use crate::skill_presentation::impacts::SINGLE_TARGET_REACH;
        let registry =
            CombatVisualRegistry::from_json(include_str!("../assets/config/combat_visuals.json"))
                .unwrap();
        let profile = registry.resolve_style(ProjectileStyle::Arcane);
        assert_eq!(profile.id, "mage_arcane");
        let burst = ImpactBurst {
            position: Vec3::new(3., 1., -2.),
            direction: Vec2::X,
            color: profile.impact.color(),
            scale: profile.impact.scale,
            lifetime: profile.impact.lifetime,
            kind: BurstKind::for_style(ProjectileStyle::Arcane),
            seed: 7,
        };
        let particles = burst_particles(&burst);
        assert_eq!(particles.len(), 12);
        let reach = particles
            .iter()
            .flat_map(|particle| {
                (0..=20).map(move |step| {
                    let mut live = particle.clone();
                    live.age = particle.lifetime * step as f32 / 20.;
                    let pose = live.pose(false, Quat::IDENTITY);
                    (pose.translation - burst.position).xz().length()
                        + unit_radius(live.shape) * pose.scale.x
                })
            })
            .fold(0., f32::max);
        assert!(reach > 1.0 && reach <= SINGLE_TARGET_REACH, "{reach}");
    }
    #[test]
    fn no_flight_puff_where_a_form_is_drawn() {
        use crate::combat_visuals::CombatVisualRegistry;
        use shared::HeroClass;
        let target = CombatVisualRegistry::from_json(include_str!(
            "skill_presentation/fixtures/target_combat_visuals.json"
        ))
        .unwrap();
        let embedded = CombatVisualRegistry::default();
        let at = Vec3::new(2., 0.85, -3.);
        // The particles one projectile of `class` leaves in a tick: where the renderer
        // draws forms, where it draws shapes in 3D, and in the flat backend.
        let puffs = |registry: &CombatVisualRegistry,
                     class: Option<HeroClass>,
                     style: ProjectileStyle,
                     slot: Option<u8>| {
            let projectile = crate::net::NetworkProjectile {
                id: 9,
                owner_id: 1,
                owner_team: crate::team::Team::Green,
                source_kind: shared::combat::CombatEntityKind::Player,
                style,
                action_slot: slot,
                direction: Vec3::X,
            };
            let profile = registry.resolve(class, style, slot, None, None);
            [(true, false), (false, false), (false, true)].map(|(forms, flat)| {
                flight_puffs(at, &projectile, Some(profile), class, forms, flat, 4.0)
            })
        };
        let basic = Some(shared::BASIC_ATTACK_ACTION_SLOT);
        // Every action of the final data that is drawn as a form or as a reach streak
        // leaves nothing behind in 3D.
        for class in HeroClass::ALL {
            for slot in [Some(0), Some(1), Some(2), Some(3), basic] {
                let style = ProjectileStyle::for_class(class);
                let profile = target.resolve(Some(class), style, slot, None, None);
                let own = crate::combat_visuals::known_basic(Some(class), slot);
                let drawn = profile.flight_body(true, own);
                let [forms, shapes, flat] = puffs(&target, Some(class), style, slot);
                if drawn != crate::combat_visuals::FlightBody::Shape {
                    assert!(forms.is_empty(), "{} {slot:?}", class.id());
                }
                // Where no form is drawn only a thrown body of a magic style puffs, in
                // the profile colour, with its two orbiting glows; never a wave or a
                // melee contact.
                let magic = matches!(style, ProjectileStyle::Arcane | ProjectileStyle::Holy)
                    && profile.presentation
                        == crate::skill_presentation::vocab::ProjectilePresentation::Projectile;
                for particles in [&shapes, &flat] {
                    assert_eq!(particles.len(), if magic { 3 } else { 0 }, "{}", profile.id);
                }
                if magic {
                    assert_eq!(shapes[0].color, profile.color().with_alpha(0.85));
                    assert_eq!(shapes[0].origin, at);
                    assert_eq!(shapes[0].velocity, Vec3::NEG_X);
                    assert_eq!(shapes[0].size, 1.5 * profile.scale.clamp(0.2, 2.0));
                    // The orbit is laid in the ground plane of the flat backend.
                    assert!(shapes[1].origin.z == at.z && flat[1].origin.y == at.y);
                }
            }
        }
        // The cases by name: the three Mage bodies and the Cleric's spark are forms, Smite
        // is a wave, the Warrior's basic is a contact; none of them puffs where it is drawn.
        for (class, style, slot) in [
            (HeroClass::Mage, ProjectileStyle::Arcane, Some(0)),
            (HeroClass::Mage, ProjectileStyle::Arcane, Some(2)),
            (HeroClass::Mage, ProjectileStyle::Arcane, Some(3)),
            (HeroClass::Mage, ProjectileStyle::Arcane, basic),
            (HeroClass::Cleric, ProjectileStyle::Holy, basic),
            (HeroClass::Cleric, ProjectileStyle::Holy, Some(0)),
            (HeroClass::Warrior, ProjectileStyle::Crescent, basic),
            (HeroClass::Wildspark, ProjectileStyle::Rocket, basic),
        ] {
            assert!(puffs(&target, Some(class), style, slot)[0].is_empty());
        }
        let [_, shapes, flat] = puffs(
            &target,
            Some(HeroClass::Cleric),
            ProjectileStyle::Holy,
            Some(0),
        );
        assert!(shapes.is_empty() && flat.is_empty());
        // The embedded profiles name no form: a magic bolt keeps its puff and its glows
        // in both backends, and an arrow or a blade no longer leaves an exhaust.
        for (class, style, count) in [
            (HeroClass::Mage, ProjectileStyle::Arcane, 3),
            (HeroClass::Cleric, ProjectileStyle::Holy, 3),
            (HeroClass::Ranger, ProjectileStyle::Arrow, 0),
            (HeroClass::Warrior, ProjectileStyle::Crescent, 0),
            (HeroClass::Warden, ProjectileStyle::Claw, 0),
            (HeroClass::Wildspark, ProjectileStyle::Bullet, 0),
        ] {
            for particles in puffs(&embedded, Some(class), style, Some(0)) {
                assert_eq!(particles.len(), count, "{}", class.id());
            }
        }
        // A projectile whose profile cannot be resolved keeps the rule of its style.
        let unresolved = |style| {
            let projectile = crate::net::NetworkProjectile {
                id: 9,
                owner_id: 1,
                owner_team: crate::team::Team::Green,
                source_kind: shared::combat::CombatEntityKind::Minion,
                style,
                action_slot: None,
                direction: Vec3::X,
            };
            flight_puffs(at, &projectile, None, None, true, false, 4.0).len()
        };
        assert_eq!(unresolved(ProjectileStyle::Holy), 3);
        assert_eq!(unresolved(ProjectileStyle::CasterBolt), 0);
    }
    #[test]
    fn delayed_particles_wait_hidden_and_malformed_ones_are_dropped() {
        let mut app = pool();
        let late = ParticleSpec {
            event_id: 5,
            delay: 0.1,
            lifetime: 1.,
            ..ParticleSpec::BASE
        };
        app.world_mut().write_message(SkillBurst(vec![
            late.clone(),
            ParticleSpec {
                lifetime: 0.,
                ..late.clone()
            },
            ParticleSpec {
                origin: Vec3::NAN,
                ..late.clone()
            },
            ParticleSpec {
                size: f32::INFINITY,
                ..late.clone()
            },
            // The pool never holds a slot hidden for longer than a quarter second.
            ParticleSpec {
                event_id: 6,
                delay: 10.,
                ..late
            },
        ]));
        app.update();
        assert_eq!(live(&mut app, |_| true), 2);
        assert_eq!(
            live(&mut app, |p| p.event_id == 6 && p.age >= -MAX_DELAY),
            1
        );
        let shown = |app: &mut App| {
            app.world_mut()
                .query::<(&ParticleSlot, &Visibility)>()
                .iter(app.world())
                .find(|(slot, _)| slot.active.as_ref().is_some_and(|p| p.event_id == 5))
                .map(|(_, visibility)| *visibility == Visibility::Visible)
        };
        assert_eq!(shown(&mut app), Some(false));
        for _ in 0..7 {
            app.update();
        }
        assert_eq!(shown(&mut app), Some(true));
    }
    #[test]
    fn generated_particles_follow_drag_gravity_curves_colours_and_orientation() {
        let spec = ParticleSpec {
            origin: Vec3::new(1., 2., 3.),
            velocity: Vec3::new(4., 3., 0.),
            lifetime: 1.,
            gravity: 6.,
            drag: 2.,
            ..ParticleSpec::BASE
        };
        // The closed form equals a fine step-by-step integration, with and without drag.
        for drag in [2., 0.] {
            let particle = Particle::from_spec(
                &ParticleSpec {
                    drag,
                    ..spec.clone()
                },
                ParticleClass::Skill,
            );
            let (mut position, mut velocity) = (Vec3::ZERO, spec.velocity);
            for _ in 0..20_000 {
                velocity += (-Vec3::Y * 6. - velocity * drag) * 5e-5;
                position += velocity * 5e-5;
            }
            assert!(particle.travel(1.).distance(position) < 2e-3, "drag {drag}");
        }
        // Reach: the farther end of the flight plus the particle's own half extent.
        let thrown = (1. - (-2f32).exp()) / 2. * 4.;
        assert!((spec.reach(spec.origin) - (thrown + 0.5)).abs() < 1e-5);
        let grown = ParticleSpec {
            shape: Shape::Ringlet,
            curve: Curve::Grow,
            size: 3.,
            velocity: Vec3::Y * 9.,
            ..ParticleSpec::BASE
        };
        assert!((grown.reach(Vec3::new(0., 5., 4.)) - (4. + 3.)).abs() < 1e-5);
        // Every curve stays within its stated peak, and reaches it.
        for curve in [
            Curve::Shrink,
            Curve::Grow,
            Curve::Pop,
            Curve::Hold,
            Curve::Stretch,
        ] {
            let most = (0..=100)
                .map(|i| curve.scale(i as f32 / 100.).max_element())
                .fold(0., f32::max);
            assert!((most - curve.peak()).abs() < 1e-5, "{curve:?}");
        }
        assert!(Curve::Pop.scale(0.).distance(Vec2::splat(0.4)) < 1e-6);
        assert_eq!(Curve::Pop.scale(0.25), Vec2::ONE);
        assert_eq!(Curve::Stretch.scale(1.), Vec2::new(1.5, 0.6));
        // Colour and gain blend toward the end colour; a particle without one keeps both.
        let blend = Particle::from_spec(
            &ParticleSpec {
                color: Tint {
                    color: Color::srgb(1., 0., 0.),
                    gain: 4.,
                },
                end_color: Some(Tint {
                    color: Color::srgb(0., 0., 1.),
                    gain: 1.,
                }),
                ..ParticleSpec::BASE
            },
            ParticleClass::Skill,
        );
        assert_eq!(blend.tint(0.).color, Color::srgb(1., 0., 0.));
        assert_eq!(blend.tint(1.).color, Color::srgb(0., 0., 1.));
        assert_eq!(blend.tint(0.5).gain, 2.5);
        assert_eq!(Particle::BASE.tint(0.7).gain, PARTICLE_HDR_GAIN);
        // A generated particle keeps its whole coverage for the first half of its life and
        // is gone at its end; a wire-style particle fades from its first frame.
        for (t, generated, wire) in [
            (0., 1., 1.),
            (0.25, 1., 0.75),
            (HOLD_SHARE, 1., 0.5),
            (0.75, 0.5, 0.25),
            (1., 0., 0.),
        ] {
            assert!((blend.opacity(t) - generated).abs() < 1e-6, "{t}");
            assert!((Particle::BASE.opacity(t) - wire).abs() < 1e-6, "{t}");
        }
        // The dense glow of a hit covers its core evenly, lets a fifth of what it covers
        // through, and falls evenly to nothing at its rim; half-way out it still covers
        // more than the soft glow does.
        assert_eq!(dense_alpha(0.), DENSE_COVER);
        assert_eq!(dense_alpha(DENSE_CORE), DENSE_COVER);
        assert_eq!(dense_alpha(1.), 0.);
        const { assert!(DENSE_COVER <= 0.8 && DENSE_CORE < 0.4) };
        let mut last = DENSE_COVER;
        for step in 0..=20 {
            let cover = dense_alpha(DENSE_CORE + (1. - DENSE_CORE) * step as f32 / 20.);
            assert!(cover <= last);
            last = cover;
        }
        assert!(dense_alpha(0.67) > 2. * (1. - 0.67_f32).powf(1.6));
        // A ground particle lies flat with its axis along the heading, and turns about the
        // vertical; a billboard keeps facing the camera.
        let facing = Quat::from_rotation_x(-1.);
        let heading = Vec3::new(0.6, 0., 0.8);
        let lying = ParticleSpec {
            angle: heading.z.atan2(heading.x),
            orient: Orient::Ground,
            curve: Curve::Hold,
            ..ParticleSpec::BASE
        };
        let pose = lying.pose_at(0.1, false, facing);
        assert!((pose.rotation * Vec3::X).distance(heading) < 1e-5);
        assert!((pose.rotation * Vec3::Z).distance(Vec3::Y) < 1e-5);
        let turning = ParticleSpec {
            spin: 5.,
            ..lying.clone()
        };
        let turned = turning.pose_at(0.1, false, facing).rotation * Vec3::X;
        assert!((turned.dot(heading) - 0.5f32.cos()).abs() < 1e-5 && turned.y.abs() < 1e-5);
        let billboard = ParticleSpec {
            orient: Orient::Billboard,
            ..lying.clone()
        };
        let pose = billboard.pose_at(0.1, false, facing);
        assert!((pose.rotation * Vec3::Z).distance(facing * Vec3::Z) < 1e-5);
        // In the flat view every particle is seen from above, turned to its heading.
        for spec in [&lying, &billboard] {
            let pose = spec.pose_at(0.1, true, facing);
            assert!((pose.rotation * Vec3::X).distance(Vec3::new(0.6, 0.8, 0.)) < 1e-5);
        }
        // A velocity particle points where it travels; at rest it keeps its heading.
        let rising = ParticleSpec {
            velocity: Vec3::Y,
            orient: Orient::Velocity,
            ..lying.clone()
        };
        let pose = rising.pose_at(0.1, false, Quat::IDENTITY);
        assert!((pose.rotation * Vec3::X).distance(Vec3::Y) < 1e-5);
        let flat = rising.pose_at(0.1, true, Quat::IDENTITY);
        assert!((flat.rotation * Vec3::X).distance(Vec3::new(0.6, 0.8, 0.)) < 1e-5);
        let falling = ParticleSpec {
            velocity: Vec3::X,
            gravity: 10.,
            orient: Orient::Velocity,
            ..lying.clone()
        };
        let nose = falling.pose_at(1., false, Quat::IDENTITY).rotation * Vec3::X;
        assert!(nose.distance(Vec3::new(1., -10., 0.).normalize()) < 1e-5);
        // A stretched particle lengthens along its axis only.
        let stretched = ParticleSpec {
            curve: Curve::Stretch,
            size: 2.,
            ..lying
        };
        assert_eq!(
            stretched
                .pose_at(stretched.lifetime, false, facing)
                .scale
                .truncate(),
            Vec2::new(3., 1.2)
        );
        assert!(spec.is_sound());
        assert!(
            !ParticleSpec {
                angle: f32::NAN,
                ..spec
            }
            .is_sound()
        );
    }
    #[test]
    fn every_particle_shape_has_a_small_symmetric_mesh_inside_its_unit_radius() {
        assert_eq!(Shape::ALL.len(), 13);
        for shape in Shape::ALL {
            let mesh = shape_mesh(*shape);
            let positions = mesh
                .attribute(Mesh::ATTRIBUTE_POSITION)
                .and_then(|values| values.as_float3())
                .unwrap();
            assert!(mesh.indices().unwrap().len() / 3 <= 200, "{shape:?}");
            // The two textured quads fade out before their corners; the rest is solid.
            let textured = matches!(shape, Shape::Glow | Shape::Streak);
            let extent = positions
                .iter()
                .map(|p| {
                    if textured {
                        p[0].abs()
                    } else {
                        Vec2::new(p[0], p[1]).length()
                    }
                })
                .fold(0., f32::max);
            assert!(
                (extent - unit_radius(*shape)).abs() < 5e-3,
                "{shape:?} reaches {extent}"
            );
            // A blade lies ahead of its origin and crosses its axis at its depth; every
            // other mesh surrounds its origin.
            let back = positions.iter().map(|p| p[0]).fold(f32::MAX, f32::min);
            if blade_depth(*shape) > 0. {
                let (near, far) = positions
                    .iter()
                    .filter(|p| p[1].abs() < 1e-4)
                    .fold((f32::MAX, f32::MIN), |(near, far), p| {
                        (near.min(p[0]), far.max(p[0]))
                    });
                assert!(back > -1e-4, "{shape:?} reaches behind its origin");
                assert!(
                    (0.5 * (near + far) - blade_depth(*shape)).abs() < 5e-3,
                    "{shape:?} crosses its axis between {near} and {far}"
                );
            } else {
                assert!(back < -0.2, "{shape:?} starts at {back}");
            }
            for p in positions {
                assert_eq!(p[2], 0., "{shape:?}");
                assert!(
                    positions
                        .iter()
                        .any(|q| (q[0] - p[0]).abs() < 1e-4 && (q[1] + p[1]).abs() < 1e-4),
                    "{shape:?} is not symmetric about its axis at {p:?}"
                );
            }
        }
    }
    #[test]
    fn butterfly_paths_stay_near_forest_and_are_continuous() {
        for i in 0..BUTTERFLY_BUDGET {
            for frame in 0..1000 {
                let t = frame as f64 / 60.;
                let p = butterfly_position(Vec3::ZERO, i as f32, t);
                assert!(p.is_finite() && p.y >= 1.05 && p.y <= 1.66);
                assert!(p.xz().length() < shared::forest_pickups::PICKUP_RADIUS);
                assert!(p.distance(butterfly_position(Vec3::ZERO, i as f32, t + 1. / 60.)) < 0.03);
            }
        }
    }
}
