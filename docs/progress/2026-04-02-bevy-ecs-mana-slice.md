# 2026-04-02 - Bevy ECS mana slice

## Goal

Start the server migration from the monolithic tick loop toward Bevy ECS by introducing a headless Bevy scheduler and moving one gameplay rule (mana regeneration) into ECS systems without breaking the current UDP gameplay surface.

## What changed

- Added `bevy = { version = "0.18.0", default-features = false }` to `server/Cargo.toml` for headless server scheduling and ECS usage.
- Replaced the manual `loop { ... thread::sleep(...) }` entrypoint in `server/src/main.rs` with a Bevy `App` configured with:
  - `MinimalPlugins`
  - `ScheduleRunnerPlugin::run_loop(SIMULATION_STEP_SLEEP)`
  - ordered `Update` systems chained per tick.
- Introduced a `ServerRuntime` Bevy resource that owns the authoritative runtime state previously held by `main()` locals (socket, players, projectiles, structures, minions, neutrals, IDs, and timing markers).
- Split the old loop body into resource methods:
  - `receive_packets()`
  - `prepare_tick()`
  - `simulate_after_mana(...)`
- Added initial ECS components/resources for the first migration slice:
  - components: `Player`, `Transform3D`, `Health`, `Mana`, `TeamMarker`
  - resources: `EcsPlayerEntities`, `SimulationDeltaSeconds`, `TickContext`
- Implemented ECS mana pipeline systems:
  - `sync_players_into_ecs_system`
  - `regenerate_mana_system`
  - `sync_players_from_ecs_system`
- Kept the rest of gameplay simulation (projectiles/minions/towers/neutrals/snapshots) behaviorally intact by running it after ECS mana sync in `server_finalize_tick_system`.

## Verification

- `cargo fmt --all` - PASS
- `cargo test -p server` - PASS (17 passed, 0 failed)

## Remaining risks / next slice

- ECS entities are currently synchronized from the legacy `HashMap` player store each tick; this is transitional and not yet a full data ownership transfer.
- Command-buffered spawn/despawn in the sync system can defer structural updates by one frame for newly connected/disconnected players (acceptable for this migration stage).
- Next migration step should move network packet handling to Bevy events and then progressively port combat/minion/neutral logic into ECS-native systems.
