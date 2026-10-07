//! One table for every boundary and area a skill may draw. Shapes are functions of the
//! received effect fields and the catalog only; presentation data cannot scale or move them.
// The body renderer, the 2D fallback, the area flash and the aim preview read this table;
// until they land only the archetype check of the parser does.
#![cfg_attr(not(test), allow(dead_code))]

use super::category::{cone_half_angle, own_kinds};
use super::vocab::Archetype;
use bevy::math::Vec2;
use shared::loadout::{EffectVisualKind, SkillEffect, SkillEffectState, SkillId, Technique, skill};

/// A unit pick takes the nearest candidate within this distance of the aim
/// (`common/src/skills/advanced.rs:168`).
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
/// Rule F: a cone whose received axis is shorter than the cast range by more than this was
/// cut by fog and is drawn as a plain segment.
pub(crate) const SECTOR_CLIP_SLACK: f32 = 0.05;

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
        assert_eq!(SECTOR_CLIP_SLACK, 0.05);
    }
}
