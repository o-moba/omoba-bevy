# Progress: TASK-12 — Balance, QA, release gate

**Date:** 2026-04-01  
**Worktree:** `omoba-bevy-TASK-12`

## Goal

Move toward TASK-12 acceptance: centralized tuning, documented release gate and QA matrix, evidence bundle refresh.

## Changes

- `server/src/balance.rs`: full tuning surface + `PRIMARY_ABILITY_DAMAGE_BY_RANK`, `SKILL_SLOT_COUNT`, unit tests (with clippy-friendly allows).
- `server/src/main.rs`: `use balance::*`, projectile damage from rank table index 0; `grant_player_xp` for neutral kills; `cast_drains_mana_*` test; crate-level `items_after_test_module` allow; `simulate_projectiles` arity allow.
- `scripts/verify_task_12_qa_matrix_live_udp.py`, `make verify-task-12`: live UDP M1/M3 evidence (fresh server per scenario).
- Docs refreshed: QA matrix run log, release checklist (P1), readiness report, `balance-tuning.md`, `features.md`, `CHANGELOG.md`.

## Checks run

- `cargo fmt --all`
- `cargo test --workspace` (all tests green)
- `cargo clippy -p server --all-targets -- -D warnings` (green)
- `cargo clippy --workspace --all-targets -- -D warnings` (**fails** on `client` with many pre-existing pedantic lints; not introduced by TASK-12)
- `python3 scripts/verify_task_12_qa_matrix_live_udp.py` after `cargo build -p server` (**PASS** M1 + M3)

## Risks / follow-ups

- Matrix **M2** (victory/rematch) and Bevy UI slices of **P3** remain **PARTIAL** until human `make start` or new automation.
- Full-workspace `clippy -D warnings` needs a dedicated client cleanup pass unrelated to TASK-12.
