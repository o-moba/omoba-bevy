# 2026-05-28 - Production authority hardening

## Goal

Close the highest-risk server authority gaps before treating the current multiplayer slice as production-ready: client-trusted movement, cast validation surface, and UDP-endpoint-only player identity.

## What changed

- Server `Transform` packets are now treated as requested corrections and clamped against authoritative movement speed, elapsed time, tolerance, and map bounds.
- Server cast validation now checks the live authoritative caster position against configured range for players, minions, structures, and neutrals before mana, cooldown, or projectile side effects.
- Join packets can carry an optional stable `session_id`; the client persists it in preferences, and the server can reclaim a timed-out player slot/id from a new UDP endpoint within the reclaim window.
- Legacy clients without a `session_id` still use endpoint identity and reconnect as a new player after timeout.
- Live UDP QA movement paths were updated to drive clients through legal server-authoritative movement instead of teleporting them into combat positions.

## Verification

- `cargo fmt --all -- --check` - PASS
- `cargo test --workspace` - PASS (25 client tests, 24 server tests, 5 skills tests)
- `cargo build --workspace` - PASS
- `python3 scripts/verify_task_02_multiplayer_session_flow.py` - PASS
- `make verify-task-12` - PASS (M1, M2, M3)

## Remaining risks / next slice

- Stable `session_id` is a reconnect token, not account auth or a cryptographic anti-cheat secret.
- Server movement is speed-clamped, but there is still no full input simulation, collision/navigation validation, or lag-compensated reconciliation.
- Long-lived identity across server restart and hosted production deployment remain out of scope for this slice.
