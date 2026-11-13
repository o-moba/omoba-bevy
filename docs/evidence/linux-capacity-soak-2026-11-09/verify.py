"""Verify archived five-minute Linux soak evidence; starts no server or load."""
from pathlib import Path
import gzip
import hashlib
import io
import json
import statistics
import sys
import tarfile
import tempfile

here = Path(__file__).resolve().parent
sys.path.insert(0, str(here.parents[2] / "scripts"))
from public_playing_capacity import (
    read_load_metric_records,
    summarize_input,
    summarize_producer_timing,
    summarize_server_load,
    summarize_wire,
    validate_population,
)

manifest = json.loads((here / "summary.json").read_text())
assert {p.name for p in here.iterdir()} >= {"README.md", "summary.json", "verify.py"}
archived = {
    path.relative_to(here).as_posix()
    for path in here.rglob("*")
    if path.is_file() and path.name not in {"README.md", "summary.json", "verify.py"}
}
assert archived == set(manifest["files"]), "unlisted or missing evidence file"


def compact_load(value):
    return {key: item for key, item in value.items() if key != "workers"}


def resource_summary(rows):
    result = {"samples": len(rows), "observed_seconds": rows[-1]["at"] - rows[0]["at"]}
    for role in ("server", "generator"):
        rates = [
            (b[role]["cpu_seconds"] - a[role]["cpu_seconds"]) / (b["at"] - a["at"]) * 100
            for a, b in zip(rows, rows[1:])
        ]
        result[role] = {
            "cpu_percent_mean_of_intervals": statistics.mean(rates),
            "rss_mib_peak": max(row[role]["rss_mib"] for row in rows),
            "processes_peak": max(row[role]["processes"] for row in rows),
        }
    return result


with tempfile.TemporaryDirectory(prefix="omoba-linux-soak-verify-") as temporary:
    restored = Path(temporary)
    raw = {}
    for archive, record in manifest["files"].items():
        compressed = (here / archive).read_bytes()
        assert hashlib.sha256(compressed).hexdigest() == record["archive_sha256"], archive
        data = gzip.decompress(compressed) if record["encoding"] == "gzip" else compressed
        assert record["encoding"] in ("gzip", "stored")
        assert len(data) == record["raw_bytes"], archive
        assert hashlib.sha256(data).hexdigest() == record["raw_sha256"], archive
        destination = restored / record["source_path"]
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(data)
        raw[record["source_path"]] = data

    with tarfile.open(fileobj=io.BytesIO(raw["native-source.tar.gz"]), mode="r:gz") as archive:
        source_hashes = {
            member.name: hashlib.sha256(archive.extractfile(member).read()).hexdigest()
            for member in archive.getmembers()
            if member.isfile()
        }
    assert len(source_hashes) == 15
    assert not any(Path(name).name.startswith("._") for name in source_hashes)
    assert hashlib.sha256(raw["native-source.tar.gz"]).hexdigest() == manifest["native_source_tar_gz_sha256"]

    for label, expected in manifest["cases"].items():
        report = json.loads(raw[f"{label}/report.json"])
        wrapper = json.loads(raw[f"{label}-summary.json"])
        clients = expected["clients"]
        assert report["clients"] == wrapper["clients"] == clients == 60
        assert report["arenas"] == 6 and report["server_bots"] == 0
        assert report["soak_seconds"] == wrapper["seconds"] == expected["seconds"] == 300
        assert report["status"] == wrapper["status"] == expected["status"]
        assert report.get("error") == wrapper.get("error") == expected["error"]
        assert report["source_parent"] == manifest["source_parent"]
        assert report["source_sha256"] == source_hashes
        assert report["source_dirty_paths"] == ["remote-fixture-not-a-git-checkout"]
        assert report["server_sha256"] == manifest["server_cgroup_wrapper_sha256"]
        assert report["bots_sha256"] == wrapper["bots_sha256"] == manifest["bots_sha256"]
        assert wrapper["server_sha256"] == manifest["server_sha256"]
        assert wrapper["server_load_metrics"] is expected["server_metrics"]
        assert wrapper["career_schema"] == 13 and wrapper["portal_schema"] == 11
        assert wrapper["owned_processes_remain"] == [] and wrapper["database_absent"] is True
        before = [report["initial_shards"][key] for key in sorted(report["initial_shards"], key=int)]
        after = [report["latest_shards"][key] for key in sorted(report["latest_shards"], key=int)]
        assert validate_population(before, clients) and validate_population(after, clients)
        assert summarize_input(before, after, 20) == report["input"] == expected["input"]
        assert summarize_wire(before, after) == report["wire"] == expected["wire"]
        assert summarize_producer_timing(before, after) == report["producer_timing"] == expected["producer_timing"]
        assert resource_summary(report["resources"]) == expected["resources"]
        assert report["input"]["all_peers_within_2_5_percent"] is (expected["status"] == "PASS")
        assert report["durable_rosters"]["human_seats"] == clients
        assert report["durable_rosters"]["running_workers"] == clients // 10
        assert report["durable_rosters"]["distinct_endpoints"] == clients // 10
        assert report["durable_rosters"]["server_bot_seats"] == 0
        workers = report["before_cleanup"]["workers"]
        assert not report["before_cleanup"]["scan_limited"] and len(workers) == clients // 10
        for row in workers:
            health, status = row["health"], row["status"]
            assert status["phase"] == "running" and health["server_epoch"] == status["server_epoch"]
            assert health["version"] == 2 and health["sql_ready"] and health["journal_ready"]
            assert not health["reply_receiver_closed"] and health["blocked_since_ms"] is None
            assert all(health[key] == 0 for key in (
                "staged_dodges", "legacy_floors", "unpersisted_results", "critical_replies"
            ))
        worker_paths = sorted((restored / label / "matches").glob("*/worker.log"))
        assert len(worker_paths) == 6
        if expected["server_metrics"]:
            measured = summarize_server_load(restored / label, clients)
            if expected["status"] == "PASS":
                assert measured == report["server_load"]
                assert compact_load(measured) == wrapper["server_load"]
            else:
                assert expected["error"] == "AssertionError: actual per-peer input cadence outside requested profile"
                assert "server_load" not in report and "server_load" not in wrapper
            assert compact_load(measured) == expected["server_load"]
            assert measured["totals"]["sequence_gaps"] == 0
            assert measured["totals"]["sequence_restarts"] == 0
        else:
            if expected["status"] == "PASS":
                assert report["server_load"] == wrapper["server_load"] == {"status": "not requested"}
            else:
                assert "server_load" not in report and "server_load" not in wrapper
            assert expected["server_load"] is None
            assert "Server signed-dispatch acceptance, transform age at snapshots and world-broadcast cadence" in report["unverified_scope"]
            for path in worker_paths:
                try:
                    read_load_metric_records(path)
                except AssertionError:
                    pass
                else:
                    raise AssertionError(f"unexpected server metrics in {path}")

    host = json.loads(raw["final-host-soak.json"])
    assert host["logical_cpus"] == 2 and host["mem_total_kib"] == 3480344
    assert host["owned_capacity_processes"] == []
    assert host["disposable_omoba_test_database_count"] == 0
    assert host["dev_services"] == {"omoba-lobby-dev": "active", "omoba-account-api-dev": "active"}

for label, expected in manifest["cases"].items():
    print(f"{label}: {expected['status']}; input min={expected['input']['successful_signed_transforms_per_second']['min']:.6f}/s")
print("Instrumented signed sequence gaps/restarts: 0/0. RU host/uplink, WAN and production SLA remain unverified.")
