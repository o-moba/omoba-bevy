//! Display text of game data, looked up by stable id.
//!
//! The shared catalogs (`shared/assets/catalog/*.json`, `reactions.json`)
//! and the wire enums keep their English text: the server and the catalog
//! validation use them, and the catalog files deny unknown fields. The
//! client shows them through these helpers instead, keyed by id, with the
//! catalog/enum English as the `en` value (tests pin the equality) and as
//! the fallback when a dictionary lacks a runtime-built key.
//!
//! | Data | Key |
//! | --- | --- |
//! | hero class | `hero.<class id>.name`, `hero.<class id>.tagline` |
//! | ability | `ability.<ability id>.name`, `ability.<ability id>.desc` |
//! | item | `item.<item id>.name`, `.desc`, `.short` |
//! | draft role | `role.<role>` |
//! | supporter aura style | `aura.<style id>` |
//! | reaction / reaction pack | `reaction.<id>`, `reaction.pack.<pack id>` |
//! | boss | `boss.<boss id>` |
//! | lane label | `lane.top`, `lane.mid`, `lane.bot`, `lane.base` |
//! | profile card title | `title.<slug>` (by index into `frontend::card::TITLES`) |
//! | join rejection / purchase error | `error.join.<code>`, `error.purchase.<code>` |
//! | Studio catalogue status | `collection.catalogue.<state>` |
use std::fmt::Write as _;

use shared::map::Lane;
use shared::prematch::Role;
use shared::protocol::JoinRejection;
use shared::shop::{ItemId, PurchaseError};
use shared::supporter::AuraStyle;
use shared::wire::NeutralCampType;
use shared::{AbilityDefinition, HeroClass};

use super::{lookup, tr};

/// Looks up `prefix + id + suffix` without allocating for ids up to a few
/// dozen bytes (the key is assembled on the stack).
fn lookup_composed(prefix: &str, id: &str, suffix: &str) -> Option<&'static str> {
    const CAPACITY: usize = 96;
    let length = prefix.len() + id.len() + suffix.len();
    if length > CAPACITY {
        let mut key = String::with_capacity(length);
        let _ = write!(key, "{prefix}{id}{suffix}");
        return lookup(&key);
    }
    let mut buffer = [0u8; CAPACITY];
    let mut end = 0;
    for part in [prefix, id, suffix] {
        buffer[end..end + part.len()].copy_from_slice(part.as_bytes());
        end += part.len();
    }
    // The parts are `&str`, so their concatenation is valid UTF-8.
    std::str::from_utf8(&buffer[..end]).ok().and_then(lookup)
}

/// A class's display name (`Warrior`).
pub(crate) fn hero_name(class: HeroClass) -> &'static str {
    lookup_composed("hero.", class.id(), ".name").unwrap_or_else(|| class.display_name())
}

/// A class's one-line pitch.
pub(crate) fn hero_tagline(class: HeroClass) -> &'static str {
    lookup_composed("hero.", class.id(), ".tagline").unwrap_or_else(|| class.tagline())
}

/// An ability's name (`Shield Bash`).
pub(crate) fn ability_name(ability: &AbilityDefinition) -> &'static str {
    lookup_composed("ability.", ability.id, ".name").unwrap_or(ability.name)
}

/// An ability's one-line description.
pub(crate) fn ability_desc(ability: &AbilityDefinition) -> &'static str {
    lookup_composed("ability.", ability.id, ".desc").unwrap_or(ability.description)
}

/// An item's name (`Ember Blade`).
pub(crate) fn item_name(item: ItemId) -> &'static str {
    lookup_composed("item.", item.id(), ".name").unwrap_or_else(|| shared::shop::item(item).name)
}

/// An item's bonus summary (`+12% damage`).
pub(crate) fn item_desc(item: ItemId) -> &'static str {
    lookup_composed("item.", item.id(), ".desc")
        .unwrap_or_else(|| shared::shop::item(item).description)
}

/// An item's one-word name for tight slots (`Blade`).
pub(crate) fn item_short(item: ItemId) -> &'static str {
    tr(match item {
        ItemId::EmberBlade => "item.ember_blade.short",
        ItemId::SwiftGrip => "item.swift_grip.short",
        ItemId::TrailBoots => "item.trail_boots.short",
        ItemId::VitalityGem => "item.vitality_gem.short",
        ItemId::FocusCharm => "item.focus_charm.short",
        ItemId::GuardianCrest => "item.guardian_crest.short",
    })
}

/// A draft role (`Solo`).
pub(crate) fn role(role: Role) -> &'static str {
    tr(match role {
        Role::Solo => "role.solo",
        Role::Jungle => "role.jungle",
        Role::Mid => "role.mid",
        Role::Carry => "role.carry",
        Role::Support => "role.support",
    })
}

/// A supporter aura style (`Solar`).
pub(crate) fn aura(style: AuraStyle) -> &'static str {
    tr(match style {
        AuraStyle::Solar => "aura.solar",
        AuraStyle::Lunar => "aura.lunar",
        AuraStyle::Verdant => "aura.verdant",
    })
}

/// A reaction by id; the catalog label for an id without a dictionary entry,
/// `Reaction` for an unknown id.
pub(crate) fn reaction(id: &str) -> &'static str {
    lookup_composed("reaction.", id, "").unwrap_or_else(|| {
        shared::social::reaction(id).map_or_else(
            || tr("reaction.fallback"),
            |reaction| reaction.label.as_str(),
        )
    })
}

/// A reaction pack by id, falling back to its catalog label (or the id).
#[cfg_attr(not(test), allow(dead_code))] // spec helper; no screen shows it yet (tests pin it)
pub(crate) fn reaction_pack(id: &str) -> String {
    lookup_composed("reaction.pack.", id, "").map_or_else(
        || {
            shared::social::catalog()
                .packs
                .iter()
                .find(|pack| pack.id == id)
                .map_or_else(|| id.to_owned(), |pack| pack.label.clone())
        },
        str::to_owned,
    )
}

/// The key of a boss's display name, for a `Localized` label.
pub(crate) fn boss_key(camp: NeutralCampType) -> &'static str {
    match camp {
        NeutralCampType::WendigoBoss => "boss.wendigo",
        NeutralCampType::KingMutatioBoss => "boss.king_mutatio",
        _ => "boss.neutral",
    }
}

/// A boss's display name (`Wendigo`).
#[cfg_attr(not(test), allow(dead_code))] // spec helper; no screen shows it yet (tests pin it)
pub(crate) fn boss(camp: NeutralCampType) -> &'static str {
    tr(boss_key(camp))
}

/// The key of a lane's short label, for a `Localized` label.
#[cfg_attr(not(test), allow(dead_code))] // spec helper; no screen shows it yet (tests pin it)
pub(crate) fn lane_key(lane: Lane) -> &'static str {
    match lane {
        Lane::Top => "lane.top",
        Lane::Mid => "lane.mid",
        Lane::Bot => "lane.bot",
    }
}

/// A lane's short label (`TOP`).
#[cfg_attr(not(test), allow(dead_code))] // spec helper; no screen shows it yet (tests pin it)
pub(crate) fn lane(lane: Lane) -> &'static str {
    tr(lane_key(lane))
}

/// The label of a base structure (`BASE`).
#[cfg_attr(not(test), allow(dead_code))] // spec helper; no screen shows it yet (tests pin it)
pub(crate) fn lane_base() -> &'static str {
    tr("lane.base")
}

/// Keys of `frontend::card::TITLES`, index for index.
const TITLE_KEYS: [&str; 6] = [
    "title.newcomer",
    "title.lane_regular",
    "title.jungle_warden",
    "title.tower_breaker",
    "title.verdant_veteran",
    "title.ancient_champion",
];

/// A profile card title by its index into `frontend::card::TITLES`
/// (clamped like `ProfileCard::title_text`).
pub(crate) fn card_title(index: usize) -> &'static str {
    let key = TITLE_KEYS[index.min(TITLE_KEYS.len() - 1)];
    super::lookup(key).unwrap_or(key)
}

/// Why the server refused to seat the player.
pub(crate) fn join_rejection(rejection: JoinRejection) -> &'static str {
    tr(match rejection {
        JoinRejection::MatchFull => "error.join.match_full",
        JoinRejection::SessionActive => "error.join.session_active",
        JoinRejection::ProtocolMismatch => "error.join.protocol_mismatch",
        JoinRejection::MapGeometryMismatch => "error.join.map_geometry_mismatch",
        JoinRejection::AvatarNotAuthorized => "error.join.avatar_not_authorized",
    })
}

/// Why a shop purchase failed.
#[cfg_attr(not(test), allow(dead_code))] // spec helper; no screen shows it yet (tests pin it)
pub(crate) fn purchase_error(error: PurchaseError) -> &'static str {
    tr(match error {
        PurchaseError::Unavailable => "error.purchase.unavailable",
        PurchaseError::Dead => "error.purchase.dead",
        PurchaseError::OutsideBase => "error.purchase.outside_base",
        PurchaseError::InsufficientGold => "error.purchase.insufficient_gold",
        PurchaseError::AlreadyOwned => "error.purchase.already_owned",
        PurchaseError::InventoryFull => "error.purchase.inventory_full",
        PurchaseError::UnknownItem => "error.purchase.unknown_item",
    })
}

/// The Studio catalogue line shown by every avatar picker. The passport
/// crate's `CatalogueStatus::label` stays the English source text.
pub(crate) fn catalogue_status(status: &omoba_passport::store::CatalogueStatus) -> &'static str {
    use omoba_passport::store::CatalogueStatus;
    tr(match status {
        CatalogueStatus::Loading { .. } => "collection.catalogue.loading",
        CatalogueStatus::Empty => "collection.catalogue.empty",
        CatalogueStatus::Ready { .. } => "collection.catalogue.ready",
        CatalogueStatus::Unavailable { cached: 0 } => "collection.catalogue.unavailable",
        CatalogueStatus::Unavailable { .. } => "collection.catalogue.unavailable_cached",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{LocaleId, lookup_in};

    fn english(key: &str) -> &'static str {
        lookup_in(LocaleId::ENGLISH, key).unwrap_or_else(|| panic!("{key} missing in English"))
    }

    fn chinese(key: &str) -> &'static str {
        let zh = LocaleId::parse("zh-Hans").unwrap();
        lookup_in(zh, key).unwrap_or_else(|| panic!("{key} missing in zh-Hans"))
    }

    /// AC4 for game data: the English dictionary is the catalog/enum text, and
    /// every id has a Chinese entry of its own.
    #[test]
    fn english_game_data_equals_the_catalogs_and_chinese_covers_every_id() {
        for class in HeroClass::ALL {
            assert_eq!(hero_name(class), class.display_name());
            assert_eq!(hero_tagline(class), class.tagline());
            for (field, expected) in [("name", class.display_name()), ("tagline", class.tagline())]
            {
                let key = format!("hero.{}.{field}", class.id());
                assert_eq!(english(&key), expected);
                assert_ne!(chinese(&key), expected, "{key} is untranslated");
            }
            for ability in class.abilities() {
                assert_eq!(ability_name(ability), ability.name);
                assert_eq!(ability_desc(ability), ability.description);
                assert_ne!(
                    chinese(&format!("ability.{}.name", ability.id)),
                    ability.name
                );
                assert_ne!(
                    chinese(&format!("ability.{}.desc", ability.id)),
                    ability.description
                );
            }
        }
        for item in ItemId::ALL {
            let definition = shared::shop::item(item);
            assert_eq!(item_name(item), definition.name);
            assert_eq!(item_desc(item), definition.description);
            chinese(&format!("item.{}.short", item.id()));
        }
        assert_eq!(
            ItemId::ALL.map(item_short),
            ["Blade", "Grip", "Boots", "Gem", "Charm", "Crest"]
        );
        for value in Role::ALL {
            assert_eq!(role(value), value.label());
        }
        for style in AuraStyle::ALL {
            assert_eq!(aura(style), style.label());
        }
        let catalog = shared::social::catalog();
        for entry in &catalog.reactions {
            assert_eq!(reaction(&entry.id), entry.label);
            assert_ne!(chinese(&format!("reaction.{}", entry.id)), entry.label);
        }
        for pack in &catalog.packs {
            assert_eq!(reaction_pack(&pack.id), pack.label);
            chinese(&format!("reaction.pack.{}", pack.id));
        }
        assert_eq!(reaction("no_such_reaction"), "Reaction");
        for camp in [
            NeutralCampType::WendigoBoss,
            NeutralCampType::KingMutatioBoss,
            NeutralCampType::Skirmisher,
        ] {
            assert_eq!(boss(camp), crate::bosses::boss_name_id(camp));
        }
        assert_eq!(
            [Lane::Top, Lane::Mid, Lane::Bot].map(lane),
            ["TOP", "MID", "BOT"]
        );
        assert_eq!(lane_base(), "BASE");
        for (index, (title, _)) in crate::frontend::card::TITLES.iter().enumerate() {
            assert_eq!(card_title(index), *title);
        }
        assert_eq!(card_title(99), crate::frontend::card::TITLES[5].0);
        for rejection in [
            JoinRejection::MatchFull,
            JoinRejection::SessionActive,
            JoinRejection::ProtocolMismatch,
            JoinRejection::MapGeometryMismatch,
            JoinRejection::AvatarNotAuthorized,
        ] {
            assert_eq!(join_rejection(rejection), rejection.message());
        }
        for error in [
            PurchaseError::Unavailable,
            PurchaseError::Dead,
            PurchaseError::OutsideBase,
            PurchaseError::InsufficientGold,
            PurchaseError::AlreadyOwned,
            PurchaseError::InventoryFull,
            PurchaseError::UnknownItem,
        ] {
            assert_eq!(purchase_error(error), error.message());
        }
    }

    #[test]
    fn composed_keys_fall_back_to_the_catalog_for_long_or_unknown_ids() {
        let long = "x".repeat(200);
        assert_eq!(lookup_composed("ability.", &long, ".name"), None);
        assert_eq!(
            lookup_composed("ability.", "shield_bash", ".name"),
            Some("Shield Bash")
        );
    }
    #[test]
    fn catalogue_status_matches_the_passport_labels() {
        use omoba_passport::store::CatalogueStatus;
        for status in [
            CatalogueStatus::Loading { cached: 2 },
            CatalogueStatus::Empty,
            CatalogueStatus::Ready { count: 3 },
            CatalogueStatus::Unavailable { cached: 0 },
            CatalogueStatus::Unavailable { cached: 4 },
        ] {
            assert_eq!(catalogue_status(&status), status.label());
        }
        assert_eq!(
            chinese("collection.catalogue.loading"),
            "正在加载已审核的 Studio 形象…"
        );
    }
}
