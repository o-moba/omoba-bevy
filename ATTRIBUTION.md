# Asset Attribution

Third-party assets bundled in this repository and their licenses.

## Original Omoba 2D assets

The terrain/prop atlases in `client/assets/world2d/` were generated specifically
for Omoba on 2026-07-27 through the Higgsfield MCP with Recraft V4.1, then
processed deterministically into checked-in atlases. No third-party game art
or commercial-game reference was used. The project dedicates the resulting
assets to CC0-1.0. Exact prompts, job/media identifiers, costs, source hashes,
and build records are retained in
`.agent/tasks/TASK-FULL-2D-WORLD/raw/higgsfield/`; the shipped declaration is
`client/assets/world2d/LICENSE.md`.

## Avatars / 3D models

### Paco (`client/assets/downloaded/paco.glb`)

- **Avatar:** Paco (Avatar 211)
- **Collection:** 100Avatars R3
- **Author / creator:** ToxSam (100 Avatars project)
- **Source gallery:** https://www.opensourceavatars.com/en/avatar/60176f95-a780-4e09-85ca-545314182146
- **Original model file:** https://arweave.net/0i-EEnHlcq1EZ1-sMi8DTZhesqGLqtf30WuCknfTHjA (`211_Paco.vrm`)
- **Original format:** VRM 0.x (glTF 2.0 binary)
- **License:** CC0 1.0 (public domain dedication) — no attribution legally required; credited here as good practice.
- **Modification:** none to the model data. The `.vrm` was staged as `.glb`
  (byte-identical glTF 2.0 binary container) via `scripts/convert_vrm_to_glb.py`
  so Bevy's standard glTF loader selects it by file extension. The VRM-specific
  extensions (`VRM`, spring bones, blendshapes) are `extensionsUsed`-only and are
  ignored by the loader.

To re-fetch and re-stage the asset:

```sh
python3 scripts/convert_vrm_to_glb.py \
    https://arweave.net/0i-EEnHlcq1EZ1-sMi8DTZhesqGLqtf30WuCknfTHjA \
    client/assets/downloaded/paco.glb
```

## Current native candidate assets — 0.18.0-rc.2

The current roster is recorded in `client/assets/avatars/manifest.json` (15
heroes) and `client/assets/bosses/manifest.json` (King Mutatio). These retain
creator, collection, source URL and declared CC0 metadata. The reviewed model
hashes and embedded-license expectations are in
`client/assets/config/asset_policy.json`; `scripts/validate_candidate_assets.py`
checks those bytes and metadata in the actual package. This inventory check
is not a substitute for resolving any newly discovered rights information.

El Bueno, Mimic Slime Classic/Water and Wendigo Hollow are **excluded** from
this candidate, including the associated previews. The old CC0 collection
labels were insufficient in light of conflicting embedded/linked permissions;
the historical evidence remains in
`docs/progress/2026-09-05-distribution-review.md`. No license metadata was
rewritten to permit their use. The earlier Paco example above describes an
optional historical import and is not part of the candidate inventory.

Both faction minions and the affected guardian now use original project-authored
geometry/materials and motion from `client/src/creatures3d.rs`.

## Original Verdant Confluence environment

`client/assets/verdant/` is derived from the project-authored Blender source in
`art/verdant-confluence/`. No external mesh or texture was used for this scene.
See `art/verdant-confluence/PROVENANCE.md` and the runtime `manifest.json` for
source hashes, derivation details, normalized walk surfaces and output hashes.
The runtime step preserves the source and is reproducible with
`python3 scripts/stage_verdant.py`.

## Retargeted actor animations

Retained imported models include Quaternius Universal Animation Library clips,
credited as CC0 in `assets-src/animations/README.md`, which records source links,
clip names and retargeting. The native package includes that attribution record.
