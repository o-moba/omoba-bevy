# 2026-07-06 — Minion Models And Visible Camps/Bosses (TASK-24)

## Goal

Lane minions were placeholder spheres, and the neutral camps + raid-boss
pits sat exactly at decorative jungle-block centers — the 12×4×12 boxes
fully enclosed the creatures, so bosses were invisible from the field.

## Changes

- **Slime minions.** Staged two CC0 "Mimic Slime" VRMs from the local
  Open Source Avatars collection (Halloween Rising, Polygonal-Mind):
  `client/assets/minions/slime-green.glb` (Classic) and `slime-blue.glb`
  (Water), UAL clips baked via the existing retarget pipeline; provenance
  manifest + ATTRIBUTION entry. Minion spawn (`net.rs`) now attaches the
  team slime scene as a 180°-rotated child (server +Z yaw convention),
  `NormalizeModelScale::scaled_by(0.6)` + `ModelScaleSource` (override keys
  `slime-green`/`slime-blue`), grounding uses the measured foot offset
  (sphere-radius fallback until measured), and
  `force_vrm_models_double_sided` covers minion roots. The unused sphere
  mesh/material fields were removed.
- **Minion animations.** New `client/src/minions.rs`
  (`MinionVisualsPlugin`), boss-pattern wiring: per-team idle/walk/attack
  graphs from the slime GLTFs, `AnimationPlayer` binding by ancestry, clip
  selection from the replicated `MinionBrainState` (marching/chasing →
  walk, attacking → attack).
- **Visible camps/bosses.** `MapLayout::boss_pit_centers()` (server
  mirror) + `decorative_jungle_block_centers()`: blocks within 10 units of
  a camp/boss anchor are not spawned (5 of 10 boxes removed); unit test
  proves no spawned block entombs an anchor and the other 5 remain.
- Analyzer scans `minions/` (36 GLBs: slime-green 1.374, slime-blue
  1.510 raw height); version 0.11.0 → 0.12.0; CHANGELOG; features.md.

## Checks

- `cargo check/clippy --workspace` clean (0 warnings);
  `cargo test --workspace` green (134 tests, incl. the new
  `no_decorative_block_entombs_a_camp_or_boss`).
- Headless analyzer measures both slime GLBs through the real glTF
  pipeline (loader compatibility proven without a GPU).

## Remaining risks / notes

- Jungle-camp creatures (Skirmisher/Bruiser/Spitter) are still spheres —
  only lane minions got models this task (spec'd out of scope).
- Minion visual size (0.6× hero) and per-slime multipliers are tunable
  live via `model_scale_overrides.json`.
- Slime GLBs are wide (2.1 units at raw scale); if they visually clip the
  lane edges, lower the multiplier in the overrides file.
- In-match visuals (animations, double-sided rendering) still deserve one
  human playtest pass; everything verifiable headless is covered by tests.
