//! Pure progression policies for heroes that level without a player at the
//! keyboard: server bots (lane bots and the practice duelist) and the offline
//! duel. Hosts apply the result through their own upgrade path.

use crate::{HeroClass, SkillSlot, unlocked_slots_for_level};

/// The order a bot spends skill points in: the ultimate first once it
/// unlocks, then Q, W and E.
const RANK_PRIORITY: [SkillSlot; 4] = [SkillSlot::R, SkillSlot::Q, SkillSlot::W, SkillSlot::E];

/// Spend `points` the way a player would: the ultimate first once it
/// unlocks, then Q, W and E, never into a locked slot or past the ability's
/// maximum rank. Returns one slot index (`SkillSlot::index`) per upgrade, in
/// order; applying them to `ranks` one at a time is always a legal upgrade.
/// Points that have nowhere to go are left unspent.
pub fn skill_upgrade_order(class: HeroClass, level: u32, ranks: [u8; 4], points: u32) -> Vec<u8> {
    let unlocked = unlocked_slots_for_level(level);
    let mut ranks = ranks;
    let mut points = points;
    let mut order = Vec::new();
    for slot in RANK_PRIORITY {
        let index = slot.index();
        if !unlocked[index] {
            continue;
        }
        let max_rank = class.ability(slot).max_rank;
        while points > 0 && ranks[index] < max_rank {
            ranks[index] += 1;
            points -= 1;
            order.push(index as u8);
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAX_ABILITY_RANK;

    fn ranks_after(class: HeroClass, level: u32, points: u32) -> [u8; 4] {
        let mut ranks = [1; 4];
        for slot in skill_upgrade_order(class, level, ranks, points) {
            ranks[slot as usize] += 1;
        }
        ranks
    }

    #[test]
    fn ultimate_first_then_q_w_e_and_never_a_locked_slot() {
        // Level 7 with its six points: R and Q maxed, the rest into W.
        assert_eq!(
            skill_upgrade_order(HeroClass::Mage, 7, [1; 4], 6),
            vec![3, 3, 0, 0, 1, 1]
        );
        for class in HeroClass::ALL {
            assert_eq!(ranks_after(class, 7, 6), [3, 3, 1, 3], "{class:?}");
            // Before level 6 the ultimate is locked, before 4 the E.
            assert_eq!(ranks_after(class, 5, 4), [3, 3, 1, 1], "{class:?}");
            assert_eq!(ranks_after(class, 3, 2), [3, 1, 1, 1], "{class:?}");
        }
        // Current ranks count: a maxed slot takes nothing more.
        assert_eq!(
            skill_upgrade_order(HeroClass::Warrior, 10, [3, 1, 2, 3], 2),
            vec![1, 1]
        );
    }

    #[test]
    fn spare_points_stay_unspent_and_no_points_plan_nothing() {
        let full = [MAX_ABILITY_RANK; 4];
        assert!(skill_upgrade_order(HeroClass::Cleric, 10, full, 5).is_empty());
        assert!(skill_upgrade_order(HeroClass::Cleric, 10, [1; 4], 0).is_empty());
        // Level one only has Q, which takes two points.
        assert_eq!(
            skill_upgrade_order(HeroClass::Warden, 1, [1; 4], 5),
            vec![0, 0]
        );
    }
}
