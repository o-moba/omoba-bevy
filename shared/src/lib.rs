//! Shared gameplay model for client/server sync: hero classes with per-class
//! Q/W/E/R ability kits, rank/unlock mechanics, per-player ability snapshots,
//! and the cosmetic avatar roster (mirrors `client/assets/avatars/manifest.json`).

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::Duration;

/// Logical hotbar slot; maps to `Q` / `W` / `E` / `R` on the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillSlot {
    Q,
    W,
    E,
    R,
}

impl SkillSlot {
    pub const ALL: [Self; 4] = [Self::Q, Self::W, Self::E, Self::R];

    pub const fn index(self) -> usize {
        match self {
            Self::Q => 0,
            Self::W => 1,
            Self::E => 2,
            Self::R => 3,
        }
    }

    pub fn from_index(index: u8) -> Option<Self> {
        match index {
            0 => Some(Self::Q),
            1 => Some(Self::W),
            2 => Some(Self::E),
            3 => Some(Self::R),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetingMode {
    /// Requires a hostile target in range.
    UnitTarget,
    /// Ignores target; effect applies to the caster only.
    SelfTarget,
}

/// Static definition for one ability. This is the single source of truth for id, costs, and UX text.
#[derive(Debug, Clone, Copy)]
pub struct AbilityDefinition {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub targeting: TargetingMode,
    pub base_mana_cost: f32,
    pub base_cooldown_secs: f32,
    pub cast_range: f32,
    pub max_rank: u8,
    pub projectile_damage: Option<f32>,
    pub self_heal: Option<f32>,
    pub self_mana_restore: Option<f32>,
}

/// Maximum investable rank shared by every ability definition.
pub const MAX_ABILITY_RANK: u8 = 3;

#[allow(clippy::too_many_arguments)]
const fn ability(
    id: &'static str,
    name: &'static str,
    description: &'static str,
    targeting: TargetingMode,
    base_mana_cost: f32,
    base_cooldown_secs: f32,
    cast_range: f32,
    projectile_damage: Option<f32>,
    self_heal: Option<f32>,
    self_mana_restore: Option<f32>,
) -> AbilityDefinition {
    AbilityDefinition {
        id,
        name,
        description,
        targeting,
        base_mana_cost,
        base_cooldown_secs,
        cast_range,
        max_rank: MAX_ABILITY_RANK,
        projectile_damage,
        self_heal,
        self_mana_restore,
    }
}

/// Playable hero class. Selected before joining; the server resolves the
/// matching ability kit authoritatively per player.
///
/// Wire format: snake_case string; **unknown values decode as the default
/// class (Warrior)** so a bad or future client cannot break packet parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HeroClass {
    #[default]
    Warrior,
    Mage,
    Ranger,
    Cleric,
}

impl HeroClass {
    pub const ALL: [Self; 4] = [Self::Warrior, Self::Mage, Self::Ranger, Self::Cleric];

    /// Stable wire/UI identifier (snake_case).
    pub const fn id(self) -> &'static str {
        match self {
            Self::Warrior => "warrior",
            Self::Mage => "mage",
            Self::Ranger => "ranger",
            Self::Cleric => "cleric",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|class| class.id() == id)
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Warrior => "Warrior",
            Self::Mage => "Mage",
            Self::Ranger => "Ranger",
            Self::Cleric => "Cleric",
        }
    }

    /// One-line kit summary for the class-select UI.
    pub const fn tagline(self) -> &'static str {
        match self {
            Self::Warrior => "Close-range bruiser: heavy hits, self-sustain",
            Self::Mage => "Long-range nuker: big damage, mana engine",
            Self::Ranger => "Fast skirmisher: rapid shots, longest reach",
            Self::Cleric => "Support: modest damage, strong heals",
        }
    }

    /// The class's Q/W/E/R kit (index = `SkillSlot::index()`).
    pub const fn abilities(self) -> &'static [AbilityDefinition; 4] {
        match self {
            Self::Warrior => &WARRIOR_ABILITIES,
            Self::Mage => &MAGE_ABILITIES,
            Self::Ranger => &RANGER_ABILITIES,
            Self::Cleric => &CLERIC_ABILITIES,
        }
    }

    pub const fn ability(self, slot: SkillSlot) -> &'static AbilityDefinition {
        &self.abilities()[slot.index()]
    }
}

impl Serialize for HeroClass {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.id())
    }
}

impl<'de> Deserialize<'de> for HeroClass {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        // Unknown class ids resolve to the default class instead of failing the
        // whole packet: the server logs the resolved class on join.
        Ok(Self::from_id(&raw).unwrap_or_default())
    }
}

pub const WARRIOR_ABILITIES: [AbilityDefinition; 4] = [
    ability(
        "shield_bash",
        "Shield Bash",
        "Slams the selected enemy at close range.",
        TargetingMode::UnitTarget,
        10.0,
        0.5,
        12.0,
        Some(24.0),
        None,
        None,
    ),
    ability(
        "battle_rally",
        "Battle Rally",
        "Steels yourself, restoring health.",
        TargetingMode::SelfTarget,
        18.0,
        6.0,
        0.0,
        None,
        Some(20.0),
        None,
    ),
    ability(
        "heroic_strike",
        "Heroic Strike",
        "A crushing blow with very short reach.",
        TargetingMode::UnitTarget,
        22.0,
        2.5,
        10.0,
        Some(38.0),
        None,
        None,
    ),
    ability(
        "rampage",
        "Rampage",
        "Devastating close-range finisher.",
        TargetingMode::UnitTarget,
        40.0,
        14.0,
        14.0,
        Some(60.0),
        None,
        None,
    ),
];

pub const MAGE_ABILITIES: [AbilityDefinition; 4] = [
    ability(
        "arc_bolt",
        "Arc Bolt",
        "Homing bolt toward a selected enemy.",
        TargetingMode::UnitTarget,
        22.0,
        0.4,
        30.0,
        Some(18.0),
        None,
        None,
    ),
    ability(
        "mana_surge",
        "Mana Surge",
        "Restores mana instantly; no target required.",
        TargetingMode::SelfTarget,
        0.0,
        8.0,
        0.0,
        None,
        None,
        Some(35.0),
    ),
    ability(
        "frost_lance",
        "Frost Lance",
        "Piercing shard with strong impact damage.",
        TargetingMode::UnitTarget,
        28.0,
        3.0,
        26.0,
        Some(34.0),
        None,
        None,
    ),
    ability(
        "pyroblast",
        "Pyroblast",
        "Massive fireball with the longest reach in the mage kit.",
        TargetingMode::UnitTarget,
        50.0,
        16.0,
        32.0,
        Some(72.0),
        None,
        None,
    ),
];

pub const RANGER_ABILITIES: [AbilityDefinition; 4] = [
    ability(
        "quick_shot",
        "Quick Shot",
        "Very fast arrow with modest damage.",
        TargetingMode::UnitTarget,
        14.0,
        0.25,
        24.0,
        Some(14.0),
        None,
        None,
    ),
    ability(
        "field_dressing",
        "Field Dressing",
        "Patches wounds on the move.",
        TargetingMode::SelfTarget,
        16.0,
        5.0,
        0.0,
        None,
        Some(14.0),
        None,
    ),
    ability(
        "piercing_arrow",
        "Piercing Arrow",
        "Heavy arrow with extended reach.",
        TargetingMode::UnitTarget,
        24.0,
        2.0,
        34.0,
        Some(30.0),
        None,
        None,
    ),
    ability(
        "longshot",
        "Longshot",
        "Sniper shot across the longest range of any kit.",
        TargetingMode::UnitTarget,
        36.0,
        12.0,
        40.0,
        Some(48.0),
        None,
        None,
    ),
];

pub const CLERIC_ABILITIES: [AbilityDefinition; 4] = [
    ability(
        "smite",
        "Smite",
        "Radiant bolt against a selected enemy.",
        TargetingMode::UnitTarget,
        12.0,
        0.6,
        22.0,
        Some(16.0),
        None,
        None,
    ),
    ability(
        "renew",
        "Renew",
        "Mends the caster's wounds.",
        TargetingMode::SelfTarget,
        20.0,
        4.0,
        0.0,
        None,
        Some(26.0),
        None,
    ),
    ability(
        "divine_favor",
        "Divine Favor",
        "Channels faith into mana.",
        TargetingMode::SelfTarget,
        0.0,
        9.0,
        0.0,
        None,
        None,
        Some(24.0),
    ),
    ability(
        "guardians_blessing",
        "Guardian's Blessing",
        "Major self-restoration; the strongest heal in the game.",
        TargetingMode::SelfTarget,
        30.0,
        18.0,
        0.0,
        None,
        Some(60.0),
        None,
    ),
];

/// Resolves the ability for a class and slot (server-authoritative kit lookup).
#[inline]
pub fn ability_for_class_slot(class: HeroClass, slot: SkillSlot) -> &'static AbilityDefinition {
    class.ability(slot)
}

/// Rank is 1-based.
#[inline]
pub fn rank_effect_scale(rank: u8) -> f32 {
    1.0 + 0.1 * (rank.saturating_sub(1) as f32)
}

#[inline]
pub fn scaled_mana_cost(def: &AbilityDefinition, rank: u8) -> f32 {
    def.base_mana_cost * rank_effect_scale(rank)
}

#[inline]
pub fn scaled_cooldown(def: &AbilityDefinition, rank: u8) -> Duration {
    let factor = (1.0 - 0.06 * (rank.saturating_sub(1) as f32)).max(0.55);
    Duration::from_secs_f32(def.base_cooldown_secs * factor)
}

#[inline]
pub fn scaled_cast_range(def: &AbilityDefinition, rank: u8) -> f32 {
    if def.cast_range <= 0.0 {
        0.0
    } else {
        def.cast_range * (1.0 + 0.04 * (rank.saturating_sub(1) as f32))
    }
}

/// Which slots are unlocked at a given hero level (foundation progression).
#[inline]
pub fn unlocked_slots_for_level(level: u32) -> [bool; 4] {
    [true, level >= 2, level >= 4, level >= 6]
}

/// Hero level at which each slot unlocks (Q/W/E/R order).
pub const SLOT_UNLOCK_LEVELS: [u32; 4] = [1, 2, 4, 6];

/// Network + UI snapshot for the local hotbar (recomputed on the server each tick).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlayerAbilitySnapshot {
    #[serde(default)]
    pub cooldown_remaining: [f32; 4],
    #[serde(default = "default_ranks")]
    pub ranks: [u8; 4],
    #[serde(default = "default_unlocked")]
    pub unlocked: [bool; 4],
    #[serde(default)]
    pub rank_upgrade_available: [bool; 4],
}

fn default_ranks() -> [u8; 4] {
    [1, 1, 1, 1]
}

fn default_unlocked() -> [bool; 4] {
    [true, false, false, false]
}

impl Default for PlayerAbilitySnapshot {
    fn default() -> Self {
        let level = 1;
        Self {
            cooldown_remaining: [0.0; 4],
            ranks: default_ranks(),
            unlocked: unlocked_slots_for_level(level),
            rank_upgrade_available: [false; 4],
        }
    }
}

impl PlayerAbilitySnapshot {
    pub fn fresh_for_level(level: u32) -> Self {
        Self {
            cooldown_remaining: [0.0; 4],
            ranks: default_ranks(),
            unlocked: unlocked_slots_for_level(level),
            rank_upgrade_available: [false; 4],
        }
    }
}

// --- Avatar roster (cosmetic; mirrors client/assets/avatars/manifest.json) ---

/// One shipped avatar. All roster avatars are CC0 VRM models staged as GLB
/// under `client/assets/avatars/<slug>.glb` with retargeted animation clips.
#[derive(Debug, Clone, Deserialize)]
pub struct AvatarDefinition {
    pub slug: String,
    pub display_name: String,
    pub collection: String,
    pub license: String,
    pub source_url: String,
    #[serde(default)]
    pub author: Option<String>,
    /// Thumbnail file name relative to `client/assets/avatars/`, if shipped.
    #[serde(default)]
    pub thumbnail: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AvatarManifest {
    avatars: Vec<AvatarDefinition>,
}

/// The committed roster manifest, embedded at compile time so client and
/// server agree on the exact shipped avatar set.
const AVATAR_MANIFEST_JSON: &str = include_str!("../../client/assets/avatars/manifest.json");

/// All shipped avatars, in manifest order.
pub fn avatar_roster() -> &'static [AvatarDefinition] {
    static ROSTER: OnceLock<Vec<AvatarDefinition>> = OnceLock::new();
    ROSTER.get_or_init(|| {
        serde_json::from_str::<AvatarManifest>(AVATAR_MANIFEST_JSON)
            .expect("embedded avatar manifest must parse")
            .avatars
    })
}

/// Looks up a shipped avatar by slug.
pub fn avatar_definition(slug: &str) -> Option<&'static AvatarDefinition> {
    avatar_roster().iter().find(|avatar| avatar.slug == slug)
}

/// Normalizes a client-supplied avatar slug: only slugs present in the shipped
/// roster survive; anything else (unknown, malformed, path-like) becomes `None`
/// so the receiving side falls back to the default model.
pub fn normalize_avatar_slug(raw: Option<&str>) -> Option<&'static str> {
    let slug = raw?.trim();
    avatar_definition(slug).map(|avatar| avatar.slug.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn unlock_slots_follow_level_gates() {
        let u1 = unlocked_slots_for_level(1);
        assert!(u1[0]);
        assert!(!u1[1] && !u1[2] && !u1[3]);
        let u2 = unlocked_slots_for_level(2);
        assert!(u2[0] && u2[1]);
        assert!(!u2[2] && !u2[3]);
        let u4 = unlocked_slots_for_level(4);
        assert!(u4[0] && u4[1] && u4[2]);
        assert!(!u4[3]);
        let u6 = unlocked_slots_for_level(6);
        assert!(u6.iter().all(|&x| x));
        for (slot, unlock_level) in SLOT_UNLOCK_LEVELS.iter().enumerate() {
            assert!(unlocked_slots_for_level(*unlock_level)[slot]);
            if *unlock_level > 1 {
                assert!(!unlocked_slots_for_level(*unlock_level - 1)[slot]);
            }
        }
    }

    #[test]
    fn every_class_resolves_a_full_distinct_kit() {
        let mut ids = HashSet::new();
        for class in HeroClass::ALL {
            for slot in SkillSlot::ALL {
                let def = ability_for_class_slot(class, slot);
                assert_eq!(def.id, class.abilities()[slot.index()].id);
                assert!(
                    ids.insert(def.id),
                    "ability id {:?} is reused across kits",
                    def.id
                );
                assert!(!def.name.is_empty() && !def.description.is_empty());
                assert_eq!(def.max_rank, MAX_ABILITY_RANK);
                // Exactly one effect primitive per ability.
                let effects = [
                    def.projectile_damage.is_some(),
                    def.self_heal.is_some(),
                    def.self_mana_restore.is_some(),
                ];
                assert_eq!(effects.iter().filter(|&&x| x).count(), 1, "{}", def.id);
                match def.targeting {
                    TargetingMode::UnitTarget => {
                        assert!(def.projectile_damage.is_some(), "{}", def.id);
                        assert!(def.cast_range > 0.0, "{}", def.id);
                    }
                    TargetingMode::SelfTarget => {
                        assert!(def.projectile_damage.is_none(), "{}", def.id);
                        assert_eq!(def.cast_range, 0.0, "{}", def.id);
                    }
                }
            }
        }
        assert_eq!(ids.len(), 16, "expected 16 distinct ability definitions");
    }

    #[test]
    fn kits_differ_between_classes_per_slot() {
        for slot in SkillSlot::ALL {
            let ids: HashSet<&str> = HeroClass::ALL
                .iter()
                .map(|class| class.ability(slot).id)
                .collect();
            assert_eq!(ids.len(), 4, "slot {slot:?} must differ across classes");
        }
        // Every class has a usable Q at level 1 (UnitTarget damage opener).
        for class in HeroClass::ALL {
            assert_eq!(
                class.ability(SkillSlot::Q).targeting,
                TargetingMode::UnitTarget
            );
        }
    }

    #[test]
    fn rank_scaling_applies_to_class_abilities() {
        let def = ability_for_class_slot(HeroClass::Mage, SkillSlot::Q);
        assert!((rank_effect_scale(1) - 1.0).abs() < f32::EPSILON);
        assert!((rank_effect_scale(3) - 1.2).abs() < f32::EPSILON);
        assert!(scaled_mana_cost(def, 3) > scaled_mana_cost(def, 1));
        assert!(scaled_cooldown(def, 3) < scaled_cooldown(def, 1));
        assert!(scaled_cast_range(def, 3) > scaled_cast_range(def, 1));
        let heal = ability_for_class_slot(HeroClass::Cleric, SkillSlot::W);
        assert_eq!(scaled_cast_range(heal, 3), 0.0);
        let scaled_heal = heal.self_heal.unwrap() * rank_effect_scale(3);
        assert!(scaled_heal > heal.self_heal.unwrap());
    }

    #[test]
    fn hero_class_wire_format_is_snake_case_with_fallback() {
        assert_eq!(
            serde_json::to_string(&HeroClass::Cleric).unwrap(),
            "\"cleric\""
        );
        let parsed: HeroClass = serde_json::from_str("\"ranger\"").unwrap();
        assert_eq!(parsed, HeroClass::Ranger);
        // Unknown class ids fall back to the default instead of failing.
        let unknown: HeroClass = serde_json::from_str("\"necromancer\"").unwrap();
        assert_eq!(unknown, HeroClass::Warrior);
        assert_eq!(HeroClass::from_id("mage"), Some(HeroClass::Mage));
        assert_eq!(HeroClass::from_id("bogus"), None);
    }

    #[test]
    fn avatar_roster_is_committed_and_licensed() {
        let roster = avatar_roster();
        assert!(
            (10..=20).contains(&roster.len()),
            "roster size {} outside [10, 20]",
            roster.len()
        );
        let mut slugs = HashSet::new();
        for avatar in roster {
            assert!(slugs.insert(avatar.slug.as_str()), "duplicate slug");
            assert_eq!(avatar.license, "CC0", "{}", avatar.slug);
            assert!(!avatar.source_url.is_empty(), "{}", avatar.slug);
            assert!(!avatar.display_name.is_empty(), "{}", avatar.slug);
        }
    }

    #[test]
    fn avatar_slug_normalization_rejects_unknown_values() {
        let first = &avatar_roster()[0];
        assert_eq!(
            normalize_avatar_slug(Some(first.slug.as_str())),
            Some(first.slug.as_str())
        );
        assert_eq!(normalize_avatar_slug(Some("../etc/passwd")), None);
        assert_eq!(normalize_avatar_slug(Some("not-a-real-avatar")), None);
        assert_eq!(normalize_avatar_slug(Some("")), None);
        assert_eq!(normalize_avatar_slug(None), None);
    }
}
