# Illustrated skill actions — 2026-09-29

All 64 active skills across the 16 classes were generated for OMOBA through
Higgsfield **GPT Image 2.5**, variant `flare`, quality `high`. Sixteen 2048×2048
sheets contain four independently composed skills each. Longshot and Sheltering
Leap were refined as individual 1024×1024 images after the small-size review.
No reference images, third-party game artwork or character likenesses were used.
The checked-in `manifest.json` records the exact prompts, completed job IDs,
source dimensions and SHA-256 digests, skill order and final atlas hashes.

## Runtime layout

Each cell is 256×256 RGBA with an antialiased circular alpha mask and a two-pixel
inset. Desktop and touch use the same art; the existing UI supplies key labels,
cooldowns, rank rings, resource states and inspection descriptions.

| Atlas | Rows, in order | Size |
|---|---|---|
| skills-atlas.png | Warrior, Mage, Ranger, Cleric, Warden | 1024×1280 |
| standard-skills.png | Dawnweaver, Wildspark | 1024×512 |
| roster-skills.png | Cinderforge, Edgeweaver, Stormfist, Veilstalker, Emberveil, Orbitwright, Riftshot, Chainkeeper, Frostguard | 1024×2304 |

Columns are Q, W, E, R. `client/src/skill_icons.rs` retains the existing stable
ability-ID mapping; no gameplay behavior or protocol changes accompany this art.

## Rebuild and check

Retrieve the approved Higgsfield outputs by the job IDs in the manifest and save
them using the corresponding `file` names in a local source directory. Original
source files and visual proofs for this run are retained locally under
`.agent/tasks/SKILL-ART-2026-09-29/raw/`; the game ships only the three atlases.

```sh
python3 scripts/build_skill_icons.py --sources /path/to/approved/sources
python3 scripts/build_skill_icons.py --check
python3 scripts/build_skill_icons.py --contact-sheet /path/to/contact-sheet.png
```

The developer utility uses the existing Pillow dependency, verifies source
hashes, splits 2×2 sheets in reading order, applies approved single-icon overrides,
resizes with Lanczos and packs the atlases. It performs no generation, painting
or placeholder substitution. Validation checks coverage of all active skills,
digests, dimensions, unique cells and transparent corners. Review the generated
64px contact sheet and native UI in addition to these structural checks.

The former geometric glyph generator and unused eight-skill SVG are retired;
Git history preserves the earlier artwork. See `LICENSE.md` for the project's
CC-BY-4.0 grant and generated-art qualification.
