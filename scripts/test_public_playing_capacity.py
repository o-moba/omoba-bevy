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
         "snapshot_meta": {"protocol_version": 16, "server_epoch": shard + 1,
                           "match_id": 1, "snapshot_tick": 10 + increment},
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
    def test_input_qualification_uses_actual_sends_instead_of_received_snapshots(self):
        before, after = [sample()], [sample(elapsed=10, increment=20)]
        for samples, count, cached in ((before, 10, 0), (after, 210, 180)):
            samples[0]["input_hz_requested"] = 20
            for peer in samples[0]["peers"]:
                peer["public"].update(sent_transforms=count, sent_gameplay_commands=count + 5,
                                      authority_resets=0)
                peer["cached_input_transforms"] = cached
        result = probe.summarize_input(before, after, 20)
        self.assertTrue(result["all_peers_within_2_5_percent"])
        self.assertEqual(result["successful_signed_transforms_per_second"]["min"], 20)
        self.assertEqual(result["cached_snapshot_transforms"]["total"], 1800)
        self.assertEqual(probe.summarize_wire(before, after)["accepted_snapshots_per_second_per_player"]["p50"], 2)
        for count in (110, 310):
            bad = copy.deepcopy(after)
            for peer in bad[0]["peers"]:
                peer["public"].update(sent_transforms=count, sent_gameplay_commands=count + 5)
                peer["cached_input_transforms"] = 0
            self.assertFalse(probe.summarize_input(before, bad, 20)["all_peers_within_2_5_percent"])
        for key, value in (("sent_transforms", None), ("sent_transforms", True),
                           ("sent_transforms", 9), ("sent_gameplay_commands", 1),
                           ("authority_resets", 1)):
            bad = copy.deepcopy(after)
            bad[0]["peers"][0]["public"][key] = value
            with self.subTest(key=key, value=value), self.assertRaises(AssertionError):
                probe.summarize_input(before, bad, 20)
        bad = copy.deepcopy(after); bad[0]["input_hz_requested"] = None
        with self.assertRaises(AssertionError):
            probe.summarize_input(before, bad, 20)

    def test_pre_cleanup_diagnostics_include_archived_workers_without_auth_fields(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for parent in (root / "matches/active", root / "matches/archive/retired"):
                (parent / "outbox/health").mkdir(parents=True)
                (parent / "status.json").write_text(json.dumps({
                    "phase": "failed", "server_epoch": 7, "auth_nonce": "secret-nonce",
                    "released_sessions": ["private-session"],
                }))
                (parent / "outbox/health/status.json").write_text(json.dumps({
                    "version": 2, "sql_ready": True, "password": "secret-password",
                }))
                (parent / "worker.log").write_text("x" * 3000 + " postgres://test:fixture@localhost/omoba_test\nend")
            result = probe.worker_diagnostics(root, 4)
            self.assertFalse(result["scan_limited"])
            self.assertEqual(len(result["workers"]), 2)
            self.assertEqual(sum(row["archived"] for row in result["workers"]), 1)
            published = json.dumps(result)
            for private in ("secret-nonce", "private-session", "secret-password", "postgres://"):
                self.assertNotIn(private, published)
            self.assertTrue(all(len(row["log_tail"]) <= 2048 for row in result["workers"]))
            self.assertTrue(probe.worker_diagnostics(root, 1)["scan_limited"])

    def test_search_release_requires_every_distinct_authenticated_unallocated_profile(self):
        samples = [sample(0), sample(1)]
        for index, peer in enumerate(p for s in samples for p in s["peers"]):
            peer["public"].update(phase="authenticated", profile_id=f"{index:064x}", allocation=None,
                                  auth_age_ms=200)
        self.assertTrue(probe.authenticated_population(samples, 20))
        self.assertFalse(probe.authenticated_population(samples[:1], 20))
        self.assertFalse(probe.authenticated_population(samples + samples, 40))
        for key, value in [("phase", "authenticating"), ("error", "Busy"),
                           ("profile_id", None), ("allocation", {"allocation_id": "unexpected"}),
                           ("auth_age_ms", None), ("auth_age_ms", True), ("auth_age_ms", 3000)]:
            bad = copy.deepcopy(samples)
            bad[0]["peers"][0]["public"][key] = value
            self.assertFalse(probe.authenticated_population(bad, 20))

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
            ("snapshot_meta", None), ("snapshot_meta.snapshot_tick", 0),
            ("snapshot_meta.server_epoch", True),
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
        self.assertEqual(result["observed_broadcast_tick_span_per_second_per_player"]["p50"], 2)
        self.assertEqual(result["accepted_fraction_of_observed_tick_span"]["p05"], 1)
        for bad in ([sample()], [sample(elapsed=10)], [sample(elapsed=10, increment=-1)]):
            with self.assertRaises(AssertionError):
                probe.summarize_wire(before, bad)

    def test_producer_ticks_expose_missing_snapshots_and_reject_mixed_authority(self):
        before, after = [sample()], [sample(elapsed=10, increment=20)]
        for peer in after[0]["peers"]:
            peer["snapshot_meta"]["snapshot_tick"] += 20
        result = probe.summarize_wire(before, after)
        self.assertEqual(result["accepted_fraction_of_observed_tick_span"]["p50"], .5)
        self.assertEqual(result["observed_broadcast_tick_span_per_second_per_player"]["p50"], 4)
        for field, value in (("server_epoch", 2), ("match_id", 2),
                             ("protocol_version", 17), ("snapshot_tick", 29),
                             ("snapshot_tick", True), ("snapshot_tick", None)):
            bad = copy.deepcopy(after)
            bad[0]["peers"][0]["snapshot_meta"][field] = value
            with self.subTest(field=field, value=value), self.assertRaises(AssertionError):
                probe.summarize_wire(before, bad)
        bad = copy.deepcopy(after)
        del bad[0]["peers"][0]["snapshot_meta"]
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
