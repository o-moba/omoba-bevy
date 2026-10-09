//! Facts about a skill that presentation data may not author, derived from the shared
//! catalog. Each match is exhaustive, so a new effect or technique does not compile until
//! its answer is written here. Server anchors name the rule each answer mirrors.
use super::vocab::{MotionPhase, StageRule};
use shared::loadout::{EffectVisualKind, SkillEffect, SkillId, Technique, skill};
use shared::{HeroClass, SkillSlot, TargetingMode};

/// A row of the registry: a modular skill or an ability of one of the five legacy classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SkillKey {
    Modular(SkillId),
    Legacy(HeroClass, SkillSlot),
}

impl SkillKey {
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        SkillId::from_id(id).map(Self::Modular).or_else(|| {
            HeroClass::LEGACY.into_iter().find_map(|class| {
                SkillSlot::ALL
                    .into_iter()
                    .find(|slot| class.ability(*slot).id == id)
                    .map(|slot| Self::Legacy(class, slot))
            })
        })
    }

    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Modular(skill) => skill.id(),
            Self::Legacy(class, slot) => class.ability(slot).id,
        }
    }

    /// The class whose default kit owns the skill; it selects the theme.
    pub(crate) fn home(self) -> HeroClass {
        match self {
            Self::Legacy(class, _) => class,
            Self::Modular(skill) => HeroClass::ALL
                .into_iter()
                .find(|class| {
                    shared::loadout::preset_for_class(*class)
                        .is_some_and(|kit| kit.skills().contains(&skill))
                })
                .expect("every modular skill belongs to one class preset"),
        }
    }

    pub(crate) fn modular(self) -> Option<SkillId> {
        match self {
            Self::Modular(skill) => Some(skill),
            Self::Legacy(..) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Category {
    /// The cast leaves a replicated world effect.
    ReplicatedEffect,
    /// A modular skill that resolves at once and replicates no effect of its own.
    InstantModular,
    /// A unit-target projectile of a legacy class.
    LegacyProjectile,
    /// A self heal or mana restore of a legacy class.
    LegacySelfCast,
}

pub(crate) fn category(key: SkillKey) -> Category {
    match key {
        SkillKey::Legacy(class, slot) => {
            if class.ability(slot).targeting == TargetingMode::SelfTarget {
                Category::LegacySelfCast
            } else {
                Category::LegacyProjectile
            }
        }
        SkillKey::Modular(skill) => {
            if own_kinds(skill).is_empty() {
                Category::InstantModular
            } else {
                Category::ReplicatedEffect
            }
        }
    }
}

/// Kinds the effect created by the first cast can replicate
/// (`common/src/skills/mod.rs:1457-1490`; the `persistent` arms of
/// `common/src/skills/advanced.rs:588-1039`). A row's `body` is bound to these.
pub(crate) fn own_kinds(id: SkillId) -> &'static [EffectVisualKind] {
    use EffectVisualKind as K;
    match skill(id).effect {
        SkillEffect::LinearProjectile { .. } => &[K::Bolt],
        SkillEffect::ReturningShield { .. } => &[K::Barrier],
        SkillEffect::RecastZone { .. } => &[K::Field],
        SkillEffect::Beam { .. } => &[K::BeamWarning, K::Beam],
        SkillEffect::TrapLine { .. } => &[K::Trap],
        SkillEffect::ImpactRocket { .. } => &[K::Rocket],
        SkillEffect::WeaponToggle { .. } => &[],
        SkillEffect::Technique { action, .. } => match action {
            Technique::TerrainLine => &[K::Trap],
            Technique::Lantern => &[K::Lantern],
            Technique::SegmentCage => &[K::Cage],
            Technique::BallField => &[K::Field],
            Technique::InterceptShield => &[K::ShieldWall],
            Technique::Parry => &[K::Barrier],
            Technique::GlacialFissure | Technique::ConeBrittle | Technique::BallPull => {
                &[K::BeamWarning]
            }
            // The wave's warning is its `aux.beam_warning`; the body is the travelling wave.
            Technique::PiercingWave
            | Technique::OnHitBolt
            | Technique::DetonationMark
            | Technique::CharmBolt
            | Technique::ConcussiveBolt
            | Technique::ReturnOrb
            | Technique::EchoStrike
            | Technique::Hook
            | Technique::SpikeVolley
            | Technique::ReturningColossus => &[K::Bolt],
            Technique::CollisionCharge
            | Technique::Lunge
            | Technique::DoubleStrike
            | Technique::VitalChallenge
            | Technique::GuardLeap
            | Technique::RevealPulse
            | Technique::ChainKick
            | Technique::Curse
            | Technique::Lash
            | Technique::ExecuteRetreat
            | Technique::GuidedFires
            | Technique::SpiritDash
            | Technique::BallMove
            | Technique::BallGuard
            | Technique::BlinkShot
            | Technique::Sweep
            | Technique::AllyLeap
            | Technique::DaggerDeadlyBlow
            | Technique::DaggerBluff
            | Technique::DaggerBackstab
            | Technique::DaggerLethalBlow => &[],
        },
    }
}

/// Secondary objects a row may give a body through `aux`. The server tags its healing
/// field, anchor and souls with fixed skills and the orb with the skill that last ordered
/// it (`common/src/skills/advanced.rs:2364-2411`).
pub(crate) fn aux_kinds(id: SkillId) -> &'static [EffectVisualKind] {
    use EffectVisualKind as K;
    match id {
        SkillId::FourfoldDuel => return &[K::Healing],
        SkillId::AnchorStep => return &[K::Anchor],
        SkillId::IronHook => return &[K::Soul],
        _ => {}
    }
    match skill(id).effect {
        SkillEffect::Technique {
            action: Technique::BallMove | Technique::BallGuard,
            ..
        } => &[K::Orb],
        SkillEffect::Technique {
            action: Technique::PiercingWave,
            ..
        } => &[K::BeamWarning],
        _ => &[],
    }
}

/// Auxiliary objects get positional ids, so one instance cannot be followed between
/// snapshots by its id (`common/src/skills/advanced.rs:2347`).
pub(crate) const fn unstable_id(kind: EffectVisualKind) -> bool {
    matches!(
        kind,
        EffectVisualKind::Orb
            | EffectVisualKind::Soul
            | EffectVisualKind::Anchor
            | EffectVisualKind::Healing
    )
}

/// Kinds replicated with `end` one unit ahead of `position`: a heading and nothing more
/// (`common/src/skills/mod.rs:1498-1503`).
pub(crate) const fn heading_only(kind: EffectVisualKind) -> bool {
    matches!(
        kind,
        EffectVisualKind::Bolt | EffectVisualKind::Barrier | EffectVisualKind::Rocket
    )
}

/// Whether an instance of this kind travels. Such a body leaves the snapshot when it
/// reaches something far more often than when its time runs out, and the client cannot
/// tell the two apart.
pub(crate) fn travels(id: SkillId, kind: EffectVisualKind) -> bool {
    use EffectVisualKind as K;
    match kind {
        K::Bolt | K::Rocket | K::Soul | K::Orb => true,
        // The returning shield flies out and back; a parry stance stands where it was cast.
        K::Barrier => matches!(skill(id).effect, SkillEffect::ReturningShield { .. }),
        K::Anchor
        | K::Healing
        | K::ShieldWall
        | K::Cage
        | K::Lantern
        | K::Field
        | K::BeamWarning
        | K::Beam
        | K::Trap => false,
    }
}

/// The wire spelling of a kind; `aux` keys use it.
pub(crate) const fn kind_id(kind: EffectVisualKind) -> &'static str {
    match kind {
        EffectVisualKind::Orb => "orb",
        EffectVisualKind::Soul => "soul",
        EffectVisualKind::Anchor => "anchor",
        EffectVisualKind::Healing => "healing",
        EffectVisualKind::ShieldWall => "shield_wall",
        EffectVisualKind::Cage => "cage",
        EffectVisualKind::Lantern => "lantern",
        EffectVisualKind::Bolt => "bolt",
        EffectVisualKind::Barrier => "barrier",
        EffectVisualKind::Field => "field",
        EffectVisualKind::BeamWarning => "beam_warning",
        EffectVisualKind::Beam => "beam",
        EffectVisualKind::Trap => "trap",
        EffectVisualKind::Rocket => "rocket",
    }
}

/// How the stage of one replicated instance is read.
pub(crate) fn stage_rule(id: SkillId, kind: EffectVisualKind) -> StageRule {
    if unstable_id(kind) {
        return StageRule::Active;
    }
    match skill(id).effect {
        // Armed after `arm_secs` (`common/src/skills/mod.rs:1028`).
        SkillEffect::TrapLine { .. } => StageRule::ArmedGate,
        // A warning until it fires, then a beam on the same id
        // (`common/src/skills/mod.rs:1479-1484`).
        SkillEffect::Beam { .. } => StageRule::KindGate,
        SkillEffect::LinearProjectile { .. }
        | SkillEffect::ReturningShield { .. }
        | SkillEffect::RecastZone { .. }
        | SkillEffect::ImpactRocket { .. }
        | SkillEffect::WeaponToggle { .. } => StageRule::Active,
        SkillEffect::Technique { action, .. } => match action {
            // The pillar arms 0.65 s after the cast (`common/src/skills/advanced.rs:660`).
            Technique::TerrainLine => StageRule::ArmedGate,
            // A warning until armed, then a bolt on the same id
            // (`common/src/skills/mod.rs:1466-1468`).
            Technique::PiercingWave => StageRule::KindGate,
            // Replicated as a telegraph until the firing tick removes it
            // (`common/src/skills/advanced.rs:1157-1204`).
            Technique::ConeBrittle | Technique::BallPull | Technique::Parry => StageRule::Fuse,
            // The fissure is live at once although it is replicated as a warning kind
            // (`common/src/skills/advanced.rs:1205-1218`).
            Technique::GlacialFissure
            | Technique::CollisionCharge
            | Technique::ReturningColossus
            | Technique::Lunge
            | Technique::DoubleStrike
            | Technique::VitalChallenge
            | Technique::EchoStrike
            | Technique::GuardLeap
            | Technique::RevealPulse
            | Technique::ChainKick
            | Technique::SpikeVolley
            | Technique::Curse
            | Technique::Lash
            | Technique::ExecuteRetreat
            | Technique::ReturnOrb
            | Technique::GuidedFires
            | Technique::CharmBolt
            | Technique::SpiritDash
            | Technique::BallMove
            | Technique::BallField
            | Technique::BallGuard
            | Technique::OnHitBolt
            | Technique::DetonationMark
            | Technique::BlinkShot
            | Technique::Hook
            | Technique::Lantern
            | Technique::Sweep
            | Technique::SegmentCage
            | Technique::ConcussiveBolt
            | Technique::AllyLeap
            | Technique::InterceptShield
            | Technique::DaggerDeadlyBlow
            | Technique::DaggerBluff
            | Technique::DaggerBackstab
            | Technique::DaggerLethalBlow => StageRule::Active,
        },
    }
}

/// The phase a windup of this skill would follow; `instant` when the skill has no telegraph
/// of its own to hold a pose against.
pub(crate) fn derived_phase(id: SkillId) -> MotionPhase {
    if matches!(
        skill(id).effect,
        SkillEffect::Technique {
            action: Technique::Parry,
            ..
        }
    ) {
        return MotionPhase::Parry;
    }
    match own_kinds(id)
        .first()
        .map(|kind| stage_rule(id, *kind))
        .unwrap_or(StageRule::Active)
    {
        StageRule::KindGate => MotionPhase::WarnFire,
        StageRule::Fuse => MotionPhase::Fuse,
        StageRule::Active | StageRule::ArmedGate => MotionPhase::Instant,
    }
}

/// A technique effect lives `max(duration, range / 22 + 0.2)` seconds
/// (`common/src/skills/advanced.rs:576`).
fn technique_lifetime_secs(duration_secs: f32, cast_range: f32) -> f32 {
    duration_secs.max(cast_range / 22.0 + 0.2)
}
/// Furnace Breath fires at 80 % of its duration (`common/src/skills/advanced.rs:670`).
const CONE_ARM_SHARE: f32 = 0.8;
/// Mirror Guard and Orbital Collapse are removed 0.2 s after they arm, in the firing tick
/// (`common/src/skills/advanced.rs:739`, `:991-996`).
const FUSE_TAIL_SECS: f32 = 0.2;
/// A beam lives 0.15 s past its windup (`common/src/skills/mod.rs:1015`).
const BEAM_TAIL_SECS: f32 = 0.15;

/// `(telegraph, tail)`: how long the effect is a telegraph and what `remaining_secs` reads
/// when the telegraph ends. Only skills with a timed telegraph have one.
fn telegraph(id: SkillId) -> Option<(f32, f32)> {
    let def = skill(id);
    match def.effect {
        SkillEffect::Beam { .. } => Some((def.windup_secs, BEAM_TAIL_SECS)),
        SkillEffect::Technique {
            action,
            duration_secs,
            ..
        } => match action {
            Technique::ConeBrittle => {
                let telegraph = duration_secs * CONE_ARM_SHARE;
                Some((
                    telegraph,
                    technique_lifetime_secs(duration_secs, def.ability.cast_range) - telegraph,
                ))
            }
            Technique::Parry | Technique::BallPull => Some((duration_secs, FUSE_TAIL_SECS)),
            // Armed after the catalog windup (`common/src/skills/advanced.rs:577`).
            Technique::PiercingWave => Some((
                def.windup_secs,
                technique_lifetime_secs(duration_secs, def.ability.cast_range) - def.windup_secs,
            )),
            _ => None,
        },
        _ => None,
    }
}

pub(crate) fn telegraph_secs(id: SkillId) -> Option<f32> {
    telegraph(id).map(|(telegraph, _)| telegraph)
}

/// `remaining_secs` at the moment the telegraph ends.
pub(crate) fn tail_secs(id: SkillId) -> Option<f32> {
    telegraph(id).map(|(_, tail)| tail)
}

/// Lifetime at spawn of bodies that stand where they were cast and are never renewed;
/// `spawn - remaining_secs` is then the age of the instance.
pub(crate) fn spawn_lifetime_secs(id: SkillId) -> Option<f32> {
    let def = skill(id);
    match def.effect {
        SkillEffect::Technique {
            action: Technique::InterceptShield | Technique::GlacialFissure,
            duration_secs,
            ..
        } => Some(technique_lifetime_secs(
            duration_secs,
            def.ability.cast_range,
        )),
        _ => None,
    }
}

/// Whether a damage receipt of this row's own slot can exist. Shield amounts, empowered
/// basic attacks and the duel's bonus damage are not hits of the skill
/// (`common/src/skills/advanced.rs:742-801`, `:1084-1101`, `:1634-1744`).
pub(crate) fn can_damage(key: SkillKey) -> bool {
    let id = match key {
        SkillKey::Legacy(class, slot) => return class.ability(slot).projectile_damage.is_some(),
        SkillKey::Modular(id) => id,
    };
    match skill(id).effect {
        SkillEffect::LinearProjectile { .. }
        | SkillEffect::RecastZone { .. }
        | SkillEffect::Beam { .. }
        | SkillEffect::TrapLine { .. }
        | SkillEffect::ImpactRocket { .. } => true,
        SkillEffect::ReturningShield { .. } | SkillEffect::WeaponToggle { .. } => false,
        SkillEffect::Technique { action, damage, .. } => {
            damage > 0.0
                && match action {
                    Technique::DoubleStrike
                    | Technique::VitalChallenge
                    | Technique::GuardLeap
                    | Technique::AllyLeap
                    | Technique::Lantern
                    | Technique::Curse
                    | Technique::InterceptShield
                    | Technique::DaggerBluff => false,
                    Technique::TerrainLine
                    | Technique::ConeBrittle
                    | Technique::CollisionCharge
                    | Technique::ReturningColossus
                    | Technique::Lunge
                    | Technique::Parry
                    | Technique::EchoStrike
                    | Technique::RevealPulse
                    | Technique::ChainKick
                    | Technique::SpikeVolley
                    | Technique::Lash
                    | Technique::ExecuteRetreat
                    | Technique::ReturnOrb
                    | Technique::GuidedFires
                    | Technique::CharmBolt
                    | Technique::SpiritDash
                    | Technique::BallMove
                    | Technique::BallField
                    | Technique::BallGuard
                    | Technique::BallPull
                    | Technique::OnHitBolt
                    | Technique::DetonationMark
                    | Technique::BlinkShot
                    | Technique::PiercingWave
                    | Technique::Hook
                    | Technique::Sweep
                    | Technique::SegmentCage
                    | Technique::ConcussiveBolt
                    | Technique::GlacialFissure
                    | Technique::DaggerDeadlyBlow
                    | Technique::DaggerBackstab
                    | Technique::DaggerLethalBlow => true,
                }
        }
    }
}

/// Whether one cast keeps travelling through a target it damaged: beams, projectiles with
/// more than one hit, and the three bodies the server lets fly on
/// (`common/src/skills/mod.rs:1194`, `:1277`; `common/src/skills/advanced.rs:1388-1393`).
pub(crate) fn pierces(key: SkillKey) -> bool {
    let Some(id) = key.modular() else {
        return false;
    };
    match skill(id).effect {
        SkillEffect::Beam { .. } => true,
        SkillEffect::LinearProjectile { max_hits, .. } => max_hits > 1,
        SkillEffect::Technique { action, .. } => matches!(
            action,
            Technique::PiercingWave | Technique::ReturnOrb | Technique::ReturningColossus
        ),
        SkillEffect::ReturningShield { .. }
        | SkillEffect::RecastZone { .. }
        | SkillEffect::WeaponToggle { .. }
        | SkillEffect::TrapLine { .. }
        | SkillEffect::ImpactRocket { .. } => false,
    }
}

/// Whether the skill damages every hostile in a disc around one point at one moment
/// (`common/src/skills/mod.rs:1076-1078`, `:1417-1419`;
/// `common/src/skills/advanced.rs:346-349`, `:810-817`, `:1015-1019`, `:1158-1162`).
pub(crate) fn area_damage(key: SkillKey) -> bool {
    let Some(id) = key.modular() else {
        return false;
    };
    match skill(id).effect {
        SkillEffect::RecastZone { .. } | SkillEffect::ImpactRocket { .. } => true,
        SkillEffect::Technique { action, .. } => matches!(
            action,
            Technique::CollisionCharge
                | Technique::RevealPulse
                | Technique::Sweep
                | Technique::BallField
                | Technique::BallPull
        ),
        SkillEffect::LinearProjectile { .. }
        | SkillEffect::ReturningShield { .. }
        | SkillEffect::Beam { .. }
        | SkillEffect::WeaponToggle { .. }
        | SkillEffect::TrapLine { .. } => false,
    }
}

/// Whether the replicated slot can report `can_recast` for this skill
/// (`common/src/skills/mod.rs:886-901`; the `recast` calls of `common/src/skills/advanced.rs`).
pub(crate) fn has_recast(id: SkillId) -> bool {
    match skill(id).effect {
        SkillEffect::RecastZone { .. } => true,
        SkillEffect::Technique { action, .. } => matches!(
            action,
            Technique::ReturningColossus
                | Technique::EchoStrike
                | Technique::Hook
                | Technique::SpikeVolley
                | Technique::GuardLeap
                | Technique::RevealPulse
                | Technique::SpiritDash
        ),
        SkillEffect::LinearProjectile { .. }
        | SkillEffect::ReturningShield { .. }
        | SkillEffect::Beam { .. }
        | SkillEffect::WeaponToggle { .. }
        | SkillEffect::TrapLine { .. }
        | SkillEffect::ImpactRocket { .. } => false,
    }
}

/// Whether the zone bursts when it is recast or runs out
/// (`common/src/skills/mod.rs:895-897`, `:1124-1126`).
pub(crate) fn detonates(id: SkillId) -> bool {
    matches!(skill(id).effect, SkillEffect::RecastZone { .. })
}

/// Whether the skill strikes when its own telegraph fires, long after the accepted cast
/// (stage rule `fuse`).
pub(crate) fn strikes_on_release(id: SkillId) -> bool {
    own_kinds(id)
        .first()
        .is_some_and(|kind| stage_rule(id, *kind) == StageRule::Fuse)
}

/// Whether the cast (or its recast) can move the caster (`dash_to` and `blink_to` in
/// `common/src/skills/advanced.rs:599-946`).
pub(crate) fn movement_capable(id: SkillId, recast: bool) -> bool {
    let SkillEffect::Technique { action, .. } = skill(id).effect else {
        return false;
    };
    match action {
        Technique::CollisionCharge
        | Technique::Lunge
        | Technique::Lash
        | Technique::ExecuteRetreat
        | Technique::BlinkShot
        | Technique::GuardLeap
        | Technique::AllyLeap => !recast,
        Technique::SpiritDash => true,
        Technique::EchoStrike | Technique::Hook => recast,
        _ => false,
    }
}

/// Whether the effect of the skill lives on when its owner dies or leaves. A hero's death
/// removes what it left behind, with one exception: a raised pillar blocks for its whole
/// time, and its effect lives exactly as long (`common/src/skills/advanced.rs:1101-1110`).
pub(crate) fn outlives_owner(id: SkillId) -> bool {
    matches!(
        skill(id).effect,
        SkillEffect::Technique {
            action: Technique::TerrainLine,
            ..
        }
    )
}

/// Whether the cast strikes along a line that ends at its own new effect
/// (`common/src/skills/advanced.rs:639-662`).
pub(crate) fn own_effect_strike(id: SkillId) -> bool {
    matches!(
        skill(id).effect,
        SkillEffect::Technique {
            action: Technique::TerrainLine,
            ..
        }
    )
}

/// Half-angle of the server's cone test for the two sector skills
/// (`dot > 0.6` at `common/src/skills/advanced.rs:1175`, `dot > 0.2` at `:881`).
pub(crate) fn cone_half_angle(id: SkillId) -> Option<f32> {
    match skill(id).effect {
        SkillEffect::Technique {
            action: Technique::ConeBrittle,
            ..
        } => Some(super::geometry::FURNACE_CONE_COS.acos()),
        SkillEffect::Technique {
            action: Technique::ExecuteRetreat,
            ..
        } => Some(super::geometry::NIGHTFALL_SECTOR_COS.acos()),
        _ => None,
    }
}

/// Where the hit of an accepted cast is resolved from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StrikeOrigin {
    /// The caster's position at the accepted cast.
    Origin,
    /// The caster's position after the move of the same cast; nothing is drawn when the
    /// arrival was not observed.
    Arrival,
    /// The position of the skill's own replicated effect.
    EffectPosition,
}

/// Skills that move first and then pick from the new position are anchored at the arrival
/// (`common/src/skills/advanced.rs:674-686`, `:726-727`, `:776`, `:901-923`).
pub(crate) fn strike_origin(id: SkillId, recast: bool) -> StrikeOrigin {
    let SkillEffect::Technique { action, .. } = skill(id).effect else {
        return StrikeOrigin::Origin;
    };
    match action {
        Technique::CollisionCharge
        | Technique::Lunge
        | Technique::AllyLeap
        | Technique::SpiritDash
        | Technique::BlinkShot => StrikeOrigin::Arrival,
        Technique::GuardLeap if !recast => StrikeOrigin::Arrival,
        // The riposte is resolved from the barrier (`common/src/skills/advanced.rs:1186`).
        Technique::Parry => StrikeOrigin::EffectPosition,
        _ => StrikeOrigin::Origin,
    }
}

/// Whether a recast hits at once although the first cast was a travelling body
/// (`common/src/skills/advanced.rs:617-624`).
pub(crate) fn recast_instant_hit(id: SkillId) -> bool {
    matches!(
        skill(id).effect,
        SkillEffect::Technique {
            action: Technique::SpikeVolley,
            ..
        }
    )
}

/// The recast is accepted only while the caster is this close to the skill's own effect.
pub(crate) fn recast_gate(id: SkillId) -> Option<f32> {
    matches!(
        skill(id).effect,
        SkillEffect::Technique {
            action: Technique::ReturningColossus,
            ..
        }
    )
    .then_some(super::geometry::RECAST_GATE_MOUNTAIN_ECHO)
}

/// Whether the first cast sends out a body that reaches its target later; a link from the
/// caster to a receipt would then claim a hit that has not happened.
pub(crate) fn travelling_body(key: SkillKey) -> bool {
    match key {
        SkillKey::Legacy(..) => category(key) == Category::LegacyProjectile,
        SkillKey::Modular(id) => {
            matches!(skill(id).effect, SkillEffect::ReturningShield { .. })
                || own_kinds(id)
                    .iter()
                    .any(|kind| matches!(kind, EffectVisualKind::Bolt | EffectVisualKind::Rocket))
        }
    }
}

/// Legacy heals are self only; their rows may not use a shield shape.
pub(crate) fn self_heal(key: SkillKey) -> bool {
    match key {
        SkillKey::Legacy(class, slot) => class.ability(slot).self_heal.is_some(),
        SkillKey::Modular(_) => false,
    }
}

/// Skills whose instant area may be flashed on the first-cast edge (coordinator ruling S1).
pub(crate) const AREA_FLASH_SIGNED_OFF: &[SkillId] = &[
    SkillId::ThunderPulse,
    SkillId::ChainSweep,
    SkillId::AnvilCharge,
];

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(skills: impl IntoIterator<Item = SkillId>) -> Vec<&'static str> {
        skills.into_iter().map(SkillId::id).collect()
    }
    fn modular_where(predicate: impl Fn(SkillId) -> bool) -> Vec<&'static str> {
        ids(SkillId::ALL.into_iter().filter(|id| predicate(*id)))
    }
    fn keys() -> Vec<SkillKey> {
        SkillId::ALL
            .into_iter()
            .map(SkillKey::Modular)
            .chain(HeroClass::LEGACY.into_iter().flat_map(|class| {
                SkillSlot::ALL
                    .into_iter()
                    .map(move |slot| SkillKey::Legacy(class, slot))
            }))
            .collect()
    }
    fn keys_where(predicate: impl Fn(SkillKey) -> bool) -> Vec<&'static str> {
        keys()
            .into_iter()
            .filter(|key| predicate(*key))
            .map(SkillKey::id)
            .collect()
    }

    #[test]
    fn every_row_key_resolves_and_names_its_home_class() {
        assert_eq!(keys().len(), HeroClass::ALL.len() * 4);
        for key in keys() {
            assert_eq!(SkillKey::from_id(key.id()), Some(key));
        }
        assert_eq!(SkillKey::from_id("basic"), None);
        for class in HeroClass::ALL {
            let owned = keys().into_iter().filter(|key| key.home() == class).count();
            assert_eq!(owned, 4, "{}", class.id());
        }
        assert_eq!(
            SkillKey::Modular(SkillId::DaggerBluff).home(),
            HeroClass::Adventurer
        );
        assert_eq!(
            SkillKey::from_id("renew").map(SkillKey::home),
            Some(HeroClass::Cleric)
        );
    }

    #[test]
    fn categories_split_the_roster_26_22_13_7() {
        let of = |category: Category| keys_where(|key| super::category(key) == category);
        assert_eq!(
            of(Category::ReplicatedEffect),
            [
                "dawn_bind",
                "dawn_barrier",
                "dawn_field",
                "dawn_ray",
                "wild_zap",
                "wild_traps",
                "wild_rocket",
                "fault_line",
                "furnace_breath",
                "mountain_echo",
                "mirror_guard",
                "echo_strike",
                "thorn_volley",
                "wandering_ember",
                "heart_tether",
                "orbital_field",
                "orbital_collapse",
                "rift_needle",
                "rift_seal",
                "horizon_wave",
                "iron_hook",
                "guiding_lantern",
                "iron_boundary",
                "winter_shard",
                "northwall",
                "winter_divide",
            ]
        );
        assert_eq!(
            of(Category::InstantModular),
            [
                "wild_switch",
                "anvil_charge",
                "edge_lunge",
                "twin_tempo",
                "fourfold_duel",
                "anchor_step",
                "thunder_pulse",
                "thunder_kick",
                "patient_curse",
                "shadow_lash",
                "nightfall",
                "kindled_wisps",
                "flame_dance",
                "orbital_command",
                "orbital_guard",
                "rift_step",
                "chain_sweep",
                "sheltering_leap",
                "dagger_deadly_blow",
                "dagger_bluff",
                "dagger_backstab",
                "dagger_lethal_blow",
            ]
        );
        assert_eq!(
            of(Category::LegacyProjectile),
            [
                "shield_bash",
                "heroic_strike",
                "rampage",
                "arc_bolt",
                "frost_lance",
                "pyroblast",
                "quick_shot",
                "piercing_arrow",
                "longshot",
                "smite",
                "feral_swipe",
                "hunters_mark",
                "primal_maul",
            ]
        );
        assert_eq!(
            of(Category::LegacySelfCast),
            [
                "battle_rally",
                "mana_surge",
                "field_dressing",
                "renew",
                "divine_favor",
                "guardians_blessing",
                "barkskin",
            ]
        );
    }

    #[test]
    fn replicated_kinds_and_stage_rules_follow_the_server_table() {
        use EffectVisualKind as K;
        let kinds = |id: SkillId| own_kinds(id).to_vec();
        assert_eq!(kinds(SkillId::DawnRay), [K::BeamWarning, K::Beam]);
        assert_eq!(kinds(SkillId::HorizonWave), [K::Bolt]);
        assert_eq!(kinds(SkillId::DawnBarrier), [K::Barrier]);
        assert_eq!(kinds(SkillId::MirrorGuard), [K::Barrier]);
        assert_eq!(kinds(SkillId::FaultLine), [K::Trap]);
        assert_eq!(kinds(SkillId::WildTraps), [K::Trap]);
        assert_eq!(kinds(SkillId::WildRocket), [K::Rocket]);
        assert_eq!(kinds(SkillId::OrbitalField), [K::Field]);
        assert_eq!(kinds(SkillId::DawnField), [K::Field]);
        assert_eq!(kinds(SkillId::GuidingLantern), [K::Lantern]);
        assert_eq!(kinds(SkillId::IronBoundary), [K::Cage]);
        assert_eq!(kinds(SkillId::Northwall), [K::ShieldWall]);
        for id in [
            SkillId::FurnaceBreath,
            SkillId::OrbitalCollapse,
            SkillId::WinterDivide,
        ] {
            assert_eq!(kinds(id), [K::BeamWarning], "{}", id.id());
        }
        assert_eq!(
            modular_where(|id| own_kinds(id) == [K::Bolt]),
            [
                "dawn_bind",
                "wild_zap",
                "mountain_echo",
                "echo_strike",
                "thorn_volley",
                "wandering_ember",
                "heart_tether",
                "rift_needle",
                "rift_seal",
                "horizon_wave",
                "iron_hook",
                "winter_shard",
            ]
        );

        let rule = |id: SkillId| stage_rule(id, own_kinds(id)[0]);
        let with_rule =
            |wanted: StageRule| modular_where(|id| !own_kinds(id).is_empty() && rule(id) == wanted);
        assert_eq!(
            with_rule(StageRule::ArmedGate),
            ["wild_traps", "fault_line"]
        );
        assert_eq!(with_rule(StageRule::KindGate), ["dawn_ray", "horizon_wave"]);
        assert_eq!(
            with_rule(StageRule::Fuse),
            ["furnace_breath", "mirror_guard", "orbital_collapse"]
        );
        assert_eq!(rule(SkillId::WinterDivide), StageRule::Active);
        assert_eq!(
            stage_rule(SkillId::HorizonWave, K::BeamWarning),
            StageRule::KindGate
        );
        // Auxiliary objects are live for as long as they are replicated.
        for (id, kind) in [
            (SkillId::OrbitalCommand, K::Orb),
            (SkillId::IronHook, K::Soul),
            (SkillId::AnchorStep, K::Anchor),
            (SkillId::FourfoldDuel, K::Healing),
        ] {
            assert_eq!(stage_rule(id, kind), StageRule::Active);
            assert!(unstable_id(kind));
        }
        assert!(!unstable_id(K::BeamWarning));
    }

    #[test]
    fn auxiliary_kinds_match_the_objects_the_server_tags() {
        use EffectVisualKind as K;
        let declared: Vec<_> = SkillId::ALL
            .into_iter()
            .filter(|id| !aux_kinds(*id).is_empty())
            .map(|id| (id.id(), aux_kinds(id).to_vec()))
            .collect();
        assert_eq!(
            declared,
            [
                ("fourfold_duel", vec![K::Healing]),
                ("anchor_step", vec![K::Anchor]),
                ("orbital_command", vec![K::Orb]),
                ("orbital_guard", vec![K::Orb]),
                ("horizon_wave", vec![K::BeamWarning]),
                ("iron_hook", vec![K::Soul]),
            ]
        );
        for kind in [K::Orb, K::Soul, K::Anchor, K::Healing, K::BeamWarning] {
            let wire = serde_json::to_value(kind).unwrap();
            assert_eq!(wire, kind_id(kind));
        }
    }

    #[test]
    fn telegraph_and_tail_equal_the_catalog_derived_values() {
        let close = |a: Option<f32>, b: f32| a.is_some_and(|a| (a - b).abs() < 1e-5);
        for (id, telegraph, tail) in [
            (SkillId::FurnaceBreath, 0.8, 0.2),
            (SkillId::MirrorGuard, 0.75, 0.2),
            (SkillId::OrbitalCollapse, 0.7, 0.2),
            (SkillId::DawnRay, 0.8, 0.15),
            // `max(9, 256 / 22 + 0.2)` minus the 0.7 s windup.
            (SkillId::HorizonWave, 0.7, 256.0 / 22.0 + 0.2 - 0.7),
        ] {
            assert!(close(telegraph_secs(id), telegraph), "{}", id.id());
            assert!(close(tail_secs(id), tail), "{}", id.id());
        }
        assert_eq!(
            modular_where(|id| telegraph_secs(id).is_some()),
            [
                "dawn_ray",
                "furnace_breath",
                "mirror_guard",
                "orbital_collapse",
                "horizon_wave",
            ]
        );
        assert_eq!(spawn_lifetime_secs(SkillId::Northwall), Some(3.0));
        assert_eq!(spawn_lifetime_secs(SkillId::WinterDivide), Some(4.0));
        assert_eq!(
            modular_where(|id| spawn_lifetime_secs(id).is_some()),
            ["northwall", "winter_divide"]
        );
    }

    #[test]
    fn derived_phases_cover_the_five_telegraphed_skills() {
        let with_phase = |wanted: MotionPhase| modular_where(|id| derived_phase(id) == wanted);
        assert_eq!(
            with_phase(MotionPhase::WarnFire),
            ["dawn_ray", "horizon_wave"]
        );
        assert_eq!(
            with_phase(MotionPhase::Fuse),
            ["furnace_breath", "orbital_collapse"]
        );
        assert_eq!(with_phase(MotionPhase::Parry), ["mirror_guard"]);
        assert_eq!(with_phase(MotionPhase::Instant).len(), 43);
    }

    #[test]
    fn damage_facts_are_pinned_to_the_catalog() {
        assert_eq!(
            keys_where(|key| !can_damage(key)),
            [
                "dawn_barrier",
                "wild_switch",
                "twin_tempo",
                "fourfold_duel",
                "anchor_step",
                "patient_curse",
                "guiding_lantern",
                "sheltering_leap",
                "northwall",
                "dagger_bluff",
                "battle_rally",
                "mana_surge",
                "field_dressing",
                "renew",
                "divine_favor",
                "guardians_blessing",
                "barkskin",
            ]
        );
        // A damaging modular row always has a positive catalog damage to deal.
        for id in SkillId::ALL {
            if can_damage(SkillKey::Modular(id)) {
                assert!(
                    skill(id).effect.damage().is_some_and(|damage| damage > 0.0),
                    "{}",
                    id.id()
                );
            }
        }
        assert_eq!(
            keys_where(pierces),
            [
                "dawn_bind",
                "dawn_ray",
                "mountain_echo",
                "wandering_ember",
                "horizon_wave",
            ]
        );
        assert_eq!(
            keys_where(area_damage),
            [
                "dawn_field",
                "wild_rocket",
                "anvil_charge",
                "thunder_pulse",
                "orbital_field",
                "orbital_collapse",
                "chain_sweep",
            ]
        );
        assert_eq!(
            keys_where(self_heal),
            [
                "battle_rally",
                "field_dressing",
                "renew",
                "guardians_blessing",
                "barkskin",
            ]
        );
        for id in AREA_FLASH_SIGNED_OFF {
            assert!(area_damage(SkillKey::Modular(*id)), "{}", id.id());
        }
        assert_eq!(
            ids(AREA_FLASH_SIGNED_OFF.iter().copied()),
            ["thunder_pulse", "chain_sweep", "anvil_charge"]
        );
    }

    #[test]
    fn recast_and_movement_facts_are_pinned_to_the_server_rules() {
        assert_eq!(
            modular_where(has_recast),
            [
                "dawn_field",
                "mountain_echo",
                "echo_strike",
                "anchor_step",
                "thunder_pulse",
                "thorn_volley",
                "flame_dance",
                "iron_hook",
            ]
        );
        assert_eq!(
            modular_where(|id| movement_capable(id, false)),
            [
                "anvil_charge",
                "edge_lunge",
                "anchor_step",
                "shadow_lash",
                "nightfall",
                "flame_dance",
                "rift_step",
                "sheltering_leap",
            ]
        );
        assert_eq!(
            modular_where(|id| movement_capable(id, true)),
            ["echo_strike", "flame_dance", "iron_hook"]
        );
        // A recast move needs a recast.
        for id in SkillId::ALL {
            assert!(!movement_capable(id, true) || has_recast(id), "{}", id.id());
        }
        assert_eq!(modular_where(own_effect_strike), ["fault_line"]);
        assert_eq!(modular_where(detonates), ["dawn_field"]);
        assert_eq!(modular_where(recast_instant_hit), ["thorn_volley"]);
        assert_eq!(recast_gate(SkillId::MountainEcho), Some(4.0));
        assert_eq!(
            modular_where(|id| recast_gate(id).is_some()),
            ["mountain_echo"]
        );
        assert_eq!(
            modular_where(|id| strike_origin(id, false) == StrikeOrigin::Arrival),
            [
                "anvil_charge",
                "edge_lunge",
                "anchor_step",
                "flame_dance",
                "rift_step",
                "sheltering_leap",
            ]
        );
        assert_eq!(
            modular_where(|id| strike_origin(id, true) == StrikeOrigin::Arrival),
            [
                "anvil_charge",
                "edge_lunge",
                "flame_dance",
                "rift_step",
                "sheltering_leap",
            ]
        );
        assert_eq!(
            modular_where(|id| strike_origin(id, false) == StrikeOrigin::EffectPosition),
            ["mirror_guard"]
        );
        for id in [
            SkillId::Nightfall,
            SkillId::ShadowLash,
            SkillId::ChainSweep,
            SkillId::ThunderPulse,
            SkillId::ThunderKick,
            SkillId::DawnField,
        ] {
            assert_eq!(
                strike_origin(id, false),
                StrikeOrigin::Origin,
                "{}",
                id.id()
            );
        }
    }

    #[test]
    fn travelling_kinds_and_released_strikes_follow_the_server_table() {
        use EffectVisualKind as K;
        const KINDS: [K; 14] = [
            K::Orb,
            K::Soul,
            K::Anchor,
            K::Healing,
            K::ShieldWall,
            K::Cage,
            K::Lantern,
            K::Bolt,
            K::Barrier,
            K::Field,
            K::BeamWarning,
            K::Beam,
            K::Trap,
            K::Rocket,
        ];
        let heading: Vec<_> = KINDS
            .into_iter()
            .filter(|kind| heading_only(*kind))
            .collect();
        assert_eq!(heading, [K::Bolt, K::Barrier, K::Rocket]);
        // The parry stance is the one barrier that stands still.
        let moving: Vec<_> = KINDS
            .into_iter()
            .filter(|kind| travels(SkillId::MirrorGuard, *kind))
            .collect();
        assert_eq!(moving, [K::Orb, K::Soul, K::Bolt, K::Rocket]);
        assert!(travels(SkillId::DawnBarrier, K::Barrier));
        // Every kind the first cast of a travelling body replicates is a travelling kind,
        // except the warning that comes before the wave.
        for id in SkillId::ALL {
            for kind in own_kinds(id) {
                assert_eq!(
                    travels(id, *kind),
                    travelling_body(SkillKey::Modular(id)),
                    "{} {kind:?}",
                    id.id()
                );
            }
        }
        assert!(!travels(SkillId::HorizonWave, K::BeamWarning));
        assert_eq!(
            modular_where(strikes_on_release),
            ["furnace_breath", "mirror_guard", "orbital_collapse"]
        );
    }

    #[test]
    fn travelling_bodies_and_cone_angles() {
        assert_eq!(
            keys_where(travelling_body),
            [
                "dawn_bind",
                "dawn_barrier",
                "wild_zap",
                "wild_rocket",
                "mountain_echo",
                "echo_strike",
                "thorn_volley",
                "wandering_ember",
                "heart_tether",
                "rift_needle",
                "rift_seal",
                "horizon_wave",
                "iron_hook",
                "winter_shard",
                "shield_bash",
                "heroic_strike",
                "rampage",
                "arc_bolt",
                "frost_lance",
                "pyroblast",
                "quick_shot",
                "piercing_arrow",
                "longshot",
                "smite",
                "feral_swipe",
                "hunters_mark",
                "primal_maul",
            ]
        );
        assert_eq!(
            modular_where(|id| cone_half_angle(id).is_some()),
            ["furnace_breath", "nightfall"]
        );
        assert_eq!(
            cone_half_angle(SkillId::FurnaceBreath),
            Some(0.6_f32.acos())
        );
        assert_eq!(cone_half_angle(SkillId::Nightfall), Some(0.2_f32.acos()));
    }
}
