# 2026-07-06 — World Scale And Spawn Platform (TASK-21)

## Goal

Two follow-ups to TASK-20: characters normalized to 0.26 world units were
barely visible in a world tuned for `PLAYER_SIZE = 1.0`, and the 0.7-tall
spawn base pad had no height handling — characters sank into it and popped
off its edge. Target: League-style smooth platform traversal, faked entirely
client-side.

## Changes

- **Size:** `DEFAULT_MODEL_TARGET_HEIGHT` 0.26 → 1.15, range [0.08, 1.2] →
  [0.3, 3.0] (`client/src/model_scale.rs`). Persisted values below the new
  minimum are legacy-scale and migrate to the new default on load with a log
  line (`migrate_model_target_height`, `client/src/persistence.rs`).
- **Terrain function:** `MapLayout::terrain_height(x, z)` — full
  `BASE_PAD_HEIGHT` on the 46×46 pad top, linear descent over
  `PAD_RAMP_LENGTH = 6.0` beyond each edge, ramp slabs extended past the
  corners with max-of-slabs overlap, 0.0 elsewhere (`client/src/maps.rs`).
- **Visible ramps:** `spawn_pad_ramps` adds 4 thin sloped cuboids per pad in
  the pad's team material; their top faces coincide with the height function.
- **Grounding:** `NormalizeModelScale` now records `foot_local_y`
  (measured bind-pose `min_y` × applied scale, on both the glTF and fallback
  paths). `player::ground_origin_y` = terrain − foot offset (half-cube for
  unmeasured/primitive models). Used by `apply_gravity`, the jump-fallback
  hop (`animate_jump` now tracks terrain instead of a fixed `start_y`), and a
  new `PostUpdate` system in `net.rs` that re-bases remote players and
  minions after interpolation overwrites their flat server Y.
- Version 0.8.0 → 0.9.0, CHANGELOG, `docs/features.md`.

## Checks

- `cargo check --workspace` clean; `cargo clippy --workspace` 0 warnings;
  `cargo test --workspace` green — 48 client tests including new coverage:
  terrain pad/spawn/ramp/corner/continuity/open-ground, persisted-height
  migration, foot-offset math.
- Headless analyzer still measures 34/34 GLBs (now reporting scale@1.15).

## Remaining risks

- Not yet play-tested with a live server + two clients; ramp feel (6-unit
  length) and the 1.15 default height may want taste tuning — both are single
  constants, and per-model multipliers hot-reload from the overrides JSON.
- Server aim heights and projectile visuals still assume the flat plane;
  on-pad combat visuals may sit slightly low (cosmetic only).
- Neutral camps/bosses/structures never stand on pads, so they were left on
  the flat plane by design.
