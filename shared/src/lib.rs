//! Shared gameplay model for client/server sync: hero classes with per-class
//! Q/W/E/R ability kits (data in `shared/assets/catalog/heroes.json`, see
//! [`catalog`]), rank/unlock mechanics, per-player ability snapshots, and the
//! frozen sprite character ids. No I/O: the avatar roster and asset root live
//! in `omoba_passport::{avatars, assets}`, the sprite manifest in the client.

use serde::{Deserialize, Serialize};
use std::time::Duration;

pub mod career;
pub mod catalog;
pub mod combat;
pub mod debug;
pub mod device_account;
pub mod forest_pickups;
pub mod hero_balance;
pub mod jungle;
pub mod live_score;
pub mod map;
pub mod match_service;
pub mod math;
pub mod navigation;
pub mod practice;
pub mod prematch;
pub mod progression;
pub mod protocol;
pub mod public_transport;
pub mod sandbox;
pub mod shop;
pub mod social;
pub mod supporter;
pub mod transport;
pub mod utility;
pub mod vision;
pub mod web_account;

/// Gameplay wire protocol: the only definition of the UDP/JSON packet types.
pub use protocol::wire;

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

/// Cosmetic action replicated with each player snapshot. Unknown future wire
/// values resolve to `None`, keeping action playback additive and safe for
/// mixed-version clients.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerActionKind {
    Attack,
    Cast,
    #[default]
    #[serde(other)]
    None,
}

/// Basic attacks are not a Q/W/E/R slot. Cosmetic action consumers can still
/// play the Attack animation without treating this event as a Q cast.
pub const BASIC_ATTACK_ACTION_SLOT: u8 = u8::MAX;

/// Shared target surfaces for authoritative attack reach and client approach.
/// These are the existing simulation radii, independent of visual model scale.
pub const PLAYER_TARGET_RADIUS: f32 = 0.62;
/// Projectile hit volume shared by authoritative combat and debug visualization.
pub const PROJECTILE_COLLISION_RADIUS: f32 = 0.22;
pub const MINION_TARGET_RADIUS: f32 = 0.55;
pub const NEUTRAL_TARGET_RADIUS: f32 = 0.62;
pub const TOWER_TARGET_RADIUS: f32 = 1.3;
pub const BASE_TOWER_TARGET_RADIUS: f32 = 3.0;

/// An always-available, mana-free strike. Skill ranks and Q/W/E/R cooldowns
/// never participate in this definition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasicAttackDefinition {
    pub range: f32,
    pub damage: f32,
    pub cooldown_secs: f32,
}

pub fn basic_attack_for_class(class: HeroClass) -> &'static BasicAttackDefinition {
    &catalog::hero(class).basic_attack
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetingMode {
    /// Requires a hostile target in range.
    UnitTarget,
    /// Ignores target; effect applies to the caster only.
    SelfTarget,
}

/// Definition of one ability: id, costs and UX text. The values come from the
/// class's kit in `shared/assets/catalog/heroes.json`.
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
    Warden,
}

impl HeroClass {
    pub const ALL: [Self; 5] = [
        Self::Warrior,
        Self::Mage,
        Self::Ranger,
        Self::Cleric,
        Self::Warden,
    ];

    /// Stable wire/UI identifier (snake_case).
    pub const fn id(self) -> &'static str {
        match self {
            Self::Warrior => "warrior",
            Self::Mage => "mage",
            Self::Ranger => "ranger",
            Self::Cleric => "cleric",
            Self::Warden => "warden",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|class| class.id() == id)
    }

    pub fn display_name(self) -> &'static str {
        catalog::hero(self).display_name
    }

    /// One-line kit summary for the class-select UI.
    pub fn tagline(self) -> &'static str {
        catalog::hero(self).tagline
    }

    /// The draft duty this kit is built for. Every class owns a distinct role,
    /// so one of each fills a five-player team. Players may still pick any role.
    pub fn primary_role(self) -> prematch::Role {
        catalog::hero(self).role
    }

    /// The class's Q/W/E/R kit (index = `SkillSlot::index()`).
    pub fn abilities(self) -> &'static [AbilityDefinition; 4] {
        &catalog::hero(self).abilities
    }

    pub fn ability(self, slot: SkillSlot) -> &'static AbilityDefinition {
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

// --- 2D sprite ids (the client owns the sprite manifest; see client/src/sprite_roster.rs) ---

/// Stable sprite ids are deliberately kept in shared code so untrusted wire
/// values can be normalized without ever becoming file-system paths.
pub const SPRITE_CHARACTER_IDS: [&str; 10] = [
    "mossback-teapot",
    "neon-axolotl-courier",
    "origami-storm-heron",
    "clockwork-turnip-oracle",
    "void-jelly-astronaut",
    "cathedral-moth-bellringer",
    "tidal-typewriter-crab",
    "lunar-marionette-giraffe",
    "aurora-magnet-ram",
    "orchard-comet-centaur",
];
pub const DEFAULT_SPRITE_CHARACTER_ID: &str = SPRITE_CHARACTER_IDS[0];

/// Normalize an optional/untrusted wire id to the frozen roster default. The
/// client's sprite manifest lists exactly these ids (pinned by a client test).
pub fn normalize_sprite_character_id(raw: Option<&str>) -> &'static str {
    raw.map(str::trim)
        .and_then(|id| {
            SPRITE_CHARACTER_IDS
                .iter()
                .copied()
                .find(|known| *known == id)
        })
        .unwrap_or(DEFAULT_SPRITE_CHARACTER_ID)
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
        assert_eq!(
            ids.len(),
            HeroClass::ALL.len() * 4,
            "expected four distinct ability definitions per class"
        );
    }

    #[test]
    fn five_classes_fill_five_distinct_roles() {
        let roles: Vec<_> = HeroClass::ALL.map(HeroClass::primary_role).into();
        for role in prematch::Role::ALL {
            assert_eq!(roles.iter().filter(|r| **r == role).count(), 1, "{role:?}");
        }
        assert_eq!(HeroClass::from_id("warden"), Some(HeroClass::Warden));
    }

    #[test]
    fn kits_differ_between_classes_per_slot() {
        for slot in SkillSlot::ALL {
            let ids: HashSet<&str> = HeroClass::ALL
                .iter()
                .map(|class| class.ability(slot).id)
                .collect();
            assert_eq!(
                ids.len(),
                HeroClass::ALL.len(),
                "slot {slot:?} must differ across classes"
            );
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
    fn player_action_kind_unknown_wire_value_is_inert() {
        assert_eq!(
            serde_json::from_str::<PlayerActionKind>(r#""attack""#).unwrap(),
            PlayerActionKind::Attack
        );
        assert_eq!(
            serde_json::from_str::<PlayerActionKind>(r#""future_action""#).unwrap(),
            PlayerActionKind::None
        );
    }

    #[test]
    fn sprite_ids_normalize_to_the_frozen_list_with_safe_fallback() {
        for id in SPRITE_CHARACTER_IDS {
            assert_eq!(normalize_sprite_character_id(Some(id)), id);
            assert_eq!(normalize_sprite_character_id(Some(&format!(" {id} "))), id);
        }
        for unsafe_or_unknown in [None, Some(""), Some("../mossback-teapot"), Some("unknown")] {
            assert_eq!(
                normalize_sprite_character_id(unsafe_or_unknown),
                DEFAULT_SPRITE_CHARACTER_ID
            );
        }
    }
}
