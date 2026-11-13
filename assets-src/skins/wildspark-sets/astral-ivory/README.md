# Astral Ivory — Wildspark set

Original community-creator test artwork by OMOBA contributors, generated with
`scripts/build_wildspark_skin.py`. Licence: **CC-BY-4.0** for the model and previews.
No imported models, textures, franchise artwork or external art dependencies.

- **Orbit Repeater:** open porcelain vanes, two gold orbital rings and an exposed
  cyan spindle. The whole forward assembly uses the existing rotor animation.
- **Crystal Lance:** a faceted cyan crystal held in swept porcelain prongs. Its
  forward assembly uses the existing launcher slide/recoil animation.
- Both share a dark grip, gold star inlays and a harp-shaped rear stock.

Upload `astral-ivory-wildspark-set.glb` as a weapon source and `cover.png` as its
cover. Select the OMOBA desktop `handheld-glb-v1` rendition and submit it for
`wildspark.handheld.set`. The hidden launcher variant, identity right-hand grip,
canonical pivots and muzzle positions are embedded in the self-contained GLB.
`manifest.json` records the authored hash, the expected Studio-prepared hash and
the five local validation results. A hosted upload must be checked separately.

Reproducible/editable source: `astral_ivory`, `_astral_chassis` and the
`astral-ivory` material palette in `scripts/build_wildspark_skin.py`.
From the repository root:

```sh
python3 scripts/build_wildspark_skin.py --theme astral-ivory
python3 scripts/build_wildspark_skin.py --theme astral-ivory --verify
```

Use `--registry-root` and `--registry-python` if validators are in another
checkout. Previews are Blender EEVEE renders of the delivered GLB, with both
variants explicitly visible on the cover. They are art previews, not gameplay
evidence. The theme-specific preview exposure does not alter game materials.

Attribution to retain when redistributing:

> Astral Ivory Wildspark set — original geometry by OMOBA contributors
> (generated with scripts/build_wildspark_skin.py); CC-BY-4.0
