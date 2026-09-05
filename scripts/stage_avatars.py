#!/usr/bin/env python3
"""Stage CC0 VRM avatars from the Open Source Avatars collection into the client.

VRM 0.x files are glTF 2.0 binary containers (GLB), so staging is a
validate-and-copy: each `model.vrm` is checked for a valid GLB header and
copied to `client/assets/avatars/<slug>.glb`; `thumbnail.png` (when present)
is copied to `client/assets/avatars/<slug>.<ext>` where `<ext>` reflects the
file's ACTUAL image format (several collection thumbnails are JPEGs with a
`.png` name, and the Bevy image loader picks its decoder by extension).
A `manifest.json` is written alongside with provenance (collection, license,
source URL, author) taken from each avatar's `meta.json`.

The roster below retains the 15 approved release avatars (VRM 0.x,
Mixamo-compatible skeletons); excluded distribution-review models are not staged. The separate `bosses` set (TASK-19) stages the raid-boss models
into `client/assets/bosses/` with the same manifest schema, keeping boss slugs
out of the player-selectable roster manifest. Run from the repo root:

    python3 scripts/stage_avatars.py [--collection-root PATH] [--out DIR]
    python3 scripts/stage_avatars.py --set bosses
"""

import argparse
import json
import shutil
import struct
import sys
from pathlib import Path

GLB_MAGIC = b"glTF"
GLB_VERSION = 2

DEFAULT_COLLECTION_ROOT = Path("/Users/wotori/git/opensourceavatars/avatars")
DEFAULT_OUT_DIR = Path("client/assets/avatars")

# slug -> directory under the collection root (TASK-17 spec shortlist).
ROSTER = {
    "agnes": "100avatars-r3/5bb03b12-bf87-44b6-acc6-fb385e1d502e",
    "anna": "100avatars-r3/8dd2c5c1-a58a-4139-aa5e-b7440b06522e",
    "megan-the-fox": "100avatars-r3/b88d28fa-c295-4415-a870-1c89608f6389",
    "lady-koi": "100avatars-r3/46dcd439-e8b8-4cff-97b1-11aed72d0621",
    "bao-samurai": "100avatars-r3/4d79d1a4-b248-4b32-afb7-ebb71174906e",
    "good-knight": "100avatars-r3/c4b17286-7dc6-4ec0-9375-b875de22dae3",
    "crowley": "100avatars-r3/4786bb21-9fa9-4b75-820c-200f04a7f579",
    "cyberpal": "100avatars-r3/afb4a06f-6bf9-4cac-a192-6f516bc79fad",
    "sport-mecha": "100avatars-r3/2e44dd36-b462-4503-ac72-dbc53e8101c7",
    "pirate-bot": "100avatars-r3/a1e071e0-f9c9-4e77-b4b2-831a1784812d",
    "mega-angel": "100avatars-r3/79461dba-2866-47bf-b79a-c6602c05a296",
    "stitch-witch": "100avatars-r3/a19cbe62-6577-4be6-adfa-cfb26d1ff378",
    "cool-tiger": "100avatars-r3/fcd73da7-6c89-46fe-8467-3eb4873248fa",
    "aurora": "toxsam/0xc1def47cf1e15ee8c2a92f4e0e968372880d18d1_1",
    "orion": "toxsam/0xc1def47cf1e15ee8c2a92f4e0e968372880d18d1_0",
}

# Raid-boss models (TASK-19). Staged into client/assets/bosses/ — deliberately
# NOT part of ROSTER so boss slugs never enter the player-selectable manifest.
BOSSES = {
    "king-mutatio": "toxsam/0x59202483529a11642a43578a6ee77ca4ec24f930_0",
}

AVATAR_SETS = {
    "roster": (ROSTER, DEFAULT_OUT_DIR),
    "bosses": (BOSSES, Path("client/assets/bosses")),
}


def sniff_image_extension(path: Path) -> str:
    """Actual image format by magic bytes (extension must match the decoder)."""
    header = path.read_bytes()[:8]
    if header.startswith(b"\x89PNG\r\n\x1a\n"):
        return "png"
    if header.startswith(b"\xff\xd8\xff"):
        return "jpg"
    raise ValueError(f"unsupported thumbnail format (magic {header[:4]!r})")


def validate_glb(data: bytes) -> None:
    """Same validation contract as scripts/convert_vrm_to_glb.py."""
    if len(data) < 12:
        raise ValueError(f"file too small to be a GLB ({len(data)} bytes)")
    magic, version, declared_len = struct.unpack("<4sII", data[:12])
    if magic != GLB_MAGIC:
        raise ValueError(f"bad GLB magic: {magic!r} (expected {GLB_MAGIC!r})")
    if version != GLB_VERSION:
        raise ValueError(f"unsupported GLB version {version} (expected {GLB_VERSION})")
    if declared_len > len(data):
        raise ValueError(f"declared length {declared_len} exceeds buffer {len(data)}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--collection-root", type=Path, default=DEFAULT_COLLECTION_ROOT)
    parser.add_argument(
        "--set",
        dest="avatar_set",
        choices=sorted(AVATAR_SETS),
        default="roster",
        help="which avatar set to stage (roster -> client/assets/avatars, bosses -> client/assets/bosses)",
    )
    parser.add_argument("--out", type=Path, default=None, help="override the set's output directory")
    args = parser.parse_args()

    entries, default_out = AVATAR_SETS[args.avatar_set]
    out_dir: Path = args.out if args.out is not None else default_out
    out_dir.mkdir(parents=True, exist_ok=True)
    manifest = {"avatars": []}
    failures = []

    for slug, rel_dir in entries.items():
        src_dir = args.collection_root / rel_dir
        model = src_dir / "model.vrm"
        meta_path = src_dir / "meta.json"
        thumb = src_dir / "thumbnail.png"
        try:
            data = model.read_bytes()
            validate_glb(data)
            meta = json.loads(meta_path.read_text())
        except (OSError, ValueError, json.JSONDecodeError) as err:
            failures.append((slug, str(err)))
            print(f"FAIL  {slug}: {err}")
            continue

        license_name = meta.get("license", "")
        if license_name != "CC0":
            failures.append((slug, f"license is {license_name!r}, expected CC0"))
            print(f"FAIL  {slug}: license {license_name!r} != CC0")
            continue

        (out_dir / f"{slug}.glb").write_bytes(data)
        thumb_name = None
        if thumb.is_file():
            ext = sniff_image_extension(thumb)
            thumb_name = f"{slug}.{ext}"
            shutil.copyfile(thumb, out_dir / thumb_name)

        entry = {
            "slug": slug,
            "display_name": meta.get("name", slug),
            "collection": meta.get("collection_name", ""),
            "license": license_name,
            "source_url": meta.get("model_file_url", ""),
            "author": meta.get("author", ""),
            "thumbnail": thumb_name,
        }
        manifest["avatars"].append(entry)
        thumb_note = "" if thumb_name else " (no thumbnail in source)"
        print(f"OK    {slug}: {len(data)} bytes{thumb_note}")

    manifest_path = out_dir / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"\nStaged {len(manifest['avatars'])}/{len(entries)} avatars -> {out_dir}")
    print(f"Manifest: {manifest_path}")
    if failures:
        print(f"Failures: {failures}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
