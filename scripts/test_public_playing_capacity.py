import copy
import contextlib
import io
import json
import os
from pathlib import Path
import tempfile
import time
import unittest
from unittest import mock

import public_playing_capacity as probe


def sample(shard=0, elapsed=0, increment=0):
    return {"phase": "running", "elapsed_secs": elapsed, "peers": [
        {"id": i + 1, "snapshot_age_ms": 20,
         "transport": {"received_datagrams": 100 + increment,
                       "received_bytes": 1000 + increment * 100,
                       "accepted_snapshots": 10 + increment,
                       "decode_errors": 0, "framing_errors": 0},
         "public": {"phase": "running", "profile_id": f"p{shard}-{i}", "error": None,
                    "allocation": {"allocation_id": f"{shard + 1:032x}",
                                   "endpoint": f"127.0.0.1:{47700 + shard}",
                                   "preference": "quick", "human_count": 10,
                                   "bot_count": 0, "rated": False}}}
        for i in range(10)]}


class PopulationEvidenceTests(unittest.TestCase):
    def test_cli_refuses_remote_large_unbounded_and_overlapping_fixtures_before_io(self):
        required = ["--server-bin", "/not-used", "--bots-bin", "/not-used",
                    "--output", "/not-created"]
        for uri, extra in [
            ("postgres://remote/omoba_test", []),
            ("postgres://localhost/omoba_test?host=remote", []),
            ("postgres://localhost/omoba_test", ["--clients", "1000"]),
            ("postgres://localhost/omoba_test", ["--clients", "1001", "--allow-large"]),
            ("postgres://localhost/omoba_test", ["--seconds", "14"]),
            ("postgres://localhost/omoba_test", ["--port", "47700", "--worker-port", "47700"]),
        ]:
            with self.subTest(uri=uri, extra=extra), \
                    mock.patch.dict(os.environ, {"OMOBA_TEST_DATABASE_URL": uri}), \
                    mock.patch.object(probe.subprocess, "run", side_effect=AssertionError("unexpected process")), \
                    mock.patch.object(probe.socket, "socket", side_effect=AssertionError("unexpected socket")), \
                    contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                probe.main(required + extra)

    def test_every_current_peer_and_its_wire_evidence_is_required(self):
        good = sample()
        self.assertTrue(probe.validate_sample(good))
        cases = [
            ("snapshot_age_ms", 2000), ("id", None),
            ("public.error", "Busy"), ("public.phase", "waiting"),
            ("public.allocation.bot_count", 1), ("public.allocation.rated", True),
            ("transport.received_bytes", None), ("transport.received_bytes", True),
            ("transport.accepted_snapshots", 0), ("transport.framing_errors", 1),
            ("transport.decode_errors", 1),
        ]
        for path, value in cases:
            with self.subTest(path=path, value=value):
                bad = copy.deepcopy(good)
                target = bad["peers"][0]
                parts = path.split(".")
                for part in parts[:-1]:
                    target = target[part]
                target[parts[-1]] = value
                self.assertFalse(probe.validate_sample(bad))
        good["peers"].pop()
        self.assertFalse(probe.validate_sample(good))

    def test_population_rejects_duplicate_identity_endpoints_and_missing_shards(self):
        good = [sample(0), sample(1)]
        self.assertTrue(probe.validate_population(good, 20))
        self.assertFalse(probe.validate_population(good[:1], 20))
        self.assertFalse(probe.validate_population([sample(0), sample(0)], 20))
        bad = copy.deepcopy(good)
        for p in bad[1]["peers"]:
            p["public"]["allocation"]["endpoint"] = "127.0.0.1:47700"
        self.assertFalse(probe.validate_population(bad, 20))
        bad = copy.deepcopy(good)
        bad[0]["peers"][0]["public"]["allocation"]["endpoint"] = "127.0.0.1:49999"
        self.assertFalse(probe.validate_population(bad, 20))

    def test_rates_require_monotonic_counters_and_fresh_native_time(self):
        before, after = [sample()], [sample(elapsed=10, increment=20)]
        result = probe.summarize_wire(before, after)
        self.assertEqual(result["bytes_per_second_per_player"]["p50"], 200)
        self.assertEqual(result["accepted_snapshots_per_second_per_player"]["p50"], 2)
        for bad in ([sample()], [sample(elapsed=10)], [sample(elapsed=10, increment=-1)]):
            with self.assertRaises(AssertionError):
                probe.summarize_wire(before, bad)

    def test_receipts_must_match_every_profile_session_endpoint_and_fresh_running_worker(self):
        current = sample()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            allocation = current["peers"][0]["public"]["allocation"]
            directory = root / "matches" / allocation["allocation_id"]
            directory.mkdir(parents=True)
            humans = [{"profile_id": p["public"]["profile_id"], "session_id": f"s{i}"}
                      for i, p in enumerate(current["peers"])]
            manifest = {"allocation_id": allocation["allocation_id"], "preference": "quick",
                        "endpoint": allocation["endpoint"], "humans": humans}
            status = {"allocation_id": allocation["allocation_id"], "phase": "running",
                      "server_epoch": 7, "heartbeat_ms": time.time() * 1000,
                      "joined_sessions": [h["session_id"] for h in humans]}
            def write(m, s):
                (directory / "manifest.json").write_text(json.dumps(m))
                (directory / "status.json").write_text(json.dumps(s))
            write(manifest, status)
            self.assertEqual(probe.verify_manifests(root, [current], 10)["human_seats"], 10)
            for key, value in [("phase", "settling"), ("heartbeat_ms", 0),
                               ("joined_sessions", [f"wrong{i}" for i in range(10)])]:
                bad = dict(status, **{key: value})
                write(manifest, bad)
                with self.assertRaises(AssertionError):
                    probe.verify_manifests(root, [current], 10)
            bad = copy.deepcopy(manifest)
            bad["humans"][0]["profile_id"] = "unknown"
            write(bad, status)
            with self.assertRaises(AssertionError):
                probe.verify_manifests(root, [current], 10)


if __name__ == "__main__":
    unittest.main()
