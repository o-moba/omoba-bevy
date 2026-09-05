#!/usr/bin/env python3
"""Read-only protocol audit: owned release 5v5 process and loopback clients.

No builds, production edits, socket-buffer changes or sysctl writes. A completed
observation is success even if no full Running snapshot arrives. JSON includes
the server's own stderr and a separate native loopback UDP size-boundary probe.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import select
import socket
import subprocess
import tempfile
import time


def udp_boundary_observations() -> list:
    observations = []
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as receiver:
        receiver.bind(("127.0.0.1", 0))
        receiver.settimeout(0.3)
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sender:
            for size in (8192, 9216, 9217, 12_000, 65_507):
                row = {"requested_bytes": size}
                try:
                    row["sent_bytes"] = sender.sendto(b"x" * size, receiver.getsockname())
                    row["received_bytes"] = len(receiver.recv(65_536))
                except OSError as error:
                    row.update(errno=error.errno, error=str(error))
                observations.append(row)
    return observations


def summary(raw: bytes, packet: dict, elapsed: float) -> dict:
    result = {
        "elapsed_seconds": round(elapsed, 3), "bytes": len(raw),
        "your_id": packet.get("your_id"), "game_state": packet.get("game_state"),
    }
    for key in ("players", "structures", "minions", "projectiles", "neutrals"):
        result[key] = len(packet.get(key, []))
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[4])
    parser.add_argument("--server-bin", type=Path)
    parser.add_argument("--output", type=Path,
                        default=Path(__file__).with_name("5v5-observations.json"))
    args = parser.parse_args()
    repo = args.repo.resolve()
    binary = (args.server_bin or repo / "target/debug/server").resolve()
    report = {"probe": "release-5v5-datagrams", "status": "PROBE_FAILED",
              "repo": str(repo), "server_binary": str(binary)}
    process = None
    clients = []
    exit_code = 1
    started = time.monotonic()
    with tempfile.TemporaryFile(mode="w+b") as log:
        try:
            digest = hashlib.sha256()
            with binary.open("rb") as stream:
                for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                    digest.update(chunk)
            report["server_sha256"] = digest.hexdigest()
            host_limits = subprocess.run(
                ["sysctl", "net.inet.udp.maxdgram", "net.inet.udp.recvspace"],
                text=True, capture_output=True, check=False,
            )
            report["read_only_sysctl"] = {
                "exit_code": host_limits.returncode,
                "stdout": host_limits.stdout.splitlines(), "stderr": host_limits.stderr.splitlines(),
            }
            report["native_loopback_udp_boundary"] = udp_boundary_observations()
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as reservation:
                reservation.bind(("127.0.0.1", 0))
                address = reservation.getsockname()
            env = os.environ.copy()
            env.update(SERVER_ADDR=f"{address[0]}:{address[1]}",
                       OMOBA_MATCH_MODE="release", OMOBA_TEAM_SIZE="5")
            process = subprocess.Popen([str(binary)], cwd=repo, env=env,
                                       stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT)
            report.update(server_pid=process.pid, server_address=f"{address[0]}:{address[1]}")
            for _ in range(10):
                client = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                client.bind(("127.0.0.1", 0))
                client.connect(address)
                client.setblocking(False)
                clients.append(client)
            stats = [{"client_index": i, "datagrams": 0, "max_bytes": 0,
                      "last_snapshot": None} for i in range(10)]
            report["clients"] = stats
            report["snapshot_shape_changes"] = []
            report["largest_received_snapshot"] = None
            report["qualifying_running_snapshots"] = 0
            next_ping = 0.0
            join_at = None
            last_shape = None
            ready_deadline = time.monotonic() + 5.0
            deadline = ready_deadline
            ping = b'{"type":"ping"}'
            join = json.dumps({
                "type": "join", "team": "green", "character": "ipfs", "hero_class": "warrior",
                "avatar": "osa-kardialtheconsumer-00bea9121db1",
                "sprite_character": "cathedral-moth-bellringer",
            }).encode()
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    raise RuntimeError(f"server exited before observation completed: {process.returncode}")
                now = time.monotonic()
                if now >= next_ping:
                    for client in clients:
                        try:
                            client.send(ping)
                        except ConnectionRefusedError:
                            pass
                    next_ping = now + 0.3
                readable, _, _ = select.select(clients, [], [], 0.05)
                for client in readable:
                    index = clients.index(client)
                    while True:
                        try:
                            raw = client.recv(65_536)
                        except (BlockingIOError, ConnectionRefusedError):
                            break
                        packet = json.loads(raw)
                        if packet.get("type") != "snapshot":
                            continue
                        row = summary(raw, packet, time.monotonic() - started)
                        stats[index]["datagrams"] += 1
                        stats[index]["max_bytes"] = max(stats[index]["max_bytes"], len(raw))
                        stats[index]["last_snapshot"] = row
                        largest = report["largest_received_snapshot"]
                        if largest is None or row["bytes"] > largest["bytes"]:
                            report["largest_received_snapshot"] = row
                        shape = (row["game_state"]["type"], row["players"], row["structures"], row["minions"])
                        if index == 0 and shape != last_shape:
                            report["snapshot_shape_changes"].append(row)
                            last_shape = shape
                        if shape == ("running", 10, 8, 18) and len(raw) > 8192:
                            report["qualifying_running_snapshots"] += 1
                if join_at is None and all(stat["datagrams"] for stat in stats):
                    for client in clients:
                        client.send(join)
                    join_at = time.monotonic()
                    deadline = join_at + 15.0
                    report["join_sent_at_seconds"] = round(join_at - started, 3)
            if join_at is None:
                raise RuntimeError("server did not send initial snapshots to all ten clients within five seconds")
            report["status"] = "OBSERVATIONS_COMPLETED"
            exit_code = 0
        except Exception as error:
            report["error"] = f"{type(error).__name__}: {error}"
        finally:
            for client in clients:
                client.close()
            if process is not None:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=3.0)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=3.0)
                report["owned_server_exit_code"] = process.returncode
            log.seek(0)
            report["server_log"] = log.read().decode("utf-8", errors="replace").splitlines()
    report["elapsed_seconds"] = round(time.monotonic() - started, 3)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"status": report["status"], "output": str(args.output),
                      "largest_received_snapshot": report.get("largest_received_snapshot"),
                      "qualifying_running_snapshots": report.get("qualifying_running_snapshots"),
                      "error": report.get("error")}, indent=2))
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
