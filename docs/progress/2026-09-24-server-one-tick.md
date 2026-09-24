# 2026-09-24 — Server: one tick, no ECS mirror

## Goal
Roadmap step 6, first slice. The server ran a Bevy `App` whose only job was
to copy players and minions into ECS components each tick, regenerate mana
and fly minion-targeted projectiles there, and copy the results back. Every
other system read the `GameWorld` maps directly, so the mirror was a second
copy of the state, a second mana regeneration and a second projectile path.

## Changes
- `sim::regenerate_mana(players, dt)`: joined, living heroes, `dt > 0`,
  pool defaulted to `MAX_MANA` when unset, clamped. Same rule the ECS
  system applied; the `cfg(test)` copy in `session.rs` is gone.
- `sim::projectiles::simulate_projectiles_filtered(world, tick, targets)`:
  one `retain` over the projectiles, one `step_homing(projectile,
  target_pos, target_radius, dt) -> bool` helper for the homing, sweep and
  contact test that was written four times, and a `TargetKind::Minion` arm
  (aim point `y + MINION_RADIUS * 0.8`, hit radius `MINION_RADIUS`, damage
  through `apply_minion_damage`). `simulate_projectiles` (all kinds) is the
  `cfg(test)` wrapper.
- `ServerRuntime::tick(now, dt)` replaces `simulate_after_mana`: mana
  regeneration, the minion projectile pass (skipped on `dt <= 0`, as the
  ECS collect system did), then the previous body with the non-minion
  projectile pass at its old place.
- `runtime::run` is a loop: `prepare_tick`, `tick`, sleep
  `SIMULATION_STEP_SLEEP - elapsed` when the step finished early (what
  `ScheduleRunnerPlugin::run_loop` did).
- Deleted: `server/src/ecs.rs`, `server/src/gameplay/{mod,combat}.rs`, the
  five mirror components in `entities.rs`, `#[derive(Resource)]` on
  `ServerRuntime`, the `bevy` imports in `main.rs`.
- Tests: the two ECS-only tests are rewritten against
  `simulate_projectiles` (`projectile_minion_receipts_preserve_nonplayer_identity_and_overkill_once`,
  `minion_projectiles_do_not_land_after_terminal_and_cannot_leak_into_next_round`);
  `mana_regenerates_and_is_clamped` marks its player joined; 42 call sites
  of `simulate_after_mana` are `tick`.

## Behaviour
- Wire format and snapshot bytes unchanged.
- Minion impacts in one tick are applied in projectile id order (the ECS
  path applied them in hash order).
- Test loops that call `ServerRuntime::tick` now also resolve
  minion-targeted projectiles; no expectation changed.

## Checks
- `cargo test -p server`: 274 passed, 3 ignored (unchanged count; two tests
  replaced one for one).
- `cargo fmt --all`, `cargo clippy --workspace --all-targets --no-deps -- -D warnings`,
  `cargo test -p shared -p server`, `cargo test -p harness --no-run`,
  `cargo test -p client --lib` (543 passed) are green.

## Left for the next slices
- Fold the two filtered projectile passes into one call once the minion
  pass may move to the projectile step.
- The `bevy` entry in `server/Cargo.toml` is now unused by the server's own
  code (removing it touches `Cargo.lock`, left out of this change).
- Hero views, the `ConnectedPlayer` struct split, `StatModifiers`, snapshot
  redaction.
