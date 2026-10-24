# Jade Fortune — community sculpture

An original lucky waving cat for the community sculpture pilot. The porcelain,
jade and gold geometry is generated locally; it contains no imported artwork,
textures, fonts or downloaded models.

The model and cover are licensed **CC-BY-4.0**. Retain this attribution:

> Jade Fortune — original sculpture by OMOBA contributors; CC-BY-4.0.

Files:

- `lucky-cat.glb`: self-contained, Y-up GLB with one two-second `Wave` animation.
- `lucky-cat.blend`: editable geometry, materials, paw animation and preview setup.
- `cover.png`: rendered model preview, not a gameplay screenshot.
- `manifest.json`: exact GLB hash, byte size and provenance.
- `../../../scripts/build_lucky_cat.py`: reproducible generator (from the repository
  root, use `scripts/build_lucky_cat.py`).

Run from the repository root with Blender 5:

```sh
/Applications/Blender.app/Contents/MacOS/Blender --background --factory-startup \
  --python scripts/build_lucky_cat.py
assimp info assets-src/community/lucky-cat/lucky-cat.glb
```

The canonical GLB remains here as source and a test fixture; the generator does
not copy it into the packaged client assets. The normal client and opt-in local
art scene load it through the Registry's authorized download path. Public
placement requires Studio upload, technical validation, curator publication,
OMOBA project approval and approval of the placement request.

Upload the GLB as a `map_prop` and the cover as its thumbnail. Select the OMOBA
`desktop/map-prop-glb-v1` profile. No humanoid rig or weapon grip is required.
Do not replace this asset with an avatar or weapon type to bypass review.
