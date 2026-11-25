//! Compatibility for handheld cosmetics. The class kit owns combat behavior.
use crate::{HeroClass, loadout::WeaponMode};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WeaponKind {
    Sword,
    Hammer,
    Scepter,
    Dagger,
    Bow,
    Repeater,
    Launcher,
    WildsparkSet,
}

impl WeaponKind {
    const fn supports(self, class: HeroClass) -> bool {
        use HeroClass::*;
        match self {
            Self::Sword => matches!(class, Warrior),
            Self::Hammer => matches!(class, Warrior | Cinderforge),
            Self::Scepter => matches!(class, Dawnweaver | Mage | Cleric),
            Self::Dagger => matches!(class, Adventurer),
            Self::Bow => matches!(class, Ranger),
            Self::Repeater => matches!(class, Wildspark | Riftshot),
            Self::Launcher | Self::WildsparkSet => matches!(class, Wildspark),
        }
    }

    fn supports_mode(self, class: HeroClass, mode: WeaponMode) -> bool {
        self.supports(class)
            && (self == Self::WildsparkSet
                || class != HeroClass::Wildspark
                || self
                    == match mode {
                        WeaponMode::Repeater => Self::Repeater,
                        WeaponMode::Rockets => Self::Launcher,
                    })
    }
}

/// Game-owner approval of one concrete item for specific classes. A compatible
/// weapon kind alone never grants permission to equip a model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeaponBinding {
    kind: WeaponKind,
    approved_for: &'static [HeroClass],
}

impl WeaponBinding {
    pub fn supports(self, class: HeroClass) -> bool {
        self.approved_for.contains(&class) && self.kind.supports(class)
    }

    fn supports_mode(self, class: HeroClass, mode: WeaponMode) -> bool {
        self.approved_for.contains(&class) && self.kind.supports_mode(class, mode)
    }
}

/// The October Forge Hammer pilot has a reviewed copy of the packaged GLB.
/// This slug pins both the Registry asset ID and the rendition SHA-256.
/// Registry approval is still required by the online admission gate.
pub const REVIEWED_FORGE_HAMMER: &str = "ekza-64a577b6a472c3bacbf809a045c6327d";

/// Exact Registry renditions reviewed for `wildspark.handheld.set` in the dev
/// catalogue, in order: Astral Ivory, Scrap Punk, Candy Neon.
pub const REVIEWED_WILDSPARK_SETS: [&str; 3] = [
    "ekza-1cbf454f0ddf490770f03d99f38cbaab",
    "ekza-b2a0d03156f8544bc14fcef67b8e9ded",
    "ekza-74960bee008e4de34f86c5f038a51824",
];

/// Explicit approvals for packaged models and the reviewed legacy rendition.
pub fn weapon_binding(id: &str) -> Option<WeaponBinding> {
    use HeroClass::*;
    use WeaponKind::*;

    let (kind, approved_for): (_, &'static [HeroClass]) = match id {
        "forge-sword" => (Sword, &[Warrior]),
        "forge-hammer" | REVIEWED_FORGE_HAMMER => (Hammer, &[Warrior, Cinderforge]),
        "dawn-scepter" => (Scepter, &[Dawnweaver, Mage, Cleric]),
        "dagger" => (Dagger, &[Adventurer]),
        "verdant-bow" => (Bow, &[Ranger]),
        "wild-repeater" => (Repeater, &[Riftshot]),
        "wildspark-repeater" => (Repeater, &[Wildspark]),
        "wild-launcher" => (Launcher, &[Wildspark]),
        id if REVIEWED_WILDSPARK_SETS.contains(&id) => (WildsparkSet, &[Wildspark]),
        _ => return None,
    };
    Some(WeaponBinding { kind, approved_for })
}

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

    /// Only Combat Test may explicitly hide a class's usual handheld.
    pub fn allowed_in_match(&self) -> bool {
        !matches!(self, Self::Unequipped)
    }

    pub fn compatible(&self, class: HeroClass) -> bool {
        match self {
            Self::ClassDefault | Self::Unequipped => true,
            Self::Item(id) => weapon_binding(id).is_some_and(|binding| binding.supports(class)),
        }
    }

    pub fn resolve(&self, class: HeroClass) -> Option<&str> {
        self.resolve_mode(class, WeaponMode::Repeater)
    }

    pub fn resolve_mode(&self, class: HeroClass, mode: WeaponMode) -> Option<&str> {
        match self {
            Self::Unequipped => None,
            Self::Item(id)
                if weapon_binding(id).is_some_and(|binding| binding.supports_mode(class, mode)) =>
            {
                Some(id)
            }
            Self::ClassDefault | Self::Item(_) => class_default(class, mode),
        }
    }
}

fn class_default(class: HeroClass, mode: WeaponMode) -> Option<&'static str> {
    use HeroClass::*;
    match (class, mode) {
        (Warrior, _) => Some("forge-sword"),
        (Adventurer, _) => Some("dagger"),
        (Wildspark, WeaponMode::Rockets) => Some("wild-launcher"),
        (Wildspark, _) => Some("wildspark-repeater"),
        (Riftshot, _) => Some("wild-repeater"),
        (Ranger, _) => Some("verdant-bow"),
        (Dawnweaver, _) => Some("dawn-scepter"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_items_require_a_compatible_class() {
        let sword = HandheldSelection::Item("forge-sword".into());
        assert!(sword.compatible(HeroClass::Warrior));
        assert!(!sword.compatible(HeroClass::Wildspark));
        assert!(!sword.compatible(HeroClass::Mage));
        let hammer = HandheldSelection::Item("forge-hammer".into());
        assert!(hammer.compatible(HeroClass::Warrior));
        assert!(hammer.compatible(HeroClass::Cinderforge));
        assert!(!HandheldSelection::Item("verdant-bow".into()).compatible(HeroClass::Warrior));
        assert!(!HandheldSelection::Item("unclassified".into()).compatible(HeroClass::Warrior));
        assert_eq!(
            sword.resolve(HeroClass::Wildspark),
            Some("wildspark-repeater")
        );
    }

    #[test]
    fn every_class_default_has_a_compatible_kind() {
        for class in HeroClass::ALL {
            for mode in [WeaponMode::Repeater, WeaponMode::Rockets] {
                if let Some(id) = HandheldSelection::ClassDefault.resolve_mode(class, mode) {
                    assert!(
                        weapon_binding(id).unwrap().supports_mode(class, mode),
                        "{class:?}"
                    );
                }
                assert_eq!(
                    HandheldSelection::Unequipped.resolve_mode(class, mode),
                    None
                );
            }
        }
    }

    #[test]
    fn wildspark_single_item_only_replaces_its_own_mode() {
        let repeater = HandheldSelection::Item("wildspark-repeater".into());
        let launcher = HandheldSelection::Item("wild-launcher".into());
        for selection in [HandheldSelection::ClassDefault, repeater, launcher] {
            assert!(selection.compatible(HeroClass::Wildspark));
            assert_eq!(
                selection.resolve_mode(HeroClass::Wildspark, WeaponMode::Repeater),
                Some("wildspark-repeater")
            );
            assert_eq!(
                selection.resolve_mode(HeroClass::Wildspark, WeaponMode::Rockets),
                Some("wild-launcher")
            );
        }
        assert!(!HandheldSelection::Item("wild-launcher".into()).compatible(HeroClass::Riftshot));
    }

    #[test]
    fn reviewed_sets_are_wildspark_only_and_cover_both_modes() {
        for id in REVIEWED_WILDSPARK_SETS {
            let selection = HandheldSelection::Item(id.into());
            for class in HeroClass::ALL {
                assert_eq!(selection.compatible(class), class == HeroClass::Wildspark);
            }
            for mode in [WeaponMode::Repeater, WeaponMode::Rockets] {
                assert_eq!(selection.resolve_mode(HeroClass::Wildspark, mode), Some(id));
            }
        }
    }

    #[test]
    fn a_repeater_approved_for_riftshot_has_no_wildspark_approval() {
        let riftshot = HandheldSelection::Item("wild-repeater".into());
        let wildspark = HandheldSelection::Item("wildspark-repeater".into());
        assert!(WeaponKind::Repeater.supports(HeroClass::Wildspark));
        assert!(riftshot.compatible(HeroClass::Riftshot));
        assert!(!riftshot.compatible(HeroClass::Wildspark));
        assert!(!wildspark.compatible(HeroClass::Riftshot));
    }
}
