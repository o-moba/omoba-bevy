//! Shared, bounded hero growth. Item/debug multipliers compose once after growth.
use crate::{
    HeroClass, SkillSlot, ability_for_class_slot, basic_attack_for_class, catalog, scaled_cooldown,
    shop::ItemBonuses,
};
use std::time::Duration;

pub const MAX_LEVEL: u32 = 10;
pub const STARTING_LEVEL: u32 = 1;
/// Hero ground speed in world units per second before growth, items and debug
/// multipliers. The server movement envelope and client prediction share it.
pub const PLAYER_SPEED: f32 = 5.0;
/// Debug movement multiplier applied when a client enables the speed-boost toggle.
pub const DEBUG_SPEED_MULTIPLIER: f32 = 2.6;
/// Pre-admission Warrior HP: joined heroes resolve their own class baseline.
/// A literal because `base_hp` reads the catalog; a test pins the two equal.
pub const DEFAULT_MAX_HP: f32 = 220.0;
pub const MAX_MANA: f32 = 100.0;
pub const MANA_REGEN_PER_SECOND: f32 = 8.0;
/// Homing hero projectiles (basic strikes and skills) travel at this speed.
pub const PROJECTILE_SPEED: f32 = 19.0;
pub const RESPAWN_DELAY_SECS: u64 = 5;
pub const LEVEL_UP_HP_BONUS: f32 = 18.0;
pub const LEVEL_UP_MANA_BONUS: f32 = 12.0;
/// XP needed to leave each level from `STARTING_LEVEL`; the last level has no
/// next threshold. Team-shared kill XP depends on roster size.
pub const LEVEL_XP_THRESHOLDS: [u32; 9] = [90, 150, 180, 220, 260, 300, 340, 380, 420];
/// Distance at which a practice or offline bot notices and engages a hero.
pub const BOT_ENGAGE_RANGE: f32 = 15.0;

/// XP required to advance from `level`; zero at the level cap.
pub fn xp_threshold_for_level(level: u32) -> u32 {
    if level >= MAX_LEVEL {
        0
    } else {
        LEVEL_XP_THRESHOLDS[level.saturating_sub(STARTING_LEVEL) as usize]
    }
}

/// Maximum HP of a hero of `class` at `level` wearing items worth `item_hp`.
pub fn max_hp_for_level(class: HeroClass, level: u32, item_hp: f32) -> f32 {
    base_hp(class)
        + LEVEL_UP_HP_BONUS * level.clamp(STARTING_LEVEL, MAX_LEVEL).saturating_sub(1) as f32
        + item_hp
}

/// Level-one maximum HP of `class` (`base_hp` in the hero catalog).
pub fn base_hp(class: HeroClass) -> f32 {
    catalog::hero(class).base_hp
}
fn growth(level: u32, cap: f32) -> f32 {
    1.0 + (cap - 1.0) * (level.clamp(1, MAX_LEVEL) - 1) as f32 / (MAX_LEVEL - 1) as f32
}
pub fn movement_multiplier(_class: HeroClass, level: u32) -> f32 {
    growth(level, 1.24)
}
/// Per-class caps (`growth` in the hero catalog); the curve itself is uniform.
pub fn basic_damage_multiplier(class: HeroClass, level: u32) -> f32 {
    growth(level, catalog::hero(class).basic_damage_cap)
}
pub fn attack_rate_multiplier(class: HeroClass, level: u32) -> f32 {
    growth(level, catalog::hero(class).attack_rate_cap)
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
    fn xp_thresholds_and_hp_growth_follow_the_level_curve() {
        assert_eq!(
            xp_threshold_for_level(STARTING_LEVEL),
            LEVEL_XP_THRESHOLDS[0]
        );
        assert_eq!(xp_threshold_for_level(MAX_LEVEL), 0);
        assert_eq!(xp_threshold_for_level(MAX_LEVEL + 5), 0);
        assert_eq!(
            LEVEL_XP_THRESHOLDS.len() as u32,
            MAX_LEVEL - STARTING_LEVEL,
            "one threshold per level-up"
        );
        let base = base_hp(HeroClass::Mage);
        assert_eq!(max_hp_for_level(HeroClass::Mage, 1, 0.0), base);
        assert_eq!(max_hp_for_level(HeroClass::Mage, 0, 0.0), base);
        assert_eq!(
            max_hp_for_level(HeroClass::Mage, 7, 30.0),
            base + 6.0 * LEVEL_UP_HP_BONUS + 30.0
        );
        assert_eq!(
            max_hp_for_level(HeroClass::Mage, 99, 0.0),
            max_hp_for_level(HeroClass::Mage, MAX_LEVEL, 0.0)
        );
    }

    #[test]
    fn default_max_hp_is_the_warrior_base_hp() {
        assert_eq!(
            DEFAULT_MAX_HP.to_bits(),
            base_hp(HeroClass::Warrior).to_bits()
        );
    }

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
