# Verdant Confluence — cinematic panorama

A silent 41.5-second location film: eight Blender camera moves with seven
half-second dissolves. Delivery is 1920×1080, 24 fps, H.264/yuv420p in an MP4
with fast-start metadata. The full scene is rendered for every frame.

## Files

- `verdant-confluence.mp4`: finished film.
- `verdant-confluence-cinematic.blend`: editable camera and render scene.
- `shots.json`: camera positions, lenses, durations and transition timing.
- `render-manifest.json`: saved Blender version/settings and global shot ranges.
- `verdant-confluence.encoding.json`: frame inventory, exact encoding commands,
  output hash and full-decode/probe results.
- `contact-sheet.jpg`: selected frames from the finished film.

The source `art/verdant-confluence/verdant-confluence.blend` remains unchanged. The cinematic copy
contains the same geometry and materials, plus eight cameras and focus targets.
Lighting uses the existing afternoon sun and studio fill, with AgX Medium High
Contrast and +0.35 exposure. Rendering uses Blender 5.0.1 EEVEE, 32 samples, ray
tracing and screen-space indirect lighting. No external textures are required.

## Edit in Blender

Open `verdant-confluence-cinematic.blend`. The collection
`04 CINEMATIC / animated presentation` contains the cameras and focus empties.
Timeline markers switch cameras. Adjust camera location keys, the focus target,
lens or orthographic scale to reframe a shot. Bezier camera keys ease the motion
at both ends, and tracking constraints keep the horizon level.

The editable timeline contains 1080 frames / 45 seconds before transition
overlaps. Encoding overlaps adjacent shots by 12 frames, giving 996 frames /
41.5 seconds. The timeline itself previews direct cuts; the encoder applies the
dissolves. Changing `shots.json` and rebuilding replaces edits to the cinematic
copy, so preserve a separately named copy if editing its cameras by hand.

## Reproduce from the repository root

Use an installed Blender 5.0+ and ffmpeg/ffprobe; no Python packages are needed.
On macOS the executable is usually `/Applications/Blender.app/Contents/MacOS/Blender`.

```sh
BLENDER_BIN=/Applications/Blender.app/Contents/MacOS/Blender

# Open the original in a separate background process, add cameras, save a copy.
"$BLENDER_BIN" --background art/verdant-confluence/verdant-confluence.blend \
  --python-exit-code 1 --python art/verdant-confluence/scripts/build_cinematic.py

# Preview the beginning, middle and end of each shot at half resolution.
"$BLENDER_BIN" --background art/verdant-confluence/cinematic/verdant-confluence-cinematic.blend \
  --python-exit-code 1 --python art/verdant-confluence/scripts/render_cinematic.py -- \
  --preview --output art/verdant-confluence/cinematic/previews

# Render all full-resolution frames. This is the expensive step.
"$BLENDER_BIN" --background art/verdant-confluence/cinematic/verdant-confluence-cinematic.blend \
  --python-exit-code 1 --python art/verdant-confluence/scripts/render_cinematic.py -- \
  --output art/verdant-confluence/cinematic/frames

# Choose a new filename: encoding preserves an existing movie.
python3 art/verdant-confluence/scripts/encode_cinematic.py \
  --frames art/verdant-confluence/cinematic/frames \
  --plan art/verdant-confluence/cinematic/shots.json \
  --output art/verdant-confluence/cinematic/verdant-confluence-rerender.mp4

# Optional 9:16 version for blog/social posts.
ffmpeg -hide_banner -i art/verdant-confluence/cinematic/verdant-confluence-rerender.mp4 \
  -vf "scale=1080:1920:force_original_aspect_ratio=increase,crop=1080:1920" \
  -c:v libx264 -preset slow -crf 18 -pix_fmt yuv420p -r 24 -an -movflags +faststart \
  art/verdant-confluence/cinematic/verdant-confluence-rerender-9x16.mp4
```

`--shot 03-river` limits rendering to one shot. `--resume` skips existing PNGs;
use it only with an unchanged scene and render setup. Frames are numbered from
one inside each shot directory. The encoder validates every filename, dimension
and hash, then verifies the final frame count and decodes the entire movie.

The delivered film has no music, narration or interface overlays. It presents
the Blender art scene; it is not a recording of live gameplay.
