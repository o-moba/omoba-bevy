#!/usr/bin/env python3
"""Recompute the Lobby identity archive with only the Python standard library."""
import hashlib
import json
import struct
from pathlib import Path

HERE = Path(__file__).resolve().parent


def read_json(name):
    return json.loads((HERE / name).read_text(encoding="utf-8"))


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


metadata = read_json("metadata.json")
assert metadata["status"] == "PASS"
manifest = read_json("artifact-sha256.json")
for name, expected in manifest.items():
    assert sha256(HERE / name) == expected, f"artifact hash mismatch: {name}"

build = read_json("build.json")
capture = read_json("capture.json")
assert build["source_parent"] == metadata["native_source"]
assert build["workflow_run_id"] == metadata["release_run_id"]
assert capture["build_source_parent"] == build["source_parent"]
assert capture["head_at_capture"] == metadata["capture_helper_head"]
assert capture["binary_sha256"] == build["binary_sha256"]
assert capture["runner_sha256"] == sha256(HERE / "capture_offscreen.py")
assert capture["status"] == "captured_pending_visual_inspection"

summary = capture["summary"]
assert summary["status"] == "passed"
assert summary["viewport"] == metadata["viewport"] == [852, 393]
assert summary["ui_profile"] == "Mobile" and summary["locale"] == "ru"
assert summary["debug_assertions"] is True and summary["color_pixels"] is True
assert summary["physical_device"] is False and summary["synthetic_state"] is True
assert capture["observed"]["screen"] == "Lobby"

labels = capture["observed"]["labels"]
tags = {name: text for name, text in labels.items() if name.startswith("LobbyRosterTag-")}
assert len(tags) == 5
assert set(tags.values()) == set(metadata["all_tags_in_qa_state"])
names = [text for name, text in labels.items() if name.startswith("LobbyRosterName-")]
assert len(names) == 5 and all(text.startswith("WWWWW") and text.endswith("…") for text in names)

checked_geometry = 0
for name, geometry in capture["observed"]["label_geometry"].items():
    layout = geometry["layout_size"]
    node = geometry["node_size"]
    assert all(layout[axis] <= node[axis] + 1.0 for axis in (0, 1)), name
    checked_geometry += 1
assert checked_geometry >= len(labels)

for name, expected in capture["source_hashes"].items():
    archived = HERE / "native-source" / name
    assert archived.is_file(), name
    assert sha256(archived) == expected, f"native source mismatch: {name}"
assert len(capture["source_hashes"]) == 19

image = HERE / "wide-lobby.png"
raw = image.read_bytes()
assert raw[:8] == b"\x89PNG\r\n\x1a\n"
assert list(struct.unpack(">II", raw[16:24])) == metadata["viewport"]
assert capture["screenshot"]["bytes"] == len(raw)
assert capture["screenshot"]["sha256"] == sha256(image)
assert "panicked at " not in (HERE / "client.log").read_text(encoding="utf-8", errors="replace")

print(json.dumps({
    "status": "PASS",
    "native_source": metadata["native_source"],
    "capture_helper_head": metadata["capture_helper_head"],
    "release_run": metadata["release_run_id"],
    "tags_in_state": sorted(tags.values()),
    "visible_tags_in_png": metadata["visible_tags_in_png"],
    "label_geometries": checked_geometry,
    "native_sources": len(capture["source_hashes"]),
    "artifacts_hashed": len(manifest),
    "visual_inspection": "Recorded in README; not an automated assertion",
}, ensure_ascii=False, sort_keys=True))
