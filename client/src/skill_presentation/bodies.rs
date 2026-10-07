//! The staged body of a replicated skill effect: which meshes it is made of and where each
//! one stands. Everything here is a pure function of the row's `body`, of the received
//! effect fields and of what the client observed of the instance. Boundary parts are posed
//! from replicated geometry alone, so no authored value can move or scale them. Authored
//! parts never grow beyond their authored size and stay inside the boundary of an area
//! body. Trails are laid on observed positions only.
use super::category;
use super::geometry::{self, GeoShape};
use super::schema::{Body, Part};
use super::stage::{self, Memory, Stage, StageView};
use super::vocab::{
    Altitude, Archetype, Behaviour, Marker, Model, PaletteSlot, ParticleShape, SatelliteLayout,
    Silhouette, Trail,
};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;
use shared::loadout::{EffectVisualKind, SkillEffectState};
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

/// Visible mesh parts of all skill effects together. Above it the bodies farthest from the
/// camera lose their authored parts; boundaries, fills and markers are never hidden.
pub(crate) const PART_BUDGET: usize = 400;
/// Mesh parts one body may have, and the larger allowance of a skill on a long cooldown.
pub(crate) const MAX_PARTS: usize = 12;
pub(crate) const MAX_PARTS_LONG_COOLDOWN: usize = 18;
pub(crate) const LONG_COOLDOWN_SECS: f32 = 40.0;
/// Effect lights alive at once; the bodies nearest to the camera keep theirs.
pub(crate) const MAX_EFFECT_LIGHTS: usize = 2;
/// Materials all skill effects may share (`3 * primaries + secondaries + accents + 9`).
pub(crate) const MATERIAL_BUDGET: usize = 272;
/// Materials that belong to no row: white and the two team colours, and six status looks.
pub(crate) const SHARED_MATERIALS: usize = 9;

/// Height of the boundary ring above the terrain, clear of the ground decals.
const BOUNDARY_LIFT: f32 = 0.06;
/// The fill lies under the boundary and the markers above it.
const FILL_LIFT: f32 = 0.04;
const MARKER_LIFT: f32 = 0.08;
/// Authored parts and trails on the ground lie above all of them.
const GROUND_LIFT: f32 = 0.12;
/// Height of a body that flies at `chest` and at `high`.
const CHEST: f32 = 0.95;
const HIGH: f32 = 2.3;
/// Height of a rocket's flight light above the root of its body.
pub(crate) const LIGHT_HEIGHT: f32 = CHEST + 0.6;
/// An effect without a replicated heading is laid out toward the side the locked camera
/// looks from, so that its upright flat parts are not seen edge-on.
const REST_HEADING: Vec2 = Vec2::NEG_X;
/// Outer radius of the ring and disc meshes: a part scaled by a diameter has that radius.
const UNIT_RADIUS: f32 = 0.5;
/// Thickness of the torus tube as a share of its diameter.
pub(crate) const TORUS_TUBE: f32 = 0.16;
/// A flat shell of the same kind of material as its core lies this far behind it, so the
/// two do not fight for depth. Light is drawn over matter without it (`effects.rs`).
const SHELL_NUDGE: f32 = 0.03;
/// Inner edge of the boundary ring as a share of its radius. A marker ring ends there, so
/// that the read-out never covers the team colour.
const MARKER_REACH: f32 = 0.47 / UNIT_RADIUS;
/// Height of a part that `rise_on_arm` keeps flat, as a share of its full height.
const FLAT_SHARE: f32 = 0.2;
/// Seconds over which `rise_on_spawn` grows a part.
const RISE_SECS: f32 = 0.2;
/// `blink_last` blinks in this much remaining time, in steps of this length.
const BLINK_SECS: f32 = 0.5;
const BLINK_STEP_SECS: f32 = 0.1;
/// Tilt of a `gyro` ring toward its direction of travel.
const GYRO_TILT: f32 = 0.9;
/// Width of a boundary line made of bars, as a share of the size it bounds and in units:
/// the share is that of the boundary ring.
const LINE_SHARE: f32 = 1.0 - MARKER_REACH;
const LINE_MIN: f32 = 0.08;
const LINE_MAX: f32 = 0.3;
/// Height of a boundary bar and of the sheet that fills or marks a strip.
const LINE_HEIGHT: f32 = 0.05;
const SHEET_HEIGHT: f32 = 0.01;
/// Band of the arc mesh that rounds the end of a strip, as a share of its radius
/// (`game_vfx::shape_mesh`). The edge bars of the strip are as wide, so its outline is one
/// line.
const ARC_BAND: f32 = 0.28;
/// Chord bars that stand for the arc of a cone.
const SECTOR_CHORDS: u8 = 6;
/// Height and thickness of the upper rail of a cage side.
const CAGE_RAIL_HEIGHT: f32 = 1.1;
const CAGE_RAIL: f32 = 0.14;
/// The kite mesh measured from its point: its widest corner and its far corner, along the
/// mesh and to the side (`game_vfx::shape_mesh`).
const KITE_SHOULDER: Vec2 = Vec2::new(0.55, 0.38);
const KITE_TOP: Vec2 = Vec2::new(0.87, 0.33);
/// Where the copies of a `fan` stand in a cone: this share of its length from the apex,
/// spread over this share of its half-angle.
const FAN_DISTANCE: f32 = 0.55;
const FAN_SPREAD: f32 = 0.6;

/// A mesh a body part is drawn with: a silhouette a row may name, or the disc the engine
/// keeps for the `fill` layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PartMesh {
    Silhouette(Silhouette),
    Disc,
}

/// The shared meshes of skill bodies. Each one fits a unit cube around its origin, has at
/// most 200 triangles and is animated through its transform alone.
#[derive(Resource)]
pub(crate) struct VfxMeshes {
    /// In the order of `Silhouette::ALL`.
    silhouettes: Vec<Handle<Mesh>>,
    disc: Handle<Mesh>,
}

impl VfxMeshes {
    pub(crate) fn new(meshes: &mut Assets<Mesh>) -> Self {
        Self {
            silhouettes: Silhouette::ALL
                .iter()
                .map(|mesh| meshes.add(silhouette_mesh(*mesh)))
                .collect(),
            disc: meshes.add(disc_mesh()),
        }
    }

    pub(crate) fn handle(&self, mesh: PartMesh) -> Handle<Mesh> {
        match mesh {
            PartMesh::Silhouette(mesh) => self.silhouettes[mesh as usize].clone(),
            PartMesh::Disc => self.disc.clone(),
        }
    }

    /// The mesh of a particle shape that is also a body silhouette. The particle pool
    /// draws with these handles, so both outlines are one asset.
    pub(crate) fn particle(&self, shape: ParticleShape) -> Option<Handle<Mesh>> {
        Silhouette::ALL
            .iter()
            .find(|mesh| flat_shape(**mesh) == Some(shape))
            .map(|mesh| self.handle(PartMesh::Silhouette(*mesh)))
    }
}

/// Creates the library once, in both render modes: a skill body, a projectile form and a
/// particle all draw from it.
pub(crate) fn setup_meshes(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    commands.insert_resource(VfxMeshes::new(&mut meshes));
}

/// The particle shape whose mesh `silhouette_mesh` builds for a flat silhouette.
const fn flat_shape(mesh: Silhouette) -> Option<ParticleShape> {
    Some(match mesh {
        Silhouette::Kite => ParticleShape::Kite,
        Silhouette::Star => ParticleShape::Star,
        Silhouette::Chevron => ParticleShape::Chevron,
        Silhouette::Diamond => ParticleShape::Diamond,
        Silhouette::Arc => ParticleShape::Arc,
        Silhouette::Drop => ParticleShape::Drop,
        Silhouette::Cross => ParticleShape::Cross,
        Silhouette::Crescent => ParticleShape::Crescent,
        Silhouette::Claw => ParticleShape::Claw,
        Silhouette::Ball
        | Silhouette::Cone
        | Silhouette::Block
        | Silhouette::Ring
        | Silhouette::Torus
        | Silhouette::Shard => return None,
    })
}

/// A regular octahedron; its authored size stretches it into a crystal or a needle.
fn octahedron() -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    for x in [UNIT_RADIUS, -UNIT_RADIUS] {
        for y in [UNIT_RADIUS, -UNIT_RADIUS] {
            for z in [UNIT_RADIUS, -UNIT_RADIUS] {
                let (a, b, c) = (Vec3::X * x, Vec3::Y * y, Vec3::Z * z);
                let normal = Vec3::new(x, y, z).normalize();
                let face = if (b - a).cross(c - a).dot(normal) > 0.0 {
                    [a, b, c]
                } else {
                    [a, c, b]
                };
                positions.extend(face.map(|corner| corner.to_array()));
                normals.extend([normal.to_array(); 3]);
            }
        }
    }
    let count = positions.len();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.5, 0.5]; count])
}

/// The mesh of one silhouette. Flat silhouettes lie in their own XY plane and point along
/// +X; the nine a particle can also have are the particle meshes, so both outlines match.
pub(crate) fn silhouette_mesh(mesh: Silhouette) -> Mesh {
    let flat = crate::game_vfx::shape_mesh;
    match mesh {
        Silhouette::Ball => Sphere::new(UNIT_RADIUS).mesh().ico(1).unwrap(),
        Silhouette::Cone => Cone::new(UNIT_RADIUS, 1.0).mesh().resolution(5).build(),
        Silhouette::Block => Cuboid::new(1.0, 1.0, 1.0).into(),
        Silhouette::Ring => Annulus::new(MARKER_REACH * UNIT_RADIUS, UNIT_RADIUS).into(),
        // Built around the vertical axis; turned to lie in the XY plane like the flat ones.
        Silhouette::Torus => Torus::new(UNIT_RADIUS - TORUS_TUBE, UNIT_RADIUS)
            .mesh()
            .minor_resolution(4)
            .major_resolution(24)
            .build()
            .rotated_by(Quat::from_rotation_x(FRAC_PI_2)),
        Silhouette::Shard => octahedron(),
        Silhouette::Kite => flat(ParticleShape::Kite),
        Silhouette::Star => flat(ParticleShape::Star),
        Silhouette::Chevron => flat(ParticleShape::Chevron),
        Silhouette::Diamond => flat(ParticleShape::Diamond),
        Silhouette::Arc => flat(ParticleShape::Arc),
        Silhouette::Drop => flat(ParticleShape::Drop),
        Silhouette::Cross => flat(ParticleShape::Cross),
        Silhouette::Crescent => flat(ParticleShape::Crescent),
        Silhouette::Claw => flat(ParticleShape::Claw),
    }
}

pub(crate) fn disc_mesh() -> Mesh {
    Circle::new(UNIT_RADIUS).into()
}

/// Whether the body is an object in flight. Its sizes are metres; the sizes of every other
/// archetype are multiples of the replicated radius.
pub(crate) const fn moving(archetype: Archetype) -> bool {
    matches!(archetype, Archetype::Traveller | Archetype::Orbiter)
}

/// The height band of a body that names none.
pub(crate) fn altitude(body: &Body) -> Altitude {
    body.altitude.unwrap_or(if moving(body.archetype) {
        Altitude::Chest
    } else {
        Altitude::Ground
    })
}

/// Whether the body has the translucent interior layer: its own pick, else on for the
/// archetypes whose whole area acts.
pub(crate) fn fills(body: &Body) -> bool {
    !moving(body.archetype)
        && body.fill.unwrap_or(matches!(
            body.archetype,
            Archetype::Zone | Archetype::Lane | Archetype::Sector
        ))
}

/// Height above the ground at which a stage one-shot of this body is drawn: where a body
/// in flight is, and on the ground for everything that stands on it.
pub(crate) fn burst_lift(body: &Body) -> f32 {
    if moving(body.archetype) {
        height(altitude(body))
    } else {
        0.0
    }
}

const fn height(altitude: Altitude) -> f32 {
    match altitude {
        Altitude::Ground => BOUNDARY_LIFT,
        Altitude::Chest => CHEST,
        Altitude::High => HIGH,
    }
}

/// Boundary parts the engine draws for an archetype over one received shape: a ring for a
/// circle, two edge bars for a strip and two half rings more for its round caps, two edge
/// bars and six chord bars for a cone, one bar for a wall or a fog-cut cone, five low and
/// five high bars for a cage. A kind without a boundary has none.
pub(crate) fn engine_parts(archetype: Archetype, shape: &GeoShape) -> u8 {
    match (archetype, shape) {
        (_, GeoShape::None) => 0,
        (Archetype::Lane, GeoShape::Capsule { .. }) => 4,
        (Archetype::Lane, _) => 2,
        (Archetype::Sector, GeoShape::Sector { .. }) => 8,
        (Archetype::Cage, _) => 10,
        (
            Archetype::Traveller
            | Archetype::Orbiter
            | Archetype::Zone
            | Archetype::Prop
            | Archetype::Wall
            | Archetype::Sector,
            _,
        ) => 1,
    }
}

const fn trail_parts(trail: Trail) -> u8 {
    match trail {
        Trail::None => 0,
        Trail::Ribbon => 2,
        Trail::Motes | Trail::Chevrons => 3,
        Trail::Links => 6,
    }
}

const fn marker_parts(marker: Marker) -> u8 {
    match marker {
        Marker::None => 0,
        Marker::ArmingPips => 3,
        Marker::RemainingRing | Marker::FillToEdge | Marker::OwnerTether => 1,
    }
}

/// Mesh parts of a body over one received shape: the engine's and every authored one.
pub(crate) fn part_total(body: &Body, shape: &GeoShape) -> usize {
    usize::from(engine_parts(body.archetype, shape))
        + usize::from(fills(body))
        + usize::from(body.core.is_some())
        + usize::from(body.shell.is_some())
        + body
            .satellites
            .as_ref()
            .map_or(0, |part| usize::from(part.count))
        + usize::from(trail_parts(body.trail))
        + usize::from(marker_parts(body.marker))
        + usize::from(body.model.is_some())
}

/// What one part of a body is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    /// The replicated boundary, in the team colour.
    Boundary(u8),
    /// The translucent interior layer.
    Fill,
    Core,
    Shell,
    Satellite(u8),
    /// A part on an earlier observed position.
    Trail(u8),
    /// The stage read-out.
    Marker(u8),
    /// The packaged prop.
    Model,
}

impl Role {
    /// Parts the budget may hide: the look a row authors. The boundary, the fill and the
    /// marker carry what the effect does and stay.
    pub(crate) const fn authored(self) -> bool {
        matches!(
            self,
            Self::Core | Self::Shell | Self::Satellite(_) | Self::Trail(_) | Self::Model
        )
    }
}

/// The material of a part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Paint {
    /// Friend or foe.
    Team,
    Slot(PaletteSlot),
    Fill,
    /// The fill of a telegraph.
    FillDim,
}

/// What a part knows of the body it belongs to.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Plan {
    archetype: Archetype,
    altitude: Altitude,
    model: Option<Model>,
    layout: Option<(SatelliteLayout, u8)>,
    trail: Trail,
    marker: Marker,
    /// The body has a `core`.
    has_core: bool,
    /// The shell is made of the same kind of material as the core: both matter, or both
    /// light.
    shell_like_core: bool,
}

impl Plan {
    /// Rule E-5 for a flat part whose sizes do not name its normal: it lies on the ground,
    /// and it stands across the heading in the air and in a wall.
    fn stands(&self) -> bool {
        self.altitude != Altitude::Ground || self.archetype == Archetype::Wall
    }
}

/// One mesh part of a body.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PartSlot {
    pub role: Role,
    /// `None` for the packaged model.
    pub mesh: Option<PartMesh>,
    /// Authored extents: lateral, vertical, along the heading.
    size: Vec3,
    slot: PaletteSlot,
    behave: Behaviour,
    plan: Plan,
}

/// The parts of a body over one received shape, in a fixed order. The boundary, the fill
/// and the marker are made of the meshes that shape asks for.
pub(crate) fn part_list(body: &Body, shape: &GeoShape) -> Vec<PartSlot> {
    let plan = Plan {
        archetype: body.archetype,
        altitude: altitude(body),
        model: body.model,
        layout: body
            .satellites
            .as_ref()
            .map(|part| (part.layout, part.count)),
        trail: body.trail,
        marker: body.marker,
        has_core: body.core.is_some(),
        shell_like_core: body.core.as_ref().zip(body.shell.as_ref()).is_some_and(
            |(core, shell)| {
                (core.slot == PaletteSlot::Secondary) == (shell.slot == PaletteSlot::Secondary)
            },
        ),
    };
    let engine = |role, mesh| PartSlot {
        role,
        mesh: Some(mesh),
        size: Vec3::ONE,
        slot: PaletteSlot::Primary,
        behave: Behaviour::Steady,
        plan,
    };
    let authored = |role, part: &Part| PartSlot {
        role,
        mesh: Some(PartMesh::Silhouette(part.mesh)),
        size: Vec3::from_array(part.size),
        slot: part.slot,
        behave: part.behave,
        plan,
    };
    let ring = PartMesh::Silhouette(Silhouette::Ring);
    let block = PartMesh::Silhouette(Silhouette::Block);
    // A circle is a ring; every other boundary is made of bars, and the two round ends of
    // a strip are half rings.
    let boundary = |index: u8| match shape {
        GeoShape::Ring { .. } | GeoShape::None => ring,
        GeoShape::Capsule { .. } if index >= 2 => PartMesh::Silhouette(Silhouette::Arc),
        _ => block,
    };
    // What covers the inside of the shape: a disc, a sheet over a strip, a kite in a cone.
    let (area, edge) = match shape {
        GeoShape::Capsule { .. } | GeoShape::Lane { .. } => (block, block),
        GeoShape::Sector { .. } => {
            let kite = PartMesh::Silhouette(Silhouette::Kite);
            (kite, kite)
        }
        _ => (PartMesh::Disc, ring),
    };
    let mut parts: Vec<PartSlot> = (0..engine_parts(body.archetype, shape))
        .map(|index| engine(Role::Boundary(index), boundary(index)))
        .collect();
    if fills(body) {
        parts.push(engine(Role::Fill, area));
    }
    parts.extend(body.core.as_ref().map(|part| authored(Role::Core, part)));
    parts.extend(body.shell.as_ref().map(|part| authored(Role::Shell, part)));
    if let Some(part) = &body.satellites {
        parts.extend((0..part.count).map(|index| PartSlot {
            role: Role::Satellite(index),
            mesh: Some(PartMesh::Silhouette(part.mesh)),
            size: Vec3::from_array(part.size),
            slot: part.slot,
            behave: part.behave,
            plan,
        }));
    }
    let trail = PartMesh::Silhouette(match body.trail {
        Trail::None | Trail::Ribbon => Silhouette::Block,
        Trail::Motes => Silhouette::Ball,
        Trail::Chevrons => Silhouette::Chevron,
        Trail::Links => Silhouette::Torus,
    });
    parts.extend((0..trail_parts(body.trail)).map(|index| engine(Role::Trail(index), trail)));
    let marker = match body.marker {
        Marker::None | Marker::RemainingRing => ring,
        Marker::FillToEdge => edge,
        Marker::ArmingPips => PartMesh::Silhouette(Silhouette::Ball),
        Marker::OwnerTether => block,
    };
    parts.extend((0..marker_parts(body.marker)).map(|index| engine(Role::Marker(index), marker)));
    if body.model.is_some() {
        parts.push(PartSlot {
            mesh: None,
            ..engine(Role::Model, ring)
        });
    }
    parts
}

/// Where the frame of a body stands on the ground and the direction its +Z points. A kind
/// that carries a heading, a strip, a cone and a wall point from `position` toward `end`;
/// everything else has no direction of its own. The strip of an owner the viewer does not
/// see is laid out from whichever end sorts first, so that nothing drawn in it says which
/// end is the origin.
fn frame(effect: &SkillEffectState, shape: &GeoShape) -> (Vec2, Vec2) {
    let position = Vec2::from_array(effect.position);
    let end = Vec2::from_array(effect.end);
    let strip = matches!(shape, GeoShape::Capsule { .. } | GeoShape::Lane { .. });
    let (origin, toward) =
        if strip && effect.owner_id == 0 && (end.x, end.y) < (position.x, position.y) {
            (end, position)
        } else {
            (position, end)
        };
    let directed = strip
        || category::heading_only(effect.kind)
        || matches!(shape, GeoShape::Sector { .. } | GeoShape::Segment { .. });
    let heading = Some(toward - origin)
        .filter(|_| directed)
        .and_then(Vec2::try_normalize)
        .unwrap_or(REST_HEADING);
    (origin, heading)
}

/// The ground position the root of the body of an effect stands on.
pub(crate) fn root_at(effect: &SkillEffectState) -> Vec2 {
    frame(
        effect,
        &geometry::boundary_shape(effect.skill, effect.kind, effect),
    )
    .0
}

/// A strip in the frame of its body: it runs from the root along +Z.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Strip {
    length: f32,
    /// Half-width of the strip.
    radius: f32,
    /// Width of its outline; the inside of the outline ends `radius - line` from the middle.
    line: f32,
}

/// One instance as the client holds it in a frame.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Seen<'a> {
    pub effect: &'a SkillEffectState,
    /// The replicated boundary.
    pub shape: GeoShape,
    pub view: StageView,
    /// Positions observed before the newest one, newest first.
    pub history: &'a [Vec2],
    /// The instance has stood still for longer than two snapshots are apart.
    pub resting: bool,
    /// A renewal of this very instance was observed.
    pub renewed: bool,
    /// The longest remaining time any sighting of this instance showed.
    pub peak_remaining_secs: f32,
    /// The chest of the owner, when the effect names one and the client draws that hero.
    pub owner: Option<Vec3>,
    /// Terrain height under the root of the body (`root_at`).
    pub ground: f32,
    /// The presentation clock.
    pub now: f32,
    /// Where the frame of the body stands and where its +Z points (`frame`).
    origin: Vec2,
    heading: Vec2,
}

impl<'a> Seen<'a> {
    /// `memory` is what the stage tracker holds of the instance; without it the instance is
    /// drawn as first seen.
    pub(crate) fn of(
        effect: &'a SkillEffectState,
        memory: Option<&'a Memory>,
        owner: Option<Vec3>,
        ground: f32,
        now: f32,
    ) -> Self {
        // An auxiliary object is known as the n-th of its kind and owner. A hero has one
        // orb, so its key names one object; souls, anchors and healing fields change
        // places under their keys, and what a key saw may be another object's path.
        let followed = memory.filter(|_| {
            !category::unstable_id(effect.kind) || effect.kind == EffectVisualKind::Orb
        });
        let shape = geometry::boundary_shape(effect.skill, effect.kind, effect);
        let (origin, heading) = frame(effect, &shape);
        Self {
            effect,
            shape,
            view: stage::view(effect),
            history: followed.map_or(&[], Memory::trail),
            resting: followed.is_some_and(Memory::resting),
            renewed: followed.is_some_and(|memory| memory.renewed),
            peak_remaining_secs: memory
                .map_or(effect.remaining_secs, |memory| memory.peak_remaining_secs),
            owner: owner.filter(|_| effect.owner_id != 0),
            ground,
            now,
            origin,
            heading,
        }
    }

    /// A ground position in the frame of the body: lateral and along the heading.
    fn local(&self, point: Vec2) -> Vec2 {
        let offset = point - self.origin;
        Vec2::new(
            offset.dot(Vec2::new(self.heading.y, -self.heading.x)),
            offset.dot(self.heading),
        )
    }

    /// The radius of the circle an area body is laid out in: the replicated circle, or the
    /// largest one inside a cage.
    fn circle(&self) -> Option<f32> {
        match self.shape {
            GeoShape::Ring { radius, .. } => Some(radius),
            GeoShape::Pentagon { radius, .. } => Some(radius * (PI / 5.0).cos()),
            _ => None,
        }
    }

    /// The strip of a lane body.
    fn strip(&self) -> Option<Strip> {
        let (from, to, radius) = match self.shape {
            GeoShape::Capsule { from, to, radius } => (from, to, radius),
            GeoShape::Lane {
                from,
                to,
                half_width,
            } => (from, to, half_width),
            _ => return None,
        };
        Some(Strip {
            length: from.distance(to),
            radius,
            line: radius * ARC_BAND,
        })
    }
}

/// The root of a body: on the ground where its frame stands, +Z along its heading.
pub(crate) fn root_pose(seen: &Seen) -> Transform {
    Transform::from_xyz(seen.origin.x, seen.ground, seen.origin.y)
        .with_rotation(Quat::from_rotation_y(seen.heading.x.atan2(seen.heading.y)))
}

/// Axes of a part: lateral, vertical, along the heading.
const LATERAL: usize = 0;
const VERTICAL: usize = 1;
const HEADING: usize = 2;

pub(crate) fn planar(mesh: PartMesh) -> bool {
    match mesh {
        PartMesh::Disc => true,
        PartMesh::Silhouette(mesh) => !matches!(
            mesh,
            Silhouette::Ball | Silhouette::Block | Silhouette::Shard | Silhouette::Cone
        ),
    }
}

/// The axis a flat mesh faces. A flat mesh has no thickness, so its normal is the thinnest
/// authored extent. Where the sizes do not decide, rule E-5 does: a mesh that `stands` has
/// its normal along the heading, any other lies in the ground plane (`Plan::stands`).
fn normal_axis(size: Vec3, stands: bool) -> usize {
    let order = if stands {
        [HEADING, VERTICAL, LATERAL]
    } else {
        [VERTICAL, HEADING, LATERAL]
    };
    let thinnest = size.min_element() * (1.0 + 1e-3);
    order
        .into_iter()
        .find(|axis| size[*axis] <= thinnest)
        .unwrap_or(order[0])
}

/// How a mesh is laid into its part: the rotation, and the part axis each mesh axis then
/// lies along. A solid keeps its axes. A cone points along its longest authored extent. A
/// flat mesh faces its normal axis and points along the heading, or upward when it faces
/// the heading.
pub(crate) fn lay(mesh: PartMesh, size: Vec3, stands: bool) -> (Quat, [usize; 3]) {
    let turned = |x: Vec3, y: Vec3, z: Vec3| Quat::from_mat3(&Mat3::from_cols(x, y, z));
    if planar(mesh) {
        return match normal_axis(size, stands) {
            VERTICAL => (
                turned(Vec3::Z, Vec3::X, Vec3::Y),
                [HEADING, LATERAL, VERTICAL],
            ),
            HEADING => (
                turned(Vec3::Y, Vec3::NEG_X, Vec3::Z),
                [VERTICAL, LATERAL, HEADING],
            ),
            _ => (
                turned(Vec3::Z, Vec3::Y, Vec3::NEG_X),
                [HEADING, VERTICAL, LATERAL],
            ),
        };
    }
    if mesh != PartMesh::Silhouette(Silhouette::Cone) {
        return (Quat::IDENTITY, [LATERAL, VERTICAL, HEADING]);
    }
    let longest = size.max_element() * (1.0 - 1e-3);
    match [VERTICAL, HEADING, LATERAL]
        .into_iter()
        .find(|axis| size[*axis] >= longest)
    {
        Some(HEADING) => (
            turned(Vec3::X, Vec3::Z, Vec3::NEG_Y),
            [LATERAL, HEADING, VERTICAL],
        ),
        Some(LATERAL) => (
            turned(Vec3::NEG_Y, Vec3::X, Vec3::Z),
            [VERTICAL, LATERAL, HEADING],
        ),
        _ => (Quat::IDENTITY, [LATERAL, VERTICAL, HEADING]),
    }
}

/// Scale of a mesh whose axes lie along `axes` of a part with these extents. A flat mesh
/// has no thickness to scale, and a torus is never thicker than its own tube.
pub(crate) fn mesh_scale(mesh: PartMesh, extent: Vec3, axes: [usize; 3]) -> Vec3 {
    let along = Vec3::new(extent[axes[0]], extent[axes[1]], extent[axes[2]]);
    if !planar(mesh) {
        return along;
    }
    let thickness = if mesh == PartMesh::Silhouette(Silhouette::Torus) {
        (along.z / TORUS_TUBE).min(along.x.min(along.y))
    } else {
        1.0
    };
    Vec3::new(along.x, along.y, thickness)
}

/// A phase that keeps two parts, and two instances, from moving in step.
fn phase(seen: &Seen, role: Role) -> f32 {
    let index = match role {
        Role::Satellite(index) | Role::Trail(index) | Role::Marker(index) => index + 3,
        Role::Boundary(index) => index,
        Role::Core | Role::Fill => 1,
        Role::Shell | Role::Model => 2,
    };
    (seen.effect.id % 64) as f32 * 0.71 + f32::from(index) * 1.9
}

/// What a behaviour does to a part in one frame. It turns, lifts, shrinks or hides the
/// part; it never makes it larger than authored.
struct Motion {
    turn: Quat,
    /// Share of the authored extent on each part axis, at most 1.
    share: Vec3,
    lift: f32,
}

/// `None` hides the part.
fn motion(slot: &PartSlot, seen: &Seen) -> Option<Motion> {
    let now = seen.now;
    let phase = phase(seen, slot.role);
    let mut motion = Motion {
        turn: Quat::IDENTITY,
        share: Vec3::ONE,
        lift: 0.0,
    };
    let telegraph = seen.view.stage == Stage::Telegraph;
    match slot.behave {
        Behaviour::Steady | Behaviour::Gyro => {}
        Behaviour::Spin => motion.turn = Quat::from_rotation_y(now * 1.6 + phase),
        Behaviour::Tumble => motion.turn = Quat::from_rotation_x(now * 5.0 + phase),
        Behaviour::Pulse => {
            motion.share = Vec3::splat(0.93 - 0.07 * (now * 5.0 + phase).sin());
        }
        Behaviour::Bob => motion.lift = 0.08 + 0.08 * (now * 2.4 + phase).sin(),
        Behaviour::Flicker => {
            let noise = (now * 19.0 + phase).sin() * (now * 31.0 + 1.7 * phase).sin();
            motion.share = Vec3::splat(0.72 + 0.28 * noise.abs());
        }
        Behaviour::HideInTelegraph if telegraph => return None,
        Behaviour::OnlyInTelegraph if !telegraph => return None,
        Behaviour::HideInTelegraph | Behaviour::OnlyInTelegraph => {}
        Behaviour::RiseOnArm if !seen.effect.armed => motion.share.y = FLAT_SHARE,
        Behaviour::RiseOnArm => {}
        Behaviour::BlinkLast => {
            let remaining = seen.view.remaining;
            if remaining <= BLINK_SECS && (remaining / BLINK_STEP_SECS) as u32 % 2 == 1 {
                return None;
            }
        }
        Behaviour::OnlyAfterRenew if !seen.renewed => return None,
        Behaviour::OnlyAfterRenew => {}
        Behaviour::RiseOnSpawn => {
            // An instance first seen later is older than the rise and stands at full height,
            // and so does one with more time left than its lifetime: its age is not known.
            if let Some(spawn) = category::spawn_lifetime_secs(seen.effect.skill) {
                let age = spawn - seen.view.remaining;
                if age >= 0.0 {
                    motion.share.y = (age / RISE_SECS).clamp(FLAT_SHARE, 1.0);
                }
            }
        }
    }
    Some(motion)
}

/// Whether the part runs the whole length of a strip. Rule E-7: the along-axis of the
/// `core` and the `shell` of a lane is a share of the received segment.
fn spans_strip(slot: &PartSlot) -> bool {
    slot.plan.archetype == Archetype::Lane && matches!(slot.role, Role::Core | Role::Shell)
}

/// The authored extents of a part in metres.
fn extent(slot: &PartSlot, seen: &Seen) -> Vec3 {
    if moving(slot.plan.archetype) {
        return slot.size;
    }
    let size = slot.size * seen.effect.radius;
    match seen.strip().filter(|_| spans_strip(slot)) {
        Some(strip) => Vec3::new(size.x, size.y, slot.size.z * strip.length),
        None => size,
    }
}

/// Radius of the circle on the ground that a part can sweep around its own centre.
fn swept(slot: &PartSlot, extent: Vec3) -> f32 {
    if matches!(slot.behave, Behaviour::Tumble | Behaviour::Gyro) {
        extent.length() * 0.5
    } else {
        extent.xz().length() * 0.5
    }
}

/// How far a part reaches on the ground from its own centre, to the side and along the
/// heading: half its extents, or the circle it sweeps when its behaviour turns it.
fn footprint(slot: &PartSlot, extent: Vec3) -> Vec2 {
    if matches!(
        slot.behave,
        Behaviour::Spin | Behaviour::Tumble | Behaviour::Gyro
    ) {
        Vec2::splat(swept(slot, extent))
    } else {
        extent.xz() * 0.5
    }
}

/// Places an authored mesh: `at` is its centre on the ground plane and its height above
/// the height band of the body, `yaw` turns the part about the vertical.
fn authored(slot: &PartSlot, seen: &Seen, at: Vec3, yaw: f32) -> Option<Transform> {
    let mesh = slot.mesh?;
    let mut motion = motion(slot, seen)?;
    if spans_strip(slot) {
        // The length of a strip is not the part's to change, and a part that long cannot
        // turn inside it.
        motion.share[HEADING] = 1.0;
        motion.turn = Quat::IDENTITY;
    }
    let extent = extent(slot, seen);
    let (laid, axes) = if slot.behave == Behaviour::Gyro && planar(mesh) {
        gyro(seen)
    } else {
        lay(mesh, slot.size, slot.plan.stands())
    };
    let scale = mesh_scale(mesh, extent * motion.share, axes);
    // A part on the ground stands on it; a mesh lying flat is as tall as it is thick.
    let tall = match (planar(mesh) && axes[2] == VERTICAL, mesh) {
        (true, PartMesh::Silhouette(Silhouette::Torus)) => scale.z * TORUS_TUBE,
        (true, _) => 0.0,
        (false, _) => extent.y * motion.share.y,
    };
    let stands = match slot.plan.altitude {
        Altitude::Ground => GROUND_LIFT + tall * 0.5,
        Altitude::Chest => CHEST,
        Altitude::High => HIGH,
    };
    Some(Transform {
        translation: Vec3::new(at.x, at.y + stands + motion.lift, at.z),
        rotation: Quat::from_rotation_y(yaw) * motion.turn * laid,
        scale,
    })
}

/// The lay of a `gyro` ring: level while the body rests, tilted toward its travel while it
/// moves.
fn gyro(seen: &Seen) -> (Quat, [usize; 3]) {
    let level = (
        Quat::from_mat3(&Mat3::from_cols(Vec3::Z, Vec3::X, Vec3::Y)),
        [HEADING, LATERAL, VERTICAL],
    );
    let travel = seen
        .history
        .first()
        .filter(|_| !seen.resting)
        .and_then(|from| (-seen.local(*from)).try_normalize());
    match travel {
        Some(travel) => {
            let axis = Vec3::Y.cross(Vec3::new(travel.x, 0.0, travel.y));
            (Quat::from_axis_angle(axis, GYRO_TILT) * level.0, level.1)
        }
        None => level,
    }
}

/// A flat ring or disc of `radius` on the ground at height `lift`.
fn flat_circle(radius: f32, lift: f32) -> Transform {
    Transform::from_xyz(0.0, lift, 0.0)
        .with_rotation(Quat::from_rotation_x(-FRAC_PI_2))
        .with_scale(Vec3::new(radius / UNIT_RADIUS, radius / UNIT_RADIUS, 1.0))
}

/// A unit block stretched from `from` to `to`.
fn bar(from: Vec3, to: Vec3, width: f32, thickness: f32) -> Option<Transform> {
    let span = to - from;
    let along = span.try_normalize()?;
    Some(Transform {
        translation: (from + to) * 0.5,
        rotation: Quat::from_rotation_arc(Vec3::Z, along),
        scale: Vec3::new(width, thickness, span.length()),
    })
}

/// Scale of a packaged prop and the height of its top above its origin at that scale. A
/// prop on an area body stays inside the boundary.
fn model_fit(model: Model, slot: &PartSlot, radius: f32) -> (f32, f32) {
    // Scale as today, half width and top of the packaged scene.
    let (scale, half_width, top) = match model {
        Model::Rocket => (1.15, 0.47, 0.47),
        Model::Trap => (radius * 1.6, 0.46, 0.4),
        Model::Hook => (1.15, 0.66, 0.12),
        Model::Lantern => (1.15, 0.42, 1.2),
        Model::Orb => (1.25, 0.48, 0.4),
    };
    let scale = if moving(slot.plan.archetype) {
        scale
    } else {
        scale.min(radius / half_width)
    };
    (scale, top * scale)
}

/// The side of the strip copy `index` of a `stagger` stands on: left and right in turn
/// from each end toward the middle, where an odd copy stands on the centre line. The row
/// is its own mirror image when the two ends of the strip change places (rule P-2).
fn stagger_side(index: u8, count: u8) -> f32 {
    let mirrored = count.saturating_sub(1).saturating_sub(index);
    let (near, flip) = if index <= mirrored {
        (index, 1.0)
    } else {
        (mirrored, -1.0)
    };
    if index == mirrored {
        0.0
    } else if near.is_multiple_of(2) {
        flip
    } else {
        -flip
    }
}

/// The wall bar in the frame of its body: its two ends.
fn wall_bar(seen: &Seen) -> Option<(Vec2, Vec2)> {
    match seen.shape {
        GeoShape::Segment { from, to } => Some((seen.local(from), seen.local(to))),
        _ => None,
    }
}

/// A place on the wall bar. Rule E-8: a `line` of N copies beside a `core` shares N + 1
/// equal slots across the bar with it, and the core has the middle one (with an odd N,
/// the slot before the middle). `copy` is the index of a satellite, `None` for the core
/// and the shell. The part stays inside the two ends of the bar.
fn wall_place(slot: &PartSlot, copy: Option<u8>, seen: &Seen) -> Option<Vec3> {
    let (from, to) = wall_bar(seen)?;
    let line = slot
        .plan
        .layout
        .filter(|(layout, _)| *layout == SatelliteLayout::Line)
        .map(|(_, count)| count);
    let (place, slots) = match (copy, line) {
        // Without a row of copies beside it the core stands in the middle of the bar.
        (None, None) => (0, 1),
        (None, Some(count)) => (count / 2, count + 1),
        (Some(index), Some(count)) if slot.plan.has_core => {
            (index + u8::from(index >= count / 2), count + 1)
        }
        (Some(index), Some(count)) => (index, count),
        (Some(_), None) => return None,
    };
    let share = (f32::from(place) + 0.5) / f32::from(slots.max(1));
    let half = from.distance(to) * 0.5;
    // Half the width of the part along the bar.
    let reach = (half - footprint(slot, extent(slot, seen)).x).max(0.0);
    let along = ((share * 2.0 - 1.0) * half).clamp(-reach, reach);
    let at = from.midpoint(to) + (to - from).normalize_or_zero() * along;
    Some(Vec3::new(at.x, 0.0, at.y))
}

/// Where the `core` and the `shell` of a body stand relative to its root: at the effect
/// position, in the middle of a strip, on the bar of a wall, and in a cone at the centre of
/// the largest circle inside it. `None` when the received shape has no such place.
fn centre(slot: &PartSlot, seen: &Seen) -> Option<Vec3> {
    let extent = extent(slot, seen);
    match slot.plan.archetype {
        Archetype::Lane => seen.strip().map(|strip| Vec3::Z * strip.length * 0.5),
        Archetype::Wall => wall_place(slot, None, seen),
        Archetype::Sector => match seen.shape {
            GeoShape::Sector {
                radius, half_angle, ..
            } => {
                let sine = half_angle.min(FRAC_PI_2).sin();
                let fit = radius * sine / (1.0 + sine);
                (swept(slot, extent) <= fit).then(|| Vec3::Z * (radius - fit))
            }
            _ => None,
        },
        // Rule E-6: the plate of a stance stands half a radius ahead of where it was
        // cast, in front of its caster, and still inside its ring.
        Archetype::Prop
            if seen.effect.kind == EffectVisualKind::Barrier && slot.mesh.is_some_and(planar) =>
        {
            Some(Vec3::Z * seen.effect.radius * 0.5)
        }
        Archetype::Traveller
        | Archetype::Orbiter
        | Archetype::Zone
        | Archetype::Prop
        | Archetype::Cage => Some(Vec3::ZERO),
    }
}

/// Where copy `index` of the satellites stands relative to the root of the body, and the
/// yaw it faces. Copies of an area body stay inside its boundary; copies of a body in
/// flight stay inside its replicated radius. A layout the received shape cannot carry has
/// no place.
fn copy_place(slot: &PartSlot, index: u8, seen: &Seen) -> Option<(Vec3, f32)> {
    let (layout, count) = slot.plan.layout?;
    let extent = extent(slot, seen);
    let flies = moving(slot.plan.archetype);
    match (layout, seen.shape) {
        (
            SatelliteLayout::Line | SatelliteLayout::Stagger,
            GeoShape::Capsule { .. } | GeoShape::Lane { .. },
        ) => {
            // Evenly along the received segment and inside its two ends; `stagger` puts
            // the copies against the inside of the outline, left and right in turn.
            let strip = seen.strip()?;
            let half = footprint(slot, extent);
            let middle = strip.length * 0.5;
            let along = ((f32::from(index) + 0.5) / f32::from(count.max(1)) * strip.length)
                .clamp(half.y.min(middle), (strip.length - half.y).max(middle));
            let side = if layout == SatelliteLayout::Stagger {
                stagger_side(index, count) * (strip.radius - strip.line - half.x).max(0.0)
            } else {
                0.0
            };
            return Some((Vec3::new(side, 0.0, along), 0.0));
        }
        (SatelliteLayout::Line, GeoShape::Segment { .. })
            if slot.plan.archetype == Archetype::Wall =>
        {
            return wall_place(slot, Some(index), seen).map(|at| (at, 0.0));
        }
        (
            SatelliteLayout::Fan,
            GeoShape::Sector {
                radius, half_angle, ..
            },
        ) => {
            // Each copy keeps the circle it sweeps away from the arc and from both edges.
            let reach = swept(slot, extent);
            let near = reach / half_angle.min(FRAC_PI_2).sin();
            let far = radius - reach;
            if near > far {
                return None;
            }
            let distance = (radius * FAN_DISTANCE).clamp(near, far);
            let widest = half_angle - (reach / distance).min(1.0).asin();
            let side = if count < 2 {
                0.0
            } else {
                f32::from(index) / f32::from(count - 1) * 2.0 - 1.0
            };
            let angle = side * widest.min(half_angle * FAN_SPREAD).max(0.0);
            return Some((
                Vec3::new(angle.sin() * distance, 0.0, angle.cos() * distance),
                angle,
            ));
        }
        (SatelliteLayout::Rim, GeoShape::Pentagon { center, radius }) => {
            // Rule E-9: the first copy stands in the first corner of the cage, toward
            // world +X, and faces its centre.
            let angle = TAU * f32::from(index) / f32::from(count.max(1));
            let reach = (radius - swept(slot, extent)).max(0.0);
            let at = seen.local(center + Vec2::from_angle(angle) * reach);
            return Some((Vec3::new(at.x, 0.0, at.y), (-at.x).atan2(-at.y)));
        }
        _ => {}
    }
    // A body in flight is laid out around its replicated radius; an area body needs the
    // circle it is bounded by.
    let radius = if flies {
        seen.effect.radius
    } else {
        seen.circle()?
    };
    let step = TAU / f32::from(count.max(1));
    let turn = f32::from(index) * step;
    let reach = (radius - swept(slot, extent)).max(0.0);
    // The same reach in the vertical plane across the heading.
    let across = (radius - extent.xy().length() * 0.5).max(0.0);
    let on_ring = |angle: f32, distance: f32| Vec3::new(angle.cos(), 0.0, angle.sin()) * distance;
    // The yaw that turns the heading of a copy at `angle` toward the centre.
    let inward = |angle: f32| (-angle.cos()).atan2(-angle.sin());
    // How far a vertical ring of this radius is raised so that it clears the ground.
    let clear = |distance: f32| (distance - height(slot.plan.altitude)).max(0.0);
    let phase = phase(seen, Role::Core);
    Some(match layout {
        SatelliteLayout::Orbit => {
            let angle = seen.now * 1.2 + phase + turn;
            let distance = if flies {
                reach
            } else {
                reach.min(radius * 0.72)
            };
            // The copy heads along its travel.
            (on_ring(angle, distance), -angle)
        }
        SatelliteLayout::Halo => {
            let angle = seen.now * 3.0 + phase + turn;
            (
                Vec3::new(
                    angle.cos() * across,
                    angle.sin() * across + clear(across),
                    0.0,
                ),
                0.0,
            )
        }
        SatelliteLayout::Helix => {
            let angle = seen.now * 6.0 + phase + turn;
            let distance = across.min(0.5);
            let spacing = (extent.z * 0.8).clamp(0.2, 1.5 / f32::from(count.max(1)));
            (
                Vec3::new(
                    angle.cos() * distance,
                    angle.sin() * distance + clear(distance),
                    -(f32::from(index) + 1.0) * spacing,
                ),
                0.0,
            )
        }
        SatelliteLayout::Column => {
            // Rule E-11: above a packaged prop the column starts at its top.
            let base = slot
                .plan
                .model
                .map_or(0.0, |model| model_fit(model, slot, radius).1);
            (Vec3::Y * (base + f32::from(index) * extent.y * 1.25), 0.0)
        }
        SatelliteLayout::QuadX => {
            let angle = FRAC_PI_4 + turn;
            let distance = if flies {
                reach
            } else {
                reach.min(radius * 0.6)
            };
            (on_ring(angle, distance), inward(angle))
        }
        SatelliteLayout::Rim => (on_ring(turn, reach), inward(turn)),
        SatelliteLayout::Fan => {
            let side = if count < 2 {
                0.0
            } else {
                f32::from(index) / f32::from(count - 1) * 2.0 - 1.0
            };
            let width = (radius - extent.x * 0.5).max(0.0);
            (Vec3::X * side * width, side * 0.4)
        }
        SatelliteLayout::Line | SatelliteLayout::Stagger => return None,
    })
}

/// A trail part. Its points are the newest position and the ones observed before it; a
/// first sighting, a turn and a relocation leave none, and a body at rest shows none.
fn trail(slot: &PartSlot, index: u8, seen: &Seen) -> Option<Transform> {
    if seen.resting {
        return None;
    }
    let stands = match slot.plan.altitude {
        Altitude::Ground => GROUND_LIFT,
        Altitude::Chest | Altitude::High => height(slot.plan.altitude),
    };
    let point = |step: usize| -> Option<Vec3> {
        if step == 0 {
            return Some(Vec3::Y * stands);
        }
        let at = seen.local(*seen.history.get(step - 1)?);
        Some(Vec3::new(at.x, stands, at.y))
    };
    let index = usize::from(index);
    let radius = seen.effect.radius;
    // Older parts are smaller.
    let fade = 1.0 - index as f32 * 0.22;
    let (at, toward) = (point(index + 1)?, point(index)?);
    let facing = (toward - at).xz();
    let yaw = facing.x.atan2(facing.y);
    let flat = Quat::from_mat3(&Mat3::from_cols(Vec3::Z, Vec3::X, Vec3::Y));
    match slot.plan.trail {
        Trail::None => None,
        Trail::Ribbon => bar(toward, at, (radius * 0.7).clamp(0.12, 0.45) * fade, 0.06),
        Trail::Motes => Some(
            Transform::from_translation(at)
                .with_scale(Vec3::splat((radius * 0.6).clamp(0.14, 0.4) * fade)),
        ),
        Trail::Chevrons => {
            let size = (radius * 1.4).clamp(0.35, 1.6) * fade;
            Some(Transform {
                translation: at,
                rotation: Quat::from_rotation_y(yaw) * flat,
                scale: Vec3::new(size, size, 1.0),
            })
        }
        Trail::Links => {
            // Every second link stands on edge, as in a chain.
            let edge = if index % 2 == 1 {
                Quat::from_rotation_z(FRAC_PI_2)
            } else {
                Quat::IDENTITY
            };
            let size = (radius * 0.7).clamp(0.28, 0.5);
            Some(Transform {
                translation: at,
                rotation: Quat::from_rotation_y(yaw) * edge * flat,
                scale: Vec3::new(size, size * 0.7, size),
            })
        }
    }
}

/// A flat sheet over a strip from its root to its far end, `half_width` to each side.
fn sheet(strip: Strip, half_width: f32, lift: f32) -> Transform {
    Transform::from_xyz(0.0, lift, strip.length * 0.5).with_scale(Vec3::new(
        half_width * 2.0,
        SHEET_HEIGHT,
        strip.length,
    ))
}

/// The largest kite that lies in a cone, at `share` of its size: its point on the apex, its
/// widest corners on the two edges and its far corners on the arc.
fn wedge(radius: f32, half_angle: f32, share: f32, lift: f32) -> Option<Transform> {
    if !(half_angle > 0.0 && half_angle < FRAC_PI_2) {
        return None;
    }
    // Width of the mesh for each unit of its length, so that its widest corners lie on
    // the edges; then the length at which the corner farthest from the apex is on the arc.
    let spread = half_angle.tan() * KITE_SHOULDER.x / KITE_SHOULDER.y;
    let farthest = (KITE_TOP * Vec2::new(1.0, spread))
        .length()
        .max(KITE_SHOULDER.x / half_angle.cos());
    let along = radius * share / farthest;
    Some(Transform {
        translation: Vec3::new(0.0, lift, along * UNIT_RADIUS),
        rotation: Quat::from_mat3(&Mat3::from_cols(Vec3::Z, Vec3::X, Vec3::Y)),
        scale: Vec3::new(along, along * spread, 1.0),
    })
}

/// The translucent layer over the inside of the received shape, and the same layer at
/// `share` of its size for a read-out that grows to the boundary: a circle from its centre,
/// a strip across its width only (rule P-2), a cone from its apex. A line has no inside.
fn area(seen: &Seen, share: f32, lift: f32) -> Option<Transform> {
    match seen.shape {
        GeoShape::Ring { .. } | GeoShape::Pentagon { .. } => {
            Some(flat_circle(seen.circle()? * share, lift))
        }
        GeoShape::Capsule { .. } | GeoShape::Lane { .. } => {
            let strip = seen.strip()?;
            Some(sheet(strip, strip.radius * share, lift))
        }
        GeoShape::Sector {
            radius, half_angle, ..
        } => wedge(radius, half_angle, share, lift),
        GeoShape::Segment { .. } | GeoShape::None => None,
    }
}

/// The boundary part `index` of the received shape, posed from replicated fields alone. No
/// part of it lies outside the shape: a line is drawn on its inner side.
fn boundary(index: u8, seen: &Seen) -> Option<Transform> {
    let lift = Vec3::Y * BOUNDARY_LIFT;
    let ground = |point: Vec2| {
        let at = seen.local(point);
        Vec3::new(at.x, BOUNDARY_LIFT, at.y)
    };
    let line = |size: f32| (size * LINE_SHARE).clamp(LINE_MIN, LINE_MAX);
    match seen.shape {
        GeoShape::Ring { radius, .. } => Some(flat_circle(radius, BOUNDARY_LIFT)),
        GeoShape::Capsule { .. } | GeoShape::Lane { .. } => {
            let strip = seen.strip()?;
            if index < 2 {
                // The two edges, at the replicated half-width.
                let side = if index == 0 { 1.0 } else { -1.0 };
                let from = lift + Vec3::X * side * (strip.radius - strip.line * 0.5);
                bar(from, from + Vec3::Z * strip.length, strip.line, LINE_HEIGHT)
            } else {
                // The two round ends: half rings around the ends of the segment.
                let (at, out) = if index == 2 {
                    (0.0, -1.0)
                } else {
                    (strip.length, 1.0)
                };
                let across = strip.radius / UNIT_RADIUS;
                Some(Transform {
                    translation: lift + Vec3::Z * at,
                    rotation: Quat::from_mat3(&Mat3::from_cols(
                        Vec3::Z * out,
                        Vec3::X * out,
                        Vec3::Y,
                    )),
                    scale: Vec3::new(across, across, 1.0),
                })
            }
        }
        GeoShape::Sector {
            radius, half_angle, ..
        } => {
            // The apex is the root and the axis is +Z; an angle turns from +Z toward +X.
            let line = line(radius);
            let toward = |angle: f32| Vec3::new(angle.sin(), 0.0, angle.cos());
            if index < 2 {
                // An edge, from the apex to the arc.
                let angle = if index == 0 { -half_angle } else { half_angle };
                let inward = Vec3::new(-angle.cos(), 0.0, angle.sin()) * angle.signum();
                // In a narrow cone the inner side starts where it clears the other edge.
                let start = if 2.0 * half_angle < FRAC_PI_2 {
                    line / (2.0 * half_angle).tan()
                } else {
                    0.0
                };
                let end = (radius * radius - line * line).max(0.0).sqrt();
                let from = lift + inward * (line * 0.5);
                bar(
                    from + toward(angle) * start,
                    from + toward(angle) * end,
                    line,
                    LINE_HEIGHT,
                )
            } else {
                // A chord of the arc between two of its points.
                let chord = index - 2;
                let step = 2.0 * half_angle / f32::from(SECTOR_CHORDS);
                let (first, second) = (
                    -half_angle + step * f32::from(chord),
                    -half_angle + step * f32::from(chord + 1),
                );
                let (from, to) = (toward(first) * radius, toward(second) * radius);
                let along = (to - from).try_normalize()?;
                // The chords at the two edges end where their inner side meets the edge.
                let trim = line * (step * 0.5).tan();
                let from = from + along * if chord == 0 { trim } else { 0.0 };
                let to = to
                    - along
                        * if chord + 1 == SECTOR_CHORDS {
                            trim
                        } else {
                            0.0
                        };
                let inward = lift - toward((first + second) * 0.5) * (line * 0.5);
                bar(inward + from, inward + to, line, LINE_HEIGHT)
            }
        }
        // The plane of a wall, or what the fog left of the axis of a cone (rule F): one
        // plain bar with two equal ends.
        GeoShape::Segment { from, to } => bar(
            ground(from),
            ground(to),
            line(from.distance(to) * 0.5),
            LINE_HEIGHT,
        ),
        GeoShape::Pentagon { center, radius } => {
            // Side `i` runs between corners `i` and `i + 1`; it has a low bar as wide as
            // the band the server tests (rule E-9) and a rail above it. A consumed side
            // has neither.
            let side = index % 5;
            if seen.effect.consumed_segments & (1 << side) != 0 {
                return None;
            }
            let corner = |corner: u8| {
                ground(center + Vec2::from_angle(TAU * f32::from(corner) / 5.0) * radius)
            };
            let (from, to) = (corner(side), corner(side + 1));
            if index < 5 {
                bar(from, to, geometry::CAGE_BAR_HALF_WIDTH * 2.0, LINE_HEIGHT)
            } else {
                let raise = Vec3::Y * (CAGE_RAIL_HEIGHT - BOUNDARY_LIFT);
                bar(from + raise, to + raise, CAGE_RAIL, CAGE_RAIL)
            }
        }
        GeoShape::None => None,
    }
}

/// A marker part: the read-out of replicated state.
fn marker(slot: &PartSlot, index: u8, seen: &Seen) -> Option<Transform> {
    match slot.plan.marker {
        Marker::None => None,
        Marker::RemainingRing => {
            let peak = seen.peak_remaining_secs;
            let left = if peak > 0.0 {
                (seen.view.remaining / peak).clamp(0.0, 1.0)
            } else {
                0.0
            };
            Some(flat_circle(
                seen.circle()? * MARKER_REACH * left,
                MARKER_LIFT,
            ))
            .filter(|_| left > 0.0)
        }
        Marker::FillToEdge => {
            // It ends at the inner side of the boundary line, where there is one.
            let progress = seen.view.progress;
            let reach = match seen.strip() {
                Some(strip) => 1.0 - strip.line / strip.radius,
                None if matches!(seen.shape, GeoShape::Sector { .. }) => 1.0,
                None => MARKER_REACH,
            };
            area(seen, reach * progress, MARKER_LIFT)
                .filter(|_| seen.view.stage == Stage::Telegraph && progress > 0.0)
        }
        Marker::ArmingPips => {
            let radius = seen.circle()?;
            // Three pips on the camera side of the circle; they swell when the effect arms.
            let angle = (f32::from(index) - 1.0) * 0.6;
            let size = (radius * 0.22).clamp(0.1, 0.3) * if seen.effect.armed { 1.0 } else { 0.6 };
            let at = Vec3::new(angle.sin(), 0.0, angle.cos()) * radius * 0.78;
            Some(
                Transform::from_translation(at + Vec3::Y * (MARKER_LIFT + size * 0.5))
                    .with_scale(Vec3::splat(size)),
            )
        }
        Marker::OwnerTether => {
            let owner = seen.owner?;
            let at = seen.local(owner.xz());
            bar(
                Vec3::Y * height(slot.plan.altitude),
                Vec3::new(at.x, owner.y - seen.ground, at.y),
                0.06,
                0.06,
            )
        }
    }
}

/// The pose of one part relative to the root of its body, in the frame the client holds
/// `seen`. A hidden part has the default pose.
pub(crate) fn part_pose(slot: &PartSlot, seen: &Seen) -> (Transform, Visibility) {
    let pose = match slot.role {
        Role::Boundary(index) => boundary(index, seen),
        Role::Fill => area(seen, 1.0, FILL_LIFT),
        Role::Core => centre(slot, seen).and_then(|at| authored(slot, seen, at, 0.0)),
        Role::Shell => {
            // A flat shell lies just behind a core of its own kind along its normal.
            let behind = slot
                .mesh
                .filter(|mesh| planar(*mesh) && slot.plan.shell_like_core)
                .map_or(Vec3::ZERO, |_| {
                    let mut nudge = Vec3::ZERO;
                    nudge[normal_axis(slot.size, slot.plan.stands())] = -SHELL_NUDGE;
                    nudge
                });
            centre(slot, seen).and_then(|at| authored(slot, seen, at + behind, 0.0))
        }
        Role::Satellite(index) => {
            copy_place(slot, index, seen).and_then(|(at, yaw)| authored(slot, seen, at, yaw))
        }
        Role::Trail(index) => trail(slot, index, seen),
        Role::Marker(index) => marker(slot, index, seen),
        Role::Model => slot.plan.model.map(|model| {
            let (scale, _) = model_fit(model, slot, seen.effect.radius);
            Transform::from_xyz(0.0, height(slot.plan.altitude), 0.0).with_scale(Vec3::splat(scale))
        }),
    };
    match pose {
        Some(pose) if pose.is_finite() && pose.scale.min_element() > 0.0 => {
            (pose, Visibility::Inherited)
        }
        _ => (Transform::default(), Visibility::Hidden),
    }
}

/// The material of one part in this frame; `None` for the packaged model, which brings its
/// own. Rule E-11: chain links and the tether are matter, not light.
pub(crate) fn part_paint(slot: &PartSlot, seen: &Seen) -> Option<Paint> {
    Some(match slot.role {
        // The upper rails of a cage are its look; the bars on the ground say whose it is.
        Role::Boundary(index) if index >= 5 && matches!(seen.shape, GeoShape::Pentagon { .. }) => {
            Paint::Slot(PaletteSlot::Primary)
        }
        Role::Boundary(_) => Paint::Team,
        Role::Fill if seen.view.stage == Stage::Telegraph => Paint::FillDim,
        Role::Fill => Paint::Fill,
        Role::Core | Role::Shell | Role::Satellite(_) => Paint::Slot(slot.slot),
        Role::Trail(_) if slot.plan.trail == Trail::Links => Paint::Slot(PaletteSlot::Secondary),
        Role::Trail(_) => Paint::Slot(PaletteSlot::Primary),
        // Over a strip and in a cone the read-out is a second dim layer that spreads over
        // the first: a telegraph grows wider, never brighter.
        Role::Marker(_)
            if slot.plan.marker == Marker::FillToEdge
                && matches!(
                    seen.shape,
                    GeoShape::Capsule { .. } | GeoShape::Lane { .. } | GeoShape::Sector { .. }
                ) =>
        {
            Paint::FillDim
        }
        Role::Marker(_) => Paint::Slot(match slot.plan.marker {
            Marker::OwnerTether => PaletteSlot::Secondary,
            // Dark until the effect arms.
            Marker::ArmingPips if !seen.effect.armed => PaletteSlot::Secondary,
            _ => PaletteSlot::Primary,
        }),
        Role::Model => return None,
    })
}

/// One drawn body as the part budget counts it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Load<K> {
    pub key: K,
    /// Distance from the camera.
    pub distance: f32,
    /// Visible boundary, fill and marker parts.
    pub kept: usize,
    /// Visible authored parts.
    pub authored: usize,
}

/// The bodies that hide their authored parts so that at most `budget` parts stay visible,
/// the farthest first. `fixed` counts visible parts no body can give up. Engine parts are
/// never hidden, so the sum can stay above the budget when they alone exceed it.
pub(crate) fn over_budget<K: Copy>(loads: &[Load<K>], fixed: usize, budget: usize) -> Vec<K> {
    let mut total = fixed
        + loads
            .iter()
            .map(|load| load.kept + load.authored)
            .sum::<usize>();
    let mut farthest: Vec<&Load<K>> = loads.iter().filter(|load| load.authored > 0).collect();
    farthest.sort_by(|a, b| b.distance.total_cmp(&a.distance));
    let mut hidden = Vec::new();
    for load in farthest {
        if total <= budget {
            break;
        }
        total -= load.authored;
        hidden.push(load.key);
    }
    hidden
}

#[cfg(test)]
mod tests;
