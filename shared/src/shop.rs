//! Small, shared item catalog. Purchases and replicated bonuses are server-owned.
use crate::{AbilityDefinition, BasicAttackDefinition, HeroClass, SkillSlot, scaled_cooldown};
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const STARTING_GOLD: u32 = 80;
pub const GOLD_PER_SECOND: f32 = 1.0;
pub const INVENTORY_CAPACITY: usize = 6;
pub const SHOP_RADIUS: f32 = 18.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemId {
    EmberBlade,
    SwiftGrip,
    TrailBoots,
    VitalityGem,
    FocusCharm,
    GuardianCrest,
}

impl ItemId {
    pub const fn id(self) -> &'static str {
        match self {
            Self::EmberBlade => "ember_blade",
            Self::SwiftGrip => "swift_grip",
            Self::TrailBoots => "trail_boots",
            Self::VitalityGem => "vitality_gem",
            Self::FocusCharm => "focus_charm",
            Self::GuardianCrest => "guardian_crest",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        ITEMS
            .iter()
            .find(|item| item.id.id() == id)
            .map(|item| item.id)
    }
}

/// Multipliers start at one; maximum resource fields are flat bonuses.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ItemBonuses {
    pub damage_multiplier: f32,
    pub attack_speed_multiplier: f32,
    pub move_speed_multiplier: f32,
    pub spell_haste_multiplier: f32,
    pub max_hp: f32,
    pub max_mana: f32,
}

impl ItemBonuses {
    pub const NONE: Self = Self {
        damage_multiplier: 1.0,
        attack_speed_multiplier: 1.0,
        move_speed_multiplier: 1.0,
        spell_haste_multiplier: 1.0,
        max_hp: 0.0,
        max_mana: 0.0,
    };
}

impl Default for ItemBonuses {
    fn default() -> Self {
        Self::NONE
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ItemDefinition {
    pub id: ItemId,
    pub name: &'static str,
    pub description: &'static str,
    pub cost: u32,
    pub bonuses: ItemBonuses,
}

pub const ITEMS: [ItemDefinition; INVENTORY_CAPACITY] = [
    ItemDefinition {
        id: ItemId::EmberBlade,
        name: "Ember Blade",
        description: "+12% damage",
        cost: 80,
        bonuses: ItemBonuses {
            damage_multiplier: 1.12,
            ..ItemBonuses::NONE
        },
    },
    ItemDefinition {
        id: ItemId::SwiftGrip,
        name: "Swift Grip",
        description: "+12% basic attack and Q rate",
        cost: 80,
        bonuses: ItemBonuses {
            attack_speed_multiplier: 1.12,
            ..ItemBonuses::NONE
        },
    },
    ItemDefinition {
        id: ItemId::TrailBoots,
        name: "Trail Boots",
        description: "+8% movement speed",
        cost: 80,
        bonuses: ItemBonuses {
            move_speed_multiplier: 1.08,
            ..ItemBonuses::NONE
        },
    },
    ItemDefinition {
        id: ItemId::VitalityGem,
        name: "Vitality Gem",
        description: "+30 maximum HP",
        cost: 80,
        bonuses: ItemBonuses {
            max_hp: 30.0,
            ..ItemBonuses::NONE
        },
    },
    ItemDefinition {
        id: ItemId::FocusCharm,
        name: "Focus Charm",
        description: "+20 mana / +10% W E R haste",
        cost: 100,
        bonuses: ItemBonuses {
            max_mana: 20.0,
            spell_haste_multiplier: 1.10,
            ..ItemBonuses::NONE
        },
    },
    ItemDefinition {
        id: ItemId::GuardianCrest,
        name: "Guardian Crest",
        description: "+15 maximum HP / +6% damage",
        cost: 120,
        bonuses: ItemBonuses {
            max_hp: 15.0,
            damage_multiplier: 1.06,
            ..ItemBonuses::NONE
        },
    },
];

pub fn item(id: ItemId) -> &'static ItemDefinition {
    ITEMS
        .iter()
        .find(|item| item.id == id)
        .expect("every ItemId has a definition")
}

/// Preference order only: every class may buy every item.
pub fn recommended_items(class: HeroClass) -> &'static [ItemId] {
    use ItemId::*;
    match class {
        HeroClass::Warrior => &[
            VitalityGem,
            EmberBlade,
            GuardianCrest,
            TrailBoots,
            SwiftGrip,
            FocusCharm,
        ],
        HeroClass::Mage => &[
            EmberBlade,
            FocusCharm,
            TrailBoots,
            VitalityGem,
            GuardianCrest,
            SwiftGrip,
        ],
        HeroClass::Ranger => &[
            SwiftGrip,
            EmberBlade,
            TrailBoots,
            VitalityGem,
            GuardianCrest,
            FocusCharm,
        ],
        HeroClass::Cleric => &[
            VitalityGem,
            FocusCharm,
            TrailBoots,
            GuardianCrest,
            SwiftGrip,
            EmberBlade,
        ],
    }
}

/// Different items add percentage points. A malformed duplicate never stacks.
pub fn item_bonuses(inventory: &[ItemId]) -> ItemBonuses {
    let mut result = ItemBonuses::NONE;
    for def in &ITEMS {
        if inventory.contains(&def.id) {
            result.damage_multiplier += def.bonuses.damage_multiplier - 1.0;
            result.attack_speed_multiplier += def.bonuses.attack_speed_multiplier - 1.0;
            result.move_speed_multiplier += def.bonuses.move_speed_multiplier - 1.0;
            result.spell_haste_multiplier += def.bonuses.spell_haste_multiplier - 1.0;
            result.max_hp += def.bonuses.max_hp;
            result.max_mana += def.bonuses.max_mana;
        }
    }
    result
}

pub fn item_cooldown(
    def: &AbilityDefinition,
    rank: u8,
    slot: SkillSlot,
    bonuses: ItemBonuses,
) -> Duration {
    let rate = if slot == SkillSlot::Q {
        bonuses.attack_speed_multiplier
    } else {
        bonuses.spell_haste_multiplier
    };
    scaled_cooldown(def, rank).div_f32(rate.max(1.0))
}

pub fn basic_attack_damage(def: &BasicAttackDefinition, bonuses: ItemBonuses) -> f32 {
    def.damage * bonuses.damage_multiplier.max(1.0)
}

pub fn basic_attack_cooldown(def: &BasicAttackDefinition, bonuses: ItemBonuses) -> Duration {
    Duration::from_secs_f32(def.cooldown_secs).div_f32(bonuses.attack_speed_multiplier.max(1.0))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PurchaseError {
    Unavailable,
    Dead,
    OutsideBase,
    InsufficientGold,
    AlreadyOwned,
    InventoryFull,
    UnknownItem,
}

impl PurchaseError {
    pub const fn message(self) -> &'static str {
        match self {
            Self::Unavailable => "The shop is unavailable until you join a match.",
            Self::Dead => "Wait to respawn before buying.",
            Self::OutsideBase => "Return to your base to buy.",
            Self::InsufficientGold => "You need more gold.",
            Self::AlreadyOwned => "You already own this item.",
            Self::InventoryFull => "All six inventory slots are full.",
            Self::UnknownItem => "This item is not in the current catalog.",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PurchaseReceipt {
    pub request_id: u64,
    pub match_id: u64,
    pub item_id: Option<ItemId>,
    pub error: Option<PurchaseError>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_recommendations_and_starter_budget_are_coherent() {
        for class in HeroClass::ALL {
            let recommended = recommended_items(class);
            assert_eq!(recommended.len(), INVENTORY_CAPACITY);
            assert!(item(recommended[0]).cost <= STARTING_GOLD);
            for def in ITEMS {
                assert_eq!(recommended.iter().filter(|id| **id == def.id).count(), 1);
                assert_eq!(ItemId::from_id(def.id.id()), Some(def.id));
            }
        }
        assert_eq!(ItemId::from_id("free_gold"), None);
    }

    #[test]
    fn bonuses_do_not_stack_duplicates_and_cooldown_rates_have_distinct_slots() {
        let all: Vec<_> = ITEMS.iter().map(|item| item.id).collect();
        let bonuses = item_bonuses(&all);
        assert!((bonuses.damage_multiplier - 1.18).abs() < 0.0001);
        assert_eq!((bonuses.max_hp, bonuses.max_mana), (45.0, 20.0));
        assert_eq!(item_bonuses(&[ItemId::VitalityGem; 6]).max_hp, 30.0);
        for slot in SkillSlot::ALL {
            let def = crate::ability_for_class_slot(HeroClass::Mage, slot);
            let speed = if slot == SkillSlot::Q { 1.12 } else { 1.10 };
            assert!(
                (item_cooldown(def, 1, slot, bonuses).as_secs_f32() * speed
                    - scaled_cooldown(def, 1).as_secs_f32())
                .abs()
                    < 0.0001
            );
        }
        assert_eq!(
            serde_json::from_str::<ItemBonuses>("{}").unwrap(),
            ItemBonuses::NONE
        );
    }
    #[test]
    fn basic_attack_uses_damage_and_attack_speed_without_skill_haste_or_ranks() {
        for class in HeroClass::ALL {
            let definition = crate::basic_attack_for_class(class);
            assert!(
                definition.range > 0.0 && definition.damage > 0.0 && definition.cooldown_secs > 0.0
            );
            assert!(definition.range < class.ability(SkillSlot::Q).cast_range);
            let base = basic_attack_cooldown(definition, ItemBonuses::NONE);
            let haste = item_bonuses(&[ItemId::FocusCharm]);
            assert_eq!(basic_attack_cooldown(definition, haste), base);
            let attack_items = item_bonuses(&[ItemId::SwiftGrip, ItemId::EmberBlade]);
            assert!(
                (basic_attack_damage(definition, attack_items) - definition.damage * 1.12).abs()
                    < 0.0001
            );
            assert!(
                (basic_attack_cooldown(definition, attack_items).as_secs_f32() * 1.12
                    - base.as_secs_f32())
                .abs()
                    < 0.0001
            );
        }
    }
}
