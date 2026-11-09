#!/usr/bin/env python3
"""Measure signed playing clients in an owned loopback Casual lobby.

Uses ten-client native AI shards, never a live server. Build current server and
harness --bin bots first and prepare the disposable omoba_test database. Large
runs require --allow-large and sufficient existing test DB connections; this
tool never changes database, operating-system or production configuration.
"""
import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import socket
import subprocess
import time

from public_bots_rehearsal import isolated_database, read_events, stop_owned


def valid_snapshot_meta(meta):
    return (isinstance(meta, dict)
            and all(type(meta.get(key)) is int and meta[key] > 0
                    for key in ("protocol_version", "server_epoch", "match_id", "snapshot_tick")))


def worker_diagnostics(root, limit):
    """Disposable pre-teardown observations, never match/health authority."""
    paths = sorted((root / "matches").glob("*/status.json"))
    paths += sorted((root / "matches/archive").glob("*/status.json"))
    rows = []
    for path in paths[:limit]:
        row = {"allocation_id": path.parent.name,
               "archived": path.parent.parent.name == "archive"}
        for name, fields in (("status.json", ("allocation_id", "server_epoch", "heartbeat_ms", "phase", "result_id")),
                             ("outbox/health/status.json", ("version", "updated_ms", "server_epoch", "probe_checked_ms", "sql_ready", "journal_ready", "staged_dodges", "legacy_floors", "unpersisted_results", "critical_replies", "reply_receiver_closed", "blocked_since_ms"))):
            try:
                value = json.loads((path.parent / name).read_text())
                row["status" if name == "status.json" else "health"] = {key: value.get(key) for key in fields}
            except (OSError, ValueError, AttributeError) as error:
                row["read_error_" + name] = type(error).__name__
        try:
            with (path.parent / "worker.log").open("rb") as log:
                log.seek(0, 2)
                row["log_bytes"] = log.tell()
                log.seek(max(0, log.tell() - 2048))
                tail = log.read().decode("utf-8", errors="replace")
            row["log_tail"] = re.sub(r"postgres(?:ql)?://[^\s\"<>]+", "[fixture-database-url]", tail)
        except OSError as error:
            row["log_read_error"] = type(error).__name__
        rows.append(row)
    return {"observed_unix_ms": int(time.time() * 1000), "scan_limited": len(paths) > limit,
            "workers": rows}


def validate_sample(sample, size=10):
    peers = sample.get("peers", [])
    if sample.get("phase") != "running" or len(peers) != size:
        return False
    profiles = set()
    for peer in peers:
        status, wire = peer.get("public") or {}, peer.get("transport") or {}
        allocation = status.get("allocation") or {}
        if (peer.get("id") is None or peer.get("snapshot_age_ms") is None
                or not 0 <= peer["snapshot_age_ms"] < 2000
                or not valid_snapshot_meta(peer.get("snapshot_meta"))
                or status.get("phase") != "running" or not status.get("profile_id")
                or status.get("error") is not None
                or allocation.get("human_count") != 10 or allocation.get("bot_count") != 0
                or allocation.get("rated") is not False
                or allocation.get("preference") != "quick"
                or not allocation.get("allocation_id")
                or not allocation.get("endpoint", "").startswith("127.0.0.1:")
                or any(type(wire.get(k)) is not int or wire[k] < 1
                       for k in ("received_datagrams", "received_bytes", "accepted_snapshots"))
                or wire.get("decode_errors") != 0 or wire.get("framing_errors") != 0):
            return False
        profiles.add(status["profile_id"])
    return len(profiles) == size


def validate_population(samples, count):
    """No historical success may substitute for a missing/current peer."""
    if len(samples) != count // 10 or not all(validate_sample(s) for s in samples):
        return False
    peers = [p for sample in samples for p in sample["peers"]]
    profiles = [p["public"]["profile_id"] for p in peers]
    allocations = {}
    for peer in peers:
        a = peer["public"]["allocation"]
        allocations.setdefault(a["allocation_id"], []).append(peer)
    return (len(set(profiles)) == count and len(allocations) == count // 10
            and all(len(peers) == 10 for peers in allocations.values())
            and all(len({p["public"]["allocation"]["endpoint"] for p in peers}) == 1
                    for peers in allocations.values())
            and len({peers[0]["public"]["allocation"]["endpoint"]
                     for peers in allocations.values()}) == count // 10)


def authenticated_population(samples, count):
    """All real sign-ins must finish before any controller presses Search."""
    if len(samples) != count // 10 or any(len(s.get("peers", [])) != 10 for s in samples):
        return False
    profiles = []
    for sample in samples:
        for peer in sample["peers"]:
            status = peer.get("public") or {}
            profile = status.get("profile_id")
            if (status.get("phase") != "authenticated" or status.get("error") is not None
                    or status.get("allocation") is not None or not isinstance(profile, str)
                    or type(status.get("auth_age_ms")) is not int
                    or not 0 <= status["auth_age_ms"] < 3000
                    or len(profile) != 64 or any(c not in "0123456789abcdef" for c in profile)):
                return False
            profiles.append(profile)
    return len(set(profiles)) == count


def percentile(values, fraction):
    values = sorted(values)
    return values[min(len(values) - 1, int(len(values) * fraction))]


def verify_manifests(root, samples, count):
    peers = [p for sample in samples for p in sample["peers"]]
    allocations = {p["public"]["allocation"]["allocation_id"] for p in peers}
    profiles = {p["public"]["profile_id"] for p in peers}
    manifest_profiles, endpoints, receipts = [], set(), []
    for allocation in allocations:
        assert len(allocation) == 32 and all(c in "0123456789abcdef" for c in allocation)
        directory = root / "matches" / allocation
        manifest = json.loads((directory / "manifest.json").read_text())
        status = json.loads((directory / "status.json").read_text())
        assert manifest["allocation_id"] == status["allocation_id"] == allocation
        assert manifest["preference"] == "quick" and len(manifest["humans"]) == 10
        assert status["phase"] == "running" and status["server_epoch"] != 0
        assert set(status["joined_sessions"]) == {h["session_id"] for h in manifest["humans"]}
        assert all(p["public"]["allocation"]["endpoint"] == manifest["endpoint"]
                   for p in peers if p["public"]["allocation"]["allocation_id"] == allocation)
        assert 0 <= time.time() * 1000 - status["heartbeat_ms"] < 5000, "stale worker receipt"
        manifest_profiles.extend(h["profile_id"] for h in manifest["humans"])
        endpoints.add(manifest["endpoint"])
        receipts.append({"allocation_id": allocation, "phase": status["phase"],
                         "server_epoch": status["server_epoch"],
                         "heartbeat_ms": status["heartbeat_ms"],
                         "joined_count": len(status["joined_sessions"]),
                         "profiles": [h["profile_id"] for h in manifest["humans"]],
                         "endpoint": manifest["endpoint"],
                         "manifest_sha256": hashlib.sha256((directory / "manifest.json").read_bytes()).hexdigest()})
    assert len(manifest_profiles) == count and set(manifest_profiles) == profiles
    assert len(endpoints) == count // 10
    return {"running_workers": len(allocations), "human_seats": count,
            "server_bot_seats": 0, "distinct_endpoints": len(endpoints),
            "receipts_at_acceptance": receipts}


def summarize_wire(before, after):
    # The legacy broadcast-tick field name is retained for archived verifiers.
    # Its sequence also advances for bootstrap views, even when no authenticated
    # player receives one. The accepted/sequence ratio is NOT packet delivery.
    assert before and len(before) == len(after), "missing native shard telemetry"
    rates, snapshots, tick_rates, delivery = [], [], [], []
    for initial, final in zip(before, after):
        seconds = final["elapsed_secs"] - initial["elapsed_secs"]
        assert seconds > 0, "no new native telemetry during soak"
        old = {p["public"]["profile_id"]: p for p in initial["peers"]}
        for peer in final["peers"]:
            previous = old[peer["public"]["profile_id"]]
            a, b = previous["transport"], peer["transport"]
            assert all(b[k] >= a[k] for k in a), "transport counters moved backwards"
            assert b["accepted_snapshots"] > a["accepted_snapshots"], "peer stopped receiving snapshots"
            rates.append((b["received_bytes"] - a["received_bytes"]) / seconds)
            snapshots.append((b["accepted_snapshots"] - a["accepted_snapshots"]) / seconds)
            first, last = previous.get("snapshot_meta"), peer.get("snapshot_meta")
            assert valid_snapshot_meta(first) and valid_snapshot_meta(last), "missing/invalid per-peer producer ticks"
            assert all(first[k] == last[k] for k in ("protocol_version", "server_epoch", "match_id")), "snapshot authority changed during soak"
            ticks = last["snapshot_tick"] - first["snapshot_tick"]
            accepted = b["accepted_snapshots"] - a["accepted_snapshots"]
            assert ticks >= accepted > 0, "producer ticks contradict accepted snapshots"
            tick_rates.append(ticks / seconds)
            delivery.append(accepted / ticks)
    return {"bytes_per_second_per_player": {
                "p50": percentile(rates, .5), "p95": percentile(rates, .95),
                "p99": percentile(rates, .99), "max": max(rates)},
            "accepted_snapshots_per_second_per_player": {
                "p50": percentile(snapshots, .5), "p05": percentile(snapshots, .05)},
            "observed_broadcast_tick_span_per_second_per_player": {
                "p50": percentile(tick_rates, .5), "p05": percentile(tick_rates, .05)},
            "accepted_fraction_of_observed_tick_span": {
                "p50": percentile(delivery, .5), "p05": percentile(delivery, .05)},
            "scope": "received wire including account traffic; ordered per-peer producer tick span, not independent send timestamps or latency; native driver drains all buffered snapshots"}


def summarize_input(before, after, requested_hz):
    """Measure successful signed movement sends, including idle/dead poses."""
    assert type(requested_hz) is int and 1 <= requested_hz <= 60, "invalid requested input profile"
    assert before and len(before) == len(after), "missing input shard telemetry"
    rates, cached = [], []
    for initial, final in zip(before, after):
        assert initial.get("input_hz_requested") == final.get("input_hz_requested") == requested_hz, "producer input mode mismatch"
        seconds = final["elapsed_secs"] - initial["elapsed_secs"]
        assert seconds > 0, "no fresh input measurement interval"
        previous = {p["public"]["profile_id"]: p for p in initial["peers"]}
        for peer in final["peers"]:
            old = previous[peer["public"]["profile_id"]]
            for item in (old, peer):
                status = item["public"]
                assert all(type(status.get(k)) is int and status[k] >= 0
                           for k in ("sent_transforms", "sent_gameplay_commands", "authority_resets")), "missing/invalid actual-send counters"
                assert type(item.get("cached_input_transforms")) is int and 0 <= item["cached_input_transforms"] <= status["sent_transforms"], "invalid cached-input count"
                assert status["sent_gameplay_commands"] >= status["sent_transforms"], "signed command counters contradict movement"
            a, b = old["public"], peer["public"]
            assert b["sent_gameplay_commands"] >= a["sent_gameplay_commands"] and b["sent_transforms"] > a["sent_transforms"], "input stopped/counters moved backwards"
            assert b["authority_resets"] == a["authority_resets"], "authority reset during input measurement"
            assert peer["cached_input_transforms"] >= old["cached_input_transforms"], "cached input counter moved backwards"
            rates.append((b["sent_transforms"] - a["sent_transforms"]) / seconds)
            cached.append(peer["cached_input_transforms"] - old["cached_input_transforms"])
    return {"requested_hz": requested_hz,
            "successful_signed_transforms_per_second": {"min": min(rates), "p05": percentile(rates, .05),
                "p50": percentile(rates, .5), "p95": percentile(rates, .95), "max": max(rates)},
            "cached_snapshot_transforms": {"total": sum(cached), "min_per_peer": min(cached)},
            "all_peers_within_2_5_percent": all(requested_hz * .975 <= rate <= requested_hz * 1.025 for rate in rates),
            "scope": "successful signed UDP sends at producer; no per-command server ACK, send-gap or input-to-snapshot latency qualification"}


def load_timing_summary(value, bounds):
    """Validate one cumulative server histogram and publish bucket-bounded quantiles."""
    zero = {"count": 0, "total_us": 0, "buckets": [0] * (len(bounds) + 1)}
    result = timing_delta(zero, value, bounds)
    result["mean_us"] = result["total_us"] / result["count"] if result["count"] else None
    for quantile in (95, 99):
        cumulative = 0
        result[f"p{quantile}_exclusive_upper_us"] = None
        for index, count in enumerate(result["buckets"]):
            cumulative += count
            if result["count"] and cumulative * 100 >= result["count"] * quantile:
                result[f"p{quantile}_exclusive_upper_us"] = bounds[index] if index < len(bounds) else None
                break
    return result


def merge_load_timings(values, bounds):
    merged = {"count": 0, "total_us": 0, "buckets": [0] * (len(bounds) + 1)}
    for value in values:
        checked = timing_delta({"count": 0, "total_us": 0, "buckets": [0] * (len(bounds) + 1)}, value, bounds)
        for counter in ("count", "total_us"):
            merged[counter] += checked[counter]
        merged["buckets"] = [a + b for a, b in zip(merged["buckets"], checked["buckets"])]
    return load_timing_summary(merged, bounds)


def read_load_metric_records(path):
    records = []
    with path.open(encoding="utf-8", errors="replace") as log:
        for line_number, line in enumerate(log, 1):
            if not line.startswith("LOAD_METRICS "):
                continue
            try:
                value = json.loads(line[len("LOAD_METRICS "):])
            except ValueError as error:
                raise AssertionError(f"malformed LOAD_METRICS at {path}:{line_number}") from error
            assert isinstance(value, dict), f"non-object LOAD_METRICS at {path}:{line_number}"
            records.append(value)
    assert records, f"no LOAD_METRICS records in {path}"
    return records


def summarize_server_load(root, clients):
    """Summarize final cumulative metrics from every live match worker.

    Sequence gaps expose signed commands that did not reach application dispatch.
    Latest-transform age is deliberately not presented as per-command latency.
    """
    assert type(clients) is int and clients >= 10 and clients % 10 == 0
    paths = sorted((root / "matches").glob("*/worker.log"))
    assert len(paths) == clients // 10, "missing or unexpected live worker metric logs"
    workers, all_endpoints = [], []
    common_bounds = None
    for path in paths:
        records = read_load_metric_records(path)
        previous_elapsed = -1.0
        for record in records:
            elapsed = record.get("elapsed_secs")
            assert type(elapsed) in (int, float) and elapsed > previous_elapsed, "non-monotonic server metric time"
            previous_elapsed = elapsed
        final = records[-1]
        bounds = final.get("bucket_upper_us")
        assert (isinstance(bounds, list) and bounds
                and all(type(n) is int and n > 0 for n in bounds)
                and bounds == sorted(set(bounds))), "invalid server timing bounds"
        if common_bounds is None:
            common_bounds = bounds
        assert bounds == common_bounds, "server timing profile changed between workers"
        assert type(final.get("broadcasts")) is int and final["broadcasts"] >= 1, "missing world broadcasts"
        endpoints = final.get("endpoints")
        assert isinstance(endpoints, list) and len(endpoints) == 10, "worker does not have ten measured endpoints"
        rows = []
        for endpoint in endpoints:
            assert isinstance(endpoint, dict), "invalid server endpoint report"
            for key in ("transport_commands", "gameplay_transforms", "last_sequence",
                        "sequence_gaps", "sequence_restarts", "snapshots"):
                assert type(endpoint.get(key)) is int and endpoint[key] >= 0, f"invalid {key}"
            assert type(endpoint.get("player_id")) is int and endpoint["player_id"] > 0, "missing gameplay player identity"
            assert endpoint["transport_commands"] >= endpoint["gameplay_transforms"] > 0, "command counters contradict transforms"
            assert endpoint["snapshots"] > 0, "endpoint stopped before final snapshot report"
            row = {key: endpoint[key] for key in ("player_id", "transport_commands", "gameplay_transforms",
                   "last_sequence", "sequence_gaps", "sequence_restarts", "snapshots")}
            row["latest_transform_age_at_snapshot"] = load_timing_summary(
                endpoint.get("latest_transform_age_at_snapshot"), bounds)
            assert row["latest_transform_age_at_snapshot"]["count"] > 0, "no transform-age observations"
            rows.append(row)
        assert len({row["player_id"] for row in rows}) == 10, "duplicate gameplay player identity"
        broadcast_gaps = load_timing_summary(final.get("broadcast_gaps"), bounds)
        worker = {
            "allocation_id": path.parent.name,
            "worker_log_sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            "metric_records": len(records),
            "elapsed_secs": final["elapsed_secs"],
            "broadcasts": final["broadcasts"],
            "broadcast_gaps": broadcast_gaps,
            "totals": {key: sum(row[key] for row in rows) for key in
                       ("transport_commands", "gameplay_transforms", "sequence_gaps", "sequence_restarts", "snapshots")},
            "latest_transform_age_at_snapshot": merge_load_timings(
                [row["latest_transform_age_at_snapshot"] for row in rows], bounds),
            "endpoints": rows,
        }
        workers.append(worker)
        all_endpoints.extend(rows)
    totals = {key: sum(row[key] for row in all_endpoints) for key in
              ("transport_commands", "gameplay_transforms", "sequence_gaps", "sequence_restarts", "snapshots")}
    return {
        "status": "measured",
        "workers": workers,
        "totals": totals,
        "all_endpoints_without_sequence_gaps": totals["sequence_gaps"] == 0,
        "all_endpoints_without_sequence_restarts": totals["sequence_restarts"] == 0,
        "bucket_upper_us": common_bounds,
        "broadcasts": sum(worker["broadcasts"] for worker in workers),
        "broadcast_gaps": merge_load_timings([worker["broadcast_gaps"] for worker in workers], common_bounds),
        "latest_transform_age_at_snapshot": merge_load_timings(
            [row["latest_transform_age_at_snapshot"] for row in all_endpoints], common_bounds),
        "scope": "final cumulative verified signed application dispatch and ordinary gameplay Transform handler entry; snapshot age of the latest transform after complete kernel sends; actual world-broadcast intervals; not per-command ACK latency, coordinate-change proof, packet delivery, WAN or SLA",
        "quantile_scope": "mean and exclusive histogram upper bounds only; overflow has no finite upper bound; cumulative process lifetime, not soak-only subtraction",
        "instrumentation_scope": "opt-in per-second JSON stderr reports add measurement overhead; this is an instrumented loopback run",
    }


def timing_delta(before, after, bounds):
    """Cumulative histogram difference: never let warm-up mask soak stalls."""
    result = {}
    for item in (before, after):
        assert all(type(item.get(k)) is int and item[k] >= 0 for k in ("count", "total_us")), "invalid timing counters"
        buckets = item.get("buckets")
        assert isinstance(buckets, list) and len(buckets) == len(bounds) + 1, "missing timing buckets"
        assert all(type(n) is int and n >= 0 for n in buckets), "invalid timing bucket"
        assert sum(buckets) == item["count"], "timing count contradicts buckets"
    for key in ("count", "total_us"):
        result[key] = after[key] - before[key]
        assert result[key] >= 0, "timing counters moved backwards"
    result["buckets"] = [b - a for a, b in zip(before["buckets"], after["buckets"])]
    assert all(n >= 0 for n in result["buckets"]), "timing buckets moved backwards"
    return result


def summarize_producer_timing(before, after):
    assert before and len(before) == len(after), "missing timing shard telemetry"
    if not all("producer_timing" in sample for sample in before + after):
        return {"status": "not measured by this producer"}
    bounds = before[0]["producer_timing"]["bucket_upper_us"]
    assert bounds and all(type(n) is int and n > 0 for n in bounds), "invalid timing bounds"
    assert bounds == sorted(set(bounds)), "timing bounds not strictly ordered"
    shard_keys = ["loop_work", "receive_work", "wake_lateness"]
    for key in ("session_work", "telemetry_prepare_work", "telemetry_format_write_work"):
        if all(key in s["producer_timing"] for s in before + after):
            shard_keys.append(key)
    navigation = all("navigation_work" in p for s in before + after for p in s["peers"])
    commands = all("command_work" in p["public"] for s in before + after for p in s["peers"])
    command_stages = ("prepare", "sign", "encode", "send") if commands else ()
    keys = shard_keys + ["transform_gaps"] + (["navigation_work"] if navigation else [])
    keys += ["command_" + stage for stage in command_stages]
    totals = {key: {"count": 0, "total_us": 0, "buckets": [0] * (len(bounds) + 1)} for key in keys}
    rebases = 0

    def add(key, initial, final):
        delta = timing_delta(initial, final, bounds)
        for counter in ("count", "total_us"):
            totals[key][counter] += delta[counter]
        totals[key]["buckets"] = [a + b for a, b in zip(totals[key]["buckets"], delta["buckets"])]
        return delta

    for initial, final in zip(before, after):
        a, b = initial["producer_timing"], final["producer_timing"]
        assert a["bucket_upper_us"] == b["bucket_upper_us"] == bounds, "timing profile changed"
        assert all(type(item.get("overrun_rebases")) is int and item["overrun_rebases"] >= 0
                   for item in (a, b)), "invalid rebase counter"
        assert b["overrun_rebases"] >= a["overrun_rebases"], "rebase counter moved backwards"
        rebases += b["overrun_rebases"] - a["overrun_rebases"]
        for key in shard_keys:
            add(key, a[key], b[key])
        old = {p["public"]["profile_id"]: p for p in initial["peers"]}
        for peer in final["peers"]:
            previous = old[peer["public"]["profile_id"]]
            if navigation:
                add("navigation_work", previous["navigation_work"], peer["navigation_work"])
            a, b = previous["public"], peer["public"]
            for stage in command_stages:
                delta = add("command_" + stage, a["command_work"][stage], b["command_work"][stage])
                assert delta["count"] == b["sent_gameplay_commands"] - a["sent_gameplay_commands"], "command timing contradicts signed sends"
            delta = add("transform_gaps", a["transform_gaps"], b["transform_gaps"])
            assert delta["count"] == b["sent_transforms"] - a["sent_transforms"], "send-gap count contradicts actual sends"
    for timing in totals.values():
        timing["mean_us"] = timing["total_us"] / timing["count"] if timing["count"] else None
        for quantile in (95, 99):
            cumulative = 0
            timing[f"p{quantile}_exclusive_upper_us"] = None
            for index, count in enumerate(timing["buckets"]):
                cumulative += count
                if timing["count"] and cumulative * 100 >= timing["count"] * quantile:
                    timing[f"p{quantile}_exclusive_upper_us"] = bounds[index] if index < len(bounds) else None
                    break
    return {"bucket_upper_us": bounds, **totals, "overrun_rebases": rebases,
            "scope": "soak-only cumulative histogram differences; wall time includes OS descheduling; quantiles are bucket bounds, not exact percentiles; overflow has no finite upper bound"}


def process_sample(lobby, shards):
    result = subprocess.run(["ps", "-axo", "pid=,ppid=,rss=,time="],
                            capture_output=True, text=True, check=True, timeout=5)
    rows = {}
    for line in result.stdout.splitlines():
        parts = line.split()
        if len(parts) != 4:
            continue
        pid, ppid, rss = map(int, parts[:3])
        clock = parts[3].split(":")
        seconds = sum(float(value) * (60 ** index)
                      for index, value in enumerate(reversed(clock)))
        rows[pid] = (ppid, rss / 1024, seconds)
    server_ids = {lobby.pid}
    while True:
        expanded = server_ids | {pid for pid, row in rows.items() if row[0] in server_ids}
        if expanded == server_ids:
            break
        server_ids = expanded
    def totals(ids):
        values = [rows[pid] for pid in ids if pid in rows]
        return {"processes": len(values), "rss_mib": sum(r[1] for r in values),
                "cpu_seconds": sum(r[2] for r in values)}
    return {"at": time.monotonic(), "server": totals(server_ids),
            "generator": totals({p.pid for p in shards})}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server-bin", type=Path, required=True)
    parser.add_argument("--bots-bin", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--clients", type=int, default=10)
    parser.add_argument("--seconds", type=float, default=15)
    parser.add_argument("--input-hz", type=int, default=20,
                        help="independent signed movement cadence1..60Hz;0 explicitly retains legacy snapshot-paced mode")
    parser.add_argument("--timeout", type=float, default=240)
    parser.add_argument("--allow-large", action="store_true")
    parser.add_argument("--server-load-metrics", action="store_true",
                        help="emit authoritative signed-dispatch, applied-transform and snapshot-send histograms")
    parser.add_argument("--port", type=int, default=47600)
    parser.add_argument("--worker-port", type=int, default=47700)
    args = parser.parse_args(argv)
    database = os.environ.get("OMOBA_TEST_DATABASE_URL", "")
    if not isolated_database(database):
        parser.error("set OMOBA_TEST_DATABASE_URL to a loopback /omoba_test URI without options")
    if (not 10 <= args.clients <= (1000 if args.allow_large else 100)
            or args.clients % 10 or not 15 <= args.seconds <= 300
            or not 0 <= args.input_hz <= 60
            or not args.seconds + 30 <= args.timeout <= 600):
        parser.error("require multiples of10 clients (large opt-in above100),15..300s soak,0..60 input Hz and bounded timeout")
    rooms = args.clients // 10
    if not (1024 <= args.port <= 65535 and 1024 <= args.worker_port <= 65536 - rooms
            and args.port not in range(args.worker_port, args.worker_port + rooms)):
        parser.error("invalid/overlapping loopback lobby and worker port range")
    for binary in (args.server_bin, args.bots_bin):
        if not binary.is_file() or not os.access(binary, os.X_OK):
            parser.error(f"not an executable: {binary}")
    for port in [args.port, *range(args.worker_port, args.worker_port + rooms)]:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            try:
                sock.bind(("127.0.0.1", port))
            except OSError as error:
                parser.error(f"loopback UDP port {port} is unavailable ({error.strerror}); no server was started")
    setting = subprocess.run(["psql", database, "--no-psqlrc", "-At", "-v", "ON_ERROR_STOP=1",
                              "-c", "SHOW max_connections"],
                             check=True, capture_output=True, text=True, timeout=10)
    if int(setting.stdout.strip()) < rooms + 8:
        parser.error("isolated test DB needs at least rooms+8 existing connections; no setting was changed")
    root = args.output.resolve()
    root.mkdir(parents=True, exist_ok=False)
    search_gate = root / "start-search"
    env = {k: v for k, v in os.environ.items()
           if not k.startswith(("OMOBA_", "GAME_")) and k != "SERVER_ADDR"}
    env.update(SERVER_ADDR=f"127.0.0.1:{args.port}", OMOBA_SERVER_ROLE="lobby",
               OMOBA_DATABASE_URL=database, OMOBA_MATCH_ROOT=str(root / "matches"),
               OMOBA_CAREER_OUTBOX=str(root / "outbox"), OMOBA_MATCH_CAPACITY=str(rooms),
               OMOBA_MATCH_FIRST_PORT=str(args.worker_port), OMOBA_MATCH_BIND_IP="127.0.0.1",
               OMOBA_MATCH_PUBLIC_HOST="127.0.0.1", OMOBA_LOBBY_MAX_WAITING="2048",
               OMOBA_LIMIT_ENDPOINTS_PER_IP="2048", OMOBA_LIMIT_ARENAS_PER_IP="100",
               OMOBA_LIMIT_PROBES_PER_IP="10000", OMOBA_LIMIT_PROBES_PER_NETWORK="10000",
               OMOBA_LIMIT_PROBES_GLOBAL="10000", OMOBA_LIMIT_NEW_PROFILES_PER_IP="1000000",
               OMOBA_LIMIT_NEW_PROFILES_TOTAL="10000000")
    if args.server_load_metrics:
        env["OMOBA_LOAD_METRICS"] = "1"
    report = {"status": "FAIL", "clients": args.clients, "arenas": rooms,
              "preference": "quick", "server_bots": 0, "soak_seconds": args.seconds,
              "scope": "local Casual signed native AI clients; not RU/cgroup or Ranked proof",
              "acceptance_scope": "Running population, freshness and durable rosters; received counters and opt-in server dispatch/cadence metrics",
              "input_hz_requested": args.input_hz or None,
              "unverified_scope": [
                  "Latency/SLA, physical devices, WAN and real RU host/uplink",
                  "Packet delivery/loss: snapshot sequence includes unsent bootstrap ticks; sequence gaps only expose signed commands absent from application dispatch",
                  "Per-command ACK latency and proof that a Transform changed coordinates",
              ],
              "fixture_allowances": {k: v for k, v in env.items() if k.startswith("OMOBA_LIMIT_")},
              "server_sha256": hashlib.sha256(args.server_bin.read_bytes()).hexdigest(),
              "bots_sha256": hashlib.sha256(args.bots_bin.read_bytes()).hexdigest(),
              "resources": []}
    source_root = Path(__file__).resolve().parents[1]
    revision = subprocess.run(["git", "rev-parse", "HEAD"], cwd=source_root,
                              capture_output=True, text=True, check=True, timeout=5)
    report["source_parent"] = revision.stdout.strip()
    changes = subprocess.run(["git", "status", "--porcelain", "--untracked-files=normal"],
                             cwd=source_root, capture_output=True, text=True,
                             check=True, timeout=5)
    report["source_dirty_paths"] = [line[3:] for line in changes.stdout.splitlines()]
    if not args.server_load_metrics:
        report["unverified_scope"].insert(0, "Server signed-dispatch acceptance, transform age at snapshots and world-broadcast cadence")
    report["source_sha256"] = {
        name: hashlib.sha256((source_root / name).read_bytes()).hexdigest()
        for name in ("scripts/public_playing_capacity.py", "scripts/public_bots_rehearsal.py",
                     "harness/src/bin/bots.rs", "harness/src/bot.rs", "harness/src/public_client.rs",
                     "server/src/main.rs", "server/src/load_metrics.rs", "server/src/public_transport.rs",
                     "server/src/runtime/dispatch.rs", "server/src/runtime/mod.rs", "server/src/snapshot.rs",
                     "Cargo.lock", "shared/src/lib.rs", "shared/src/transport.rs", "shared/src/protocol.rs")}
    children, writers, readers, shards, latest = [], [], [], [], {}
    auth_latest = {}
    def register(child):
        children.append(child)
        (root / "owned-processes.json").write_text(json.dumps([
            {"pid": p.pid, "role": "lobby" if i == 0 else "generator",
             "process_group": p.pid} for i, p in enumerate(children)
        ], indent=2) + "\n")
    started, baseline, running_at = time.monotonic(), None, None
    authenticated_since = None
    try:
        log = (root / "lobby.log").open("w"); writers.append(log)
        lobby = subprocess.Popen([str(args.server_bin.resolve())], env=env, cwd=root,
                                 stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        register(lobby)
        while "UDP game server is listening on" not in (root / "lobby.log").read_text():
            assert lobby.poll() is None, "owned lobby exited before listening"
            assert time.monotonic() - started < 30, "owned lobby startup timed out"
            time.sleep(.05)
        for index in range(rooms):
            log = (root / f"bots-{index:03d}.log").open("w"); writers.append(log)
            command = [str(args.bots_bin.resolve()), "--count", "10", "--server",
                                        f"127.0.0.1:{args.port}", "--public", "--telemetry",
                                        "--preference", "quick", "--search-gate", str(search_gate)]
            if args.input_hz:
                command += ["--input-hz", str(args.input_hz)]
            process = subprocess.Popen(command, env=env, cwd=root,
                                       stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            register(process); shards.append(process)
            readers.append((root / f"bots-{index:03d}.log").open())
        next_resources = 0
        while time.monotonic() - started < args.timeout:
            now = time.monotonic()
            exited = [{"pid": p.pid, "returncode": code,
                       "role": "lobby" if p is lobby else "generator",
                       "generator_index": None if p is lobby else shards.index(p)}
                      for p in children if (code := p.poll()) is not None]
            if exited:
                report["exited_before_cleanup"] = exited
                raise AssertionError("owned server/generator exited unexpectedly")
            for index, reader in enumerate(readers):
                for kind, sample in read_events(reader):
                    if kind == "BOT_SAMPLE":
                        latest[index] = (now, sample)
                    elif kind == "BOT_AUTH":
                        auth_latest[index] = (now, sample)
            current = [latest[i][1] for i in range(rooms) if i in latest]
            fresh = len(latest) == rooms and all(now - at < 3 for at, _ in latest.values())
            if not search_gate.exists():
                current_auth = [auth_latest[i][1] for i in range(rooms) if i in auth_latest]
                auth_fresh = len(auth_latest) == rooms and all(now - at < 3 for at, _ in auth_latest.values())
                if auth_fresh and authenticated_population(current_auth, args.clients):
                    if authenticated_since is None:
                        authenticated_since = now
                        report["all_authenticated_after_seconds"] = now - started
                    if now - authenticated_since >= 5:
                        search_gate.touch(exist_ok=False)
                        report["search_released_after_seconds"] = now - started
                        report["login_fixture"] = "ordinary sign-in; all profiles stable5s before pressing Search; no qualification or policy override"
                else:
                    authenticated_since = None
            valid = fresh and validate_population(current, args.clients)
            if running_at is None and valid:
                if args.input_hz:
                    assert all(s.get("input_hz_requested") == args.input_hz for s in current), "native producer did not confirm requested input cadence"
                running_at, baseline = now, current
                report["all_running_after_seconds"] = now - started
                report["initial_shards"] = {str(i): sample for i, sample in enumerate(current)}
            if running_at is not None:
                assert valid, "population lost Running/freshness/identity/roster/error acceptance during soak"
                if now >= next_resources:
                    report["resources"].append(process_sample(lobby, shards))
                    next_resources = now + 2
                if now - running_at >= args.seconds:
                    report["durable_rosters"] = verify_manifests(root, current, args.clients)
                    report["wire"] = summarize_wire(baseline, current)
                    if args.input_hz:
                        report["input"] = summarize_input(baseline, current, args.input_hz)
                        report["producer_timing"] = summarize_producer_timing(baseline, current)
                        assert report["input"]["all_peers_within_2_5_percent"], "actual per-peer input cadence outside requested profile"
                    else:
                        report["input"] = {"status": "not measured; explicit legacy snapshot-paced mode"}
                    if args.server_load_metrics:
                        report["server_load"] = summarize_server_load(root, args.clients)
                        assert report["server_load"]["all_endpoints_without_sequence_gaps"], "signed command sequence gaps reached final report"
                        assert report["server_load"]["all_endpoints_without_sequence_restarts"], "signed command sequence restarted during run"
                    else:
                        report["server_load"] = {"status": "not requested"}
                    report["status"] = "PASS"
                    break
            time.sleep(.1)
        assert report["status"] == "PASS", "not every playing peer reached acceptance before timeout"
    except Exception as error:
        report["error"] = f"{type(error).__name__}: {error}"
        raise
    finally:
        report["elapsed_seconds"] = time.monotonic() - started
        report["before_cleanup"] = worker_diagnostics(root, rooms * 4)
        report["latest_shards"] = {str(i): sample for i, (_, sample) in latest.items()}
        report["latest_auth_shards"] = {str(i): sample for i, (_, sample) in auth_latest.items()}
        (root / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        stop_owned(children)
        for file in readers + writers:
            file.close()
    print(json.dumps({k: v for k, v in report.items() if k not in ("latest_shards", "resources")}))


if __name__ == "__main__":
    main()
