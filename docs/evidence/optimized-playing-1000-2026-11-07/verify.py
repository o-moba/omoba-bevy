"""Verify archived local population evidence; no new live game is executed."""
from pathlib import Path
import gzip
import hashlib
import json
import sys

here = Path(__file__).resolve().parent
sys.path.insert(0, str(here.parents[2] / "scripts"))
from public_playing_capacity import summarize_wire, validate_population  # noqa: E402

summary = json.loads((here / "summary.json").read_text())
data = {}
for name, record in summary["files"].items():
    compressed = (here / name).read_bytes()
    assert hashlib.sha256(compressed).hexdigest() == record["gzip_sha256"], name
    raw = gzip.decompress(compressed)
    assert len(raw) == record["raw_bytes"]
    assert hashlib.sha256(raw).hexdigest() == record["raw_sha256"]
    data[record["source_path"]] = json.loads(raw) if name.endswith(".json.gz") else raw.decode()

build = data["build.json"]
assert build["status"] == "PASS" and len(build["commands"]) == 8
for command in build["commands"]:
    assert command["exit"] == 0
    assert "opt-level=1" in command["command"]
    assert "debug-assertions=yes" in command["command"]
    assert "overflow-checks=yes" in command["command"]
assert "3 passed; 0 failed" in data["transport-tests.log"]
for count in (10, 1000):
    report = data[f"live-v1/instrumented-{count}/report.json"]
    assert report["status"] == "PASS" and report["clients"] == count
    def ordered(key):
        return [report[key][index] for index in sorted(report[key], key=int)]
    before, after = ordered("initial_shards"), ordered("latest_shards")
    assert validate_population(before, count) and validate_population(after, count)
    assert summarize_wire(before, after) == report["wire"]
    assert report["durable_rosters"]["running_workers"] == count // 10
    assert report["durable_rosters"]["human_seats"] == count
    assert report["durable_rosters"]["server_bot_seats"] == 0
    for peer in [p for shard in after for p in shard["peers"]]:
        assert peer["public"]["authority_resets"] == 0
        assert peer["public"]["stale_career_packets"] == 0
    assert report["server_sha256"] == next(c["output_sha256"] for c in build["commands"] if c["name"] == "server")
    assert report["bots_sha256"] == next(c["output_sha256"] for c in build["commands"] if c["name"] == "bots")
large = data["live-v1/instrumented-1000/report.json"]
assert large["soak_seconds"] == 120
assert large["elapsed_seconds"] - large["all_running_after_seconds"] >= 120
rows = large["before_cleanup"]["workers"]
assert len(rows) == 100 and not large["before_cleanup"]["scan_limited"]
for row in rows:
    health, status = row["health"], row["status"]
    assert status["phase"] == "running" and health["server_epoch"] == status["server_epoch"]
    assert health["version"] == 2 and health["sql_ready"] and health["journal_ready"]
    assert not health["reply_receiver_closed"] and health["blocked_since_ms"] is None
    assert all(health[key] == 0 for key in ("staged_dodges", "legacy_floors", "unpersisted_results", "critical_replies"))
fixture = data["live-v1/isolated-summary.json"]
assert fixture["status"] == "PASS" and fixture["cleanup_complete"]
assert fixture["no_owned_process_groups_remain"] and not fixture["final_cleanup_inventory"]
assert fixture["career_schema"] == 13 and fixture["portal_schema"] == 11
assert all(value is False for value in fixture["runtime_role"].values())
print("Archive/control/1000-client120s population and cleanup PASS;20Hz input/SLA/RU remain unverified.")
