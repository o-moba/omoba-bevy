# Shared balance and facing conventions — 2026-09-24

Roadmap step 3 (`docs/ARCHITECTURE.md`). Tuning numbers that the server and
client both depend on were declared separately in `server/src/balance.rs`,
`client/src/player.rs`, `client/src/combat.rs`, `client/src/net/offline.rs`,
`client/src/minimap.rs` and `harness/src/bot_ai.rs`, and had already drifted:
the offline playground regenerated mana at 12/s against the server's 8/s, its
projectiles flew at 30 against 19, and the legacy snapshot default for the
first level threshold was 120 against the server's 90.

## Changes

- `shared::hero_balance` gains `PLAYER_SPEED`, `DEBUG_SPEED_MULTIPLIER`,
  `DEFAULT_MAX_HP`, `MAX_MANA`, `MANA_REGEN_PER_SECOND`, `PROJECTILE_SPEED`,
  `RESPAWN_DELAY_SECS`, `STARTING_LEVEL`, `LEVEL_UP_HP_BONUS`,
  `LEVEL_UP_MANA_BONUS`, `LEVEL_XP_THRESHOLDS`, `BOT_ENGAGE_RANGE`,
  `xp_threshold_for_level` and `max_hp_for_level`. The server re-exports them
  from `balance.rs`, so call sites are unchanged; the client, offline
  simulation, minimap (sight radii from `shared::vision`) and harness import
  them instead of redeclaring.
- `shared::math` documents the two facing conventions and provides
  `hero_yaw_towards` / `unit_yaw_towards` plus their forward vectors with a
  test that mixing them is exactly a half turn. Bots, the local player, the
  strike facing, the offline simulation, minions and neutrals use it. The
  Combat Test actor was the one hero using the minion convention and faced
  backwards while moving.
- The server's receive budget and snapshot payload limit use the constants
  `shared::public_transport` and `shared::transport` already exported.

## Verification

`make check` (format, clippy with warnings as errors, shared/server/client
suites) and `cargo test -p harness --no-run`.
