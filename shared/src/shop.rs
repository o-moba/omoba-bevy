//! Shop rules and the item model. Item data (names, costs, bonuses) and each
//! class's recommended order live in `shared/assets/catalog/`; purchases and
//! replicated bonuses are server-owned.
use crate::{
    AbilityDefinition, BasicAttackDefinition, HeroClass, SkillSlot, catalog, scaled_cooldown,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const STARTING_GOLD: u32 = 80;
pub const GOLD_PER_SECOND: f32 = 2.0;
pub const HERO_KILL_GOLD: u32 = 500;
pub const HERO_ASSIST_POOL: u32 = 100;
pub const CRITICAL_DAMAGE_MULTIPLIER: f32 = 1.75;
/// A rule, not the catalog size: the catalog may hold more items than a hero
/// can carry.
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
    CritShard,
    SiphonStone,
    WindrunnerBoots,
    DuelistEdge,
    VampiricFang,
    ArcaneFocus,
    Bulwark,
    TempestBlade,
    Bloodreaver,
    AetherCrown,
}

impl ItemId {
    /// Every item in catalog order (`shared/assets/catalog/items.json`).
    pub const ALL: [Self; 16] = [
        Self::EmberBlade,
        Self::SwiftGrip,
        Self::TrailBoots,
        Self::VitalityGem,
        Self::FocusCharm,
        Self::GuardianCrest,
        Self::CritShard,
        Self::SiphonStone,
        Self::WindrunnerBoots,
        Self::DuelistEdge,
        Self::VampiricFang,
        Self::ArcaneFocus,
        Self::Bulwark,
        Self::TempestBlade,
        Self::Bloodreaver,
        Self::AetherCrown,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::EmberBlade => "ember_blade",
            Self::SwiftGrip => "swift_grip",
            Self::TrailBoots => "trail_boots",
            Self::VitalityGem => "vitality_gem",
            Self::FocusCharm => "focus_charm",
            Self::GuardianCrest => "guardian_crest",
            Self::CritShard => "crit_shard",
            Self::SiphonStone => "siphon_stone",
            Self::WindrunnerBoots => "windrunner_boots",
            Self::DuelistEdge => "duelist_edge",
            Self::VampiricFang => "vampiric_fang",
            Self::ArcaneFocus => "arcane_focus",
            Self::Bulwark => "bulwark",
            Self::TempestBlade => "tempest_blade",
            Self::Bloodreaver => "bloodreaver",
            Self::AetherCrown => "aether_crown",
        }
    }

    /// Wire decoding: driven by the enum, never by the catalog.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|item| item.id() == id)
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
    #[serde(skip_serializing_if = "is_zero")]
    pub crit_chance: f32,
    #[serde(skip_serializing_if = "is_zero")]
    pub lifesteal: f32,
}
fn is_zero(value: &f32) -> bool {
    *value == 0.0
}

impl ItemBonuses {
    pub const NONE: Self = Self {
        damage_multiplier: 1.0,
        attack_speed_multiplier: 1.0,
        move_speed_multiplier: 1.0,
        spell_haste_multiplier: 1.0,
        max_hp: 0.0,
        max_mana: 0.0,
        crit_chance: 0.0,
        lifesteal: 0.0,
    };
}

impl Default for ItemBonuses {
    fn default() -> Self {
        Self::NONE
    }
}

/// One shop item, loaded from `shared/assets/catalog/items.json`.
#[derive(Debug, Clone, Copy)]
pub struct ItemDefinition {
    pub id: ItemId,
    pub name: &'static str,
    pub description: &'static str,
    pub cost: u32,
    pub bonuses: ItemBonuses,
    /// Total price includes components; owned recipe components are consumed as credit.
    pub components: &'static [ItemId],
    pub tier: u8,
}

/// Every item definition, in `ItemId::ALL` order.
pub fn items() -> &'static [ItemDefinition] {
    catalog::items()
}

pub fn item(id: ItemId) -> &'static ItemDefinition {
    catalog::item(id)
}

/// Preference order only: every class may buy every item. Lists every item
/// exactly once (checked when the catalog loads).
pub fn recommended_items(class: HeroClass) -> &'static [ItemId] {
    &catalog::hero(class).recommended_items
}

/// Greedy shopping in the class's recommended order, the way bots, the
/// offline duel and the harness buy: skip what `owned` already holds or
/// `gold` cannot pay for, and take the rest while the inventory has room.
/// Buying the returned items one at a time, starting from `owned` and `gold`,
/// never fails a purchase check (given an `owned` without duplicates).
pub fn plan_purchases(class: HeroClass, gold: u32, owned: &[ItemId]) -> Vec<ItemId> {
    let mut gold = gold;
    let mut inventory = owned.to_vec();
    let mut plan = Vec::new();
    for id in recommended_items(class) {
        if inventory_covers(*id, &inventory) {
            continue;
        }
        let Ok(quote) = purchase_quote(*id, gold, &inventory) else {
            continue;
        };
        gold -= quote.cost;
        inventory.retain(|item| !quote.consumed.contains(item));
        inventory.push(*id);
        plan.push(*id);
    }
    plan
}

/// Recommendations do not rebuy a component already represented by an upgrade.
/// This is advisory only: manual purchases may still buy that component.
pub fn inventory_covers(id: ItemId, owned: &[ItemId]) -> bool {
    fn includes(root: ItemId, wanted: ItemId) -> bool {
        root == wanted
            || item(root)
                .components
                .iter()
                .any(|child| includes(*child, wanted))
    }
    owned.iter().any(|root| includes(*root, id))
}

/// Different items add percentage points. A malformed duplicate never stacks.
pub fn item_bonuses(inventory: &[ItemId]) -> ItemBonuses {
    let mut result = ItemBonuses::NONE;
    for def in items() {
        if inventory.contains(&def.id) {
            result.damage_multiplier += def.bonuses.damage_multiplier - 1.0;
            result.attack_speed_multiplier += def.bonuses.attack_speed_multiplier - 1.0;
            result.move_speed_multiplier += def.bonuses.move_speed_multiplier - 1.0;
            result.spell_haste_multiplier += def.bonuses.spell_haste_multiplier - 1.0;
            result.max_hp += def.bonuses.max_hp;
            result.max_mana += def.bonuses.max_mana;
            result.crit_chance += def.bonuses.crit_chance;
            result.lifesteal += def.bonuses.lifesteal;
        }
    }
    result.crit_chance = result.crit_chance.min(0.75);
    result.lifesteal = result.lifesteal.min(0.35);
    result.move_speed_multiplier = result.move_speed_multiplier.min(1.30);
    result
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PurchaseQuote {
    pub cost: u32,
    pub consumed: Vec<ItemId>,
}

/// A recipe visits each owned item once. An owned intermediate consumes its
/// branch, so its already-included leaves cannot also be credited.
pub fn upgrade_quote(id: ItemId, owned: &[ItemId]) -> PurchaseQuote {
    fn credit(id: ItemId, owned: &[ItemId], consumed: &mut Vec<ItemId>) -> u32 {
        if consumed.contains(&id) {
            return 0;
        }
        if owned.contains(&id) {
            consumed.push(id);
            item(id).cost
        } else {
            item(id)
                .components
                .iter()
                .map(|child| credit(*child, owned, consumed))
                .sum()
        }
    }
    let mut consumed = Vec::new();
    let credit: u32 = item(id)
        .components
        .iter()
        .map(|child| credit(*child, owned, &mut consumed))
        .sum();
    PurchaseQuote {
        cost: item(id).cost.saturating_sub(credit),
        consumed,
    }
}

pub fn purchase_quote(
    id: ItemId,
    gold: u32,
    owned: &[ItemId],
) -> Result<PurchaseQuote, PurchaseError> {
    if owned.contains(&id) {
        return Err(PurchaseError::AlreadyOwned);
    }
    let quote = upgrade_quote(id, owned);
    if owned.len().saturating_sub(quote.consumed.len()) >= INVENTORY_CAPACITY {
        return Err(PurchaseError::InventoryFull);
    }
    if gold < quote.cost {
        return Err(PurchaseError::InsufficientGold);
    }
    Ok(quote)
}

/// Repeated defeats without a kill reduce feed value; kills reset the streak.
pub fn hero_kill_bounty(death_streak: u32) -> u32 {
    HERO_KILL_GOLD
        .saturating_sub(death_streak.saturating_mul(100))
        .max(250)
}

pub fn item_cooldown(
    def: &AbilityDefinition,
    rank: u8,
    slot: SkillSlot,
    bonuses: ItemBonuses,
) -> Duration {
    let rate = if slot == SkillSlot::Q && crate::loadout::SkillId::from_id(def.id).is_none() {
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
            assert_eq!(recommended.len(), ItemId::ALL.len());
            assert!(item(recommended[0]).cost <= STARTING_GOLD);
            for def in items() {
                assert_eq!(recommended.iter().filter(|id| **id == def.id).count(), 1);
                assert_eq!(ItemId::from_id(def.id.id()), Some(def.id));
            }
        }
        assert_eq!(ItemId::from_id("free_gold"), None);
    }

    #[test]
    fn purchase_plans_follow_the_recommended_order_within_budget_and_room() {
        use ItemId::*;
        assert_eq!(
            plan_purchases(HeroClass::Ranger, STARTING_GOLD, &[]),
            vec![SwiftGrip]
        );
        assert_eq!(
            plan_purchases(HeroClass::Ranger, STARTING_GOLD, &[SwiftGrip]),
            vec![EmberBlade]
        );
        assert!(plan_purchases(HeroClass::Ranger, STARTING_GOLD - 1, &[]).is_empty());
        for class in HeroClass::ALL {
            let recommended = recommended_items(class);
            // Simulate every planned authority quote, including consumed slots.
            for gold in [0, 79, 80, 250, 1_000, u32::MAX] {
                let mut wallet = gold;
                let mut inventory = recommended[..2].to_vec();
                for id in plan_purchases(class, gold, &inventory) {
                    let quote = purchase_quote(id, wallet, &inventory).unwrap();
                    wallet -= quote.cost;
                    inventory.retain(|id| !quote.consumed.contains(id));
                    inventory.push(id);
                    assert!(inventory.len() <= INVENTORY_CAPACITY);
                    assert_eq!(
                        inventory
                            .iter()
                            .collect::<std::collections::HashSet<_>>()
                            .len(),
                        inventory.len()
                    );
                }
                assert!(wallet <= gold);
            }
        }
    }

    #[test]
    fn bonuses_do_not_stack_duplicates_and_cooldown_rates_have_distinct_slots() {
        let all: Vec<_> = items()[..6].iter().map(|item| item.id).collect();
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
            if !class.is_standard() {
                assert!(definition.range < class.ability(SkillSlot::Q).cast_range);
            }
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

#[cfg(test)]
mod recipe_tests {
    use super::*;
    use ItemId::*;

    #[test]
    fn intermediate_or_leaf_credit_never_double_counts_and_full_slots_can_upgrade() {
        assert_eq!(
            upgrade_quote(TempestBlade, &[EmberBlade, CritShard, SwiftGrip]).cost,
            1_090
        );
        let quote = upgrade_quote(TempestBlade, &[DuelistEdge, EmberBlade, SwiftGrip]);
        assert_eq!(quote.cost, 620);
        assert_eq!(quote.consumed, [DuelistEdge, SwiftGrip]);
        let full = [
            EmberBlade,
            CritShard,
            SwiftGrip,
            TrailBoots,
            FocusCharm,
            SiphonStone,
        ];
        assert!(purchase_quote(DuelistEdge, 470, &full).is_ok());
        assert_eq!(
            purchase_quote(DuelistEdge, 469, &full),
            Err(PurchaseError::InsufficientGold)
        );
        assert_eq!(
            purchase_quote(Bulwark, 9999, &full),
            Err(PurchaseError::InventoryFull)
        );
    }

    #[test]
    fn item_budget_and_bounds_support_distinct_builds() {
        assert_eq!(hero_kill_bounty(0), 500);
        assert_eq!(hero_kill_bounty(100), 250);
        assert!(item_bonuses(&[WindrunnerBoots]).move_speed_multiplier > 1.15);
        assert!(item_bonuses(&[TempestBlade]).attack_speed_multiplier > 1.3);
        assert!(item_bonuses(&[TempestBlade]).crit_chance >= 0.3);
        assert!(item_bonuses(&[Bloodreaver]).lifesteal >= 0.22);
        let all = item_bonuses(&ItemId::ALL);
        assert!(
            all.crit_chance <= 0.75 && all.lifesteal <= 0.35 && all.move_speed_multiplier <= 1.30
        );
        // 2g/s + a solo lane's 3 x 18g wave each minute, starting with80g.
        let solo_income = GOLD_PER_SECOND * 60.0 + 54.0;
        assert!((item(WindrunnerBoots).cost - STARTING_GOLD) as f32 / solo_income < 3.0);
        assert!((item(TempestBlade).cost - STARTING_GOLD) as f32 / solo_income < 8.0);
    }
}
