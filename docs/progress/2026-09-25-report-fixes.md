# 2026-09-25 — Report fixes (O3, O11, O7, O16, O13, O17, O29, O25)

Follow-ups from [ARCHITECTURE_REPORT.md](../ARCHITECTURE_REPORT.md) §8.2,
each in the report's scoped variant. Every problem was re-checked on
217ef9f before the fix (the report's line numbers were measured at
4efbd8b/5a1178d and had drifted); each fix has a test.

## O3: time-based movement budget (behaviour change)

- Found: `hero_stats::movement_envelope` returned `speed × elapsed +
  MOVEMENT_POSITION_TOLERANCE` for every `Transform`. Reproduced with the new
  flood test on the old code: 120 transforms in one second moved the hero
  17 units (ceiling 5.1).
- Fix: `HeroTimers::movement_slack` (starts at the tolerance, reset in
  `reset_player_round` and on both session reclaims). The envelope is
  `speed × elapsed + slack`; after the step
  `slack = (envelope − moved).clamp(0, tolerance)`. A single transform from a
  fresh budget is accepted bit-identically, so the existing single-packet
  tests are unchanged. `docs/ARCHITECTURE.md` protocol rules: which commands
  carry `request_id` (`Cast` does not, cooldown/recovery/mana refuse a
  replay) and the movement budget.
- Tests (`server/src/tests/movement.rs`, `ServerRuntime::for_test` on a
  `ManualClock` and the memory transport):
  `movement_budget_caps_a_packet_flood_at_normal_speed` (fails before: 17 vs
  5.1) and `movement_budget_leaves_a_normal_20hz_client_unclipped` (20 sends
  with 35–65 ms arrival gaps, every step accepted exactly).

## O11: career portrait for store avatars (player-visible)

- Found: `career.rs` `portrait` loaded `avatars/{thumbnail}` directly.
- Fix: `avatar_portrait_path(avatar, thumbnail)` uses
  `passport::thumbnail_asset_path`; the passport function is split into
  `thumbnail_asset_path_in(avatar, in_store)` so the path choice is testable
  without a store runtime.
- Test: `career::tests::career_portrait_loads_store_avatars_from_the_ekza_source`.
  Not checked visually (no GPU here).

## O7: settings rewrite on every prematch snapshot (player-visible)

- Found: `apply_snapshot_resources` (`client/src/net/apply.rs`) assigned the
  draft row to `TeamSelection` unconditionally, and
  `save_client_preferences_on_change` rewrote the file on every change with
  `fs::write`.
- Fix: compare first, write only on a difference; `write_atomically`
  (temp file, `sync_all`, rename, temp removed on error).
- Tests: `net::apply::tests::repeated_draft_snapshot_leaves_team_selection_unchanged`
  (fails before: the change tick moves on every snapshot) and
  `persistence::tests::preferences_are_written_atomically`.

## O16: offline practice formulas (player-visible)

- Found in `client/src/net/offline.rs`: `hero()` gave "Level 6" heroes the
  class base HP and 100 mana; self heal and mana restore were unscaled;
  mana regenerated for dead heroes; the local cast and the duelist's casts
  recorded `Cast` for Q (the server records `Attack`); a dead local hero
  could cast.
- Fix: `max_hp_for_level` and new `shared::hero_balance::max_mana_for_level`
  for the pools (the duelist's mana by its own level); the server's effect
  scale (`rank_effect_scale × ability_power_multiplier`) for heals, restores
  and damage, `scaled_mana_cost` for the cost; no regen at 0 HP; the
  `hp > 0` cast gate; `PlayerActionKind::for_cast(slot)` in `shared`, used by
  the server's `record_player_action` and both offline cast paths. Harness
  bots untouched.
- Test: `net::offline::tests::offline_formulas_match_the_server` (fails
  before on the pools).

## O13: one hostile-target resolver

- Found: `sim/cast.rs` repeated `basic_attack::resolve_hostile_target`'s four
  arms verbatim (same rules).
- Fix: the cast calls the resolver. `sim/projectiles.rs`'s `joined` check is
  unchanged.
- Test: `tests::cast::cast_on_a_protected_tower_is_rejected` (pins the rule
  through the shared resolver; it also passes on the old copy, which had the
  same check).

## O17: round lifecycle out of `record_match_metrics`

- Found: `record_match_metrics` finalized a won round as Completed and set
  `victory_at`; `restart_round` always finalized as Abandoned, so a restart
  in Victory before a tick's metrics pass recorded Abandoned.
- Fix: `settle_finished_round` (called by the tick after
  `record_match_metrics`, by `advance_career_queue` and `career_play_again`
  where the old side effect was relied on); `restart_round` picks Completed
  (with the winner) or Abandoned from the game state. Career tests that
  called `record_match_metrics` to settle now call `settle_finished_round`.
- Tests (`practice_tests.rs`):
  `restart_round_settles_a_won_round_as_completed_and_a_running_one_as_abandoned`
  (fails before: Abandoned) and `match_metrics_do_not_settle_the_round`.

## O29: pure outbox recovery

- Fix: `recover_outbox(&Path) -> RecoveredOutbox { pending, rejected_ids }`,
  the worker prologue moved verbatim; the worker loop is unchanged.
- Tests: `career_backend::tests::recover_outbox_restores_terminal_interrupts_live_and_skips_junk`
  and `recover_outbox_keeps_an_existing_recovery_allocation`.

## O25: lock-in decision

- Fix: `team::lock_in(team, selection, join_in_flight, connection, sandbox,
  offline) -> LockIn { Ignore | Reconnect | Join { command, screen } }`; the
  button system logs, writes and switches screens from it. No file move.
- Test: `team::tests::lock_in_ignores_in_flight_reconnects_dead_and_routes_by_mode`.

## Gate

`cargo fmt --all -- --check`, both clippy runs, `cargo test --workspace
--locked --exclude harness`, `cargo build -p server && cargo test --locked -p
harness -- --test-threads=1` and the Python script tests pass. Counts: server
285 → 292 (+3 ignored), client lib 562 → 567 (542 → 547 without `qa`),
shared 94, harness 22 + 24, Python 96 (1 skipped).
