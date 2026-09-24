//! Shared, bounded hero growth. Item/debug multipliers compose once after growth.
use crate::{
    HeroClass, SkillSlot, ability_for_class_slot, basic_attack_for_class, scaled_cooldown,
    shop::ItemBonuses,
};
use std::time::Duration;

pub const MAX_LEVEL: u32 = 10;

pub const fn base_hp(class: HeroClass) -> f32 {
    match class {
        HeroClass::Warrior => 220.0,
        HeroClass::Mage => 180.0,
        HeroClass::Ranger => 185.0,
        HeroClass::Cleric => 200.0,
        HeroClass::Warden => 210.0,
    }
}
fn growth(level: u32, cap: f32) -> f32 {
    1.0 + (cap - 1.0) * (level.clamp(1, MAX_LEVEL) - 1) as f32 / (MAX_LEVEL - 1) as f32
}
pub fn movement_multiplier(_class: HeroClass, level: u32) -> f32 {
    growth(level, 1.24)
}
pub fn basic_damage_multiplier(class: HeroClass, level: u32) -> f32 {
    growth(
        level,
        match class {
            HeroClass::Warrior => 1.8,
            HeroClass::Mage => 1.6,
            HeroClass::Ranger => 1.85,
            HeroClass::Cleric => 1.5,
            HeroClass::Warden => 1.75,
        },
    )
}
pub fn attack_rate_multiplier(class: HeroClass, level: u32) -> f32 {
    growth(
        level,
        match class {
            HeroClass::Warrior => 1.6,
            HeroClass::Mage => 1.5,
            HeroClass::Ranger => 1.8,
            HeroClass::Cleric => 1.55,
            HeroClass::Warden => 1.65,
        },
    )
}
pub fn ability_power_multiplier(_class: HeroClass, level: u32) -> f32 {
    growth(level, 1.35)
}
pub fn spell_haste_multiplier(_class: HeroClass, level: u32) -> f32 {
    growth(level, 1.2)
}
/// Q/W/E/R share recovery; mana-free basic attacks retain an independent clock.
pub fn skill_recovery_secs(level: u32) -> f32 {
    0.45 - 0.15 * (level.clamp(1, MAX_LEVEL) - 1) as f32 / (MAX_LEVEL - 1) as f32
}
/// Supports validated sandbox zero damage and subunit attack speed. Ordinary
/// callers sanitize item-only multipliers to their existing floor of one.
pub fn basic_damage(class: HeroClass, level: u32, bonuses: ItemBonuses) -> f32 {
    basic_attack_for_class(class).damage
        * basic_damage_multiplier(class, level)
        * bonuses.damage_multiplier.max(0.0)
}
pub fn basic_cooldown(class: HeroClass, level: u32, bonuses: ItemBonuses) -> Duration {
    Duration::from_secs_f32(basic_attack_for_class(class).cooldown_secs)
        .div_f32(attack_rate_multiplier(class, level) * bonuses.attack_speed_multiplier.max(0.1))
}
pub fn ability_cooldown(
    class: HeroClass,
    level: u32,
    rank: u8,
    slot: SkillSlot,
    bonuses: ItemBonuses,
) -> Duration {
    let def = ability_for_class_slot(class, slot);
    let rate = if slot == SkillSlot::Q {
        attack_rate_multiplier(class, level) * bonuses.attack_speed_multiplier.max(0.1)
    } else {
        spell_haste_multiplier(class, level) * bonuses.spell_haste_multiplier.max(1.0)
    };
    scaled_cooldown(def, rank.clamp(1, def.max_rank)).div_f32(rate)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_classes_grow_monotonically_and_cap_even_for_invalid_levels() {
        for class in HeroClass::ALL {
            let start = basic_cooldown(class, 1, ItemBonuses::NONE).as_secs_f32();
            let frequency =
                start / basic_cooldown(class, MAX_LEVEL, ItemBonuses::NONE).as_secs_f32();
            assert!((1.5 - 0.00001..=1.8 + 0.00001).contains(&frequency));
            assert!((1.2..=1.3).contains(&movement_multiplier(class, MAX_LEVEL)));
            assert!(basic_damage_multiplier(class, MAX_LEVEL) >= 1.25);
            for curve in [
                movement_multiplier,
                basic_damage_multiplier,
                attack_rate_multiplier,
                ability_power_multiplier,
                spell_haste_multiplier,
            ] {
                let mut previous = curve(class, 0);
                assert_eq!(previous, 1.0);
                for level in 1..=MAX_LEVEL {
                    let current = curve(class, level);
                    assert!(current.is_finite() && current >= previous);
                    previous = current;
                }
                assert_eq!(curve(class, u32::MAX), previous);
            }
        }
    }
}
