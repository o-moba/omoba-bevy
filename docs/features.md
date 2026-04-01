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

## Release Gaps Tracked In Tasks

- Runtime and startup stability hardening.
- Multiplayer session reliability and reconnect behavior.
- Match phase, restart, and rematch flow.
- Jungle camps and neutral AI.
- Full skill system, tooltip UX, and release-readiness validation.
