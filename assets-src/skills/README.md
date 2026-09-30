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
