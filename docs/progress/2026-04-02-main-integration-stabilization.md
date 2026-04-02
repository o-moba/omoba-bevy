# 2026-04-02 — Main integration stabilization

## Goal

Restore a runnable and testable `main` branch after multi-PR merge conflicts introduced compile-time breakage in core gameplay files (`client/src/net.rs`, `server/src/main.rs`) and verify that recently added gameplay/session features still work end-to-end.

## What changed

- Rebased `main` working tree to a stable integrated snapshot built from verified task commits (`TASK-12` baseline + `TASK-13` + `TASK-14` + `TASK-15` + safe `TASK-16` debug/observability additions).
- Resolved merge-induced code corruption in:
  - `client/src/net.rs` (snapshot/session pipeline coherence)
  - `server/src/main.rs` (authoritative gameplay loop integrity)
  - `client/src/combat.rs` (cast pipeline consistency)
- Preserved and validated HUD/onboarding/session reliability modules:
  - `client/src/help_overlay.rs`
  - `client/src/match_hud.rs`
  - `client/src/input_bindings.rs`
  - `client/src/session_config.rs`
  - `client/src/persistence.rs`
- Kept documentation and release-surface files aligned:
  - `CHANGELOG.md`
  - `docs/features.md`
- Cleaned one dead-code warning source in `client/src/debug_console.rs` by removing an unused helper.

## Verification

- `cargo check --workspace` — PASS
- `cargo test --workspace` — PASS
- `python3 scripts/verify_task_12_qa_matrix_live_udp.py` — PASS (`M1`, `M2`, `M3`)
- `cargo fmt --all --check` — PASS

## Remaining notes

- This stabilization prioritizes a compilable, test-backed, and gameplay-verified `main` state over preserving every conflict-heavy line from the previously merged PR graph.
- The four existing untracked progress files from 2026-04-01 were left untouched.
