# Handheld pilot sources

Original project-authored low-poly sword, hammer and scepter; no downloaded
geometry or textures. `handheld-pilot.blend` is the editable source scene.
Regenerate with:

```sh
/Applications/Blender.app/Contents/MacOS/Blender --background --python scripts/build_handheld_models.py
```

The script creates a separate scene, preserves the pre-existing scene and
exports each prop separately. Exported GLB has Scene0, embedded geometry,
unlit painted materials and hashed `asset.extras.ekza_handheld_v1` metadata.
Origin: centre of the handle; +Y: shaft/blade, +Z: towards the fingertips in the
canonical palm frame; units: metres. Default scale is for a 0.25 m forearm.
Models inherit avatar normalization and bone animation through their parent.

After changing a model, inspect each output with `assimp info`, update its hash
in `client/assets/config/asset_policy.json`, and run the candidate asset audit.
See `docs/handheld-weapons.md` for the SDK import contract and pilot limits.
