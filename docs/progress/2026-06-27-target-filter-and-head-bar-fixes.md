# 2026-06-27 — Tab target filter + deterministic head bar

## Goal
Fix two gameplay/UX bugs reported during playtest, tracked in `tasks/task01.md`
(head bar) and `tasks/task02.md` (target selection).

## Changes
- **TASK02 — target selection** (`client/src/combat.rs`): `find_nearest_enemy_target`
  (Tab) skipped the team check for minions, so friendly minions — always nearer
  than the enemy base — were selected and the tower was unreachable. Minion
  candidates now skip `*team == local_team`, matching players/structures. Same
  fix applied to `find_target_near_point` (middle-click) for consistency.
- **TASK01 — head bar** (`client/src/world.rs`, `client/src/combat.rs`):
  `normalize_model_scale_system` now records a pivot-independent
  `NormalizeModelScale.head_local_y` (model top above origin, re-scaled to the
  normalized size). `sync_combat_bar_transforms_system` anchors player bars to
  that value instead of per-frame AABB + `MIN_PLAYER_BAR_Y` clamp, which drifted
  to mid-body for rigged/center-pivot models (`wang`, `toka`). Structures,
  minions, and neutrals keep the AABB path.

## Checks
- `cargo build -p client` — pass.
- `cargo test -p client` — 25 passed.

## Remaining risks
- Both fixes need a live visual pass (`make game`): confirm Tab grabs the enemy
  tower near friendly minions, and the bar sits over the head for all 4 models.
- Optional follow-up: ground-align models (feet on y=0) via a holder node; would
  also let the bar use a constant offset. Deferred — see `tasks/task01.md`.
