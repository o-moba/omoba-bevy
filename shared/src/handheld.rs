//! Cosmetic equipment is independent of avatar, class and combat inventory.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "id", rename_all = "snake_case")]
pub enum HandheldSelection {
    #[default]
    ClassDefault,
    Unequipped,
    Item(String),
}
impl HandheldSelection {
    pub fn is_default(&self) -> bool {
        *self == Self::ClassDefault
    }
    pub fn resolve(&self, class: crate::HeroClass) -> Option<&str> {
        match self {
            Self::ClassDefault if class == crate::HeroClass::Warrior => Some("forge-sword"),
            Self::ClassDefault if class == crate::HeroClass::Adventurer => Some("dagger"),
            Self::ClassDefault if class == crate::HeroClass::Wildspark => Some("wild-repeater"),
            Self::ClassDefault if class == crate::HeroClass::Ranger => Some("verdant-bow"),
            Self::ClassDefault if class == crate::HeroClass::Riftshot => Some("wild-repeater"),
            Self::ClassDefault if class == crate::HeroClass::Dawnweaver => Some("dawn-scepter"),
            Self::ClassDefault | Self::Unequipped => None,
            Self::Item(id) => Some(id),
        }
    }

    /// Only the class default follows the equipped gun mode. An explicitly
    /// selected SDK or packaged skin remains the user's choice in either mode.
    pub fn resolve_mode(
        &self,
        class: crate::HeroClass,
        mode: crate::loadout::WeaponMode,
    ) -> Option<&str> {
        if *self == Self::ClassDefault
            && class == crate::HeroClass::Wildspark
            && mode == crate::loadout::WeaponMode::Rockets
        {
            Some("wild-launcher")
        } else {
            self.resolve(class)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HeroClass;

    #[test]
    fn adventurer_defaults_to_dagger_and_any_class_can_equip_it() {
        assert_eq!(
            HandheldSelection::ClassDefault.resolve(HeroClass::Adventurer),
            Some("dagger")
        );
        let equipped = HandheldSelection::Item("dagger".into());
        for class in HeroClass::ALL {
            assert_eq!(equipped.resolve(class), Some("dagger"));
            assert_eq!(HandheldSelection::Unequipped.resolve(class), None);
        }
    }
}

#[cfg(test)]
mod ranged_tests {
    use super::*;
    use crate::{HeroClass, loadout::WeaponMode};
    #[test]
    fn ranged_defaults_change_gun_mode_but_preserve_explicit_sdk_skins_and_unequip() {
        assert_eq!(
            HandheldSelection::ClassDefault.resolve(HeroClass::Ranger),
            Some("verdant-bow")
        );
        assert_eq!(
            HandheldSelection::ClassDefault
                .resolve_mode(HeroClass::Wildspark, WeaponMode::Repeater),
            Some("wild-repeater")
        );
        assert_eq!(
            HandheldSelection::ClassDefault.resolve_mode(HeroClass::Wildspark, WeaponMode::Rockets),
            Some("wild-launcher")
        );
        for mode in [WeaponMode::Repeater, WeaponMode::Rockets] {
            assert_eq!(
                HandheldSelection::Unequipped.resolve_mode(HeroClass::Wildspark, mode),
                None
            );
            assert_eq!(
                HandheldSelection::Item("sdk-skin".into()).resolve_mode(HeroClass::Wildspark, mode),
                Some("sdk-skin")
            );
        }
    }
}
