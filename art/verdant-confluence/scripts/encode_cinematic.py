#!/usr/bin/env python3
"""Encode complete Blender shot frames, crossfade, and verify a silent H.264 film.

Requires installed ffmpeg/ffprobe; Python uses only its standard library.
The MP4 is published only after its probe and complete decode succeed.
"""
import argparse
from datetime import datetime, timezone
from fractions import Fraction
import hashlib
import json
import math
from pathlib import Path
import re
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import time


PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"


def utc_now():
    return datetime.now(timezone.utc).isoformat()


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def whole_frames(seconds, fps, label):
    require(isinstance(seconds, (int, float)) and not isinstance(seconds, bool)
            and math.isfinite(seconds) and 0 < seconds <= 120,
            f"{label} must be a finite positive duration up to 120 seconds")
    count = seconds * fps
    require(abs(count - round(count)) < 0.000001, f"{label} must contain a whole number of frames")
    return round(count)


def read_plan(path):
    plan = json.loads(path.read_text())
    require(isinstance(plan, dict), "plan must be a JSON object")
    for name, maximum in (("fps", 120), ("width", 8192), ("height", 8192)):
        value = plan.get(name)
        require(type(value) is int and 1 <= value <= maximum,
                f"{name} must be a positive integer up to {maximum}")
    require(plan["width"] % 2 == 0 and plan["height"] % 2 == 0,
            "yuv420p requires even width and height")
    transition_frames = whole_frames(plan.get("transition_seconds"), plan["fps"], "transition_seconds")
    shots = plan.get("shots")
    require(isinstance(shots, list) and 2 <= len(shots) <= 16, "plan needs between 2 and 16 shots")
    identifiers = set()
    frame_counts = []
    for shot in shots:
        require(isinstance(shot, dict), "each shot must be a JSON object")
        identifier = shot.get("id")
        require(isinstance(identifier, str) and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]{0,79}", identifier),
                "shot ids must be safe directory names using letters, digits, underscores or dashes")
        require(identifier not in identifiers, f"duplicate shot id: {identifier}")
        identifiers.add(identifier)
        count = whole_frames(shot.get("duration_seconds"), plan["fps"], f"{identifier} duration_seconds")
        require(count > 2 * transition_frames, f"{identifier} must be longer than two transitions")
        frame_counts.append(count)
    total_frames = sum(frame_counts) - transition_frames * (len(shots) - 1)
    return plan, frame_counts, total_frames


def frame_inventory(frames, plan, frame_counts):
    inventory = []
    for shot, count in zip(plan["shots"], frame_counts):
        directory = frames / shot["id"]
        require(directory.is_dir(), f"missing shot directory: {directory}")
        expected = {f"frame_{index:04d}.png" for index in range(1, count + 1)}
        actual = {path.name for path in directory.iterdir() if path.is_file() and path.suffix.lower() == ".png"}
        missing, extra = sorted(expected - actual), sorted(actual - expected)
        require(not missing and not extra,
                f"{shot['id']} frame inventory mismatch: missing={missing[:8]}, unexpected={extra[:8]}")
        records = []
        for name in sorted(expected):
            path = directory / name
            with path.open("rb") as stream:
                header = stream.read(24)
            require(len(header) == 24 and header[:8] == PNG_SIGNATURE
                    and header[8:16] == b"\x00\x00\x00\rIHDR", f"invalid PNG header: {path}")
            width, height = struct.unpack(">II", header[16:24])
            require((width, height) == (plan["width"], plan["height"]),
                    f"wrong frame dimensions {width}x{height}: {path}")
            records.append({"name": name, "bytes": path.stat().st_size, "sha256": sha256(path)})
        inventory.append({"id": shot["id"], "frame_count": count, "frames": records})
    return inventory


def filter_graph(plan, with_title=False):
    fps = plan["fps"]
    transition = plan["transition_seconds"]
    filters = [f"[{index}:v]fps={fps},settb=AVTB,setpts=PTS-STARTPTS,format=yuv420p[s{index}]"
               for index in range(len(plan["shots"]))]
    previous = "s0"
    duration = plan["shots"][0]["duration_seconds"]
    for index, shot in enumerate(plan["shots"][1:], 1):
        current = f"mix{index}"
        offset = duration - transition
        filters.append(f"[{previous}][s{index}]xfade=transition=fade:duration={transition:g}:offset={offset:g}[{current}]")
        duration += shot["duration_seconds"] - transition
        previous = current
    tail = f"[{previous}]fps={fps},format=yuv420p"
    if with_title:
        # The text itself is in a local file with expansion disabled; it never
        # becomes shell or filter syntax. Keep titles optional and restrained.
        tail += (",drawtext=textfile=title.txt:expansion=none:fontcolor=white@0.94:fontsize=44"
                 ":box=1:boxcolor=black@0.20:boxborderw=14:x=(w-text_w)/2:y=h*0.085"
                 ":enable='between(t,0.8,3.8)'"
                 ":alpha='if(lt(t,1.4),(t-0.8)/0.6,if(gt(t,3.2),(3.8-t)/0.6,1))'")
    filters.append(tail + "[film]")
    return ";".join(filters)


def verify_probe(probe, plan, expected_frames):
    streams = probe.get("streams", [])
    require(len(streams) == 1 and streams[0].get("codec_type") == "video", "film must contain one video stream and no audio")
    stream = streams[0]
    for name, expected in (("codec_name", "h264"), ("pix_fmt", "yuv420p"),
                           ("width", plan["width"]), ("height", plan["height"])):
        require(stream.get(name) == expected, f"unexpected {name}: {stream.get(name)!r}; expected {expected!r}")
    for name in ("r_frame_rate", "avg_frame_rate"):
        require(Fraction(stream.get(name, "0/1")) == plan["fps"], f"unexpected {name}: {stream.get(name)!r}")
    require(int(stream.get("nb_read_frames", -1)) == expected_frames,
            f"decoded frame count is {stream.get('nb_read_frames')}; expected {expected_frames}")
    expected_duration = expected_frames / plan["fps"]
    measured_duration = float(probe.get("format", {}).get("duration", "nan"))
    require(math.isfinite(measured_duration) and abs(measured_duration - expected_duration) < 1 / plan["fps"],
            f"measured duration is {measured_duration}; expected {expected_duration}")
    return {"status": "PASS", "frames": expected_frames, "duration_seconds": measured_duration,
            "fps": plan["fps"], "width": plan["width"], "height": plan["height"],
            "codec": "h264", "pixel_format": "yuv420p", "audio_streams": 0}


class CommandRecorder:
    def __init__(self, report, report_path, log_directory):
        self.report = report
        self.report_path = report_path
        self.log_directory = log_directory

    def save(self):
        temporary = self.report_path.with_name(self.report_path.name + ".tmp")
        temporary.write_text(json.dumps(self.report, indent=2) + "\n")
        temporary.replace(self.report_path)

    def run(self, label, command, cwd):
        log_path = self.log_directory / f"{len(self.report['commands']) + 1:02d}-{label}.log"
        record = {"label": label, "argv": command, "cwd": str(cwd), "log": str(log_path),
                  "started_at": utc_now(), "status": "RUNNING"}
        self.report["commands"].append(record)
        self.save()
        started = time.monotonic()
        process = None
        try:
            with log_path.open("w") as log:
                process = subprocess.Popen(command, cwd=cwd, stdin=subprocess.DEVNULL,
                                           stdout=log, stderr=subprocess.STDOUT, text=True)
                record["pid"] = process.pid
                record["returncode"] = process.wait()
            require(record["returncode"] == 0, f"{label} failed ({record['returncode']}); see {log_path}")
            record["status"] = "PASS"
            return log_path.read_text(errors="replace")
        except BaseException as error:
            record["status"] = "FAIL"
            record["error"] = str(error) or type(error).__name__
            if process is not None and process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
            if process is not None:
                record["returncode"] = process.returncode
            raise
        finally:
            record["completed_at"] = utc_now()
            record["elapsed_seconds"] = round(time.monotonic() - started, 3)
            self.save()


def encode(args):
    frames = args.frames.resolve()
    output = args.output.resolve()
    plan_path = args.plan.resolve()
    require(output.suffix.lower() == ".mp4", "output must have an .mp4 extension")
    require(not output.exists(), f"output already exists; choose a new path to preserve it: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    report_path = output.with_suffix(".encoding.json")
    log_directory = output.parent / (output.stem + ".encoding-logs")
    log_directory.mkdir(exist_ok=True)
    report = {"format_version": 1, "status": "RUNNING", "started_at": utc_now(),
              "frames_root": str(frames), "plan_path": str(plan_path), "output": str(output),
              "commands": [], "title": args.title}
    recorder = CommandRecorder(report, report_path, log_directory)
    recorder.save()
    try:
        ffmpeg, ffprobe = shutil.which("ffmpeg"), shutil.which("ffprobe")
        require(ffmpeg is not None and ffprobe is not None, "installed ffmpeg and ffprobe are required")
        plan, counts, total_frames = read_plan(plan_path)
        report["plan"] = plan
        report["plan_sha256"] = sha256(plan_path)
        report["expected_frames"] = total_frames
        report["expected_duration_seconds"] = total_frames / plan["fps"]
        report["frame_inventory"] = frame_inventory(frames, plan, counts)
        report["input_validation"] = {"status": "PASS", "frames": sum(counts)}
        recorder.save()
        print(f"Validated {sum(counts)} PNGs; encoding {total_frames} final frames ({total_frames / plan['fps']:g}s).", flush=True)
        with tempfile.TemporaryDirectory(prefix=".verdant-encode-", dir=output.parent) as directory:
            working = Path(directory)
            recorder.run("ffmpeg-version", [ffmpeg, "-version"], working)
            recorder.run("ffprobe-version", [ffprobe, "-version"], working)
            if args.title is not None:
                filters = recorder.run("available-filters", [ffmpeg, "-hide_banner", "-filters"], working)
                require(re.search(r"\bdrawtext\s", filters) is not None, "--title requires ffmpeg's drawtext filter; omit --title for a clean film")
                (working / "title.txt").write_text(args.title)
            clips = []
            encoding = ["-c:v", "libx264", "-preset", "slow", "-crf", "18", "-pix_fmt", "yuv420p",
                        "-r", str(plan["fps"]), "-fps_mode", "cfr", "-threads", "4", "-an", "-movflags", "+faststart"]
            for shot, count in zip(plan["shots"], counts):
                clip = working / (shot["id"] + ".mp4")
                command = [ffmpeg, "-hide_banner", "-nostdin", "-y", "-xerror", "-framerate", str(plan["fps"]),
                           "-start_number", "1", "-i", str(frames / shot["id"] / "frame_%04d.png"),
                           "-frames:v", str(count), *encoding, str(clip)]
                print(f"Encoding shot {shot['id']}...", flush=True)
                recorder.run("encode-" + shot["id"], command, working)
                clips.append(clip)
            graph = filter_graph(plan, with_title=args.title is not None)
            report["filter_graph"] = graph
            staging = working / "verified-film.mp4"
            command = [ffmpeg, "-hide_banner", "-nostdin", "-y", "-xerror", "-filter_complex_threads", "1"]
            for clip in clips:
                command.extend(["-threads", "1", "-i", str(clip)])
            command.extend(["-filter_complex", graph, "-map", "[film]", "-frames:v", str(total_frames), *encoding, str(staging)])
            print("Joining shots with crossfades...", flush=True)
            recorder.run("crossfade-film", command, working)
            probe_text = recorder.run("ffprobe-full", [ffprobe, "-v", "error", "-count_frames", "-show_streams",
                                                       "-show_format", "-of", "json", str(staging)], working)
            report["ffprobe"] = json.loads(probe_text)
            report["output_validation"] = verify_probe(report["ffprobe"], plan, total_frames)
            recorder.run("full-decode", [ffmpeg, "-hide_banner", "-nostdin", "-v", "error", "-xerror", "-i",
                                         str(staging), "-map", "0:v:0", "-f", "null", "-"], working)
            report["full_decode"] = {"status": "PASS"}
            require(sha256(plan_path) == report["plan_sha256"], "plan changed during encoding")
            require(frame_inventory(frames, plan, counts) == report["frame_inventory"], "render frames changed during encoding")
            report["output_sha256"] = sha256(staging)
            report["output_bytes"] = staging.stat().st_size
            staging.replace(output)
        report["status"] = "PASS"
        report["completed_at"] = utc_now()
        recorder.save()
        print(f"Verified film: {output}\nEncoding manifest: {report_path}", flush=True)
        return 0
    except BaseException as error:
        report["status"] = "FAIL"
        report["completed_at"] = utc_now()
        report["error"] = str(error) or type(error).__name__
        recorder.save()
        raise


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--frames", type=Path, required=True, help="root containing one frame_0001.png sequence per shot id")
    parser.add_argument("--output", type=Path, required=True, help="new final MP4 path")
    parser.add_argument("--plan", type=Path, required=True, help="cinematic plan JSON with fps, width, height, transition_seconds and shots")
    parser.add_argument("--title", help="optional short opening title; requires ffmpeg drawtext (default: no text)")
    args = parser.parse_args(argv)

    def interrupted(_number, _frame):
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, interrupted)
    try:
        return encode(args)
    except KeyboardInterrupt:
        print("Encoding interrupted; owned subprocess stopped.", file=sys.stderr)
        return 130
    except (OSError, ValueError, KeyError, TypeError, ZeroDivisionError) as error:
        print(f"Cinematic encoding failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
