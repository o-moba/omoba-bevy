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
            Self::ClassDefault | Self::Unequipped => None,
            Self::Item(id) => Some(id),
        }
    }
}
