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
- Persistent local client preferences (graphics, character, optional server address) with safe clamping on load; override directory with `OMOBA_CLIENT_CONFIG_DIR` for tests or portable installs.

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
- Match phase, restart, and rematch flow.
- Jungle camps and neutral AI.
- Full skill system, tooltip UX, and release-readiness validation.
