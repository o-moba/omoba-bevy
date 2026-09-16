"""Render previews or complete shots from the separate cinematic .blend.

blender -b cinematic.blend --python render_cinematic.py -- --output FRAME_DIR
Add --preview for start/middle/end frames at half resolution.
"""
import argparse
import json
import sys
import time
from pathlib import Path

import bpy


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--preview", action="store_true")
    parser.add_argument("--samples", type=int, default=None)
    parser.add_argument("--shot", action="append")
    parser.add_argument("--resume", action="store_true")
    args = parser.parse_args(sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else [])
    scene = bpy.context.scene
    shots = json.loads(scene["cinematic_shots"])
    if args.shot and not set(args.shot).issubset({shot["id"] for shot in shots}):
        raise ValueError("Unknown shot")
    scene.render.resolution_percentage = 50 if args.preview else 100
    if args.preview:
        scene.eevee.taa_render_samples = 16
    if args.samples is not None:
        scene.eevee.taa_render_samples = args.samples
    for shot in shots:
        if args.shot and shot["id"] not in args.shot:
            continue
        folder = args.output.resolve() / shot["id"]
        folder.mkdir(parents=True, exist_ok=True)
        count = shot["frame_count"]
        indices = [1, count // 2, count] if args.preview else range(1, count + 1)
        scene.camera = bpy.data.objects[shot["camera"]]
        for local in indices:
            output = folder / f"frame_{local:04d}.png"
            if args.resume and output.exists():
                continue
            started = time.monotonic()
            scene.frame_set(shot["frame_start"] + local - 1)
            scene.camera = bpy.data.objects[shot["camera"]]
            scene.render.filepath = str(output)
            bpy.ops.render.render(write_still=True)
            print("CINEMATIC_FRAME " + json.dumps({"shot": shot["id"], "local_frame": local,
                  "seconds": round(time.monotonic() - started, 3), "path": str(output)}), flush=True)


if __name__ == "__main__":
    main()
