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

`--profile v2` records the sources of the second trailer instead: clean 60 fps
clips at the capture resolution in `clips/`, no captions and no joined video.
They are frame-stepped captures of the offline practice (no server process):
game time advances exactly 1/60 s per rendered frame, so every frame is one
video frame at any resolution (see client/src/qa/record_qa.rs). The capture
runs slower than real time; raw PNG frames are deleted after encoding unless
`--keep-frames` is given.

    python3 scripts/record_demo.py --profile v2 --build --output /Volumes/work/omoba-v2
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
# Trailer v2 clips. `size` is pixels and the layout is `size / scale` points:
# 2532x1170 at scale 3 is the 844x390 phone layout, 2560x1440 at scale 2 the
# 1280x720 desktop one. `zoom` is the camera zoom (smaller is closer) and
# `lane` the lane of the `lane` and `showcase` director scripts.
V2_FPS = 60
PHONE = dict(size=(2532, 1170), scale=3.0, touch=True)
DESKTOP = dict(size=(2560, 1440), scale=2.0, touch=False)
V2_CLIPS = {
    "phone-wildspark": dict(PHONE, script="showcase", hero="wildspark", lane="mid", zoom=0.62,
                            label="Phone interface · Wildspark"),
    "phone-stormfist": dict(PHONE, script="lane", hero="stormfist", lane="top", zoom=0.62,
                            label="Phone interface · Stormfist"),
    "phone-emberveil": dict(PHONE, script="lane", hero="emberveil", lane="bot", zoom=0.62,
                            label="Phone interface · Emberveil"),
    "phone-warden": dict(PHONE, script="jungle", hero="warden", zoom=0.62,
                         label="Phone interface · Warden"),
    "desktop-dawnweaver": dict(DESKTOP, script="showcase", hero="dawnweaver", lane="mid", zoom=0.62,
                               label="Desktop · Dawnweaver"),
    "desktop-frostguard": dict(DESKTOP, script="lane", hero="frostguard", lane="top", zoom=0.62,
                               label="Desktop · Frostguard"),
}
for _clip in V2_CLIPS.values():
    _clip.update(offline=True, fps=V2_FPS)
PROFILES = {"v1": CLIPS, "v2": V2_CLIPS}
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


def clip_environment(clip, raw, address, workdir, assets, pace=15, window_at=None):
    """The demo client's (and v1 server's) environment for one clip. Offline
    clips play the in-process practice in frame-stepped capture; `window_at`
    ("x,y" pixels) parks their always-on-top window."""
    env = {key: value for key, value in os.environ.items()
           if not (key.startswith("OMOBA_") and ("_QA" in key or key == "OMOBA_AUTOJOIN"))}
    for key in ("OMOBA_OFFLINE_PRACTICE", "OMOBA_RECORD_STEP", "OMOBA_RECORD_PACE",
                "OMOBA_RECORD_WINDOW_AT", "OMOBA_DEMO_ZOOM", "OMOBA_DEMO_LANE"):
        env.pop(key, None)
    env.update(SERVER_ADDR=address, GAME_SERVER_ADDR=address,
               OMOBA_MATCH_MODE="practice", OMOBA_TEAM_SIZE="5",
               OMOBA_CLIENT_CONFIG_DIR=str(Path(workdir) / "config"), OMOBA_ASSET_DIR=str(assets),
               OMOBA_PLAYER_VISUAL_MODE="models3d", OMOBA_DEBUG_UI="0",
               OMOBA_QA_WIDTH=str(clip["size"][0]), OMOBA_QA_HEIGHT=str(clip["size"][1]),
               OMOBA_QA_SCALE=f"{clip.get('scale', 1.0):g}",
               OMOBA_TOUCH_CONTROLS="1" if clip["touch"] else "0",
               OMOBA_DEMO_QA_DIR=str(raw), OMOBA_DEMO_SCRIPT=clip["script"], OMOBA_DEMO_CLASS=clip["hero"],
               OMOBA_RECORD_DIR=str(raw / "frames"), OMOBA_RECORD_FPS=str(clip.get("fps", FPS)))
    if clip.get("zoom"):
        env["OMOBA_DEMO_ZOOM"] = f"{clip['zoom']:g}"
    if clip.get("lane"):
        env["OMOBA_DEMO_LANE"] = clip["lane"]
    if clip.get("offline"):
        # A slow capture must survive the desktop focus moving to other work.
        env.update(OMOBA_OFFLINE_PRACTICE="1", OMOBA_RECORD_STEP="1", OMOBA_RECORD_PACE=str(pace),
                   OMOBA_QA_SYNTHETIC_FOCUS="1")
        if window_at:
            env["OMOBA_RECORD_WINDOW_AT"] = window_at
    return env


def record_clip(name, clip, binaries, assets, raw, timeout, pace=15, window_at=None):
    raw.mkdir(parents=True, exist_ok=True)
    frames = raw / "frames"
    if frames.exists():
        shutil.rmtree(frames)
    fps = clip.get("fps", FPS)
    record = dict(clip=name, **{k: v for k, v in clip.items() if k != "size"}, size=list(clip["size"]))
    processes = []
    with tempfile.TemporaryDirectory(prefix=f"omoba-demo-{name}-") as workdir:
        host, port = free_address()
        env = clip_environment(clip, raw, f"{host}:{port}", workdir, assets, pace, window_at)
        try:
            if not clip.get("offline"):
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
                      and len(record["frames"]) > fps * 5 and bool(record["events"]))
    if clip.get("offline"):
        # Frame-stepped: frame n is video frame n, so none may be missing.
        record["pass"] = record["pass"] and stepped_frames_complete(rows, record["frames"], fps)
    return record


def stepped_frames_complete(rows, saved, fps):
    """True when a frame-stepped log is gapless: every logged frame was saved,
    indices count up from zero and each timestamp is `index / fps`."""
    return (len(saved) == len(rows) and all(
        row["index"] == index and row.get("skipped_before", 0) == 0
        and abs(row["seconds"] - index / fps) < 1e-6 for index, row in enumerate(rows)))


def encode_stepped(record, raw, output):
    """Clean constant-rate clip at the capture resolution; frame n of the
    capture is frame n of the video, so director events keep their seconds."""
    fps = record["fps"]
    subprocess.run([FFMPEG, "-y", "-loglevel", "error", "-framerate", str(fps), "-i", "frames/frame-%06d.png",
                    "-vf", "scale=out_color_matrix=bt709:out_range=tv,format=yuv420p",
                    "-c:v", "libx264", "-preset", "medium", "-crf", "14", "-g", str(fps // 2),
                    "-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709",
                    "-color_range", "tv", "-movflags", "+faststart", str(output)], check=True, cwd=raw)
    return len(record["frames"]) / fps


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


def clip_span(record):
    """Recorder-clock start/end of the kept part of a clip (from its first caption)."""
    frames, events = record["frames"], record["events"]
    start = max(events[0]["seconds"] - 0.4, frames[0]["seconds"]) if events else frames[0]["seconds"]
    return start, frames[-1]["seconds"] + 1.0 / FPS


def encode_clip(record, raw, output, captions=True):
    """Real-time clip from timestamped frames, fitted into 1280x720. With
    `captions`, the profile label and event captions are burned in; without,
    only the timelapse badge is (the trailer adds its own titles)."""
    frames = record["frames"]
    events = record["events"]
    pace = record.get("pace", [])
    start, end = clip_span(record)
    kept = [row for row in frames if row["seconds"] >= start]
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
             f"fps={FPS}", "format=yuv420p"]
    if captions:
        chain.append(f"drawtext=fontfile='{font}':text='{label}':fontsize=18:fontcolor=white@0.75:"
                     f"x=24:y=h-th-20:box=1:boxcolor=black@0.45:boxborderw=8")
        chain += caption_filters(events, start, end, pace)
    else:
        chain += [f for f in caption_filters([], start, end, pace)]
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
    parser.add_argument("--profile", choices=list(PROFILES), default="v1",
                        help="v1: the captioned real-time demo; v2: frame-stepped 60 fps trailer clips")
    parser.add_argument("--clip", action="append", help="repeatable; default: all clips of the profile")
    parser.add_argument("--build", action="store_true", help="cargo build the dev workspace first")
    parser.add_argument("--client-bin", type=Path)
    parser.add_argument("--server-bin", type=Path)
    parser.add_argument("--assets", type=Path, default=ROOT / "client/assets")
    parser.add_argument("--timeout", type=int, help="seconds per clip (default 300; 3600 for v2)")
    parser.add_argument("--pace", type=int, default=15,
                        help="v2: frames captured per wall-clock second (lower it if frames overrun)")
    parser.add_argument("--keep-frames", action="store_true", help="v2: keep the raw PNG frames")
    parser.add_argument("--window-at", metavar="X,Y",
                        help="v2: park the always-on-top capture window at these pixels, e.g. with "
                             "only a corner on screen; a fully covered window captures ~1 frame/s on macOS")
    parser.add_argument("--attempts", type=int, default=2)
    parser.add_argument("--encode-only", action="store_true", help="re-encode the existing raw clips")
    args = parser.parse_args()
    global FFMPEG
    FFMPEG = find_ffmpeg()
    if not FFMPEG:
        parser.error("ffmpeg with drawtext is required (brew install ffmpeg-full, or set FFMPEG)")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    clips = PROFILES[args.profile]
    stepped = args.profile == "v2"
    unknown = sorted(set(args.clip or []) - set(clips))
    if unknown:
        parser.error(f"unknown {args.profile} clip(s): {', '.join(unknown)}; choose from {', '.join(clips)}")
    names = args.clip or list(clips)
    timeout = args.timeout or (3600 if stepped else 300)
    manifest_path = output / "demo-manifest.json"
    if args.encode_only:
        manifest = json.loads(manifest_path.read_text())
        # The recording decides the kind of encode, whatever --profile says now.
        stepped = manifest.get("profile", "v1") == "v2"
    else:
        if args.build:
            from package_native import build_executables
            binaries = build_executables("dev")
        elif args.client_bin and (args.server_bin or stepped):
            binaries = dict(client=args.client_bin.resolve())
            if args.server_bin:
                binaries["server"] = args.server_bin.resolve()
        else:
            parser.error("pass --build, or --client-bin with --server-bin (a dev build; v2 needs only the client)")
        method = ("scripted production inputs against the in-process offline practice (server bots); "
                  "frame-stepped window readbacks, one per 1/60 s of game time; no captions"
                  if stepped else
                  "scripted production inputs against a live practice server; "
                  "timestamped window readbacks; captions added by ffmpeg")
        manifest = dict(schema_version=1, generator="scripts/record_demo.py", profile=args.profile,
                        recorded_on=datetime.date.today().isoformat(), source=source_identity(),
                        binaries={name: dict(path=str(path), sha256=sha256(path))
                                  for name, path in binaries.items() if name in ("client", "server")},
                        method=method, clips=[])
        if manifest_path.is_file() and args.clip:
            # Re-recording some clips keeps the others of the same profile.
            previous = json.loads(manifest_path.read_text())
            if previous.get("profile", "v1") == args.profile:
                manifest["clips"] = [clip for clip in previous["clips"] if clip["clip"] not in names]
        for name in names:
            for attempt in range(1, args.attempts + 1):
                print(f"[demo] {name}: recording (attempt {attempt}) ...", flush=True)
                record = record_clip(name, clips[name], binaries, args.assets.resolve(),
                                     output / "raw" / name, timeout, args.pace, args.window_at)
                record["attempt"] = attempt
                if record["pass"]:
                    break
            print(f"[demo] {name}: {'ok' if record['pass'] else 'FAILED'} "
                  f"({len(record['frames'])} frames){' - ' + record['error'] if record.get('error') else ''}",
                  flush=True)
            manifest["clips"].append(record)
        manifest["clips"].sort(key=lambda clip: list(clips).index(clip["clip"]))
        manifest_path.write_text(json.dumps(manifest, indent=1) + "\n")
    if stepped:
        return finish_stepped(manifest, output, names, args.keep_frames)
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


def finish_stepped(manifest, output, names, keep_frames):
    """Encodes the clean v2 clips of `names`, drops their raw frames and writes
    the summary; there is no joined video."""
    folder = output / "clips"
    folder.mkdir(exist_ok=True)
    for record in manifest["clips"]:
        raw = output / "raw" / record["clip"]
        if not record.get("pass") or record["clip"] not in names:
            continue
        video = folder / f"{record['clip']}.mp4"
        if not any((raw / "frames").glob("frame-*.png")):
            # A re-run after the raw frames were deleted keeps the encoded clip.
            print(f"[demo] {record['clip']}: no raw frames to encode"
                  + ("; keeping the existing clip" if video.is_file() else ""), flush=True)
            continue
        record["encoded_seconds"] = encode_stepped(record, raw, video)
        record["video"] = dict(file=f"clips/{video.name}", bytes=video.stat().st_size, sha256=sha256(video))
        if not keep_frames:
            for frame in (raw / "frames").glob("frame-*.png"):
                frame.unlink()
    (output / "demo-manifest.json").write_text(json.dumps(manifest, indent=1) + "\n")
    slim = dict(manifest, clips=[{k: v for k, v in clip.items() if k != "frames"} | dict(frame_count=len(clip["frames"]))
                                 for clip in manifest["clips"]])
    (output / "demo-summary.json").write_text(json.dumps(slim, indent=2) + "\n")
    failed = [clip["clip"] for clip in manifest["clips"] if not clip.get("pass")]
    print(f"[demo] {folder}" + (f"; failed clips: {', '.join(failed)}" if failed else ""))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
