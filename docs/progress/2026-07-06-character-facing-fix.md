# 2026-07-06 — Character Facing Fix

## Goal

Characters walked backwards: whatever direction the player clicked, the model
faced 180° away from its heading.

## Changes

- `client/src/player.rs::move_player`: local walk yaw was
  `direction.x.atan2(direction.z)`, which aligns the entity's +Z axis with the
  movement direction — but character models face -Z (Bevy forward), so every
  model rendered back-first. Now `(-direction.x).atan2(-direction.z)` points
  -Z along the heading. The yaw replicates to other clients unchanged, so
  remote players render consistently with no protocol change.
- `client/src/bosses.rs::attach_boss_models`: the server keeps its +Z yaw
  convention for AI entities; raid-boss GLBs share the VRM-staged facing
  (wendigo-hollow is literally the same conversion as the osa-wendigo-blond
  avatar), so the boss model child now carries a 180° Y rotation to face its
  move/attack direction. Minions and neutral camp creatures are spheres —
  no visible facing.

## Checks

- `cargo check -p client` clean, `cargo clippy -p client` 0 warnings,
  `cargo test -p client` 48 passed.

## Remaining risks

- Facing is only visually verifiable in a live run; if some individual GLB is
  authored with the opposite facing, it will now be the one walking
  backwards — flip would then belong in that model's staging, not the shared
  yaw convention.
