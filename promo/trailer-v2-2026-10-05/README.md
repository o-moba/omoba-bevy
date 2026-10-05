# Gameplay trailer v2, October 2026

The second trailer: 1920×1080 at 60 fps, phone interface first, cut to music.
It replaces the footage of the [September trailer](../trailer-2026-09-26/README.md),
which was recorded from build `0.23.0-rc.6` at 1280×720 and about 23 frames per second.

Only the text records live in git. The video, the clean clips, the music and the
rendered stills are in the owner's video archive, in
`015-omoba/omoba-trailer-v2-2026-10-05/`. The trailer is pinned on IPFS and has
played on [omoba.io](https://omoba.io/#trailer) since 2026-10-05; see `ipfs.json`.

| File here | What it is |
| --- | --- |
| `trailer-manifest.json` | Shot list, capture method, music prompt, build and SHA-256 hashes |
| `demo-summary.json` | The recording summary: clips, director events, binary hash |
| `ipfs.json` | Published copy: CID, gateway URL, SHA-256 |
| `motion/` | The motion-graphics source ([Remotion](https://www.remotion.dev/)): timeline, typography, device frame |

## What is on screen

- Real game rendering of build `0.41.0`, driven by scripted ordinary inputs (class
  buttons, a finger dragging the class list, the phone joystick, ATK and ability
  taps, clicks and Q/W/E/R on desktop) in the **offline practice** with server bots.
  Offline practice starts the hero at level 6, so all four abilities are unlocked.
- Phone scenes are the 844×390 phone interface rendered by a desktop development
  build at scale 3 (2532×1170 pixels), not a device recording; every phone scene
  carries that label. Desktop scenes are the 1280×720 layout at scale 2.
- Classes shown and named: Wildspark, Stormfist, Emberveil and Warden on the phone
  layout, Dawnweaver and Frostguard on desktop; the hero-select scene scrolls all 17.
- Titles, the device frame, the close-up lens (a crop of the same footage),
  transitions and callouts are motion graphics added in the edit. Gameplay plays
  at recorded speed and the walks between fights are cut out; only the
  hero-select tour of the reveal scene (0:04–0:12) plays at 1.5×.

## How the footage is captured

Real-time readbacks could not deliver this: the v1 recorder reached about 35
frames per second at 720p and 14 at 1080p. `--profile v2` uses **frame-stepped
capture** instead (`client/src/qa/record_qa.rs`, `OMOBA_RECORD_STEP=1`): game time
advances exactly 1/60 s per rendered frame and every frame is saved, so frame `n`
is at `n / 60` seconds however long it took to render. The in-process offline
practice is stepped by the same clock, so the whole match stays in step. A capture
runs at 5–15 frames per wall-clock second; a one-minute clip takes 5–10 minutes
and the Warden's jungle tour about 25.

The capture does not need focus: it runs on a timer, keeps its window above the
others (parked with `--window-at`, so only a corner is on screen), treats the
window as focused for the game and never moves the real pointer.

## Re-record

```sh
make trailer-clips DEMO_OUTPUT=/Volumes/work/omoba-v2.noindex
# or, with an existing dev client:
python3 scripts/record_demo.py --profile v2 --client-bin target/debug/client \
    --output /Volumes/work/omoba-v2.noindex --window-at=2864,1804 --pace 10
```

Output: `clips/<clip>.mp4` (H.264, 60 fps, capture resolution, no captions),
`demo-manifest.json` and `demo-summary.json`. Raw PNG frames (about 8 GB a clip)
are deleted after encoding unless `--keep-frames` is given; put the output on a
scratch volume and give the folder a `.noindex` suffix so Spotlight leaves the
frames alone. `--clip NAME` re-records single clips and keeps the others.

## Re-edit

The edit is a Remotion composition: `motion/src/Trailer.tsx` (timeline and
scenes), `motion/src/shots.ts` (which seconds of which clip) and
`motion/src/lib.tsx` (palette, device frame, type, transitions).

1. `npm install` in `motion/` (Remotion 4, React 18).
2. Fill `motion/public/` (ignored by git): the music as `music/v2.mp3`, the
   fonts in `fonts/` (`VPUnboundedBlack.ttf`, `VPUnboundedSemiBold.ttf`,
   `VPManropeExtraBold.ttf`, and the game's own `BarlowCondensed-Bold.ttf` and
   `BarlowCondensed-SemiBold.ttf` from `client/assets/ui/verdant/fonts/`; all SIL
   OFL) and the effects in `sfx/` (`whoosh`, `boom`, `riser`, `pop`, as `.mp3`).
3. `python3 prepare.py <folder with the recorder's clips>` extracts the frames
   of every shot in `shots.ts` into `public/gp/<clip>/<frame>.jpg`. The edit
   shows these stills by frame number instead of playing video: Remotion's
   video frame extraction occasionally showed a gameplay frame twice (about one
   in 45–75 frames, also with intra-only copies of the clips), which breaks the
   60 fps cadence the capture guarantees.
4. `node stills.mjs 14 18 46` renders review stills of those seconds;
   `./render.sh` renders `out/omoba-trailer-v2.mp4` (1920×1080, 60 fps, H.264
   BT.709, AAC). `FRAMES=1920-2159 ./render.sh out/check.mp4` renders one scene.
   Use these scripts rather than the Remotion CLI; `bundle.mjs` links `public/`
   into the bundle instead of copying a gigabyte, so keep `TMPDIR` on the same
   volume.
5. The published file is that render with the audio normalised to -14 LUFS
   (two-pass `ffmpeg ... -af loudnorm=I=-14:TP=-1.5:LRA=11:...:linear=true`,
   video stream copied).
6. `python3 manifest.py [<record_demo output dir> ...]` rewrites the manifests.

The music was composed with ElevenLabs Music (the prompt is in the manifest)
under the studio's account; check ElevenLabs' terms before reusing it elsewhere.
