#!/usr/bin/env python3
"""Exercise signed AI fill clients on an owned loopback lobby and disposable DB.

Build server and harness --bin bots first. Database migrations must already be
applied (make test-postgres prepares the disposable omoba_test database). This
runner never connects to a configured live lobby or changes its limits.
"""
import argparse
from collections import deque
import json
import math
import os
from pathlib import Path
import signal
import socket
import subprocess
import time
from urllib.parse import urlsplit


def isolated_database(raw):
    try:
        url = urlsplit(raw)
        return (url.scheme in ("postgres", "postgresql")
                and url.hostname in ("127.0.0.1", "localhost", "::1")
                and url.path == "/omoba_test" and not url.query and not url.fragment)
    except ValueError:
        return False


def free_port():
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def running_roster(sample, count):
    peers = sample.get("peers", [])
    return (sample.get("phase") == "running" and len(peers) == count
            and len({peer.get("id") for peer in peers}) == count
            and all(peer.get("id") is not None
                    and peer.get("snapshot_age_ms") is not None
                    and peer["snapshot_age_ms"] < 3000
                    and peer.get("public", {}).get("phase") == "running"
                    and peer.get("public", {}).get("profile_id") for peer in peers)
            and len({peer["public"]["profile_id"] for peer in peers}) == count)


def verify_results(statuses, count, ranked, custom_map=False):
    assert len(statuses) == count, "not every AI client returned a saved result"
    profiles = {status["profile"]["profile_id"] for status in statuses}
    assert len(profiles) == count, "duplicate result/profile identities"
    ids = {status["result"]["result_id"] for status in statuses}
    assert len(ids) == 1, "clients settled different matches"
    initial_profiles = {s["profile"]["profile_id"]: s["initial_profile"] for s in statuses}
    for status in statuses:
        result, profile, initial = (status[k] for k in ("result", "profile", "initial_profile"))
        assert result["saved"] and result["outcome"] == "completed"
        assert result.get("gameplay_started", True) and result["winner"] is not None
        assert result["rated"] == ranked
        if custom_map:
            assert not result["rated"]
            assert result["ruleset"] in ("custom-unrated-v1", "practice-bots-v1")
        else:
            assert result["ruleset"] == "verdant-default-v1"
        assert len(result["participants"]) == 10
        assert profile["matches_played"] == initial["matches_played"] + 1
        assert sum(row["result_id"] == result["result_id"] for row in status["history"]) == 1
        me = next(p for p in result["participants"] if p["profile_id"] == profile["profile_id"])
        assert not me.get("is_bot", False)
        assert profile["progression_xp"] == initial["progression_xp"] + me["progression_xp_gained"]
        if custom_map:
            assert me["rating"] is None and me["progression_xp_gained"] == 0
            assert profile["rating"] == initial["rating"]
            assert profile["rated_matches"] == initial["rated_matches"]
        history = next(row for row in status["history"] if row["result_id"] == result["result_id"])
        assert history["rating"] == me["rating"]
        if ranked:
            assert {p["profile_id"] for p in result["participants"]} == profiles
            assert all(not p.get("is_bot", False) and p["profile_id"] for p in result["participants"])
            rating = me["rating"]
            assert rating
            assert rating["before"] == initial["rating"]
            assert rating["after"] == profile["rating"] == rating["before"] + rating["delta"]
            assert profile["rated_matches"] == initial["rated_matches"] + 1
            won = me["team"] == result["winner"]
            team_ratings = [initial_profiles[p["profile_id"]]["rating"]
                            for p in result["participants"] if p["team"] == me["team"]]
            opposing_ratings = [initial_profiles[p["profile_id"]]["rating"]
                                for p in result["participants"] if p["team"] != me["team"]]
            expected_score = 1 / (1 + 10 ** ((sum(opposing_ratings) / len(opposing_ratings)
                                            - sum(team_ratings) / len(team_ratings)) / 400))
            k = 48 if initial["rated_matches"] < 20 else 32
            requested = k * (int(won) - expected_score)
            # Rust f64::round rounds half away from zero, unlike Python round.
            delta = math.floor(abs(requested) + 0.5) * (1 if requested >= 0 else -1)
            expected_after = max(0, min(5000, initial["rating"] + delta))
            assert rating["after"] == expected_after, "wrong placement/team Elo award"
            assert me["progression_xp_gained"] == (150 if won else 100)
            assert profile["wins"] == initial["wins"] + int(won)
            assert profile["losses"] == initial["losses"] + int(not won)
    return ids.pop()


def read_events(stream):
    """Leave a partially flushed JSON line for the next poll, never consume it."""
    while True:
        offset = stream.tell()
        line = stream.readline()
        if not line:
            return
        if not line.endswith("\n"):
            stream.seek(offset)
            return
        for kind in ("BOT_SAMPLE", "BOT_RESULT"):
            prefix = kind + " "
            if line.startswith(prefix):
                yield kind, json.loads(line[len(prefix):])
                break


def stop_owned(children):
    for child in reversed(children):
        try:
            os.killpg(child.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    for child in reversed(children):
        try:
            child.wait(timeout=20)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.wait()


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server-bin", type=Path, required=True)
    parser.add_argument("--bots-bin", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--preference", choices=("humans_only", "quick", "bot_practice"), default="humans_only")
    parser.add_argument("--clients", type=int, default=10)
    parser.add_argument("--seconds", type=float, default=15)
    parser.add_argument("--require-victory", action="store_true")
    parser.add_argument("--test-map", type=Path, help="explicit local custom map; never a rated proof")
    parser.add_argument("--timeout", type=float, default=180)
    args = parser.parse_args(argv)
    database = os.environ.get("OMOBA_TEST_DATABASE_URL", "")
    if not isolated_database(database):
        parser.error("set OMOBA_TEST_DATABASE_URL to a loopback /omoba_test database without URL options")
    if not 1 <= args.clients <= 10 or (args.preference == "humans_only" and args.clients != 10):
        parser.error("use 1..10 clients; Ranked requires ten")
    if not 0 < args.seconds <= args.timeout <= 3600:
        parser.error("require 0 < seconds <= timeout <= 3600")
    for binary in (args.server_bin, args.bots_bin):
        if not binary.is_file() or not os.access(binary, os.X_OK):
            parser.error(f"not an executable: {binary}")
    root = args.output.resolve()
    # Never reuse a prior allocation root or overwrite somebody else's evidence.
    root.mkdir(parents=True, exist_ok=False)
    port, worker = free_port(), free_port()
    while worker == port:
        worker = free_port()
    env = {k: v for k, v in os.environ.items()
           if not k.startswith(("OMOBA_", "GAME_")) and k != "SERVER_ADDR"}
    env.update(SERVER_ADDR=f"127.0.0.1:{port}", OMOBA_SERVER_ROLE="lobby",
               OMOBA_DATABASE_URL=database, OMOBA_MATCH_ROOT=str(root / "matches"),
               OMOBA_CAREER_OUTBOX=str(root / "outbox"), OMOBA_MATCH_CAPACITY="1",
               OMOBA_MATCH_FIRST_PORT=str(worker), OMOBA_MATCH_BIND_IP="127.0.0.1",
               OMOBA_MATCH_PUBLIC_HOST="127.0.0.1", OMOBA_LIMIT_ENDPOINTS_PER_IP="16",
               OMOBA_LIMIT_NEW_PROFILES_PER_IP="1000000", OMOBA_LIMIT_NEW_PROFILES_TOTAL="10000000")
    command = [str(args.bots_bin.resolve()), "--public", "--count", str(args.clients),
               "--server", f"127.0.0.1:{port}", "--preference", args.preference, "--telemetry"]
    if args.require_victory:
        command.append("--exit-after-result")
    if args.test_map:
        command.append("--rush-objectives")
    if args.test_map:
        source = args.test_map.read_bytes()
        if len(source) > 65536:
            parser.error("rehearsal map exceeds 64 KiB")
        json.loads(source)
        config = root / "test-map.json"
        config.write_bytes(source)
        env["OMOBA_MATCH_TEST_MAP_CONFIG"] = str(config)
    children, samples = [], deque(maxlen=60)
    report = {"status": "RUNNING", "clients": args.clients, "preference": args.preference,
              "victory_required": args.require_victory, "test_map_override": bool(args.test_map)}
    started, running_at, bots_exited_at = time.monotonic(), None, None
    try:
        with (root / "lobby.log").open("w") as lf, (root / "bots.log").open("w") as bf:
            lobby = subprocess.Popen([str(args.server_bin.resolve())], env=env, cwd=root,
                                     stdout=lf, stderr=subprocess.STDOUT, start_new_session=True)
            children.append(lobby)
            ready_deadline = time.monotonic() + min(30, args.timeout)
            while "UDP game server is listening on" not in (root / "lobby.log").read_text():
                assert lobby.poll() is None, "owned lobby failed to start"
                assert time.monotonic() < ready_deadline, "owned lobby did not listen"
                time.sleep(0.05)
            bots = subprocess.Popen(command, env=env, cwd=root,
                                    stdout=bf, stderr=subprocess.STDOUT, start_new_session=True)
            children.append(bots)
            with (root / "bots.log").open() as evidence:
                while time.monotonic() - started < args.timeout:
                    for kind, payload in read_events(evidence):
                        if kind == "BOT_SAMPLE":
                            sample = payload
                            samples.append(sample)
                            if running_roster(sample, args.clients):
                                running_at = running_at or time.monotonic()
                        elif kind == "BOT_RESULT":
                            statuses = payload
                            report["result_id"] = verify_results(statuses, args.clients, args.preference == "humans_only" and not args.test_map, bool(args.test_map))
                            report["results"] = statuses
                    assert lobby.poll() is None, "owned lobby exited"
                    if report.get("result_id"):
                        assert bots.wait(timeout=5) == 0, "AI clients failed after result"
                        break
                    if bots.poll() is not None:
                        bots_exited_at = bots_exited_at or time.monotonic()
                        # The child can flush and exit immediately after our read.
                        # Drain its final line on the next poll before declaring loss.
                        assert time.monotonic() - bots_exited_at < 1, "AI clients exited before expected evidence"
                    if not args.require_victory and running_at and time.monotonic() - running_at >= args.seconds:
                        assert running_roster(samples[-1], args.clients), "roster disconnected during soak"
                        break
                    time.sleep(0.1)
                else:
                    raise AssertionError("deadline expired before the required gameplay evidence")
        assert running_at is not None, "no unanimous running roster"
        manifests = list((root / "matches").glob("*/manifest.json"))
        assert len(manifests) == 1, "expected exactly one worker allocation"
        manifest = json.loads(manifests[0].read_text())
        assert len(manifest["humans"]) == args.clients
        report.update(status="PASS", elapsed_seconds=time.monotonic() - started,
                      running_after_seconds=running_at - started, allocation_id=manifest["allocation_id"])
    except BaseException as error:
        report.update(status="FAIL", error=f"{type(error).__name__}: {error}")
        raise
    finally:
        stop_owned(children)
        report["samples"] = list(samples)
        (root / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps({k: v for k, v in report.items() if k not in ("samples", "results")}), flush=True)
    return 0


if __name__ == "__main__":
    def interrupted(_signal, _frame):
        raise InterruptedError("rehearsal interrupted; stopping owned processes")
    signal.signal(signal.SIGTERM, interrupted)
    raise SystemExit(main())
