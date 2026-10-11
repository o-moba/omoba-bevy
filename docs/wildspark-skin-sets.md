# Wildspark skin sets

A Wildspark set replaces both Wildspark guns, the repeater and the launcher,
with one weapon asset. It is a single static `handheld-glb-v1` GLB that carries
both variants. Studio, the registry and the game treat it as one weapon: one
upload, one curator and owner review, one free claim. The game shows the variant
for the hero's current weapon mode and animates only that variant's mechanism:
the repeater's rotor, or the launcher's slide in rockets mode and during W/R
draws. The runtime rules are in
[handheld weapons](handheld-weapons.md#wildspark-set-repeater-and-launcher-in-one-glb);
this page is the artist's contract and recipe. Submissions target the OMOBA
requirement `wildspark.handheld.set` ("Wildspark — repeater + launcher set").

## Set convention

Coordinates use the canonical grip frame, in metres: origin at the palm grip
point, +Y up the frame, +Z towards the muzzle, +X to the gun's right. Blender
is Z-up, so a canonical point `(x, y, z)` is Blender `(x, −z, y)`. In Blender the
muzzle points along −Y and the palm stays at the origin; the glTF exporter's
+Y Up option converts back.

```text
scene root "wildspark-set"            identity transform
 ├─ WildsparkSetRepeater              identity transform
 │   ├─ any receiver meshes           static
 │   └─ WildsparkRotor                T = (0, 0.12, 0.25), no rotation, scale 1
 │       └─ (rotor geometry)          spins about the node's local Z
 └─ WildsparkSetLauncher              scale 0 in the shipped file
     ├─ any receiver meshes           static
     └─ WildsparkSlide                T = (0, 0.13, 0.20), no rotation, scale 1
         └─ (slide geometry)          recoils 0.095 m along −Z from rest
```

| | Repeater | Launcher |
| --- | --- | --- |
| Moving part pivot | `WildsparkRotor` (0, 0.12, 0.25) | `WildsparkSlide` (0, 0.13, 0.20) |
| Muzzle tip | (0, 0.12, 0.735) | (0, 0.13, 0.81) |
| Baseline bounds | X ±0.17, Y −0.155…0.2575, Z −0.3375…0.735 | X ±0.1825, Y −0.155…0.3705, Z −0.3375…0.81 |
| Allowed bounds (baseline ± 10 % of its extent) | X ±0.204, Y −0.196…0.298, Z −0.444…0.842 | X ±0.219, Y −0.207…0.423, Z −0.452…0.924 |

- The four names `WildsparkSetRepeater`, `WildsparkSetLauncher`,
  `WildsparkRotor` and `WildsparkSlide` must match exactly. Blender silently
  renames duplicates to `WildsparkRotor.001`, which breaks the binding. Other
  node names are free; the generated sets use `WildsparkRepeaterReceiver` and
  `WildsparkLauncherReceiver`.
- Everything that spins goes under `WildsparkRotor`; everything that recoils
  goes under `WildsparkSlide`. Their rest rotation is identity. The game moves
  them relative to the rest transform in the file.
- Both guns share one grip, the identity right-hand grip below. Keep the
  handle around the origin, roughly where the baseline grip is
  (X ±0.04, Y −0.155…0.085, Z ±0.05).
- Geometry ends at the muzzle tip: the front of each gun must lie between
  20 mm behind and 5 mm past the tip along Z. The muzzle flash spawns there.
- The launcher's scale 0 hides it in viewers and older clients that do not know
  the convention; they show the repeater only.

`asset.extras` carries the grip and an optional hint. Keep `asset.copyright`
set to your credit line:

```json
"asset": {
  "version": "2.0",
  "copyright": "<Set name> — <credit>; CC-BY-4.0",
  "extras": {
    "ekza_handheld_v1": {"bone": "rightHand", "offset": [0, 0, 0], "rotation_degrees": [0, 0, 0], "scale": 1},
    "omoba_wildspark_set_v1": {
      "repeater": "WildsparkSetRepeater",
      "launcher": "WildsparkSetLauncher",
      "muzzle": {"repeater": [0, 0.12, 0.735], "launcher": [0, 0.13, 0.81]}
    }
  }
}
```

## Limits

The whole file, both guns together, must stay within `handheld-glb-v1`:

- one scene, at most 64 nodes and 16 meshes, at most 8 MiB, one embedded buffer;
- no skins, armatures or animations, so no preview turntable actions;
- Principled BSDF with Emission Strength ≤ 1. A higher strength makes Blender
  write `KHR_materials_emissive_strength`, which Studio and the registry reject
  (the game alone would accept it). Bright glows come from the emission colour;
- textures embedded as PNG or JPEG, at most 4096 × 4096 each, 64 images and
  32 megapixels in total; no Draco or meshopt compression, no WebP or KTX2;
- extensions only `KHR_materials_unlit` and `KHR_texture_transform`.

## Exporting by hand from Blender

1. Build the tree above with empties: `wildspark-set` at the origin, the two
   variant empties under it, and the meshes under those. Put the
   `WildsparkRotor` origin at Blender (0, −0.25, 0.12) and the `WildsparkSlide`
   origin at (0, −0.20, 0.13). Apply rotation and scale on the moving parts so
   they have none.
2. Keep `WildsparkSetLauncher` at scale 1 while you work and export. Blender's
   exporter writes children of a zero-scale parent with scale 0 and drops their
   translation, which would lose the slide pivot.
3. File → Export → glTF 2.0: format glTF Binary (`.glb`), +Y Up, Apply
   Modifiers; materials exported, images Automatic; compression, animation,
   shape keys and skinning off.
4. Finalize and check the export. This adds both extras and the copyright line,
   sets the launcher to scale 0, then runs every gate:

   ```sh
   python3 scripts/build_wildspark_skin.py --finalize my-export.glb --output my-set.glb \
     --credit "My Wildspark set — by Jane Doe; CC-BY-4.0"
   ```

## Generated sets

`scripts/build_wildspark_skin.py` generates two original sets. Blender 5.0.1
runs headless with `--factory-startup` and an empty scene; all geometry is
built procedurally by the script. It never opens or saves
`assets-src/skills/wildspark-reference.blend` and refuses to write under
`client/` or `assets-src/skills/`. Each theme writes to
`assets-src/skins/wildspark-sets/<theme>/`:

- `<theme>-wildspark-set.glb`, the upload file;
- `cover.png` (640 × 480, both guns), `repeater.png` and `launcher.png`
  (512 × 512), rendered with EEVEE from the shipped GLB on a dark backdrop;
- `manifest.json`, with hashes, node and mesh counts, bounds, licence,
  attribution, grip and every gate result.

```sh
python3 scripts/build_wildspark_skin.py --theme all          # build, render and validate
python3 scripts/build_wildspark_skin.py --verify             # re-check the committed files
python3 -m unittest discover -s scripts -p test_build_wildspark_skin.py -v
```

The export is deterministic: an unchanged script reproduces the same GLB bytes.
EEVEE renders can differ by a few pixels between runs; `--skip-render` keeps the
committed PNGs and only rewrites the GLB and manifest.

| Set | Repeater | Launcher | File |
| --- | --- | --- | --- |
| **Candy Neon** | pink toy frame, gumball hopper, heart sight, donut drum, candy-cane stock with a lollipop butt; three spiral-striped barrels and a star brace on the rotor | mint frame, glowing heart core, spare candy rocket; striped rocket pod with four swept fins, rolled lemon lip, glowing warhead and star sight | 365,640 B, 7 nodes, 4 meshes, 7 materials, no textures |
| **Scrap Punk** | taped pipe grip, bolted plates, hazard-lidded paint-can drum, welded half shroud; four mismatched pipes with hose clamps and a toxic core | fuel tank; square sheet-metal tube with hazard band, warning signs, bolted patch, carry handle and ladder sight; toxic warhead | 627,168 B, 7 nodes, 4 meshes, 9 materials, two 256² tiling textures (rust, grime) |

Both sets keep the baseline pivots, muzzle tips and shared identity grip and fit
the allowed bounds. They are original OMOBA geometry and contain no
franchise-specific designs or character names. Licence: CC-BY-4.0, credited as
in each manifest's `attribution`.

## Validation gates

A build, `--verify`, `--finalize` and `--check FILE…` run the same five gates.
A build records the results in the manifest; every mode exits non-zero if a gate
fails.

| Gate | What runs |
| --- | --- |
| budgets | the limits above, read from the GLB in Python |
| c4 | exact names, hierarchy, pivots and rest pose, launcher scale 0, extras, bounds and muzzle tips |
| studio | ekza-registry `web/app/lib/handheld-glb.ts` `parseHandheld(final=true)` under Node ≥ 22.6 (type stripping) |
| registry | ekza-registry `app.services.studio.validate_model(asset_kind="weapon")`, which runs `handheld.py` and `handheld_checks.py` |
| game | `weapon-build --source FILE --output-dir <fresh scratch>` from this repository's passport crate |

The registry checkout defaults to `../ekza-registry` next to the primary
omoba-bevy checkout (`--registry-root` or `EKZA_REGISTRY_ROOT` override it).
`--registry-python` names a Python with the backend dependencies; by default
that is `<registry-root>/backend/.venv/bin/python`. It runs with `-B`, so no
`__pycache__` is written into the registry checkout. `weapon-build` comes from
the shared cargo cache (`CARGO_TARGET_DIR`, else `<primary checkout>/target`),
built with `cargo build -p omoba-passport --bin weapon-build`. If the binary
predates the newest commit of `passport/src/weapons.rs` or `weapon-build.rs`,
the gate still runs it and records a warning; rebuild and re-run `--verify`.

## Upload and submit in Studio

1. Studio → Publish a model. Model type **Weapon**; pick the set GLB.
2. Grip preparation: keep the embedded grip (right hand, zero offset and
   rotation, scale 1). Check both variants on the hand preview where Studio
   offers a launcher toggle. Studio always uploads its `-prepared.glb`, which
   re-serialises the JSON, so the uploaded sha256 differs from the local file
   even with an unchanged grip. The manifest's `gates.studio.prepared` records
   that exact sha256 and size; review and approve against it. The set hint,
   node names and the launcher's scale 0 survive preparation.
3. Cover image: `cover.png`. Usage license: `CC-BY-4.0`. Attribution: the
   manifest's `attribution` line, or your own credit for your own set.
4. After the curator accepts the source, use **Prepare for Omoba**
   (`desktop / handheld-glb-v1`), then **Submit to Omoba**. Choose the requirement
   "Wildspark — repeater + launcher set" where Studio offers it; through the
   API it is `requirementId: "wildspark.handheld.set"` on the submission. The game
   detects a set by its node names, not by the requirement.
5. The OMOBA owner reviews the exact rendition. An approved free set joins the
   game's weapon catalogue like any other approved handheld; equipped on
   Wildspark, it replaces both guns.
