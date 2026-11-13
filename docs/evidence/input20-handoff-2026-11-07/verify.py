"""Verify archived input rejection; does not run the prepared v2 scenario."""
from pathlib import Path
import gzip
import hashlib
import json
import sys

here = Path(__file__).resolve().parent
sys.path.insert(0, str(here.parents[2] / "scripts"))
from public_playing_capacity import summarize_input, summarize_wire, validate_population

summary = json.loads((here / "summary.json").read_text())
data = {}
for name, record in summary["files"].items():
    compressed = (here / name).read_bytes()
    assert hashlib.sha256(compressed).hexdigest() == record["gzip_sha256"]
    raw = gzip.decompress(compressed)
    assert len(raw) == record["raw_bytes"]
    assert hashlib.sha256(raw).hexdigest() == record["raw_sha256"]
    data[record["source_path"]] = json.loads(raw)

blocking = data["blocking-build.json"]
for count in (10, 1000):
    report = data[f"live-v1/instrumented-{count}/report.json"]
    before = [report["initial_shards"][k] for k in sorted(report["initial_shards"], key=int)]
    after = [report["latest_shards"][k] for k in sorted(report["latest_shards"], key=int)]
    assert validate_population(before, count) and validate_population(after, count)
    assert summarize_input(before, after, 20) == report["input"]
    assert summarize_wire(before, after) == report["wire"]
    assert report["bots_sha256"] == next(c["sha256"] for c in blocking["commands"] if c["name"] == "bots")
    assert report["durable_rosters"]["human_seats"] == count
    assert report["durable_rosters"]["server_bot_seats"] == 0
    assert report["input"]["cached_snapshot_transforms"]["min_per_peer"] > 0
    if count == 10:
        assert report["status"] == "PASS" and report["input"]["all_peers_within_2_5_percent"]
    else:
        assert report["status"] == "FAIL" and not report["input"]["all_peers_within_2_5_percent"]
        assert report["error"] == "AssertionError: actual per-peer input cadence outside requested profile"
        assert report["soak_seconds"] == 120
        assert report["elapsed_seconds"] - report["all_running_after_seconds"] >= 120
fixture = data["live-v1/isolated-summary.json"]
assert fixture["status"] == "FAIL" and fixture["cleanup_complete"]
assert fixture["no_owned_process_groups_remain"] and not fixture["final_cleanup_inventory"]
assert fixture["career_schema"] == 13 and fixture["portal_schema"] == 11
checks = json.loads((here / "clippy-checks.json").read_text())
assert len(checks) == 4 and all(c["exit"] == 0 for c in checks)
assert data["build.json"]["commands"][1]["sha256"] != blocking["commands"][1]["sha256"]
print("Archive integrity/control PASS; large v1 remains FAIL; live nonblocking v2 NOT RUN.")
