//! Versioned, local-only combat cosmetics. No field enters the simulation.
use std::collections::HashMap;
use std::f32::consts::PI;
use std::sync::OnceLock;

use crate::skill_presentation::bodies::{self, PartMesh};
use crate::skill_presentation::vocab::{ProjectileForm, ProjectilePresentation, Silhouette};
use bevy::{
    asset::{AssetLoader, LoadContext, io::Reader},
    prelude::*,
};
use serde::Deserialize;
use shared::{HeroClass, combat::ProjectileStyle};

// Deliberately independent of the editable packaged file: even malformed user
// JSON must leave usable offline class visuals after recompiling the client.
const BUILT_INS: &str = r#"{"schema_version":1,"profiles":{
"standard":{"shape":"bolt","color":[0.8,0.9,1,1]},
"ranger_arrow":{"shape":"arrow","color":[0.65,1,0.72,1]},
"mage_arcane":{"shape":"arcane","color":[0.68,0.42,1,1]},
"cleric_holy":{"shape":"holy","color":[1,0.84,0.35,1]},
"warrior_crescent":{"shape":"crescent","color":[1,0.63,0.3,1]},
"warden_claw":{"shape":"crescent","color":[0.55,0.95,0.4,1],"scale":0.9},
"caster_bolt":{"shape":"arcane","color":[0.4,0.95,1,1],"scale":0.7},
"wild_bullet":{"shape":"bolt","color":[1,0.85,0.25,1],"scale":0.5},
"wild_rocket":{"shape":"bolt","color":[1,0.3,0.05,1],"scale":1.1,"model":{"path":"weapons/wild-rocket.glb","scale":1.4}},
"tower_bolt":{"shape":"bolt","color":[1,0.43,0.24,1],"scale":1.4}},
"defaults":{"bullet":"wild_bullet","rocket":"wild_rocket","standard":"standard","arrow":"ranger_arrow","arcane":"mage_arcane","holy":"cleric_holy","crescent":"warrior_crescent","claw":"warden_claw","caster_bolt":"caster_bolt","tower_bolt":"tower_bolt"},
"classes":{"ranger":{"default":"ranger_arrow"},"mage":{"default":"mage_arcane"},"cleric":{"default":"cleric_holy"},"warrior":{"default":"warrior_crescent"},"warden":{"default":"warden_claw"}}}"#;
const CONFIG_PATH: &str = "config/combat_visuals.json";

/// Render container whose visible drawable descendants represent this network root.
#[derive(Component, Clone, Copy)]
pub(crate) struct ProjectilePresentationRoot {
    #[cfg(any(test, feature = "qa"))]
    pub owner: Entity,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProjectileShape {
    Arrow,
    Arcane,
    Holy,
    Crescent,
    Bolt,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrailSettings {
    pub seconds: f32,
    pub width: f32,
    pub samples: usize,
}

impl Default for TrailSettings {
    fn default() -> Self {
        Self {
            seconds: 0.18,
            width: 0.08,
            samples: 8,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImpactSettings {
    pub color: [f32; 4],
    pub scale: f32,
    pub lifetime: f32,
}

impl Default for ImpactSettings {
    fn default() -> Self {
        Self {
            color: [1.0, 0.8, 0.5, 1.0],
            scale: 0.8,
            lifetime: 0.3,
        }
    }
}

impl ImpactSettings {
    pub fn color(&self) -> Color {
        Color::srgba(self.color[0], self.color[1], self.color[2], self.color[3])
    }
}

fn unit_scale() -> f32 {
    1.0
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectileModel {
    pub path: String,
    #[serde(default)]
    pub scene: usize,
    #[serde(default = "unit_scale")]
    pub scale: f32,
    #[serde(default)]
    pub rotation_degrees: [f32; 3],
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectileSprite {
    pub path: String,
    pub frame_size: [u32; 2],
    pub columns: u32,
    pub rows: u32,
    #[serde(default)]
    pub first_frame: usize,
    pub frames: usize,
    pub fps: f32,
    pub world_height: f32,
}

impl ProjectileSprite {
    pub fn matches_image(&self, image: &Image) -> bool {
        image.width() == self.frame_size[0] * self.columns
            && image.height() == self.frame_size[1] * self.rows
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CombatVisualProfile {
    #[serde(skip)]
    pub id: String,
    pub shape: ProjectileShape,
    pub color: [f32; 4],
    #[serde(default = "unit_scale")]
    pub scale: f32,
    #[serde(default)]
    pub trail: TrailSettings,
    #[serde(default)]
    pub impact: ImpactSettings,
    #[serde(default)]
    pub model: Option<ProjectileModel>,
    #[serde(default)]
    pub sprite: Option<ProjectileSprite>,
    /// 3D body built from the shared silhouettes; `shape` stays the 2D look and the 3D
    /// look when this is absent.
    #[serde(default)]
    pub form: Option<ProjectileForm>,
    /// Mesh of the form; the form's default when absent.
    #[serde(default)]
    pub silhouette: Option<Silhouette>,
    #[serde(default)]
    pub presentation: ProjectilePresentation,
}

impl CombatVisualProfile {
    pub fn color(&self) -> Color {
        Color::srgba(self.color[0], self.color[1], self.color[2], self.color[3])
    }

    /// What stands for a projectile of this profile in 3D. Without the shared mesh
    /// `library` every profile is its `shape`. A form is drawn instead of a packaged model.
    /// A melee contact is the basic attack of a hero whose class the client knows
    /// (`known_basic`): only that flight is short enough for a streak to stand for it, so any
    /// other projectile that resolves to such a profile (an ability, or a style default
    /// taken for an unknown owner) keeps the thrown body.
    pub(crate) fn flight_body(&self, library: bool, known_basic: bool) -> FlightBody {
        match (self.presentation, self.form) {
            _ if !library => FlightBody::Shape,
            (ProjectilePresentation::MeleeContact, _) if known_basic => FlightBody::Reach,
            (ProjectilePresentation::MeleeContact, _) | (_, None) => FlightBody::Shape,
            (_, Some(form)) => {
                FlightBody::Form(form, self.silhouette.unwrap_or(default_silhouette(form)))
            }
        }
    }

    /// Whether a projectile drawn as `body` leaves glow puffs behind it: only a thrown
    /// `shape` body of a magic wire style does. A form carries its own wake, and a wave or
    /// a melee contact is no missile in either render mode.
    pub(crate) fn puffs(&self, body: FlightBody, style: ProjectileStyle) -> bool {
        body == FlightBody::Shape
            && self.presentation == ProjectilePresentation::Projectile
            && matches!(style, ProjectileStyle::Arcane | ProjectileStyle::Holy)
    }

    /// Whether the gizmo trail is drawn: only for the `projectile` presentation, and not
    /// for a profile whose trail lasts no time at all.
    pub(crate) fn trails(&self) -> bool {
        self.trail.seconds > 0.0 && self.presentation == ProjectilePresentation::Projectile
    }

    /// Whether `body` slides along the ground under the replicated position instead of
    /// flying at its height.
    pub(crate) fn hugs_ground(&self, body: FlightBody) -> bool {
        match body {
            FlightBody::Reach => true,
            FlightBody::Form(..) => self.presentation == ProjectilePresentation::Wave,
            FlightBody::Shape => false,
        }
    }
}

/// Whether a projectile is the basic attack of a hero whose class the client knows.
pub(crate) fn known_basic(class: Option<HeroClass>, action_slot: Option<u8>) -> bool {
    class.is_some() && action_key(action_slot) == "basic"
}

/// The 3D body of a projectile in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FlightBody {
    /// The thrown body of the profile's `shape`, or its packaged model once that loaded.
    Shape,
    /// A form built from one shared silhouette.
    Form(ProjectileForm, Silhouette),
    /// The engine's reach streak of a melee contact; not authorable.
    Reach,
}

impl FlightBody {
    /// The mesh parts of the body, the team cue last; a `shape` body is not in this table.
    pub(crate) fn parts(self) -> Vec<FormPart> {
        let parts = match self {
            Self::Shape => Vec::new(),
            Self::Form(form, silhouette) => form_parts(form, silhouette),
            Self::Reach => reach_streak(),
        };
        debug_assert!(parts.len() <= FORM_PARTS);
        parts
    }
}

/// Widest a ground wave may be after scale: one hero (`2 * PLAYER_TARGET_RADIUS` is 1.24).
/// The projectile strikes one unit, so a wider wave would read as a sweep.
pub(crate) const WAVE_WIDTH: f32 = 1.3;
/// Widest a volley may fan out after scale, for the same reason.
pub(crate) const VOLLEY_WIDTH: f32 = 0.6;
/// Mesh parts of one projectile body, its team cue included.
pub(crate) const FORM_PARTS: usize = 6;
/// Slack of the width comparison: a profile authored exactly at a cap is legal.
const WIDTH_SLACK: f32 = 1e-4;

/// What a part of a projectile body is painted with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FormPaint {
    /// The profile colour as light.
    Tint,
    /// The deep shade of the profile colour: a rim or an afterimage that sets the light
    /// parts off against pale ground.
    Echo,
    /// White-hot.
    Core,
    /// The colour of the owner's team.
    Team,
}

/// How a part of a projectile body moves; every motion is a change of its transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum FormMotion {
    Still,
    /// Turns about the vertical axis, in radians per second.
    Spin(f32),
    /// Turns end over end about the lateral axis, the top going forward.
    Tumble(f32),
    /// Circles the heading axis at `radius`, starting at the angle `phase`.
    Wind {
        radius: f32,
        rate: f32,
        phase: f32,
    },
    /// Swells and shrinks by `depth` of its size.
    Pulse {
        depth: f32,
        rate: f32,
    },
}

/// One mesh of a projectile body, in the frame of the projectile: x to its side, y up,
/// z along its heading, in units at profile scale 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FormPart {
    pub mesh: Silhouette,
    pub paint: FormPaint,
    pub at: Vec3,
    /// Extents along the three axes. A flat mesh faces its thinnest extent and points
    /// along the heading, or upward when it faces the heading (`bodies::lay`).
    pub size: Vec3,
    /// Turn about the vertical axis at rest.
    pub yaw: f32,
    pub motion: FormMotion,
}

impl FormPart {
    /// Rotation and scale of the mesh at rest.
    fn rest(&self) -> (Quat, Vec3) {
        let mesh = PartMesh::Silhouette(self.mesh);
        let (lay, axes) = bodies::lay(mesh, self.size, false);
        (
            Quat::from_rotation_y(self.yaw) * lay,
            bodies::mesh_scale(mesh, self.size, axes),
        )
    }

    /// Where the part is `secs` into the flight.
    pub(crate) fn pose(&self, secs: f32) -> Transform {
        let (rest, scale) = self.rest();
        let (offset, turn, swell) = match self.motion {
            FormMotion::Still => (Vec3::ZERO, Quat::IDENTITY, 1.0),
            FormMotion::Spin(rate) => (Vec3::ZERO, Quat::from_rotation_y(rate * secs), 1.0),
            FormMotion::Tumble(rate) => (Vec3::ZERO, Quat::from_rotation_x(rate * secs), 1.0),
            FormMotion::Wind {
                radius,
                rate,
                phase,
            } => {
                let angle = phase + rate * secs;
                (
                    Vec3::new(angle.cos(), angle.sin(), 0.0) * radius,
                    Quat::IDENTITY,
                    1.0,
                )
            }
            FormMotion::Pulse { depth, rate } => (
                Vec3::ZERO,
                Quat::IDENTITY,
                1.0 + depth * (rate * secs).sin(),
            ),
        };
        Transform {
            translation: self.at + offset,
            rotation: turn * rest,
            scale: scale * swell,
        }
    }

    /// The farthest any point of the mesh gets from the heading axis to either side, at
    /// any moment of its motion.
    fn lateral_reach(&self) -> f32 {
        let (rest, scale) = self.rest();
        let at = self.at.x;
        outline(self.mesh)
            .iter()
            .map(|vertex| {
                let point = rest * (*vertex * scale);
                match self.motion {
                    FormMotion::Still | FormMotion::Tumble(_) => (at + point.x).abs(),
                    FormMotion::Spin(_) => at.abs() + point.x.hypot(point.z),
                    FormMotion::Wind { radius, .. } => (at + point.x).abs() + radius,
                    FormMotion::Pulse { depth, .. } => (at + point.x * (1.0 + depth))
                        .abs()
                        .max((at + point.x * (1.0 - depth)).abs()),
                }
            })
            .fold(0.0, f32::max)
    }
}

/// The vertices of a shared silhouette: the form table is measured on the meshes it draws.
fn outline(mesh: Silhouette) -> &'static [Vec3] {
    static OUTLINES: OnceLock<Vec<Vec<Vec3>>> = OnceLock::new();
    &OUTLINES.get_or_init(|| {
        Silhouette::ALL
            .iter()
            .map(|mesh| {
                bodies::silhouette_mesh(*mesh)
                    .attribute(Mesh::ATTRIBUTE_POSITION)
                    .and_then(|values| values.as_float3())
                    .map(|points| {
                        points
                            .iter()
                            .map(|point| Vec3::from_array(*point))
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .collect()
    })[mesh as usize]
}

/// The silhouette of a form whose profile names none.
pub(crate) const fn default_silhouette(form: ProjectileForm) -> Silhouette {
    match form {
        ProjectileForm::Dart => Silhouette::Shard,
        ProjectileForm::Comet | ProjectileForm::TwinHelix => Silhouette::Ball,
        ProjectileForm::DiscSkim => Silhouette::Kite,
        ProjectileForm::Tumbler => Silhouette::Diamond,
        ProjectileForm::Wavefront => Silhouette::Crescent,
        ProjectileForm::Volley => Silhouette::Claw,
    }
}

/// Size of a flat part that lies level: `lateral` wide and `heading` long.
fn level(lateral: f32, heading: f32) -> Vec3 {
    Vec3::new(lateral, bodies::TORUS_TUBE * lateral.min(heading), heading)
}

fn team_cue(at: Vec3) -> FormPart {
    FormPart {
        mesh: Silhouette::Ball,
        paint: FormPaint::Team,
        at,
        size: Vec3::splat(0.2),
        yaw: 0.0,
        motion: FormMotion::Still,
    }
}

/// The mesh parts of a form at profile scale 1: at most `FORM_PARTS`, the small team cue
/// every projectile carries last. A flat silhouette lies level, so that the camera above
/// sees its face; a solid one keeps its volume. The light of a form stands on a larger
/// part in its deep shade, which keeps the silhouette readable on pale ground.
pub(crate) fn form_parts(form: ProjectileForm, mesh: Silhouette) -> Vec<FormPart> {
    use FormMotion::{Pulse, Spin, Still, Tumble, Wind};
    use FormPaint::{Core, Echo, Tint};
    let flat = bodies::planar(PartMesh::Silhouette(mesh));
    // The size of the silhouette itself; a solid is `height` times as tall as it is wide.
    let body = |lateral: f32, heading: f32, height: f32| {
        if flat {
            level(lateral, heading)
        } else {
            Vec3::new(lateral, lateral * height, heading)
        }
    };
    let part = |mesh, paint, at, size, motion| FormPart {
        mesh,
        paint,
        at,
        size,
        yaw: 0.0,
        motion,
    };
    // A flat mesh points along its own x axis and need not be centred on its origin:
    // `foot` trails, `tip` leads and `middle` is half-way.
    let (foot, tip) = if flat {
        outline(mesh)
            .iter()
            .fold((f32::MAX, f32::MIN), |(low, high), vertex| {
                (low.min(vertex.x), high.max(vertex.x))
            })
    } else {
        (-0.5, 0.5)
    };
    let middle = if flat { (foot + tip) / 2.0 } else { 0.0 };
    // A flat light part lies just above its rim; a solid one stands inside it.
    let over = |z: f32| Vec3::new(0.0, if flat { 0.04 } else { 0.0 }, z);
    let ahead = |z: f32| Vec3::new(0.0, 0.0, z);
    match form {
        // A long head in its rim, on a dark shaft.
        ProjectileForm::Dart => vec![
            part(
                mesh,
                Echo,
                ahead(0.3 - middle * 2.3),
                body(0.84, 2.3, 1.0),
                Still,
            ),
            part(
                mesh,
                Tint,
                over(0.34 - middle * 1.75),
                body(0.56, 1.75, 1.0),
                Still,
            ),
            part(
                Silhouette::Block,
                Echo,
                ahead(-0.5),
                Vec3::new(0.14, 0.14, 2.8),
                Still,
            ),
            team_cue(Vec3::new(0.0, 0.16, -1.75)),
        ],
        // Every copy points back along the path, so a teardrop flies bulb first. The
        // afterimages cool from the light of the head to its deep shade.
        ProjectileForm::Comet => {
            let pulse = Pulse {
                depth: 0.12,
                rate: 19.0,
            };
            let copy = |paint, at: Vec3, size: f32, motion| FormPart {
                yaw: PI,
                ..part(
                    mesh,
                    paint,
                    at + Vec3::Z * middle * size,
                    body(size, size, 1.0),
                    motion,
                )
            };
            vec![
                copy(Echo, ahead(0.0), 1.42, pulse),
                copy(Tint, over(0.0), 1.05, pulse),
                copy(Tint, over(-1.05), 0.8, Still),
                copy(Echo, ahead(-1.8), 0.6, Still),
                copy(Echo, ahead(-2.35), 0.4, Still),
                team_cue(Vec3::new(0.0, 0.3, -0.6)),
            ]
        }
        // A deep rim under a bright face, turning as one plate.
        ProjectileForm::DiscSkim => vec![
            part(mesh, Echo, Vec3::ZERO, body(1.0, 1.0, 0.3), Spin(9.0)),
            part(
                mesh,
                Tint,
                Vec3::new(0.0, 0.06, 0.0),
                body(0.66, 0.66, 0.3),
                Spin(9.0),
            ),
            team_cue(Vec3::new(0.0, 0.14, -0.62)),
        ],
        // A flat silhouette flips; its deep copy stands across it, so that the pair is
        // never seen edge-on. A solid one wears a band instead.
        ProjectileForm::Tumbler => {
            let tumble = Tumble(11.0);
            let (plate, second, core) = if flat {
                (
                    level(1.1, 1.7),
                    Vec3::new(bodies::TORUS_TUBE * 1.1, 1.1, 1.7),
                    part(Silhouette::Ball, Core, Vec3::ZERO, Vec3::splat(0.32), Still),
                )
            } else {
                (
                    Vec3::new(0.75, 0.75, 1.4),
                    Vec3::new(0.9, 0.9, 0.36),
                    part(
                        Silhouette::Block,
                        Core,
                        Vec3::ZERO,
                        Vec3::new(0.24, 0.24, 1.65),
                        tumble,
                    ),
                )
            };
            vec![
                part(mesh, Tint, Vec3::ZERO, plate, tumble),
                part(mesh, Echo, Vec3::ZERO, second, tumble),
                core,
                team_cue(Vec3::new(0.0, 0.1, -1.05)),
            ]
        }
        // Across the heading on the ground: a deep rim, a bright face and a low crest
        // standing over them. Every part is as wide as the rim or narrower.
        ProjectileForm::Wavefront => {
            let mut parts = if flat {
                // The leading edges lie just ahead of the replicated position, and the
                // crest stands on its foot.
                let rise = 0.7 / (tip - foot);
                vec![
                    part(mesh, Echo, ahead(0.35 - tip * 1.9), level(1.0, 1.9), Still),
                    part(
                        mesh,
                        Tint,
                        Vec3::new(0.0, 0.04, 0.3 - tip * 1.45),
                        level(0.78, 1.45),
                        Still,
                    ),
                    part(
                        mesh,
                        Tint,
                        Vec3::new(0.0, 0.03 - foot * rise, -0.05),
                        Vec3::new(1.0, rise, bodies::TORUS_TUBE * rise.min(1.0)),
                        Still,
                    ),
                ]
            } else {
                vec![
                    part(
                        mesh,
                        Echo,
                        Vec3::new(0.0, 0.19, 0.0),
                        Vec3::new(1.0, 0.38, 0.6),
                        Still,
                    ),
                    part(
                        mesh,
                        Tint,
                        Vec3::new(0.0, 0.44, 0.06),
                        Vec3::new(0.8, 0.2, 0.4),
                        Still,
                    ),
                    part(
                        mesh,
                        Tint,
                        Vec3::new(0.0, 0.1, -0.62),
                        Vec3::new(0.64, 0.2, 0.34),
                        Still,
                    ),
                ]
            };
            parts.push(team_cue(Vec3::new(0.0, 0.2, -1.0)));
            parts
        }
        // Three copies in echelon: the light one leads, the two in its deep shade
        // follow, turned a little outward.
        ProjectileForm::Volley => {
            let copy = |paint, side: f32, back: f32| FormPart {
                yaw: 0.06 * side,
                ..part(
                    mesh,
                    paint,
                    Vec3::new(0.07 * side, 0.0, -back),
                    body(0.7, 1.5, 1.0),
                    Still,
                )
            };
            vec![
                copy(Tint, 0.0, -0.6),
                copy(Echo, -1.0, 0.15),
                copy(Echo, 1.0, 0.9),
                team_cue(Vec3::new(0.0, 0.14, -1.75)),
            ]
        }
        // A bright strand and a deep one wind around a thread of light.
        ProjectileForm::TwinHelix => {
            let strand = |paint, phase| {
                part(
                    mesh,
                    paint,
                    Vec3::ZERO,
                    body(0.7, 1.05, 1.0),
                    Wind {
                        radius: 0.46,
                        rate: 11.0,
                        phase,
                    },
                )
            };
            vec![
                strand(Tint, 0.0),
                strand(Echo, PI),
                part(
                    Silhouette::Block,
                    Tint,
                    ahead(-0.2),
                    Vec3::new(0.1, 0.1, 1.9),
                    Still,
                ),
                team_cue(Vec3::new(0.0, 0.12, -1.2)),
            ]
        }
    }
}

/// The body of a melee contact: two low slivers on the ground behind the replicated
/// position, the light one on its deep rim. No blade is thrown, and the hero who swings
/// is the team read.
pub(crate) fn reach_streak() -> Vec<FormPart> {
    let sliver = |paint, lift: f32, lateral: f32, heading: f32| FormPart {
        mesh: Silhouette::Diamond,
        paint,
        at: Vec3::new(0.0, lift, -0.8),
        size: level(lateral, heading),
        yaw: 0.0,
        motion: FormMotion::Still,
    };
    vec![
        sliver(FormPaint::Echo, 0.0, 0.8, 2.6),
        sliver(FormPaint::Tint, 0.04, 0.48, 2.0),
    ]
}

/// The full width of a form at profile scale 1, measured across the heading: twice the
/// farthest any part but the team cue reaches to either side while it moves. It is a fact
/// of the form table and of the shared meshes; a profile cannot author it.
pub(crate) fn form_lateral_extent(form: ProjectileForm, silhouette: Silhouette) -> f32 {
    2.0 * form_parts(form, silhouette)
        .iter()
        .filter(|part| part.paint != FormPaint::Team)
        .map(FormPart::lateral_reach)
        .fold(0.0, f32::max)
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AnimationAliases {
    pub idle: Vec<String>,
    pub walk: Vec<String>,
    pub run: Vec<String>,
    pub attack: Vec<String>,
    pub cast: Vec<String>,
    pub death: Vec<String>,
}

type ActionProfiles = HashMap<String, String>;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryConfig {
    schema_version: u32,
    #[serde(default)]
    profiles: HashMap<String, CombatVisualProfile>,
    #[serde(default)]
    defaults: HashMap<String, String>,
    #[serde(default)]
    classes: HashMap<String, ActionProfiles>,
    #[serde(default)]
    avatar_overrides: HashMap<String, ActionProfiles>,
    #[serde(default)]
    sprite_overrides: HashMap<String, ActionProfiles>,
    #[serde(default)]
    animation_aliases: HashMap<String, AnimationAliases>,
}

#[derive(Resource, Clone, Debug)]
pub struct CombatVisualRegistry {
    config: RegistryConfig,
    revision: u64,
}

impl Default for CombatVisualRegistry {
    fn default() -> Self {
        let mut registry = Self {
            config: serde_json::from_str(BUILT_INS).expect("embedded combat cosmetics JSON"),
            revision: 0,
        };
        registry
            .validate()
            .expect("embedded combat cosmetics are valid");
        registry
    }
}

fn in_range(value: f32, min: f32, max: f32) -> bool {
    value.is_finite() && (min..=max).contains(&value)
}
fn valid_color(color: [f32; 4]) -> bool {
    color.into_iter().all(|value| in_range(value, 0.0, 1.0)) && color[3] >= 0.25
}
fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 96 && !key.chars().any(char::is_control)
}

/// Asset-root-relative paths only: no URLs, labels, traversal or drive prefixes.
pub fn safe_asset_path(path: &str, extension: &str) -> bool {
    path.len() <= 200
        && path.ends_with(extension)
        && !path.starts_with('/')
        && path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/_.-".contains(&byte))
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn style_key(style: ProjectileStyle) -> &'static str {
    match style {
        ProjectileStyle::Standard => "standard",
        ProjectileStyle::Bullet => "bullet",
        ProjectileStyle::Rocket => "rocket",
        ProjectileStyle::Arrow => "arrow",
        ProjectileStyle::Arcane => "arcane",
        ProjectileStyle::Holy => "holy",
        ProjectileStyle::Crescent => "crescent",
        ProjectileStyle::Claw => "claw",
        ProjectileStyle::CasterBolt => "caster_bolt",
        ProjectileStyle::TowerBolt => "tower_bolt",
    }
}

fn action_key(slot: Option<u8>) -> &'static str {
    match slot {
        None | Some(255) => "basic",
        Some(0) => "q",
        Some(1) => "w",
        Some(2) => "e",
        Some(3) => "r",
        _ => "default",
    }
}

impl CombatVisualRegistry {
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Partial manifests extend the embedded defaults; any invalid field rejects the manifest.
    pub fn from_json(json: &str) -> Result<Self, String> {
        if json.len() > 256 * 1024 {
            return Err("cosmetic manifest exceeds 256 KiB".into());
        }
        let custom: RegistryConfig =
            serde_json::from_str(json).map_err(|error| error.to_string())?;
        if custom.schema_version != 1 {
            return Err("unsupported cosmetic schema_version".into());
        }
        let mut registry = Self::default();
        registry.config.profiles.extend(custom.profiles);
        registry.config.defaults.extend(custom.defaults);
        registry.config.classes.extend(custom.classes);
        registry
            .config
            .avatar_overrides
            .extend(custom.avatar_overrides);
        registry
            .config
            .sprite_overrides
            .extend(custom.sprite_overrides);
        registry
            .config
            .animation_aliases
            .extend(custom.animation_aliases);
        registry.validate()?;
        Ok(registry)
    }

    pub fn resolve_style(&self, style: ProjectileStyle) -> &CombatVisualProfile {
        let key = self
            .config
            .defaults
            .get(style_key(style))
            .map(String::as_str)
            .unwrap_or("standard");
        self.config
            .profiles
            .get(key)
            .unwrap_or_else(|| &self.config.profiles["standard"])
    }

    pub fn resolve(
        &self,
        class: Option<HeroClass>,
        style: ProjectileStyle,
        action_slot: Option<u8>,
        avatar: Option<&str>,
        sprite: Option<&str>,
    ) -> &CombatVisualProfile {
        let action = action_key(action_slot);
        for candidate in [
            sprite.and_then(|key| self.config.sprite_overrides.get(key)),
            avatar.and_then(|key| self.config.avatar_overrides.get(key)),
            class.and_then(|class| self.config.classes.get(class.id())),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(profile) = candidate
                .get(action)
                .or_else(|| candidate.get("default"))
                .and_then(|key| self.config.profiles.get(key))
            {
                return profile;
            }
        }
        self.resolve_style(style)
    }

    /// Exact, case-sensitive named animation clips; callers retain their existing heuristics.
    pub fn animation_aliases(&self, avatar_key: &str) -> Option<&AnimationAliases> {
        self.config.animation_aliases.get(avatar_key)
    }

    fn validate(&mut self) -> Result<(), String> {
        let config = &mut self.config;
        if config.profiles.len() > 128
            || config.avatar_overrides.len() > 256
            || config.sprite_overrides.len() > 256
            || config.animation_aliases.len() > 256
        {
            return Err("too many cosmetic profiles/overrides".into());
        }
        for (id, profile) in &mut config.profiles {
            if !valid_key(id)
                || !valid_color(profile.color)
                || !in_range(profile.scale, 0.25, 3.0)
                || !in_range(profile.trail.seconds, 0.0, 0.6)
                || !in_range(profile.trail.width, 0.02, 0.35)
                || !(1..=12).contains(&profile.trail.samples)
                || !valid_color(profile.impact.color)
                || !in_range(profile.impact.scale, 0.1, 2.0)
                || !in_range(profile.impact.lifetime, 0.08, 1.2)
            {
                return Err(format!("invalid or unbounded cosmetic profile {id}"));
            }
            // A melee contact throws no body, a silhouette is the mesh of a form, and a
            // wave is a form on the ground.
            if (profile.presentation == ProjectilePresentation::MeleeContact
                && profile.form.is_some())
                || (profile.silhouette.is_some() && profile.form.is_none())
                || (profile.presentation == ProjectilePresentation::Wave && profile.form.is_none())
            {
                return Err(format!("invalid projectile form in {id}"));
            }
            // A legacy projectile strikes one unit: its body may not look wider than that.
            if let Some(form) = profile.form {
                let width = profile.scale
                    * form_lateral_extent(
                        form,
                        profile.silhouette.unwrap_or(default_silhouette(form)),
                    );
                if (profile.presentation == ProjectilePresentation::Wave
                    && width > WAVE_WIDTH + WIDTH_SLACK)
                    || (form == ProjectileForm::Volley && width > VOLLEY_WIDTH + WIDTH_SLACK)
                {
                    return Err(format!("projectile form of {id} is {width} units wide"));
                }
            }
            if let Some(model) = &profile.model {
                if !safe_asset_path(&model.path, ".glb")
                    || model.scene > 31
                    || !in_range(model.scale, 0.01, 4.0)
                    || !model
                        .rotation_degrees
                        .into_iter()
                        .all(|v| in_range(v, -360.0, 360.0))
                {
                    return Err(format!("invalid packaged projectile model in {id}"));
                }
            }
            if let Some(sprite) = &profile.sprite {
                let cells = sprite.columns.saturating_mul(sprite.rows) as usize;
                if !safe_asset_path(&sprite.path, ".png")
                    || !(1..=16).contains(&sprite.columns)
                    || !(1..=16).contains(&sprite.rows)
                    || !sprite
                        .frame_size
                        .into_iter()
                        .all(|v| (1..=1024).contains(&v))
                    || sprite.frame_size[0].saturating_mul(sprite.columns) > 4096
                    || sprite.frame_size[1].saturating_mul(sprite.rows) > 4096
                    || sprite.frames == 0
                    || sprite.first_frame.saturating_add(sprite.frames) > cells
                    || !in_range(sprite.fps, 1.0, 60.0)
                    || !in_range(sprite.world_height, 0.3, 4.0)
                {
                    return Err(format!("invalid packaged projectile sprite in {id}"));
                }
            }
            profile.id.clone_from(id);
        }
        for (style, profile) in &config.defaults {
            if ![
                "standard",
                "arrow",
                "bullet",
                "rocket",
                "arcane",
                "holy",
                "crescent",
                "claw",
                "caster_bolt",
                "tower_bolt",
            ]
            .contains(&style.as_str())
                || !config.profiles.contains_key(profile)
            {
                return Err(format!("invalid style default {style}"));
            }
        }
        for class in config.classes.keys() {
            if HeroClass::from_id(class).is_none() {
                return Err(format!("unknown cosmetic class {class}"));
            }
        }
        for (key, actions) in config
            .classes
            .iter()
            .chain(&config.avatar_overrides)
            .chain(&config.sprite_overrides)
        {
            if !valid_key(key) || actions.len() > 6 {
                return Err("invalid cosmetic override key/count".into());
            }
            for (action, profile) in actions {
                if !["basic", "q", "w", "e", "r", "default"].contains(&action.as_str())
                    || !config.profiles.contains_key(profile)
                {
                    return Err(format!("invalid action/profile reference {key}/{action}"));
                }
            }
        }
        for (key, aliases) in &config.animation_aliases {
            if !valid_key(key) {
                return Err("invalid avatar animation key".into());
            }
            for names in [
                &aliases.idle,
                &aliases.walk,
                &aliases.run,
                &aliases.attack,
                &aliases.cast,
                &aliases.death,
            ] {
                if names.len() > 8 || names.iter().any(|name| !valid_key(name)) {
                    return Err("invalid animation clip aliases".into());
                }
            }
        }
        Ok(())
    }
}

#[derive(Asset, TypePath)]
struct LoadedCombatVisuals(CombatVisualRegistry);
#[derive(Default, TypePath)]
struct CombatVisualLoader;
impl AssetLoader for CombatVisualLoader {
    type Asset = LoadedCombatVisuals;
    type Settings = ();
    type Error = std::io::Error;
    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let json = std::str::from_utf8(&bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        CombatVisualRegistry::from_json(json)
            .map(LoadedCombatVisuals)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
    }
    fn extensions(&self) -> &[&str] {
        &["json"]
    }
}

#[derive(Resource, Default)]
struct PendingConfig(Option<Handle<LoadedCombatVisuals>>);

pub struct CombatVisualsPlugin;
impl Plugin for CombatVisualsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CombatVisualRegistry>()
            .init_resource::<PendingConfig>()
            .init_asset::<LoadedCombatVisuals>()
            .init_asset_loader::<CombatVisualLoader>()
            .add_systems(Startup, request_config)
            .add_systems(Update, apply_config);
    }
}
fn request_config(server: Res<AssetServer>, mut pending: ResMut<PendingConfig>) {
    pending.0 = Some(server.load(CONFIG_PATH));
}
fn apply_config(
    server: Res<AssetServer>,
    loaded: Res<Assets<LoadedCombatVisuals>>,
    mut pending: ResMut<PendingConfig>,
    mut registry: ResMut<CombatVisualRegistry>,
) {
    let Some(handle) = pending.0.as_ref() else {
        return;
    };
    if let Some(config) = loaded.get(handle) {
        *registry = config.0.clone();
        registry.revision = 1;
        pending.0 = None;
    } else if matches!(
        server.get_load_state(handle.id()),
        Some(bevy::asset::LoadState::Failed(_))
    ) {
        warn!("Packaged combat cosmetics unavailable; using embedded procedural profiles");
        pending.0 = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_classes_are_distinct_and_legacy_style_has_fallback() {
        let registry = CombatVisualRegistry::default();
        let shapes: std::collections::HashSet<_> = HeroClass::LEGACY
            .into_iter()
            .map(|class| {
                registry
                    .resolve(Some(class), ProjectileStyle::Standard, None, None, None)
                    .shape
            })
            .collect();
        assert_eq!(shapes.len(), 4);
        // Resolved skill effects use their own bounded geometry; basic shots
        // retain a usable style fallback independent of the chosen avatar.
        for class in [HeroClass::Dawnweaver, HeroClass::Wildspark] {
            assert_eq!(
                registry
                    .resolve(Some(class), ProjectileStyle::Standard, None, None, None)
                    .shape,
                ProjectileShape::Bolt
            );
        }
        assert_eq!(
            registry.resolve_style(ProjectileStyle::Standard).shape,
            ProjectileShape::Bolt
        );
        let packaged =
            CombatVisualRegistry::from_json(include_str!("../assets/config/combat_visuals.json"))
                .unwrap();
        let bullet = packaged.resolve(
            Some(HeroClass::Wildspark),
            ProjectileStyle::Bullet,
            Some(shared::BASIC_ATTACK_ACTION_SLOT),
            None,
            None,
        );
        let rocket = packaged.resolve(
            Some(HeroClass::Wildspark),
            ProjectileStyle::Rocket,
            Some(shared::BASIC_ATTACK_ACTION_SLOT),
            None,
            None,
        );
        assert_ne!(bullet.id, rocket.id);
        assert!(bullet.scale < rocket.scale);
        assert!(bullet.model.is_none());
        assert_eq!(
            rocket.model.as_ref().unwrap().path,
            "weapons/wild-rocket.glb"
        );
    }
    #[test]
    fn override_precedence_actions_and_animation_aliases_are_real() {
        let registry = CombatVisualRegistry::from_json(
            r#"{"schema_version":1,
          "avatar_overrides":{"agnes":{"basic":"mage_arcane","q":"warrior_crescent"}},
          "sprite_overrides":{"ranger":{"default":"cleric_holy"}},
          "animation_aliases":{"agnes":{"attack":["Sword_Swing"]}}}"#,
        )
        .unwrap();
        let resolve = |slot, avatar, sprite| {
            &registry
                .resolve(
                    Some(HeroClass::Ranger),
                    ProjectileStyle::Arrow,
                    slot,
                    avatar,
                    sprite,
                )
                .id
        };
        assert_eq!(resolve(None, Some("agnes"), Some("ranger")), "cleric_holy");
        assert_eq!(resolve(None, Some("agnes"), None), "mage_arcane");
        assert_eq!(resolve(Some(255), Some("agnes"), None), "mage_arcane");
        assert_eq!(resolve(Some(0), Some("agnes"), None), "warrior_crescent");
        assert_eq!(resolve(Some(2), Some("missing"), None), "ranger_arrow");
        assert_eq!(
            registry.animation_aliases("agnes").unwrap().attack,
            ["Sword_Swing"]
        );
    }
    #[test]
    fn invalid_config_rejects_gameplay_values_paths_and_unbounded_settings() {
        for json in [
            r#"{"schema_version":2}"#,
            r#"{"schema_version":1,"damage":99}"#,
            r#"{"schema_version":1,"classes":{"mage":{"q":"missing"}}}"#,
            r#"{"schema_version":1,"profiles":{"bad":{"shape":"arrow","color":[1,1,1,1],"scale":100}}}"#,
            r#"{"schema_version":1,"profiles":{"bad":{"shape":"arrow","color":[1,1,1,1],"model":{"path":"../secret.glb"}}}}"#,
            r#"{"schema_version":1,"profiles":{"bad":{"shape":"arrow","color":[1,1,1,1],"sprite":{"path":"shot.png","frame_size":[64,64],"columns":1,"rows":1,"frames":2,"fps":12,"world_height":1}}}}"#,
        ] {
            assert!(CombatVisualRegistry::from_json(json).is_err(), "{json}");
        }
        for path in [
            "https://evil/shot.glb",
            "/shot.glb",
            "../shot.glb",
            "a/../shot.glb",
            "a\\shot.glb",
            "a.glb#Scene0",
            "file://a.glb",
        ] {
            assert!(!safe_asset_path(path, ".glb"), "{path}");
        }
        assert!(safe_asset_path("cosmetics/arrow-v2.glb", ".glb"));
    }
    #[test]
    fn forms_are_optional_vocabulary_and_agree_with_the_presentation() {
        let profile = |fields: &str| {
            CombatVisualRegistry::from_json(&format!(
                r#"{{"schema_version":1,"profiles":{{"shot":{{"shape":"crescent","color":[1,1,1,1]{fields}}}}},
                "classes":{{"warrior":{{"e":"shot"}}}}}}"#
            ))
            .map(|registry| {
                registry
                    .resolve(
                        Some(HeroClass::Warrior),
                        ProjectileStyle::Crescent,
                        Some(2),
                        None,
                        None,
                    )
                    .clone()
            })
        };
        // Without the new fields a profile is the thrown `shape` body, as before.
        let plain = profile("").unwrap();
        assert_eq!((plain.form, plain.silhouette), (None, None));
        assert_eq!(plain.presentation, ProjectilePresentation::Projectile);
        let wave = profile(r#","form":"wavefront","silhouette":"crescent","presentation":"wave""#)
            .unwrap();
        assert_eq!(wave.form, Some(ProjectileForm::Wavefront));
        assert_eq!(wave.silhouette, Some(Silhouette::Crescent));
        assert_eq!(wave.presentation, ProjectilePresentation::Wave);
        assert_eq!(wave.shape, ProjectileShape::Crescent);
        let contact = profile(r#","presentation":"melee_contact""#).unwrap();
        assert_eq!(contact.presentation, ProjectilePresentation::MeleeContact);
        assert!(profile(r#","form":"dart""#).unwrap().silhouette.is_none());
        for rejected in [
            r#","form":"dart","presentation":"melee_contact""#,
            r#","silhouette":"kite""#,
            r#","form":"reach_streak""#,
            r#","form":"dart","silhouette":"disc""#,
            r#","presentation":"area""#,
        ] {
            assert!(profile(rejected).is_err(), "{rejected}");
        }
    }

    const TARGET: &str = include_str!("skill_presentation/fixtures/target_combat_visuals.json");
    /// Mesh parts of each form, its team cue included (`architecture.md` 5.13).
    const PART_COUNTS: [(ProjectileForm, usize); 7] = [
        (ProjectileForm::Dart, 4),
        (ProjectileForm::Comet, 6),
        (ProjectileForm::DiscSkim, 3),
        (ProjectileForm::Tumbler, 4),
        (ProjectileForm::Wavefront, 4),
        (ProjectileForm::Volley, 4),
        (ProjectileForm::TwinHelix, 4),
    ];
    /// Moments of a flight, dense enough to meet every turn and swell near its extreme.
    fn moments() -> impl Iterator<Item = f32> {
        (0..1500).map(|step| step as f32 * 0.002)
    }

    /// The vertices of a part at one moment, in the frame of the projectile.
    fn drawn(part: &FormPart, secs: f32) -> Vec<Vec3> {
        let pose = part.pose(secs);
        bodies::silhouette_mesh(part.mesh)
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|values| values.as_float3())
            .unwrap()
            .iter()
            .map(|point| pose.transform_point(Vec3::from_array(*point)))
            .collect()
    }

    /// What holds for every form whatever silhouette it is built from: the part count of
    /// the table, one team cue at the end, parts that have a size and a finite pose.
    fn body_of(form: ProjectileForm, mesh: Silhouette) -> Vec<FormPart> {
        let parts = form_parts(form, mesh);
        let count = PART_COUNTS.iter().find(|(id, _)| *id == form).unwrap().1;
        assert_eq!(parts.len(), count, "{form:?} {mesh:?}");
        assert!(count <= FORM_PARTS);
        assert_eq!(FlightBody::Form(form, mesh).parts(), parts);
        let cues: Vec<usize> = (0..parts.len())
            .filter(|index| parts[*index].paint == FormPaint::Team)
            .collect();
        assert_eq!(cues, [count - 1], "{form:?} {mesh:?}");
        // The cue is small, and the silhouette the profile names is the body: both its
        // light and the deep shade under it.
        assert!(parts[count - 1].size.max_element() <= 0.2);
        for paint in [FormPaint::Tint, FormPaint::Echo] {
            assert!(
                parts
                    .iter()
                    .any(|part| part.mesh == mesh && part.paint == paint),
                "{form:?} {mesh:?} {paint:?}"
            );
        }
        for part in &parts {
            assert!(part.size.min_element() > 0.0, "{form:?} {mesh:?}");
            for secs in [0.0, 0.017, 0.4, 1.3, 7.7, 600.0] {
                let pose = part.pose(secs);
                assert!(pose.is_finite() && pose.rotation.is_normalized());
                assert!(pose.scale.min_element() > 0.0, "{form:?} {mesh:?}");
            }
        }
        parts
    }

    /// The outer corners of a part at one moment: its smallest and largest coordinates.
    fn bounds(part: &FormPart, secs: f32) -> (Vec3, Vec3) {
        drawn(part, secs).into_iter().fold(
            (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
            |(low, high), point| (low.min(point), high.max(point)),
        )
    }

    #[test]
    fn a_dart_is_one_silhouette_stretched_along_the_heading() {
        for mesh in Silhouette::ALL {
            let parts = body_of(ProjectileForm::Dart, *mesh);
            // The light head lies inside its deep rim.
            assert_eq!(
                (parts[0].paint, parts[1].paint),
                (FormPaint::Echo, FormPaint::Tint)
            );
            let ((rim_low, rim_high), (low, high)) =
                (bounds(&parts[0], 0.0), bounds(&parts[1], 0.0));
            assert!(rim_low.x <= low.x && rim_high.x >= high.x, "{mesh:?}");
            assert!(rim_low.z < low.z && rim_high.z > high.z, "{mesh:?}");
            assert!(parts[1].size.z > 3.0 * parts[1].size.x.max(parts[1].size.y));
            // The dark shaft is longer still and trails behind; nothing turns.
            assert_eq!(
                (parts[2].mesh, parts[2].paint),
                (Silhouette::Block, FormPaint::Echo)
            );
            assert!(parts[2].size.z > rim_high.z - rim_low.z && parts[2].at.z < 0.0);
            assert!(parts.iter().all(|part| part.motion == FormMotion::Still));
        }
        // A cone flies point first and a flat silhouette shows its face to the sky.
        let cone = form_parts(ProjectileForm::Dart, Silhouette::Cone)[1];
        let tip = cone.pose(0.0).transform_point(Vec3::Y * 0.5);
        assert!(tip.z > 1.0 && tip.x.abs() < 1e-5 && tip.y.abs() < 1e-5);
        let chevron = form_parts(ProjectileForm::Dart, Silhouette::Chevron)[1];
        assert!((chevron.pose(0.0).rotation * Vec3::Z).distance(Vec3::Y) < 1e-5);
        assert!((chevron.pose(0.0).rotation * Vec3::X).distance(Vec3::Z) < 1e-5);
    }

    #[test]
    fn a_comet_is_a_pulsing_head_and_three_shrinking_afterimages() {
        for mesh in Silhouette::ALL {
            let parts = body_of(ProjectileForm::Comet, *mesh);
            // The light head and the deep rim under it swell and shrink as one.
            assert_eq!(
                (parts[0].paint, parts[1].paint),
                (FormPaint::Echo, FormPaint::Tint)
            );
            assert!(matches!(parts[1].motion, FormMotion::Pulse { .. }));
            assert_eq!(parts[0].motion, parts[1].motion);
            assert!(parts[0].size.x > parts[1].size.x);
            let centre = |part: &FormPart| {
                let (low, high) = bounds(part, 0.0);
                (low.z + high.z) / 2.0
            };
            assert!(
                (centre(&parts[0]) - centre(&parts[1])).abs() < 1e-4,
                "{mesh:?}"
            );
            let scales: Vec<f32> = moments()
                .map(|secs| parts[1].pose(secs).scale.x / parts[1].pose(0.0).scale.x)
                .collect();
            let (least, most) = scales.iter().fold((f32::MAX, f32::MIN), |(low, high), s| {
                (low.min(*s), high.max(*s))
            });
            assert!(
                (least - 0.88).abs() < 0.01 && (most - 1.12).abs() < 0.01,
                "{mesh:?}"
            );
            // Three copies of the head behind it, each smaller and farther back; they
            // cool from its light to its deep shade.
            let echoes = &parts[2..5];
            for pair in [&parts[1..3], &parts[2..4], &parts[3..5]] {
                assert_eq!(pair[1].mesh, *mesh);
                assert!(pair[1].at.z < pair[0].at.z && pair[1].size.x < pair[0].size.x);
            }
            assert_eq!(
                echoes.iter().map(|echo| echo.paint).collect::<Vec<_>>(),
                [FormPaint::Tint, FormPaint::Echo, FormPaint::Echo]
            );
            assert!(echoes.iter().all(|echo| echo.motion == FormMotion::Still));
        }
        // A teardrop flies bulb first: its point trails.
        let drop = form_parts(ProjectileForm::Comet, Silhouette::Drop)[1];
        assert!(drop.pose(0.0).transform_point(Vec3::X * 0.5).z < -0.3);
    }

    #[test]
    fn a_disc_skim_spins_a_level_plate_about_the_vertical_axis() {
        for mesh in Silhouette::ALL {
            let parts = body_of(ProjectileForm::DiscSkim, *mesh);
            // A deep rim under a smaller bright face, turning together.
            assert_eq!(
                (parts[0].paint, parts[1].paint),
                (FormPaint::Echo, FormPaint::Tint)
            );
            assert!(parts[1].size.x < parts[0].size.x && parts[1].at.y > parts[0].at.y);
            assert_eq!(parts[0].motion, parts[1].motion);
            let FormMotion::Spin(rate) = parts[0].motion else {
                panic!("{mesh:?} does not spin");
            };
            let (rest, later) = (parts[0].pose(0.0), parts[0].pose(0.1));
            // The vertical axis keeps still while everything else turns about it.
            assert!((later.rotation * rest.rotation.inverse() * Vec3::Y).distance(Vec3::Y) < 1e-5);
            let turned = later.rotation * rest.rotation.inverse() * Vec3::Z;
            assert!((turned.z - (rate * 0.1).cos()).abs() < 1e-4 && turned.y.abs() < 1e-5);
            // Flat: far wider than tall at every moment.
            for secs in [0.0, 0.07, 0.33] {
                let (low, high) = bounds(&parts[0], secs);
                assert!(high.y - low.y < 0.45 * (high.x - low.x), "{mesh:?}");
            }
        }
    }

    #[test]
    fn a_tumbler_turns_end_over_end_and_is_never_seen_edge_on() {
        for mesh in Silhouette::ALL {
            let parts = body_of(ProjectileForm::Tumbler, *mesh);
            let FormMotion::Tumble(rate) = parts[0].motion else {
                panic!("{mesh:?} does not tumble");
            };
            assert_eq!(parts[1].motion, parts[0].motion);
            // The lateral axis keeps still; the top goes forward.
            let turn = parts[0].pose(0.05).rotation * parts[0].pose(0.0).rotation.inverse();
            assert!((turn * Vec3::X).distance(Vec3::X) < 1e-5);
            assert!((turn * Vec3::Y).z > 0.0 && rate > 0.0);
            assert_eq!((parts[1].mesh, parts[1].paint), (*mesh, FormPaint::Echo));
            if bodies::planar(PartMesh::Silhouette(*mesh)) {
                // Two plates across each other: one of the two faces any camera.
                let face = |part: &FormPart, secs: f32| part.pose(secs).rotation * Vec3::Z;
                for secs in [0.0, 0.03, 0.2] {
                    assert!(face(&parts[0], secs).dot(face(&parts[1], secs)).abs() < 1e-5);
                }
                assert!(face(&parts[1], 0.2).distance(face(&parts[1], 0.0)) < 1e-5);
            } else {
                // A band around the middle of a solid, wider than the solid itself.
                assert!(parts[1].size.x > parts[0].size.x && parts[1].size.z < parts[0].size.z);
            }
        }
    }

    #[test]
    fn a_wavefront_lies_across_the_heading_and_keeps_low() {
        for mesh in Silhouette::ALL {
            let parts = body_of(ProjectileForm::Wavefront, *mesh);
            assert!(parts.iter().all(|part| part.motion == FormMotion::Still));
            let (rim_low, rim_high) = bounds(&parts[0], 0.0);
            for part in &parts[..3] {
                let (low, high) = bounds(part, 0.0);
                // Nothing is wider than the rim, sinks under the ground or stands tall.
                assert!(
                    low.x >= rim_low.x - 1e-5 && high.x <= rim_high.x + 1e-5,
                    "{mesh:?}"
                );
                // (A torus lies in the ground lift by half its tube.)
                assert!(low.y > -0.09 && high.y < 0.8, "{mesh:?} {low} {high}");
            }
            // The leading edge of the rim and of the face is just ahead of the
            // replicated position.
            for part in &parts[..2] {
                assert!((0.2..0.4).contains(&bounds(part, 0.0).1.z), "{mesh:?}");
            }
            assert_eq!(parts[0].paint, FormPaint::Echo);
        }
        // The crescent of Heroic Strike: wider than long, its convex edge leading.
        let parts = form_parts(ProjectileForm::Wavefront, Silhouette::Crescent);
        let (low, high) = bounds(&parts[0], 0.0);
        assert!(high.x - low.x > 1.2 * (high.z - low.z));
        let middle = parts[0].pose(0.0).transform_point(Vec3::X * 0.5);
        assert!((middle.z - high.z).abs() < 1e-5 && middle.x.abs() < 1e-5);
        // Its crest stands across the heading: the face looks along it.
        assert!((parts[2].pose(0.0).rotation * Vec3::Z).distance(Vec3::Z) < 1e-5);
    }

    #[test]
    fn a_volley_is_three_copies_in_a_tight_fan() {
        for mesh in Silhouette::ALL {
            let parts = body_of(ProjectileForm::Volley, *mesh);
            let copies = &parts[..3];
            assert!(
                copies
                    .iter()
                    .all(|copy| copy.mesh == *mesh && copy.size == parts[0].size)
            );
            assert!(copies.iter().all(|copy| copy.motion == FormMotion::Still));
            // One leads on the axis; the other two follow to either side, turned outward.
            assert_eq!((copies[0].at.x, copies[0].yaw), (0.0, 0.0));
            assert!(copies[1].at.x < 0.0 && copies[1].yaw < 0.0 && copies[1].at.z < copies[0].at.z);
            assert!(copies[2].at.x > 0.0 && copies[2].yaw > 0.0 && copies[2].at.z < copies[1].at.z);
            assert_eq!(copies[1].at.x, -copies[2].at.x);
        }
        // The claws of Feral Swipe point along the heading, a hand apart at most.
        let width = form_lateral_extent(ProjectileForm::Volley, Silhouette::Claw);
        assert!((0.5..=VOLLEY_WIDTH).contains(&width), "{width}");
    }

    #[test]
    fn a_twin_helix_winds_two_strands_around_the_heading_axis() {
        for mesh in Silhouette::ALL {
            let parts = body_of(ProjectileForm::TwinHelix, *mesh);
            assert_eq!(
                (parts[0].paint, parts[1].paint),
                (FormPaint::Tint, FormPaint::Echo)
            );
            let FormMotion::Wind { radius, rate, .. } = parts[0].motion else {
                panic!("{mesh:?} does not wind");
            };
            let mut quarter = None;
            for secs in moments().take(400) {
                let (a, b) = (
                    parts[0].pose(secs).translation,
                    parts[1].pose(secs).translation,
                );
                // Always opposite each other, at one distance from the axis.
                assert!((a + b).length() < 1e-4 && a.z.abs() < 1e-6, "{mesh:?}");
                assert!((a.truncate().length() - radius).abs() < 1e-4);
                if (rate * secs - std::f32::consts::FRAC_PI_2).abs() < 0.02 {
                    quarter = Some(a);
                }
            }
            // A quarter turn later the strand that was beside the axis is above it.
            assert!(quarter.unwrap().y > 0.99 * radius);
            assert_eq!(
                (parts[2].mesh, parts[2].paint),
                (Silhouette::Block, FormPaint::Tint)
            );
            assert!(parts[2].size.z > 3.0 * radius && parts[2].motion == FormMotion::Still);
        }
    }

    #[test]
    fn a_melee_contact_draws_the_reach_streak_only_for_a_known_basic_attack() {
        let registry = CombatVisualRegistry::from_json(TARGET).unwrap();
        // Two low parts and no blade: a sliver of light on its deep rim, on the ground.
        let streak = reach_streak();
        assert_eq!(FlightBody::Reach.parts(), streak);
        assert_eq!(streak.len(), 2);
        assert_eq!(
            (streak[0].paint, streak[1].paint),
            (FormPaint::Echo, FormPaint::Tint)
        );
        for sliver in &streak {
            let (low, high) = bounds(sliver, 0.0);
            assert!(low.y >= 0.0 && high.y < 0.05 && high.x - low.x < 0.5 && high.z - low.z > 1.9);
            // It trails the replicated position.
            assert!(high.z <= 0.5 && low.z < -1.5);
            assert_eq!(sliver.motion, FormMotion::Still);
        }
        assert!(FlightBody::Shape.parts().is_empty());

        for (class, style) in [
            (HeroClass::Warrior, ProjectileStyle::Crescent),
            (HeroClass::Warden, ProjectileStyle::Claw),
        ] {
            let basic = registry.resolve(Some(class), style, Some(255), None, None);
            assert_eq!(basic.presentation, ProjectilePresentation::MeleeContact);
            assert!(known_basic(Some(class), Some(255)) && known_basic(Some(class), None));
            assert_eq!(basic.flight_body(true, true), FlightBody::Reach);
            assert!(basic.hugs_ground(FlightBody::Reach) && !basic.trails());
            // The style default of a hidden or unknown owner is the same profile. Its
            // projectile may be an ability on a long flight, so it keeps the thrown body.
            let unknown = registry.resolve(None, style, Some(255), None, None);
            assert_eq!(unknown.id, basic.id);
            assert!(!known_basic(None, Some(255)) && !known_basic(None, None));
            assert_eq!(unknown.flight_body(true, false), FlightBody::Shape);
            assert!(!unknown.hugs_ground(FlightBody::Shape));
            for slot in 0..4 {
                assert!(!known_basic(Some(class), Some(slot)));
            }
            // Without the shared meshes nothing but the `shape` can be drawn.
            assert_eq!(basic.flight_body(false, true), FlightBody::Shape);
            // No body of a melee contact is a missile.
            for body in [FlightBody::Reach, FlightBody::Shape] {
                assert!(!basic.puffs(body, style) && !basic.puffs(body, ProjectileStyle::Arcane));
            }
        }
    }

    #[test]
    fn form_beats_model_when_meshes_exist() {
        let registry = CombatVisualRegistry::from_json(TARGET).unwrap();
        let rocket = registry.resolve(
            Some(HeroClass::Wildspark),
            ProjectileStyle::Rocket,
            Some(shared::BASIC_ATTACK_ACTION_SLOT),
            None,
            None,
        );
        assert_eq!(
            rocket.model.as_ref().unwrap().path,
            "weapons/wild-rocket.glb"
        );
        let canister = FlightBody::Form(ProjectileForm::Tumbler, Silhouette::Block);
        for known_basic in [false, true] {
            assert_eq!(rocket.flight_body(true, known_basic), canister);
            // Without the shared meshes the profile is its `shape`, and with it its model.
            assert_eq!(rocket.flight_body(false, known_basic), FlightBody::Shape);
        }
        // A form without a named silhouette takes the one of its form.
        let plain = CombatVisualRegistry::from_json(
            r#"{"schema_version":1,"profiles":{"shot":{"shape":"bolt","color":[1,1,1,1],"form":"comet"}},
            "classes":{"mage":{"q":"shot"}}}"#,
        )
        .unwrap();
        let shot = plain.resolve(
            Some(HeroClass::Mage),
            ProjectileStyle::Arcane,
            Some(0),
            None,
            None,
        );
        assert_eq!(
            shot.flight_body(true, false),
            FlightBody::Form(ProjectileForm::Comet, Silhouette::Ball)
        );
        for form in ProjectileForm::ALL {
            assert_eq!(
                form_parts(*form, default_silhouette(*form))[0].mesh,
                default_silhouette(*form)
            );
        }
        // The embedded and the packaged profiles name no form yet: every body is a shape.
        for registry in [
            CombatVisualRegistry::default(),
            CombatVisualRegistry::from_json(include_str!("../assets/config/combat_visuals.json"))
                .unwrap(),
        ] {
            for profile in registry.config.profiles.values() {
                assert_eq!(
                    profile.flight_body(true, true),
                    FlightBody::Shape,
                    "{}",
                    profile.id
                );
            }
        }
    }

    /// Rule V-LAT: a legacy projectile strikes one unit, so a ground wave is at most one
    /// hero wide and a volley fans out over at most 0.6 units, after scale.
    #[test]
    fn the_width_of_a_wave_and_of_a_volley_is_capped() {
        let target: serde_json::Value = serde_json::from_str(TARGET).unwrap();
        let parse = |edit: &dyn Fn(&mut serde_json::Value)| {
            let mut json = target.clone();
            edit(&mut json["profiles"]);
            CombatVisualRegistry::from_json(&json.to_string())
        };
        let registry = parse(&|_| {}).unwrap();
        let width = |id: &str| {
            let profile = &registry.config.profiles[id];
            (
                profile.scale
                    * form_lateral_extent(profile.form.unwrap(), profile.silhouette.unwrap()),
                profile,
            )
        };
        // Every bound profile of the final data, with the width it is drawn at.
        let mut waves = Vec::new();
        for profile in registry.config.profiles.values() {
            if profile.presentation == ProjectilePresentation::Wave {
                let (drawn, _) = width(&profile.id);
                assert!(drawn <= WAVE_WIDTH + WIDTH_SLACK, "{} {drawn}", profile.id);
                waves.push(profile.id.as_str());
            }
        }
        waves.sort_unstable();
        assert_eq!(
            waves,
            ["chainkeeper_links", "heroic_strike", "primal_maul", "smite"]
        );
        // One hero is 1.24 wide: the two wavefronts are that wide, never wider.
        for id in ["heroic_strike", "primal_maul"] {
            assert!(
                (1.2..=WAVE_WIDTH + WIDTH_SLACK).contains(&width(id).0),
                "{id}"
            );
        }
        let (fan, swipe) = width("feral_swipe");
        assert_eq!(swipe.form, Some(ProjectileForm::Volley));
        assert!((0.5..=VOLLEY_WIDTH).contains(&fan), "{fan}");

        // The same profiles one step larger are refused, and with them the whole file.
        for (id, scale) in [
            ("heroic_strike", 1.4),
            ("primal_maul", 1.31),
            ("smite", 1.35),
            ("chainkeeper_links", 0.95),
            ("feral_swipe", 1.05),
        ] {
            let error = parse(&|profiles| profiles[id]["scale"] = scale.into())
                .err()
                .unwrap_or_else(|| panic!("{id} at {scale} is accepted"));
            assert!(
                error.contains(id) && error.contains("units wide"),
                "{error}"
            );
        }
        // The cap is on the look of a wave and of a volley, not on a thrown body, and the
        // two forms it was written for are legal up to a scale that equals their cap.
        assert!(parse(&|profiles| profiles["shield_bash"]["scale"] = 3.0.into()).is_ok());
        assert!(parse(&|profiles| profiles["feral_swipe"]["scale"] = 1.0.into()).is_ok());
        assert!(form_lateral_extent(ProjectileForm::Wavefront, Silhouette::Block) <= 1.0);
        assert!(form_lateral_extent(ProjectileForm::Wavefront, Silhouette::Crescent) <= 1.0);
        // A volley on the ground is bound by both caps, a wave needs a form to measure,
        // and the default silhouette of a form is measured like a named one.
        for edit in [
            &(|profiles: &mut serde_json::Value| {
                profiles["feral_swipe"]["presentation"] = "wave".into();
                profiles["feral_swipe"]["scale"] = 1.05.into();
            }) as &dyn Fn(&mut serde_json::Value),
            &|profiles: &mut serde_json::Value| {
                profiles["smite"].as_object_mut().unwrap().remove("form");
                profiles["smite"]
                    .as_object_mut()
                    .unwrap()
                    .remove("silhouette");
            },
            &|profiles: &mut serde_json::Value| {
                profiles["heroic_strike"]
                    .as_object_mut()
                    .unwrap()
                    .remove("silhouette");
                profiles["heroic_strike"]["scale"] = 1.4.into();
            },
        ] {
            assert!(parse(edit).is_err());
        }
        assert!(
            parse(&|profiles| {
                profiles["heroic_strike"]
                    .as_object_mut()
                    .unwrap()
                    .remove("silhouette");
            })
            .is_ok()
        );
    }

    /// The validated width is the drawn width: at every moment every vertex of every part
    /// but the team cue lies within half the lateral extent of the heading axis, and some
    /// vertex reaches it.
    #[test]
    fn the_drawn_width_of_a_form_is_its_lateral_extent() {
        for form in ProjectileForm::ALL {
            for mesh in Silhouette::ALL {
                let half = form_lateral_extent(*form, *mesh) / 2.0;
                let parts = form_parts(*form, *mesh);
                let mut reached: f32 = 0.0;
                for part in parts.iter().filter(|part| part.paint != FormPaint::Team) {
                    for secs in moments() {
                        for point in drawn(part, secs) {
                            assert!(point.x.abs() <= half + 1e-4, "{form:?} {mesh:?}");
                            reached = reached.max(point.x.abs());
                        }
                    }
                }
                assert!(
                    reached > 0.995 * half,
                    "{form:?} {mesh:?}: {reached} of {half}"
                );
            }
        }
        // The bound rows of the final data at their scale: inside half their cap.
        let registry = CombatVisualRegistry::from_json(TARGET).unwrap();
        for (id, cap) in [
            ("heroic_strike", WAVE_WIDTH),
            ("primal_maul", WAVE_WIDTH),
            ("smite", WAVE_WIDTH),
            ("chainkeeper_links", WAVE_WIDTH),
            ("feral_swipe", VOLLEY_WIDTH),
        ] {
            let profile = &registry.config.profiles[id];
            let FlightBody::Form(form, mesh) = profile.flight_body(true, false) else {
                panic!("{id} has no form");
            };
            for part in form_parts(form, mesh)
                .iter()
                .filter(|part| part.paint != FormPaint::Team)
            {
                for secs in moments().step_by(7) {
                    for point in drawn(part, secs) {
                        assert!((point.x * profile.scale).abs() <= cap / 2.0 + 1e-4, "{id}");
                    }
                }
            }
        }
    }

    #[test]
    fn only_a_thrown_shape_of_a_magic_style_leaves_puffs_and_only_a_thrown_body_a_trail() {
        let registry = CombatVisualRegistry::from_json(TARGET).unwrap();
        let profile = |id: &str| &registry.config.profiles[id];
        let magic = [ProjectileStyle::Arcane, ProjectileStyle::Holy];
        for style in [
            ProjectileStyle::Standard,
            ProjectileStyle::Bullet,
            ProjectileStyle::Rocket,
            ProjectileStyle::Arrow,
            ProjectileStyle::Arcane,
            ProjectileStyle::Holy,
            ProjectileStyle::Crescent,
            ProjectileStyle::Claw,
            ProjectileStyle::CasterBolt,
            ProjectileStyle::TowerBolt,
        ] {
            // A thrown shape, in either render mode.
            assert_eq!(
                profile("mage_arcane").puffs(FlightBody::Shape, style),
                magic.contains(&style),
                "{style:?}"
            );
            // A drawn form, a wave and a melee contact, whatever stands for them.
            let comet = FlightBody::Form(ProjectileForm::Comet, Silhouette::Drop);
            assert!(!profile("pyroblast").puffs(comet, style));
            for id in [
                "smite",
                "heroic_strike",
                "chainkeeper_links",
                "warrior_crescent",
            ] {
                for body in [FlightBody::Shape, FlightBody::Reach, comet] {
                    assert!(!profile(id).puffs(body, style), "{id} {style:?}");
                }
            }
        }
        // Rule T0: a trail that lasts no time is no trail, and neither a wave nor a melee
        // contact draws one.
        for (id, trails) in [
            ("ranger_arrow", true),
            ("pyroblast", true),
            ("shield_bash", false),
            ("rampage", false),
            ("emberveil_petal", false),
            ("smite", false),
            ("primal_maul", false),
            ("warden_claw", false),
            ("warrior_crescent", false),
        ] {
            assert_eq!(profile(id).trails(), trails, "{id}");
        }
        // Only a form on the ground and the reach streak leave the height of the flight.
        let wave = profile("heroic_strike");
        assert!(wave.hugs_ground(wave.flight_body(true, false)));
        assert!(!wave.hugs_ground(wave.flight_body(false, false)));
        let thrown = profile("rampage");
        assert!(!thrown.hugs_ground(thrown.flight_body(true, false)));
    }
}
