# 2026-04-02 - Bevy ECS combat damage slice

## Goal

Move one combat path (`projectile -> minion` damage) from monolithic `simulate_projectiles` flow into a Bevy ECS/message pipeline while keeping the existing authoritative `ServerRuntime` maps and reward logic intact.

## What changed

- Added `server/src/gameplay/mod.rs` with `GameplayPlugin` and `server/src/gameplay/combat.rs` with `CombatPlugin`.
- Registered a message-driven damage lane:
  - `DamageEvent` (`#[derive(Message)]`)
  - `collect_projectile_minion_damage_system` (collision detection + message enqueue)
  - `apply_projectile_minion_damage_system` (authoritative damage application through existing `apply_minion_damage`)
- Added ECS minion mirror state for the combat slice:
  - `CombatMinion` component
  - `EcsMinionEntities` resource (`minion_id -> Entity`)
  - `sync_minions_into_ecs_system` for spawn/update/despawn synchronization from runtime maps.
- Wired the new systems into the server update chain before `server_finalize_tick_system`.
- Removed legacy `TargetKind::Minion` hit processing from `simulate_projectiles` to avoid duplicate damage paths.

## Verification

- `cargo test -p server` - PASS (17 passed, 0 failed)
- `ReadLints` on `server/src/main.rs` and `server/src/gameplay/` - PASS (no diagnostics)

## Remaining risks / next slice

- This slice is still bridge-based: runtime `HashMap` state remains authoritative, and ECS mirrors are synchronized each tick.
- Damage application is message-driven, but kill/death side effects still execute through legacy runtime helpers.
- Next high-value step is to migrate projectile entities fully into ECS ownership and mirror out only snapshot state.
