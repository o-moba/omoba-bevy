//! One table for every boundary and area a skill may draw. Shapes are functions of the
//! received effect fields and the catalog only; presentation data cannot scale or move them.
//! The parser, the area flash, the body renderer, the 2D fallback and the aim preview read
//! this table.

use super::category::{cone_half_angle, own_kinds};
use super::vocab::{Archetype, PreviewShape};
use bevy::math::Vec2;
use shared::TargetingMode;
use shared::loadout::{
    EffectVisualKind, SkillDefinition, SkillEffect, SkillEffectState, SkillId, Technique, skill,
};
use shared::wire::TargetKind;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// A unit pick takes the nearest candidate within this distance of the aim
/// (`common/src/skills/advanced.rs:186`).
pub(crate) const PICK_RADIUS: f32 = 2.0;
/// Furnace Breath hits where the direction cosine exceeds this
/// (`common/src/skills/advanced.rs:1175`).
pub(crate) const FURNACE_CONE_COS: f32 = 0.6;
/// Nightfall hits where the direction cosine exceeds this
/// (`common/src/skills/advanced.rs:881`).
pub(crate) const NIGHTFALL_SECTOR_COS: f32 = 0.2;
/// The dagger corridor is at most this wide to each side (`common/src/skills/dagger.rs:134`).
pub(crate) const DAGGER_LANE_CLAMP: f32 = 0.55;
/// Thunder Kick pushes its target this far (`common/src/skills/advanced.rs:836`).
pub(crate) const KICK_LENGTH: f32 = 10.0;
/// Nightfall retreats this far against the aim (`common/src/skills/advanced.rs:892`).
pub(crate) const NIGHTFALL_RETREAT: f32 = 7.0;
/// Mountain Echo is redirected only within this distance of the colossus
/// (`common/src/skills/advanced.rs:509`).
pub(crate) const RECAST_GATE_MOUNTAIN_ECHO: f32 = 4.0;
/// A cage side hits within this distance of its segment (`common/src/skills/advanced.rs:1113`).
pub(crate) const CAGE_BAR_HALF_WIDTH: f32 = 0.4;
/// The shield wall is centred this far ahead of its owner
/// (`common/src/skills/advanced.rs:1790`).
pub(crate) const WALL_AHEAD: f32 = 1.0;
/// Chain Sweep moves every unit it hits this far along the aim
/// (`common/src/skills/advanced.rs:1021`).
pub(crate) const SWEEP_PUSH: f32 = 3.0;
/// Rule F: a cone whose received axis is shorter than the cast range by more than this was
/// cut by fog and is drawn as a plain segment.
pub(crate) const SECTOR_CLIP_SLACK: f32 = 0.05;

/// Radii the server replicates for its auxiliary objects
/// (`common/src/skills/advanced.rs:2377`, `:2388`, `:2398`, `:2408`).
pub(crate) const ORB_RADIUS: f32 = 0.65;
pub(crate) const HEALING_RADIUS: f32 = 5.0;
pub(crate) const ANCHOR_RADIUS: f32 = 0.5;
pub(crate) const SOUL_RADIUS: f32 = 0.35;

/// The `radius` an effect of this skill and kind is replicated with: the catalog value of
/// the skill for the effect of the first cast (`common/src/skills/mod.rs:1456-1488`), a
/// server literal for an auxiliary object. The parser sizes body parts against it; a
/// drawn boundary always takes the received value.
pub(crate) fn replicated_radius(id: SkillId, kind: EffectVisualKind) -> f32 {
    match kind {
        EffectVisualKind::Orb => ORB_RADIUS,
        EffectVisualKind::Healing => HEALING_RADIUS,
        EffectVisualKind::Anchor => ANCHOR_RADIUS,
        EffectVisualKind::Soul => SOUL_RADIUS,
        _ => catalog_radius(id),
    }
}

/// The catalog radius of a skill: the half-width of its sweep, the radius of its zone or
/// strike, or the half-width of its beam.
fn catalog_radius(id: SkillId) -> f32 {
    match skill(id).effect {
        SkillEffect::Technique { radius, .. }
        | SkillEffect::LinearProjectile { radius, .. }
        | SkillEffect::ReturningShield { radius, .. }
        | SkillEffect::RecastZone { radius, .. }
        | SkillEffect::TrapLine { radius, .. }
        | SkillEffect::ImpactRocket { radius, .. } => radius,
        SkillEffect::Beam { width, .. } => width,
        SkillEffect::WeaponToggle { .. } => 0.0,
    }
}

/// Geometry in simulation ground coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum GeoShape {
    /// The circle of `radius` around `center`.
    Ring { center: Vec2, radius: f32 },
    /// Every point within `radius` of the segment, so both ends are round.
    Capsule { from: Vec2, to: Vec2, radius: f32 },
    /// The strip of `half_width` to each side of the segment, with flat ends.
    Lane {
        from: Vec2,
        to: Vec2,
        half_width: f32,
    },
    /// The cone from `apex` along the unit `axis`, `radius` long, `half_angle` to each side.
    Sector {
        apex: Vec2,
        axis: Vec2,
        radius: f32,
        half_angle: f32,
    },
    /// The regular pentagon with corners at `radius`; side `i` runs from angle `i * TAU / 5`
    /// (`common/src/skills/advanced.rs:1108-1112`).
    Pentagon { center: Vec2, radius: f32 },
    /// A plain line: the wall bar, or a fog-cut cone.
    Segment { from: Vec2, to: Vec2 },
    /// Nothing may be drawn as a boundary.
    None,
}

/// Line segments an outline spends on a full circle.
const OUTLINE_STEPS: usize = 48;

impl GeoShape {
    /// The boundary as polylines in ground coordinates, for a backend that draws lines. A
    /// closed line repeats its first point. A pentagon yields its five sides one by one,
    /// side `i` at index `i`, so that a consumed side can be left out.
    pub(crate) fn outline(&self) -> Vec<Vec<Vec2>> {
        let arc = |center: Vec2, radius: f32, from: f32, sweep: f32| -> Vec<Vec2> {
            let steps = (sweep.abs() / TAU * OUTLINE_STEPS as f32).ceil().max(1.0) as usize;
            (0..=steps)
                .map(|i| {
                    let angle = from + sweep * i as f32 / steps as f32;
                    center + Vec2::from_angle(angle) * radius
                })
                .collect()
        };
        match *self {
            Self::Ring { center, radius } => vec![arc(center, radius, 0.0, TAU)],
            Self::Capsule { from, to, radius } => {
                let Some(along) = (to - from).try_normalize() else {
                    return vec![arc(from, radius, 0.0, TAU)];
                };
                let heading = along.to_angle();
                // Round the far end, come back along the other side, round the near end.
                let mut points = arc(to, radius, heading - FRAC_PI_2, PI);
                points.extend(arc(from, radius, heading + FRAC_PI_2, PI));
                points.push(points[0]);
                vec![points]
            }
            Self::Lane {
                from,
                to,
                half_width,
            } => {
                let side = (to - from).normalize_or_zero().perp() * half_width;
                vec![vec![
                    from + side,
                    to + side,
                    to - side,
                    from - side,
                    from + side,
                ]]
            }
            Self::Sector {
                apex,
                axis,
                radius,
                half_angle,
            } => {
                let mut points = vec![apex];
                points.extend(arc(
                    apex,
                    radius,
                    axis.to_angle() - half_angle,
                    2.0 * half_angle,
                ));
                points.push(apex);
                vec![points]
            }
            Self::Pentagon { center, radius } => (0..5)
                .map(|side| {
                    [side, side + 1]
                        .map(|corner| center + Vec2::from_angle(TAU * corner as f32 / 5.0) * radius)
                        .to_vec()
                })
                .collect(),
            Self::Segment { from, to } => vec![vec![from, to]],
            Self::None => Vec::new(),
        }
    }
}

/// The family of shape a replicated kind of a skill takes, before any effect is seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GeoClass {
    Ring,
    Capsule,
    Lane,
    Sector,
    Pentagon,
    Bar,
    None,
}

pub(crate) fn boundary_class(id: SkillId, kind: EffectVisualKind) -> GeoClass {
    match kind {
        EffectVisualKind::Orb
        | EffectVisualKind::Anchor
        | EffectVisualKind::Healing
        | EffectVisualKind::Lantern
        | EffectVisualKind::Field
        | EffectVisualKind::Trap
        | EffectVisualKind::Bolt
        | EffectVisualKind::Barrier
        | EffectVisualKind::Rocket => GeoClass::Ring,
        // The replicated 0.35 of a soul is cosmetic; the pick-up reach is a server literal.
        EffectVisualKind::Soul => GeoClass::None,
        EffectVisualKind::ShieldWall => GeoClass::Bar,
        EffectVisualKind::Cage => GeoClass::Pentagon,
        EffectVisualKind::BeamWarning | EffectVisualKind::Beam => match skill(id).effect {
            SkillEffect::Technique {
                action: Technique::ConeBrittle,
                ..
            } => GeoClass::Sector,
            // Replicated with `position == end` (`common/src/skills/advanced.rs:983-984`).
            SkillEffect::Technique {
                action: Technique::BallPull,
                ..
            } => GeoClass::Ring,
            // Swept segment tests: every point within the radius of the segment
            // (`common/src/skills/mod.rs:530-540`, `:1277`; `common/src/skills/advanced.rs:1206`).
            SkillEffect::Beam { .. }
            | SkillEffect::Technique {
                action: Technique::GlacialFissure | Technique::PiercingWave,
                ..
            } => GeoClass::Capsule,
            _ => GeoClass::Lane,
        },
    }
}

/// The boundary of one received effect. A fog-clipped segment is drawn as received; a hidden
/// origin is never reconstructed.
pub(crate) fn boundary_shape(
    id: SkillId,
    kind: EffectVisualKind,
    e: &SkillEffectState,
) -> GeoShape {
    let position = Vec2::from_array(e.position);
    let end = Vec2::from_array(e.end);
    let radius = e.radius;
    match boundary_class(id, kind) {
        GeoClass::Ring => GeoShape::Ring {
            center: position,
            radius,
        },
        GeoClass::Capsule => GeoShape::Capsule {
            from: position,
            to: end,
            radius,
        },
        GeoClass::Lane => GeoShape::Lane {
            from: position,
            to: end,
            half_width: radius,
        },
        GeoClass::Sector => {
            let length = position.distance(end);
            let Some(half_angle) = cone_half_angle(id)
                .filter(|_| length >= skill(id).ability.cast_range - SECTOR_CLIP_SLACK)
            else {
                return GeoShape::Segment {
                    from: position,
                    to: end,
                };
            };
            GeoShape::Sector {
                apex: position,
                axis: (end - position) / length,
                radius: length,
                half_angle,
            }
        }
        GeoClass::Pentagon => GeoShape::Pentagon {
            center: position,
            radius,
        },
        GeoClass::Bar => {
            let Some(heading) = (end - position).try_normalize() else {
                return GeoShape::None;
            };
            let center = position + heading * WALL_AHEAD;
            let side = heading.perp() * radius;
            GeoShape::Segment {
                from: center - side,
                to: center + side,
            }
        }
        GeoClass::None => GeoShape::None,
    }
}

/// Whether a body of this archetype may be bound to a replicated kind of the skill: the
/// kind must be one the archetype can show and its boundary must be the archetype's shape.
pub(crate) fn archetype_fits(archetype: Archetype, id: SkillId, kind: EffectVisualKind) -> bool {
    use EffectVisualKind as K;
    let class = boundary_class(id, kind);
    match archetype {
        Archetype::Traveller => {
            matches!(kind, K::Bolt | K::Rocket | K::Barrier | K::Soul)
        }
        Archetype::Orbiter => matches!(kind, K::Orb | K::Barrier | K::Bolt | K::Anchor),
        Archetype::Zone => {
            matches!(kind, K::Field | K::Healing | K::Lantern | K::Anchor)
                || (kind == K::BeamWarning && class == GeoClass::Ring)
        }
        Archetype::Lane => {
            matches!(kind, K::BeamWarning | K::Beam)
                && matches!(class, GeoClass::Capsule | GeoClass::Lane)
        }
        Archetype::Sector => kind == K::BeamWarning && class == GeoClass::Sector,
        Archetype::Prop => matches!(kind, K::Trap | K::Barrier | K::Lantern),
        Archetype::Wall => kind == K::ShieldWall,
        Archetype::Cage => kind == K::Cage,
    }
}

/// What the client observed around an accepted first cast.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AreaContext {
    /// The caster's position at the edge, before any move of the same snapshot.
    pub origin: Vec2,
    /// The caster's position after a move observed with the cast.
    pub arrival: Option<Vec2>,
    /// Unit direction of the accepted cast.
    pub direction: Vec2,
    pub recast: bool,
}

/// The area an instant skill damages at its first-cast edge, from catalog values and
/// observed positions (`common/src/skills/advanced.rs:674-686`, `:810-817`, `:877-883`,
/// `:1015-1019`). A recast has no such area, and an unobserved landing yields nothing.
pub(crate) fn instant_area(id: SkillId, ctx: &AreaContext) -> Option<GeoShape> {
    if ctx.recast {
        return None;
    }
    let def = skill(id);
    let SkillEffect::Technique { action, radius, .. } = def.effect else {
        return None;
    };
    match action {
        Technique::RevealPulse => Some(GeoShape::Ring {
            center: ctx.origin,
            radius,
        }),
        Technique::Sweep => Some(GeoShape::Ring {
            center: ctx.origin,
            radius: def.ability.cast_range,
        }),
        Technique::CollisionCharge => ctx.arrival.map(|center| GeoShape::Ring { center, radius }),
        Technique::ExecuteRetreat => {
            let axis = ctx.direction.try_normalize()?;
            Some(GeoShape::Sector {
                apex: ctx.origin,
                axis,
                radius: def.ability.cast_range,
                half_angle: cone_half_angle(id)?,
            })
        }
        _ => None,
    }
}

/// Every kind a row's `body` is bound to must fit the archetype.
pub(crate) fn body_fits(archetype: Archetype, id: SkillId) -> bool {
    let kinds = own_kinds(id);
    !kinds.is_empty()
        && kinds
            .iter()
            .all(|kind| archetype_fits(archetype, id, *kind))
}

/// A unit the client sees, as the server's pick rules read it
/// (`common/src/skills/mod.rs:438-506`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PickCandidate {
    pub kind: TargetKind,
    pub id: u64,
    pub position: Vec2,
    /// The target radius the server adds to every reach.
    pub radius: f32,
    /// On the caster's team. The caster is a candidate of its own.
    pub ally: bool,
}

/// The order in which the server breaks a tie between two candidates
/// (`common/src/skills/mod.rs:38-48`).
fn pick_order(candidate: &PickCandidate) -> (u8, u64) {
    let kind = match candidate.kind {
        TargetKind::Player => 0,
        TargetKind::Minion => 1,
        TargetKind::Structure => 2,
        TargetKind::Neutral => 3,
    };
    (kind, candidate.id)
}

/// The kinds a pick looks among near the aim. A kind that is left out never shadows the
/// pick: the server filters before it ranks (`Pick`, `common/src/skills/advanced.rs:154-164`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PickKinds {
    /// Every kind the side admits.
    Any,
    /// Heroes only.
    Hero,
    /// Anything that can be moved: no structures.
    Unit,
}

/// How a pick skill chooses its unit and what it does without one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PickRule {
    /// The pick is made on the caster's own team.
    pub ally: bool,
    /// The kinds the pick is made among.
    pub kinds: PickKinds,
    /// The cast is accepted without a pick (`common/src/skills/advanced.rs:503-514`).
    pub pick_optional: bool,
}

/// The unit a pick skill takes: the candidate nearest the aim within `PICK_RADIUS` plus its
/// own radius of it and within the cast range plus its radius of the caster, among the
/// kinds of the rule; an ally pick takes heroes and minions only
/// (`select`, `common/src/skills/advanced.rs:165-197`). The server also asks for vision: the
/// caller passes the units the client sees.
pub(crate) fn server_pick(
    candidates: &[PickCandidate],
    aim: Vec2,
    origin: Vec2,
    range: f32,
    rule: PickRule,
) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            c.ally == rule.ally
                && (!rule.ally || matches!(c.kind, TargetKind::Player | TargetKind::Minion))
                && match rule.kinds {
                    PickKinds::Any => true,
                    PickKinds::Hero => c.kind == TargetKind::Player,
                    PickKinds::Unit => c.kind != TargetKind::Structure,
                }
                && c.position.distance(aim) <= PICK_RADIUS + c.radius
                && c.position.distance(origin) <= range + c.radius
        })
        .min_by(|(_, a), (_, b)| {
            a.position
                .distance(aim)
                .total_cmp(&b.position.distance(aim))
                .then_with(|| pick_order(a).cmp(&pick_order(b)))
        })
        .map(|(index, _)| index)
}

/// The pick rule of a skill that selects a unit near its aim
/// (`common/src/skills/advanced.rs:490-502`): heroes only for the duel, the curse and the
/// orb's guard, no structure for the kick, and every kind of its side for the lash and the
/// two leaps.
pub(crate) fn pick_rule(id: SkillId) -> Option<PickRule> {
    let SkillEffect::Technique { action, .. } = skill(id).effect else {
        return None;
    };
    let rule = |ally, kinds, pick_optional| PickRule {
        ally,
        kinds,
        pick_optional,
    };
    match action {
        Technique::VitalChallenge | Technique::Curse => Some(rule(false, PickKinds::Hero, false)),
        Technique::ChainKick => Some(rule(false, PickKinds::Unit, false)),
        Technique::Lash => Some(rule(false, PickKinds::Any, false)),
        Technique::GuardLeap => Some(rule(true, PickKinds::Any, true)),
        Technique::AllyLeap => Some(rule(true, PickKinds::Any, false)),
        Technique::BallGuard => Some(rule(true, PickKinds::Hero, false)),
        _ => None,
    }
}

/// The shape of the aim preview of a skill: for its first cast, or for the press its slot
/// offers while `recast` is replicated. A recast that ignores the aim previews nothing
/// (`common/src/skills/mod.rs:886-900`; `common/src/skills/advanced.rs:599-611`, `:755-761`,
/// `:803-808`); the spike recast strikes around the caster (`:617-624`) and the colossus is
/// redirected from where it stands (`:695-707`).
pub(crate) fn preview_kind(id: SkillId, recast: bool) -> PreviewShape {
    use PreviewShape as P;
    let action = match skill(id).effect {
        SkillEffect::WeaponToggle { .. } => return P::None,
        SkillEffect::LinearProjectile { .. }
        | SkillEffect::ReturningShield { .. }
        | SkillEffect::ImpactRocket { .. } => return P::Lane,
        SkillEffect::Beam { .. } => return P::LaneCapsule,
        SkillEffect::RecastZone { .. } if recast => return P::None,
        SkillEffect::RecastZone { .. } => return P::PointRing,
        SkillEffect::TrapLine { .. } => return P::TrapRow,
        SkillEffect::Technique { action, .. } => action,
    };
    match action {
        Technique::EchoStrike | Technique::Hook | Technique::GuardLeap | Technique::RevealPulse
            if recast =>
        {
            P::None
        }
        Technique::SpikeVolley if recast => P::RangeRing,
        Technique::ReturningColossus if recast => P::EffectOriginLane,
        Technique::TerrainLine
        | Technique::ReturningColossus
        | Technique::Parry
        | Technique::EchoStrike
        | Technique::SpikeVolley
        | Technique::ReturnOrb
        | Technique::CharmBolt
        | Technique::OnHitBolt
        | Technique::DetonationMark
        | Technique::Hook
        | Technique::ConcussiveBolt => P::Lane,
        Technique::PiercingWave | Technique::GlacialFissure => P::LaneCapsule,
        Technique::DaggerDeadlyBlow
        | Technique::DaggerBluff
        | Technique::DaggerBackstab
        | Technique::DaggerLethalBlow => P::LaneToPoint,
        Technique::Lantern | Technique::BallField | Technique::BallPull => P::PointRing,
        Technique::ConeBrittle | Technique::ExecuteRetreat => P::Sector,
        Technique::RevealPulse | Technique::Sweep | Technique::GuidedFires => P::SelfRing,
        Technique::SegmentCage => P::SelfPentagon,
        Technique::CollisionCharge | Technique::Lunge | Technique::SpiritDash => P::DashLanding,
        Technique::BlinkShot => P::BlinkLanding,
        Technique::VitalChallenge
        | Technique::Curse
        | Technique::Lash
        | Technique::GuardLeap
        | Technique::AllyLeap
        | Technique::BallGuard => P::UnitPick,
        Technique::ChainKick => P::PickThenLane,
        Technique::BallMove => P::EffectOriginLane,
        Technique::InterceptShield => P::WallAhead,
        Technique::DoubleStrike => P::None,
    }
}

/// A reading aid of an aim preview. It claims no area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PreviewMark {
    /// The straight ground move of the caster.
    Path { from: Vec2, to: Vec2 },
    /// Where the caster comes to stand.
    Landing(Vec2),
    /// The unit the pick rule takes, with its target radius.
    Picked { at: Vec2, radius: f32 },
    /// The way and the distance every hit unit is pushed.
    Push { from: Vec2, to: Vec2 },
}

/// What the aim preview of a held skill draws, in simulation ground coordinates.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Preview {
    pub shape: PreviewShape,
    /// The areas of the server rule. The server adds the radius of each target to them.
    pub areas: Vec<GeoShape>,
    pub marks: Vec<PreviewMark>,
    /// Index of the candidate the pick rule takes.
    pub pick: Option<usize>,
    /// The server would refuse the cast as it is aimed.
    pub refused: bool,
}

/// What the client knows while a skill key is held.
pub(crate) struct PreviewContext<'a> {
    /// The caster's position.
    pub origin: Vec2,
    /// The aim the client would send: the caster for a self cast, bounded by the cast range
    /// for a point skill.
    pub aim: Vec2,
    /// The replicated slot offers a recast.
    pub recast: bool,
    /// The caster's replicated orb.
    pub orb: Option<Vec2>,
    /// The caster's id; 0 when it is not known.
    pub hero: u64,
    /// The replicated effects.
    pub effects: &'a [SkillEffectState],
    /// The living units the client sees, the caster included.
    pub candidates: &'a [PickCandidate],
    /// Where a ground move from the first point toward the second ends
    /// (`common/src/skills/advanced.rs:277-279`).
    pub clip: &'a dyn Fn(Vec2, Vec2) -> Vec2,
}

/// The direction the server derives from two points; coincident points give +Z
/// (`common/src/skills/advanced.rs:145-152`).
fn server_direction(from: Vec2, to: Vec2) -> Vec2 {
    let delta = to - from;
    let length = delta.length();
    if length < 0.001 {
        Vec2::Y
    } else {
        delta / length
    }
}

/// The aim preview of a skill: its server rule drawn at the aim the client would send. The
/// numbers are the catalog's and the mirrored server literals; the cast range is never
/// scaled by rank for a modular skill (`shared/src/lib.rs:316-319`).
pub(crate) fn preview_shape(def: &SkillDefinition, ctx: &PreviewContext) -> Preview {
    let PreviewContext { origin, aim, .. } = *ctx;
    let mut preview = Preview {
        shape: preview_kind(def.id, ctx.recast),
        areas: Vec::new(),
        marks: Vec::new(),
        pick: None,
        refused: false,
    };
    let range = def.ability.cast_range;
    let radius = catalog_radius(def.id);
    let action = match def.effect {
        SkillEffect::Technique { action, .. } => Some(action),
        _ => None,
    };
    // A directional cast without a direction is dropped (`common/src/skills/mod.rs:920-929`,
    // `common/src/skills/advanced.rs:437-439`); the other casts fall back to +Z.
    let dir = if def.ability.targeting == TargetingMode::Direction {
        let least = if action.is_some() { 0.001 } else { 0.0001 };
        let Some(dir) = (aim - origin)
            .try_normalize()
            .filter(|_| origin.distance(aim) >= least)
        else {
            preview.refused = preview.shape != PreviewShape::None;
            return preview;
        };
        dir
    } else {
        server_direction(origin, aim)
    };
    match preview.shape {
        PreviewShape::None => {}
        PreviewShape::Lane => preview.areas.push(GeoShape::Lane {
            from: origin,
            to: origin + dir * range,
            half_width: radius,
        }),
        PreviewShape::LaneCapsule => preview.areas.push(GeoShape::Capsule {
            from: origin,
            to: origin + dir * range,
            radius,
        }),
        // The corridor ends at the aim and is never wider than the clamp
        // (`common/src/skills/dagger.rs:131-134`).
        PreviewShape::LaneToPoint => preview.areas.push(GeoShape::Lane {
            from: origin,
            to: aim,
            half_width: radius.min(DAGGER_LANE_CLAMP),
        }),
        PreviewShape::PointRing => {
            // The field and the collapse stand on the orb, which a cast puts on the caster
            // when there is none (`common/src/skills/advanced.rs:973-984`, `:2277-2305`).
            let center = match action {
                Some(Technique::BallField | Technique::BallPull) => ctx.orb.unwrap_or(origin),
                _ => aim,
            };
            preview.areas.push(GeoShape::Ring { center, radius });
        }
        PreviewShape::TrapRow => {
            if let SkillEffect::TrapLine { count, spacing, .. } = def.effect {
                // `common/src/skills/mod.rs:930-934`, `:1022-1026`.
                let along = if origin.distance(aim) > 0.0001 {
                    (aim - origin).normalize()
                } else {
                    Vec2::Y
                };
                let middle = (f32::from(count) - 1.0) * 0.5;
                preview.areas.extend((0..count).map(|n| GeoShape::Ring {
                    center: aim + along.perp() * (f32::from(n) - middle) * spacing,
                    radius,
                }));
            }
        }
        PreviewShape::Sector => {
            if let Some(half_angle) = cone_half_angle(def.id) {
                preview.areas.push(GeoShape::Sector {
                    apex: origin,
                    axis: dir,
                    radius: range,
                    half_angle,
                });
            }
            if action == Some(Technique::ExecuteRetreat) {
                let behind = (ctx.clip)(origin, origin - dir * NIGHTFALL_RETREAT);
                preview.marks.push(PreviewMark::Landing(behind));
            }
        }
        PreviewShape::SelfRing => {
            // The pulse uses its radius; the sweep and the fires use the cast range
            // (`common/src/skills/advanced.rs:814`, `:1018`, `:912-920`).
            let reach = if action == Some(Technique::RevealPulse) {
                radius
            } else {
                range
            };
            preview.areas.push(GeoShape::Ring {
                center: origin,
                radius: reach,
            });
            if action == Some(Technique::Sweep) {
                preview.marks.push(PreviewMark::Push {
                    from: origin,
                    to: origin + dir * SWEEP_PUSH,
                });
            }
        }
        PreviewShape::SelfPentagon => preview.areas.push(GeoShape::Pentagon {
            center: origin,
            radius,
        }),
        PreviewShape::RangeRing => preview.areas.push(GeoShape::Ring {
            center: origin,
            radius: range,
        }),
        PreviewShape::DashLanding => {
            // A charge runs its whole range along the aim; the others go to the aim point
            // (`common/src/skills/advanced.rs:570-574`, `:674`, `:726`, `:904`).
            let goal = if def.ability.targeting == TargetingMode::Direction {
                origin + dir * range
            } else {
                aim
            };
            let landing = (ctx.clip)(origin, goal);
            preview.marks.push(PreviewMark::Path {
                from: origin,
                to: landing,
            });
            preview.areas.push(GeoShape::Ring {
                center: landing,
                radius,
            });
        }
        // A blink is not clipped: an illegal landing drops the cast
        // (`common/src/skills/advanced.rs:458-460`, `:902`).
        PreviewShape::BlinkLanding => {
            preview.marks.push(PreviewMark::Landing(aim));
            preview.areas.push(GeoShape::Ring {
                center: aim,
                radius,
            });
        }
        PreviewShape::UnitPick | PreviewShape::PickThenLane => {
            let Some(rule) = pick_rule(def.id) else {
                return preview;
            };
            preview.areas.push(GeoShape::Ring {
                center: aim,
                radius: PICK_RADIUS,
            });
            preview.pick = server_pick(ctx.candidates, aim, origin, range, rule);
            let picked = preview.pick.map(|index| ctx.candidates[index]);
            if let Some(unit) = picked {
                preview.marks.push(PreviewMark::Picked {
                    at: unit.position,
                    radius: unit.radius,
                });
            }
            // A unit of a kind the rule leaves out is never picked, so only the lack of a
            // pick refuses the cast (`common/src/skills/advanced.rs:503-514`).
            preview.refused = picked.is_none() && !rule.pick_optional;
            match action {
                // The leap goes to the picked unit, or to the aim point without one
                // (`common/src/skills/advanced.rs:763`, `:776`).
                Some(Technique::GuardLeap) => {
                    let goal = picked.map_or(aim, |unit| unit.position);
                    preview
                        .marks
                        .push(PreviewMark::Landing((ctx.clip)(origin, goal)));
                }
                // The orb flies from where it is to the picked hero
                // (`common/src/skills/advanced.rs:947-971`, `:1933-1938`).
                Some(Technique::BallGuard) => {
                    if let Some(ally) = picked {
                        preview.areas.push(GeoShape::Lane {
                            from: ctx.orb.unwrap_or(origin),
                            to: ally.position,
                            half_width: radius,
                        });
                    }
                }
                // The kicked unit is carried away from the caster and strikes what it
                // passes (`common/src/skills/advanced.rs:834-846`).
                Some(Technique::ChainKick) => {
                    if let Some(unit) = picked {
                        let away = server_direction(origin, unit.position);
                        preview.areas.push(GeoShape::Lane {
                            from: unit.position,
                            to: (ctx.clip)(unit.position, unit.position + away * KICK_LENGTH),
                            half_width: radius,
                        });
                    }
                }
                _ => {}
            }
        }
        PreviewShape::EffectOriginLane => {
            if action == Some(Technique::ReturningColossus) {
                // The server drops the recast outside the gate and redirects its first
                // effect of the skill along the caster's aim
                // (`common/src/skills/advanced.rs:503-512`, `:696-703`).
                let body = ctx
                    .effects
                    .iter()
                    .filter(|effect| effect.owner_id == ctx.hero && effect.skill == def.id)
                    .min_by_key(|effect| effect.id)
                    .filter(|_| {
                        super::status::recast_in_reach(def.id, ctx.hero, origin, ctx.effects)
                    });
                let Some(body) = body else {
                    preview.shape = PreviewShape::None;
                    return preview;
                };
                let from = Vec2::from_array(body.position);
                preview.areas.push(GeoShape::Lane {
                    from,
                    to: from + dir * range,
                    half_width: radius,
                });
            } else {
                // The orb flies from where it is to the aim point
                // (`common/src/skills/advanced.rs:947-962`).
                preview.areas.push(GeoShape::Lane {
                    from: ctx.orb.unwrap_or(origin),
                    to: aim,
                    half_width: radius,
                });
            }
        }
        PreviewShape::WallAhead => {
            let center = origin + dir * WALL_AHEAD;
            let side = dir.perp() * radius;
            preview.areas.push(GeoShape::Segment {
                from: center - side,
                to: center + side,
            });
        }
    }
    preview
}

#[cfg(test)]
mod preview_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn effect(id: SkillId, kind: EffectVisualKind) -> SkillEffectState {
        SkillEffectState {
            id: 1,
            owner_id: 7,
            owner_team: shared::map::Team::Green,
            skill: id,
            kind,
            position: [3.0, -2.0],
            end: [3.0, 5.0],
            radius: 1.5,
            remaining_secs: 1.0,
            armed: true,
            consumed_segments: 0,
        }
    }

    #[test]
    fn boundaries_use_only_received_fields() {
        use EffectVisualKind as K;
        let position = Vec2::new(3.0, -2.0);
        let end = Vec2::new(3.0, 5.0);
        for (id, kind) in [
            (SkillId::DawnBind, K::Bolt),
            (SkillId::WildRocket, K::Rocket),
            (SkillId::DawnBarrier, K::Barrier),
            (SkillId::MirrorGuard, K::Barrier),
            (SkillId::DawnField, K::Field),
            (SkillId::WildTraps, K::Trap),
            (SkillId::GuidingLantern, K::Lantern),
            (SkillId::OrbitalCommand, K::Orb),
            (SkillId::AnchorStep, K::Anchor),
            (SkillId::FourfoldDuel, K::Healing),
            (SkillId::OrbitalCollapse, K::BeamWarning),
        ] {
            assert_eq!(
                boundary_shape(id, kind, &effect(id, kind)),
                GeoShape::Ring {
                    center: position,
                    radius: 1.5
                },
                "{}",
                id.id()
            );
        }
        for (id, kind) in [
            (SkillId::DawnRay, K::BeamWarning),
            (SkillId::DawnRay, K::Beam),
            (SkillId::HorizonWave, K::BeamWarning),
            (SkillId::WinterDivide, K::BeamWarning),
        ] {
            assert_eq!(
                boundary_shape(id, kind, &effect(id, kind)),
                GeoShape::Capsule {
                    from: position,
                    to: end,
                    radius: 1.5
                },
                "{}",
                id.id()
            );
        }
        assert_eq!(
            boundary_shape(
                SkillId::IronBoundary,
                K::Cage,
                &effect(SkillId::IronBoundary, K::Cage)
            ),
            GeoShape::Pentagon {
                center: position,
                radius: 1.5
            }
        );
        // The replicated radius of a soul is not a rule the server applies.
        assert_eq!(
            boundary_shape(
                SkillId::IronHook,
                K::Soul,
                &effect(SkillId::IronHook, K::Soul)
            ),
            GeoShape::None
        );
    }

    #[test]
    fn the_wall_bar_stands_one_unit_ahead_at_the_replicated_half_length() {
        let mut e = effect(SkillId::Northwall, EffectVisualKind::ShieldWall);
        e.position = [1.0, 1.0];
        e.end = [1.0, 3.5];
        e.radius = 2.5;
        assert_eq!(
            boundary_shape(SkillId::Northwall, e.kind, &e),
            GeoShape::Segment {
                from: Vec2::new(3.5, 2.0),
                to: Vec2::new(-1.5, 2.0)
            }
        );
        e.end = e.position;
        assert_eq!(
            boundary_shape(SkillId::Northwall, e.kind, &e),
            GeoShape::None
        );
    }

    #[test]
    fn the_cone_is_a_sector_at_full_range_and_a_segment_when_fog_cut_it() {
        let range = skill(SkillId::FurnaceBreath).ability.cast_range;
        let mut e = effect(SkillId::FurnaceBreath, EffectVisualKind::BeamWarning);
        e.position = [0.0, 0.0];
        e.end = [0.0, range];
        let GeoShape::Sector {
            apex,
            axis,
            radius,
            half_angle,
        } = boundary_shape(SkillId::FurnaceBreath, e.kind, &e)
        else {
            panic!("full-length cone must be a sector");
        };
        assert_eq!((apex, axis, radius), (Vec2::ZERO, Vec2::Y, range));
        assert!((half_angle.cos() - FURNACE_CONE_COS).abs() < 1e-6);
        // Inside the slack the cone is still complete; below it only the received line is known.
        e.end = [0.0, range - 0.04];
        assert!(matches!(
            boundary_shape(SkillId::FurnaceBreath, e.kind, &e),
            GeoShape::Sector { .. }
        ));
        e.end = [0.0, range - 0.06];
        assert_eq!(
            boundary_shape(SkillId::FurnaceBreath, e.kind, &e),
            GeoShape::Segment {
                from: Vec2::ZERO,
                to: Vec2::new(0.0, range - 0.06)
            }
        );
    }

    #[test]
    fn every_kind_of_every_skill_has_one_fitting_archetype_family() {
        use EffectVisualKind as K;
        let fitting = |id: SkillId, kind: K| -> Vec<Archetype> {
            Archetype::ALL
                .iter()
                .copied()
                .filter(|archetype| archetype_fits(*archetype, id, kind))
                .collect()
        };
        assert_eq!(fitting(SkillId::DawnRay, K::Beam), [Archetype::Lane]);
        assert_eq!(
            fitting(SkillId::WinterDivide, K::BeamWarning),
            [Archetype::Lane]
        );
        assert_eq!(
            fitting(SkillId::FurnaceBreath, K::BeamWarning),
            [Archetype::Sector]
        );
        assert_eq!(
            fitting(SkillId::OrbitalCollapse, K::BeamWarning),
            [Archetype::Zone]
        );
        assert_eq!(
            fitting(SkillId::Northwall, K::ShieldWall),
            [Archetype::Wall]
        );
        assert_eq!(fitting(SkillId::IronBoundary, K::Cage), [Archetype::Cage]);
        assert_eq!(fitting(SkillId::WildTraps, K::Trap), [Archetype::Prop]);
        assert_eq!(
            fitting(SkillId::MirrorGuard, K::Barrier),
            [Archetype::Traveller, Archetype::Orbiter, Archetype::Prop]
        );
        assert_eq!(
            fitting(SkillId::WinterShard, K::Bolt),
            [Archetype::Traveller, Archetype::Orbiter]
        );
        assert_eq!(
            fitting(SkillId::AnchorStep, K::Anchor),
            [Archetype::Orbiter, Archetype::Zone]
        );
        assert_eq!(fitting(SkillId::IronHook, K::Soul), [Archetype::Traveller]);
        // No modular skill replicates a kind without a legal archetype.
        for id in SkillId::ALL {
            for kind in own_kinds(id)
                .iter()
                .chain(super::super::category::aux_kinds(id))
            {
                assert!(!fitting(id, *kind).is_empty(), "{} {kind:?}", id.id());
            }
        }
        assert!(body_fits(Archetype::Lane, SkillId::DawnRay));
        assert!(!body_fits(Archetype::Zone, SkillId::DawnRay));
        assert!(!body_fits(Archetype::Traveller, SkillId::ThunderPulse));
    }

    #[test]
    fn instant_areas_come_from_the_catalog_and_observed_positions() {
        let ctx = AreaContext {
            origin: Vec2::new(2.0, 3.0),
            arrival: Some(Vec2::new(9.0, 3.0)),
            direction: Vec2::X,
            recast: false,
        };
        assert_eq!(
            instant_area(SkillId::ThunderPulse, &ctx),
            Some(GeoShape::Ring {
                center: ctx.origin,
                radius: 5.0
            })
        );
        assert_eq!(
            instant_area(SkillId::ChainSweep, &ctx),
            Some(GeoShape::Ring {
                center: ctx.origin,
                radius: 6.0
            })
        );
        assert_eq!(
            instant_area(SkillId::AnvilCharge, &ctx),
            Some(GeoShape::Ring {
                center: Vec2::new(9.0, 3.0),
                radius: 3.0
            })
        );
        let Some(GeoShape::Sector {
            apex,
            axis,
            radius,
            half_angle,
        }) = instant_area(SkillId::Nightfall, &ctx)
        else {
            panic!("the retreat strike is a sector");
        };
        assert_eq!((apex, axis, radius), (ctx.origin, Vec2::X, 8.0));
        assert!((half_angle.cos() - NIGHTFALL_SECTOR_COS).abs() < 1e-6);
        // Nothing at a departure point, on a recast, or for a skill without an instant area.
        let unseen = AreaContext {
            arrival: None,
            ..ctx
        };
        assert_eq!(instant_area(SkillId::AnvilCharge, &unseen), None);
        let again = AreaContext {
            recast: true,
            ..ctx
        };
        for id in SkillId::ALL {
            assert_eq!(instant_area(id, &again), None, "{}", id.id());
        }
        let with_area: Vec<_> = SkillId::ALL
            .into_iter()
            .filter(|id| instant_area(*id, &ctx).is_some())
            .map(SkillId::id)
            .collect();
        assert_eq!(
            with_area,
            ["anvil_charge", "thunder_pulse", "nightfall", "chain_sweep"]
        );
    }

    #[test]
    fn outlines_lie_on_the_boundary_they_draw() {
        let from = Vec2::new(3.0, -2.0);
        let to = Vec2::new(-4.0, 6.0);
        let along = (to - from).normalize();
        let length = from.distance(to);
        let closed = |line: &[Vec2]| line.first().unwrap().distance(*line.last().unwrap()) < 1e-4;
        // Distance of a point from the segment.
        let off_segment = |point: Vec2| {
            let t = (point - from).dot(along).clamp(0.0, length);
            point.distance(from + along * t)
        };

        for radius in [0.5, 3.0, 9.0] {
            let ring = GeoShape::Ring {
                center: from,
                radius,
            }
            .outline();
            assert_eq!(ring.len(), 1);
            assert!(closed(&ring[0]) && ring[0].len() == OUTLINE_STEPS + 1);
            for point in &ring[0] {
                assert!((point.distance(from) - radius).abs() < 1e-4);
            }

            // Every point of a capsule outline is `radius` from the segment, and both
            // round ends are drawn.
            let capsule = GeoShape::Capsule { from, to, radius }.outline();
            assert_eq!(capsule.len(), 1);
            assert!(closed(&capsule[0]));
            for point in &capsule[0] {
                assert!((off_segment(*point) - radius).abs() < 1e-3, "{point}");
            }
            for tip in [to + along * radius, from - along * radius] {
                assert!(capsule[0].iter().any(|point| point.distance(tip) < 1e-3));
            }
            // A capsule without a length is the circle around its point.
            let dot = GeoShape::Capsule {
                from,
                to: from,
                radius,
            };
            assert_eq!(dot.outline(), ring);

            // A lane has flat ends: four corners and nothing beyond them.
            let lane = GeoShape::Lane {
                from,
                to,
                half_width: radius,
            }
            .outline();
            let side = along.perp() * radius;
            assert_eq!(
                lane,
                [vec![
                    from + side,
                    to + side,
                    to - side,
                    from - side,
                    from + side
                ]]
            );
        }

        // A sector runs from its apex along both edges to an arc at the full radius.
        let half_angle = FURNACE_CONE_COS.acos();
        let sector = GeoShape::Sector {
            apex: from,
            axis: along,
            radius: 9.0,
            half_angle,
        }
        .outline();
        assert_eq!(sector.len(), 1);
        let line = &sector[0];
        assert_eq!((line[0], *line.last().unwrap()), (from, from));
        let arc = &line[1..line.len() - 1];
        for point in arc {
            assert!((point.distance(from) - 9.0).abs() < 1e-4);
            assert!((*point - from).angle_to(along).abs() <= half_angle + 1e-4);
        }
        for (edge, sign) in [(arc[0], -1.0), (*arc.last().unwrap(), 1.0)] {
            assert!(((edge - from).angle_to(along) + sign * half_angle).abs() < 1e-4);
        }

        // The cage is five separate sides between the corners the server tests
        // (`common/src/skills/advanced.rs:1108-1112`).
        let cage = GeoShape::Pentagon {
            center: from,
            radius: 6.0,
        }
        .outline();
        assert_eq!(cage.len(), 5);
        for (side, line) in cage.iter().enumerate() {
            let corner = |i: usize| from + Vec2::from_angle(TAU * i as f32 / 5.0) * 6.0;
            assert_eq!(line.len(), 2);
            assert!(line[0].distance(corner(side)) < 1e-4);
            assert!(line[1].distance(corner(side + 1)) < 1e-4);
        }

        assert_eq!(GeoShape::Segment { from, to }.outline(), [vec![from, to]]);
        assert!(GeoShape::None.outline().is_empty());
    }

    /// Whether the in-process authority damages a hero who stands still at `offset` from
    /// the caster of `id`, cast toward `aim` from the caster, within 1.5 s of the cast.
    fn authority_hits(class: shared::HeroClass, id: SkillId, aim: Vec2, offset: Vec2) -> bool {
        use common::offline::{EPOCH, LOCAL_ADDR, PracticeSession};
        use shared::practice::PracticeCommand;
        use shared::wire::{CharacterChoice, ClientPacket};

        let slot = shared::loadout::preset_for_class(class)
            .unwrap()
            .skills()
            .iter()
            .position(|skill| *skill == id)
            .unwrap() as u8;
        let mut session = PracticeSession::new(std::time::Instant::now());
        session.command(ClientPacket::Join {
            handheld: Default::default(),
            prematch: false,
            team: shared::map::Team::Green,
            character: CharacterChoice::Ipfs,
            hero_class: class,
            avatar: None,
            sprite_character: None,
            session_id: None,
            passport_ticket: None,
        });
        for command in [PracticeCommand::ClearBots, PracticeCommand::SpawnDummy] {
            session.command(ClientPacket::Practice { command });
        }
        // Nothing else may move or hurt the target.
        session.bots = Default::default();
        session.world.structures.clear();
        session.world.minions.clear();
        session.world.neutrals.clear();
        let caster = &session.world.players[&LOCAL_ADDR].hero;
        let origin = Vec2::new(caster.x, caster.z);
        let target = session
            .world
            .players
            .values_mut()
            .find(|player| player.hero.identity.is_bot)
            .unwrap();
        let target_id = target.hero.identity.id;
        target.hero.x = origin.x + offset.x;
        target.hero.z = origin.y + offset.y;
        let full = target.hero.hp;
        session.command(ClientPacket::CastSkill {
            slot,
            aim: (origin + aim).to_array(),
            server_epoch: EPOCH,
            match_id: 1,
            request_id: 1,
        });
        for _ in 0..30 {
            session.advance(0.05);
        }
        let target = session
            .world
            .players
            .values()
            .find(|player| player.hero.identity.id == target_id)
            .unwrap();
        // The target stood still, so the answer is about that one place.
        assert!((target.hero.x - origin.x - offset.x).abs() < 1e-3);
        target.hero.hp < full
    }

    /// Parity with the in-process authority for the cone the flat view now draws: Furnace
    /// Breath hits a target one degree inside either edge of the drawn sector and misses
    /// one a degree outside it.
    #[test]
    fn the_drawn_cone_has_the_edges_the_authority_hits_within() {
        let id = SkillId::FurnaceBreath;
        // Whether the breath aimed along +X damages a hero standing `offset` from the caster.
        let hits = |offset: Vec2| {
            authority_hits(shared::HeroClass::Cinderforge, id, Vec2::X * 5.0, offset)
        };

        let GeoShape::Sector {
            radius, half_angle, ..
        } = boundary_shape(
            id,
            EffectVisualKind::BeamWarning,
            &SkillEffectState {
                end: [3.0 + skill(id).ability.cast_range, -2.0],
                ..effect(id, EffectVisualKind::BeamWarning)
            },
        )
        else {
            panic!("a full-length cone is a sector");
        };
        assert_eq!(radius, 7.0);
        let at = |angle: f32| Vec2::from_angle(angle) * 4.0;
        let margin = 1.0_f32.to_radians();
        assert!(hits(at(0.0)));
        for side in [-1.0, 1.0] {
            assert!(hits(at(side * (half_angle - margin))), "inside {side}");
            assert!(!hits(at(side * (half_angle + margin))), "outside {side}");
        }
        // Behind the caster nothing is hit, however near.
        assert!(!hits(Vec2::new(-1.5, 0.0)));
    }

    /// Parity with the in-process authority for the cage the body draws: a hero is struck
    /// within `CAGE_BAR_HALF_WIDTH` plus its own radius of a side of the pentagon whose
    /// first corner points along world +X, and not beyond.
    #[test]
    fn the_drawn_cage_has_the_band_the_authority_hits_within() {
        let id = SkillId::IronBoundary;
        let hits =
            |offset: Vec2| authority_hits(shared::HeroClass::Chainkeeper, id, Vec2::ZERO, offset);
        let radius = replicated_radius(id, EffectVisualKind::Cage);
        assert_eq!(radius, 5.0);
        let GeoShape::Pentagon {
            radius: corners, ..
        } = boundary_shape(
            id,
            EffectVisualKind::Cage,
            &SkillEffectState {
                radius,
                ..effect(id, EffectVisualKind::Cage)
            },
        )
        else {
            panic!("a cage is a pentagon");
        };
        assert_eq!(corners, radius);
        // The middle of side 0 lies between the corners at 0 and 72 degrees.
        let outward = Vec2::from_angle(TAU / 10.0);
        let side = radius * (PI / 5.0).cos();
        let reach = CAGE_BAR_HALF_WIDTH + common::balance::PLAYER_HIT_RADIUS;
        for toward in [-1.0, 1.0] {
            assert!(hits(outward * (side + toward * (reach - 0.03))), "{toward}");
            assert!(
                !hits(outward * (side + toward * (reach + 0.03))),
                "{toward}"
            );
        }
        // The orientation: just beyond the reach of a side there is a corner along +X and
        // none along the middle of a side.
        let beyond = radius + reach - 0.03;
        assert!(hits(Vec2::X * beyond));
        assert!(!hits(outward * beyond));
        // Nothing happens at the keeper.
        assert!(!hits(Vec2::new(0.5, 0.5)));
    }

    /// A change of a mirrored literal must be deliberate. Parity against the in-process
    /// authority is tested where each literal gets its first consumer.
    #[test]
    fn mirrored_literals_are_pinned() {
        assert_eq!(PICK_RADIUS, 2.0);
        assert_eq!(FURNACE_CONE_COS, 0.6);
        assert_eq!(NIGHTFALL_SECTOR_COS, 0.2);
        assert_eq!(DAGGER_LANE_CLAMP, 0.55);
        assert_eq!(KICK_LENGTH, 10.0);
        assert_eq!(NIGHTFALL_RETREAT, 7.0);
        assert_eq!(RECAST_GATE_MOUNTAIN_ECHO, 4.0);
        assert_eq!(CAGE_BAR_HALF_WIDTH, 0.4);
        assert_eq!(WALL_AHEAD, 1.0);
        assert_eq!(SWEEP_PUSH, 3.0);
        assert_eq!(SECTOR_CLIP_SLACK, 0.05);
        assert_eq!(
            [ORB_RADIUS, HEALING_RADIUS, ANCHOR_RADIUS, SOUL_RADIUS],
            [0.65, 5.0, 0.5, 0.35]
        );
    }

    /// Parity with the in-process authority: every effect a default kit replicates in the
    /// first second after its cast carries the radius the parser sizes its body against.
    #[test]
    fn replicated_radii_equal_what_the_authority_sends() {
        use common::offline::{EPOCH, LOCAL_ADDR, PracticeSession};
        use shared::practice::PracticeCommand;
        use shared::wire::{CharacterChoice, ClientPacket, ServerPacket};

        let mut seen = std::collections::BTreeSet::new();
        for class in shared::HeroClass::ALL {
            let Some(kit) = shared::loadout::preset_for_class(class) else {
                continue;
            };
            // The dummy stands in the aim, so skills that need a unit find one: near for
            // the short picks, far enough for a rocket to be seen in flight.
            let casts = (0..kit.skills().len() as u8)
                .flat_map(|slot| [3.0, 12.0].map(|distance| (slot, distance)));
            for (slot, distance) in casts {
                let mut session = PracticeSession::new(std::time::Instant::now());
                session.command(ClientPacket::Join {
                    handheld: Default::default(),
                    prematch: false,
                    team: shared::map::Team::Green,
                    character: CharacterChoice::Ipfs,
                    hero_class: class,
                    avatar: None,
                    sprite_character: None,
                    session_id: None,
                    passport_ticket: None,
                });
                for command in [PracticeCommand::ClearBots, PracticeCommand::SpawnDummy] {
                    session.command(ClientPacket::Practice { command });
                }
                session.bots = Default::default();
                let caster = &session.world.players[&LOCAL_ADDR].hero;
                let origin = Vec2::new(caster.x, caster.z);
                let target = session
                    .world
                    .players
                    .values_mut()
                    .find(|player| player.hero.identity.is_bot)
                    .unwrap();
                target.hero.x = origin.x + distance;
                target.hero.z = origin.y;
                session.command(ClientPacket::CastSkill {
                    slot,
                    aim: [origin.x + distance, origin.y],
                    server_epoch: EPOCH,
                    match_id: 1,
                    request_id: 1,
                });
                for _ in 0..20 {
                    session.advance(0.05);
                    let ServerPacket::Snapshot { skill_effects, .. } = session.snapshot() else {
                        panic!("practice publishes a snapshot");
                    };
                    for effect in skill_effects {
                        assert_eq!(
                            effect.radius,
                            replicated_radius(effect.skill, effect.kind),
                            "{} {:?}",
                            effect.skill.id(),
                            effect.kind
                        );
                        seen.insert((
                            effect.skill.id(),
                            super::super::category::kind_id(effect.kind),
                        ));
                    }
                }
            }
        }
        // The loop is not vacuous: travelling bodies, zones, props and auxiliary objects
        // were all replicated.
        for expected in [
            ("dawn_bind", "bolt"),
            ("dawn_field", "field"),
            ("wild_traps", "trap"),
            ("wild_rocket", "rocket"),
            ("dawn_barrier", "barrier"),
            ("guiding_lantern", "lantern"),
            ("iron_boundary", "cage"),
            ("northwall", "shield_wall"),
            ("orbital_command", "orb"),
            ("anchor_step", "anchor"),
        ] {
            assert!(
                seen.contains(&expected),
                "{expected:?} was not replicated: {seen:?}"
            );
        }
    }
}
