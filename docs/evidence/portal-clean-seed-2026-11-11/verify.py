#!/usr/bin/env python3
"""Standard-library verification of the retained clean-seed acceptance pack."""
import hashlib
import json
import re
import struct
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]


def read_json(name):
    return json.loads((HERE / name).read_text(encoding="utf-8"))


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


metadata = read_json("metadata.json")
assert metadata["status"] == "PASS"
assert (HERE / "game-commit.txt").read_text().strip() == metadata["game_merge_commit"]
assert (HERE / "portal-commit.txt").read_text().strip() == metadata["portal_commit"]

manifest = read_json("artifact-sha256.json")
for name, expected in manifest.items():
    assert sha256(HERE / name) == expected, f"artifact hash mismatch: {name}"

unit = (HERE / "unit-tests.log").read_text(encoding="utf-8")
counts = {
    key: int(re.search(rf"^ℹ {label} (\d+)$", unit, re.MULTILINE).group(1))
    for key, label in [
        ("tests", "tests"),
        ("passed", "pass"),
        ("failed", "fail"),
        ("expected_skips", "skipped"),
    ]
}
assert counts == metadata["unit"], counts
assert counts["passed"] + counts["failed"] + counts["expected_skips"] == counts["tests"]
assert "tsc --noEmit" in (HERE / "typecheck.log").read_text(encoding="utf-8")
build = (HERE / "build.log").read_text(encoding="utf-8")
assert "Compiled successfully" in build and "Generating static pages using 3 workers (9/9)" in build

seed = read_json("seed-result.json")
result = read_json("browser/result.json")
assert seed["synthetic"] is True and seed["saved"] is True
assert len(seed["result_ids"]) == 2 and len(set(seed["result_ids"])) == 2
assert result["passed"] is True and result["entries"] == len(result["ratings"]) >= 2
ratings = result["ratings"]
assert len({row["rating"] for row in ratings}) >= 2
assert [row["rank"] for row in ratings] == list(range(1, len(ratings) + 1))
assert [row["rating"] for row in ratings] == sorted((row["rating"] for row in ratings), reverse=True)
assert seed["leaderboard_profile_ids"] == [row["profile_id"] for row in ratings]
assert [row["rating"] for row in ratings] == metadata["browser"]["ratings"]
placement = result["placement"]
assert placement["lifetime_rated_matches"] >= 20
assert 0 <= placement["season_rated_matches"] < 20
assert placement["text"] == f"Калибровка {placement['season_rated_matches']}/20"
assert placement["text"] == metadata["browser"]["placement_text"]
assert "PASS leaderboard ordering" in (HERE / "leaderboard.log").read_text(encoding="utf-8")
assert "Account API listening on 127.0.0.1:40550" in (HERE / "account-api.log").read_text(encoding="utf-8")
assert "Ready in" in (HERE / "portal.log").read_text(encoding="utf-8")

source_lines = (HERE / "source-and-binary-sha256.txt").read_text(encoding="utf-8").splitlines()
assert len(source_lines) == 4
source_hashes = {line.split(maxsplit=1)[1]: line.split(maxsplit=1)[0] for line in source_lines}
game_seed = "game/account-api/examples/seed-local-portal.rs"
assert source_hashes[game_seed] == sha256(ROOT / "account-api/examples/seed-local-portal.rs")
portal_test = "portal/tests/leaderboard.e2e.mjs"
portal_source = ROOT.parent / "mm-web/tests/leaderboard.e2e.mjs"
portal_source_checked = portal_source.exists()
if portal_source_checked:
    assert source_hashes[portal_test] == sha256(portal_source)

png_dimensions = {}
for name in ["mobile.png", "hidden.png"]:
    raw = (HERE / "browser" / name).read_bytes()
    assert raw[:8] == b"\x89PNG\r\n\x1a\n"
    width, height = struct.unpack(">II", raw[16:24])
    assert width == metadata["browser"]["mobile_viewport_width"] and height > 0
    assert len(raw) > 10000
    png_dimensions[name] = [width, height]

print(json.dumps({
    "status": "PASS",
    "run_id": metadata["run_id"],
    "unit": counts,
    "entries": result["entries"],
    "ratings": metadata["browser"]["ratings"],
    "placement": placement["text"],
    "png_dimensions": png_dimensions,
    "game_seed_source_checked": True,
    "portal_test_source_checked": portal_source_checked,
    "artifacts_hashed": len(manifest),
    "visual_inspection": "Recorded in README; not an automated assertion",
}, ensure_ascii=False, sort_keys=True))
