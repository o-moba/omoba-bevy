"""Verify archived observations; this does not execute a new live load test."""
from pathlib import Path
import gzip
import hashlib
import json
import sys

here = Path(__file__).resolve().parent
root = here.parents[2]
sys.path.insert(0, str(root / "scripts"))
from public_playing_capacity import summarize_wire  # noqa: E402

summary = json.loads((here / "summary.json").read_text())
payloads = {}
for name, record in summary["files"].items():
    compressed = (here / name).read_bytes()
    assert hashlib.sha256(compressed).hexdigest() == record["gzip_sha256"], name
    raw = gzip.decompress(compressed)
    assert len(raw) == record["raw_bytes"], name
    assert hashlib.sha256(raw).hexdigest() == record["raw_sha256"], name
    if record["raw_path"].endswith(".json"):
        payloads[record["raw_path"]] = json.loads(raw)

control = payloads["instrumented-10/report.json"]
baseline = payloads["instrumented-10-baseline.json"]
assert control["status"] == "PASS" and control["clients"] == 10
assert summarize_wire([baseline], [control["latest_shards"]["0"]]) == control["wire"]
failed = payloads["instrumented-1000/report.json"]
assert failed["status"] == "FAIL" and failed["clients"] == 1000
assert failed.get("all_running_after_seconds") is None and failed.get("wire") is None
assert "exited unexpectedly" in failed["error"]
assert failed["server_sha256"] == control["server_sha256"] == summary["production_binary_sha256"]
fixture = payloads["isolated-summary.json"]
assert fixture["status"] == "FAIL" and fixture["cleanup_complete"] is True
assert fixture["career_schema"] == 13 and fixture["portal_schema"] == 11
assert fixture["max_connections"] == 160
assert all(value is False for value in fixture["runtime_role"].values())
print("Archived evidence integrity PASS; 10-client control PASS; 1000-client acceptance FAIL.")
