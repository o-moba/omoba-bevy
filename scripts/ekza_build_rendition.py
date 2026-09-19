#!/usr/bin/env python3
"""Build Omoba's rendition of an avatar: VRM in, playable GLB out.

This is the builder behind the Ekza rendition profile `desktop / humanoid-glb-v1`.
The Ekza registry runs it as an external command for every avatar a creator wants
in Omoba, so the requirements and the code that satisfies them live with the game.

    python3 scripts/ekza_build_rendition.py --source avatar.vrm --output-dir out/

Contract (stable; the registry depends on it):
  * stdout carries exactly one JSON document, nothing else.
  * Success, exit 0:
      {"ok": true, "profile": "desktop/humanoid-glb-v1", "format": "glb",
       "assetPath": "<output-dir>/<sha256>.glb", "sha256": "...", "sizeBytes": N,
       "report": {"sourceFormat": "vrm0|vrm1", "sourceSha256": "...", "clips": [...]}}
  * The avatar cannot become a valid rendition, exit 2:
      {"ok": false, "issues": [{"code": "...", "message": "..."}]}
  * Anything else (bad arguments, missing animation library) exits 1.

It never reads the network, never writes outside --output-dir, and the output name
is the content hash, so a repeated build of the same source is idempotent.
Standard library only.
"""

from __future__ import annotations

import argparse
import contextlib
import hashlib
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import ekza_publish as publish  # noqa: E402

PROFILE = "desktop/humanoid-glb-v1"
MAX_BYTES = publish.MAX_MODEL_BYTES


def issue(code: str, message: str) -> dict:
    return {"code": code, "message": message}


def build(source: bytes) -> tuple[bytes | None, dict]:
    """Return (rendition, report) or (None, {"issues": [...]})."""
    if not 20 <= len(source) <= MAX_BYTES:
        return None, {
            "issues": [issue("source_size", f"source must be 20 bytes to {MAX_BYTES} bytes")]
        }
    try:
        document = publish.glb_document(source)
    except (publish.PublishError, ValueError) as error:
        return None, {"issues": [issue("source_container", str(error))]}
    try:
        source_format = publish.vrm_format(document)
    except publish.PublishError as error:
        return None, {"issues": [issue("source_not_vrm", str(error))]}
    try:
        # The shared build step prints progress; stdout is reserved for the result.
        with contextlib.redirect_stdout(sys.stderr):
            rendition = publish.build_omoba_rendition(source, verbose=False)
    except publish.PublishError as error:
        text = str(error)
        code = "animation_library_missing" if "animation library missing" in text else "build_failed"
        if code == "animation_library_missing":
            raise SystemExit(text)
        return None, {"issues": [issue(code, text)]}
    if len(rendition) > MAX_BYTES:
        return None, {
            "issues": [issue("rendition_size", f"built rendition exceeds {MAX_BYTES} bytes")]
        }
    return rendition, {
        "sourceFormat": source_format,
        "sourceSha256": hashlib.sha256(source).hexdigest(),
        "clips": list(publish.REQUIRED_CLIPS),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    if not args.source.is_file():
        parser.error(f"source is not a file: {args.source}")
    if args.source.stat().st_size > MAX_BYTES:
        print(json.dumps({"ok": False, "issues": [issue("source_size", "source is too large")]}))
        return 2
    rendition, report = build(args.source.read_bytes())
    if rendition is None:
        print(json.dumps({"ok": False, **report}))
        return 2
    digest = hashlib.sha256(rendition).hexdigest()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    target = args.output_dir / f"{digest}.glb"
    staging = args.output_dir / f".{digest}.partial"
    staging.write_bytes(rendition)
    staging.replace(target)
    print(
        json.dumps(
            {
                "ok": True,
                "profile": PROFILE,
                "format": "glb",
                "assetPath": str(target),
                "sha256": digest,
                "sizeBytes": len(rendition),
                "report": report,
            }
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
