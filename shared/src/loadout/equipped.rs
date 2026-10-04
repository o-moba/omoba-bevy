//! Read-only skill metadata for an accepted kit. Physical buttons, authored
//! progression roles and core growth are independent concepts.
use super::{BuildRecipe, LoadoutError, ResolvedLoadout, SkillDefinition};
use crate::{AbilityDefinition, HeroClass, SkillSlot, hero_balance, shop::ItemBonuses};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EquippedSkills {
    class: HeroClass,
    loadout: Option<ResolvedLoadout>,
}

impl EquippedSkills {
    /// Resolve untrusted/snapshot data. Only absence means a preset/legacy kit;
    /// an invalid supplied recipe never silently becomes a different skill.
    pub fn resolve(class: HeroClass, recipe: Option<&BuildRecipe>) -> Result<Self, LoadoutError> {
        let loadout = recipe.map(super::resolve).transpose()?;
        if let Some(loadout) = loadout
            && loadout.core().class() != class
        {
            return Err(LoadoutError::CoreMismatch {
                class,
                core: loadout.core(),
            });
        }
        Ok(Self::from_resolved(class, loadout))
    }

    /// Trusted runtime state. Missing modular data uses the class preset;
    /// legacy classes retain their original class-owned Q/W/E/R definitions.
    pub fn from_resolved(class: HeroClass, loadout: Option<ResolvedLoadout>) -> Self {
        Self {
            class,
            loadout: loadout.or_else(|| super::preset_for_class(class)),
        }
    }

    pub const fn resolved(self) -> Option<ResolvedLoadout> {
        self.loadout
    }

    pub fn skill(self, slot: SkillSlot) -> Option<&'static SkillDefinition> {
        self.loadout.map(|loadout| loadout.skill(slot))
    }

    pub fn ability(self, slot: SkillSlot) -> &'static AbilityDefinition {
        self.skill(slot)
            .map(|skill| &skill.ability)
            .unwrap_or_else(|| crate::ability_for_class_slot(self.class, slot))
    }

    /// Display and admission share the equipped skill's resource policy.
    pub fn mana_cost(self, rank: u8, slot: SkillSlot, recast: bool) -> f32 {
        self.skill(slot).map_or_else(
            || {
                if recast {
                    0.0
                } else {
                    let ability = self.ability(slot);
                    crate::scaled_mana_cost(ability, rank.clamp(1, ability.max_rank))
                }
            },
            |skill| skill.mana_cost(rank, recast),
        )
    }

    /// Moving an ultimate to Q never grants level-one access to that ultimate.
    pub fn unlock_level(self, slot: SkillSlot) -> u32 {
        self.loadout
            .map_or(crate::SLOT_UNLOCK_LEVELS[slot.index()], |loadout| {
                loadout.unlock_level(slot)
            })
    }

    pub fn unlocked(self, level: u32) -> [bool; 4] {
        SkillSlot::ALL.map(|slot| level.max(1) >= self.unlock_level(slot))
    }

    /// Same duration on authority and client. Modular abilities use spell haste
    /// regardless of binding; the legacy basic-Q attack-speed rule is preserved.
    pub fn cooldown(self, level: u32, rank: u8, slot: SkillSlot, bonuses: ItemBonuses) -> Duration {
        if let Some(skill) = self.skill(slot) {
            let rate = hero_balance::spell_haste_multiplier(self.class, level)
                * bonuses.spell_haste_multiplier.max(1.0);
            crate::scaled_cooldown(&skill.ability, rank.clamp(1, skill.ability.max_rank))
                .div_f32(rate)
        } else {
            hero_balance::ability_cooldown(self.class, level, rank, slot, bonuses)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loadout::{CoreId, SkillId, resolve};

    #[test]
    fn presets_keep_every_definition_unlock_and_cooldown() {
        for class in HeroClass::ALL {
            let equipped = EquippedSkills::resolve(class, None).unwrap();
            for slot in SkillSlot::ALL {
                assert_eq!(
                    equipped.ability(slot).id,
                    crate::ability_for_class_slot(class, slot).id
                );
                assert_eq!(
                    equipped.unlock_level(slot),
                    crate::SLOT_UNLOCK_LEVELS[slot.index()]
                );
                for level in [1, 3, 6, 10] {
                    for rank in [1, 2, 3] {
                        let bonuses = ItemBonuses {
                            spell_haste_multiplier: 1.7,
                            attack_speed_multiplier: 2.0,
                            ..ItemBonuses::NONE
                        };
                        assert_eq!(
                            equipped.cooldown(level, rank, slot, bonuses),
                            hero_balance::ability_cooldown(class, level, rank, slot, bonuses)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn reordered_skills_keep_identity_progression_and_spell_cooldown() {
        let mut recipe = CoreId::Dawnweaver.preset();
        recipe.skills = [
            SkillId::DawnRay,
            SkillId::DaggerDeadlyBlow,
            SkillId::DawnBarrier,
            SkillId::DawnField,
        ];
        let equipped = EquippedSkills::resolve(HeroClass::Dawnweaver, Some(&recipe)).unwrap();
        assert_eq!(equipped.ability(SkillSlot::Q).id, "dawn_ray");
        assert_eq!(equipped.ability(SkillSlot::W).id, "dagger_deadly_blow");
        assert_eq!(equipped.ability(SkillSlot::W).base_mana_cost, 12.0);
        assert_eq!(equipped.ability(SkillSlot::W).cast_range, 2.6);
        assert_eq!(equipped.unlocked(1), [false, true, false, false]);
        assert_eq!(equipped.unlocked(2), [false, true, true, false]);
        assert_eq!(equipped.unlocked(4), [false, true, true, true]);
        assert_eq!(equipped.unlocked(6), [true; 4]);
        let baseline = EquippedSkills::resolve(HeroClass::Dawnweaver, None).unwrap();
        assert_eq!(
            equipped.cooldown(6, 2, SkillSlot::Q, ItemBonuses::NONE),
            baseline.cooldown(6, 2, SkillSlot::R, ItemBonuses::NONE)
        );
    }

    #[test]
    fn four_distinct_ultimates_are_valid_but_not_unlocked_early() {
        let mut recipe = CoreId::Dawnweaver.preset();
        recipe.skills = [
            SkillId::DawnRay,
            SkillId::WildRocket,
            SkillId::FourfoldDuel,
            SkillId::Nightfall,
        ];
        let equipped = EquippedSkills::resolve(HeroClass::Dawnweaver, Some(&recipe)).unwrap();
        assert_eq!(equipped.unlocked(5), [false; 4]);
        assert_eq!(equipped.unlocked(6), [true; 4]);
        assert_eq!(equipped.resolved().unwrap().recipe(), recipe);
    }

    #[test]
    fn capability_and_unique_identity_constraints_survive_permutation() {
        let mut recipe = CoreId::Dawnweaver.preset();
        recipe.skills.swap(0, 3);
        assert!(resolve(&recipe).is_ok());
        recipe.skills[1] = SkillId::WildSwitch;
        assert!(matches!(
            resolve(&recipe),
            Err(LoadoutError::RequiresRepeater { .. })
        ));
        recipe.skills[1] = SkillId::OrbitalCollapse;
        assert!(matches!(
            resolve(&recipe),
            Err(LoadoutError::RequiresOrbController { .. })
        ));
        recipe.skills[2] = SkillId::OrbitalGuard;
        assert!(resolve(&recipe).is_ok());
        recipe.skills[3] = recipe.skills[0];
        assert!(matches!(
            resolve(&recipe),
            Err(LoadoutError::DuplicateSkill { .. })
        ));
    }

    #[test]
    fn invalid_or_mismatched_recipe_never_falls_back_to_preset() {
        let mut recipe = CoreId::Dawnweaver.preset();
        assert!(matches!(
            EquippedSkills::resolve(HeroClass::Wildspark, Some(&recipe)),
            Err(LoadoutError::CoreMismatch { .. })
        ));
        recipe.catalog_revision = "unrecognized".into();
        assert_eq!(
            EquippedSkills::resolve(HeroClass::Dawnweaver, Some(&recipe)),
            Err(LoadoutError::CatalogRevision)
        );
    }
    #[test]
    fn recast_cost_follows_skill_across_cores_and_bindings() {
        let mut recipe = CoreId::Dawnweaver.preset();
        recipe.skills[3] = SkillId::EchoStrike;
        let equipped = EquippedSkills::resolve(HeroClass::Dawnweaver, Some(&recipe)).unwrap();
        assert_eq!(equipped.mana_cost(1, SkillSlot::R, true), 25.0);
        assert_eq!(
            equipped.mana_cost(1, SkillSlot::R, false),
            super::super::skill(SkillId::EchoStrike)
                .ability
                .base_mana_cost
        );
        let mut recipe = CoreId::Stormfist.preset();
        recipe.skills[0] = SkillId::DawnField;
        let equipped = EquippedSkills::resolve(HeroClass::Stormfist, Some(&recipe)).unwrap();
        assert_eq!(equipped.mana_cost(1, SkillSlot::Q, true), 0.0);
        for id in [
            SkillId::EchoStrike,
            SkillId::AnchorStep,
            SkillId::ThunderPulse,
        ] {
            assert_eq!(super::super::skill(id).mana_cost(3, true), 25.0);
        }
    }
}
