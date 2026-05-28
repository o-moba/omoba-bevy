# Feature Inventory

Canonical version: `0.2.0`

## Current Playable Surface

- Headless Bevy-scheduled authoritative UDP loop with periodic player snapshots; player mana regeneration and `projectile -> minion` damage now run through ECS/message-driven systems bridged to the current authoritative state maps.
- Server-authoritative hardening for player movement and casts: client transforms are speed/map clamped, non-finite positions are ignored, and casts require the authoritative caster position to be in range of the live target.
- Local multiplayer flow with server startup plus multi-client local play via `make start`.
- Team join flow with character selection and player spawning.
- Ekza-Stellar SDK extraction: shared `ekza-stellar-sdk` workspace crate owns stable character ids, built-in 3D model manifest metadata, GLB validation, and Bevy model catalog loading for future dependency publishing.
- Core combat loop with projectiles, structures, minions, death, respawn, mana regeneration, and base-destruction win condition.
- Map layout with three lanes and simple jungle blocks.
- Player progression with level-based XP thresholds, HP/mana scaling on level-up, and tracked skill points.
- In-game local HUD display for level and XP progression.
- Persistent local client preferences (graphics, character, optional server address, stable client session id) with safe clamping on load; override directory with `OMOBA_CLIENT_CONFIG_DIR` for tests or portable installs.
- In-game match HUD (below minimap): level, XP, skill points, reserved upgrade key label (`U`), local HP/mana, target hints, objective line, and F1 help reminder; bottom-right skill bar showing `Q`–`R` keys (same cast binding until per-skill server support).
- F1 toggle help overlay with movement, camera, targeting, casting, objective, and pause guidance; does not reset simulation when toggled. The panel is shown only while the match is `Running` (toggle state is preserved when returning to a live match so lobby/victory screens are not covered).

## Multiplayer Session Reliability

- **TASK-14 (client)**: Explicit session states, non-blocking wait when the server is down, bounded `WaitingForServer` timeout, stale snapshot detection while connected, UDP transport error thresholds, snapshot-channel disconnect detection when the UDP thread ends, full teardown (replicated entities + team overlay) on disconnect, manual reconnect via **Retry** (no silent rejoin into a match), pause menu auto-closes on **Disconnected**, minimap hidden unless **Connected**.
- Named timing constants and failure-detection summary: `docs/network-client-session.md` and `client/src/session_config.rs`.
- Join is authoritative on the server: the client may optimistically pick a team and character, but the snapshot for `your_id` is the source of truth for spawn side, team, and character.
- Repeated `Join` packets from the same UDP endpoint are deterministic: the last processed `Join` wins for team, character, spawn position, HP, mana, gold, and XP reset.
- If a client stops sending packets, the server removes that player after `PLAYER_TIMEOUT = 5s`; remaining clients stop receiving that player in snapshots after the timeout expires.
- Reconnect policy for this version: clients that send a valid stable `client_session_id` in `Join` can reclaim a timed-out player slot/id for a short server-side window. Legacy clients without a session id still use endpoint identity and reconnect as a new player.
- If the server restarts while clients stay open, the next packet from an existing client creates a fresh default session on the restarted server. Team and character return to defaults until that client sends `Join` again.
- The client applies only the latest queued snapshot per frame. This "last snapshot wins for the current frame" behavior is intentional for now and validated by the session-flow checks.

## Release Gaps Tracked In Tasks

- Runtime and startup stability hardening.
- Account-backed identity, cryptographic session authentication, and long-lived reconnect across server restarts.
- Publishable SDK packaging: registry metadata, versioning policy, examples, entitlement/auth hooks, and non-blocking asset delivery are still future work.
- Full reconnect slot reclaim across disconnects and NAT changes.
- Full skill system (four distinct server-validated abilities with per-rank tuning), tooltip UX.

## Release gate and balance (TASK-12)

- Authoritative tuning constants: `server/src/balance.rs` (see `docs/balance-tuning.md`).
- Release checklist, manual QA matrix, and readiness report: `docs/release-gate-checklist.md`, `docs/manual-qa-matrix.md`, `docs/release-readiness-report.md`.
- Live UDP QA smoke (two clients + cast): `make verify-task-12` or `python3 scripts/verify_task_12_qa_matrix_live_udp.py` (after `cargo build -p server`).
- Expanded skill roster, tooltip UX, balance passes, and release-scale QA beyond the current cast-and-HUD surface.

## Operations and playtest documentation

- [README.md](../README.md) — setup, controls summary, links to tester docs.
- [RUNBOOK.md](../RUNBOOK.md) — startup, env vars, troubleshooting with recovery steps.
- [docs/playtest-script.md](playtest-script.md) — timeboxed MVP session checklist.
- [docs/bug-report-template.md](bug-report-template.md) — internal report format.
- [docs/mvp-scope-and-limitations.md](mvp-scope-and-limitations.md) — explicit MVP scope and limitations.
- [tasks/MVP-CHECKLIST.md](../tasks/MVP-CHECKLIST.md) — MVP-blocking vs deferrable classification.
