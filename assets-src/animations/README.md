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
needed to regenerate it. Three exporter tables define its 58 motions.

### Full-rate clips (`CLIPS` in `scripts/export_humanoid_motion.py`)

Every source key is sampled. "Contact" is the time from the clip start to the
key at which the strike or gesture is fully formed.

| Runtime motion | UAL source | Loop | Contact | Purpose |
| --- | --- | --- | --- | --- |
| `idle` | `Idle_Loop` | Yes | - | Standing, including models with no embedded clips |
| `walk` | `Walk_Loop` | Yes | - | Reserved for future slow movement/debuff behavior |
| `run` | `Sprint_Loop` | Yes | - | Normal hero movement; an actual sprint, not accelerated walking |
| `attack` | `Sword_Attack` | No | 0.417 s | Shared attack fallback |
| `cast` | `Spell_Simple_Shoot` | No | 0.083 s | Shared cast fallback |
| `death` | `Death01` | No | - | Shared death fallback |
| `spell_prepare` | `Spell_Simple_Enter` | No | 0.417 s | Dawn ray preparation; hold last pose until authoritative release |
| `spell_finish` | `Spell_Simple_Exit` | No | 0.25 s | Dawn field gesture |
| `pistol_shoot` | `Pistol_Shoot` | No | 0.042 s | Wildspark shooting and Repeater basic attacks |
| `pistol_reload` | `Pistol_Reload` | No | 0.333 s | Wildspark weapon-mode switch |
| `pistol_aim` | `Pistol_Aim_Neutral` | No | 0 s | Static aim pose for profile authoring |
| `interact` | `Interact` | No | 0.5 s | Barrier / trap placement gesture |
| `punch` | `Punch_Cross` | No | 0.25 s | Right cross |
| `guard` | `Punch_Enter` | No | 0.333 s | Hands up into the fist guard |
| `shoulder_drive` | `Punch_Jab` | No | 0.208 s | Left jab |
| `roll` | `Roll` | No | 0.375 s | Forward roll |

### Dagger timing edits (`DAGGER_MOTIONS`)

`dagger_stab` (0.22 s), `dagger_feint` (0.155 s) and `dagger_heavy_thrust`
(0.336 s) sample `Punch_Cross` or `Punch_Jab` at a few phases and spread the
keys evenly; the value in brackets is the contact. `dagger_backstab` joins two
source clips and is therefore a row of the derived table below.

### Derived motions (`assets-src/animations/derived-motions.json`)

A row builds one clip from pose keys of the CC0 source. A key is a source clip
and a normalised phase of it; nothing is drawn by hand. The exporter can only

- pick, reorder, repeat and retime keys of one or several source clips
  (segment, retime, reverse, cross-clip sequence, hold);
- mirror a key or the whole clip through the sagittal plane of the symmetric
  reference pose (left and right streams swap, `(x, y, z, w)` becomes
  `(x, -y, -z, w)`);
- bake a whole-body turn about the vertical axis (`ops.spin_deg`, degrees per
  key, at most 90 per step, ending on a full turn);
- lift the hips (`ops.hips_y`, source metres per key, ending on the ground);
- close a decimated loop on its first key.

Hips X/Z are zero in every derived clip, so no motion moves the hero. Streams
are hemisphere-corrected between keys. Row fields:

| Field | Meaning |
| --- | --- |
| `duration` | Clip length in seconds |
| `looping` | A loop repeats its first key last; the exporter closes that seam exactly |
| `keys` | `[{clip, phase, mirror?}]` in play order |
| `times` | Optional key times in seconds; evenly spaced when absent |
| `ops` | Optional `mirror`, `spin_deg`, `hips_y` as described above; `ops.mirror` and a key's own `mirror` cancel |
| `contact` | Seconds from the clip start to the contact pose; required for an action clip, forbidden for a loop |
| `approx` | The row joins poses that were never authored as one movement and needs a look on two rigs before a skill uses it |

| Runtime motion | UAL source keys | Keys / seconds | Contact | Purpose |
| --- | --- | --- | --- | --- |
| `slash_down` | `Sword_Attack` | 6 / 0.55 | 0.165 s | Downward weapon chop |
| `slash_rising` | `Sword_Attack` | 5 / 0.5 | 0.125 s | Rising return stroke as its own strike |
| `cleave_slam` | `Sword_Attack` | 7 / 0.9 | 0.3 s | Heavy overhead blow with a low hold |
| `spin_cleave` | `Sword_Attack`; one baked turn | 10 / 0.6 | 0.1 s | Full-circle sweep |
| `slash_down_m` | `Sword_Attack`; mirrored | 6 / 0.55 | 0.165 s | Mirror of `slash_down` (off hand) |
| `slash_rising_m` | `Sword_Attack`; mirrored | 5 / 0.5 | 0.125 s | Mirror of `slash_rising` (off hand) |
| `blade_flourish` | `Sword_Idle`, `Sword_Attack` | 4 / 0.45 | 0.15 s | Short draw cut and return to the blade stance (approx.) |
| `blade_ready_loop` | `Sword_Idle` | 6 / 1.667 | loop | Blade stance, windup hold |
| `thrust_lunge` | `Punch_Jab`; mirrored | 5 / 0.45 | 0.113 s | Straight right-hand thrust (mirrored jab) |
| `jab_cross` | `Punch_Jab`, `Punch_Cross` | 7 / 0.6 | 0.1 s | Left jab, right cross |
| `fist_guard_loop` | `Punch_Cross` | 2 / 1 | loop | Fist guard, windup hold (static pose) |
| `ground_pound` | `Punch_Enter`, `Jump_Land` | 5 / 0.7 | 0.17 s | Fists up, then a deep crouch with both hands at the ground (approx.) |
| `flying_knee` | `Jump_Start`, `Jump_Land`; lift 0.5 m | 7 / 0.7 | 0.17 s | Tucked hop and planted landing; a stand-in, not a kick (approx.) |
| `leap_land` | `Jump_Start`, `Jump_Land`; lift 0.9 m | 6 / 0.65 | 0.12 s | Arrival of an instant leap: airborne, impact, rise |
| `dive_lunge` | `Swim_Fwd_Loop`, `Jump_Land` | 6 / 0.6 | 0.12 s | Horizontal reach, then a landing crouch (approx.) |
| `vault_flip` | `Roll` | 7 / 0.7 | 0.35 s | Forward roll in place |
| `backflip_retreat` | `Roll` | 7 / 0.6 | 0.1 s | Backward roll in place (approx.) |
| `dance_twirl` | `Dance_Loop`; one baked turn | 8 / 0.5 | 0.143 s | Dance step turning once (approx.) |
| `overhead_plant` | `Idle_Loop`, `Pistol_Aim_Up`, `Pistol_Aim_Down` | 5 / 0.7 | 0.26 s | Both hands overhead, then driven down in front (approx.) |
| `rally_raise` | `Idle_Loop`, `Pistol_Aim_Up` | 4 / 0.6 | 0.15 s | Both arms to the sky, held |
| `raise_from_earth` | `Fixing_Kneeling`, `Pistol_Aim_Up`, `Idle_Loop` | 6 / 0.8 | 0.2 s | Kneel, touch the ground, rise with both arms up (approx.) |
| `two_hand_push` | `Punch_Enter`, `Push_Loop`, `Idle_Loop` | 5 / 0.6 | 0.1 s | Two-hand shove from a lunge; a stand-in for a shield bash (approx.) |
| `shot_heavy` | `Pistol_Aim_Neutral`, `Pistol_Shoot`, `Hit_Chest` | 6 / 0.8 | 0.17 s | Two-hand shot, weapon thrown down to the hip with a backward lean, slow re-aim (approx.) |
| `burst_fire` | `Pistol_Shoot` | 7 / 0.45 | 0.075 s | Three short recoil beats |
| `sky_shot` | `Pistol_Aim_Neutral`, `Pistol_Aim_Up`, `Pistol_Shoot`, `Idle_Loop` | 6 / 0.6 | 0.15 s | Aim to the sky, shot, lower through the aim |
| `ground_shot` | `Pistol_Aim_Neutral`, `Pistol_Aim_Down`, `Pistol_Shoot`, `Idle_Loop` | 5 / 0.5 | 0.14 s | Aim at the ground, shot, recoil up |
| `reload_snap` | `Pistol_Reload` | 6 / 0.6 | 0.34 s | Off hand drops and claps back to the weapon |
| `aim_hold_loop` | `Pistol_Idle_Loop` | 6 / 1.667 | loop | Two-hand aim, windup hold |
| `aim_loose_r` | `Spell_Simple_Enter`, `Spell_Simple_Shoot`, `Spell_Simple_Exit`; mirrored | 7 / 0.55 | 0.14 s | Right arm aims, holds and snaps back; a stand-in for a bow release (approx.) |
| `cast_thrust_r` | `Spell_Simple_Enter`, `Spell_Simple_Shoot`, `Spell_Simple_Exit`; mirrored | 6 / 0.6 | 0.24 s | Weapon-hand cast thrust (mirrored spell clips) |
| `point_command` | `Idle_Loop`, `Idle_Torch_Loop` | 4 / 0.5 | 0.16 s | Left hand raised forward, held, lowered |
| `hover_pulse` | `Idle_Loop`, `Swim_Idle_Loop`; lift 0.2 m | 5 / 0.6 | 0.15 s | Short lift off the ground with spread arms (approx.) |
| `levitate_loop` | `Swim_Idle_Loop`; lift 0.25 m | 8 / 1.6 | loop | Hover, windup hold (approx.) |
| `draw_in` | `PickUp_Table` | 5 / 0.5 | 0.125 s | Reach forward and pull across the chest |
| `hurl_overhand` | `Sword_Attack`, `Spell_Simple_Shoot`, `Spell_Simple_Exit`, `Idle_Loop`; one stance mirrored | 6 / 0.5 | 0.2 s | Overhand throw from high behind into a pointing arm (approx.) |
| `toss_underhand` | `PickUp_Table`, `Idle_Loop` | 4 / 0.4 | 0.13 s | Underhand toss with the left hand (approx.) |
| `place_quick` | `Interact` | 5 / 0.6 | 0.15 s | Quick placing gesture (0.6 s instead of the 2 s `interact`) |
| `kneel_plant` | `Fixing_Kneeling` | 7 / 0.75 | 0.36 s | Kneel, set both hands low, stand |
| `dagger_backstab` | `Punch_Cross`, `Crouch_Idle_Loop` | 6 / 0.65 | 0.11 s | Dagger stab, then a drop into a crouch (approx.) |

Not available from this library and not faked: a true kick, a bow draw, a
shield raise, a whip or chain swing, a two-handed great-weapon swing.
`flying_knee`, `aim_loose_r` and `two_hand_push` are stand-ins and must be
called that in user-facing notes.

The exported JSON carries a top-level `contacts` map (motion ID to seconds) for
every action clip: all motions except the loops and `death`.

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
committed asset. The test module also checks every derived row: in place,
declared timing, short-way interpolation after retargeting, distance from the
idle pose, mirrors within 5 mm, turns that return to the start heading and
hops that end on the ground. `--audit` reads all 15 shipped avatar GLBs, retargets Run in
memory, and reports actual joint rotation excursions, hips/feet position
ranges, finite values, loop seams and unchanged model hashes. Native runtime
rendering and lifecycle checks are separate evidence, not implied by this
numeric audit.

The older `scripts/retarget_animations.py` remains a legacy importer for the
five embedded clips already present in shipped GLBs. The new exporter reuses
its source sampler and quaternion math, and does not invoke its baking or
GLB-writing functions. Do not rerun legacy baking to add Run to models.

## Shared asset contract (version 1)

The JSON resource is roughly 1.58 MiB and contains 58 motions (698 keys) for 52
semantic bones. Its schema uses glTF right-handed, Y-up coordinates and
`[x, y, z, w]` unit quaternions. Numeric values are rounded to seven decimal
places.

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
- `clips`: maps motion IDs to objects with
  `source_clip`, `duration`, `looping`, strictly increasing `times`,
  `world_rotation_deltas` (bone to quaternion array), and `hips_world_deltas`
  (position array in source metres). Every channel has one value per time.
  The six base states remain required. Up to 64 named motions are accepted;
  all extra motions receive the same validation, retargeting and model cache.
- `contacts`: optional map from motion ID to the seconds between the clip
  start and its contact pose. The runtime rejects an entry that names no clip
  or lies outside it. Adding the map left the bytes of every kept clip object
  unchanged; `scripts/test_humanoid_motion.py` pins a SHA-256 per clip.

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
