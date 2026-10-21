# VRM 1.0 sources in the Ekza rendition builder

Goal: a VRM 1.0 avatar (`VRMC_vrm`, as Ekza Studio exports) gets an Omoba
`desktop/humanoid-glb-v1` rendition. Before this change `ekza_build_rendition.py`
reported `sourceFormat: vrm1` support but the bake step read only `extensions.VRM`,
so every VRM 1.0 upload failed with `build_failed: no VRM extension`.

## Changes

- `scripts/retarget_animations.py`: `humanoid_bone_map` reads the VRM 0.x list or the
  VRM 1.0 object; VRM 1.0 thumbs (Metacarpal/Proximal/Distal) map onto the VRM 0.x
  thumb chain, as `client/src/humanoid/retarget.rs::source_semantic` does. A model
  declaring both `VRM` and `VRMC_vrm` is refused, as in `passport::humanoid`.
- VRM 1.0 rests facing +Z; the hero model forward and every VRM 0.x avatar is -Z.
  When a VRM 1.0 rest faces +Z (hands/head basis) the scene roots are turned 180°
  about Y before baking (shared helper `normalize_vrm1_rest_facing`, recorded as
  `asset.extras.omoba_rest_yaw180`). A re-bake sees -Z and does not turn it again.
  VRM 0.x is never turned.
- Small robustness: the BIN appender tolerates documents with no accessor/bufferView
  arrays; `Gltf.from_glb_bytes`.
- Tests in `scripts/test_ekza_build_rendition.py`: a Studio-shaped synthetic VRM 1.0
  (identity rest, 2° elbow/knee flexion, no upperChest, `KHR_materials_unlit`); a
  shipped CC0 VRM 0.x avatar re-declared as VRM 1.0 and turned to +Z must bake the
  same per-node streams as its VRM 0.x original (within 1e-5); VRM 0.x roots are never
  turned; re-bake is byte-stable; conflicting declarations exit 2. A real VRM 1.0
  export is exercised when `OMOBA_VRM1_SAMPLE` (default: the Ekza Studio
  `fixtures/models/seed-san.vrm`, VRM Public License 1.0) exists; it is never copied
  here.

## Follow-up in the same branch

- `passport::humanoid`: positive, bounded scale is still required on every node, but
  uniformity (1e-5) only on humanoid bones and their ancestors — the only nodes whose
  scale the retargeter composes. Accessory/helper nodes keep their authored
  transform in Bevy, so exporter roundoff (seed-san `bag_rope_L`
  `[0.99975, 1, 0.99968]`) no longer rejects the avatar. Chosen over a looser global
  tolerance because it keeps the check exact where it matters and ignores it where
  nothing reads it.
- `client/src/player/animation.rs`: when a VRM-tagged model's runtime rig is
  rejected, the match path falls through to the embedded-clip (legacy) path instead
  of leaving the hero static. The avatar preview and party stage already did this.
- `scripts/convert_vrm_to_glb.py` (local import) applies the same VRM 1.0 turn via
  the shared helper; VRM 0.x and already -Z files are copied byte for byte; a model
  declaring both VRM versions fails without writing output.

## Checks

- `python3 -m unittest scripts/test_ekza_build_rendition.py`: 10 tests OK. Mutation
  check: disabling the turn fails 3 tests; disabling thumb renaming fails 2.
- All 15 shipped roster avatars: rendition SHA-256 before and after are identical.
- `seed-san.vrm` (VRM 1.0, 51 mapped bones) builds; world trajectories of walk left
  foot / attack right hand correlate 0.999 with a VRM 0.x avatar along Z.
- `omoba_passport::validate_humanoid_profile` accepts the seed-san, synthetic and
  re-declared VRM 1.0 renditions; after the follow-up `validate-runtime-vrm` accepts
  all three plus the seed-san source.
- Follow-up: `scripts/test_convert_vrm_to_glb.py` 5 tests OK;
  `cargo test -p omoba-passport` OK (new scale-scope test);
  `cargo test -p client --lib -- player:: humanoid::` 80 OK, including the fallback
  test (fails when the old `continue` is restored) and a seed-san runtime retarget
  test (skips without the sample); clippy `-D warnings` and rustfmt clean.

## Remaining risks

- Not visually checked in the native client (facing, fallback playback).
