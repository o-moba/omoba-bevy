//! Versioned skills and validated recipes. Presets are assignments, not executors.
//! Only this resolver constructs a `ResolvedLoadout`; avatar/cosmetic data never
//! participates. The current UI exposes presets; composition is an authoring API.

mod equipped;
pub use equipped::EquippedSkills;

use crate::map::Team;
use crate::{AbilityDefinition, HeroClass, MAX_ABILITY_RANK, SkillSlot, TargetingMode};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

pub const CATALOG_REVISION: &str = "standard-kits-5";
pub const RECIPE_SCHEMA_VERSION: u16 = 1;
pub const MAX_ACTIVE_EFFECTS: usize = 128;
pub const MAX_EFFECTS_PER_OWNER: usize = 16;
pub const MAX_AIM_COORDINATE: f32 = 4096.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreId {
    Dawnweaver,
    Wildspark,
    Cinderforge,
    Edgeweaver,
    Stormfist,
    Veilstalker,
    Emberveil,
    Orbitwright,
    Riftshot,
    Chainkeeper,
    Frostguard,
    Adventurer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillId {
    DawnBind,
    DawnBarrier,
    DawnField,
    DawnRay,
    WildSwitch,
    WildZap,
    WildTraps,
    WildRocket,
    FaultLine,
    FurnaceBreath,
    AnvilCharge,
    MountainEcho,
    EdgeLunge,
    MirrorGuard,
    TwinTempo,
    FourfoldDuel,
    EchoStrike,
    AnchorStep,
    ThunderPulse,
    ThunderKick,
    ThornVolley,
    PatientCurse,
    ShadowLash,
    Nightfall,
    WanderingEmber,
    KindledWisps,
    HeartTether,
    FlameDance,
    OrbitalCommand,
    OrbitalField,
    OrbitalGuard,
    OrbitalCollapse,
    RiftNeedle,
    RiftSeal,
    RiftStep,
    HorizonWave,
    IronHook,
    GuidingLantern,
    ChainSweep,
    IronBoundary,
    WinterShard,
    ShelteringLeap,
    Northwall,
    WinterDivide,
    DaggerDeadlyBlow,
    DaggerBluff,
    DaggerBackstab,
    DaggerLethalBlow,
}
impl SkillId {
    pub const ALL: [Self; 48] = [
        Self::DawnBind,
        Self::DawnBarrier,
        Self::DawnField,
        Self::DawnRay,
        Self::WildSwitch,
        Self::WildZap,
        Self::WildTraps,
        Self::WildRocket,
        Self::FaultLine,
        Self::FurnaceBreath,
        Self::AnvilCharge,
        Self::MountainEcho,
        Self::EdgeLunge,
        Self::MirrorGuard,
        Self::TwinTempo,
        Self::FourfoldDuel,
        Self::EchoStrike,
        Self::AnchorStep,
        Self::ThunderPulse,
        Self::ThunderKick,
        Self::ThornVolley,
        Self::PatientCurse,
        Self::ShadowLash,
        Self::Nightfall,
        Self::WanderingEmber,
        Self::KindledWisps,
        Self::HeartTether,
        Self::FlameDance,
        Self::OrbitalCommand,
        Self::OrbitalField,
        Self::OrbitalGuard,
        Self::OrbitalCollapse,
        Self::RiftNeedle,
        Self::RiftSeal,
        Self::RiftStep,
        Self::HorizonWave,
        Self::IronHook,
        Self::GuidingLantern,
        Self::ChainSweep,
        Self::IronBoundary,
        Self::WinterShard,
        Self::ShelteringLeap,
        Self::Northwall,
        Self::WinterDivide,
        Self::DaggerDeadlyBlow,
        Self::DaggerBluff,
        Self::DaggerBackstab,
        Self::DaggerLethalBlow,
    ];
    pub const fn id(self) -> &'static str {
        match self {
            Self::DawnBind => "dawn_bind",
            Self::DawnBarrier => "dawn_barrier",
            Self::DawnField => "dawn_field",
            Self::DawnRay => "dawn_ray",
            Self::WildSwitch => "wild_switch",
            Self::WildZap => "wild_zap",
            Self::WildTraps => "wild_traps",
            Self::WildRocket => "wild_rocket",
            Self::FaultLine => "fault_line",
            Self::FurnaceBreath => "furnace_breath",
            Self::AnvilCharge => "anvil_charge",
            Self::MountainEcho => "mountain_echo",
            Self::EdgeLunge => "edge_lunge",
            Self::MirrorGuard => "mirror_guard",
            Self::TwinTempo => "twin_tempo",
            Self::FourfoldDuel => "fourfold_duel",
            Self::EchoStrike => "echo_strike",
            Self::AnchorStep => "anchor_step",
            Self::ThunderPulse => "thunder_pulse",
            Self::ThunderKick => "thunder_kick",
            Self::ThornVolley => "thorn_volley",
            Self::PatientCurse => "patient_curse",
            Self::ShadowLash => "shadow_lash",
            Self::Nightfall => "nightfall",
            Self::WanderingEmber => "wandering_ember",
            Self::KindledWisps => "kindled_wisps",
            Self::HeartTether => "heart_tether",
            Self::FlameDance => "flame_dance",
            Self::OrbitalCommand => "orbital_command",
            Self::OrbitalField => "orbital_field",
            Self::OrbitalGuard => "orbital_guard",
            Self::OrbitalCollapse => "orbital_collapse",
            Self::RiftNeedle => "rift_needle",
            Self::RiftSeal => "rift_seal",
            Self::RiftStep => "rift_step",
            Self::HorizonWave => "horizon_wave",
            Self::IronHook => "iron_hook",
            Self::GuidingLantern => "guiding_lantern",
            Self::ChainSweep => "chain_sweep",
            Self::IronBoundary => "iron_boundary",
            Self::WinterShard => "winter_shard",
            Self::ShelteringLeap => "sheltering_leap",
            Self::Northwall => "northwall",
            Self::WinterDivide => "winter_divide",
            Self::DaggerDeadlyBlow => "dagger_deadly_blow",
            Self::DaggerBluff => "dagger_bluff",
            Self::DaggerBackstab => "dagger_backstab",
            Self::DaggerLethalBlow => "dagger_lethal_blow",
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.id() == id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PassiveId {
    Radiance,
    Momentum,
    Tempered,
    Vitals,
    Flow,
    Shroud,
    Essence,
    Clockwork,
    Resonance,
    Souls,
    Concussion,
    DaggerMastery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttackProfileId {
    Melee,
    LightBolt,
    Repeater,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DamageType {
    Physical,
    Magic,
    True,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeaponMode {
    #[default]
    Repeater,
    Rockets,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildRecipe {
    pub schema_version: u16,
    pub catalog_revision: String,
    pub core: CoreId,
    pub passive: PassiveId,
    pub skills: [SkillId; 4],
}

/// Fields are private: unchecked recipes cannot become executable loadouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedLoadout {
    core: CoreId,
    passive: PassiveId,
    skills: [SkillId; 4],
    attack_profile: AttackProfileId,
}
impl ResolvedLoadout {
    pub const fn core(&self) -> CoreId {
        self.core
    }
    pub const fn passive(&self) -> PassiveId {
        self.passive
    }
    pub const fn skills(&self) -> [SkillId; 4] {
        self.skills
    }
    pub const fn attack_profile(&self) -> AttackProfileId {
        self.attack_profile
    }
    pub fn skill(&self, slot: SkillSlot) -> &'static SkillDefinition {
        skill(self.skills[slot.index()])
    }
    /// The authored progression role stays with the skill when its binding moves.
    pub fn unlock_level(&self, slot: SkillSlot) -> u32 {
        crate::SLOT_UNLOCK_LEVELS[self.skill(slot).slot.index()]
    }
    pub fn unlocked(&self, level: u32) -> [bool; 4] {
        SkillSlot::ALL.map(|slot| level.max(1) >= self.unlock_level(slot))
    }
    pub fn recipe(&self) -> BuildRecipe {
        BuildRecipe {
            schema_version: RECIPE_SCHEMA_VERSION,
            catalog_revision: CATALOG_REVISION.into(),
            core: self.core,
            passive: self.passive,
            skills: self.skills,
        }
    }
}

impl CoreId {
    pub const fn class(self) -> HeroClass {
        match self {
            Self::Dawnweaver => HeroClass::Dawnweaver,
            Self::Wildspark => HeroClass::Wildspark,
            Self::Cinderforge => HeroClass::Cinderforge,
            Self::Edgeweaver => HeroClass::Edgeweaver,
            Self::Stormfist => HeroClass::Stormfist,
            Self::Veilstalker => HeroClass::Veilstalker,
            Self::Emberveil => HeroClass::Emberveil,
            Self::Orbitwright => HeroClass::Orbitwright,
            Self::Riftshot => HeroClass::Riftshot,
            Self::Chainkeeper => HeroClass::Chainkeeper,
            Self::Frostguard => HeroClass::Frostguard,
            Self::Adventurer => HeroClass::Adventurer,
        }
    }
    pub const fn attack_profile(self) -> AttackProfileId {
        match self {
            Self::Dawnweaver => AttackProfileId::LightBolt,
            Self::Wildspark => AttackProfileId::Repeater,
            Self::Cinderforge => AttackProfileId::Melee,
            Self::Edgeweaver => AttackProfileId::Melee,
            Self::Stormfist => AttackProfileId::Melee,
            Self::Veilstalker => AttackProfileId::Melee,
            Self::Emberveil => AttackProfileId::LightBolt,
            Self::Orbitwright => AttackProfileId::LightBolt,
            Self::Riftshot => AttackProfileId::LightBolt,
            Self::Chainkeeper => AttackProfileId::LightBolt,
            Self::Frostguard => AttackProfileId::Melee,
            Self::Adventurer => AttackProfileId::Melee,
        }
    }
    pub fn preset(self) -> BuildRecipe {
        let (passive, skills) = match self {
            Self::Cinderforge => (
                PassiveId::Tempered,
                [
                    SkillId::FaultLine,
                    SkillId::FurnaceBreath,
                    SkillId::AnvilCharge,
                    SkillId::MountainEcho,
                ],
            ),
            Self::Edgeweaver => (
                PassiveId::Vitals,
                [
                    SkillId::EdgeLunge,
                    SkillId::MirrorGuard,
                    SkillId::TwinTempo,
                    SkillId::FourfoldDuel,
                ],
            ),
            Self::Stormfist => (
                PassiveId::Flow,
                [
                    SkillId::EchoStrike,
                    SkillId::AnchorStep,
                    SkillId::ThunderPulse,
                    SkillId::ThunderKick,
                ],
            ),
            Self::Veilstalker => (
                PassiveId::Shroud,
                [
                    SkillId::ThornVolley,
                    SkillId::PatientCurse,
                    SkillId::ShadowLash,
                    SkillId::Nightfall,
                ],
            ),
            Self::Emberveil => (
                PassiveId::Essence,
                [
                    SkillId::WanderingEmber,
                    SkillId::KindledWisps,
                    SkillId::HeartTether,
                    SkillId::FlameDance,
                ],
            ),
            Self::Orbitwright => (
                PassiveId::Clockwork,
                [
                    SkillId::OrbitalCommand,
                    SkillId::OrbitalField,
                    SkillId::OrbitalGuard,
                    SkillId::OrbitalCollapse,
                ],
            ),
            Self::Riftshot => (
                PassiveId::Resonance,
                [
                    SkillId::RiftNeedle,
                    SkillId::RiftSeal,
                    SkillId::RiftStep,
                    SkillId::HorizonWave,
                ],
            ),
            Self::Chainkeeper => (
                PassiveId::Souls,
                [
                    SkillId::IronHook,
                    SkillId::GuidingLantern,
                    SkillId::ChainSweep,
                    SkillId::IronBoundary,
                ],
            ),
            Self::Frostguard => (
                PassiveId::Concussion,
                [
                    SkillId::WinterShard,
                    SkillId::ShelteringLeap,
                    SkillId::Northwall,
                    SkillId::WinterDivide,
                ],
            ),

            Self::Adventurer => (
                PassiveId::DaggerMastery,
                [
                    SkillId::DaggerDeadlyBlow,
                    SkillId::DaggerBluff,
                    SkillId::DaggerBackstab,
                    SkillId::DaggerLethalBlow,
                ],
            ),
            Self::Dawnweaver => (
                PassiveId::Radiance,
                [
                    SkillId::DawnBind,
                    SkillId::DawnBarrier,
                    SkillId::DawnField,
                    SkillId::DawnRay,
                ],
            ),
            Self::Wildspark => (
                PassiveId::Momentum,
                [
                    SkillId::WildSwitch,
                    SkillId::WildZap,
                    SkillId::WildTraps,
                    SkillId::WildRocket,
                ],
            ),
        };
        BuildRecipe {
            schema_version: RECIPE_SCHEMA_VERSION,
            catalog_revision: CATALOG_REVISION.into(),
            core: self,
            passive,
            skills,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadoutError {
    SchemaVersion,
    CatalogRevision,
    DuplicateSkill { skill: SkillId },
    CoreMismatch { class: HeroClass, core: CoreId },
    RequiresRepeater { skill: SkillId },
    RequiresOrbController { skill: SkillId },
}
impl std::fmt::Display for LoadoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for LoadoutError {}

pub fn resolve(recipe: &BuildRecipe) -> Result<ResolvedLoadout, LoadoutError> {
    if recipe.schema_version != RECIPE_SCHEMA_VERSION {
        return Err(LoadoutError::SchemaVersion);
    }
    if recipe.catalog_revision != CATALOG_REVISION {
        return Err(LoadoutError::CatalogRevision);
    }
    let attack_profile = recipe.core.attack_profile();
    for (index, id) in recipe.skills.into_iter().enumerate() {
        let def = skill(id);
        // Stateful/recast skills have one identity per actor. Distinct skills
        // may use any binding; duplicate identities require a separate design.
        if recipe.skills[..index].contains(&id) {
            return Err(LoadoutError::DuplicateSkill { skill: id });
        }
        if matches!(
            def.effect,
            SkillEffect::Technique {
                action: Technique::BallField | Technique::BallPull,
                ..
            }
        ) && !recipe.skills.iter().any(|id| {
            matches!(
                skill(*id).effect,
                SkillEffect::Technique {
                    action: Technique::BallMove | Technique::BallGuard,
                    ..
                }
            )
        }) {
            return Err(LoadoutError::RequiresOrbController { skill: id });
        }
        if matches!(def.effect, SkillEffect::WeaponToggle { .. })
            && attack_profile != AttackProfileId::Repeater
        {
            return Err(LoadoutError::RequiresRepeater { skill: id });
        }
    }
    Ok(ResolvedLoadout {
        core: recipe.core,
        passive: recipe.passive,
        skills: recipe.skills,
        attack_profile,
    })
}

pub fn preset_for_class(class: HeroClass) -> Option<ResolvedLoadout> {
    let core = match class {
        HeroClass::Dawnweaver => CoreId::Dawnweaver,
        HeroClass::Wildspark => CoreId::Wildspark,
        HeroClass::Cinderforge => CoreId::Cinderforge,
        HeroClass::Edgeweaver => CoreId::Edgeweaver,
        HeroClass::Stormfist => CoreId::Stormfist,
        HeroClass::Veilstalker => CoreId::Veilstalker,
        HeroClass::Emberveil => CoreId::Emberveil,
        HeroClass::Orbitwright => CoreId::Orbitwright,
        HeroClass::Riftshot => CoreId::Riftshot,
        HeroClass::Chainkeeper => CoreId::Chainkeeper,
        HeroClass::Frostguard => CoreId::Frostguard,
        HeroClass::Adventurer => CoreId::Adventurer,

        _ => return None,
    };
    Some(resolve(&core.preset()).expect("embedded preset is valid"))
}

pub fn valid_aim(aim: [f32; 2]) -> bool {
    aim.into_iter()
        .all(|v| v.is_finite() && v.abs() <= MAX_AIM_COORDINATE)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Technique {
    TerrainLine,
    ConeBrittle,
    CollisionCharge,
    ReturningColossus,
    Lunge,
    Parry,
    DoubleStrike,
    VitalChallenge,
    EchoStrike,
    GuardLeap,
    RevealPulse,
    ChainKick,
    SpikeVolley,
    Curse,
    Lash,
    ExecuteRetreat,
    ReturnOrb,
    GuidedFires,
    CharmBolt,
    SpiritDash,
    BallMove,
    BallField,
    BallGuard,
    BallPull,
    OnHitBolt,
    DetonationMark,
    BlinkShot,
    PiercingWave,
    Hook,
    Lantern,
    Sweep,
    SegmentCage,
    ConcussiveBolt,
    AllyLeap,
    InterceptShield,
    GlacialFissure,
    DaggerDeadlyBlow,
    DaggerBluff,
    DaggerBackstab,
    DaggerLethalBlow,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SkillEffect {
    Technique {
        action: Technique,
        damage: f32,
        radius: f32,
        duration_secs: f32,
        speed: f32,
    },
    LinearProjectile {
        speed: f32,
        radius: f32,
        damage: f32,
        max_hits: u8,
        root_secs: f32,
        slow_multiplier: f32,
        slow_secs: f32,
        reveal_secs: f32,
    },
    ReturningShield {
        speed: f32,
        radius: f32,
        amount: f32,
        duration_secs: f32,
    },
    RecastZone {
        radius: f32,
        damage: f32,
        duration_secs: f32,
        slow_multiplier: f32,
    },
    Beam {
        width: f32,
        damage: f32,
    },
    WeaponToggle {
        rocket_range: f32,
        rocket_damage_multiplier: f32,
        rocket_cooldown_multiplier: f32,
        rocket_mana_cost: f32,
        splash_radius: f32,
        minigun_stack_attack_speed: f32,
        stack_duration_secs: f32,
        max_stacks: u8,
    },
    TrapLine {
        count: u8,
        spacing: f32,
        radius: f32,
        damage: f32,
        root_secs: f32,
        arm_secs: f32,
        duration_secs: f32,
    },
    ImpactRocket {
        speed: f32,
        radius: f32,
        blast_radius: f32,
        damage: f32,
        min_damage_multiplier: f32,
        max_distance: f32,
        missing_health_ratio: f32,
    },
}

impl SkillEffect {
    pub const fn damage(self) -> Option<f32> {
        match self {
            Self::Technique { damage, .. }
            | Self::LinearProjectile { damage, .. }
            | Self::RecastZone { damage, .. }
            | Self::Beam { damage, .. }
            | Self::TrapLine { damage, .. }
            | Self::ImpactRocket { damage, .. } => Some(damage),
            _ => None,
        }
    }
    fn validate(self) -> bool {
        let positive = |v: f32| v.is_finite() && v > 0.0 && v <= 4096.0;
        let nonnegative = |v: f32| v.is_finite() && (0.0..=4096.0).contains(&v);
        let multiplier = |v: f32| v.is_finite() && (0.0..=1.0).contains(&v);
        match self {
            Self::Technique {
                damage,
                radius,
                duration_secs,
                speed,
                ..
            } => {
                nonnegative(damage)
                    && positive(radius)
                    && positive(duration_secs)
                    && nonnegative(speed)
            }
            Self::LinearProjectile {
                speed,
                radius,
                damage,
                max_hits,
                root_secs,
                slow_multiplier,
                slow_secs,
                reveal_secs,
            } => {
                [speed, radius, damage].into_iter().all(positive)
                    && (1..=16).contains(&max_hits)
                    && [root_secs, slow_secs, reveal_secs]
                        .into_iter()
                        .all(nonnegative)
                    && multiplier(slow_multiplier)
            }
            Self::ReturningShield {
                speed,
                radius,
                amount,
                duration_secs,
            } => [speed, radius, amount, duration_secs]
                .into_iter()
                .all(positive),
            Self::RecastZone {
                radius,
                damage,
                duration_secs,
                slow_multiplier,
            } => {
                [radius, damage, duration_secs].into_iter().all(positive)
                    && multiplier(slow_multiplier)
            }
            Self::Beam { width, damage } => [width, damage].into_iter().all(positive),
            Self::WeaponToggle {
                rocket_range,
                rocket_damage_multiplier,
                rocket_cooldown_multiplier,
                rocket_mana_cost,
                splash_radius,
                minigun_stack_attack_speed,
                stack_duration_secs,
                max_stacks,
            } => {
                [
                    rocket_range,
                    rocket_damage_multiplier,
                    rocket_cooldown_multiplier,
                    splash_radius,
                    stack_duration_secs,
                ]
                .into_iter()
                .all(positive)
                    && [rocket_mana_cost, minigun_stack_attack_speed]
                        .into_iter()
                        .all(nonnegative)
                    && (1..=16).contains(&max_stacks)
            }
            Self::TrapLine {
                count,
                spacing,
                radius,
                damage,
                root_secs,
                arm_secs,
                duration_secs,
            } => {
                (1..=5).contains(&count)
                    && [spacing, radius, damage, root_secs, duration_secs]
                        .into_iter()
                        .all(positive)
                    && nonnegative(arm_secs)
                    && arm_secs < duration_secs
            }
            Self::ImpactRocket {
                speed,
                radius,
                blast_radius,
                damage,
                min_damage_multiplier,
                max_distance,
                missing_health_ratio,
            } => {
                [speed, radius, blast_radius, damage, max_distance]
                    .into_iter()
                    .all(positive)
                    && multiplier(min_damage_multiplier)
                    && multiplier(missing_health_ratio)
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SkillDefinition {
    pub id: SkillId,
    /// Authored default binding and progression role, never the equipped index.
    pub slot: SkillSlot,
    pub ability: AbilityDefinition,
    pub effect: SkillEffect,
    pub damage_type: DamageType,
    pub windup_secs: f32,
    /// Follow-up resource cost belongs to the skill, independently of hero core.
    pub recast_mana_cost: f32,
}
impl SkillDefinition {
    pub fn mana_cost(&self, rank: u8, recast: bool) -> f32 {
        if recast {
            self.recast_mana_cost
        } else {
            crate::scaled_mana_cost(&self.ability, rank.clamp(1, self.ability.max_rank))
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum PassiveEffect {
    Advanced(PassiveId),
    Radiance {
        mark_duration_secs: f32,
        bonus_damage: f32,
    },
    Momentum {
        duration_secs: f32,
        move_multiplier: f32,
        attack_speed_multiplier: f32,
        assist_window_secs: f32,
    },
}
pub fn passive(id: PassiveId) -> PassiveEffect {
    match id {
        PassiveId::Tempered
        | PassiveId::Vitals
        | PassiveId::Flow
        | PassiveId::Shroud
        | PassiveId::Essence
        | PassiveId::Clockwork
        | PassiveId::Resonance
        | PassiveId::Souls
        | PassiveId::Concussion
        | PassiveId::DaggerMastery => PassiveEffect::Advanced(id),
        PassiveId::Radiance => PassiveEffect::Radiance {
            mark_duration_secs: 6.0,
            bonus_damage: 18.0,
        },
        PassiveId::Momentum => PassiveEffect::Momentum {
            duration_secs: 6.0,
            move_multiplier: 1.4,
            attack_speed_multiplier: 1.5,
            assist_window_secs: 3.0,
        },
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCatalog {
    schema_version: u16,
    revision: String,
    skills: Vec<RawSkill>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSkill {
    id: SkillId,
    slot: SkillSlot,
    name: String,
    description: String,
    targeting: TargetingMode,
    mana_cost: f32,
    #[serde(default)]
    recast_mana_cost: f32,
    cooldown_secs: f32,
    cast_range: f32,
    windup_secs: f32,
    damage_type: DamageType,
    effect: SkillEffect,
}
const SKILLS_JSON: &str = include_str!("../assets/catalog/skills.json");
static SKILLS: LazyLock<Vec<SkillDefinition>> = LazyLock::new(|| {
    parse_skills(SKILLS_JSON)
        .unwrap_or_else(|e| panic!("invalid shared/assets/catalog/skills.json: {e}"))
});

fn parse_skills(json: &str) -> Result<Vec<SkillDefinition>, String> {
    let raw: RawCatalog = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if raw.schema_version != RECIPE_SCHEMA_VERSION
        || raw.revision != CATALOG_REVISION
        || raw.skills.len() != SkillId::ALL.len()
    {
        return Err("catalog schema, revision or skill count mismatch".into());
    }
    let mut result = Vec::new();
    for (expected, r) in SkillId::ALL.into_iter().zip(raw.skills) {
        if r.id != expected
            || r.name.trim().is_empty()
            || r.description.trim().is_empty()
            || ![
                r.mana_cost,
                r.recast_mana_cost,
                r.cooldown_secs,
                r.cast_range,
                r.windup_secs,
            ]
            .into_iter()
            .all(|n| n.is_finite() && (0.0..=4096.0).contains(&n))
            || r.cooldown_secs == 0.0
            || !r.effect.validate()
        {
            return Err(format!("invalid definition for {}", expected.id()));
        }
        let targeting_valid = match r.effect {
            SkillEffect::Technique { .. } => {
                matches!(r.targeting, TargetingMode::Direction | TargetingMode::Point)
                    && r.cast_range > 0.0
                    || r.targeting == TargetingMode::SelfTarget
            }
            SkillEffect::WeaponToggle { .. } => {
                r.targeting == TargetingMode::SelfTarget && r.cast_range == 0.0
            }
            SkillEffect::RecastZone { .. } | SkillEffect::TrapLine { .. } => {
                r.targeting == TargetingMode::Point && r.cast_range > 0.0
            }
            _ => r.targeting == TargetingMode::Direction && r.cast_range > 0.0,
        };
        if !targeting_valid {
            return Err(format!("invalid targeting for {}", expected.id()));
        }
        result.push(SkillDefinition {
            id: r.id,
            slot: r.slot,
            effect: r.effect,
            damage_type: r.damage_type,
            windup_secs: r.windup_secs,
            recast_mana_cost: r.recast_mana_cost,
            ability: AbilityDefinition {
                id: r.id.id(),
                name: r.name.leak(),
                description: r.description.leak(),
                targeting: r.targeting,
                base_mana_cost: r.mana_cost,
                base_cooldown_secs: r.cooldown_secs,
                cast_range: r.cast_range,
                max_rank: MAX_ABILITY_RANK,
                projectile_damage: r.effect.damage(),
                self_heal: None,
                self_mana_restore: None,
            },
        });
    }
    Ok(result)
}
pub fn skill(id: SkillId) -> &'static SkillDefinition {
    &SKILLS[id as usize]
}
pub fn ensure_loaded() {
    LazyLock::force(&SKILLS);
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SkillSlotState {
    pub can_recast: bool,
    pub recast_remaining_secs: f32,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoadoutState {
    pub concussion_stacks: u8,
    pub brittle: bool,
    pub vital_rotation: u8,
    pub challenge_target: Option<u64>,
    pub challenge_sides: u8,
    pub forge_ready: bool,
    #[serde(default)]
    pub forge_remaining_secs: f32,
    pub energy: bool,
    pub camouflaged: bool,
    pub parrying: bool,
    pub souls: u16,
    pub orb_position: Option<[f32; 2]>,
    pub forged: bool,
    pub recipe: Option<BuildRecipe>,
    pub slots: [SkillSlotState; 4],
    pub weapon_mode: WeaponMode,
    pub shield_hp: f32,
    pub root_remaining_secs: f32,
    /// Stun alone freezes facing; ordinary roots only prevent translation.
    #[serde(default, skip_serializing_if = "seconds_are_zero")]
    pub stun_remaining_secs: f32,
    pub slow_multiplier: f32,
    pub movement_multiplier: f32,
    pub basic_attack_range: f32,
    pub basic_attack_mana_cost: f32,
    pub passive_stacks: u8,
    pub passive_remaining_secs: f32,
    pub mark_remaining_secs: f32,
    pub cast_request_id: u64,
}
fn seconds_are_zero(seconds: &f32) -> bool {
    *seconds == 0.0
}

impl Default for LoadoutState {
    fn default() -> Self {
        Self {
            concussion_stacks: 0,
            brittle: false,
            vital_rotation: 0,
            challenge_target: None,
            challenge_sides: 0,
            forge_ready: false,
            forge_remaining_secs: 0.0,
            energy: false,
            camouflaged: false,
            parrying: false,
            souls: 0,
            orb_position: None,
            forged: false,
            recipe: None,
            slots: [SkillSlotState::default(); 4],
            weapon_mode: WeaponMode::Repeater,
            shield_hp: 0.0,
            root_remaining_secs: 0.0,
            stun_remaining_secs: 0.0,
            slow_multiplier: 1.0,
            movement_multiplier: 1.0,
            basic_attack_range: 0.0,
            basic_attack_mana_cost: 0.0,
            passive_stacks: 0,
            passive_remaining_secs: 0.0,
            mark_remaining_secs: 0.0,
            cast_request_id: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectVisualKind {
    Orb,
    Soul,
    Anchor,
    Healing,
    ShieldWall,
    Cage,
    Lantern,
    Bolt,
    Barrier,
    Field,
    BeamWarning,
    Beam,
    Trap,
    Rocket,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillEffectState {
    pub id: u64,
    pub owner_id: u64,
    pub owner_team: Team,
    pub skill: SkillId,
    pub kind: EffectVisualKind,
    pub position: [f32; 2],
    pub end: [f32; 2],
    pub radius: f32,
    pub remaining_secs: f32,
    pub armed: bool,
    /// Consumed cage walls; lower five bits, zero for other effects.
    #[serde(default)]
    pub consumed_segments: u8,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_and_mixed_recipes_resolve_with_explicit_dependencies() {
        for core in [CoreId::Dawnweaver, CoreId::Wildspark] {
            let recipe = core.preset();
            assert_eq!(resolve(&recipe).unwrap().recipe(), recipe);
        }
        let mut mixed = CoreId::Wildspark.preset();
        mixed.skills[0] = SkillId::DawnBind;
        mixed.skills[1] = SkillId::DawnBarrier;
        mixed.passive = PassiveId::Radiance;
        assert_eq!(
            resolve(&mixed).unwrap().skill(SkillSlot::W).id,
            SkillId::DawnBarrier
        );
        let mut invalid = CoreId::Dawnweaver.preset();
        invalid.skills[0] = SkillId::WildSwitch;
        assert!(matches!(
            resolve(&invalid),
            Err(LoadoutError::RequiresRepeater { .. })
        ));
        invalid.skills[0] = SkillId::DawnRay;
        assert!(matches!(
            resolve(&invalid),
            Err(LoadoutError::DuplicateSkill { .. })
        ));
    }
    #[test]
    fn adventurer_kit_and_each_dagger_skill_resolve_on_unrelated_cores() {
        let adventurer = CoreId::Adventurer.preset();
        let resolved = resolve(&adventurer).unwrap();
        assert_eq!(resolved.core().class(), HeroClass::Adventurer);
        assert_eq!(resolved.passive(), PassiveId::DaggerMastery);
        assert_eq!(resolved.attack_profile(), AttackProfileId::Melee);
        assert_eq!(
            serde_json::from_str::<BuildRecipe>(&serde_json::to_string(&adventurer).unwrap())
                .unwrap(),
            adventurer
        );
        for (slot, skill) in SkillSlot::ALL.into_iter().zip(adventurer.skills) {
            assert_eq!(resolved.skill(slot).id, skill);
            assert_eq!(resolved.skill(slot).ability.targeting, TargetingMode::Point);
            for core in [CoreId::Dawnweaver, CoreId::Wildspark, CoreId::Stormfist] {
                let mut mixed = core.preset();
                mixed.skills[slot.index()] = skill;
                mixed.passive = PassiveId::DaggerMastery;
                let resolved = resolve(&mixed).unwrap();
                assert_eq!(resolved.skill(slot).id, skill);
                assert_eq!(resolved.core(), core);
            }
        }
        let mut fully_mixed = CoreId::Dawnweaver.preset();
        fully_mixed.skills = adventurer.skills;
        assert_eq!(resolve(&fully_mixed).unwrap().skills(), adventurer.skills);
    }

    #[test]
    fn stun_presentation_state_is_distinct_from_root_and_legacy_absence_is_zero() {
        let rooted = LoadoutState {
            root_remaining_secs: 1.5,
            ..Default::default()
        };
        let legacy = serde_json::to_value(&rooted).unwrap();
        assert!(legacy.get("stun_remaining_secs").is_none());
        let parsed: LoadoutState = serde_json::from_value(legacy).unwrap();
        assert_eq!(parsed.stun_remaining_secs, 0.0);
        assert_eq!(parsed.root_remaining_secs, 1.5);
        let stunned = LoadoutState {
            stun_remaining_secs: 0.7,
            ..rooted
        };
        assert_eq!(
            serde_json::from_value::<LoadoutState>(serde_json::to_value(&stunned).unwrap())
                .unwrap(),
            stunned
        );
    }

    #[test]
    fn untrusted_recipes_cannot_inject_numbers_scripts_unknown_skills_or_revision() {
        let recipe = CoreId::Dawnweaver.preset();
        let mut old = recipe.clone();
        old.catalog_revision = "latest".into();
        assert_eq!(resolve(&old), Err(LoadoutError::CatalogRevision));
        old = recipe.clone();
        old.schema_version = 99;
        assert_eq!(resolve(&old), Err(LoadoutError::SchemaVersion));
        let value = serde_json::to_value(&recipe).unwrap();
        for field in ["damage", "avatar", "script", "max_hp"] {
            let mut bad = value.clone();
            bad[field] = serde_json::json!(9999);
            assert!(serde_json::from_value::<BuildRecipe>(bad).is_err());
        }
        let mut bad = value;
        bad["skills"][0] = serde_json::json!("other_skill");
        assert!(serde_json::from_value::<BuildRecipe>(bad).is_err());
        assert!(!valid_aim([f32::NAN, 0.0]));
        assert!(!valid_aim([0.0, f32::INFINITY]));
        assert!(!valid_aim([MAX_AIM_COORDINATE + 1.0, 0.0]));
    }
    #[test]
    fn skill_catalog_rejects_bad_geometry_targeting_and_extra_fields() {
        let good: serde_json::Value = serde_json::from_str(SKILLS_JSON).unwrap();
        for (pointer, value) in [
            ("/skills/0/effect/radius", serde_json::json!(-1)),
            ("/skills/16/recast_mana_cost", serde_json::json!(-1)),
            ("/skills/0/effect/max_hits", serde_json::json!(0)),
            ("/skills/1/targeting", serde_json::json!("unit_target")),
        ] {
            let mut bad = good.clone();
            *bad.pointer_mut(pointer).unwrap() = value;
            assert!(parse_skills(&bad.to_string()).is_err());
        }
        let mut bad = good;
        bad["skills"][0]["effect"]["script"] = serde_json::json!("run()");
        assert!(parse_skills(&bad.to_string()).is_err());
        assert_eq!(parse_skills(SKILLS_JSON).unwrap().len(), SkillId::ALL.len());
    }
}
