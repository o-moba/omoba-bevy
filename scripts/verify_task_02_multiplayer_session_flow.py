#!/usr/bin/env python3
from __future__ import annotations
import json
import math
import os
import signal
import socket
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path

PLAYER_TIMEOUT_SECONDS = 5.0
SNAPSHOT_WAIT_SECONDS = 3.0
SERVER_START_WAIT_SECONDS = 1.0
PING_INTERVAL_SECONDS = 0.4
SERVER_ADDR = ("127.0.0.1", 4010)
MAX_PACKET_SIZE = 8 * 1024

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
    _half_map_size = half_inner_side + base_padding
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


def assert_spawn_matches(player: dict, team: str, epsilon: float = 0.05) -> None:
    expected_x, expected_y, expected_z = spawn_for_team(team)
    assert_true(abs(player["x"] - expected_x) <= epsilon, f"{team} spawn x mismatch: {player}")
    assert_true(abs(player["y"] - expected_y) <= epsilon, f"{team} spawn y mismatch: {player}")
    assert_true(abs(player["z"] - expected_z) <= epsilon, f"{team} spawn z mismatch: {player}")


@dataclass
class ScenarioResult:
    name: str
    details: str


class ServerHandle:
    def __init__(self, repo_root: Path, server_addr: tuple[str, int]):
        self.repo_root = repo_root
        self.server_addr = server_addr
        self.log_file = tempfile.NamedTemporaryFile(
            prefix="task-02-server-", suffix=".log", delete=False
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
            raise AssertionError(f"Missing server binary at {server_bin}; build the workspace first.")
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
        self.sock.settimeout(0.2)
        self.last_snapshot: dict | None = None

    @property
    def local_addr(self) -> tuple[str, int]:
        return self.sock.getsockname()

    def close(self) -> None:
        self.sock.close()

    def send(self, packet: dict) -> None:
        payload = json.dumps(packet).encode("utf-8")
        try:
            self.sock.send(payload)
        except ConnectionRefusedError:
            # Connected UDP sockets can surface a transient ICMP error after
            # the restart scenario stops the server. The pump loop will keep
            # retrying until a fresh snapshot arrives or the scenario times out.
            return

    def ping(self) -> None:
        self.send({"type": "ping"})

    def join(self, team: str, character: str) -> None:
        self.send({"type": "join", "team": team, "character": character})

    def recv_once(self) -> dict | None:
        try:
            payload = self.sock.recv(MAX_PACKET_SIZE)
        except (socket.timeout, ConnectionRefusedError):
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
    last_snapshots = [client.last_snapshot for client in clients]
    while time.monotonic() < deadline:
        now = time.monotonic()
        if now - last_ping_at >= PING_INTERVAL_SECONDS:
            for client in clients:
                client.ping()
            last_ping_at = now

        for index, client in enumerate(clients):
            packet = client.recv_once()
            if packet is not None and packet.get("type") == "snapshot":
                last_snapshots[index] = packet
        if predicate(last_snapshots):
            return last_snapshots
        time.sleep(0.02)

    raise AssertionError(f"Timed out while waiting for {description}. Last snapshots: {last_snapshots}")


def player_map(snapshot: dict) -> dict[int, dict]:
    return {player["id"]: player for player in snapshot["players"]}


def expect_player_state(
    snapshot: dict,
    player_id: int,
    team: str,
    character: str,
) -> dict:
    players = player_map(snapshot)
    assert_true(player_id in players, f"Missing player {player_id} in snapshot {snapshot}")
    player = players[player_id]
    assert_true(player["team"] == team, f"Team mismatch for {player_id}: {player}")
    assert_true(player["character"] == character, f"Character mismatch for {player_id}: {player}")
    assert_spawn_matches(player, team)
    return player


def scenario_sequential_join(server: ServerHandle) -> ScenarioResult:
    client_a = ProtocolClient(server.server_addr)
    client_b = ProtocolClient(server.server_addr)
    try:
        client_a.join("green", "ipfs")
        snapshot_a = pump_until(
            [client_a],
            lambda snapshots: snapshots[0] is not None and len(snapshots[0]["players"]) == 1,
            SNAPSHOT_WAIT_SECONDS,
            "first client join acknowledgement",
        )[0]
        assert snapshot_a is not None
        a_id = snapshot_a["your_id"]
        expect_player_state(snapshot_a, a_id, "green", "ipfs")

        client_b.join("blue", "wang")
        snapshots = pump_until(
            [client_a, client_b],
            lambda snapshots: all(
                snapshot is not None and len(snapshot["players"]) == 2 for snapshot in snapshots
            ),
            SNAPSHOT_WAIT_SECONDS,
            "two sequential clients in shared session",
        )
        snapshot_a, snapshot_b = snapshots
        assert snapshot_a is not None and snapshot_b is not None
        b_id = snapshot_b["your_id"]
        assert_true(a_id != b_id, "Sequential join reused the same player id")
        expect_player_state(snapshot_a, a_id, "green", "ipfs")
        expect_player_state(snapshot_a, b_id, "blue", "wang")
        expect_player_state(snapshot_b, a_id, "green", "ipfs")
        expect_player_state(snapshot_b, b_id, "blue", "wang")
        return ScenarioResult(
            name="M1 sequential join",
            details=f"client {a_id} and client {b_id} joined, saw each other, and spawned on correct sides",
        )
    finally:
        client_a.close()
        client_b.close()


def scenario_simultaneous_join(server: ServerHandle) -> ScenarioResult:
    client_a = ProtocolClient(server.server_addr)
    client_b = ProtocolClient(server.server_addr)
    try:
        client_a.join("green", "cube")
        client_b.join("blue", "toka")
        snapshots = pump_until(
            [client_a, client_b],
            lambda snapshots: all(
                snapshot is not None and len(snapshot["players"]) == 2 for snapshot in snapshots
            ),
            SNAPSHOT_WAIT_SECONDS,
            "simultaneous join snapshots",
        )
        snapshot_a, snapshot_b = snapshots
        assert snapshot_a is not None and snapshot_b is not None
        a_id = snapshot_a["your_id"]
        b_id = snapshot_b["your_id"]
        assert_true(a_id != b_id, "Simultaneous join reused the same player id")
        expect_player_state(snapshot_a, a_id, "green", "cube")
        expect_player_state(snapshot_a, b_id, "blue", "toka")
        expect_player_state(snapshot_b, a_id, "green", "cube")
        expect_player_state(snapshot_b, b_id, "blue", "toka")
        return ScenarioResult(
            name="M2 simultaneous join",
            details=f"clients {a_id}/{b_id} joined back-to-back without id or spawn confusion",
        )
    finally:
        client_a.close()
        client_b.close()


def scenario_disconnect_and_reconnect(server: ServerHandle) -> ScenarioResult:
    client_a = ProtocolClient(server.server_addr)
    client_b = ProtocolClient(server.server_addr)
    try:
        client_a.join("green", "ipfs")
        client_b.join("blue", "wang")
        snapshots = pump_until(
            [client_a, client_b],
            lambda snapshots: all(
                snapshot is not None and len(snapshot["players"]) == 2 for snapshot in snapshots
            ),
            SNAPSHOT_WAIT_SECONDS,
            "disconnect scenario initial snapshots",
        )
        snapshot_a, snapshot_b = snapshots
        assert snapshot_a is not None and snapshot_b is not None
        a_id = snapshot_a["your_id"]
        b_id = snapshot_b["your_id"]

        client_b.close()
        snapshots = pump_until(
            [client_a],
            lambda snapshots: snapshots[0] is not None and len(snapshots[0]["players"]) == 1,
            PLAYER_TIMEOUT_SECONDS + 2.0,
            "timed-out remote removal after disconnect",
        )
        snapshot_after_timeout = snapshots[0]
        assert snapshot_after_timeout is not None
        players_after_timeout = player_map(snapshot_after_timeout)
        assert_true(b_id not in players_after_timeout, "Timed-out player still present in snapshots")

        client_b_reconnect = ProtocolClient(server.server_addr)
        try:
            client_b_reconnect.join("blue", "wang")
            reconnect_snapshots = pump_until(
                [client_a, client_b_reconnect],
                lambda snapshots: all(
                    snapshot is not None and len(snapshot["players"]) == 2 for snapshot in snapshots
                ),
                SNAPSHOT_WAIT_SECONDS,
                "reconnect as a new player",
            )
            reconnect_snapshot_a, reconnect_snapshot_b = reconnect_snapshots
            assert reconnect_snapshot_a is not None and reconnect_snapshot_b is not None
            new_b_id = reconnect_snapshot_b["your_id"]
            assert_true(new_b_id != b_id, "Reconnect reused the old player id unexpectedly")
            expect_player_state(reconnect_snapshot_a, a_id, "green", "ipfs")
            expect_player_state(reconnect_snapshot_a, new_b_id, "blue", "wang")
            return ScenarioResult(
                name="M3 disconnect and reconnect",
                details=(
                    f"client {b_id} timed out cleanly; reconnect from a new UDP endpoint became "
                    f"new player {new_b_id}"
                ),
            )
        finally:
            client_b_reconnect.close()
    finally:
        client_a.close()


def scenario_server_restart(repo_root: Path, server_addr: tuple[str, int]) -> ScenarioResult:
    server = ServerHandle(repo_root, server_addr)
    client = ProtocolClient(server_addr)
    try:
        server.start()
        client.join("blue", "toka")
        snapshot = pump_until(
            [client],
            lambda snapshots: snapshots[0] is not None and len(snapshots[0]["players"]) == 1,
            SNAPSHOT_WAIT_SECONDS,
            "initial join before restart",
        )[0]
        assert snapshot is not None
        first_id = snapshot["your_id"]
        expect_player_state(snapshot, first_id, "blue", "toka")

        server.stop()
        time.sleep(0.5)
        server.start()

        snapshot_after_restart = pump_until(
            [client],
            lambda snapshots: snapshots[0] is not None and len(snapshots[0]["players"]) == 1,
            SNAPSHOT_WAIT_SECONDS,
            "fresh snapshot after server restart",
        )[0]
        assert snapshot_after_restart is not None
        restart_id = snapshot_after_restart["your_id"]
        default_player = expect_player_state(snapshot_after_restart, restart_id, "green", "ipfs")
        assert_true(default_player["team"] == "green", "Post-restart player did not reset to default team")

        client.join("blue", "toka")
        restored_snapshot = pump_until(
            [client],
            lambda snapshots: snapshots[0] is not None
            and player_map(snapshots[0]).get(snapshots[0]["your_id"], {}).get("team") == "blue",
            SNAPSHOT_WAIT_SECONDS,
            "team restore after rejoin on restarted server",
        )[0]
        assert restored_snapshot is not None
        expect_player_state(restored_snapshot, restored_snapshot["your_id"], "blue", "toka")

        return ScenarioResult(
            name="M4 server restart",
            details=(
                f"client recovered as a fresh session after restart (id {first_id} -> {restart_id}) "
                "and restored team/character after resending Join"
            ),
        )
    finally:
        client.close()
        server.stop()


def scenario_four_clients(server: ServerHandle) -> ScenarioResult:
    clients = [ProtocolClient(server.server_addr) for _ in range(4)]
    team_character_pairs = [
        ("green", "ipfs"),
        ("blue", "wang"),
        ("green", "cube"),
        ("blue", "toka"),
    ]
    try:
        for client, (team, character) in zip(clients, team_character_pairs):
            client.join(team, character)
        snapshots = pump_until(
            clients,
            lambda snapshots: all(
                snapshot is not None and len(snapshot["players"]) == 4 for snapshot in snapshots
            ),
            SNAPSHOT_WAIT_SECONDS,
            "four clients in one session",
        )
        seen_ids = set()
        for client, snapshot, (team, character) in zip(clients, snapshots, team_character_pairs):
            assert snapshot is not None
            your_id = snapshot["your_id"]
            assert_true(your_id not in seen_ids, f"Duplicate player id {your_id} in four-client test")
            seen_ids.add(your_id)
            expect_player_state(snapshot, your_id, team, character)
        return ScenarioResult(
            name="M5 four clients",
            details=f"four concurrent clients received unique ids and consistent 4-player snapshots: {sorted(seen_ids)}",
        )
    finally:
        for client in clients:
            client.close()


def scenario_repeated_join(server: ServerHandle) -> ScenarioResult:
    client = ProtocolClient(server.server_addr)
    try:
        client.join("green", "ipfs")
        initial = pump_until(
            [client],
            lambda snapshots: snapshots[0] is not None and len(snapshots[0]["players"]) == 1,
            SNAPSHOT_WAIT_SECONDS,
            "initial repeated-join setup",
        )[0]
        assert initial is not None
        player_id = initial["your_id"]
        expect_player_state(initial, player_id, "green", "ipfs")

        client.join("blue", "wang")
        updated = pump_until(
            [client],
            lambda snapshots: snapshots[0] is not None
            and player_map(snapshots[0]).get(snapshots[0]["your_id"], {}).get("team") == "blue",
            SNAPSHOT_WAIT_SECONDS,
            "repeat join overwrite",
        )[0]
        assert updated is not None
        expect_player_state(updated, player_id, "blue", "wang")
        return ScenarioResult(
            name="Repeated Join",
            details=f"player {player_id} kept the same id and the last Join won for team/character",
        )
    finally:
        client.close()


def run_with_server(repo_root: Path, name: str, scenario_func) -> ScenarioResult:
    server = ServerHandle(repo_root, SERVER_ADDR)
    try:
        server.start()
        result = scenario_func(server)
        return result
    finally:
        server.stop()


def main() -> int:
    repo_root = Path(__file__).resolve().parents[1]
    results: list[ScenarioResult] = []
    try:
        results.append(run_with_server(repo_root, "M1", scenario_sequential_join))
        results.append(run_with_server(repo_root, "M2", scenario_simultaneous_join))
        results.append(run_with_server(repo_root, "Repeated Join", scenario_repeated_join))
        results.append(run_with_server(repo_root, "M3", scenario_disconnect_and_reconnect))
        results.append(scenario_server_restart(repo_root, SERVER_ADDR))
        results.append(run_with_server(repo_root, "M5", scenario_four_clients))
    except Exception as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1

    summary = {
        "overall": "PASS",
        "scenarios": [{"name": result.name, "details": result.details} for result in results],
        "server_addr": f"{SERVER_ADDR[0]}:{SERVER_ADDR[1]}",
    }
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
