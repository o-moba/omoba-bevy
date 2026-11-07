"""Verify recorded producer measurements and rejection; no live load executes."""
from pathlib import Path
import gzip
import hashlib
import json
import sys

here = Path(__file__).resolve().parent
sys.path.insert(0, str(here.parents[2] / "scripts"))
from public_playing_capacity import (
    summarize_input, summarize_producer_timing, summarize_wire, validate_population,
)

summary = json.loads((here / "summary.json").read_text())
data = {}
for name, record in summary["files"].items():
    compressed = (here / name).read_bytes()
    assert hashlib.sha256(compressed).hexdigest() == record["gzip_sha256"]
    raw = gzip.decompress(compressed)
    assert len(raw) == record["raw_bytes"]
    assert hashlib.sha256(raw).hexdigest() == record["raw_sha256"]
    data[record["source_path"]] = json.loads(raw) if record["source_path"].endswith(".json") else raw.decode()

for version, count, build_name in ((2, 10, "nonblocking-build.json"), (2, 1000, "nonblocking-build.json"),
                                  (3, 10, "timed-build.json"), (4, 1000, "timed-build.json"),
                                  (5, 1000, "build.json")):
    report = data[f"live-v{version}/instrumented-{count}/report.json"]
    before = [report["initial_shards"][k] for k in sorted(report["initial_shards"], key=int)]
    after = [report["latest_shards"][k] for k in sorted(report["latest_shards"], key=int)]
    assert validate_population(before, count) and validate_population(after, count)
    assert summarize_input(before, after, 20) == report["input"]
    assert summarize_wire(before, after) == report["wire"]
    if "producer_timing" in report:
        assert summarize_producer_timing(before, after) == report["producer_timing"]
    build = data[build_name]
    assert report["bots_sha256"] == next(c["sha256"] for c in build["commands"] if c["name"] == "bots")
    assert report["server_sha256"] == "5c6f4e6fc0795e5008cdd423168b17c515ed93c7b2eda3cd839aa514925c0edd"
    assert report["durable_rosters"]["human_seats"] == count
    assert report["durable_rosters"]["running_workers"] == count // 10
    assert report["durable_rosters"]["server_bot_seats"] == 0
    for sample in after:
        assert all(p["public"]["authority_resets"] == 0 for p in sample["peers"])
    assert report["input"]["all_peers_within_2_5_percent"] == (count == 10)
    assert report["status"] == ("PASS" if count == 10 else "FAIL")
    if count == 1000:
        assert report["error"] == "AssertionError: actual per-peer input cadence outside requested profile"
        assert report["soak_seconds"] == 120
        assert report["elapsed_seconds"] - report["all_running_after_seconds"] >= 120
for version in (2, 3, 4, 5):
    fixture = data[f"live-v{version}/isolated-summary.json"]
    assert fixture["status"] == "FAIL"
    assert fixture["cleanup_complete"] and fixture["no_owned_process_groups_remain"]
    assert not fixture["final_cleanup_inventory"]
    assert fixture["career_schema"] == 13 and fixture["portal_schema"] == 11
    assert all(value is False for value in fixture["runtime_role"].values())
assert "Address already in use" in data["live-v3/instrumented-1000.log"]
checks = data["clippy-checks.json"]
assert len(checks) == 4 and all(c["exit"] == 0 for c in checks)
assert "38 passed; 0 failed" in data["library-tests.log"]
print("Archive/rates/timing/cleanup verified; all completed full-target20Hz runs remain rejected.")
