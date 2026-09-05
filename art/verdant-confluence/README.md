# Verdant Confluence

Original offline 3D environment for OMOBA, art-directed as **Verdant Concord: ruins of a living observatory**. This package stages the next 3D art pass; it is not yet loaded by the game.

## Open and inspect

Open `verdant-confluence.blend` in Blender 5.0 or newer. The active scene is the assembled arena and camera `01 / Atlas overview`. The Outliner separates `01 ENVIRONMENT`, `02 LIBRARY`, and `03 PRESENTATION`. The library is hidden for presentation; enable its viewport monitor to inspect the original asset roots at the origin. Placed objects share the library mesh data.

- `renders/01-overview.png`: entire arena.
- `renders/02-sanctuary.png`: faction sanctuary and gardens.
- `renders/03-river-gameplay.png`: river crossing at gameplay-oriented scale.
- `renders/04-tactical-plan.png`: top-down route/objective layout.
- `library/*.glb`: 24 original reusable architecture and vegetation variants.
- `exports/verdant-confluence.glb`: assembled environment, with named instance roots and materials; excludes presentation lights, cameras, and horizon.
- `manifest.json`: inventory, bounds, triangles, placements, seed, and map anchors.
- `ART_DIRECTION.md`: creative brief and visual acceptance criteria.

## Rebuild

No third-party Python packages or external textures are required. Use an installed Blender executable; run the build in a dedicated factory-startup process because it replaces that process's scene.

```sh
blender --background --factory-startup --threads 1 --python art/verdant-confluence/scripts/build_scene.py
blender --background art/verdant-confluence/verdant-confluence.blend --threads 8 --python art/verdant-confluence/scripts/render_scene.py
python3 art/verdant-confluence/scripts/validate_art.py --output art-validation.json
```

On macOS the executable may be `/Applications/Blender.app/Contents/MacOS/Blender`. Run as the desktop user so Blender can write the project files. `render_scene.py` accepts specific output filenames after `--` to rerender individual cameras. Builder seed is `260905`.

## Next-stage Bevy handoff

One Blender meter equals one current game unit. Blender authoring is Z-up. Export converts `(Blender x, y, z)` to `(Bevy/glTF x, z, -y)`; do **not** rotate the GLB a second time. Asset roots are ground pivots, with a deliberate exception for bridge abutments extending below the walking datum. Actor forward direction is not assigned to static environment props. Architecture `bridge` spans local Blender X, with a 12m-wide traversal deck and a 28m length.

The map is derived from the current `client/src/maps.rs` snapshot: 217.099m square, base centers at game XZ ±79.5495, 46m base pads at 0.7m height, 6m approach ramps, three 12m lane centerlines, an 18m diagonal river, three camp and two boss anchors. The presentation uses a mitered base-ramp skirt to remove overlapping corner slabs; reconcile corner height sampling with the runtime's overlapping-ramp implementation during integration. Pads, central side ramps, and overall ramp reach retain the source dimensions. The visible channel continues to the map boundary; the manifest retains the original river control points. Raised/crowned bridge decks will likewise need an explicit gameplay height decision.

Suggested integration order:
1. Load individual assets using `#Scene0` and the manifest IDs; preserve meter scale and shared handles.
2. Replace the primitive map presentation using the environment export or manifest placements. Retain authoritative routes/objective positions and existing server logic.
3. Add suitable collision/navigation or keep decorative props non-colliding; validate ramps and bridges against gameplay height queries.
4. Recreate sun/ambient lighting and optional moving water in Bevy. Current water is opaque PBR, and crystals contain restrained emissive materials. Blender's AgX view transform, studio softboxes, and denoising are presentation settings outside GLB.
5. Profile actual gameplay, then add LODs, mesh merging, instancing, and culling as warranted. This delivery makes no in-game FPS claim.

## Scope and provenance

All architecture, nature, terrain, and layout geometry was authored for this task. No downloaded models, external textures, or additional production dependencies. Materials are named PBR color/roughness/metallic/emissive values embedded in each export. See `PROVENANCE.md`.

Characters, bosses, combat effects, animations, collision meshes, navigation, LODs, runtime integration, and runtime performance testing belong to the subsequent game task. Existing game files and user assets are not replaced by this art package. The current runtime SemVer is unchanged; this is an unreleased art source delivery.
