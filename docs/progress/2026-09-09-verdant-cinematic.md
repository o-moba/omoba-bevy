# Verdant Confluence cinematic — 2026-09-09

An actual Blender-rendered presentation of the saved arena: opening panorama,
emerald sanctuary, central bridge and forest, ruined observatory, closing vista.
Five slow camera moves become a 26-second 1080p / 24 fps silent H.264 film, with
four half-second dissolves. There are 672 source renders and 624 final frames.

Delivery directory: `art/verdant-confluence/cinematic/` in the ordinary project
checkout. It contains the MP4, separately saved editable `.blend`, shot plan,
render/encoding manifests, contact sheet and reproduction instructions. Scripts
are in `art/verdant-confluence/scripts/*_cinematic.py`.

The scene uses EEVEE 32 samples and the authored afternoon lighting, with a
restrained AgX contrast/exposure adjustment. The camera copy adds five cameras
and focus empties. It preserves the original 440 mesh geometries, 2,373 original
object transforms/data bindings and all materials. Both original scene files
retain SHA-256 `04653b828e82a5cb5f3306dba8ee9283c8b6908f06c27f1013f1779f1e6076aa`.
No game code, runtime assets, dependency or release version changes are involved.

The task proof is kept in `.agent/tasks/VERDANT-CINEMATIC-2026-09-09/`: frozen
specification, actual Blender logs, start/middle/end previews, final frames,
encoding and full-decode results, independent visual review and final verdict.
The movie is an art presentation, not evidence of runtime graphics or performance.

See `docs/verdant-cinematic.md` for current build/render/encode
commands and how to edit the camera timeline. The 28-second raw Blender timeline
uses camera cuts; the encoder applies the overlapping dissolves for the final
26-second film.

This note records the original five-shot delivery. The retained source plan was
subsequently extended to eight shots / 41.5 seconds; the current reproduction
guide documents that plan. Rendered movies and frames remain local artifacts.
