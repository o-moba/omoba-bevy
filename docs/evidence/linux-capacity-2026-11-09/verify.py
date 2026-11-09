"""Verify archived Linux capacity evidence; starts no server and runs no load."""
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


with tempfile.TemporaryDirectory(prefix="omoba-linux-capacity-verify-") as temporary:
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

    source_hashes = None
    with tarfile.open(fileobj=io.BytesIO(raw["native-source.tar.gz"]), mode="r:gz") as archive:
        source_hashes = {
            member.name: hashlib.sha256(archive.extractfile(member).read()).hexdigest()
            for member in archive.getmembers()
            if member.isfile()
        }
    assert hashlib.sha256(raw["native-source.tar.gz"]).hexdigest() == manifest["native_source_tar_gz_sha256"]

    for label, expected in manifest["cases"].items():
        report = json.loads(raw[f"{label}/report.json"])
        wrapper = json.loads(raw[f"{label}-summary.json"])
        clients = expected["clients"]
        assert report["clients"] == wrapper["clients"] == clients
        assert report["arenas"] == clients // 10 and report["server_bots"] == 0
        assert report["soak_seconds"] == wrapper["seconds"] == expected["seconds"]
        assert report["status"] == wrapper["status"] == expected["status"]
        assert report.get("error") == expected["error"]
        assert report["source_parent"] == manifest["source_parent"]
        assert report["source_sha256"] == source_hashes
        assert report["source_dirty_paths"] == ["remote-fixture-not-a-git-checkout"]
        assert report["server_sha256"] == manifest["server_cgroup_wrapper_sha256"]
        assert report["bots_sha256"] == wrapper["bots_sha256"] == manifest["bots_sha256"]
        assert wrapper["server_sha256"] == manifest["server_sha256"]
        assert wrapper["career_schema"] == 13 and wrapper["portal_schema"] == 11
        assert wrapper["owned_processes_remain"] == [] and wrapper["database_absent"] is True
        before = [report["initial_shards"][key] for key in sorted(report["initial_shards"], key=int)]
        after = [report["latest_shards"][key] for key in sorted(report["latest_shards"], key=int)]
        assert validate_population(before, clients) and validate_population(after, clients)
        assert summarize_input(before, after, 20) == report["input"] == expected["input"]
        assert summarize_wire(before, after) == report["wire"] == expected["wire"]
        assert summarize_producer_timing(before, after) == report["producer_timing"]
        measured = summarize_server_load(restored / label, clients)
        assert compact_load(measured) == expected["server_load"]
        assert resource_summary(report["resources"]) == expected["resources"]
        if expected["status"] == "PASS":
            assert measured == report["server_load"]
            assert compact_load(measured) == wrapper["server_load"]
            assert report["input"]["all_peers_within_2_5_percent"] is True
            assert report["durable_rosters"]["human_seats"] == clients
            assert report["durable_rosters"]["running_workers"] == clients // 10
            assert report["durable_rosters"]["distinct_endpoints"] == clients // 10
            assert report["durable_rosters"]["server_bot_seats"] == 0
            assert measured["totals"]["sequence_gaps"] == 0
            assert measured["totals"]["sequence_restarts"] == 0
        else:
            assert label == "native-capacity-8-120"
            assert report["input"]["all_peers_within_2_5_percent"] is False
            assert measured["totals"]["sequence_gaps"] == 7
            assert measured["totals"]["sequence_restarts"] == 0
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

    for label, expected in manifest["node_ab"].items():
        report = json.loads(raw[f"{label}/report.json"])
        wrapper = json.loads(raw[f"{label}-summary.json"])
        quick_report = report["per_arena_type"]["quick"]
        quick_wrapper = wrapper["per_arena_type"]["quick"]
        assert report["clients"] == wrapper["clients"] == expected["clients"] == 40
        assert report["soak_seconds"] == wrapper["seconds"] == expected["seconds"] == 180
        assert quick_report["snapshot_gap_ms"] == quick_wrapper["snapshot_gap_ms"] == expected["snapshot_gap_ms"]
        assert quick_report["snapshot_gap_ms"]["p95"] > 100 and expected["capacity_gate_pass"] is False
        assert quick_report["snapshot_bytes_per_second_per_player"] == expected["snapshot_bytes_per_second_per_player"]
        assert wrapper["soak_resources"] == expected["soak_resources"]
        assert wrapper["owned_processes_remain"] == [] and wrapper["database_absent"] is True
        if expected["server_metrics"]:
            assert compact_load(summarize_server_load(restored / label, 40)) == expected["server_load"]
        else:
            assert wrapper["server_load_metrics"] is False

    host = json.loads(raw["final-host.json"])
    assert host["logical_cpus"] == 2 and host["mem_total_kib"] == 3480344
    assert host["owned_capacity_processes"] == []
    assert host["disposable_omoba_test_database_count"] == 0
    assert host["dev_services"] == {"omoba-lobby-dev": "active", "omoba-account-api-dev": "active"}

print("Native Linux PASS retained at 10/30s, 40/120s and 60/120s; 80/120s FAIL retained with 7 sequence gaps.")
print("Node A/B client-gap gate rejected; native boundary is dev-VPS-only. Long soak, RU host/uplink and WAN remain open.")
