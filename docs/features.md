# Feature Inventory

Canonical version: `0.2.0`

## Current Playable Surface

- Authoritative UDP server loop with periodic player snapshots.
- Local multiplayer flow with server startup plus multi-client local play via `make start`.
- Team join flow with character selection and player spawning.
- Core combat loop with projectiles, structures, minions, death, respawn, mana regeneration, and base-destruction win condition.
- Map layout with three lanes and simple jungle blocks.
- Player progression with level-based XP thresholds, HP/mana scaling on level-up, and tracked skill points.
- In-game match HUD (below minimap): level, XP, skill points, reserved upgrade key label (`U`), local HP/mana, target hints, objective line, and F1 help reminder; bottom-right skill bar showing `Q`–`R` keys (same cast binding until per-skill server support).
- F1 toggle help overlay with movement, camera, targeting, casting, objective, and pause guidance; does not reset simulation when toggled. The panel is shown only while the match is `Running` (toggle state is preserved when returning to a live match so lobby/victory screens are not covered).

## Multiplayer Session Reliability

- **TASK-14 (client)**: Explicit session states, non-blocking wait when the server is down, bounded `WaitingForServer` timeout, stale snapshot detection while connected, UDP transport error thresholds, snapshot-channel disconnect detection when the UDP thread ends, full teardown (replicated entities + team overlay) on disconnect, manual reconnect via **Retry** (no silent rejoin into a match), pause menu auto-closes on **Disconnected**, minimap hidden unless **Connected**.
- Named timing constants and failure-detection summary: `docs/network-client-session.md` and `client/src/session_config.rs`.
- Join is authoritative on the server: the client may optimistically pick a team and character, but the snapshot for `your_id` is the source of truth for spawn side, team, and character.
- Repeated `Join` packets from the same UDP endpoint are deterministic: the last processed `Join` wins for team, character, spawn position, HP, mana, gold, and XP reset.
- If a client stops sending packets, the server removes that player after `PLAYER_TIMEOUT = 5s`; remaining clients stop receiving that player in snapshots after the timeout expires.
- Reconnect policy for this version: a new UDP endpoint is treated as a new player identity. Slot reclaim is not implemented.
- If the server restarts while clients stay open, the next packet from an existing client creates a fresh default session on the restarted server. Team and character return to defaults until that client sends `Join` again.
- The client applies only the latest queued snapshot per frame. This "last snapshot wins for the current frame" behavior is intentional for now and validated by the session-flow checks.

## Release Gaps Tracked In Tasks

- Runtime and startup stability hardening.
- Full reconnect slot reclaim across disconnects and NAT changes.
- Expanded skill roster, tooltip UX, balance passes, and release-scale QA beyond the current cast-and-HUD surface.

## Operations and playtest documentation

- [README.md](../README.md) — setup, controls summary, links to tester docs.
- [RUNBOOK.md](../RUNBOOK.md) — startup, env vars, troubleshooting with recovery steps.
- [docs/playtest-script.md](playtest-script.md) — timeboxed MVP session checklist.
- [docs/bug-report-template.md](bug-report-template.md) — internal report format.
- [docs/mvp-scope-and-limitations.md](mvp-scope-and-limitations.md) — explicit MVP scope and limitations.
- [tasks/MVP-CHECKLIST.md](../tasks/MVP-CHECKLIST.md) — MVP-blocking vs deferrable classification.
