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
            Self::ClassDefault | Self::Unequipped => None,
            Self::Item(id) => Some(id),
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
