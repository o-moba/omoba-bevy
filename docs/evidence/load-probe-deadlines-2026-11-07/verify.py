"""Check archived probe data, without executing a new live rehearsal."""
from pathlib import Path
import gzip
import hashlib
import json
import sys

here = Path(__file__).resolve().parent
root = here.parents[2]
sys.path.insert(0, str(root / "scripts"))
from public_playing_capacity import summarize_wire, validate_population  # noqa: E402

summary = json.loads((here / "summary.json").read_text())
data = {}
for name, record in summary["files"].items():
    compressed = (here / name).read_bytes()
    assert hashlib.sha256(compressed).hexdigest() == record["gzip_sha256"]
    raw = gzip.decompress(compressed)
    assert len(raw) == record["raw_bytes"]
    assert hashlib.sha256(raw).hexdigest() == record["raw_sha256"]
    data[record["source_path"]] = json.loads(raw) if name.endswith(".json.gz") else raw.decode()

for stage in ("load-deadline", "load-career-order"):
    control = data[stage + "/instrumented-10/report.json"]
    assert control["status"] == "PASS" and control["clients"] == 10
    initial = list(control["initial_shards"].values())
    latest = list(control["latest_shards"].values())
    assert validate_population(initial, 10) and validate_population(latest, 10)
    assert summarize_wire(initial, latest) == control["wire"]
    failed = data[stage + "/instrumented-1000/report.json"]
    assert failed["status"] == "FAIL" and failed["clients"] == 1000
    assert failed.get("all_running_after_seconds") is None and failed.get("wire") is None
    assert failed["exited_before_cleanup"]
    assert all(row["role"] == "generator" and row["returncode"] == 101
               for row in failed["exited_before_cleanup"])
    assert any(row.get("status", {}).get("phase") == "failed"
               for row in failed["before_cleanup"]["workers"])
    assert failed["server_sha256"] == control["server_sha256"] == summary["production_server_sha256"]
    cleanup = data[stage + "/controller-cleanup-audit.json"]
    assert cleanup["no_owned_process_groups_remain"] and cleanup["owned_database_removed"]

last = data["load-career-order/instrumented-1000/report.json"]
peers = [peer for shard in last["latest_shards"].values() for peer in shard["peers"]]
assert sum(p["public"]["stale_career_packets"] for p in peers) == 0
assert sum(p["public"]["authority_resets"] for p in peers) == 209
build = data["load-cadence/build.json"]
assert build["bots_sha256"] == last["bots_sha256"]
assert "2 failed" in data["load-cadence/deadline-before.log"]
assert "1 failed" in data["load-cadence/career-order-before.log"]
assert "35 passed; 0 failed" in data["load-cadence/harness-unit-tests.log"]
assert "3 passed; 0 failed" in data["load-cadence/ordered-transport-tests.log"]
print("Archived integrity/control metrics PASS; both1000-client rehearsals remain FAIL.")
