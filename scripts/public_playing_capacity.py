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
from pathlib import Path
import socket
import subprocess
import time

from public_bots_rehearsal import isolated_database, read_events, stop_owned


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
    assert before and len(before) == len(after), "missing native shard telemetry"
    rates, snapshots = [], []
    for initial, final in zip(before, after):
        seconds = final["elapsed_secs"] - initial["elapsed_secs"]
        assert seconds > 0, "no new native telemetry during soak"
        old = {p["public"]["profile_id"]: p["transport"] for p in initial["peers"]}
        for peer in final["peers"]:
            a, b = old[peer["public"]["profile_id"]], peer["transport"]
            assert all(b[k] >= a[k] for k in a), "transport counters moved backwards"
            assert b["accepted_snapshots"] > a["accepted_snapshots"], "peer stopped receiving snapshots"
            rates.append((b["received_bytes"] - a["received_bytes"]) / seconds)
            snapshots.append((b["accepted_snapshots"] - a["accepted_snapshots"]) / seconds)
    return {"bytes_per_second_per_player": {
                "p50": percentile(rates, .5), "p95": percentile(rates, .95),
                "p99": percentile(rates, .99), "max": max(rates)},
            "accepted_snapshots_per_second_per_player": {
                "p50": percentile(snapshots, .5), "p05": percentile(snapshots, .05)},
            "scope": "received wire including account traffic; native driver drains to freshest snapshot"}


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
    parser.add_argument("--timeout", type=float, default=240)
    parser.add_argument("--allow-large", action="store_true")
    parser.add_argument("--port", type=int, default=47600)
    parser.add_argument("--worker-port", type=int, default=47700)
    args = parser.parse_args(argv)
    database = os.environ.get("OMOBA_TEST_DATABASE_URL", "")
    if not isolated_database(database):
        parser.error("set OMOBA_TEST_DATABASE_URL to a loopback /omoba_test URI without options")
    if (not 10 <= args.clients <= (1000 if args.allow_large else 100)
            or args.clients % 10 or not 15 <= args.seconds <= 120
            or not args.seconds + 30 <= args.timeout <= 600):
        parser.error("require multiples of10 clients (large opt-in above100),15..120s soak and bounded timeout")
    rooms = args.clients // 10
    if not (1024 <= args.port <= 65535 and 1024 <= args.worker_port <= 65536 - rooms
            and args.port not in range(args.worker_port, args.worker_port + rooms)):
        parser.error("invalid/overlapping loopback lobby and worker port range")
    for binary in (args.server_bin, args.bots_bin):
        if not binary.is_file() or not os.access(binary, os.X_OK):
            parser.error(f"not an executable: {binary}")
    for port in [args.port, *range(args.worker_port, args.worker_port + rooms)]:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.bind(("127.0.0.1", port))
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
    report = {"status": "FAIL", "clients": args.clients, "arenas": rooms,
              "preference": "quick", "server_bots": 0, "soak_seconds": args.seconds,
              "scope": "local Casual signed native AI clients; not RU/cgroup or Ranked proof",
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
    report["source_sha256"] = {
        name: hashlib.sha256((source_root / name).read_bytes()).hexdigest()
        for name in ("scripts/public_playing_capacity.py", "scripts/public_bots_rehearsal.py",
                     "harness/src/bin/bots.rs", "harness/src/bot.rs", "harness/src/public_client.rs",
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
            process = subprocess.Popen([str(args.bots_bin.resolve()), "--count", "10", "--server",
                                        f"127.0.0.1:{args.port}", "--public", "--telemetry",
                                        "--preference", "quick", "--search-gate", str(search_gate)], env=env, cwd=root,
                                       stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            register(process); shards.append(process)
            readers.append((root / f"bots-{index:03d}.log").open())
        next_resources = 0
        while time.monotonic() - started < args.timeout:
            now = time.monotonic()
            assert all(p.poll() is None for p in children), "owned server/generator exited unexpectedly"
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
                running_at, baseline = now, current
                report["all_running_after_seconds"] = now - started
            if running_at is not None:
                assert valid, "population lost Running/freshness/identity/roster/error acceptance during soak"
                if now >= next_resources:
                    report["resources"].append(process_sample(lobby, shards))
                    next_resources = now + 2
                if now - running_at >= args.seconds:
                    report["durable_rosters"] = verify_manifests(root, current, args.clients)
                    report["wire"] = summarize_wire(baseline, current)
                    report["status"] = "PASS"
                    break
            time.sleep(.1)
        assert report["status"] == "PASS", "not every playing peer reached acceptance before timeout"
    except Exception as error:
        report["error"] = f"{type(error).__name__}: {error}"
        raise
    finally:
        report["elapsed_seconds"] = time.monotonic() - started
        report["latest_shards"] = {str(i): sample for i, (_, sample) in latest.items()}
        report["latest_auth_shards"] = {str(i): sample for i, (_, sample) in auth_latest.items()}
        (root / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        stop_owned(children)
        for file in readers + writers:
            file.close()
    print(json.dumps({k: v for k, v in report.items() if k not in ("latest_shards", "resources")}))


if __name__ == "__main__":
    main()
