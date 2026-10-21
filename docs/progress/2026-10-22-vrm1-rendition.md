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
  about Y before baking, recorded as `asset.extras.omoba_retarget.restYaw180`. A
  re-bake sees -Z and does not turn it again. VRM 0.x is never turned.
- Small robustness: the BIN appender tolerates documents with no accessor/bufferView
  arrays; `Gltf.from_glb_bytes`.
- Tests in `scripts/test_ekza_build_rendition.py`: a Studio-shaped synthetic VRM 1.0
  (identity rest, 2° elbow/knee flexion, no upperChest, `KHR_materials_unlit`); a
  shipped CC0 VRM 0.x avatar re-declared as VRM 1.0 and turned to +Z must bake the
  same per-node streams as its VRM 0.x original (within 1e-5); VRM 0.x roots are never
  turned; re-bake is byte-stable; conflicting declarations exit 2. A real VRM 1.0
  export is exercised when `OMOBA_VRM1_SAMPLE` (default: the Ekza Animator
  `seed-san.vrm` fixture, VRM Public License 1.0) exists; it is never copied here.

## Checks

- `python3 -m unittest scripts/test_ekza_build_rendition.py`: 10 tests OK. Mutation
  check: disabling the turn fails 3 tests; disabling thumb renaming fails 2.
- All 15 shipped roster avatars: rendition SHA-256 before and after are identical.
- `seed-san.vrm` (VRM 1.0, 51 mapped bones) builds; world trajectories of walk left
  foot / attack right hand correlate 0.999 with a VRM 0.x avatar along Z.
- `omoba_passport::validate_humanoid_profile` accepts the seed-san, synthetic and
  re-declared VRM 1.0 renditions; `validate-runtime-vrm` accepts the synthetic and
  re-declared ones.

## Remaining risks

- `validate-runtime-vrm` rejects seed-san (source and rendition) because a
  non-humanoid node (`bag_rope_L`) has scale `[0.99975, 1, 0.99968]`, outside the
  1e-5 uniform-scale tolerance. In the client a VRM-tagged model whose runtime rig
  is rejected gets no animation set at all (`player/animation.rs` removes the set
  and skips the embedded-clip path), so by code reading such a rendition would not animate in game
  even though it passes `validate_humanoid_profile`.
- The local clipless import path (`convert_vrm_to_glb.py`) does not turn VRM 1.0, so a
  locally staged VRM 1.0 rests facing +Z in game.
- Not visually checked in the native client.
