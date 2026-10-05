# Handheld pilot sources

Original project-authored low-poly sword, hammer and scepter; no downloaded
geometry or textures. `handheld-pilot.blend` is the editable source scene.
The visual source and its three exported props are **CC-BY-4.0**, credited to
**Open Moba contributors**. See the [scoped handheld license](../../client/assets/weapons/LICENSE.md)
and [repository license map](../../LICENSING.md). The generator script and
manifest metadata retain MPL-2.0; imported or downloaded assets keep their own terms.

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

Adventurer dagger uses the same canonical grip with a shorter 0.53 m blade-tip
extent and a distinct diamond profile. Regenerate only this isolated source:
`/Applications/Blender.app/Contents/MacOS/Blender --background --factory-startup --python scripts/build_dagger_model.py`.
It writes `dagger.blend` and `dagger.glb`, preserving the pilot armory source.

Original **Wildspark Repeater**, **Wildspark Rocket Launcher**, **Verdant Bow**
and **Wildspark Rocket** (`wild-repeater.glb`, `wild-launcher.glb`,
`verdant-bow.glb`, `wild-rocket.glb`) and `ranged-handhelds.blend` are CC-BY-4.0,
credited to Open Moba contributors. Regenerate using Blender and
`scripts/build_ranged_handhelds.py`. No external geometry or textures.
Gun muzzles point along canonical palm +Z; bow limbs follow +Y.
