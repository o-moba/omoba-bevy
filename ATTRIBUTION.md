# Asset Attribution

Third-party assets bundled in this repository and their licenses.

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
