# VRM avatar support + CC0 humanoid `Paco`

- **Date:** 2026-06-28
- **Version:** 0.2.0 → 0.3.0 (backward-compatible feature; SDK bumped to match)

## Goal

Add VRM avatar support to the omoba MOBA engine and integrate one Creative-Commons
avatar as a selectable character, end to end, without adding a heavy/incompatible
VRM dependency to Bevy 0.18.

## Approach chosen: VRM-as-GLB through the existing glTF pipeline

VRM 0.x is a glTF 2.0 **binary** container. The VRM-specific data (humanoid bone
map, blendshapes, spring bones) lives in glTF extensions that — in the avatar we
shipped — appear under `extensionsUsed` only, never `extensionsRequired`. Bevy's
standard `GltfLoader` therefore loads the mesh + skeleton and ignores the unknown
extensions.

Bevy selects an asset loader by **file extension**, and a `.vrm` extension is not
registered to the glTF loader. Rather than re-register the glTF loader for `.vrm`
(fragile in Bevy 0.18's `GltfPlugin`) or add an unproven `bevy_vrm`-style crate,
we stage the avatar as `.glb` (a byte-identical glTF 2.0 container). The
"conversion" is a validate-and-copy implemented in `scripts/convert_vrm_to_glb.py`
(checks `glTF` magic + version 2, then writes the bytes with a `.glb` extension).
This keeps the model-catalog path identical to the existing `Toka`/`Wang` GLBs and
the SDK's `validate_glb_file` check works unchanged.

## Avatar

- **Paco** (Avatar 211), collection **100Avatars R3**, author **ToxSam**.
- Source: https://www.opensourceavatars.com/en/avatar/60176f95-a780-4e09-85ca-545314182146
- Original VRM: https://arweave.net/0i-EEnHlcq1EZ1-sMi8DTZhesqGLqtf30WuCknfTHjA
- License: **CC0 1.0** (public domain). Recorded in `ATTRIBUTION.md`.
- glTF JSON inspection: 1 mesh, 70 nodes (humanoid rig), 1 skin, **0 animations**,
  1 unlit material; `extensionsUsed = [KHR_materials_unlit, KHR_texture_transform, VRM]`;
  no `extensionsRequired`; default `scene = 0` (Bevy label `Scene0`).

## Changes

### `ekza-bevy-sdk` (sibling repo)
- `src/lib.rs`: new `EkzaCharacter::Paco` variant; `ALL` grown to 5; `as_str`/`slug`
  arms; `BUILTIN_MODEL_MANIFEST` grown to 5 with a `LocalGlb` entry pointing at
  `downloaded/paco.glb` (`locomotion_animations: false`); serde snake_case test
  extended with `"paco"`.
- `examples/model_cache.rs`, `examples/model_viewer.rs`: `"paco"` parse arm and
  updated usage strings.
- `Cargo.toml`: version 0.2.0 → 0.3.0.

### `omoba-bevy`
- `client/assets/downloaded/paco.glb`: the staged avatar (3.6 MB).
- `scripts/convert_vrm_to_glb.py`: documented validate-and-copy VRM→GLB script.
- `ATTRIBUTION.md`: new file crediting the avatar (CC0).
- `Cargo.toml`: workspace version 0.2.0 → 0.3.0.
- `CHANGELOG.md`, `docs/features.md`: documented the feature.
- No changes to `client/src/*` or `server/src/*` were required: the
  character-select UI iterates `EkzaCharacter::ALL`, and the model catalog /
  `NormalizeModelScale` spawn path is data-driven from the manifest.

## Animation status (honest)

Paco ships **zero** animation clips, so it uses the existing anim-less fallback:
`setup_player_animation_library` only evaluates `Ipfs`/`Toka`/`Wang`, and the
jump fallback (`should_use_jump_fallback`) is hardcoded to `Cube`. Paco therefore
renders as a **static** skinned mesh — no idle/walk locomotion and no jump bounce.
This is the documented MVP gap; nothing fabricated.

To add locomotion later: bundle a GLB whose `named_animations` contains clips with
names containing `idle` and `walk`/`walkcycle` (the engine keys on those
substrings in `player.rs`), then add `Paco` to the evaluation loop in
`setup_player_animation_library`. Sourcing/retargeting CC0 humanoid clips onto the
VRM rig headless was out of scope for this slice.

## Checks

- `cargo build --workspace` — pass.
- `cargo test -p server` — pass.
- `cargo test -p client` — pass.
- `cargo test -p harness -- --test-threads=1` — pass.
- `cargo test -p ekza-bevy-sdk` (sibling) — pass.

## Remaining risks / human verification

Headless, the following could **not** be verified and need a human running
`make start`:
- Actual GPU rendering of the Paco mesh and materials (unlit VRM material under
  the scene lighting).
- That `NormalizeModelScale` produces a sensible on-screen height and head-bar
  anchor for this rig.
- VRM blendshapes and spring bones are intentionally **not** supported (ignored by
  the glTF loader); face/hair physics will not animate.
- Confirm the avatar is selectable in the character-select overlay and spawns on
  team join.
