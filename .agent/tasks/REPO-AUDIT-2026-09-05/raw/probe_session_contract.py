#!/usr/bin/env python3
"""Observe the current server's session contract using only loopback UDP.

This is an audit probe, not a regression gate: observed behavior is recorded
without assuming whether Join, reclaim, or release debug commands are accepted.
Exit 0 means every observation completed; nonzero means the probe failed.
The supplied server must already have been built from the audited checkout.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import select
import socket
import subprocess
import sys
import tempfile
import time
import uuid


class ProbeFailure(RuntimeError):
    pass


class Bot:
    def __init__(self, address: tuple[str, int], team: str):
        self.socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.socket.bind(("127.0.0.1", 0))
        self.socket.connect(address)
        self.socket.setblocking(False)
        self.team = team
        self.session_id = "audit-" + uuid.uuid4().hex
        self.snapshot: dict | None = None
        self.last_ping = 0.0
        self.closed = False

    def send(self, packet: dict) -> None:
        self.socket.send(json.dumps(packet, separators=(",", ":")).encode())

    def join(self) -> None:
        self.send({
            "type": "join", "team": self.team, "character": "ipfs",
            "hero_class": "warrior", "session_id": self.session_id,
        })

    def own(self) -> dict | None:
        if self.snapshot is None:
            return None
        return find_player(self.snapshot, self.snapshot["your_id"])

    def close(self) -> None:
        if not self.closed:
            self.socket.close()
            self.closed = True


def find_player(snapshot: dict, player_id: int) -> dict | None:
    return next((p for p in snapshot.get("players", []) if p["id"] == player_id), None)


def distance(a: dict, b: dict) -> float:
    return math.hypot(a["x"] - b["x"], a["z"] - b["z"])


def state(player: dict | None) -> dict | None:
    if player is None:
        return None
    fields = (
        "id", "team", "hero_class", "x", "y", "z", "hp", "max_hp",
        "mana", "max_mana", "level", "xp", "next_level_xp", "gold",
        "skill_points", "ranks", "action_sequence",
    )
    return {field: player.get(field) for field in fields}


class Probe:
    def __init__(self, process: subprocess.Popen, deadline: float):
        self.process = process
        self.deadline = deadline
        self.bots: list[Bot] = []

    def pump(self, seconds: float = 0.05) -> None:
        if time.monotonic() > self.deadline:
            raise ProbeFailure("overall 90-second probe budget exceeded")
        if self.process.poll() is not None:
            raise ProbeFailure(f"server exited early: {self.process.returncode}")
        active = [bot for bot in self.bots if not bot.closed]
        now = time.monotonic()
        for bot in active:
            if now - bot.last_ping >= 0.35:
                try:
                    bot.send({"type": "ping"})
                except ConnectionRefusedError:
                    pass  # Startup may precede the server binding its socket.
                bot.last_ping = now
        sockets = {bot.socket: bot for bot in active}
        readable, _, _ = select.select(list(sockets), [], [], seconds)
        for sock in readable:
            while True:
                try:
                    raw = sock.recv(65_536)
                except (BlockingIOError, ConnectionRefusedError):
                    break
                packet = json.loads(raw)
                if packet.get("type") == "snapshot":
                    sockets[sock].snapshot = packet

    def until(self, description: str, predicate, timeout: float = 5.0, action=None):
        deadline = time.monotonic() + timeout
        next_action = 0.0
        while time.monotonic() < deadline:
            now = time.monotonic()
            if action is not None and now >= next_action:
                action()
                next_action = now + 0.05
            self.pump()
            value = predicate()
            if value:
                return value
        raise ProbeFailure(f"timed out waiting for {description}")

    def settle(self, seconds: float) -> None:
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            self.pump(min(0.05, max(0.0, deadline - time.monotonic())))


def move_toward_center(bot: Bot) -> None:
    # Requested positions are intentionally distant: the production server
    # clamps each 20 Hz request using its own speed and elapsed-time limits.
    bot.send({"type": "transform", "x": 0.0, "y": 0.5, "z": 0.0, "yaw": 0.0})


def movement_sample(probe: Probe, bot: Bot, seconds: float = 1.3) -> dict:
    before = dict(bot.own())
    started = time.monotonic()
    deadline = started + seconds
    next_move = 0.0
    while time.monotonic() < deadline:
        now = time.monotonic()
        if now >= next_move:
            move_toward_center(bot)
            next_move = now + 0.05
        probe.pump(0.01)
    probe.settle(0.12)
    after = dict(bot.own())
    return {
        "before": state(before), "after": state(after),
        "elapsed_seconds": round(time.monotonic() - started, 3),
        "distance": round(distance(before, after), 4),
    }


def cast_and_observe(probe: Probe, caster: Bot, target: Bot) -> dict:
    before_caster = dict(caster.own())
    before_target = dict(target.own())
    caster.send({"type": "cast", "slot": 0,
                 "target": {"kind": "player", "id": before_target["id"]}})
    probe.until("accepted cast action", lambda: (
        caster.own() and caster.own().get("action_sequence", 0)
        != before_caster.get("action_sequence", 0)
    ))
    probe.until("projectile damage", lambda: (
        target.own() and target.own()["hp"] < before_target["hp"]
    ))
    return {"caster_before": state(before_caster), "caster_after": state(caster.own()),
            "target_before": state(before_target), "target_after": state(target.own())}


def run_observations(probe: Probe, address: tuple[str, int], observations: dict) -> None:
    green, blue = Bot(address, "green"), Bot(address, "blue")
    probe.bots.extend((green, blue))
    probe.until("initial transport snapshots", lambda: green.snapshot and blue.snapshot)
    green.join()
    blue.join()
    probe.until("both joins and release 1v1 countdown", lambda: (
        green.own() and blue.own()
        and green.snapshot["game_state"]["type"] == "running"
    ), timeout=8.0)
    observations["initial_players"] = [state(green.own()), state(blue.own())]

    green.send({"type": "set_speed_boost", "enabled": False})
    probe.settle(0.15)
    normal = movement_sample(probe, green)
    green.send({"type": "set_speed_boost", "enabled": True})
    probe.settle(0.15)
    boosted = movement_sample(probe, green)
    green.send({"type": "set_speed_boost", "enabled": False})
    observations["release_speed_boost"] = {
        "normal": normal, "after_enable_command": boosted,
        "distance_ratio": round(boosted["distance"] / max(normal["distance"], 0.0001), 3),
        "larger_distance_observed": boosted["distance"] > normal["distance"] * 1.5,
    }

    probe.until("players within safe Q range", lambda: (
        green.own() and blue.own() and distance(green.own(), blue.own()) < 8.0
    ), timeout=30.0, action=lambda: (move_toward_center(green), move_toward_center(blue)))
    if min(green.own()["hp"], blue.own()["hp"]) <= 0:
        raise ProbeFailure("a player died before the controlled combat observation")

    first_cast = cast_and_observe(probe, green, blue)
    before_enable = dict(blue.own())
    blue.send({"type": "set_god_mode", "enabled": True})
    probe.settle(0.2)
    after_enable = dict(blue.own())
    observations["release_god_mode"] = {
        "wounding_cast": first_cast,
        "before_enable": state(before_enable), "after_enable": state(after_enable),
        "healed_to_full_after_command": (
            before_enable["hp"] < before_enable["max_hp"]
            and after_enable["hp"] == after_enable["max_hp"]
        ),
    }
    blue.send({"type": "set_god_mode", "enabled": False})
    probe.settle(0.65)
    observations["second_controlled_cast"] = cast_and_observe(probe, green, blue)

    before_duplicate = dict(green.own())
    green.join()
    probe.settle(0.2)
    after_duplicate = dict(green.own())
    observations["duplicate_join"] = {
        "before": state(before_duplicate), "after": state(after_duplicate),
        "same_player_id": before_duplicate["id"] == after_duplicate["id"],
        "position_delta": round(distance(before_duplicate, after_duplicate), 4),
        "mana_delta": round(after_duplicate["mana"] - before_duplicate["mana"], 4),
        "action_sequence_reset": (
            before_duplicate.get("action_sequence", 0) > 0
            and after_duplicate.get("action_sequence", 0) == 0
        ),
    }

    original_id = blue.own()["id"]
    token = blue.session_id
    last_visible = dict(blue.own())
    disconnected_at = time.monotonic()
    blue.close()

    def removed():
        nonlocal last_visible
        player = find_player(green.snapshot, original_id)
        if player is not None:
            last_visible = dict(player)
            return False
        return True

    probe.until("old endpoint timeout/removal", removed, timeout=8.0)
    timeout_elapsed = time.monotonic() - disconnected_at
    replacement = Bot(address, "blue")
    replacement.session_id = token
    probe.bots.append(replacement)
    probe.until("replacement transport snapshot", lambda: replacement.snapshot)
    replacement.join()
    probe.until("reclaimed or fresh joined player", lambda: replacement.own())
    after_rejoin = dict(replacement.own())
    observations["reconnect"] = {
        "old_id": original_id, "new_id": after_rejoin["id"],
        "same_player_id": original_id == after_rejoin["id"],
        "timeout_seconds": round(timeout_elapsed, 3),
        "last_visible_before_timeout": state(last_visible),
        "after_rejoin": state(after_rejoin),
        "position_delta": round(distance(last_visible, after_rejoin), 4),
        "hp_delta": round(after_rejoin["hp"] - last_visible["hp"], 4),
    }
    observations["limitations"] = [
        "Loopback observation; this probe does not emulate packet loss or reordering.",
        "Progression is recorded but not artificially raised; rematch progression requires a separate test.",
        "Movement samples include the production server's per-request tolerance and are not speed benchmarks.",
        "Observed differences are evidence, not automatic pass/fail judgments of the intended product contract.",
    ]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[4])
    parser.add_argument("--server-bin", type=Path)
    args = parser.parse_args()
    repo = args.repo.resolve()
    server_bin = (args.server_bin or repo / "target/debug/server").resolve()
    report: dict = {"probe": "session-contract-live-udp", "status": "PROBE_FAILED",
                    "server_binary": str(server_bin), "repo": str(repo), "observations": {}}
    process = None
    probe = None
    started = time.monotonic()
    exit_code = 1
    with tempfile.TemporaryFile(mode="w+b") as server_log:
        try:
            if not server_bin.is_file():
                raise ProbeFailure(f"prebuilt server missing: {server_bin}")
            digest = hashlib.sha256()
            with server_bin.open("rb") as binary:
                for chunk in iter(lambda: binary.read(1024 * 1024), b""):
                    digest.update(chunk)
            report["server_sha256"] = digest.hexdigest()
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as reservation:
                reservation.bind(("127.0.0.1", 0))
                port = reservation.getsockname()[1]
            env = os.environ.copy()
            env.update(SERVER_ADDR=f"127.0.0.1:{port}", OMOBA_MATCH_MODE="release", OMOBA_TEAM_SIZE="1")
            process = subprocess.Popen([str(server_bin)], cwd=repo, env=env,
                                       stdin=subprocess.DEVNULL, stdout=server_log, stderr=subprocess.STDOUT)
            probe = Probe(process, started + 90.0)
            report["server_pid"] = process.pid
            report["server_address"] = f"127.0.0.1:{port}"
            run_observations(probe, ("127.0.0.1", port), report["observations"])
            report["status"] = "OBSERVATIONS_COMPLETED"
            exit_code = 0
        except Exception as error:
            report["error"] = f"{type(error).__name__}: {error}"
        finally:
            if probe is not None:
                for bot in probe.bots:
                    bot.close()
            if process is not None:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=3.0)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=3.0)
                report["owned_server_exit_code"] = process.returncode
            server_log.seek(0)
            report["server_log"] = server_log.read().decode("utf-8", errors="replace").splitlines()
    report["elapsed_seconds"] = round(time.monotonic() - started, 3)
    print(json.dumps(report, indent=2))
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
