# 2026-09-24 — Server: authoritative hero state, `PlayerState` only in the views

## Goal
Roadmap step 6, third slice. After the views slice, `ConnectedPlayer.state`
(the wire `PlayerState`) was still the storage for everything authoritative:
identity, position and pools, progression, economy, utility marks and the
last-action record, next to two loose economy fields on `ConnectedPlayer`.
This slice moves all of it into server-owned structs and deletes the stored
`PlayerState`, so the wire struct exists on the server only as the output of
`owner_view` / `public_view`.

## Changes
- `server/src/hero.rs` (new): `Hero { identity: HeroIdentity, x, y, z, yaw,
  hp, max_hp, mana, max_mana, progress: HeroProgress, utility: HeroUtility,
  last_action: HeroAction }` and `HeroEconomy { gold, earned_gold, inventory,
  item_bonuses, last_purchase, basic_attack_request_id, purchase_sequence,
  gold_income_remainder }`. `HeroIdentity` carries id, `is_bot`, team,
  `hero_class`, `character`, `avatar`, `sprite_character` and
  `supporter_aura`: the fields set at join or reconnect and never by the
  simulation. `HeroAction` keeps the wire's `kind: PlayerActionKind` and
  `slot: u8` (with `BASIC_ATTACK_ACTION_SLOT`) because the sandbox clears
  only the kind and the view must reproduce the stored slot. `Hero::new(id,
  spawn)`, `HeroProgress::starting()` and `HeroEconomy::starting()` replace
  the join-time and round-reset literals.
- `ConnectedPlayer` (`entities.rs`): `hero: Hero` and `economy: HeroEconomy`
  replace `state: PlayerState`, `purchase_sequence` and
  `gold_income_remainder`. `owner_view` is now a full `PlayerState` literal
  built from `hero`, `economy` and the `hero_timers` reads; `public_view`
  still returns it. `snapshot::build_players_snapshot` and the test accessor
  `ServerRuntime::player_view` are unchanged.
- `session.rs`: `ensure_connected` builds `Hero::new(player_id, spawn)`
  instead of a `PlayerState`; the join handler writes the identity fields;
  `reset_player_round` resets `economy`, `progress`, `utility` and
  `last_action` whole. Reconnect still moves the whole `ConnectedPlayer`, so
  the economy and progression survive it as before.
- `progression.rs`: `apply_level_up(&mut Hero)`, `grant_player_xp(&mut Hero,
  amount)`. `shop::shop_is_available` / `inside_own_shop` and
  `sim::neutrals::neutral_horizontal_distance_sq_from_anchor` take `&Hero`.
- Mechanical rename everywhere else (`shop.rs`, `bots.rs`, `sim/*`,
  `basic_attack.rs`, `utility.rs`, `sandbox.rs`, `vision.rs`, `prematch.rs`,
  `career_runtime.rs`, `match_stats.rs`, `combat_feedback.rs`,
  `forest_pickups.rs`, `formation.rs`, `runtime/*`, `hero_timers.rs` and
  every test module): `player.state.gold` → `player.economy.gold`,
  `player.state.level` → `player.hero.progress.level`,
  `player.state.utility.dash_sequence` → `player.hero.utility.dash_sequence`,
  `player.state.action_sequence` → `player.hero.last_action.sequence`,
  `player.state.team` → `player.hero.identity.team`, `player.state.hp` →
  `player.hero.hp`, and so on. No logic changed in the renamed sites.
- Tests: `tests/player_view.rs::assert_view` builds `want` as a hand-written
  `PlayerState` literal from `hero` and `economy` plus the expected clocks,
  so the field mapping is pinned independently of `owner_view`; the "stored
  struct never carries the derived fields" assertions are gone because the
  stored structs no longer have those fields. Tests that cloned the stored
  `PlayerState` now clone `Hero` (`movement`, `sessions`, `balance_probe`,
  `forest_pickups`, `practice_tests`, `release_tests`, which compares
  `(Hero, HeroEconomy)` with `PartialEq`) or read `rt.player_view(addr, now)`
  where they assert wire output (`shop.rs` rejection test,
  `combat_feedback/tests.rs`). No `#[cfg(test)]` builder was needed: the one
  `ConnectedPlayer` literal is `ensure_connected`.

## Behaviour
- Wire format and snapshot bytes unchanged for every recipient; `shared/` is
  untouched. Reset and respawn write the same values in the same order as
  before (whole-struct resets replace field-by-field resets of the same
  fields).

## Checks
- `cargo test -p server`: 276 passed, 3 ignored, before and after (no test
  added or removed).
- `cargo fmt --all`, `cargo clippy --workspace --all-targets --no-deps -- -D warnings`,
  `cargo test -p shared -p server`, `cargo test -p harness --no-run`,
  `cargo test -p client --lib` are green.

## Left for the next slices
- `StatModifiers` / `hero_stats.rs`: one place for the class, level, item
  and sandbox multipliers that `sandbox::effective_*` and the movement
  envelope compute today.
- Redaction: `public_view` hides what other teams should not see; the
  broadcast then picks the view per recipient.
