#!/usr/bin/env python3
"""Record a short gameplay demo video from real native matches.

Each clip runs a local practice server (5v5 with server bots) and a dev client
with the demo director (client/src/qa/demo_qa.rs) and the frame recorder
(client/src/qa/record_qa.rs). The director plays ordinary inputs: class
buttons, the shop, move orders, Tab targeting, right-click attacks, ability
keys and phone touches. Frames carry wall-clock timestamps, so the video keeps
real-time pacing; captions come from the director's events.

    python3 scripts/record_demo.py --build --output /tmp/omoba-demo
    python3 scripts/record_demo.py --client-bin ... --server-bin ... --output /tmp/d --clip desktop

Output: `omoba-demo.mp4` (1280x720, H.264), `demo-manifest.json` and one raw
folder per clip (frames, logs, events). Needs ffmpeg with libx264 and a GPU
window for a few minutes. The phone clip is the phone UI rendered by a desktop
development build (OMOBA_TOUCH_CONTROLS=1), not a device recording.
"""
import argparse
import datetime
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
from capture_showcase import ROOT, free_address, run_until_exit, sha256, source_identity, stop  # noqa: E402

WIDTH, HEIGHT, FPS = 1280, 720, 30
FONT = ROOT / "client/assets/ui/Inter.ttf"

# Clip order is the video order. `script` selects the director timeline.
CLIPS = {
    "desktop": dict(script="desktop", hero="mage", size=(1280, 720), touch=False,
                    label="Desktop"),
    "jungle": dict(script="jungle", hero="warden", size=(1280, 720), touch=False,
                   label="Desktop · Warden"),
    "phone": dict(script="phone", hero="ranger", size=(844, 390), touch=True,
                  label="Phone interface preview (rendered on desktop)"),
}
INTRO = ("O-MOBA", "An open-source MOBA built with Rust and Bevy")
OUTRO = ("omoba.io", "Play, create and contribute · github.com/o-moba")
CAPTION_SECONDS = 3.2


def find_ffmpeg():
    """An ffmpeg with drawtext (libfreetype); Homebrew's plain bottle lacks it."""
    candidates = [os.environ.get("FFMPEG"), "/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg",
                  "/usr/local/opt/ffmpeg-full/bin/ffmpeg", shutil.which("ffmpeg")]
    for candidate in filter(None, candidates):
        if not Path(candidate).is_file():
            continue
        filters = subprocess.run([candidate, "-hide_banner", "-filters"], capture_output=True, text=True).stdout
        if " drawtext " in filters:
            return candidate
    return None


FFMPEG = "ffmpeg"


def record_clip(name, clip, binaries, assets, raw, timeout):
    raw.mkdir(parents=True, exist_ok=True)
    frames = raw / "frames"
    if frames.exists():
        shutil.rmtree(frames)
    record = dict(clip=name, **{k: v for k, v in clip.items() if k != "size"}, size=list(clip["size"]))
    processes = []
    with tempfile.TemporaryDirectory(prefix=f"omoba-demo-{name}-") as workdir:
        host, port = free_address()
        env = {key: value for key, value in os.environ.items()
               if not (key.startswith("OMOBA_") and ("_QA" in key or key == "OMOBA_AUTOJOIN"))}
        env.update(SERVER_ADDR=f"{host}:{port}", GAME_SERVER_ADDR=f"{host}:{port}",
                   OMOBA_MATCH_MODE="practice", OMOBA_TEAM_SIZE="5",
                   OMOBA_CLIENT_CONFIG_DIR=str(Path(workdir) / "config"), OMOBA_ASSET_DIR=str(assets),
                   OMOBA_PLAYER_VISUAL_MODE="models3d", OMOBA_DEBUG_UI="0",
                   OMOBA_QA_WIDTH=str(clip["size"][0]), OMOBA_QA_HEIGHT=str(clip["size"][1]),
                   OMOBA_TOUCH_CONTROLS="1" if clip["touch"] else "0",
                   OMOBA_DEMO_QA_DIR=str(raw), OMOBA_DEMO_SCRIPT=clip["script"], OMOBA_DEMO_CLASS=clip["hero"],
                   OMOBA_RECORD_DIR=str(frames), OMOBA_RECORD_FPS=str(FPS))
        try:
            log = (raw / "server.log").open("w")
            processes.append(subprocess.Popen([str(binaries["server"])], cwd=workdir, env=env,
                                              stdout=log, stderr=subprocess.STDOUT))
            deadline = time.monotonic() + 20
            while "is listening" not in (raw / "server.log").read_text(errors="replace"):
                if processes[0].poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError("server did not report listening")
                time.sleep(0.05)
            log = (raw / "client.log").open("w")
            client = subprocess.Popen([str(binaries["client"])], cwd=workdir, env=env,
                                      stdout=log, stderr=subprocess.STDOUT)
            processes.append(client)
            record["client_exit_code"] = run_until_exit(processes, client, timeout)
        except Exception as error:
            record["error"] = str(error)
        finally:
            stop(processes)
    events_path = raw / "demo-events.json"
    summary = json.loads(events_path.read_text()) if events_path.is_file() else {}
    record["events"] = summary.get("events", [])
    record["pace"] = summary.get("pace", [])
    frame_log = frames / "frames.jsonl"
    rows = [json.loads(line) for line in frame_log.read_text().splitlines()] if frame_log.is_file() else []
    record["frames"] = [row for row in rows if (frames / row["file"]).is_file()]
    record["pass"] = (record.get("client_exit_code") == 0 and "error" not in record
                      and len(record["frames"]) > FPS * 5 and bool(record["events"]))
    return record


def escape(text):
    return text.replace("\\", "\\\\").replace(":", "\\:").replace("'", "’").replace(",", "\\,")


def speed_at(pace, seconds):
    speed = 1.0
    for marker in pace:
        if marker["seconds"] <= seconds:
            speed = marker["speed"]
    return speed


def warp(pace, start, seconds):
    """Video time of a recorder time, with timelapse segments compressed."""
    points = sorted({start, seconds, *(m["seconds"] for m in pace if start < m["seconds"] < seconds)})
    return sum((b - a) / speed_at(pace, a) for a, b in zip(points, points[1:]))


def caption_filters(events, start, end, pace):
    filters = []
    font = str(FONT).replace(":", "\\:")
    total = warp(pace, start, end)
    for event in events:
        shown = warp(pace, start, max(event["seconds"], start))
        if shown >= total:
            continue
        until = shown + CAPTION_SECONDS
        enable = f"enable='between(t,{shown:.2f},{until:.2f})'"
        # Top centre sits between the minimap and the score strip on both HUDs.
        filters.append(f"drawbox=x=(iw-660)/2:y=12:w=660:h=96:color=black@0.6:t=fill:{enable}")
        filters.append(f"drawtext=fontfile='{font}':text='{escape(event['title'])}':fontsize=38:"
                       f"fontcolor=white:x=(w-tw)/2:y=22:{enable}")
        filters.append(f"drawtext=fontfile='{font}':text='{escape(event['subtitle'])}':fontsize=22:"
                       f"fontcolor=0xc9f3ff:x=(w-tw)/2:y=72:{enable}")
    for marker, following in zip(pace, pace[1:] + [None]):
        if marker["speed"] == 1.0:
            continue
        a = warp(pace, start, max(marker["seconds"], start))
        b = warp(pace, start, min(following["seconds"] if following else end, end))
        filters.append(f"drawtext=fontfile='{font}':text='▶▶ {marker['speed']:g}×':fontsize=26:"
                       f"fontcolor=white:x=w-tw-24:y=h-60:box=1:boxcolor=black@0.5:boxborderw=10:"
                       f"enable='between(t,{a:.2f},{b:.2f})'")
    return filters


def encode_clip(record, raw, output):
    """Real-time clip from timestamped frames, fitted into 1280x720, with captions."""
    frames = record["frames"]
    events = record["events"]
    pace = record.get("pace", [])
    start = max(events[0]["seconds"] - 0.4, frames[0]["seconds"]) if events else frames[0]["seconds"]
    kept = [row for row in frames if row["seconds"] >= start]
    end = kept[-1]["seconds"] + 1.0 / FPS
    listing = raw / "frames.ffconcat"
    lines = ["ffconcat version 1.0"]
    for current, following in zip(kept, kept[1:] + [None]):
        duration = ((following["seconds"] if following else end) - current["seconds"]) / speed_at(
            pace, current["seconds"])
        lines += [f"file 'frames/{current['file']}'", f"duration {max(duration, 1.0 / FPS / 4):.4f}"]
    lines.append(f"file 'frames/{kept[-1]['file']}'")
    listing.write_text("\n".join(lines) + "\n")
    font = str(FONT).replace(":", "\\:")
    label = escape(record["label"])
    chain = [f"scale={WIDTH}:{HEIGHT}:force_original_aspect_ratio=decrease",
             f"pad={WIDTH}:{HEIGHT}:(ow-iw)/2:(oh-ih)/2:color=0x05060c",
             f"fps={FPS}", "format=yuv420p",
             f"drawtext=fontfile='{font}':text='{label}':fontsize=18:fontcolor=white@0.75:"
             f"x=24:y=h-th-20:box=1:boxcolor=black@0.45:boxborderw=8"]
    chain += caption_filters(events, start, end, pace)
    subprocess.run([FFMPEG, "-y", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i", str(listing),
                    "-vf", ",".join(chain), "-r", str(FPS), "-c:v", "libx264", "-preset", "medium",
                    "-crf", "20", "-pix_fmt", "yuv420p", str(output)], check=True, cwd=raw)
    return warp(pace, start, end)


def title_card(title, subtitle, seconds, output):
    font = str(FONT).replace(":", "\\:")
    fade = f"fade=t=in:st=0:d=0.4,fade=t=out:st={seconds - 0.4}:d=0.4"
    vf = (f"drawtext=fontfile='{font}':text='{escape(title)}':fontsize=84:fontcolor=white:"
          f"x=(w-tw)/2:y=h/2-90,"
          f"drawtext=fontfile='{font}':text='{escape(subtitle)}':fontsize=30:fontcolor=0x22d3ee:"
          f"x=(w-tw)/2:y=h/2+20,{fade},format=yuv420p")
    subprocess.run([FFMPEG, "-y", "-loglevel", "error", "-f", "lavfi", "-i",
                    f"color=c=0x05060c:s={WIDTH}x{HEIGHT}:r={FPS}:d={seconds}", "-vf", vf,
                    "-c:v", "libx264", "-crf", "20", "-pix_fmt", "yuv420p", str(output)], check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--clip", action="append", choices=list(CLIPS), help="repeatable; default: all")
    parser.add_argument("--build", action="store_true", help="cargo build the dev workspace first")
    parser.add_argument("--client-bin", type=Path)
    parser.add_argument("--server-bin", type=Path)
    parser.add_argument("--assets", type=Path, default=ROOT / "client/assets")
    parser.add_argument("--timeout", type=int, default=300, help="seconds per clip")
    parser.add_argument("--attempts", type=int, default=2)
    parser.add_argument("--encode-only", action="store_true", help="re-encode the existing raw clips")
    args = parser.parse_args()
    global FFMPEG
    FFMPEG = find_ffmpeg()
    if not FFMPEG:
        parser.error("ffmpeg with drawtext is required (brew install ffmpeg-full, or set FFMPEG)")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    names = args.clip or list(CLIPS)
    manifest_path = output / "demo-manifest.json"
    if args.encode_only:
        manifest = json.loads(manifest_path.read_text())
    else:
        if args.build:
            from package_native import build_executables
            binaries = build_executables("dev")
        elif args.client_bin and args.server_bin:
            binaries = dict(client=args.client_bin.resolve(), server=args.server_bin.resolve())
        else:
            parser.error("pass --build or both --client-bin and --server-bin (a dev build)")
        manifest = dict(schema_version=1, generator="scripts/record_demo.py",
                        recorded_on=datetime.date.today().isoformat(), source=source_identity(),
                        binaries={name: dict(path=str(path), sha256=sha256(path))
                                  for name, path in binaries.items() if name in ("client", "server")},
                        method="scripted production inputs against a live practice server; "
                               "timestamped window readbacks; captions added by ffmpeg",
                        clips=[])
        for name in names:
            for attempt in range(1, args.attempts + 1):
                print(f"[demo] {name}: recording (attempt {attempt}) ...", flush=True)
                record = record_clip(name, CLIPS[name], binaries, args.assets.resolve(),
                                     output / "raw" / name, args.timeout)
                record["attempt"] = attempt
                if record["pass"]:
                    break
            print(f"[demo] {name}: {'ok' if record['pass'] else 'FAILED'} "
                  f"({len(record['frames'])} frames){' - ' + record['error'] if record.get('error') else ''}",
                  flush=True)
            manifest["clips"].append(record)
        manifest_path.write_text(json.dumps(manifest, indent=1) + "\n")
    parts = output / "parts"
    parts.mkdir(exist_ok=True)
    pieces = [parts / "00-intro.mp4"]
    title_card(*INTRO, 2.6, pieces[0])
    for index, record in enumerate(manifest["clips"], start=1):
        if not record.get("pass"):
            continue
        piece = parts / f"{index:02d}-{record['clip']}.mp4"
        record["encoded_seconds"] = encode_clip(record, output / "raw" / record["clip"], piece)
        pieces.append(piece)
    pieces.append(parts / "99-outro.mp4")
    title_card(*OUTRO, 3.0, pieces[-1])
    listing = parts / "video.ffconcat"
    listing.write_text("ffconcat version 1.0\n" + "".join(f"file '{piece.name}'\n" for piece in pieces))
    video = output / "omoba-demo.mp4"
    subprocess.run([FFMPEG, "-y", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i", str(listing),
                    "-c:v", "libx264", "-crf", "20", "-preset", "medium", "-pix_fmt", "yuv420p",
                    "-movflags", "+faststart", str(video)], check=True)
    manifest["video"] = dict(file=video.name, bytes=video.stat().st_size, sha256=sha256(video))
    slim = dict(manifest, clips=[{k: v for k, v in clip.items() if k != "frames"} | dict(frame_count=len(clip["frames"]))
                                 for clip in manifest["clips"]])
    (output / "demo-summary.json").write_text(json.dumps(slim, indent=2) + "\n")
    failed = [clip["clip"] for clip in manifest["clips"] if not clip.get("pass")]
    print(f"[demo] {video}" + (f"; failed clips: {', '.join(failed)}" if failed else ""))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
