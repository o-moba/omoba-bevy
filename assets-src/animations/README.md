# Animation Source Files

Pipeline inputs for `scripts/retarget_animations.py`. These files are **not**
loaded by the game at runtime; the retarget script bakes the clips into each
avatar GLB under `client/assets/avatars/`.

## Quaternius Universal Animation Library (UAL)

| File | Description |
| --- | --- |
| `AnimationLibrary_Godot_Standard.gltf` | glTF 2.0 JSON with 46 humanoid clips on a Blender Rigify `DEF-*` deform skeleton |
| `AnimationLibrary_Godot_Standard.bin` | Binary buffer referenced by the `.gltf` |

- Author: Quaternius (https://quaternius.com)
- License: **CC0 1.0 Universal** (public domain dedication)
- glTF mirror used for download (pinned in the task spec):
  - https://github.com/J-Ponzo/gltf-universal-animation-library
  - Raw file URLs (non-interactive download with `curl -L`):
    - https://raw.githubusercontent.com/J-Ponzo/gltf-universal-animation-library/main/glTF/AnimationLibrary_Godot_Standard.gltf
    - https://raw.githubusercontent.com/J-Ponzo/gltf-universal-animation-library/main/glTF/AnimationLibrary_Godot_Standard.bin

Clips consumed by the pipeline (target clip name in parentheses):
`Idle_Loop` (idle), `Walk_Loop` (walk), `Sword_Attack` (attack),
`Spell_Simple_Shoot` (cast), `Death01` (death). The `A_TPose` clip is used as
the source rest-pose reference for world-space delta retargeting.

## Avatar sources

All shipped avatars come from the Open Source Avatars collection
(https://github.com/ToxSam/open-source-avatars), license **CC0**; per-avatar
provenance (collection, author, source URL) is recorded in
`client/assets/avatars/manifest.json`.
