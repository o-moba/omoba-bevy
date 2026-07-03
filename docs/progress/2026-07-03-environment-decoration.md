# 2026-07-03 — TASK-18: Environment Decoration (Procedural Vegetation from Primitives)

## Goal

Make the arena visually rich by scattering stylized low-poly props — trees,
bushes, grass tufts, flowers, rocks — built entirely from Bevy mesh primitives
(no external art assets), procedurally and deterministically, while keeping
every gameplay area (lanes, bases, towers, camps, river) clear and readable.

## Changes

- `client/src/decor.rs` (new): `DecorPlugin` with
  - a pure, render-independent `layout` submodule: inline splitmix64 PRNG
    (no `rand` dependency), `generate_layout(seed) -> Vec<Placement>`,
    `ExclusionZones` derived from `MapLayout`, and rejection-sampled scatter
    (edge forest belts, jungle-block rings, open-meadow fill);
  - prop builders composing 12 variants from unit primitives (Cylinder,
    Sphere, Cone, Cuboid, Capsule3d): oak/pine/birch trees, round/low bushes,
    grass tufts, daisy/sun/tulip/bell flowers, small rocks and boulders, with
    per-instance yaw and scale jitter from the seeded RNG;
  - one `Startup` spawn pass under a single `DecorRoot` entity, reusing
    5 shared mesh handles and 12 shared material handles (`DecorAssets`
    resource) — 396 props = 970 entities, budget ceiling 1200;
  - a client-local F4 toggle flipping `Visibility` on `DecorRoot` (logged);
    no networking, no collision.
- `client/src/maps.rs`: exposed the shared layout math as `pub(crate)`
  methods on `MapLayout` (`lane_polylines`, `river_polyline`,
  `jungle_block_centers`, `camp_centers`) and refactored `setup_moba_map` to
  consume them, so the rendered map and the decor exclusion zones share one
  source of truth. `LANE_WIDTH`, `RIVER_WIDTH`, `BASE_PAD_SIZE` are now
  `pub(crate)`.
- `client/src/main.rs`: registered `DecorPlugin` (separate `add_plugins`
  call; the existing plugin tuple is at Bevy's 15-element limit).
- Version bump 0.4.0 -> 0.5.0, CHANGELOG entry, features inventory update.

## Checks

- `cargo build --workspace` — pass.
- `cargo test --workspace` — pass, including 5 new decor tests:
  determinism, prop-kind coverage, in-bounds + exclusion-zone compliance
  against the real map constants, entity budget/density, and a headless F4
  visibility-toggle test.
- `cargo test -p harness -- --test-threads=1` — headless gameplay scenarios
  still pass (movement/casting unobstructed; decor adds no collision).
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- Live run (server + autojoined client): startup log reports
  `decor: spawned 970 decor entities (396 props + 1 root, budget 1200)
  reusing 5 shared meshes and 12 shared materials`; F4 toggle logged.
- `git diff` touches no files under `server/` or `shared/`.

## Remaining risks

- Visual tuning is subjective: prop palette/density may need iteration after
  playtests; the seed and scatter targets are compile-time constants that are
  easy to adjust.
- Decor draw cost is bounded (shared handles, ~970 entities) but was reasoned
  rather than profiled; if low-end FPS suffers, F4 hides the layer and the
  scatter targets can be reduced in one place.
