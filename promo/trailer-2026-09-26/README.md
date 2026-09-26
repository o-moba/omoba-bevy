# Gameplay trailer, September 2026

The 53-second trailer on [omoba.io](https://omoba.io/#trailer) and its sources.
Footage is real-time capture of the `0.23.0-rc.6` build in local practice
matches with server bots, driven by scripted ordinary inputs
([`scripts/record_demo.py`](../../scripts/record_demo.py) and
`client/src/qa/demo_qa.rs`). Walks are sped up 4× and badged. Phone scenes show
the phone interface rendered by a desktop development build, not a device.

| File | What it is |
| --- | --- |
| `omoba-trailer.mp4` | Final trailer: 1280×720 H.264, AAC, 53 s |
| `omoba-demo-captioned.mp4` | Long cut (1:55) with burned-in captions, no music |
| `poster.jpg` | Poster frame used on the website |
| `trailer-manifest.json` | Shot list, music prompt and hashes |
| `demo-summary.json` | Recording summary: clips, captions, timelapse markers |
| `ipfs.json` | Published copy: CID, gateway URL, SHA-256 |
| `sources/trailer/clean-*.mp4` | The three recorded clips without captions: the edit sources |
| `sources/trailer/music.mp3` | Music (ElevenLabs Music, instrumental) |
| `sources/demo-manifest.json` | Frame timings, caption events and timelapse markers of each clip |
| `sources/raw/<clip>/` | Director events and the frame timing log of each clip |

## Rebuild or re-edit

Change shots or titles in the `EDIT` table of
[`scripts/edit_trailer.py`](../../scripts/edit_trailer.py), then run it on a
copy of `sources/` (it writes shots and the output next to them):

```sh
cp -R promo/trailer-2026-09-26/sources /tmp/trailer-sources
python3 scripts/edit_trailer.py --demo /tmp/trailer-sources --music /tmp/trailer-sources/trailer/music.mp3
```

With the committed sources and music this reproduces `omoba-trailer.mp4`
exactly. To record new footage, run `make demo-video` (needs a GPU window and
ffmpeg with drawtext, `brew install ffmpeg-full`), then
`python3 scripts/edit_trailer.py --demo builds/demo --generate-music`
(`ELEVENLABS_API_KEY`).

The music was generated with ElevenLabs Music under the studio's account; check
ElevenLabs' terms before reusing it outside this project.
