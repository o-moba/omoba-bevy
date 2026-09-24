# 2026-09-24 — Data-driven hero and item catalogs

## Goal
Roadmap step 12 (plan: `docs/plans/steps-11-13.md`, "Step 12"; slices
12a-12e in one PR). Every per-class and per-item number and string lived in
Rust: five `*_ABILITIES` constants and `const fn` matches in
`shared/src/lib.rs`, base HP and growth caps in `hero_balance.rs`, `ITEMS`
and the recommended orders in `shop.rs`, the projectile style in
`combat.rs`, and copies of the class list, item costs and projectile styles
in the Python scripts. `ITEMS` was typed `[ItemDefinition; INVENTORY_CAPACITY]`,
so a seventh item would have forced a seventh inventory slot.

## Changes
- 12a, API prep: `ItemId::ALL`; `ItemId::from_id` searches the enum instead
  of `ITEMS`; `shop::items() -> &'static [ItemDefinition]` replaced every
  direct `ITEMS` use (client `shop.rs`, `sandbox/ui.rs`,
  `sandbox/presets.rs`; server `shop.rs` test, `sandbox/tests.rs`); harness
  `combat_actions.rs` reads the Warrior Q through `ability_for_class_slot`
  instead of `WARRIOR_ABILITIES[0]`; `DEFAULT_MAX_HP` became the literal
  `220.0` with `hero_balance::tests::default_max_hp_is_the_warrior_base_hp`.
- 12b, data and loader: `shared/assets/catalog/heroes.json` (`{schema_version,
  classes: [...]}` in `HeroClass::ALL` order) and `items.json`
  (`{schema_version, items: [...]}` in `ItemId::ALL` order), and
  `shared/src/catalog.rs`. Two `LazyLock` tables parse the `include_str!`
  text through private `Raw*` structs with `deny_unknown_fields`, validate
  it and convert it into the unchanged public types. Validation failures
  panic with `invalid catalog <path>: <entry>: <problem>`.
- 12c, switch: `basic_attack_for_class`, `HeroClass::{display_name, tagline,
  primary_role, abilities, ability}`, `hero_balance::{base_hp,
  basic_damage_multiplier, attack_rate_multiplier}`, `shop::{items, item,
  recommended_items}` and `ProjectileStyle::for_class` read the catalog; the
  tables, the `ability` const helper and the migration test are gone.
  `shared::catalog::ensure_loaded()` is the first line of server
  `runtime::run` and client `main`.
- 12d, capacity: the catalog length is `ItemId::ALL.len()`. The late-game
  preset (`client/src/sandbox/presets.rs`) and the server tests that fill an
  inventory (`server/src/shop.rs` `InventoryFull`, `sandbox/tests.rs` twice)
  take at most `INVENTORY_CAPACITY` items; the preset test expects
  `min(items, capacity)`. `practice::tests::duel_gold_cap_buys_a_full_inventory`
  replaces the comment "Six items cost 540 in total, so the cap buys the full
  inventory": the dearest `INVENTORY_CAPACITY` items cost at most
  `DUEL_MAX_GOLD`.
- 12e, Python: `scripts/catalog.py` (`heroes`, `items`, `hero_ids`,
  `item_costs`, `projectile_styles`, `offensive_slots`; checks
  `schema_version`) reads `shared/assets/catalog/*.json`. `combat_test.py`
  takes its `--hero` choices, `verify_beta_match.py` its `ITEM_COSTS` and
  offensive slots, `capture_combat.py` its `STYLES` from it.
  `check_combat_balance.py` keeps taking its classes from the captures (its
  baseline is a frozen artefact).
- Docs: `ARCHITECTURE.md` ("Adding content" rewritten, crate map, the
  one-home rule, roadmap line 12), `docs/balance-tuning.md` (points at the
  JSON), `REFACTORING.md` (row 12 and the step 12 section), `CHANGELOG.md`.

## Schema
`heroes.json`, per class: `id`, `display_name`, `tagline`, `role`
(`prematch::Role`), `base_hp`, `growth {basic_damage_cap, attack_rate_cap}`,
`basic_attack {range, damage, cooldown_secs}`, `projectile_style`
(`ProjectileStyle` wire name), `abilities` (four, Q/W/E/R: `id`, `name`,
`description`, `targeting`, `mana_cost`, `cooldown_secs`, `cast_range`, and
exactly one of `projectile_damage`, `self_heal`, `self_mana_restore`;
`max_rank` is always `MAX_ABILITY_RANK`), `recommended_items`.
`items.json`, per item: `id`, `name`, `description`, `cost`, `bonuses` (a
partial `ItemBonuses`; omitted fields are neutral, unknown ones rejected).

## Decisions
- JSON, not RON: `serde_json` is already a dependency and Python reads it
  natively. Embedded only, like `reactions.json`: client and server must
  agree, so no runtime override.
- Strings are leaked once per process (about 100) so `AbilityDefinition` and
  `ItemDefinition` keep their `&'static str` fields and stay `Copy`; none of
  the field-access sites changed. Borrowing from the `include_str!` text
  would break on any JSON escape.
- Tables are indexed by `class as usize` / `id as usize`. The loader checks
  the file order against `HeroClass::ALL` / `ItemId::ALL`, and a test checks
  that `ALL[i] as usize == i`, so a lookup is one atomic load and an index.
- `ItemBonuses` keeps its wire-friendly `serde(default)` without
  `deny_unknown_fields`; the loader instead rejects any `bonuses` key that
  `ItemBonuses::NONE` does not serialize, so a typo cannot silently become a
  neutral bonus.
- `ProjectileStyle` decodes unknown names as `Standard` on the wire; the
  loader requires the name to round-trip exactly.
- Structural rules (enum coverage and order, kit shape, effect and
  targeting, value ranges, unique ability ids, recommendations) are load
  validation. Design rules (five distinct roles, basic attack shorter than
  Q, starter budget) stay tests over the shipped data.
- `plan_purchases` from the plan's validation list is step 11c; the starter
  budget test uses a local greedy shopper with the same rules as the bots,
  the offline duel and the harness.
- The plan listed three "all items" sites for 12d; the second loop in
  `sandbox/tests.rs` (grant every item, then expect a duplicate to be
  refused) had the same coupling and also takes `INVENTORY_CAPACITY`.

## Verification
- Migration: `catalog::tests::catalog_matches_the_rust_tables_bit_for_bit`
  compared every field of every class, ability and item with the Rust
  tables through `f32::to_bits` (and string/enum equality) and passed before
  12c deleted it with the tables.
- Neutrality: the snapshot byte pins in `server/src/tests/player_view.rs`
  and the golden JSON tests in `shared/src/protocol/wire.rs` are unchanged
  and green.
- GATE_PLACEHOLDER
