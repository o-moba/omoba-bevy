"""Verify archived Docker capacity evidence with only the Python standard library."""
from pathlib import Path
import gzip
import hashlib
import io
import json
import sys
import tarfile
import tempfile

here = Path(__file__).resolve().parent
manifest = json.loads((here / "summary.json").read_text())
assert {p.name for p in here.iterdir()} >= {"README.md", "summary.json", "verify.py"}
archived = {
    path.relative_to(here).as_posix() for path in here.rglob("*")
    if path.is_file() and path.name not in {"README.md", "summary.json", "verify.py"}
}
assert archived == set(manifest["files"]), "unlisted or missing evidence file"


def key_values(text):
    return {key: int(value) for key, value in (line.split() for line in text.splitlines())}


def cgroup_summary(samples, limits):
    result = {
        "limits": limits,
        "sample_count": len(samples),
        "memory_current_max": max((s["memory_current"] or 0) for s in samples),
        "memory_peak_final": samples[-1]["memory_peak"],
        "pids_current_max": max((s["pids_current"] or 0) for s in samples),
        "pids_peak_final": samples[-1]["pids_peak"],
        "memory_events_first": samples[0]["memory_events"],
        "memory_events_final": samples[-1]["memory_events"],
        "network_first": samples[0]["network"],
        "network_final": samples[-1]["network"],
        "cpu_interval_percent_mean": None,
        "cpu_interval_percent_max": None,
        "nr_throttled_final": samples[-1]["nr_throttled"],
        "throttled_usec_final": samples[-1]["throttled_usec"],
    }
    intervals = []
    for first, second in zip(samples, samples[1:]):
        elapsed = second["monotonic"] - first["monotonic"]
        used = second["cpu_usage_usec"] - first["cpu_usage_usec"]
        if elapsed > 0 and used >= 0:
            intervals.append(used / 10000 / elapsed)
    if intervals:
        result["cpu_interval_percent_mean"] = sum(intervals) / len(intervals)
        result["cpu_interval_percent_max"] = max(intervals)
    return result


with tempfile.TemporaryDirectory(prefix="omoba-docker-capacity-verify-") as temporary:
    restored = Path(temporary)
    raw = {}
    for archive_name, record in manifest["files"].items():
        compressed = (here / archive_name).read_bytes()
        assert hashlib.sha256(compressed).hexdigest() == record["archive_sha256"], archive_name
        assert record["encoding"] in ("gzip", "stored")
        data = gzip.decompress(compressed) if record["encoding"] == "gzip" else compressed
        assert len(data) == record["raw_bytes"], archive_name
        assert hashlib.sha256(data).hexdigest() == record["raw_sha256"], archive_name
        destination = restored / record["source_path"]
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(data)
        raw[record["source_path"]] = data

    with tarfile.open(fileobj=io.BytesIO(raw["native-source.tar.gz"]), mode="r:gz") as archive:
        source_hashes = {}
        for member in archive.getmembers():
            if not member.isfile():
                continue
            data = archive.extractfile(member).read()
            source_hashes[member.name] = hashlib.sha256(data).hexdigest()
            destination = restored / member.name
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(data)
    assert source_hashes == manifest["source_sha256"]
    assert len(source_hashes) == 43
    assert not any(Path(name).name.startswith("._") for name in source_hashes)

    sys.path.insert(0, str(restored / "scripts"))
    from public_playing_capacity import (
        allocation_preferences, summarize_input, summarize_producer_timing,
        summarize_server_load, summarize_wire, validate_population,
    )

    config = json.loads(raw["fixture.json"])
    artifacts = json.loads(raw["artifact-hashes.json"])
    assert config["source_parent"] == manifest["source_parent"]
    assert config["release_run"] == manifest["release_run"]
    assert config["source_sha256"] == source_hashes
    assert artifacts == {key: manifest[key] for key in ("server_sha256", "bots_sha256", "helper_sha256")}
    wrapper_sha256 = hashlib.sha256(raw["server-native-scope"]).hexdigest()

    labels = set(manifest["summary_labels"])
    assert labels == {path.name.removesuffix("-summary.json") for path in restored.glob("*-summary.json")}
    for label in sorted(labels):
        wrapper = json.loads(raw[f"{label}-summary.json"])
        samples = json.loads(raw[f"{label}-cgroup-samples.json"])
        assert wrapper["source_parent"] == manifest["source_parent"]
        assert wrapper["release_run"] == manifest["release_run"]
        assert wrapper["server_sha256"] == manifest["server_sha256"]
        assert wrapper["bots_sha256"] == manifest["bots_sha256"]
        assert wrapper["helper_sha256"] == manifest["helper_sha256"]
        assert wrapper["career_schema"] == 13 and wrapper["portal_schema"] == 11
        assert wrapper["cgroup"] == cgroup_summary(samples, wrapper["cgroup"]["limits"])
        assert wrapper["cgroup"]["limits"] == {"cpu.max": "400000 100000", "memory.max": "4294967296", "pids.max": "512"}
        assert wrapper["owned_processes_remain"] == []
        assert wrapper["database_absent"] is True and wrapper["roles_absent"] is True
        for event in ("oom", "oom_kill", "oom_group_kill"):
            assert samples[-1]["memory_events"][event] == samples[0]["memory_events"][event]

    for label in manifest["full_cases"]:
        report = json.loads(raw[f"{label}/report.json"])
        wrapper = json.loads(raw[f"{label}-summary.json"])
        clients = report["clients"]
        expected_status = "FAIL" if label == "casual-800-120-v2" else "PASS"
        assert report["status"] == wrapper["status"] == expected_status
        assert report["source_parent"] == wrapper["source_parent"] == manifest["source_parent"]
        assert report["source_sha256"] == config["report_source_sha256"]
        assert set(report["source_sha256"].items()) <= set(source_hashes.items())
        assert report["source_dirty_paths"] == ["local-docker-fixture-not-a-git-checkout"]
        assert report["server_sha256"] == wrapper_sha256
        assert report["bots_sha256"] == wrapper["bots_sha256"] == manifest["bots_sha256"]
        assert wrapper["server_sha256"] == manifest["server_sha256"]
        assert wrapper["server_load_metrics"] is True
        assert report["soak_seconds"] == wrapper["seconds"]
        preferences = report["shard_preferences"]
        before = [report["initial_shards"][key] for key in sorted(report["initial_shards"], key=int)]
        after = [report["latest_shards"][key] for key in sorted(report["latest_shards"], key=int)]
        assert validate_population(before, clients, preferences)
        assert validate_population(after, clients, preferences)
        measured_input = summarize_input(before, after, 20)
        assert measured_input == report["input"] == wrapper["input"]
        assert measured_input["all_peers_within_2_5_percent"] is (expected_status == "PASS")
        assert summarize_wire(before, after) == report["wire"] == wrapper["wire"]
        assert summarize_producer_timing(before, after) == report["producer_timing"] == wrapper["producer_timing"]
        assert report["durable_rosters"]["human_seats"] == clients
        assert report["durable_rosters"]["running_workers"] == report["arenas"]
        assert report["durable_rosters"]["distinct_endpoints"] == report["arenas"]
        workers = report["before_cleanup"]["workers"]
        assert not report["before_cleanup"]["scan_limited"] and len(workers) == report["arenas"]
        for row in workers:
            health, status = row["health"], row["status"]
            assert status["phase"] == "running" and health["server_epoch"] == status["server_epoch"]
            assert health["version"] == 2 and health["sql_ready"] and health["journal_ready"]
            assert not health["reply_receiver_closed"] and health["blocked_since_ms"] is None
            assert all(health[key] == 0 for key in ("staged_dodges", "legacy_floors", "unpersisted_results", "critical_replies"))
        if expected_status == "PASS":
            # The runner reads metrics and then hashes each live log. A worker
            # can add another metric between those operations and before exit.
            for recorded_worker in report["server_load"]["workers"]:
                log_path = restored / label / "matches" / recorded_worker["allocation_id"] / "worker.log"
                log_data = log_path.read_bytes()
                prefix_length = 0
                prefix_found = False
                for line in log_data.splitlines(keepends=True):
                    prefix_length += len(line)
                    if hashlib.sha256(log_data[:prefix_length]).hexdigest() == recorded_worker["worker_log_sha256"]:
                        prefix_found = True
                        break
                assert prefix_found, "recorded pre-cleanup worker-log hash not found"
                teardown_lines = log_data[prefix_length:].splitlines()
                assert teardown_lines and teardown_lines[-1] == b"SHUTDOWN_METRIC terminal_outbox=durable"
                metric_lines = [line for line in log_data[:prefix_length].splitlines(keepends=True)
                                if line.startswith(b"LOAD_METRICS ")]
                assert len(metric_lines) >= recorded_worker["metric_records"]
                log_path.write_bytes(b"".join(metric_lines[:recorded_worker["metric_records"]]))
        measured_load = summarize_server_load(restored / label, clients, allocation_preferences(after))
        compact_load = {key: value for key, value in measured_load.items() if key != "workers"}
        if expected_status == "PASS":
            assert compact_load == {key: value for key, value in report["server_load"].items() if key != "workers"}
            assert compact_load == wrapper["server_load"]
            assert len(measured_load["workers"]) == len(report["server_load"]["workers"])
            for measured_worker, recorded_worker in zip(measured_load["workers"], report["server_load"]["workers"]):
                assert measured_worker["allocation_id"] == recorded_worker["allocation_id"]
                assert {key: value for key, value in measured_worker.items() if key != "worker_log_sha256"} == {key: value for key, value in recorded_worker.items() if key != "worker_log_sha256"}
        else:
            assert report["error"] == wrapper["error"] == "AssertionError: actual per-peer input cadence outside requested profile"
            assert "server_load" not in report and "server_load" not in wrapper
        expected_gaps = 0 if expected_status == "PASS" else 707
        assert measured_load["totals"]["sequence_gaps"] == expected_gaps
        assert measured_load["totals"]["sequence_restarts"] == 0
        print(f"{label}: {expected_status}; input min={measured_input['successful_signed_transforms_per_second']['min']:.6f}/s; gaps/restarts={expected_gaps}/0")

    early = json.loads(raw["casual-500-300/report.json"])
    assert early["status"] == "FAIL"
    assert early["error"] == "AssertionError: population lost Running/freshness/identity/roster/error acceptance during soak"
    finished = [row for row in early["before_cleanup"]["workers"] if (row.get("status") or {}).get("phase") == "finished"]
    assert len(finished) == 1 and "event=victory" in finished[0]["log_tail"]
    early_v2 = json.loads(raw["casual-500-300-v2/report.json"])
    assert early_v2["status"] == "FAIL" and early_v2["error"] == early["error"]
    assert "MATCH_METRIC event=victory" in raw["casual-500-300-v2-winning-worker.log"].decode()
    assert "duration_ms=130148" in raw["casual-500-300-v2-winning-worker.log"].decode()

    transient = json.loads(raw["casual-800-120/report.json"])
    allocation = "d88b535478871366935afe9607512ecd"
    assert "FileNotFoundError" in transient["error"] and allocation in transient["error"]
    observed = next(row for row in transient["before_cleanup"]["workers"] if row["allocation_id"] == allocation)
    assert observed["status"]["phase"] == "running"

    ports = json.loads(raw["casual-240-120/report.json"])
    assert ports["status"] == "FAIL" and "AddrInUse" in json.dumps(ports["before_cleanup"])
    missing_source = json.loads(raw["casual-10-control-summary.json"])
    assert missing_source["status"] == "FAIL" and "harness/src/bin/bots.rs" in missing_source["error"]

print(f"Verified {len(manifest['files'])} payload files and {len(manifest['summary_labels'])} cgroup summaries.")
print("Docker ARM64 loopback only: RU x86_64 host, uplink, WAN and production SLA remain unverified.")
