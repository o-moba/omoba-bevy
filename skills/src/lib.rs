//! Data-driven skill parameters shared by the game client and server.
//! All gameplay numbers for tooltips and simulation must be derived from this module.

use std::time::Duration;

/// Number of ability slots in the HUD (Q/W/E/R style).
pub const SLOT_COUNT: usize = 4;

/// Every skill starts at this rank; points raise it up to [`MAX_SKILL_RANK`].
pub const STARTING_RANK: u8 = 1;

/// Maximum rank per skill (inclusive).
pub const MAX_SKILL_RANK: u8 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillMeta {
    pub slot: usize,
    pub name: &'static str,
    pub description: &'static str,
}

/// Static presentation + semantics for one slot index (0..4).
pub fn skill_meta(slot: usize) -> Option<SkillMeta> {
    match slot {
        0 => Some(SkillMeta {
            slot: 0,
            name: "Ranged Shot",
            description: "Fires a homing projectile at the selected enemy.",
        }),
        1 => Some(SkillMeta {
            slot: 1,
            name: "Vitality",
            description: "Passive: restores health over time while alive.",
        }),
        2 => Some(SkillMeta {
            slot: 2,
            name: "Focus",
            description: "Passive: increases mana regeneration.",
        }),
        3 => Some(SkillMeta {
            slot: 3,
            name: "Velocity",
            description: "Passive: increases Ranged Shot projectile speed.",
        }),
        _ => None,
    }
}

fn clamp_rank(rank: u8) -> u8 {
    rank.clamp(STARTING_RANK, MAX_SKILL_RANK)
}

// --- Slot 0: Ranged Shot (active projectile) ---

const SLOT0_BASE_MANA: f32 = 20.0;
const SLOT0_MANA_PER_RANK: f32 = -1.5;
const SLOT0_BASE_COOLDOWN_MS: u64 = 350;
const SLOT0_COOLDOWN_DELTA_MS_PER_RANK: i64 = -25;
const SLOT0_BASE_DAMAGE: f32 = 20.0;
const SLOT0_DAMAGE_PER_RANK: f32 = 4.0;
const SLOT0_BASE_PROJECTILE_SPEED: f32 = 19.0;
const SLOT0_SPEED_PER_RANK: f32 = 0.6;

/// Mana cost for Ranged Shot at the given rank.
pub fn slot0_mana_cost(rank: u8) -> f32 {
    let r = clamp_rank(rank);
    let steps = u32::from(r - STARTING_RANK);
    (SLOT0_BASE_MANA + SLOT0_MANA_PER_RANK * steps as f32).max(6.0)
}

/// Cooldown after casting Ranged Shot.
pub fn slot0_cooldown(rank: u8) -> Duration {
    let r = clamp_rank(rank);
    let steps = i64::from(r - STARTING_RANK);
    let ms = (SLOT0_BASE_COOLDOWN_MS as i64 + SLOT0_COOLDOWN_DELTA_MS_PER_RANK * steps).max(200);
    Duration::from_millis(ms as u64)
}

/// Projectile impact damage for Ranged Shot.
pub fn slot0_damage(rank: u8) -> f32 {
    let r = clamp_rank(rank);
    let steps = u32::from(r - STARTING_RANK);
    SLOT0_BASE_DAMAGE + SLOT0_DAMAGE_PER_RANK * steps as f32
}

/// Base projectile linear speed before Velocity passive.
pub fn slot0_projectile_speed(rank: u8) -> f32 {
    let r = clamp_rank(rank);
    let steps = u32::from(r - STARTING_RANK);
    SLOT0_BASE_PROJECTILE_SPEED + SLOT0_SPEED_PER_RANK * steps as f32
}

// --- Slot 1: Vitality (HP regen / sec) ---

const VITALITY_BASE: f32 = 0.35;
const VITALITY_PER_RANK: f32 = 0.22;

pub fn vitality_hp_per_second(rank: u8) -> f32 {
    let r = clamp_rank(rank);
    let steps = u32::from(r - STARTING_RANK);
    VITALITY_BASE + VITALITY_PER_RANK * steps as f32
}

// --- Slot 2: Focus (additive mana regen / sec) ---

const FOCUS_BASE: f32 = 0.35;
const FOCUS_PER_RANK: f32 = 0.55;

pub fn focus_mana_regen_bonus(rank: u8) -> f32 {
    let r = clamp_rank(rank);
    let steps = u32::from(r - STARTING_RANK);
    FOCUS_BASE + FOCUS_PER_RANK * steps as f32
}

// --- Slot 3: Velocity (multiplier on projectile speed) ---

pub fn velocity_speed_multiplier(rank: u8) -> f32 {
    let r = clamp_rank(rank);
    let steps = u32::from(r - STARTING_RANK);
    1.0 + 0.05 * steps as f32
}

/// Effective projectile speed with both Ranged Shot rank and Velocity rank.
pub fn effective_projectile_speed(ranged_rank: u8, velocity_rank: u8) -> f32 {
    slot0_projectile_speed(ranged_rank) * velocity_speed_multiplier(velocity_rank)
}

/// Primary "value" line for tooltip: combat-relevant number per slot.
pub fn primary_value_for_slot(slot: usize, ranks: &[u8; SLOT_COUNT]) -> f32 {
    match slot {
        0 => slot0_damage(ranks[0]),
        1 => vitality_hp_per_second(ranks[1]),
        2 => focus_mana_regen_bonus(ranks[2]),
        3 => velocity_speed_multiplier(ranks[3]),
        _ => 0.0,
    }
}

/// Next rank primary value, or `None` if already at max.
pub fn next_rank_primary_value(slot: usize, ranks: &[u8; SLOT_COUNT]) -> Option<f32> {
    let r = ranks.get(slot).copied()?;
    if r >= MAX_SKILL_RANK {
        return None;
    }
    let mut next = *ranks;
    next[slot] = r + 1;
    Some(primary_value_for_slot(slot, &next))
}

/// Whether the player can spend a point on this slot (server-side rules mirrored for UI).
pub fn can_upgrade_slot(ranks: &[u8; SLOT_COUNT], slot: usize, skill_points: u32) -> bool {
    skill_points > 0 && slot < SLOT_COUNT && ranks[slot] < MAX_SKILL_RANK
}

/// Cooldown label for UI: active slot 0 uses real cooldown; passives show em dash.
pub fn cooldown_label_ms(slot: usize, ranged_rank: u8) -> Option<u64> {
    if slot == 0 {
        Some(slot0_cooldown(ranged_rank).as_millis() as u64)
    } else {
        None
    }
}

/// Full tooltip line for the primary stat at the player's current rank (same semantics as simulation).
pub fn primary_value_tooltip_current(slot: usize, value: f32) -> String {
    match slot {
        0 => format!("{value:.0} projectile damage"),
        1 => format!("{value:.2} HP restored per second"),
        2 => format!("{value:.2} bonus mana regen per second"),
        3 => format!("×{value:.2} projectile speed multiplier"),
        _ => format!("{value:.2}"),
    }
}

/// Compact primary-stat fragment for the "next rank" preview line in tooltips.
pub fn primary_value_tooltip_next_rank(slot: usize, value: f32) -> String {
    match slot {
        0 => format!("{value:.0} damage"),
        1 => format!("{value:.2} HP/s"),
        2 => format!("{value:.2} mana/s"),
        3 => format!("×{value:.2} speed"),
        _ => format!("{value:.2}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upgrade_only_when_points_and_below_cap() {
        let mut ranks = [1_u8; SLOT_COUNT];
        assert!(can_upgrade_slot(&ranks, 0, 1));
        ranks[0] = MAX_SKILL_RANK;
        assert!(!can_upgrade_slot(&ranks, 0, 1));
        assert!(!can_upgrade_slot(&ranks, 0, 0));
    }

    #[test]
    fn slot0_scales_with_rank() {
        let d1 = slot0_damage(STARTING_RANK);
        let d5 = slot0_damage(MAX_SKILL_RANK);
        assert!(d5 > d1);
        assert!(slot0_mana_cost(MAX_SKILL_RANK) < slot0_mana_cost(STARTING_RANK));
        assert!(slot0_cooldown(MAX_SKILL_RANK) < slot0_cooldown(STARTING_RANK));
    }

    #[test]
    fn next_rank_preview_is_some_until_max() {
        let ranks = [1, 2, 3, 4];
        assert!(next_rank_primary_value(0, &ranks).is_some());
        let maxed = [MAX_SKILL_RANK; SLOT_COUNT];
        assert!(next_rank_primary_value(0, &maxed).is_none());
    }

    #[test]
    fn next_rank_primary_matches_incremented_rank_vector() {
        let ranks = [2_u8, 3, 1, 4];
        for slot in 0..SLOT_COUNT {
            if ranks[slot] >= MAX_SKILL_RANK {
                continue;
            }
            let preview = next_rank_primary_value(slot, &ranks).expect("below max");
            let mut bumped = ranks;
            bumped[slot] = ranks[slot] + 1;
            let direct = primary_value_for_slot(slot, &bumped);
            assert!(
                (preview - direct).abs() < 1e-4,
                "slot {slot}: preview {preview} vs direct {direct}"
            );
        }
    }

    #[test]
    fn tooltip_current_line_matches_slot0_damage_text() {
        let ranks = [3_u8, 1, 1, 1];
        let v = primary_value_for_slot(0, &ranks);
        assert_eq!(v, slot0_damage(ranks[0]));
        let line = primary_value_tooltip_current(0, v);
        assert_eq!(
            line,
            format!("{:.0} projectile damage", slot0_damage(ranks[0]))
        );
    }
}
