# VRM1-RENDITION-20261022: VRM 1.0 sources in the hosted rendition builder

Frozen 2026-10-22 before implementation.

## Problem

`scripts/ekza_build_rendition.py` (the `desktop/humanoid-glb-v1` builder the Ekza
registry runs) classifies a VRM 1.0 source as `vrm1`, but the bake step
`retarget_animations.retarget_avatar` reads only `extensions.VRM` and fails with
`build_failed: no VRM extension`. Every VRM 1.0 upload, including Ekza Studio
exports, therefore gets no Omoba rendition.

## Acceptance criteria

- AC1: A VRM 1.0 source (`extensions.VRMC_vrm`, `humanoid.humanBones` object,
  `thumbMetacarpal/thumbProximal/thumbDistal`, optional `upperChest`, +Z rest
  facing, small elbow/knee rest flexion, `KHR_materials_unlit`) builds through
  `ekza_build_rendition.py` with exit 0, `report.sourceFormat == "vrm1"`, and the
  five clips idle/walk/attack/cast/death.
- AC2: VRM1 thumbs are driven by the same source bones as VRM0 thumbs, matching the
  client runtime retargeter (`client/src/humanoid/retarget.rs::source_semantic`).
- AC3: The VRM1 rendition faces -Z like every VRM0 rendition the game renders (the
  hero model forward); clips are baked against that rest. VRM1 metadata is kept.
- AC4: VRM0 renditions are byte-identical to the pre-change builder for all 15
  shipped roster avatars.
- AC5: The VRM1 rendition passes `omoba_passport::validate_humanoid_profile` and
  `validate-runtime-vrm`; the Python profile mirror passes in tests.
- AC6: A model declaring both `VRM` and `VRMC_vrm` is refused with exit 2
  (mirrors passport), not crashed.
- AC7: Tests (stdlib unittest) cover VRM0 unchanged + VRM1 success; docs, CHANGELOG
  and a progress note describe the change.
