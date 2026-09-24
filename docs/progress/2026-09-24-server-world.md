# 2026-09-24 — Server `GameWorld` and module split

## Goal
Roadmap step 5: `server/src/main.rs` held wire glue, `ServerRuntime` (41
fields), the packet dispatcher, the tick, snapshot building, the
projectile/minion/neutral/tower simulation and 50 inline tests (~6000 lines).
Free functions took 5-12 `&mut HashMap` parameters and the dispatcher and
tick destructured `self` to split borrows.

## Changes
- `GameWorld` (`server/src/game_world.rs`): players, disconnected sessions,
  projectiles, structures, minions, neutrals, team buffs, forest pickups,
  game state, map layout/config, id allocators, wave clock. `ServerRuntime`
  holds it as `world`; `GameWorld::new(map_config, now)` builds the fresh
  round state and `#[cfg(test)] GameWorld::empty()` is the unit-test fixture.
- `TickCtx { now, dt }` replaces the `(dt, now)` argument pairs.
- Converted to `&mut GameWorld`: `simulate_projectiles`, `simulate_minions`,
  `simulate_tower_attacks`, `simulate_neutrals`, `spawn_minion_waves_if_due`,
  `handle_respawns`, `regenerate_base_hp`, `regenerate_team_buff_hp`,
  `restore_god_mode_players`, `handle_cast_request`,
  `handle_basic_attack_request`, `start_match_running`,
  `advance_formation_on_join`, `tick_match_formation`; `vision::{sources,
  target_visible, filter_snapshot}` take `&GameWorld`. `GameWorld` methods
  `ensure_connected`, `ensure_player_for_join`, `reset_round` replace the
  session free functions (the `cfg(test)` `reset_match` wrapper is gone).
  Leaf damage helpers keep field-level parameters.
- Module split (verbatim moves, `pub(crate)` added, crate-root
  `pub(crate) use module::*` keeps the glob-importing modules compiling):
  `entities`, `formation`, `snapshot` (+ `ServerRuntime::broadcast_snapshots`
  with recipients collected before the per-recipient combat-log drain),
  `ecs`, `runtime/{mod,dispatch,tick}`, `sim/{mod,cast,projectiles,minions,
  neutrals,towers}`, `tests/{mod,snapshot,movement,neutrals,minions,
  progression,sessions,cast,bosses,formation}`. `main.rs` is 100 lines.
- Tests that built loose `HashMap`s now use `GameWorld::empty()`; tests that
  passed a literal `GameState` set `world.game_state` first.

## Checks
- `cargo test -p server`: 274 passed, 3 ignored (unchanged).
- `cargo clippy -p server --all-targets --no-deps -- -D warnings` and the
  workspace clippy are clean; `cargo test -p harness --no-run` compiles.

## Left for the next PR
- Split `handle_packet_authorized` into per-command handlers.
- Replace the crate-root glob re-exports with explicit imports.
