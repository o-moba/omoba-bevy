use super::*;

pub(crate) fn xp_threshold_for_level(level: u32) -> u32 {
    if level >= MAX_LEVEL {
        0
    } else {
        let index = level.saturating_sub(STARTING_LEVEL) as usize;
        LEVEL_XP_THRESHOLDS[index]
    }
}

pub(crate) fn apply_level_up(state: &mut PlayerState) {
    state.level = state.level.saturating_add(1);
    state.skill_points = state.skill_points.saturating_add(1);
    state.max_hp += LEVEL_UP_HP_BONUS;
    state.max_mana += LEVEL_UP_MANA_BONUS;
    state.hp = (state.hp + LEVEL_UP_HP_BONUS).clamp(0.0, state.max_hp);
    state.mana = (state.mana + LEVEL_UP_MANA_BONUS).clamp(0.0, state.max_mana);
    state.next_level_xp = xp_threshold_for_level(state.level);
}

pub(crate) fn grant_player_xp(state: &mut PlayerState, amount: u32) {
    if amount == 0 {
        return;
    }
    if state.level >= MAX_LEVEL {
        state.xp = 0;
        state.next_level_xp = 0;
        return;
    }

    state.xp = state.xp.saturating_add(amount);
    while state.level < MAX_LEVEL && state.next_level_xp > 0 && state.xp >= state.next_level_xp {
        state.xp -= state.next_level_xp;
        apply_level_up(state);
    }

    if state.level >= MAX_LEVEL {
        state.xp = 0;
        state.next_level_xp = 0;
    }
}
