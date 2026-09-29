use crate::balance::{LEVEL_UP_HP_BONUS, LEVEL_UP_MANA_BONUS, MAX_LEVEL};
use crate::hero::Hero;

pub fn xp_threshold_for_level(level: u32) -> u32 {
    shared::hero_balance::xp_threshold_for_level(level)
}

pub fn apply_level_up(hero: &mut Hero) {
    let progress = &mut hero.progress;
    progress.level = progress.level.saturating_add(1);
    progress.skill_points = progress.skill_points.saturating_add(1);
    progress.next_level_xp = xp_threshold_for_level(progress.level);
    hero.max_hp += LEVEL_UP_HP_BONUS;
    let mana_growth = if hero
        .skills
        .loadout
        .is_some_and(|l| l.core() == shared::loadout::CoreId::Stormfist)
    {
        0.0
    } else {
        LEVEL_UP_MANA_BONUS
    };
    hero.max_mana += mana_growth;
    // Shared team XP also reaches dead players. Stat growth must not revive
    // them before the authoritative respawn timer teleports them home.
    if hero.hp > 0.0 {
        hero.hp = (hero.hp + LEVEL_UP_HP_BONUS).min(hero.max_hp);
    }
    hero.mana = (hero.mana + mana_growth).clamp(0.0, hero.max_mana);
}

pub fn grant_player_xp(hero: &mut Hero, amount: u32) {
    if amount == 0 {
        return;
    }
    let progress = &mut hero.progress;
    if progress.level >= MAX_LEVEL {
        progress.xp = 0;
        progress.next_level_xp = 0;
        return;
    }

    progress.xp = progress.xp.saturating_add(amount);
    while hero.progress.level < MAX_LEVEL
        && hero.progress.next_level_xp > 0
        && hero.progress.xp >= hero.progress.next_level_xp
    {
        hero.progress.xp -= hero.progress.next_level_xp;
        apply_level_up(hero);
    }

    let progress = &mut hero.progress;
    if progress.level >= MAX_LEVEL {
        progress.xp = 0;
        progress.next_level_xp = 0;
    }
}
