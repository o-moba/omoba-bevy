//! Shared ability model and per-player ability snapshot for client/server sync.

use serde::{Deserialize, Serialize};
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

pub const ABILITIES: [AbilityDefinition; 4] = [
    AbilityDefinition {
        id: "arc_bolt",
        name: "Arc Bolt",
        description: "Homing bolt toward a selected enemy.",
        targeting: TargetingMode::UnitTarget,
        base_mana_cost: 20.0,
        base_cooldown_secs: 0.35,
        cast_range: 28.0,
        max_rank: 3,
        projectile_damage: Some(20.0),
        self_heal: None,
        self_mana_restore: None,
    },
    AbilityDefinition {
        id: "second_wind",
        name: "Second Wind",
        description: "Restores health; no target required.",
        targeting: TargetingMode::SelfTarget,
        base_mana_cost: 15.0,
        base_cooldown_secs: 2.0,
        cast_range: 0.0,
        max_rank: 3,
        projectile_damage: None,
        self_heal: Some(15.0),
        self_mana_restore: None,
    },
    AbilityDefinition {
        id: "focused_strike",
        name: "Focused Strike",
        description: "Fast bolt with shorter reach.",
        targeting: TargetingMode::UnitTarget,
        base_mana_cost: 12.0,
        base_cooldown_secs: 0.25,
        cast_range: 18.0,
        max_rank: 3,
        projectile_damage: Some(12.0),
        self_heal: None,
        self_mana_restore: None,
    },
    AbilityDefinition {
        id: "mana_surge",
        name: "Mana Surge",
        description: "Restores mana instantly; no target required.",
        targeting: TargetingMode::SelfTarget,
        base_mana_cost: 0.0,
        base_cooldown_secs: 5.0,
        cast_range: 0.0,
        max_rank: 3,
        projectile_damage: None,
        self_heal: None,
        self_mana_restore: Some(25.0),
    },
];

#[inline]
pub fn ability_for_slot(slot: SkillSlot) -> &'static AbilityDefinition {
    &ABILITIES[slot.index()]
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
    [
        true,
        level >= 2,
        level >= 4,
        level >= 6,
    ]
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlock_slots_follow_level_gates() {
        let u1 = unlocked_slots_for_level(1);
        assert!(u1[0]);
        assert!(!u1[1] && !u1[2] && !u1[3]);
        let u2 = unlocked_slots_for_level(2);
        assert!(u2[0] && u2[1]);
        assert!(!u2[2] && !u2[3]);
        let u6 = unlocked_slots_for_level(6);
        assert!(u6.iter().all(|&x| x));
    }

    #[test]
    fn targeted_and_self_definitions_are_distinct() {
        assert_eq!(
            ability_for_slot(SkillSlot::Q).targeting,
            TargetingMode::UnitTarget
        );
        assert_eq!(
            ability_for_slot(SkillSlot::W).targeting,
            TargetingMode::SelfTarget
        );
    }
}
