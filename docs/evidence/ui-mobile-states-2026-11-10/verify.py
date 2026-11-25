#!/usr/bin/env python3
"""Recompute the archived UI packet's binary, image, and geometry links.

The raw captures deliberately retain `captured_pending_visual_inspection`.
This verifier checks machine-readable invariants only; README records the separate
human visual verdicts. It does not claim a physical-device or live-server test.
"""

import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
CASES = (
    ("compatibility-19c2616d", "19c2616d", "HomeCompatibilityDetail"),
    ("wide-draft-5f314981", "5f314981", "PrematchStageText-"),
    ("wide-lobby-5f314981", "5f314981", "LobbyMemberName-"),
)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> None:
    builds = {}
    for build_path in (ROOT / "accepted").glob("build-*.json"):
        build = json.loads(build_path.read_text())
        builds[build["source_parent"]] = build

    checked_labels = 0
    for stem, short_sha, required_prefix in CASES:
        capture_path = ROOT / "accepted" / f"{stem}-capture.json"
        image_path = ROOT / "accepted" / f"{stem}.png"
        capture = json.loads(capture_path.read_text())
        assert capture["status"] == "captured_pending_visual_inspection"
        assert capture["summary"]["status"] == "passed"
        assert capture["summary"]["viewport"] == [852, 393]
        assert capture["summary"]["ui_profile"] == "Mobile"
        assert capture["summary"]["locale"] == "ru"
        assert capture["summary"]["physical_device"] is False
        assert capture["head_at_capture"].startswith(short_sha)
        assert capture["binary_sha256"] == builds[capture["head_at_capture"]]["binary_sha256"]
        assert capture["screenshot"]["bytes"] == image_path.stat().st_size
        assert capture["screenshot"]["sha256"] == sha256(image_path)
        assert required_prefix in capture["observed"]["labels"] or any(
            key.startswith(required_prefix) for key in capture["observed"]["labels"]
        )
        for key, geometry in capture["observed"]["label_geometry"].items():
            layout_width, layout_height = geometry["layout_size"]
            node_width, node_height = geometry["node_size"]
            assert layout_width <= node_width + 1.0, (stem, key, "width")
            assert layout_height <= node_height + 1.0, (stem, key, "height")
            checked_labels += 1
        if stem.startswith("wide-"):
            for key, text in capture["observed"]["labels"].items():
                if key.startswith(("PrematchStageText-", "LobbyMemberName-", "LobbyRosterName-")):
                    assert text.strip("…"), (stem, key, text)
                    if key.endswith("-0") or key.startswith("LobbyRosterName-"):
                        assert "…" in text, (stem, key, text)

    for capture_path in (ROOT / "rejected").glob("*-capture.json"):
        capture = json.loads(capture_path.read_text())
        assert capture["head_at_capture"] == "19c2616d5150f4a1256eb85a8cb7cf2ea2009478"
        assert capture["screenshot"]["sha256"] == sha256(
            capture_path.with_name(capture_path.name.replace("-capture.json", ".png"))
        )

    print(f"PASS: {len(CASES)} accepted captures, {checked_labels} label geometries, 2 rejected captures")
    print("Scope: offscreen synthetic RU Mobile preview; manual visual verdicts remain in README.md")


if __name__ == "__main__":
    main()
