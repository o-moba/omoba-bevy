#!/usr/bin/env python3
"""Validate the committed avatar roster and its retargeted animation clips.

For every avatar in `client/assets/avatars/manifest.json`, checks:
  * the GLB parses (valid glTF 2.0 binary container, JSON chunk decodes);
  * mandatory clips `idle` and `walk` are embedded (missing `attack`, `cast`,
    or `death` is a warning, per spec these are stretch clips);
  * every animation channel targets a node index present in the GLB node
    tree, and every rotation/translation channel targets a skin joint;
  * every clip has duration > 0;
  * the walk clip shows plausible motion: rotation keys on the hips/leg bones
    (resolved through the VRM humanoid bone map) with nontrivial variance,
    plus an animated hips translation channel;
  * manifest fields are complete (slug, display_name, collection,
    license == CC0, source_url, author) and the referenced thumbnail exists.

Also enforces the roster size window (10..20 shipped avatars).
Exits nonzero on any mandatory failure. Run from the repo root:

    python3 scripts/validate_avatar_assets.py [--avatars-dir DIR]
"""

import argparse
import json
import struct
import sys
from pathlib import Path

AVATARS_DIR = Path("client/assets/avatars")
MANDATORY_CLIPS = ("idle", "walk")
STRETCH_CLIPS = ("attack", "cast", "death")
ROSTER_MIN, ROSTER_MAX = 10, 20

GLB_MAGIC = b"glTF"
CHUNK_JSON = 0x4E4F534A
COMPONENT_COUNT = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}
FLOAT = 5126

# Bones (via the VRM humanoid map) that must visibly move during `walk`.
WALK_MOTION_BONES = ("leftUpperLeg", "rightUpperLeg", "leftLowerLeg", "rightLowerLeg")
ROTATION_VARIANCE_MIN = 1e-4
TRANSLATION_VARIANCE_MIN = 1e-6


def load_glb_json(path: Path) -> dict:
    data = path.read_bytes()
    if len(data) < 12:
        raise ValueError("file too small to be a GLB")
    magic, version, declared_len = struct.unpack("<4sII", data[:12])
    if magic != GLB_MAGIC:
        raise ValueError(f"bad GLB magic {magic!r}")
    if version != 2:
        raise ValueError(f"unsupported GLB version {version}")
    if declared_len > len(data):
        raise ValueError("declared length exceeds file size")
    offset = 12
    js = None
    binary = b""
    while offset + 8 <= declared_len:
        clen, ctype = struct.unpack("<II", data[offset : offset + 8])
        offset += 8
        chunk = data[offset : offset + clen]
        offset += clen
        if ctype == CHUNK_JSON:
            js = json.loads(chunk)
        elif ctype == 0x004E4942:
            binary = chunk
    if js is None:
        raise ValueError("missing JSON chunk")
    return js, binary


def read_float_accessor(js, binary, index):
    acc = js["accessors"][index]
    if acc["componentType"] != FLOAT:
        raise ValueError(f"accessor {index}: not float32")
    n = COMPONENT_COUNT[acc["type"]]
    bv = js["bufferViews"][acc["bufferView"]]
    start = bv.get("byteOffset", 0) + acc.get("byteOffset", 0)
    values = struct.unpack_from(f"<{acc['count'] * n}f", binary, start)
    if n == 1:
        return [(v,) for v in values]
    return [tuple(values[i * n : (i + 1) * n]) for i in range(acc["count"])]


def component_variance(samples):
    if len(samples) < 2:
        return 0.0
    best = 0.0
    for k in range(len(samples[0])):
        column = [s[k] for s in samples]
        mean = sum(column) / len(column)
        var = sum((v - mean) ** 2 for v in column) / len(column)
        best = max(best, var)
    return best


class Report:
    def __init__(self, slug):
        self.slug = slug
        self.errors = []
        self.warnings = []

    def error(self, msg):
        self.errors.append(msg)

    def warn(self, msg):
        self.warnings.append(msg)


def validate_avatar(entry, avatars_dir: Path) -> Report:
    slug = entry.get("slug", "<missing slug>")
    report = Report(slug)

    # Manifest completeness.
    for field in ("slug", "display_name", "collection", "license", "source_url", "author"):
        if not entry.get(field):
            report.error(f"manifest field {field!r} missing or empty")
    if entry.get("license") != "CC0":
        report.error(f"license is {entry.get('license')!r}, expected 'CC0'")
    if entry.get("source_url") and not str(entry["source_url"]).startswith("http"):
        report.error(f"source_url does not look like a URL: {entry['source_url']!r}")
    thumb = entry.get("thumbnail")
    if thumb:
        if not (avatars_dir / thumb).is_file():
            report.error(f"declared thumbnail {thumb!r} not found")
    else:
        report.warn("no thumbnail")

    glb_path = avatars_dir / f"{slug}.glb"
    if not glb_path.is_file():
        report.error(f"{glb_path} does not exist")
        return report
    try:
        js, binary = load_glb_json(glb_path)
    except (ValueError, json.JSONDecodeError, struct.error) as err:
        report.error(f"GLB parse failed: {err}")
        return report

    node_count = len(js.get("nodes", []))
    joints = set()
    for skin in js.get("skins", []):
        joints.update(skin.get("joints", []))
    anims = {a.get("name"): a for a in js.get("animations", [])}

    for clip in MANDATORY_CLIPS:
        if clip not in anims:
            report.error(f"mandatory clip {clip!r} missing")
    for clip in STRETCH_CLIPS:
        if clip not in anims:
            report.warn(f"stretch clip {clip!r} missing")

    humanoid = {}
    vrm = js.get("extensions", {}).get("VRM", {})
    for bone in vrm.get("humanoid", {}).get("humanBones", []):
        if "bone" in bone and "node" in bone:
            humanoid[bone["bone"]] = bone["node"]
    if not humanoid:
        report.error("no VRM humanoid bone map (cannot check walk plausibility)")

    for name, anim in anims.items():
        try:
            input_indices = set()
            for ch in anim.get("channels", []):
                node = ch.get("target", {}).get("node")
                if node is None or not 0 <= node < node_count:
                    report.error(f"clip {name!r}: channel targets invalid node {node}")
                    continue
                if joints and node not in joints:
                    report.error(
                        f"clip {name!r}: channel targets node {node} outside skin joints"
                    )
                sampler = anim["samplers"][ch["sampler"]]
                input_indices.add(sampler["input"])
            duration = 0.0
            for idx in input_indices:
                times = [v[0] for v in read_float_accessor(js, binary, idx)]
                if times != sorted(times):
                    report.error(f"clip {name!r}: input times not sorted")
                duration = max(duration, times[-1] if times else 0.0)
            if duration <= 0.0:
                report.error(f"clip {name!r}: duration {duration} (must be > 0)")
        except (KeyError, IndexError, ValueError, struct.error) as err:
            report.error(f"clip {name!r}: malformed ({err})")

    # Walk plausibility: legs must rotate, hips must translate.
    walk = anims.get("walk")
    if walk and humanoid:
        by_target = {}
        for ch in walk.get("channels", []):
            key = (ch["target"].get("node"), ch["target"].get("path"))
            by_target[key] = walk["samplers"][ch["sampler"]]
        moving_legs = 0
        for bone in WALK_MOTION_BONES:
            node = humanoid.get(bone)
            sampler = by_target.get((node, "rotation")) if node is not None else None
            if sampler is None:
                continue
            var = component_variance(read_float_accessor(js, binary, sampler["output"]))
            if var > ROTATION_VARIANCE_MIN:
                moving_legs += 1
        if moving_legs < 2:
            report.error(
                f"walk clip: only {moving_legs} leg bones show rotation variance "
                f"> {ROTATION_VARIANCE_MIN} (expected >= 2)"
            )
        hips_node = humanoid.get("hips")
        hips_sampler = by_target.get((hips_node, "translation"))
        if hips_sampler is None:
            report.error("walk clip: no hips translation channel")
        else:
            var = component_variance(
                read_float_accessor(js, binary, hips_sampler["output"])
            )
            if var <= TRANSLATION_VARIANCE_MIN:
                report.error(f"walk clip: hips translation variance {var:.2e} too low")
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--avatars-dir", type=Path, default=AVATARS_DIR)
    args = parser.parse_args()

    manifest_path = args.avatars_dir / "manifest.json"
    try:
        manifest = json.loads(manifest_path.read_text())
    except (OSError, json.JSONDecodeError) as err:
        print(f"FATAL: cannot read {manifest_path}: {err}")
        return 2
    avatars = manifest.get("avatars", [])

    failed = []
    total_bytes = 0
    for entry in avatars:
        report = validate_avatar(entry, args.avatars_dir)
        glb = args.avatars_dir / f"{report.slug}.glb"
        size = glb.stat().st_size if glb.is_file() else 0
        total_bytes += size
        thumb = entry.get("thumbnail")
        if thumb and (args.avatars_dir / thumb).is_file():
            total_bytes += (args.avatars_dir / thumb).stat().st_size
        status = "FAIL" if report.errors else "PASS"
        print(f"{status}  {report.slug} ({size / 1e6:.1f} MB)")
        for msg in report.errors:
            print(f"      ERROR: {msg}")
        for msg in report.warnings:
            print(f"      warn:  {msg}")
        if report.errors:
            failed.append(report.slug)

    print()
    print(f"Roster: {len(avatars)} avatars, {total_bytes / 1e6:.1f} MB (GLBs + thumbnails)")
    ok = not failed
    if not ROSTER_MIN <= len(avatars) <= ROSTER_MAX:
        print(f"ERROR: roster size {len(avatars)} outside [{ROSTER_MIN}, {ROSTER_MAX}]")
        ok = False
    if failed:
        print(f"FAILED avatars: {', '.join(failed)}")
    print("RESULT:", "PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
