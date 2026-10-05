#!/usr/bin/env python3
"""Extract the frames the edit shows into public/gp/<clip>/<frame>.jpg.

    python3 prepare.py [<folder with the recorder's clips>]    (default ../sources/clips)

The edit reads still frames, not video: frame n of a clip is exactly one
captured frame, whatever the renderer's video seeking does. Ranges come from
src/shots.ts (`from`, `len`); frames already on disk are kept. Needs ffmpeg.
"""
from pathlib import Path
import re
import subprocess
import sys

HERE = Path(__file__).resolve().parent
FPS = 60


def ranges():
    """clip -> sorted frame numbers used by any shot."""
    wanted = {}
    text = (HERE / "src/shots.ts").read_text()
    for clip, start, length in re.findall(r'clip: "([\w-]+)", from: ([\d.]+), len: ([\d.]+)', text):
        first = round(float(start) * FPS)
        wanted.setdefault(clip, set()).update(range(first, first + round(float(length) * FPS) + 1))
    return {clip: sorted(frames) for clip, frames in wanted.items()}


def runs(frames):
    """Consecutive runs of frame numbers as (first, count)."""
    start = previous = frames[0]
    for frame in frames[1:]:
        if frame != previous + 1:
            yield start, previous - start + 1
            start = frame
        previous = frame
    yield start, previous - start + 1


def main():
    source = Path(sys.argv[1]) if len(sys.argv) > 1 else HERE.parent / "sources/clips"
    for clip, frames in ranges().items():
        folder = HERE / "public/gp" / clip
        folder.mkdir(parents=True, exist_ok=True)
        missing = [frame for frame in frames if not (folder / f"{frame:06d}.jpg").is_file()]
        if not missing:
            continue
        for first, count in runs(missing):
            # Exact frames by number; JPEG is full-range BT.601, the clips limited-range BT.709.
            subprocess.run(["ffmpeg", "-v", "error", "-y", "-ss", f"{max(first - 30, 0) / FPS:.6f}", "-i",
                            str(source / f"{clip}.mp4"), "-an",
                            "-vf", f"select='gte(n\\,{first - max(first - 30, 0)})',"
                                   "scale=in_color_matrix=bt709:in_range=tv:out_color_matrix=bt601:out_range=pc",
                            "-fps_mode", "passthrough", "-frames:v", str(count), "-q:v", "2", "-start_number", str(first),
                            str(folder / "%06d.jpg")], check=True)
        print(f"{clip}: {len(missing)} frames")


if __name__ == "__main__":
    main()
