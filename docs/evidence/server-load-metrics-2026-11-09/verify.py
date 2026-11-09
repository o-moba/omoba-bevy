"""Recompute the instrumented local server-load evidence; starts no game."""
from pathlib import Path
import gzip
import hashlib
import json
import sys
import tempfile

here = Path(__file__).resolve().parent
root = here.parents[2]
sys.path.insert(0, str(root / "scripts"))
from public_playing_capacity import (
    summarize_input, summarize_producer_timing, summarize_server_load,
    summarize_wire, validate_population,
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

build = data[".agent/codex-ranked-readiness/load-server-metrics/build.json"]
bots_build = data[".agent/codex-ranked-readiness/load-input20/build.json"]
server_hash = next(item["output_sha256"] for item in build["commands"] if item["name"] == "server")
bots_hash = next(item["sha256"] for item in bots_build["commands"] if item["name"] == "bots")
assert build["status"] == "PASS"
for clients in (10, 1000):
    prefix = f".agent/codex-ranked-readiness/load-server-metrics/live-v2/instrumented-{clients}"
    report = data[f"{prefix}/report.json"]
    metric_workers = data[f"live-v2/instrumented-{clients}/load-metrics.json"]
    observed = json.loads((here / f"observed-{clients}.json").read_text())
    before = [report["initial_shards"][key] for key in sorted(report["initial_shards"], key=int)]
    after = [report["latest_shards"][key] for key in sorted(report["latest_shards"], key=int)]
    assert report["status"] == "PASS" and report["soak_seconds"] == (30 if clients == 10 else 120)
    assert report["elapsed_seconds"] - report["all_running_after_seconds"] >= report["soak_seconds"]
    assert validate_population(before, clients) and validate_population(after, clients)
    assert summarize_input(before, after, 20) == report["input"]
    assert summarize_wire(before, after) == report["wire"]
    assert summarize_producer_timing(before, after) == report["producer_timing"]
    assert report["input"]["all_peers_within_2_5_percent"]
    assert report["durable_rosters"]["human_seats"] == clients
    assert report["durable_rosters"]["running_workers"] == clients // 10
    assert report["durable_rosters"]["distinct_endpoints"] == clients // 10
    assert report["durable_rosters"]["server_bot_seats"] == 0
    assert all(peer["public"]["authority_resets"] == 0 for sample in after for peer in sample["peers"])
    assert report["server_sha256"] == server_hash and report["bots_sha256"] == bots_hash
    for name, digest in report["source_sha256"].items():
        assert hashlib.sha256((root / name).read_bytes()).hexdigest() == digest
    with tempfile.TemporaryDirectory() as temporary:
        metric_root = Path(temporary)
        for worker in metric_workers:
            directory = metric_root / "matches" / worker["allocation_id"]
            directory.mkdir(parents=True)
            text = "".join("LOAD_METRICS " + json.dumps(record, separators=(",", ":")) + "\n"
                           for record in worker["records"])
            assert "postgres://" not in text and "postgresql://" not in text
            (directory / "worker.log").write_text(text)
        computed = summarize_server_load(metric_root, clients)
    assert len(computed["workers"]) == len(metric_workers) == clients // 10
    expected_workers = {row["allocation_id"]: row for row in report["server_load"]["workers"]}
    archived_workers = {row["allocation_id"]: row for row in metric_workers}
    for row in computed["workers"]:
        expected = expected_workers[row["allocation_id"]]
        archived = archived_workers[row["allocation_id"]]
        assert row["metric_records"] == len(archived["records"]) == expected["metric_records"]
        assert len(archived["worker_log_sha256"]) == 64
        # The report hashes its observation-time log. The retained raw log can
        # gain ordinary lines before teardown; only metric lines are archived.
        row["worker_log_sha256"] = expected["worker_log_sha256"]
    assert computed == report["server_load"]
    totals = report["server_load"]["totals"]
    assert totals["sequence_gaps"] == totals["sequence_restarts"] == 0
    assert report["server_load"]["all_endpoints_without_sequence_gaps"]
    assert report["server_load"]["all_endpoints_without_sequence_restarts"]
    assert totals["gameplay_transforms"] > 0 and totals["snapshots"] > 0
    assert report["server_load"]["latest_transform_age_at_snapshot"]["count"] > 0
    assert report["server_load"]["broadcast_gaps"]["count"] == report["server_load"]["broadcasts"] - clients // 10
    assert observed["server_load"]["totals"] == totals
    workers = report["before_cleanup"]["workers"]
    assert len(workers) == clients // 10 and not report["before_cleanup"]["scan_limited"]
    for row in workers:
        health, status = row["health"], row["status"]
        assert status["phase"] == "running" and health["server_epoch"] == status["server_epoch"]
        assert health["version"] == 2 and health["sql_ready"] and health["journal_ready"]
        assert not health["reply_receiver_closed"] and health["blocked_since_ms"] is None
        assert all(health[key] == 0 for key in ("staged_dodges", "legacy_floors", "unpersisted_results", "critical_replies"))
fixture = data[".agent/codex-ranked-readiness/load-server-metrics/live-v2/isolated-summary.json"]
assert fixture["status"] == "PASS" and fixture["cleanup_complete"] and fixture["no_owned_process_groups_remain"]
assert not fixture["final_cleanup_inventory"]
assert fixture["career_schema"] == 13 and fixture["portal_schema"] == 11
assert all(value is False for value in fixture["runtime_role"].values())
assert "562 passed; 0 failed; 16 ignored" in data[".agent/codex-ranked-readiness/load-server-metrics/full-server-tests.log"]
assert "Ran 12 tests" in data[".agent/codex-ranked-readiness/load-server-metrics/python-tests.log"]
print("PASS: 10/30s and 1000/120s input, dispatch, transform-age, cadence, rosters, health and cleanup recomputed; WAN/RU/SLA remain open.")
