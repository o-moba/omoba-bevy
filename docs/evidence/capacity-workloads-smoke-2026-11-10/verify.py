#!/usr/bin/env python3
"""Standalone recomputation for the selected native workload smoke packet."""
import sys
sys.dont_write_bytecode = True

import hashlib
import json
from pathlib import Path

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
SOURCE_PARENT = "36acb77129011a3799867f9755ca9684f153bc7e"
SERVER_SHA256 = "f8c6daad7fbd556f1b64769ce99240e127033c9c9b254345d6576ad2b747e2d8"
BOTS_SHA256 = "5ee8d341de8c0d4e2b3f063d561f14408c33cf5623f66d73a2cb68a8dafc3234"
COUNTERS = ("transport_commands", "gameplay_transforms", "sequence_gaps", "sequence_restarts", "snapshots")


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def metric_records(path):
    records = []
    for line in path.read_text(encoding="utf-8", errors="strict").splitlines():
        if line.startswith("LOAD_METRICS "):
            value = json.loads(line[len("LOAD_METRICS "):])
            assert isinstance(value, dict)
            records.append(value)
    assert records
    elapsed = [record["elapsed_secs"] for record in records]
    assert all(type(value) in (int, float) for value in elapsed)
    assert elapsed == sorted(elapsed) and len(elapsed) == len(set(elapsed))
    return records


def verify_run(name, expected):
    workload, preferences, clients, arenas, server_bots = expected
    root = ROOT / name
    report = json.loads((root / "report.json").read_text())
    assert report["status"] == "PASS" and report["workload"] == workload
    assert report["clients"] == clients and report["generator_shards"] == clients // 10
    assert report["arenas"] == arenas and report["server_bots"] == server_bots
    assert report["shard_preferences"] == preferences and report["soak_seconds"] == 15
    assert report["source_parent"] == SOURCE_PARENT
    assert report["server_sha256"] == SERVER_SHA256 and report["bots_sha256"] == BOTS_SHA256
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
        assert qualification["target"] == 10 and qualification["profile_count"] == 10 * preferences.count("humans_only")
        assert qualification["helper_sha256"]
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
        manifest = json.loads((directory / "manifest.json").read_text())
        status = json.loads((directory / "status.json").read_text())
        health = json.loads((directory / "outbox/health/status.json").read_text())
        assert manifest["allocation_id"] == status["allocation_id"] == allocation
        assert manifest["preference"] == receipt["preference"] == preference
        assert len(manifest["humans"]) == receipt["human_count"] == rules["humans"]
        assert receipt["bot_count"] == rules["bots"] and receipt["rated"] is rules["rated"]
        assert receipt["profiles"] == [human["profile_id"] for human in manifest["humans"]]
        assert receipt["phase"] == before_cleanup[allocation]["status"]["phase"] == "running"
        # The selected on-disk status was copied after owned teardown and may
        # therefore say failed; the report's pre-teardown receipt is the gate.
        assert status["phase"] in ("running", "failed")
        assert status["server_epoch"] == receipt["server_epoch"] != 0
        assert status["heartbeat_ms"] >= receipt["heartbeat_ms"]
        assert set(status["joined_sessions"]) == {human["session_id"] for human in manifest["humans"]}
        assert receipt["joined_count"] == rules["humans"]
        assert receipt["endpoint"] == manifest["endpoint"]
        assert receipt["manifest_sha256"] == sha256(directory / "manifest.json")
        assert health["version"] == 2 and health["sql_ready"] is True and health["journal_ready"] is True
        assert health["server_epoch"] == status["server_epoch"]

        worker = worker_by_allocation[allocation]
        assert worker["preference"] == preference
        log = directory / "worker.log"
        records = metric_records(log)
        # Logs were selected after teardown and can contain later lines. The
        # acceptance summary used the first metric_records observations.
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
    cleanup = json.loads((root / "cleanup.json").read_text())
    owned = json.loads((root / "owned-processes.json").read_text())
    assert cleanup["owned_process_count"] == len(owned) == clients // 10 + 1
    assert cleanup["alive_owned_processes"] == []
    return {"run": name, "status": "PASS", "clients": clients, "arenas": arenas,
            "workers": arenas, "sequence_gaps": 0, "sequence_restarts": 0,
            "input_min": rates["min"], "input_max": rates["max"]}


def main():
    results = [verify_run(name, expected) for name, expected in RUNS.items()]
    for report_name in RUNS:
        report = json.loads((ROOT / report_name / "report.json").read_text())
        assert set(report["source_sha256"]) == set(json.loads((ROOT / "bots-10/report.json").read_text())["source_sha256"])
    archived_sources = {str(path.relative_to(ROOT / "sources")) for path in (ROOT / "sources").rglob("*") if path.is_file()}
    expected_sources = set(json.loads((ROOT / "bots-10/report.json").read_text())["source_sha256"])
    assert archived_sources == expected_sources and len(archived_sources) == 19
    print(json.dumps({"status": "PASS", "source_files": len(archived_sources), "runs": results}, indent=2))


if __name__ == "__main__":
    main()
