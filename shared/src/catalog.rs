//! Data-driven hero and item catalogs.
//!
//! `shared/assets/catalog/heroes.json` and `items.json` hold what differs per
//! class or per item: display text, draft role, base HP, growth caps, basic
//! attack, projectile style, the Q/W/E/R kit, the recommended item order, and
//! item costs and bonuses. What is uniform (enums, rank scaling, unlock
//! levels, global balance values, the Warden's jungle passive) stays in code.
//!
//! Both files are embedded at compile time: client and server must agree, so
//! there is no runtime override. They are parsed and validated once per
//! process into static tables; bad data panics with the file and the entry
//! that failed. [`ensure_loaded`] runs that at startup so it never happens on
//! the first tick.
//!
//! The tables are converted into the public model types
//! ([`AbilityDefinition`], [`ItemDefinition`], [`BasicAttackDefinition`]).
//! Their `&'static str` fields are kept by leaking each string once, so the
//! definitions stay `Copy` and the accessors keep their signatures.

use crate::combat::ProjectileStyle;
use crate::prematch::Role;
use crate::shop::{ItemBonuses, ItemDefinition, ItemId};
use crate::{
    AbilityDefinition, BasicAttackDefinition, HeroClass, MAX_ABILITY_RANK, SkillSlot, TargetingMode,
};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::sync::LazyLock;

/// Layout version of both catalog files. Bump it with any schema change.
pub const SCHEMA_VERSION: u32 = 1;

const HEROES_PATH: &str = "shared/assets/catalog/heroes.json";
const ITEMS_PATH: &str = "shared/assets/catalog/items.json";
const HEROES_JSON: &str = include_str!("../assets/catalog/heroes.json");
const ITEMS_JSON: &str = include_str!("../assets/catalog/items.json");

static HEROES: LazyLock<Vec<HeroDefinition>> =
    LazyLock::new(|| loaded(HEROES_PATH, parse_heroes(HEROES_JSON)));
static ITEMS: LazyLock<Vec<ItemDefinition>> =
    LazyLock::new(|| loaded(ITEMS_PATH, parse_items(ITEMS_JSON)));

fn loaded<T>(path: &str, result: Result<T, String>) -> T {
    result.unwrap_or_else(|error| panic!("invalid catalog {path}: {error}"))
}

/// Parses and validates both catalogs now. The server's `runtime::run` and
/// the client's `main` call it first, so bad data stops the process at
/// startup instead of on the first lookup.
pub fn ensure_loaded() {
    LazyLock::force(&HEROES);
    LazyLock::force(&ITEMS);
}

/// Everything the catalog says about one class, in model types.
#[derive(Debug, Clone)]
pub(crate) struct HeroDefinition {
    pub display_name: &'static str,
    pub tagline: &'static str,
    pub role: Role,
    pub base_hp: f32,
    pub basic_damage_cap: f32,
    pub attack_rate_cap: f32,
    pub basic_attack: BasicAttackDefinition,
    pub projectile_style: ProjectileStyle,
    pub abilities: [AbilityDefinition; 4],
    pub recommended_items: Vec<ItemId>,
}

/// The class's catalog entry. The table is in `HeroClass::ALL` order, which
/// is the enum's declaration order (checked at load and by a test).
pub(crate) fn hero(class: HeroClass) -> &'static HeroDefinition {
    &HEROES[class as usize]
}

/// Every item, in `ItemId::ALL` order.
pub(crate) fn items() -> &'static [ItemDefinition] {
    &ITEMS
}

pub(crate) fn item(id: ItemId) -> &'static ItemDefinition {
    &ITEMS[id as usize]
}

// --- File layout (private; converted into the model types above) ---

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHeroCatalog {
    schema_version: u32,
    classes: Vec<RawHero>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHero {
    id: String,
    display_name: String,
    tagline: String,
    role: Role,
    base_hp: f32,
    growth: RawGrowth,
    basic_attack: RawBasicAttack,
    projectile_style: String,
    abilities: Vec<RawAbility>,
    recommended_items: Vec<ItemId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGrowth {
    basic_damage_cap: f32,
    attack_rate_cap: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBasicAttack {
    range: f32,
    damage: f32,
    cooldown_secs: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAbility {
    id: String,
    name: String,
    description: String,
    targeting: TargetingMode,
    mana_cost: f32,
    cooldown_secs: f32,
    cast_range: f32,
    projectile_damage: Option<f32>,
    self_heal: Option<f32>,
    self_mana_restore: Option<f32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawItemCatalog {
    schema_version: u32,
    items: Vec<RawItem>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawItem {
    id: ItemId,
    name: String,
    description: String,
    cost: u32,
    /// A partial `ItemBonuses`: omitted fields keep their neutral value.
    #[serde(default)]
    bonuses: Map<String, Value>,
}

// --- Parsing and validation ---

fn check(condition: bool, message: impl FnOnce() -> String) -> Result<(), String> {
    if condition { Ok(()) } else { Err(message()) }
}

fn check_schema(version: u32) -> Result<(), String> {
    check(version == SCHEMA_VERSION, || {
        format!("schema_version {version}, expected {SCHEMA_VERSION}")
    })
}

fn check_text(context: &str, field: &str, value: &str) -> Result<(), String> {
    check(!value.trim().is_empty(), || {
        format!("{context}: {field} is empty")
    })
}

fn check_positive(context: &str, field: &str, value: f32) -> Result<(), String> {
    check(value.is_finite() && value > 0.0, || {
        format!("{context}: {field} must be a finite number above 0, got {value}")
    })
}

fn check_at_least(context: &str, field: &str, value: f32, minimum: f32) -> Result<(), String> {
    check(value.is_finite() && value >= minimum, || {
        format!("{context}: {field} must be a finite number of at least {minimum}, got {value}")
    })
}

fn leak(text: String) -> &'static str {
    text.leak()
}

/// `ProjectileStyle` decodes unknown wire names as `Standard`; the catalog
/// must name a style exactly.
fn projectile_style(context: &str, name: &str) -> Result<ProjectileStyle, String> {
    let style: ProjectileStyle =
        serde_json::from_value(Value::from(name)).map_err(|e| e.to_string())?;
    let canonical = serde_json::to_value(style).map_err(|e| e.to_string())?;
    check(canonical.as_str() == Some(name), || {
        format!("{context}: unknown projectile_style {name:?}")
    })?;
    Ok(style)
}

fn parse_heroes(json: &str) -> Result<Vec<HeroDefinition>, String> {
    let raw: RawHeroCatalog = serde_json::from_str(json).map_err(|e| e.to_string())?;
    check_schema(raw.schema_version)?;
    check(raw.classes.len() == HeroClass::ALL.len(), || {
        format!(
            "{} classes, expected one per HeroClass ({})",
            raw.classes.len(),
            HeroClass::ALL.len()
        )
    })?;
    let mut ability_ids = HashSet::new();
    let mut heroes = Vec::with_capacity(raw.classes.len());
    for (index, (class, hero)) in HeroClass::ALL.into_iter().zip(raw.classes).enumerate() {
        check(hero.id == class.id(), || {
            format!(
                "classes[{index}] is {:?}, expected {:?} (HeroClass::ALL order)",
                hero.id,
                class.id()
            )
        })?;
        heroes.push(parse_hero(class, hero, &mut ability_ids)?);
    }
    Ok(heroes)
}

fn parse_hero(
    class: HeroClass,
    hero: RawHero,
    ability_ids: &mut HashSet<String>,
) -> Result<HeroDefinition, String> {
    let context = class.id();
    check_text(context, "display_name", &hero.display_name)?;
    check_text(context, "tagline", &hero.tagline)?;
    check_positive(context, "base_hp", hero.base_hp)?;
    check_at_least(
        context,
        "growth.basic_damage_cap",
        hero.growth.basic_damage_cap,
        1.0,
    )?;
    check_at_least(
        context,
        "growth.attack_rate_cap",
        hero.growth.attack_rate_cap,
        1.0,
    )?;
    check_positive(context, "basic_attack.range", hero.basic_attack.range)?;
    check_positive(context, "basic_attack.damage", hero.basic_attack.damage)?;
    check_positive(
        context,
        "basic_attack.cooldown_secs",
        hero.basic_attack.cooldown_secs,
    )?;
    let projectile_style = projectile_style(context, &hero.projectile_style)?;

    check(hero.abilities.len() == SkillSlot::ALL.len(), || {
        format!(
            "{context}: {} abilities, expected one per slot (Q/W/E/R)",
            hero.abilities.len()
        )
    })?;
    let mut abilities = Vec::with_capacity(SkillSlot::ALL.len());
    for (slot, ability) in SkillSlot::ALL.into_iter().zip(hero.abilities) {
        let context = format!("{context} {slot:?}");
        abilities.push(parse_ability(&context, ability, ability_ids)?);
    }

    // Unknown item ids already failed to parse, so this makes the list a
    // permutation of `ItemId::ALL`.
    for id in ItemId::ALL {
        let count = hero
            .recommended_items
            .iter()
            .filter(|item| **item == id)
            .count();
        check(count == 1, || {
            format!(
                "{context}: recommended_items lists {:?} {count} times, expected exactly once",
                id.id()
            )
        })?;
    }

    Ok(HeroDefinition {
        display_name: leak(hero.display_name),
        tagline: leak(hero.tagline),
        role: hero.role,
        base_hp: hero.base_hp,
        basic_damage_cap: hero.growth.basic_damage_cap,
        attack_rate_cap: hero.growth.attack_rate_cap,
        basic_attack: BasicAttackDefinition {
            range: hero.basic_attack.range,
            damage: hero.basic_attack.damage,
            cooldown_secs: hero.basic_attack.cooldown_secs,
        },
        projectile_style,
        abilities: abilities
            .try_into()
            .expect("ability count was checked against SkillSlot::ALL"),
        recommended_items: hero.recommended_items,
    })
}

fn parse_ability(
    context: &str,
    ability: RawAbility,
    ids: &mut HashSet<String>,
) -> Result<AbilityDefinition, String> {
    check_text(context, "id", &ability.id)?;
    check_text(context, "name", &ability.name)?;
    check_text(context, "description", &ability.description)?;
    check(ids.insert(ability.id.clone()), || {
        format!("{context}: ability id {:?} is used twice", ability.id)
    })?;
    check_at_least(context, "mana_cost", ability.mana_cost, 0.0)?;
    check_positive(context, "cooldown_secs", ability.cooldown_secs)?;
    check_at_least(context, "cast_range", ability.cast_range, 0.0)?;

    let effects = [
        ("projectile_damage", ability.projectile_damage),
        ("self_heal", ability.self_heal),
        ("self_mana_restore", ability.self_mana_restore),
    ];
    let present: Vec<_> = effects
        .iter()
        .filter_map(|(name, value)| value.map(|value| (*name, value)))
        .collect();
    check(present.len() == 1, || {
        format!(
            "{context}: needs exactly one of projectile_damage, self_heal, self_mana_restore, found {}",
            present.len()
        )
    })?;
    let (effect, amount) = present[0];
    check_positive(context, effect, amount)?;
    match ability.targeting {
        TargetingMode::UnitTarget => {
            check(ability.projectile_damage.is_some(), || {
                format!("{context}: a unit_target ability deals projectile_damage")
            })?;
            check_positive(context, "cast_range", ability.cast_range)?;
        }
        TargetingMode::SelfTarget => {
            check(ability.projectile_damage.is_none(), || {
                format!("{context}: a self_target ability cannot deal projectile_damage")
            })?;
            check(ability.cast_range == 0.0, || {
                format!("{context}: a self_target ability has cast_range 0")
            })?;
        }
    }

    Ok(AbilityDefinition {
        id: leak(ability.id),
        name: leak(ability.name),
        description: leak(ability.description),
        targeting: ability.targeting,
        base_mana_cost: ability.mana_cost,
        base_cooldown_secs: ability.cooldown_secs,
        cast_range: ability.cast_range,
        max_rank: MAX_ABILITY_RANK,
        projectile_damage: ability.projectile_damage,
        self_heal: ability.self_heal,
        self_mana_restore: ability.self_mana_restore,
    })
}

fn parse_items(json: &str) -> Result<Vec<ItemDefinition>, String> {
    let raw: RawItemCatalog = serde_json::from_str(json).map_err(|e| e.to_string())?;
    check_schema(raw.schema_version)?;
    check(raw.items.len() == ItemId::ALL.len(), || {
        format!(
            "{} items, expected one per ItemId ({})",
            raw.items.len(),
            ItemId::ALL.len()
        )
    })?;
    let Value::Object(bonus_fields) =
        serde_json::to_value(ItemBonuses::NONE).map_err(|e| e.to_string())?
    else {
        unreachable!("ItemBonuses serializes as an object")
    };
    let mut items = Vec::with_capacity(raw.items.len());
    for (index, (id, item)) in ItemId::ALL.into_iter().zip(raw.items).enumerate() {
        check(item.id == id, || {
            format!(
                "items[{index}] is {:?}, expected {:?} (ItemId::ALL order)",
                item.id.id(),
                id.id()
            )
        })?;
        let context = id.id();
        check_text(context, "name", &item.name)?;
        check_text(context, "description", &item.description)?;
        check(item.cost > 0, || format!("{context}: cost must be above 0"))?;
        if let Some(unknown) = item
            .bonuses
            .keys()
            .find(|key| !bonus_fields.contains_key(*key))
        {
            return Err(format!("{context}: unknown bonus {unknown:?}"));
        }
        let bonuses: ItemBonuses = serde_json::from_value(Value::Object(item.bonuses))
            .map_err(|e| format!("{context}: bonuses: {e}"))?;
        for (field, value) in [
            ("damage_multiplier", bonuses.damage_multiplier),
            ("attack_speed_multiplier", bonuses.attack_speed_multiplier),
            ("move_speed_multiplier", bonuses.move_speed_multiplier),
            ("spell_haste_multiplier", bonuses.spell_haste_multiplier),
        ] {
            check_at_least(context, field, value, 1.0)?;
        }
        check_at_least(context, "max_hp", bonuses.max_hp, 0.0)?;
        check_at_least(context, "max_mana", bonuses.max_mana, 0.0)?;
        items.push(ItemDefinition {
            id,
            name: leak(item.name),
            description: leak(item.description),
            cost: item.cost,
            bonuses,
        });
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::practice::DUEL_MAX_GOLD;
    use crate::shop::{INVENTORY_CAPACITY, STARTING_GOLD};

    fn heroes_with(mutate: impl FnOnce(&mut Value)) -> Result<Vec<HeroDefinition>, String> {
        let mut value: Value = serde_json::from_str(HEROES_JSON).unwrap();
        mutate(&mut value);
        parse_heroes(&value.to_string())
    }

    fn items_with(mutate: impl FnOnce(&mut Value)) -> Result<Vec<ItemDefinition>, String> {
        let mut value: Value = serde_json::from_str(ITEMS_JSON).unwrap();
        mutate(&mut value);
        parse_items(&value.to_string())
    }

    fn assert_rejected<T>(result: Result<T, String>, expected: &str) {
        match result {
            Ok(_) => panic!("catalog accepted data that should fail with {expected:?}"),
            Err(error) => assert!(
                error.contains(expected),
                "error {error:?} does not mention {expected:?}"
            ),
        }
    }

    /// Greedy shopping in recommended order, the way bots, the offline duel
    /// and the harness buy (roadmap step 11c moves it into `shop`).
    fn plan_purchases(class: HeroClass, mut gold: u32) -> Vec<ItemId> {
        let mut bought = Vec::new();
        for id in crate::shop::recommended_items(class) {
            let cost = crate::shop::item(*id).cost;
            if bought.len() < INVENTORY_CAPACITY && gold >= cost {
                gold -= cost;
                bought.push(*id);
            }
        }
        bought
    }

    #[test]
    fn shipped_catalogs_load() {
        ensure_loaded();
        assert_eq!(HEROES.len(), HeroClass::ALL.len());
        assert_eq!(ITEMS.len(), ItemId::ALL.len());
    }

    #[test]
    #[should_panic(expected = "invalid catalog shared/assets/catalog/heroes.json")]
    fn a_bad_file_panics_with_its_path() {
        loaded(HEROES_PATH, parse_heroes("{}"));
    }

    #[test]
    fn tables_follow_their_enums_in_order_and_ids_round_trip() {
        for (index, class) in HeroClass::ALL.into_iter().enumerate() {
            assert_eq!(class as usize, index, "{class:?}");
            assert_eq!(HeroClass::from_id(class.id()), Some(class));
            let wire = serde_json::to_value(class).unwrap();
            assert_eq!(wire, class.id());
            assert_eq!(serde_json::from_value::<HeroClass>(wire).unwrap(), class);
        }
        for (index, id) in ItemId::ALL.into_iter().enumerate() {
            assert_eq!(id as usize, index, "{id:?}");
            assert_eq!(items()[index].id, id);
            assert_eq!(item(id).id, id);
            assert_eq!(ItemId::from_id(id.id()), Some(id));
            let wire = serde_json::to_value(id).unwrap();
            assert_eq!(wire, id.id());
            assert_eq!(serde_json::from_value::<ItemId>(wire).unwrap(), id);
        }
        assert_rejected(
            heroes_with(|v| v["classes"].as_array_mut().unwrap().swap(0, 1)),
            "classes[0] is \"mage\", expected \"warrior\"",
        );
        assert_rejected(
            heroes_with(|v| {
                v["classes"].as_array_mut().unwrap().pop();
            }),
            "4 classes, expected one per HeroClass (5)",
        );
        assert_rejected(
            items_with(|v| v["items"].as_array_mut().unwrap().swap(1, 2)),
            "items[1] is \"trail_boots\", expected \"swift_grip\"",
        );
        assert_rejected(
            items_with(|v| {
                v["items"].as_array_mut().unwrap().pop();
            }),
            "5 items, expected one per ItemId (6)",
        );
    }

    #[test]
    fn schema_version_and_unknown_fields_are_rejected() {
        assert_rejected(
            heroes_with(|v| v["schema_version"] = 2.into()),
            "schema_version 2, expected 1",
        );
        assert_rejected(
            items_with(|v| v["schema_version"] = 0.into()),
            "schema_version 0, expected 1",
        );
        assert_rejected(
            heroes_with(|v| v["classes"][0]["armor"] = 3.into()),
            "unknown field `armor`",
        );
        assert_rejected(
            heroes_with(|v| v["classes"][1]["abilities"][0]["max_rank"] = 5.into()),
            "unknown field `max_rank`",
        );
        assert_rejected(
            items_with(|v| v["items"][0]["bonuses"]["damage_multiplyer"] = 1.5.into()),
            "ember_blade: unknown bonus \"damage_multiplyer\"",
        );
        assert_rejected(
            heroes_with(|v| v["classes"][0]["projectile_style"] = "laser".into()),
            "warrior: unknown projectile_style \"laser\"",
        );
        assert_rejected(
            heroes_with(|v| v["classes"][0]["role"] = "tank".into()),
            "unknown variant `tank`",
        );
    }

    #[test]
    fn every_class_has_a_full_well_formed_kit() {
        for class in HeroClass::ALL {
            let hero = hero(class);
            assert!(!hero.display_name.is_empty() && !hero.tagline.is_empty());
            assert!(hero.base_hp > 0.0);
            assert!(hero.basic_damage_cap >= 1.0 && hero.attack_rate_cap >= 1.0);
            for ability in &hero.abilities {
                assert_eq!(ability.max_rank, MAX_ABILITY_RANK, "{}", ability.id);
            }
        }
        assert_rejected(
            heroes_with(|v| {
                v["classes"][2]["abilities"].as_array_mut().unwrap().pop();
            }),
            "ranger: 3 abilities, expected one per slot (Q/W/E/R)",
        );
        assert_rejected(
            heroes_with(|v| v["classes"][0]["abilities"][1]["self_mana_restore"] = 5.into()),
            "warrior W: needs exactly one of projectile_damage, self_heal, self_mana_restore, found 2",
        );
        assert_rejected(
            heroes_with(|v| {
                let q = v["classes"][1]["abilities"][0].as_object_mut().unwrap();
                q.remove("projectile_damage");
                q.insert("self_heal".into(), 10.into());
            }),
            "mage Q: a unit_target ability deals projectile_damage",
        );
        assert_rejected(
            heroes_with(|v| v["classes"][3]["abilities"][1]["cast_range"] = 8.into()),
            "cleric W: a self_target ability has cast_range 0",
        );
        assert_rejected(
            heroes_with(|v| v["classes"][4]["abilities"][3]["id"] = "rampage".into()),
            "warden R: ability id \"rampage\" is used twice",
        );
        assert_rejected(
            heroes_with(|v| v["classes"][0]["abilities"][2]["name"] = " ".into()),
            "warrior E: name is empty",
        );
        assert_rejected(
            heroes_with(|v| v["classes"][0]["abilities"][0]["cooldown_secs"] = 0.into()),
            "warrior Q: cooldown_secs must be a finite number above 0",
        );
        assert_rejected(
            heroes_with(|v| v["classes"][0]["growth"]["attack_rate_cap"] = 0.9.into()),
            "warrior: growth.attack_rate_cap must be a finite number of at least 1",
        );
        assert_rejected(
            heroes_with(|v| v["classes"][1]["base_hp"] = (-1).into()),
            "mage: base_hp must be a finite number above 0",
        );
    }

    #[test]
    fn item_costs_and_bonuses_are_in_range() {
        for item in items() {
            let bonuses = item.bonuses;
            assert!(item.cost > 0, "{:?}", item.id);
            for multiplier in [
                bonuses.damage_multiplier,
                bonuses.attack_speed_multiplier,
                bonuses.move_speed_multiplier,
                bonuses.spell_haste_multiplier,
            ] {
                assert!(multiplier >= 1.0, "{:?}", item.id);
            }
            assert!(bonuses.max_hp >= 0.0 && bonuses.max_mana >= 0.0);
            assert_ne!(bonuses, ItemBonuses::NONE, "{:?} grants nothing", item.id);
        }
        assert_rejected(
            items_with(|v| v["items"][3]["cost"] = 0.into()),
            "vitality_gem: cost must be above 0",
        );
        assert_rejected(
            items_with(|v| v["items"][0]["bonuses"]["damage_multiplier"] = 0.9.into()),
            "ember_blade: damage_multiplier must be a finite number of at least 1",
        );
        assert_rejected(
            items_with(|v| v["items"][4]["bonuses"]["max_mana"] = (-5).into()),
            "focus_charm: max_mana must be a finite number of at least 0",
        );
        assert_rejected(
            items_with(|v| v["items"][5]["description"] = "".into()),
            "guardian_crest: description is empty",
        );
    }

    #[test]
    fn recommendations_list_every_item_once_and_reject_unknown_ids() {
        for class in HeroClass::ALL {
            let recommended = &hero(class).recommended_items;
            for id in ItemId::ALL {
                assert_eq!(recommended.iter().filter(|r| **r == id).count(), 1);
            }
            assert_eq!(recommended.len(), ItemId::ALL.len());
        }
        assert_rejected(
            heroes_with(|v| v["classes"][0]["recommended_items"][0] = "free_gold".into()),
            "unknown variant `free_gold`",
        );
        assert_rejected(
            heroes_with(|v| v["classes"][1]["recommended_items"][1] = "ember_blade".into()),
            "mage: recommended_items lists \"ember_blade\" 2 times, expected exactly once",
        );
        assert_rejected(
            heroes_with(|v| {
                v["classes"][2]["recommended_items"]
                    .as_array_mut()
                    .unwrap()
                    .pop();
            }),
            "ranger: recommended_items lists \"focus_charm\" 0 times",
        );
    }

    #[test]
    fn starter_budget_opens_every_build_and_the_duel_budget_fills_the_inventory() {
        for class in HeroClass::ALL {
            let first = crate::shop::recommended_items(class)[0];
            assert!(
                crate::shop::item(first).cost <= STARTING_GOLD,
                "{class:?} cannot afford its first item"
            );
            assert_eq!(
                plan_purchases(class, STARTING_GOLD + DUEL_MAX_GOLD).len(),
                INVENTORY_CAPACITY,
                "{class:?}"
            );
        }
    }
}
