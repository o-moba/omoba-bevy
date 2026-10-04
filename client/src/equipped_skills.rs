//! The client metadata boundary for the accepted loadout. Missing recipes use
//! shared preset/legacy rules; malformed recipes disable skill interaction.
use crate::net::{PlayerLoadout, PlayerProgression};
use shared::{HeroClass, SkillSlot, loadout::EquippedSkills};

pub(crate) fn resolve(class: HeroClass, loadout: Option<&PlayerLoadout>) -> Option<EquippedSkills> {
    resolve_state(class, loadout.and_then(|state| state.0.as_ref()))
}

pub(crate) fn resolve_state(
    class: HeroClass,
    state: Option<&shared::loadout::LoadoutState>,
) -> Option<EquippedSkills> {
    EquippedSkills::resolve(class, state.and_then(|state| state.recipe.as_ref())).ok()
}

/// An explicit sandbox snapshot is authoritative even below authored unlock levels.
pub(crate) fn unlocked(skills: &EquippedSkills, progression: &PlayerProgression) -> [bool; 4] {
    progression
        .sandbox_unlocked
        .unwrap_or_else(|| skills.unlocked(progression.level))
}

pub(crate) fn upgrade_eligible(
    skills: &EquippedSkills,
    progression: &PlayerProgression,
    slot: usize,
) -> bool {
    slot < 4
        && progression.skill_points > 0
        && unlocked(skills, progression)[slot]
        && progression.ranks[slot] < skills.ability(SkillSlot::ALL[slot]).max_rank
}

pub(crate) fn cooldown(
    skills: &EquippedSkills,
    level: u32,
    rank: u8,
    slot: SkillSlot,
    mut bonuses: shared::shop::ItemBonuses,
    sandbox: bool,
) -> f32 {
    if !sandbox {
        bonuses.attack_speed_multiplier = bonuses.attack_speed_multiplier.max(1.0);
        bonuses.spell_haste_multiplier = bonuses.spell_haste_multiplier.max(1.0);
    }
    skills.cooldown(level, rank, slot, bonuses).as_secs_f32()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::loadout::{CoreId, LoadoutState, SkillId};

    #[test]
    fn mixed_permuted_metadata_and_upgrade_follow_skill_identity() {
        let mut recipe = CoreId::Dawnweaver.preset();
        recipe.skills = [
            SkillId::DawnRay,
            SkillId::WildZap,
            SkillId::DawnField,
            SkillId::DawnBind,
        ];
        let state = PlayerLoadout(Some(LoadoutState {
            recipe: Some(recipe),
            ..Default::default()
        }));
        let skills = resolve(HeroClass::Dawnweaver, Some(&state)).unwrap();
        assert_eq!(skills.ability(SkillSlot::W).id, "wild_zap");
        assert_eq!(skills.unlock_level(SkillSlot::Q), 6);
        assert_eq!(skills.unlock_level(SkillSlot::R), 1);
        let prog = PlayerProgression {
            level: 1,
            skill_points: 1,
            ranks: [0; 4],
            ..Default::default()
        };
        assert!(!upgrade_eligible(&skills, &prog, 0));
        assert!(upgrade_eligible(&skills, &prog, 3));
        let card = crate::combat::skill_card::SkillCardView::of_equipped(
            &skills, &prog, 0, 100.0, 8.0, false,
        );
        assert!(card.locked);
        assert_eq!(card.skills.unwrap().ability(SkillSlot::Q).id, "dawn_ray");
    }

    #[test]
    fn sandbox_unlock_snapshot_overrides_role_and_rank_cap_still_applies() {
        let skills = resolve(HeroClass::Dawnweaver, None).unwrap();
        let mut progression = PlayerProgression {
            level: 1,
            skill_points: 1,
            sandbox_unlocked: Some([true; 4]),
            ..Default::default()
        };
        assert!(upgrade_eligible(&skills, &progression, 3));
        let card = crate::combat::skill_card::SkillCardView::of_equipped(
            &skills,
            &progression,
            3,
            100.0,
            8.0,
            false,
        );
        assert!(!card.locked);
        progression.ranks[3] = skills.ability(SkillSlot::R).max_rank;
        assert!(!upgrade_eligible(&skills, &progression, 3));
        progression.sandbox_unlocked = Some([false; 4]);
        assert!(!unlocked(&skills, &progression)[0]);
    }

    #[test]
    fn absent_recipe_uses_preset_but_malformed_recipe_fails_closed() {
        assert_eq!(
            resolve(HeroClass::Dawnweaver, None)
                .unwrap()
                .ability(SkillSlot::Q)
                .id,
            "dawn_bind"
        );
        assert!(
            resolve(HeroClass::Warrior, None)
                .unwrap()
                .resolved()
                .is_none()
        );
        let mut recipe = CoreId::Dawnweaver.preset();
        recipe.skills[1] = recipe.skills[0];
        let state = PlayerLoadout(Some(LoadoutState {
            recipe: Some(recipe),
            ..Default::default()
        }));
        assert!(resolve(HeroClass::Dawnweaver, Some(&state)).is_none());
    }
}
