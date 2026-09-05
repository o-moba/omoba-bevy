---
name: omoba-sprite-character
description: Produce or extend Omoba playable 2D character sprites with Higgsfield, especially static pixel-art masters animated through Seedance and converted into validated idle/run/attack/cast/hit/death atlases. Use for new heroes, replacement animation states, sprite-sheet QA, portrait integration, or Higgsfield provenance work in this repository.
---

# Omoba Sprite Character

Produce one character end to end without changing gameplay, server authority, map, or renderer architecture. Treat the sprite manifest and validator as runtime contracts, not suggestions.

## Preferred branch: `seedance-video-to-spritesheet`

1. Read the active frozen task spec and `client/assets/sprites/manifest.json`. Claim one stable character ID and do not edit another designer's files.
2. Capture baseline hashes for every existing character sheet before editing shared manifests or atlases.
3. Use Higgsfield MCP exclusively for artistic source generation:
   - call `models_explore(action="recommend")` for the static master and video model;
   - call generation with `get_cost:true` before every new model/quality batch;
   - generate one full-body pixel-art master on pure white, with one character, generous clearance, no floor/contact shadow, text, watermark, scenery, crop, or extra subject;
   - approve identity, silhouette, palette, value grouping, camera, facing, light, pixel density, scale, and animation envelope before video generation;
   - use the approved master as a Higgsfield Seedance reference for six separate 720p, 1:1, silent, locked-camera clips: `idle`, `run`, `attack`, `cast`, `hit`, and `death`;
   - start with Seedance 2.0 `fast`; retry only failed states with stronger prompting or `std` quality.
4. Author motion semantics explicitly:
   - `idle`: subtle readable breathing/sway, seamless 8→1 loop;
   - `run`: clear locomotion cycle with contact/passing/air phases, seamless 8→1 loop;
   - `attack`: anticipation, acceleration, contact, overshoot, recovery;
   - `cast`: gather, compact release, recoil, settle;
   - `hit`: impact compression/recoil, secondary lag, recovery;
   - `death`: progressive non-gory collapse into a nonempty identifiable final hold.
5. Keep every clip on clean white with fixed framing, scale, lighting, facing, palette, and camera. Prohibit cuts, pan, zoom, motion blur that destroys the pixel clusters, shadows, background texture, cropped parts, duplicated subjects, and free-floating effects outside the safe envelope.
6. Poll each job with `job_display`. Preserve exact provider responses. Download and retain approved masters, all source clips, failed attempts used to justify retries/fallbacks, and returned thumbnails.
7. Select eight intentional source timecodes per state. Do not choose uniformly if that misses anticipation/contact/passing/hold poses. Record every runtime frame's source job, clip, timecode, and source frame number.
8. Run `scripts/seedance_to_row.sh` for deterministic extraction, white-matte removal, common-sequence crop, nearest-neighbor scaling, stable baseline registration, and 8×1 row assembly. Inspect matte edges at gameplay scale; the script is not a substitute for visual QA.
9. Run `scripts/pack_character_atlas.sh` to build the 8×2 locomotion and 8×4 action sheets.
10. Add the manifest entry and portrait only after all six rows pass QA. Run the repository validator, focused Rust tests, contact-board review, and the task proof loop.

Use direct Higgsfield grid generation only for one specific state after documented Seedance attempts failed. Record failed job IDs, exact defect, fallback decision, fallback prompt/cost, and cell mapping. Never use the fallback only to save time or credits.

## Minimum runtime contract

Each playable character must ship:

- `client/assets/sprites/<id>.png`: RGBA `2048x512`, 8 columns × 2 rows, 256px cells; `idle`, then `run`.
- `client/assets/sprites/actions/<id>.png`: RGBA `2048x1024`, 8 columns × 4 rows; `attack`, `cast`, `hit`, then `death`.
- One 256×256 selection portrait in roster order.
- One manifest entry with safe relative paths, display name, theme, palette, CC0-1.0, exact provenance, pivot, and world height.

Use these minimum animation definitions unless a frozen task explicitly strengthens them:

| State | Frames | FPS | Playback |
| --- | ---: | ---: | --- |
| idle | 8 | 6 | loop |
| run | 8 | 12 | loop |
| attack | 8 | 12 | once |
| cast | 8 | 10 | once |
| hit | 8 | 14 | once |
| death | 8 | 8 | hold last |

Runtime priority is `death > hit > attack/cast > run > idle`. Every cell must be nonempty and visually distinct, with no visible pixel inside the two-pixel cell boundary. Keep identity, camera, light, palette, apparent scale, foot/hover baseline, and pivot stable across all 48 frames. Do not accept recolor-only, translation-only, cloned, or mechanically interpolated frames.

## Prompt skeleton

Describe the frozen identity first, then the single state and its eight-phase motion. Include:

```text
One complete original pixel-art game character, fixed front three-quarter/top-down game camera,
locked framing and scale, crisp intentional pixel clusters, pure #FFFFFF background, no shadow.
Preserve exactly: [silhouette, anatomy/prop count, palette, facing, light, pivot].
Animate only this state: [state-specific anticipation/event/recovery or loop phases].
The first and last pose [join seamlessly / form start and final hold].
No camera movement, cuts, zoom, floor, scenery, text, logo, watermark, extra subject,
cropping, identity drift, palette drift, soft painterly blur, or effects outside the safe margin.
```

For idle/run, explicitly request a loop and inspect the final-to-first seam. For attack/cast/hit, select an event frame and recovery. For death, require an opaque, recognizable final pose rather than disappearance.

## Provenance and review gates

Store task-local raw evidence beneath `.agent/tasks/<TASK_ID>/raw/<id>/`. Record:

- designer agent/task name and review approver;
- full positive/negative prompts and exposed parameters;
- exact MCP tool, model/version, request/job/media IDs, status, timestamps, and returned URLs;
- exact preflight/actual credit payloads and checked totals;
- raw master/clip/frame/sheet SHA-256 hashes;
- clip dimensions, duration, FPS, eight chosen timecodes/source frames per state;
- every deterministic cleanup/packing command and input/output hash;
- original-work declaration and CC0-1.0 dedication.

Do not infer a missing provider field. Preserve the raw response and leave the criterion non-PASS until resolved.

Approve each state only when identity, camera, palette, pixel density, silhouette, alpha matte, safe margin, motion arc, baseline/pivot stability, state readability, and gameplay-size readability all pass. Idle/run additionally require a clean 8→1 seam; death requires a usable final hold. Generate contact boards from final shipped cells, never from superseded sources.

## Integration and verification

- Append roster entries; never rename, reorder, or regenerate existing characters unless the frozen task says so.
- Derive portrait/selection counts from the manifest; do not add another hard-coded roster size.
- Keep all new art offline and allow-listed. Do not add runtime downloads or production dependencies.
- Run `scripts/validate_sprite_assets.py --self-test`, focused sprite/selection/network tests, workspace fmt/build/test/clippy, size/hash checks, and `git diff --check`.
- Follow the repository proof loop and obtain a fresh verifier verdict. Do not claim completion with any non-PASS criterion.

## Script usage

```bash
.claude/skills/omoba-sprite-character/scripts/seedance_to_row.sh \
  source.mp4 idle-row.png "0.12,0.54,0.98,1.42,1.86,2.30,2.74,3.18"

.claude/skills/omoba-sprite-character/scripts/pack_character_atlas.sh \
  rows/ client/assets/sprites/<id>.png client/assets/sprites/actions/<id>.png
```

The extraction script requires `ffmpeg`, `ffprobe`, and ImageMagick 7 `magick`. It uses one crop/scale transform for the whole eight-frame sequence to avoid per-frame zoom jitter. Review the generated `*.frames.tsv` mapping and final cells manually.
