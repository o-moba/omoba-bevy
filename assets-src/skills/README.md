# Standard skill props

Original low-poly pilot assets created for OMOBA in Blender 5.0.1. No downloaded
geometry or textures. Project license applies. `standard-skill-props.blend`
contains editable `rocket` and `trap` objects in a separate scene.

Regenerate from the repository root:

```sh
/Applications/Blender.app/Contents/MacOS/Blender --background --python scripts/build_standard_skill_models.py
assimp info client/assets/cosmetics/standard/rocket.glb
assimp info client/assets/cosmetics/standard/trap.glb
```

The script creates its own scene without deleting existing objects. GLB Scene0
contains only the selected prop at its local origin; source layout offsets are
applied after export. Runtime coordinates are Y up, +Z forward. No armature,
external buffers, textures or runtime download is required.

| Prop | Vertices | Triangles | Materials | Runtime bounds before scale |
| --- | ---: | ---: | ---: | --- |
| Rocket | 344 | 194 | 4 | X/Y ±0.47; Z −0.79…0.96 |
| Trap | 400 | 200 | 3 | X/Z ±0.455; Y 0…0.4 |

Materials use a dark steel / brass / orange palette and a cyan rocket engine.
The client adds team outlines and a procedural fallback while a GLB is missing
or loading. These are world objects; hand-held weapons, grip corrections and
socket binding remain separate work in the visual uplift TODO.

## Roster world props

`roster-skill-props.blend` contains five original project-authored props with no
external textures. Their unlit painted materials preserve color under strong
arena daylight. Regenerate with Blender's `--background --python
scripts/build_roster_skill_models.py`. Each exported GLB was inspected with
`assimp info`; hashes are recorded in `client/assets/config/asset_policy.json`.

| Prop | Vertices | Triangles | Materials | Runtime bounds |
| --- | ---: | ---: | ---: | --- |
| Hook | 200 | 98 | 2 | X −0.44…0.48; Y ±0.12; Z −0.49…0.59 |
| Lantern | 232 | 120 | 3 | X/Z ±0.42; Y 0…1.2 |
| Sphere | 200 | 106 | 3 | X/Z ±0.48; Y ±0.4 |
| Pillar | 192 | 84 | 2 | X ±0.72; Y −0.08…1.7; Z ±0.65 |
| Colossus | 160 | 76 | 3 | X ±0.9; Y 0.01…1.44; Z −0.65…0.79 |

The pillar's small buried skirt prevents a floating base on terrain. Source
objects are spaced for editing only after export, keeping runtime origins local.


## Wildspark reference models (October 9)

For current Wildspark art, run `scripts/build_wildspark_models.py` with Blender
5.0.1 in background mode. It saves `wildspark-reference.blend` and exports five
Scene0 GLBs: `weapons/wildspark-repeater.glb`, `weapons/wild-launcher.glb`,
`weapons/wild-rocket.glb`, `cosmetics/standard/rocket.glb`, and
`cosmetics/standard/trap.glb`. This generator supersedes the old generators for
these paths; it leaves the legacy `wild-repeater.glb` used by Riftshot untouched.

Original geometry/materials, no downloads or textures; CC-BY-4.0, attribution
OpenMoba contributors. Exports embed attribution and handheld grip metadata.
The three named moving assemblies preserve canonical Y-up/Z-forward pivots;
model bounds remain within the runtime collision-envelope presentation limits.
Policy hashes are in `client/assets/config/asset_policy.json`. Regenerating art
requires rechecking those hashes and inspecting the GLBs with `assimp info`.
