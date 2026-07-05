# 2026-07-05 — Character Scale Normalization (TASK-20)

## Goal

Characters from different sources (legacy SDK models 0.64–1.90 m, 28 VRM-staged
roster avatars 0.96–2.04 m, raid bosses up to 2.41 m) rendered at visibly
mismatched sizes. Build a dedicated module that analyzes every character model
and normalizes all of them to one shared size by default, with per-character
tweak overrides.

## Changes

- New `client/src/model_scale.rs` (`ModelScalePlugin`):
  - Bind-pose measurement straight from the loaded glTF asset graph (node
    TRS × mesh AABBs), cached per `AssetId<Gltf>` — deterministic,
    independent of animation state or spawn timing.
  - Absolute rescale: root scale = target / raw height. Re-applying after a
    pause-menu target change or override edit is idempotent; the old system
    compounded relative factors on top of the already-scaled transform.
  - Per-model multipliers in `client/assets/config/model_scale_overrides.json`
    (slug → multiplier, `_`-prefixed keys are comments, clamped to
    [0.1, 10.0]), hot-reloaded ~1 s by mtime polling.
  - Headless analyzer: `OMOBA_MEASURE_MODELS=1 cargo run -p client` prints the
    measured height table via the exact in-game code path (no window/GPU).
  - Legacy world-AABB normalization kept as fallback for roots without a glTF
    source; now remembers its first raw measurement so it is absolute too.
- `ModelScaleSource` (gltf handle + override key) attached at all four model
  spawn sites: local player (`world.rs`), networked local + remote players
  (`net.rs`), raid bosses (`bosses.rs`, keeps the 3× presence multiplier).
- `NormalizeModelScale`/`ModelScaleSettings`/height constants moved out of
  `world.rs`; importers repointed.

## Checks

- `cargo check --workspace` clean; `cargo clippy --workspace` 0 warnings;
  `cargo test --workspace` green (7 new unit tests in `model_scale`).
- In-engine analyzer output matches the offline stdlib-python GLB reference
  for all 34 staged GLBs with 0 relative deviation
  (`.agent/tasks/TASK-20-character-scale-normalization/raw/`).

## Remaining risks

- Visual in-match confirmation (two clients + server) not yet performed; the
  analyzer verifies measurement, not the final on-screen composition. If any
  model still looks off, tune its slug in `model_scale_overrides.json` — the
  file reloads live.
- Models whose pivot is not at the feet (e.g. `lady-koi` min_y −0.22, `toka`
  −0.33) keep their authored pivot offset: normalization scales but does not
  ground-align, matching previous behavior.
