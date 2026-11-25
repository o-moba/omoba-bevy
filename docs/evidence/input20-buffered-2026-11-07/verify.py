"""Verify measured local input20 evidence; no new live game runs."""
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

for version, count in ((6, 1000), (7, 10), (7, 1000)):
    report = data[f"live-v{version}/instrumented-{count}/report.json"]
    before = [report["initial_shards"][k] for k in sorted(report["initial_shards"], key=int)]
    after = [report["latest_shards"][k] for k in sorted(report["latest_shards"], key=int)]
    assert validate_population(before, count) and validate_population(after, count)
    assert summarize_input(before, after, 20) == report["input"]
    assert summarize_wire(before, after) == report["wire"]
    assert summarize_producer_timing(before, after) == report["producer_timing"]
    build = data["stages-before-buffer-build.json" if version == 6 else "build.json"]
    assert report["bots_sha256"] == next(c["sha256"] for c in build["commands"] if c["name"] == "bots")
    assert report["server_sha256"] == "5c6f4e6fc0795e5008cdd423168b17c515ed93c7b2eda3cd839aa514925c0edd"
    assert report["durable_rosters"]["human_seats"] == count
    assert report["durable_rosters"]["running_workers"] == count // 10
    assert report["durable_rosters"]["distinct_endpoints"] == count // 10
    assert report["durable_rosters"]["server_bot_seats"] == 0
    assert all(p["public"]["authority_resets"] == 0 for sample in after for p in sample["peers"])
    assert report["input"]["all_peers_within_2_5_percent"] == (version == 7)
    assert report["status"] == ("PASS" if version == 7 else "FAIL")
    if version == 6:
        assert report["soak_seconds"] == 30
        assert report["error"] == "AssertionError: actual per-peer input cadence outside requested profile"
    elif count == 1000:
        assert report["soak_seconds"] == 120
        assert report["elapsed_seconds"] - report["all_running_after_seconds"] >= 120
        assert report["input"]["cached_snapshot_transforms"]["min_per_peer"] > 0
        assert not report["before_cleanup"]["scan_limited"]
        workers = report["before_cleanup"]["workers"]
        assert len(workers) == 100
        for row in workers:
            health, status = row["health"], row["status"]
            assert status["phase"] == "running" and health["server_epoch"] == status["server_epoch"]
            assert health["version"] == 2 and health["sql_ready"] and health["journal_ready"]
            assert not health["reply_receiver_closed"] and health["blocked_since_ms"] is None
            assert all(health[k] == 0 for k in ("staged_dodges", "legacy_floors", "unpersisted_results", "critical_replies"))
for version in (6, 7):
    fixture = data[f"live-v{version}/isolated-summary.json"]
    assert fixture["status"] == ("FAIL" if version == 6 else "PASS")
    assert fixture["cleanup_complete"] and fixture["no_owned_process_groups_remain"]
    assert not fixture["final_cleanup_inventory"]
    assert fixture["career_schema"] == 13 and fixture["portal_schema"] == 11
    assert all(value is False for value in fixture["runtime_role"].values())
checks = data["clippy-checks.json"]
assert len(checks) == 4 and all(c["exit"] == 0 for c in checks)
log = data["library-tests.log"]
assert "41 passed; 0 failed" in log
for name in ("unchanged_large_record_uses_one_sink_write", "interrupted_and_short_writes_preserve_the_whole_record", "zero_write_failure_is_not_reported_as_delivery"):
    assert f"telemetry::tests::{name} ... ok" in log
print("V6 rejection retained; V7 local1000/120s actual-input20/rosters/health/cleanup PASS. WAN/latency/RU remain open.")
