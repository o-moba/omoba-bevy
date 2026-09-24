# 2026-09-24 — Server packet handlers and explicit imports

## Goal
Roadmap step 14. `runtime/dispatch.rs::handle_packet_authorized` was a
400-line function: ordered admission pre-checks followed by one `match` with
a 100-line `Join` arm and `unreachable!` arms for the variants the
pre-checks had already handled. `main.rs` re-exported two dozen modules
through globs, and 79 `use crate::*;`/`use super::*;` imports let every
module see everything, so no file said what it depended on.

## Changes
- `server/src/runtime/handlers/` (new): `mod.rs` plus one file per
  concern, each an `impl ServerRuntime` block with
  `pub(in crate::runtime) fn handle_<variant>(&mut self, addr, <fields>, now)
  -> ControlFlow<()>` and the arm body moved verbatim.
  - `join.rs`: `handle_join` (reclaim, team assignment per
    `rules.team_assignment`, bot seat replacement, loadout, formation). The
    worker allocation's `allocated_team` is resolved by the dispatcher
    before the join normalisation and passed in.
  - `movement.rs`: `handle_transform`.
  - `combat.rs`: `handle_cast`, `handle_basic_attack`, `handle_upgrade_skill`.
  - `utility.rs`: `handle_utility`.
  - `shop.rs`: `handle_buy_item`.
  - `debug.rs`: `handle_set_god_mode`, `handle_set_speed_boost`,
    `handle_practice`, `handle_sandbox_packet`.
  - `session.rs`: `handle_hello`, `handle_ping`, `handle_leave`,
    `handle_career_rematch`, `handle_request_rematch`.
- `server/src/runtime/dispatch.rs`: keeps `receive_packets`,
  `handle_packet` and `handle_packet_authorized`. The pre-checks up to the
  career and practice join admission are untouched. The rest is one `match`
  whose first arms are the former early-return checks in their old order
  (`Leave`; `RequestRematch if self.career_flow_active()`; `Practice`;
  `Sandbox`; movement and combat `if` the sandbox is paused), then one line
  per handler, then a single `unreachable!` for `Career`/`Social`/`Prematch`
  (routed by `handle_packet`). The post-command tail runs when the arm
  returned `Continue`.
- Imports: `main.rs` is now the module list, `use std::io;` and `fn main`.
  The crate-root globs (`use balance::*`, `basic_attack`, `combat_feedback`,
  `neutrals`, `progression`, `session`, `shop`, `utility`, `world`;
  `pub(crate) use entities::*`, `formation`, `game_world`, `hero`,
  `match_rules`, `runtime::dispatch`, `runtime::ports`, `runtime`,
  `snapshot`, `sim::{cast, minions, neutrals, projectiles, towers, *}`), the
  explicit root re-exports (`CareerPort`, `StatModifiers`, `HeroTimers`) and
  the root `use shared::…`, `use std::…`, `use omoba_career_store::…` lines
  are gone. Every module names its imports (71 existing files touched), grouped std /
  external / crate at module granularity (`use crate::balance::{AIM_HEIGHT,
  …};`, `use crate::{bots, sandbox};` for module paths). Imports needed
  only by `#[cfg(test)]` code in a non-test module carry `#[cfg(test)]`.
- Docs: `ARCHITECTURE.md` Server tick (dispatcher, handlers, import rule)
  and roadmap line 5, `REFACTORING.md` row 14 and the step 14 section (and
  row 7's stale "this PR" → #32), `CHANGELOG.md`.

## Decisions
- Handlers return `ControlFlow<()>` instead of `()`: the old arms returned
  early from `handle_packet_authorized` in twelve places, which skipped the
  post-command tail (`last_seen = wall_now`, `initialize_sandbox_players`,
  `fill_practice_bots`, `tick_prematch`, `track_round_start`,
  `register_career_participant`). `Break` is exactly those returns.
- The single `match` with guards keeps the order identical: every moved
  pre-check matched a distinct variant, the paused-sandbox guard only
  matches the four gameplay variants, and the reads moved ahead of them
  (`self.sandbox.map_or(now, |s| s.now)`, `career_flow_active`, the pause
  flag) have no side effects. The pre-check arms receive the wall clock,
  the handlers the sandbox clock, as before. The per-handler captures
  (`sandbox_allowed()`, `targeting_qa`, `rules`, `match_id`,
  `server_epoch`) are read at the top of the handler; nothing between the
  old capture point and the arm could change them.
- `pub(in crate::runtime)` instead of `pub(super)`: `pub(super)` in
  `runtime/handlers/join.rs` would stop at `runtime::handlers`, and the
  dispatcher lives in `runtime::dispatch`.
- The imports were derived from the compiler, not by hand, and every name
  resolves to the item the crate-root glob resolved it to: the root's
  explicit imports first (`shared::map::Team`, `std::time::Instant`, …),
  then the glob module that defines the item (an audit found no name
  defined in two glob modules). The `balance.rs` re-exports of
  `shared::hero_balance` stay the import path (`crate::balance::MAX_MANA`),
  and `crate::progression::xp_threshold_for_level` (a wrapper) stays
  distinct from `shared::hero_balance::xp_threshold_for_level`.
- Test modules keep `use super::*;` for their parent module (29 sites);
  the root-level test files (`practice_tests.rs`, `release_tests.rs`, …)
  and `tests/mod.rs` import explicitly because their parent is the crate
  root. The `use shared::{career, sandbox, utility, vision}::*;` globs
  import a foreign domain module and are out of scope.

## Checks
- `cargo fmt --all`, `cargo clippy --workspace --all-targets --no-deps -- -D warnings` clean.
- `cargo check -p server` (no tests) warning-free.
- `cargo test -p shared -p server`: shared 78, server 282 (+3 ignored), unchanged.
- `cargo test -p harness --no-run` builds.
- Black-box harness run by the maintainer before merging.

## Remaining risks
- The dispatcher's arm order is now load-bearing in the same way the
  pre-check order was; a new arm for an existing variant must go after the
  guarded pre-check arms.
- `handle_join` takes ten parameters (allowed by the workspace lint set);
  a `JoinRequest` struct would read better but would change the moved body.
