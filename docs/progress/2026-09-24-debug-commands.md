# 2026-09-24 — Debug command family (shared and server half)

## Goal
Roadmap step 11, slices 11a-11c (plan: `docs/plans/steps-11-13.md`,
"Step 11"; slice 11-0 landed in #34). The development toggles and the
practice sandbox were three unrelated packets with a gate each: the toggles
checked `rules.debug_commands` plus a worker check in
`runtime/handlers/debug.rs`, the practice commands checked
`rules.fills_with_bots` plus a worker check deep inside `bots.rs`, and
`bots.rs` mixed the bot AI with the practice orchestration. The dummy
constants existed twice (server and offline), and the "ultimate first, then
Q, W, E" rank policy and the greedy recommended-order shopper existed three
or four times (server bots, the offline duel, the harness bots, the catalog
test).

## Changes
- 11a, shared: `shared/src/debug.rs`.
  - `DebugCommand { GodMode(bool), SpeedBoost(bool), Practice(PracticeCommand) }`,
    `Debug, Clone, Copy, PartialEq, Eq`, no serde. `to_packet()` and
    `from_packet(&ClientPacket) -> Option<DebugCommand>` map it onto
    `SetGodMode`, `SetSpeedBoost` and `Practice`; every other packet
    (including the Combat Test `Sandbox`) is `None`.
  - `DebugAccess { toggles, practice }` (`Default` = neither) with
    `for_match_mode(&str)`: `"dev"` toggles only, `"practice"` and
    `OFFLINE_PRACTICE_MODE` both, anything else neither; `allows(command)`.
  - `DUMMY_MAX_HP` (600), `DUMMY_DISTANCE` (4.5), `OFFLINE_PRACTICE_MODE`
    (`"offline_practice"`); `pub use crate::practice::{self, PracticeCommand}`.
  - `PracticeCommand::Unsupported` with `#[serde(other)]`. Serde allows a
    unit `other` variant in an internally tagged enum, which
    `PracticeCommand` is (`tag = "kind"`): an unknown kind decodes, extra
    fields are ignored, and a known kind with a malformed body still fails.
- 11b, server: `server/src/debug/`.
  - `mod.rs`: `ServerRuntime::debug_access()` and
    `ServerRuntime::handle_debug(addr, DebugCommand, now) -> ControlFlow<()>`.
    `handle_debug` asks `debug_access().allows(command)` once and keeps the
    per-variant flow of the old handlers: a refused toggle returns `Break`
    before `ensure_connected`; an accepted one runs the setter (`Break` if
    the sender has not joined, else `Continue`); a practice command touches
    `last_seen` first, runs only when allowed, and always returns `Break`.
    The parity test lives here.
  - `toggles.rs`: `set_god_mode` and `set_speed_boost`, the bodies of the
    old `handle_set_god_mode`/`handle_set_speed_boost` without the gate
    (`debug_toggles_allowed` from #34 became `debug_access().toggles`).
  - `practice.rs`: `handle_practice_command` (without its own access check,
    plus an `Unsupported => {}` arm), `dummy_anchor`, `configure_duelist`,
    `opposite_team`, moved from `bots.rs`.
  - `bots.rs`: loses the dummy constants and the orchestration above;
    `spawn_bot`, `remove_all_bots`, `remove_bot`, `place_dummy` and the
    `BotControllers.sandbox` flag are `pub(crate)` for `debug::practice`.
  - `runtime/handlers/debug.rs` → `runtime/handlers/tools.rs` with only
    `handle_sandbox_packet`.
  - `runtime/dispatch.rs`: the `Practice` arm and the (now one)
    `SetGodMode | SetSpeedBoost` arm call `dispatch_debug(addr, &packet,
    clock)`, which converts with `DebugCommand::from_packet` and calls
    `handle_debug`.
- 11c, planners: `shared::progression::skill_upgrade_order(class, level,
  ranks, points) -> Vec<u8>` (new module) and
  `shared::shop::plan_purchases(class, gold, owned) -> Vec<ItemId>`.
  - Server `bots::auto_rank_skills` applies the order through
    `apply_skill_upgrade`; `bots::auto_shop` calls `handle_purchase` once per
    planned item, after the same `shop_is_available` check.
  - Harness `bot_ai::choose_shop_item` is the first planned item (after the
    snapshot's `shop_available` and HP checks).
  - Offline `StartDuel` computes ranks from `[1; 4]` with `level - 1` points
    and buys `plan_purchases(class, STARTING_GOLD + gold, &[])`, keeping the
    unspent gold; `ranks_for_level` and `shop_with` are gone. The other
    `offline.rs` edits are the shared constants and the `Unsupported` arm.
  - `catalog::tests` uses `shop::plan_purchases` instead of its local copy.
- Docs: `ARCHITECTURE.md` (crate map, dispatcher paragraph, new "Debug
  commands" section, roadmap line 11), `REFACTORING.md` (row 11, the step 11
  section; row 12 now names #38), `CHANGELOG.md`.

## Dispatcher arm mapping
| Packet | Arm position (unchanged) | Clock | Flow |
| --- | --- | --- | --- |
| `Practice` | third, after `Leave` and the career-flow `RequestRematch`, before `Sandbox` and the paused-sandbox gate | wall clock | `Break` always; endpoint touched even when refused |
| `SetGodMode`, `SetSpeedBoost` | after `RequestRematch`, before `UpgradeSkill` (the two arms are one or-pattern) | sandbox simulation clock | `Break` when refused (endpoint untouched) or not joined; `Continue` once applied |

## Decisions
- `place_dummy` stays in `bots.rs` (the plan's range put it in
  `practice.rs`): `simulate_bots` also calls it to walk a dummy back after a
  respawn, and the bot AI should not depend on the debug module.
- The one behaviour difference is inherent to `Unsupported`: a practice
  datagram with an unknown kind used to fail decoding and be dropped before
  dispatch; now it reaches the `Practice` arm, refreshes `last_seen` like any
  refused practice command, and does nothing else.
- `DebugAccess` has no serde derives yet; 11f (`Snapshot.debug_access`)
  adds them when the field exists.
- `for_match_mode` spells the server's `MatchMode::id` strings; the server
  parity test (over an exhaustive list of modes) is what keeps them in step.
  A worker round still reports `"practice"` while refusing both families
  (`match_allocation` test asserts `debug_access()` is empty); the client
  learns that only with 11f.
- `plan_purchases` takes the owned items rather than the whole player, so
  the harness (`PlayerState`), the server (`HeroEconomy`) and offline (an
  empty wallet) share it without adapters.

## Verification
- Equivalence (temporary tests, deleted with the old code): the old code was
  copied verbatim next to the new calls.
  - Server `auto_rank_skills`: every class × level 1-10 × all 81 rank
    vectors in 1..=3 × 0-12 points, 52,650 cases, identical `Hero` and
    `HeroEconomy`.
  - Server `auto_shop` on a joined hero in its base: every class × all 64
    owned-item subsets × gold 0-1000, 320,320 cases (775,280 purchases),
    identical `Hero` and `HeroEconomy` (gold, inventory, bonuses, receipts,
    purchase sequence, HP and mana).
  - Harness `choose_shop_item`: every class × 64 owned subsets × gold 0-1000
    × three eligibility states, 960,960 cases, identical.
  - Offline duel through the real `StartDuel` command: every class × level
    1-10 × gold 0-1000, 50,050 cases, identical ranks, inventory, item
    bonuses and remaining gold.
- Neutrality: the golden JSON tests in `shared/src/protocol/wire.rs` and the
  snapshot byte pins in `server/src/tests/player_view.rs` are unchanged and
  green; `debug::tests::every_command_encodes_as_todays_packet` pins the
  family to the same strings.
- New tests: `debug::tests` (encodings, unknown kind, access table),
  `progression::tests` (2), `shop::tests::purchase_plans_follow_the_recommended_order_within_budget_and_room`,
  server `debug::tests::match_mode_access_equals_the_server_access_without_a_worker`
  and `practice_tests::an_unknown_practice_kind_from_a_newer_client_is_ignored`
  (raw datagrams through the memory transport); the worker test from #34
  also asserts an empty `debug_access()`.
- Re-check after a container restart (the equivalence tests above were run
  by the first session and deleted with the old code): the old server
  `auto_rank_skills`/`auto_shop` and the old offline `ranks_for_level`/
  `shop_with` and harness `choose_shop_item` bodies were copied from
  `5a1178d` into temporary tests again and compared with the new code
  (server: 52,650 rank cases and 320,320 shop cases on a real duelist in its
  base, identical `Hero` and `HeroEconomy`; offline: every class × level
  1-10 × gold 0-1000, 50,050 cases; harness: every class × owned subset ×
  gold 0-1000, 320,320 cases). All green; the temporary tests are deleted.
- Gate (all green):
  - `cargo fmt --all -- --check`: clean.
  - `cargo clippy --workspace --all-targets --no-deps -- -D warnings`: clean.
  - `cargo clippy -p client --lib --no-deps --no-default-features -- -D warnings`: clean.
  - `cargo test -p shared -p server`: shared 94 passed; server 285 passed,
    3 ignored.
  - `cargo test -p client --lib`: 549 passed.
  - `cargo build -p server && cargo test --locked -p harness -- --test-threads=1`:
    22 unit + 24 integration passed.
  - `python3 -m unittest discover -s scripts -p 'test_*.py'`: 93 tests OK
    (1 skipped).
