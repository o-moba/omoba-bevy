# Feature Inventory

Canonical version: `0.2.0`

## Current Playable Surface

- Authoritative UDP server loop with periodic player snapshots.
- Local multiplayer flow with server startup plus multi-client local play via `make start`.
- Team join flow with character selection and player spawning.
- Core combat loop with projectiles, structures, minions, death, respawn, mana regeneration, and base-destruction win condition.
- Map layout with three lanes and simple jungle blocks.
- Player progression with level-based XP thresholds, HP/mana scaling on level-up, and tracked skill points.
- In-game local HUD display for level and XP progression.
- **Skill 1 (melee)**: `Q` or the in-game skill button casts on the current TAB/middle-click target. Server validates horizontal range, mana, cooldown, and target type (enemy players, enemy minions, neutrals; not structures or allies). Damage and cooldown scale with melee rank; spend skill points with `1` when available. Failed local checks show messages in the debug console without sending a cast.

## Multiplayer Session Reliability

- Join is authoritative on the server: the client may optimistically pick a team and character, but the snapshot for `your_id` is the source of truth for spawn side, team, and character.
- Repeated `Join` packets from the same UDP endpoint are deterministic: the last processed `Join` wins for team, character, spawn position, HP, mana, gold, and XP reset.
- If a client stops sending packets, the server removes that player after `PLAYER_TIMEOUT = 5s`; remaining clients stop receiving that player in snapshots after the timeout expires.
- Reconnect policy for this version: a new UDP endpoint is treated as a new player identity. Slot reclaim is not implemented.
- If the server restarts while clients stay open, the next packet from an existing client creates a fresh default session on the restarted server. Team and character return to defaults until that client sends `Join` again.
- The client applies only the latest queued snapshot per frame. This "last snapshot wins for the current frame" behavior is intentional for now and validated by the session-flow checks.

## Release Gaps Tracked In Tasks

- Runtime and startup stability hardening.
- Full reconnect slot reclaim across disconnects and NAT changes.
- Full skill system (four distinct server-validated abilities with per-rank tuning), tooltip UX.

## Release gate and balance (TASK-12)

- Authoritative tuning constants: `server/src/balance.rs` (see `docs/balance-tuning.md`).
- Release checklist, manual QA matrix, and readiness report: `docs/release-gate-checklist.md`, `docs/manual-qa-matrix.md`, `docs/release-readiness-report.md`.
- Live UDP QA smoke (two clients + cast): `make verify-task-12` or `python3 scripts/verify_task_12_qa_matrix_live_udp.py` (after `cargo build -p server`).
