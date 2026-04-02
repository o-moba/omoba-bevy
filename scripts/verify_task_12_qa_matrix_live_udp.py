#!/usr/bin/env python3
"""Live UDP checks for TASK-12 manual QA matrix rows M1/M2/M3.

Requires a debug server binary (`cargo build -p server`). Uses a free localhost port
to avoid colliding with `verify_task_02_multiplayer_session_flow.py` (4010).

Evidence: run and capture stdout/stderr for `docs/manual-qa-matrix.md` run log references.
"""
from __future__ import annotations

import json
import math
import os
import socket
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path

SERVER_START_WAIT_SECONDS = 1.0
SNAPSHOT_WAIT_SECONDS = 4.0
PING_INTERVAL_SECONDS = 0.25
# Cross-map homing projectile needs several seconds of simulation at current speed.
CAST_PUMP_SECONDS = 16.0
SERVER_ADDR = ("127.0.0.1", 4012)
# Large snapshots (minions, structures); must not truncate JSON.
MAX_PACKET_SIZE = 64 * 1024
M2_VICTORY_TIMEOUT_SECONDS = 75.0
M2_REMATCH_TIMEOUT_SECONDS = 16.0

TARGET_BASE_RUN_TIME_SECONDS = 45.0
PLAYER_SPEED = 5.0
BASE_PAD_SIZE = 46.0
BASE_EDGE_MARGIN = 6.0
PLAYER_SPAWN_OFFSET = 7.0


def assert_true(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def spawn_for_team(team: str) -> tuple[float, float, float]:
    target_base_distance = PLAYER_SPEED * TARGET_BASE_RUN_TIME_SECONDS
    inner_side = target_base_distance / math.sqrt(2.0)
    half_inner_side = inner_side * 0.5
    base_padding = BASE_PAD_SIZE * 0.5 + BASE_EDGE_MARGIN
    if team == "green":
        base_x = -half_inner_side
        base_z = -half_inner_side
    elif team == "blue":
        base_x = half_inner_side
        base_z = half_inner_side
    else:
        raise AssertionError(f"Unknown team: {team}")
    length = math.sqrt(base_x * base_x + base_z * base_z)
    dir_x = -base_x / length
    dir_z = -base_z / length
    return (
        base_x + dir_x * PLAYER_SPAWN_OFFSET,
        0.5,
        base_z + dir_z * PLAYER_SPAWN_OFFSET,
    )


def assert_spawn_matches(player: dict, team: str, epsilon: float = 0.08) -> None:
    ex, ey, ez = spawn_for_team(team)
    assert_true(abs(player["x"] - ex) <= epsilon, f"{team} spawn x mismatch: {player}")
    assert_true(abs(player["y"] - ey) <= epsilon, f"{team} spawn y mismatch: {player}")
    assert_true(abs(player["z"] - ez) <= epsilon, f"{team} spawn z mismatch: {player}")


@dataclass
class RowResult:
    matrix_id: str
    outcome: str
    details: str


class ServerHandle:
    def __init__(self, repo_root: Path, server_addr: tuple[str, int]):
        self.repo_root = repo_root
        self.server_addr = server_addr
        self.log_file = tempfile.NamedTemporaryFile(
            prefix="task-12-server-", suffix=".log", delete=False
        )
        self.process: subprocess.Popen[str] | None = None

    @property
    def log_path(self) -> str:
        return self.log_file.name

    def start(self) -> None:
        env = os.environ.copy()
        env["SERVER_ADDR"] = f"{self.server_addr[0]}:{self.server_addr[1]}"
        server_bin = self.repo_root / "target" / "debug" / "server"
        if not server_bin.exists():
            raise AssertionError(f"Missing server binary at {server_bin}; run `cargo build -p server`.")
        self.process = subprocess.Popen(
            [str(server_bin)],
            cwd=self.repo_root,
            env=env,
            stdout=self.log_file,
            stderr=subprocess.STDOUT,
            text=True,
        )
        time.sleep(SERVER_START_WAIT_SECONDS)
        if self.process.poll() is not None:
            raise AssertionError(
                f"Server exited early with code {self.process.returncode}. Log: {self.log_path}"
            )

    def stop(self) -> None:
        if self.process is None or self.process.poll() is not None:
            return
        self.process.terminate()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=5)


class ProtocolClient:
    def __init__(self, server_addr: tuple[str, int]):
        self.server_addr = server_addr
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.bind(("127.0.0.1", 0))
        self.sock.connect(server_addr)
        self.sock.settimeout(0.15)
        self.last_snapshot: dict | None = None

    def close(self) -> None:
        self.sock.close()

    def send(self, packet: dict) -> None:
        self.sock.send(json.dumps(packet).encode("utf-8"))

    def ping(self) -> None:
        self.send({"type": "ping"})

    def join(self, team: str, character: str) -> None:
        self.send({"type": "join", "team": team, "character": character})

    def transform_at_spawn(self, team: str, yaw: float = 0.0) -> None:
        x, y, z = spawn_for_team(team)
        self.send({"type": "transform", "x": x, "y": y, "z": z, "yaw": yaw})

    def cast_player(self, target_id: int) -> None:
        self.send({"type": "cast", "target": {"kind": "player", "id": target_id}})

    def cast_structure(self, target_id: int) -> None:
        self.send({"type": "cast", "target": {"kind": "structure", "id": target_id}})

    def request_rematch(self) -> None:
        self.send({"type": "request_rematch"})

    def recv_once(self) -> dict | None:
        try:
            payload = self.sock.recv(MAX_PACKET_SIZE)
        except socket.timeout:
            return None
        packet = json.loads(payload.decode("utf-8"))
        if packet.get("type") == "snapshot":
            self.last_snapshot = packet
        return packet


def pump_until(
    clients: list[ProtocolClient],
    predicate,
    timeout_seconds: float,
    description: str,
) -> list[dict | None]:
    deadline = time.monotonic() + timeout_seconds
    last_ping_at = 0.0
    last_snapshots = [c.last_snapshot for c in clients]
    while time.monotonic() < deadline:
        now = time.monotonic()
        if now - last_ping_at >= PING_INTERVAL_SECONDS:
            for c in clients:
                c.ping()
            last_ping_at = now
        for i, c in enumerate(clients):
            pkt = c.recv_once()
            if pkt is not None and pkt.get("type") == "snapshot":
                last_snapshots[i] = pkt
        if predicate(last_snapshots):
            return last_snapshots
        time.sleep(0.02)
    raise AssertionError(f"Timed out: {description}. Last: {last_snapshots}")


def player_map(snapshot: dict) -> dict[int, dict]:
    return {int(p["id"]): p for p in snapshot["players"]}


def game_state_kind(snapshot: dict) -> str:
    game_state = snapshot.get("game_state", {})
    if isinstance(game_state, dict):
        return str(game_state.get("type", "unknown"))
    return str(game_state)


def game_state_winner(snapshot: dict) -> str | None:
    game_state = snapshot.get("game_state", {})
    if isinstance(game_state, dict):
        winner = game_state.get("winner")
        return str(winner) if winner is not None else None
    return None


def blue_base_tower(snapshot: dict) -> dict:
    for structure in snapshot.get("structures", []):
        if structure.get("team") == "blue" and structure.get("kind") == "base_tower":
            return structure
    raise AssertionError("Could not find blue base tower in snapshot")


def scenario_m1_two_clients(server: ServerHandle) -> RowResult:
    a = ProtocolClient(server.server_addr)
    b = ProtocolClient(server.server_addr)
    try:
        a.join("green", "ipfs")
        pump_until(
            [a],
            lambda s: s[0] is not None and len(s[0]["players"]) == 1,
            SNAPSHOT_WAIT_SECONDS,
            "first join",
        )
        b.join("blue", "wang")
        snaps = pump_until(
            [a, b],
            lambda s: all(
                x is not None and len(x["players"]) == 2 for x in s
            ),
            SNAPSHOT_WAIT_SECONDS,
            "two players visible",
        )
        sa, sb = snaps
        assert sa is not None and sb is not None
        aid, bid = sa["your_id"], sb["your_id"]
        assert_true(aid != bid, "duplicate player id")
        expect_player(sa, aid, "green", "ipfs")
        expect_player(sa, bid, "blue", "wang")
        expect_player(sb, aid, "green", "ipfs")
        expect_player(sb, bid, "blue", "wang")
        return RowResult(
            "M1",
            "PASS",
            f"UDP clients saw distinct ids {aid}/{bid} and consistent two-player snapshots",
        )
    finally:
        a.close()
        b.close()


def expect_player(snapshot: dict, pid: int, team: str, character: str) -> None:
    players = player_map(snapshot)
    assert_true(pid in players, f"missing {pid}")
    pl = players[pid]
    assert_true(pl["team"] == team, pl)
    assert_true(pl["character"] == character, pl)
    assert_spawn_matches(pl, team)


def scenario_m3_cast_player(server: ServerHandle) -> RowResult:
    green = ProtocolClient(server.server_addr)
    blue = ProtocolClient(server.server_addr)
    try:
        green.join("green", "ipfs")
        blue.join("blue", "wang")
        pump_until(
            [green, blue],
            lambda s: all(x is not None and len(x["players"]) == 2 for x in s),
            SNAPSHOT_WAIT_SECONDS,
            "pre-cast join",
        )
        assert green.last_snapshot is not None
        gid = green.last_snapshot["your_id"]
        players = player_map(green.last_snapshot)
        bid = next(pid for pid, p in players.items() if p["team"] == "blue")

        g0 = players[gid]
        mana_before = float(g0["mana"])
        b0_hp = float(players[bid]["hp"])
        moved = False

        green.send(
            {
                "type": "transform",
                "x": float(g0["x"]) + 2.0,
                "y": float(g0["y"]),
                "z": float(g0["z"]),
                "yaw": float(g0.get("yaw", 0.0)),
            }
        )

        green.cast_player(bid)

        deadline = time.monotonic() + CAST_PUMP_SECONDS
        last_ping = 0.0
        ok_mana = False
        ok_hp = False
        while time.monotonic() < deadline:
            t = time.monotonic()
            if t - last_ping >= PING_INTERVAL_SECONDS:
                green.ping()
                blue.ping()
                last_ping = t
            for c in (green, blue):
                c.recv_once()
            gs = green.last_snapshot
            if gs is None:
                continue
            gm = player_map(gs).get(gid, {})
            bm = player_map(gs).get(bid, {})
            if gm and abs(float(gm["x"]) - float(g0["x"])) > 0.5:
                moved = True
            # Mana regen runs between snapshots; require a clear spend, not exact post-cast value.
            if gm and float(gm["mana"]) < mana_before - 5.0:
                ok_mana = True
            if bm and float(bm["hp"]) < b0_hp - 0.01:
                ok_hp = True
            if ok_mana and ok_hp and moved:
                return RowResult(
                    "M3",
                    "PASS",
                    "Move+cast path verified: transform changed position, cast drained mana, target HP dropped",
                )
            time.sleep(0.02)

        return RowResult(
            "M3",
            "FAIL",
            f"moved={moved} mana_ok={ok_mana} hp_ok={ok_hp} (see server log {server.log_path})",
        )
    finally:
        green.close()
        blue.close()


def scenario_m2_victory_and_rematch(server: ServerHandle) -> RowResult:
    g1 = ProtocolClient(server.server_addr)
    g2 = ProtocolClient(server.server_addr)
    try:
        g1.join("green", "ipfs")
        g2.join("green", "wang")
        snaps = pump_until(
            [g1, g2],
            lambda s: all(x is not None and len(x["players"]) >= 2 for x in s),
            SNAPSHOT_WAIT_SECONDS,
            "M2 pre-join",
        )
        s1 = snaps[0]
        assert s1 is not None
        blue_base = blue_base_tower(s1)
        base_id = int(blue_base["id"])
        initial_blue_base_hp = float(blue_base["hp"])
        min_hp_seen = initial_blue_base_hp

        victory_seen = False
        victory_at = 0.0
        deadline = time.monotonic() + M2_VICTORY_TIMEOUT_SECONDS
        last_ping = 0.0
        while time.monotonic() < deadline:
            now = time.monotonic()
            if now - last_ping >= PING_INTERVAL_SECONDS:
                g1.transform_at_spawn("green")
                g2.transform_at_spawn("green")
                g1.cast_structure(base_id)
                g2.cast_structure(base_id)
                g1.ping()
                g2.ping()
                last_ping = now
            for c in (g1, g2):
                c.recv_once()
            s = g1.last_snapshot
            if s is None:
                time.sleep(0.02)
                continue
            try:
                base = blue_base_tower(s)
                min_hp_seen = min(min_hp_seen, float(base["hp"]))
            except AssertionError:
                # Base tower can disappear from the structures list after destruction.
                min_hp_seen = 0.0
            if game_state_kind(s) == "victory" and game_state_winner(s) == "green":
                victory_seen = True
                victory_at = now
                break
            time.sleep(0.02)

        if not victory_seen:
            return RowResult(
                "M2",
                "FAIL",
                f"Did not reach victory in {M2_VICTORY_TIMEOUT_SECONDS}s, min blue base HP seen={min_hp_seen:.2f}",
            )

        rematch_deadline = victory_at + M2_REMATCH_TIMEOUT_SECONDS
        rematch_running = False
        while time.monotonic() < rematch_deadline:
            for c in (g1, g2):
                c.ping()
                c.recv_once()
            s = g1.last_snapshot
            if s is None:
                time.sleep(0.05)
                continue
            if game_state_kind(s) == "running":
                try:
                    base = blue_base_tower(s)
                    if float(base["hp"]) >= initial_blue_base_hp * 0.95:
                        rematch_running = True
                        break
                except AssertionError:
                    pass
            time.sleep(0.05)

        if not rematch_running:
            return RowResult(
                "M2",
                "FAIL",
                "Victory reached, but rematch/reset to running with restored blue base was not observed in time",
            )

        return RowResult(
            "M2",
            "PASS",
            f"Reached victory by destroying blue base (min HP {min_hp_seen:.2f}), then auto-rematch reset to running",
        )
    finally:
        g1.close()
        g2.close()


def run_isolated(repo_root: Path, scenario) -> RowResult:
    """Fresh server per scenario so prior UDP sessions cannot leave extra players."""
    server = ServerHandle(repo_root, SERVER_ADDR)
    try:
        server.start()
        return scenario(server)
    finally:
        server.stop()


def main() -> int:
    repo_root = Path(__file__).resolve().parents[1]
    rows: list[RowResult] = []
    try:
        rows.append(run_isolated(repo_root, scenario_m1_two_clients))
        time.sleep(0.3)
        rows.append(run_isolated(repo_root, scenario_m2_victory_and_rematch))
        time.sleep(0.3)
        rows.append(run_isolated(repo_root, scenario_m3_cast_player))
    except Exception as err:
        print(f"FAIL: {err}", file=sys.stderr)
        return 1

    summary = {
        "script": "verify_task_12_qa_matrix_live_udp.py",
        "server_addr": f"{SERVER_ADDR[0]}:{SERVER_ADDR[1]}",
        "rows": [{"id": r.matrix_id, "outcome": r.outcome, "details": r.details} for r in rows],
    }
    print(json.dumps(summary, indent=2))
    if any(r.outcome != "PASS" for r in rows):
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
