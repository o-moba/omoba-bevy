# 2D sprite player prototype — 2026-07-27

Implemented a selectable sprite renderer without changing the authoritative
gameplay world. Player transforms, terrain grounding, collisions, input,
combat, camera follow, server corrections, and remote interpolation remain on
the existing roots; sprite visuals are billboarded child quads.

## Delivered

- Added client-local `models3d` / `sprite2d` mode selection. The default and
  invalid-environment fallback are `models3d`; `OMOBA_PLAYER_VISUAL_MODE`
  controls the initial selection.
- Added five independent sprite-character selections while preserving the
  selected 3D avatar when switching modes.
- Added a schema-versioned manifest defining sheets, grid, pivot, world height,
  idle/run frame ranges, and timing. UI thumbnails and runtime animation read
  the same manifest-backed definitions.
- Added time-based idle/run animation driven by owner XZ movement, including
  interpolation grace and multi-frame advancement after a long render frame.
- Added optional/defaulted sprite identity to client/server/harness packets,
  committed reconnect loadout, authoritative player state, and snapshots.
  Unknown and omitted ids normalize to `mossback-teapot`; ids never become
  paths.
- Added a dependency-free PNG validator with negative tests for roster ids,
  frame ranges, dimensions, and unsafe paths, plus focused client animation
  tests and a live multi-client server/harness round-trip test.

## Asset convention

Each `client/assets/sprites/<id>.png` is a 2048×512, 8-bit RGBA sheet with
256×256 cells. Row 0 is eight idle frames at 6 fps and row 1 is eight run
frames at 12 fps. The outer two pixels of every cell remain transparent. See
`client/assets/sprites/manifest.json` and `LICENSE.md` for metadata and CC0
provenance.

## Verification commands

```sh
python3 scripts/validate_sprite_assets.py --self-test
cargo test -p client sprite::tests
cargo build -p server
cargo test -p harness --test sprite_cosmetics
```
