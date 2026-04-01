# Session: TASK-04 jungle camps (builder)

## Goal

Implement TASK-04-jungle-camps-and-neutral-entities per frozen spec (AC1–AC7).

## Changes

- Server: neutral templates, camp spawns, AI/leash/respawn/rewards, projectile damage path, snapshot `neutrals` field, `TargetKind::Neutral`.
- Client: serde-aligned types, snapshot apply/spawn/update, shared net interpolation for minions and neutrals, combat targeting and HP bars for neutrals.

## Checks

- `cargo build --workspace` (succeeded).
- `cargo test -p server` (succeeded).

## Risks / follow-up

- Spitter camp uses longer server attack range (ranged) without a separate visible neutral projectile; behavior matches “projectile vs melee” differentiation at simulation level only.
- AC7 (minion/tower regression) not exercised by automated tests here; manual two-client smoke still recommended for verifier.
