# 2026-07-06 — Bot Lane-Push AI (TASK-23)

## Goal

TASK-22 fill bots only wandered at spawn. Needed: believable play — bots
push their lanes toward the enemy tower, fight what they meet, so a solo
developer's test match looks and feels like a real game.

## Changes

- New `harness/src/bot_ai.rs`: pure, unit-tested bot brain.
  - Lane waypoints (Mid/Top/Bot) mirroring the server's
    `lane_control_points` geometry, oriented by team (Green home→away,
    Blue reversed).
  - Per-tick decision: nearest enemy unit (player/minion) within
    `Q range × 0.9 + 9.0` pulls the bot; inside cast range it holds and
    casts Q; otherwise it approaches. With no units, same rule for enemy
    structures (tower sieging). Otherwise follow waypoints
    (2.0-unit reach radius, hold at the enemy base).
  - Q cast ranges from the shared per-class ability kits
    (`shared::ability_for_class_slot`); harness now depends on `shared`.
  - `resync` re-aims at the nearest waypoint after a respawn.
  - `step_toward` produces server-legal movement steps
    (`PLAYER_SPEED × dt`) with the TASK-21 facing convention.
- Harness protocol mirror: snapshots now model `minions` and `structures`
  (id/team/x/z/hp) for targeting.
- `bots` binary rewritten around per-bot runners: pre-match queue wander
  kept; during `Running` alive bots are brain-driven (10 Hz, ~0.7 s client
  cast throttle); dead bots wait for the server respawn and resync; on
  Victory brains reset for the auto-rematch.
- Docs: RUNBOOK/README/Makefile bot descriptions updated; features.md;
  version 0.10.0 → 0.11.0.

## Checks

- 9 new unit tests in `bot_ai` (lane orientation for both teams and all
  lanes, waypoint follow/advance/hold, approach vs in-range cast, unit
  priority over structures, tower siege, respawn resync, speed budget +
  facing, snapshot enemy filtering).
- New live integration test `brain_driven_bot_pushes_its_lane_on_a_live_server`:
  real dev-mode server, brain-driven bot, asserts ≥20 units of
  server-accepted progress toward the enemy base (12 s budget).
- Live smoke: release 1v1 (`OMOBA_TEAM_SIZE=1`) with two AI bots — queue →
  countdown → `match running - bots pushing lanes`, no errors in either
  log over 35 s.
- `cargo check/clippy --workspace` clean; `cargo test --workspace` green
  (133 tests).

## Remaining limitations

- Nearest-target only: no focus fire, no retreat/heal, no W/E/R usage, no
  jungle/boss play, no difficulty tuning.
- Bots do not path around the jungle block colliders (server movement
  clamp keeps them legal; they can rub along obstacles on Top/Bot bends).
- Lane geometry is a mirrored formula: if the server map constants change,
  update `harness/src/bot_ai.rs` alongside (documented in the module).
