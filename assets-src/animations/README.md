# Animation source and shared humanoid motion

These CC0 source files are pipeline inputs, not runtime downloads. The runtime
uses one engine-owned semantic motion resource:
`client/assets/animations/humanoid-motion-v1.json`. The exporter never modifies
an avatar, so the same motion can drive models with different proportions,
bone names, optional joints and rest orientations after validated binding.

## Quaternius Universal Animation Library (UAL)

| File | Description |
| --- | --- |
| `AnimationLibrary_Godot_Standard.gltf` | glTF 2.0 JSON with 46 humanoid clips on a Blender Rigify `DEF-*` deform skeleton |
| `AnimationLibrary_Godot_Standard.bin` | Binary buffer referenced by the `.gltf` |

- Author: Quaternius (https://quaternius.com)
- License: **CC0 1.0 Universal** (public domain dedication)
- Original mirror: https://github.com/J-Ponzo/gltf-universal-animation-library
- Existing imported inputs:
  - https://raw.githubusercontent.com/J-Ponzo/gltf-universal-animation-library/main/glTF/AnimationLibrary_Godot_Standard.gltf
  - https://raw.githubusercontent.com/J-Ponzo/gltf-universal-animation-library/main/glTF/AnimationLibrary_Godot_Standard.bin

The generated library records the SHA-256 of both inputs. No new download is
needed to regenerate it.

| Runtime motion | UAL source | Loop | Purpose |
| --- | --- | --- | --- |
| `idle` | `Idle_Loop` | Yes | Standing, including models with no embedded clips |
| `walk` | `Walk_Loop` | Yes | Reserved for future slow movement/debuff behavior |
| `run` | `Sprint_Loop` | Yes | Normal hero movement; an actual sprint, not accelerated walking |
| `attack` | `Sword_Attack` | No | Shared attack fallback |
| `cast` | `Spell_Simple_Shoot` | No | Shared cast fallback |
| `death` | `Death01` | No | Shared death fallback |

`A_TPose` is the reference pose. Sprint has 17 keys over approximately 0.667
seconds, while Walk has 33 keys over approximately 1.333 seconds. Idle, Walk
and Run close their final quaternion and hips keys to the first pose; their
small source export seams are removed. Hips sway remains, but each loop has
zero net displacement. Gameplay remains responsible for moving the hero's
world root.

## Regeneration and validation

Run from any directory (Python standard library only):

```sh
python3 scripts/export_humanoid_motion.py
python3 scripts/export_humanoid_motion.py --check --audit /tmp/omoba-motion-audit.json
python3 -m unittest discover -s scripts -p test_humanoid_motion.py -v
```

`--check` regenerates in memory and compares exact bytes without changing the
committed asset. `--audit` reads all 15 shipped avatar GLBs, retargets Run in
memory, and reports actual joint rotation excursions, hips/feet position
ranges, finite values, loop seams and unchanged model hashes. Native runtime
rendering and lifecycle checks are separate evidence, not implied by this
numeric audit.

The older `scripts/retarget_animations.py` remains a legacy importer for the
five embedded clips already present in shipped GLBs. The new exporter reuses
its source sampler and quaternion math, and does not invoke its baking or
GLB-writing functions. Do not rerun legacy baking to add Run to models.

## Shared asset contract (version 1)

The JSON resource is roughly 495 KiB and contains six motions for 52 semantic
bones. Its schema uses glTF right-handed, Y-up coordinates and `[x, y, z, w]`
unit quaternions. Numeric values are rounded to seven decimal places.

- `schema_version`: `1`.
- `source`: CC0 author, source filenames and SHA-256 hashes, reference clip,
  coordinate system and loop-processing provenance.
- `source_hips_height`: reference hips height above the source ground.
- `source_hips_to_feet_distance`: hips height minus the mean foot-joint height;
  the runtime uses the equivalent target span for proportion scaling.
- `source_reference_facing`: normalized `(leftHand - rightHand) × up`.
  `source_left_hand_x` is supplemental source metadata.
- `bones`: sorted semantic bone names in VRM 0 convention. For VRM 1 thumbs,
  `Metacarpal`, `Proximal`, `Distal` correspond to the source `Proximal`,
  `Intermediate`, `Distal`; the runtime adapter resolves this difference.
- `clips`: maps `idle`, `walk`, `run`, `attack`, `cast`, `death` to objects with
  `source_clip`, `duration`, `looping`, strictly increasing `times`,
  `world_rotation_deltas` (bone to quaternion array), and `hips_world_deltas`
  (position array in source metres). Every channel has one value per time.

A rotation delta is `source_world(t) * inverse(source_reference_world)`.
For facing alignment `A`, the target world rotation is
`A * delta * inverse(A) * target_reference_world`. The runtime recovers local
rotations through the actual animated parent chain. Non-hips local
translations remain the target's own limb lengths. Hips displacement is
aligned and scaled by target/source hips-to-feet span; it changes the skeleton,
not the authoritative hero root.

Optional joints can be skipped because each channel includes its accumulated
world motion. If `upperChest` is absent, the target `chest` uses the source
`upperChest` channel, matching the legacy torso convention. Finger channels
are optional. Validated target bone indices, rather than source node names,
identify the driven joints.

## Avatar sources

All shipped avatars come from the Open Source Avatars collection
(https://github.com/ToxSam/open-source-avatars), license **CC0**; per-avatar
provenance (collection, author, source URL) is recorded in
`client/assets/avatars/manifest.json`.
