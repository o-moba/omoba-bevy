#!/usr/bin/env python3
"""Standalone recomputation for the exact-Release Linux workload packet."""
import sys
sys.dont_write_bytecode = True

import gzip
import hashlib
import io
import json
from pathlib import Path
import tarfile

ROOT = Path(__file__).resolve().parent
RULES = {
    "quick": {"humans": 10, "bots": 0, "rated": False},
    "humans_only": {"humans": 10, "bots": 0, "rated": True},
    "bot_practice": {"humans": 1, "bots": 9, "rated": False},
}
RUNS = {
    "bots-10": ("bots", ["bot_practice"], 10, 10, 90),
    "ranked-10": ("ranked", ["humans_only"], 10, 1, 0),
    "mixed-30": ("mixed", ["quick", "humans_only", "bot_practice"], 30, 12, 90),
}
SOURCE_PARENT = "73651bc6281aa5eac03d0094b3061f6cee1cadf8"
COUNTERS = ("transport_commands", "gameplay_transforms", "sequence_gaps", "sequence_restarts", "snapshots")


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def payload(path):
    data = path.read_bytes()
    return gzip.decompress(data) if path.suffix == ".gz" else data


def json_file(path):
    return json.loads(payload(path))


def metric_records(path):
    records = []
    for line in payload(path).decode("utf-8").splitlines():
        if line.startswith("LOAD_METRICS "):
            value = json.loads(line[len("LOAD_METRICS "):])
            assert isinstance(value, dict)
            records.append(value)
    assert records
    elapsed = [record["elapsed_secs"] for record in records]
    assert all(type(value) in (int, float) for value in elapsed)
    assert elapsed == sorted(elapsed) and len(elapsed) == len(set(elapsed))
    return records


def verify_run(name, expected, summary):
    workload, preferences, clients, arenas, server_bots = expected
    root = ROOT / name
    report = json_file(root / "report.json.gz")
    wrapper = json_file(root / "wrapper-summary.json.gz")
    binaries = summary["binary_sha256"]
    assert report["status"] == wrapper["status"] == "PASS"
    assert report["workload"] == wrapper["workload"] == workload
    assert report["clients"] == wrapper["clients"] == clients
    assert report["generator_shards"] == clients // 10
    assert report["arenas"] == wrapper["arenas"] == arenas
    assert report["server_bots"] == server_bots
    assert report["shard_preferences"] == preferences and report["soak_seconds"] == wrapper["seconds"] == 30
    assert report["source_parent"] == wrapper["source_parent"] == SOURCE_PARENT
    assert report["source_dirty_paths"] == ["remote-fixture-not-a-git-checkout"]
    assert report["server_sha256"] == sha256(ROOT / "server-native-scope") == summary["server_cgroup_wrapper_sha256"]
    assert wrapper["server_sha256"] == binaries["server_sha256"]
    assert report["bots_sha256"] == wrapper["bots_sha256"] == binaries["bots_sha256"]
    assert wrapper["helper_sha256"] == binaries["helper_sha256"]
    assert report["release_run"] if "release_run" in report else wrapper["release_run"] == summary["release_run"]
    for source, expected_hash in report["source_sha256"].items():
        archived = ROOT / "sources" / source
        assert archived.is_file() and sha256(archived) == expected_hash, source
    assert report["input"]["all_peers_within_2_5_percent"] is True
    rates = report["input"]["successful_signed_transforms_per_second"]
    assert 19.5 <= rates["min"] <= rates["max"] <= 20.5
    assert report["wire"]["accepted_snapshots_per_second_per_player"]["p05"] > 0
    qualification = report["ranked_qualification"]
    if "humans_only" in preferences:
        assert qualification["required"] is True and qualification["status"] == "seeded"
        assert qualification["target"] == 10
        assert qualification["profile_count"] == 10 * preferences.count("humans_only")
        assert qualification["helper_sha256"] == binaries["helper_sha256"]
        assert qualification["result"]["real_games_played"] == 0
        assert qualification["result"]["receipts_created"] == 100 * preferences.count("humans_only")
    else:
        assert qualification == {"required": False, "status": "not used", "shards": [], "helper_sha256": None}

    rosters = report["durable_rosters"]
    assert rosters["running_workers"] == rosters["distinct_endpoints"] == arenas
    assert rosters["human_seats"] == clients and rosters["server_bot_seats"] == server_bots
    receipts = rosters["receipts_at_acceptance"]
    assert len(receipts) == arenas
    expected_preference_counts = {preference: preferences.count(preference) * (10 if preference == "bot_practice" else 1)
                                  for preference in set(preferences)}
    actual_preference_counts = {preference: sum(receipt["preference"] == preference for receipt in receipts)
                                for preference in set(preferences)}
    assert actual_preference_counts == expected_preference_counts
    receipt_by_allocation = {receipt["allocation_id"]: receipt for receipt in receipts}
    assert len(receipt_by_allocation) == arenas
    assert {path.name for path in (root / "matches").iterdir() if path.is_dir()} == set(receipt_by_allocation)

    server_load = report["server_load"]
    worker_by_allocation = {worker["allocation_id"]: worker for worker in server_load["workers"]}
    assert set(worker_by_allocation) == set(receipt_by_allocation)
    before_cleanup = {worker["allocation_id"]: worker for worker in report["before_cleanup"]["workers"]}
    assert set(before_cleanup) == set(receipt_by_allocation)
    aggregate = {key: 0 for key in COUNTERS}
    for allocation, receipt in receipt_by_allocation.items():
        preference = receipt["preference"]
        rules = RULES[preference]
        directory = root / "matches" / allocation
        manifest = json_file(directory / "manifest.json.gz")
        status = json_file(directory / "status.json.gz")
        health = json_file(directory / "outbox/health/status.json.gz")
        assert manifest["allocation_id"] == status["allocation_id"] == allocation
        assert manifest["preference"] == receipt["preference"] == preference
        assert len(manifest["humans"]) == receipt["human_count"] == rules["humans"]
        assert receipt["bot_count"] == rules["bots"] and receipt["rated"] is rules["rated"]
        assert receipt["profiles"] == [human["profile_id"] for human in manifest["humans"]]
        assert receipt["phase"] == before_cleanup[allocation]["status"]["phase"] == "running"
        # Archived files were copied after teardown and may have advanced to
        # settling/failed; only the pre-teardown Running receipt is acceptance.
        assert status["phase"] in ("running", "settling", "failed")
        assert status["server_epoch"] == receipt["server_epoch"] != 0
        assert status["heartbeat_ms"] >= receipt["heartbeat_ms"]
        assert set(status["joined_sessions"]) == {human["session_id"] for human in manifest["humans"]}
        assert receipt["joined_count"] == rules["humans"]
        assert receipt["endpoint"] == manifest["endpoint"]
        assert receipt["manifest_sha256"] == hashlib.sha256(payload(directory / "manifest.json.gz")).hexdigest()
        assert health["version"] == 2 and health["sql_ready"] is True and health["journal_ready"] is True
        assert health["server_epoch"] == status["server_epoch"]

        worker = worker_by_allocation[allocation]
        assert worker["preference"] == preference
        records = metric_records(directory / "worker.log.gz")
        assert len(records) >= worker["metric_records"]
        endpoints = records[worker["metric_records"] - 1]["endpoints"]
        assert len(endpoints) == rules["humans"]
        assert len({endpoint["player_id"] for endpoint in endpoints}) == rules["humans"]
        for endpoint in endpoints:
            assert endpoint["transport_commands"] >= endpoint["gameplay_transforms"] > 0
            assert endpoint["snapshots"] > 0
            assert endpoint["latest_transform_age_at_snapshot"]["count"] > 0
        totals = {key: sum(endpoint[key] for endpoint in endpoints) for key in COUNTERS}
        assert totals == worker["totals"]
        for key in COUNTERS:
            aggregate[key] += totals[key]
    assert aggregate == server_load["totals"]
    assert aggregate["sequence_gaps"] == aggregate["sequence_restarts"] == 0
    assert server_load["all_endpoints_without_sequence_gaps"] is True
    assert server_load["all_endpoints_without_sequence_restarts"] is True
    cleanup = json_file(root / "cleanup.json.gz")
    owned = json_file(root / "owned-processes.json.gz")
    assert cleanup["owned_process_count"] == len(owned) == clients // 10 + 1
    assert cleanup["alive_owned_processes"] == []
    assert cleanup["database_absent"] is True and cleanup["roles_absent"] is True
    case = summary["cases"][name]
    for key in ("workload", "clients", "generator_shards", "arenas", "server_bots", "seconds", "status", "input", "wire", "ranked_qualification"):
        assert case[key] == report[{"seconds": "soak_seconds"}.get(key, key)]
    assert case["server_load"] == {key: value for key, value in server_load.items() if key != "workers"}
    return {"run": name, "status": "PASS", "clients": clients, "arenas": arenas,
            "workers": arenas, "sequence_gaps": 0, "sequence_restarts": 0,
            "gameplay_transforms": aggregate["gameplay_transforms"],
            "input_min": rates["min"], "input_max": rates["max"]}


def main():
    manifest = json.loads((ROOT / "manifest.json").read_text())
    for relative, expected_hash in manifest.items():
        assert sha256(ROOT / relative) == expected_hash, relative
    actual = {str(path.relative_to(ROOT)) for path in ROOT.rglob("*")
              if path.is_file() and path.name not in ("manifest.json", "README.md", "verify.py")}
    assert actual == set(manifest)
    summary = json.loads((ROOT / "summary.json").read_text())
    config = json.loads((ROOT / "fixture.json").read_text())
    assert summary["source_parent"] == config["source_parent"] == SOURCE_PARENT
    assert summary["release_run"] == config["release_run"] == 42657406745
    assert set(summary["release_platforms"].values()) == {"success"}
    with tarfile.open(ROOT / "native-source.tar.gz", "r:gz") as archive:
        sources = {member.name: hashlib.sha256(archive.extractfile(member).read()).hexdigest()
                   for member in archive.getmembers() if member.isfile()}
    assert sources == config["source_sha256"]
    results = [verify_run(name, expected, summary) for name, expected in RUNS.items()]
    for report_name in RUNS:
        report = json_file(ROOT / report_name / "report.json.gz")
        assert report["source_sha256"] == config["report_source_sha256"]
        wrapper = json_file(ROOT / report_name / "wrapper-summary.json.gz")
        assert wrapper["migration_sha256"] == {name: config["source_sha256"][name] for name in wrapper["migration_sha256"]}
    archived_sources = {str(path.relative_to(ROOT / "sources")) for path in (ROOT / "sources").rglob("*") if path.is_file()}
    assert archived_sources == set(config["report_source_sha256"]) and len(archived_sources) == 19
    host = json.loads((ROOT / "final-host.json").read_text())
    assert host["owned_capacity_processes"] == []
    assert host["disposable_omoba_test_database_count"] == host["disposable_capacity_role_count"] == 0
    assert set(host["dev_services"].values()) == {"active"}
    print(json.dumps({"status": "PASS", "source_files": len(archived_sources),
                      "release_run": summary["release_run"], "runs": results}, indent=2))


if __name__ == "__main__":
    main()
