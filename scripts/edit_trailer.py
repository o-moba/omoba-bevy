#!/usr/bin/env python3
"""Cut a short music trailer from the clips recorded by record_demo.py.

The recorded clips stay real-time, real-input footage; this step only edits:
it picks shots by the director's caption events (EDIT below), crossfades them,
adds lower-third titles, a cold-open logo and an end card, and lays music
under the picture. Music is either a local file (--music) or generated with
the ElevenLabs Music API (--generate-music, reads ELEVENLABS_API_KEY).

    python3 scripts/record_demo.py --build --output /tmp/omoba-demo
    python3 scripts/edit_trailer.py --demo /tmp/omoba-demo --generate-music

Output in the demo directory: `omoba-trailer.mp4` (1280x720, H.264 + AAC,
-14 LUFS), `trailer-manifest.json` (shots, music source/prompt, hashes) and
`trailer/` with the clean clips and shots.
"""
import argparse
import datetime
import json
import os
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import record_demo  # noqa: E402
from record_demo import FONT, FPS, HEIGHT, WIDTH, clip_span, encode_clip, escape, sha256, warp  # noqa: E402

XFADE = 0.5
# (clip, caption event, offset seconds after it, shot seconds, title, subtitle).
# An empty title means no lower third; `None` event means clip start.
EDIT = [
    ("desktop", "Abilities", 2.0, 4.0, "", ""),
    ("desktop", "Five classes", 0.4, 5.6, "Five classes", "One role for every player"),
    ("desktop", "Sanctuary shop", 0.3, 3.6, "Build your hero", "Items bought at your base"),
    ("desktop", "Push the lane", 0.5, 4.2, "Push the lanes", "Waves meet in the middle"),
    ("desktop", "Targeting", 0.4, 8.5, "Lock your target", "Click locks · right-click attacks"),
    ("desktop", "Abilities", 0.0, 7.5, "Four abilities", "Q W E R · unlock as you level"),
    ("jungle", "Jungle camps", 0.3, 8.5, "Warden jungler", "Faster camps, more gold"),
    ("phone", "Phone controls", 0.2, 3.8, "Built for phones too", "joystick and thumb controls"),
    ("phone", "Touch combat", 0.3, 7.5, "Touch combat", "phone interface preview"),
]
COLD_OPEN = ("O-MOBA", "An open-source MOBA")
END_CARD = ("O-MOBA", "Play · Create · Contribute", "omoba.io  ·  discord.gg/DMhvaVpj7Q  ·  github.com/o-moba")
END_SECONDS = 4.5
MUSIC_PROMPT = (
    "Instrumental game trailer music for a bright fantasy team battle game: punchy modern "
    "drums, driving synth bass, heroic brass and string stabs, playful arpeggios, a short "
    "rising intro, confident energetic middle, and a clean final hit with a short tail. "
    "About 120 BPM, no vocals.")


def run(args, **kwargs):
    subprocess.run([record_demo.FFMPEG, "-y", "-loglevel", "error", *args], check=True, **kwargs)


def lower_third(title, subtitle, seconds, phone=False):
    """Desktop: accent bar and two short lines at the lower left, clear of the
    centred skill bar. Phone: one line in the top letterbox band, clear of the
    joystick and thumb buttons. Fades in after 0.25 s."""
    if not title:
        return []
    font = str(FONT).replace(":", "\\:")
    fade = "if(lt(t,0.25),0,if(lt(t,0.6),(t-0.25)/0.35,if(gt(t,{e}),max(0,({s}-t)/0.35),1)))".format(
        e=seconds - 0.35, s=seconds)
    show = f"enable='between(t,0.25,{seconds:.2f})'"
    if phone:
        line = escape(f"{title} — {subtitle}" if subtitle else title)
        return [f"drawtext=fontfile='{font}':text='{line}':fontsize=30:fontcolor=white:"
                f"x=(w-tw)/2:y=18:alpha='{fade}'"]
    return [
        f"drawbox=x=36:y=h-122:w=5:h=74:color=0x22d3ee@0.95:t=fill:{show}",
        f"drawtext=fontfile='{font}':text='{escape(title)}':fontsize=36:fontcolor=white:"
        f"shadowcolor=black@0.8:shadowx=2:shadowy=2:x=54:y=h-124:alpha='{fade}'",
        f"drawtext=fontfile='{font}':text='{escape(subtitle)}':fontsize=20:fontcolor=0xd8f6ff:"
        f"shadowcolor=black@0.8:shadowx=1:shadowy=1:x=56:y=h-76:alpha='{fade}'",
    ]


def centred_title(title, subtitle, seconds, extra=""):
    font = str(FONT).replace(":", "\\:")
    fade = "if(lt(t,0.3),t/0.3,if(gt(t,{e}),max(0,({s}-t)/0.4),1))".format(e=seconds - 0.4, s=seconds)
    filters = [
        f"drawtext=fontfile='{font}':text='{escape(title)}':fontsize=96:fontcolor=white:"
        f"shadowcolor=black@0.8:shadowx=3:shadowy=3:x=(w-tw)/2:y=h/2-96:alpha='{fade}'",
        f"drawtext=fontfile='{font}':text='{escape(subtitle)}':fontsize=32:fontcolor=0x22d3ee:"
        f"shadowcolor=black@0.8:shadowx=2:shadowy=2:x=(w-tw)/2:y=h/2+18:alpha='{fade}'",
    ]
    if extra:
        filters.append(f"drawtext=fontfile='{font}':text='{escape(extra)}':fontsize=24:fontcolor=white@0.85:"
                       f"x=(w-tw)/2:y=h/2+84:alpha='{fade}'")
    return filters


def cut_shot(clean, at, seconds, filters, output):
    vf = ",".join(["fps=%d" % FPS, "format=yuv420p", *filters]) if filters else f"fps={FPS},format=yuv420p"
    run(["-ss", f"{at:.3f}", "-i", str(clean), "-t", f"{seconds:.3f}", "-an", "-vf", vf,
         "-c:v", "libx264", "-crf", "18", "-preset", "medium", "-pix_fmt", "yuv420p", str(output)])


def end_card(output):
    title, subtitle, extra = END_CARD
    filters = centred_title(title, subtitle, END_SECONDS, extra)
    run(["-f", "lavfi", "-i", f"color=c=0x05060c:s={WIDTH}x{HEIGHT}:r={FPS}:d={END_SECONDS}",
         "-vf", ",".join(["format=yuv420p", *filters]), "-c:v", "libx264", "-crf", "18",
         "-pix_fmt", "yuv420p", str(output)])


def generate_music(seconds, output, prompt):
    key = os.environ.get("ELEVENLABS_API_KEY")
    if not key:
        raise SystemExit("ELEVENLABS_API_KEY is not set")
    body = json.dumps({"prompt": prompt, "music_length_ms": int(seconds * 1000),
                       "force_instrumental": True})
    # curl reads the key from stdin, so it never appears in the process list.
    config = "".join(f"{k} = {json.dumps(v)}\n" for k, v in [
        ("url", "https://api.elevenlabs.io/v1/music?output_format=mp3_44100_192"),
        ("request", "POST"), ("header", "xi-api-key: " + key),
        ("header", "Content-Type: application/json"), ("data", body),
        ("output", str(output)), ("max-time", "600"), ("retry", "2"), ("fail-with-body", "")])
    result = subprocess.run(["curl", "-q", "--silent", "--show-error", "--config", "-"],
                            input=config.replace('fail-with-body = ""', "fail-with-body"),
                            text=True, capture_output=True)
    if result.returncode or not output.is_file() or output.stat().st_size < 10_000:
        detail = output.read_text(errors="replace")[:300] if output.is_file() else result.stderr
        output.unlink(missing_ok=True)
        raise SystemExit(f"ElevenLabs music request failed: {detail.strip()}")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--demo", type=Path, required=True, help="record_demo.py output directory")
    parser.add_argument("--music", type=Path, help="music file to lay under the trailer")
    parser.add_argument("--generate-music", action="store_true", help="compose music with ElevenLabs")
    parser.add_argument("--music-prompt", default=MUSIC_PROMPT)
    args = parser.parse_args()
    record_demo.FFMPEG = record_demo.find_ffmpeg()
    if not record_demo.FFMPEG:
        parser.error("ffmpeg with drawtext is required (brew install ffmpeg-full, or set FFMPEG)")
    demo = args.demo.resolve()
    manifest = json.loads((demo / "demo-manifest.json").read_text())
    clips = {clip["clip"]: clip for clip in manifest["clips"] if clip.get("pass")}
    work = demo / "trailer"
    work.mkdir(exist_ok=True)

    clean = {}
    for name, record in clips.items():
        clean[name] = work / f"clean-{name}.mp4"
        if not clean[name].exists():
            encode_clip(record, demo / "raw" / name, clean[name], captions=False)

    shots, plan = [], []
    for index, (name, event, offset, seconds, title, subtitle) in enumerate(EDIT):
        record = clips.get(name)
        if not record:
            print(f"[trailer] skipping {name}: clip not recorded", flush=True)
            continue
        start, _ = clip_span(record)
        pace = record.get("pace", [])
        marker = next((e for e in record["events"] if e["title"] == event), None)
        at = warp(pace, start, max(marker["seconds"], start)) + offset if marker else offset
        filters = lower_third(title, subtitle, seconds, phone=name == "phone")
        if index == 0:
            filters = ["eq=brightness=-0.08:saturation=1.15", *centred_title(*COLD_OPEN, seconds)]
        shot = work / f"shot-{index:02d}.mp4"
        cut_shot(clean[name], at, seconds, filters, shot)
        shots.append((shot, seconds))
        plan.append(dict(clip=name, event=event, at=round(at, 2), seconds=seconds, title=title))
    card = work / "end-card.mp4"
    end_card(card)
    shots.append((card, END_SECONDS))

    # Crossfade chain: each xfade starts XFADE before the running end.
    inputs, chain, offset = [], [], 0.0
    for shot, _ in shots:
        inputs += ["-i", str(shot)]
    label = "[0:v]"
    for index, (_, seconds) in enumerate(shots[:-1]):
        offset += seconds - XFADE
        out = f"[v{index + 1}]"
        chain.append(f"{label}[{index + 1}:v]xfade=transition=fade:duration={XFADE}:offset={offset:.3f}{out}")
        label = out
    total = offset + shots[-1][1]
    picture = work / "picture.mp4"
    run([*inputs, "-filter_complex", ";".join(chain), "-map", label, "-c:v", "libx264", "-crf", "18",
         "-preset", "medium", "-pix_fmt", "yuv420p", str(picture)])

    music_info = None
    music = args.music.resolve() if args.music else None
    if args.generate_music and not music:
        music = work / "music.mp3"
        if not music.exists():
            print(f"[trailer] composing {total:.0f}s of music with ElevenLabs ...", flush=True)
            generate_music(total + 1.0, music, args.music_prompt)
        music_info = dict(source="ElevenLabs Music API (/v1/music, force_instrumental)",
                          prompt=args.music_prompt, file=music.name, sha256=sha256(music))
    elif music:
        music_info = dict(source="local file", file=str(music), sha256=sha256(music))

    video = demo / "omoba-trailer.mp4"
    if music:
        audio = (f"[1:a]atrim=0:{total:.3f},afade=t=in:d=0.3,afade=t=out:st={total - 2.5:.3f}:d=2.5,"
                 "loudnorm=I=-14:TP=-1.5:LRA=11[a]")
        run(["-i", str(picture), "-i", str(music), "-filter_complex", audio, "-map", "0:v", "-map", "[a]",
             "-c:v", "copy", "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart", "-shortest", str(video)])
    else:
        run(["-i", str(picture), "-c", "copy", "-movflags", "+faststart", str(video)])
    summary = dict(generator="scripts/edit_trailer.py", edited_on=datetime.date.today().isoformat(),
                   source=manifest.get("source"), recorded_on=manifest.get("recorded_on"),
                   footage="real-time native captures with scripted inputs; walks timelapsed and badged",
                   shots=plan, crossfade_seconds=XFADE, seconds=round(total, 2), music=music_info,
                   video=dict(file=video.name, bytes=video.stat().st_size, sha256=sha256(video)))
    (demo / "trailer-manifest.json").write_text(json.dumps(summary, indent=2, ensure_ascii=False) + "\n")
    print(f"[trailer] {video} ({total:.1f}s)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
