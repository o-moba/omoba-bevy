#!/usr/bin/env python3
"""Capture actual Bevy overview/river/sanctuary/follow frames using isolated native binaries.

Example: python3 scripts/capture_verdant.py --package /tmp/omoba-rc2 --output /tmp/verdant-qa
No source working directory, user configuration, CUA, or additional dependency is required.
This is a bounded automated renderer scenario, not an interactive gameplay certification.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import socket
import struct
import subprocess
import tempfile
import time


EXPECTED_IMAGES = ("01-overview.png", "03-river-gameplay.png", "02-sanctuary.png", "04-follow-gameplay.png")
FRAME_HEADER = struct.Struct("<4sHQQHHI")


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


class ScenarioPeer:
    """Normal admitted UDP peer; never uses god mode, teleport or server cheats."""
    def __init__(self, address, index):
        self.socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.socket.connect(address)
        self.socket.setblocking(False)
        self.index = index
        self.last_hello = -1.0
        self.pending = {}
        self.own = None
        self.snapshot_tick = 0
        self.joined = False
        self.target = ((-4.0, -3.0), (7.0, 5.0), (-7.0, -6.0), (10.0, 7.0))[index]

    def send(self, data):
        self.socket.send(json.dumps(data, separators=(",", ":")).encode())

    def update(self, now):
        if now - self.last_hello >= 1.0:
            self.last_hello = now
            self.send(dict(type="hello", protocol_version=1))
            if not self.joined:
                self.send(dict(type="join", team="green" if self.index % 2 == 0 else "blue",
                               character="cube", hero_class="warrior", avatar="agnes",
                               session_id=f"verdant-qa-{os.getpid()}-{self.index}"))
        self.pending = {key: value for key, value in self.pending.items() if now - value[0] < 2.0}
        while True:
            try:
                data = self.socket.recv(65536)
            except BlockingIOError:
                break
            if data.startswith(b"OMB1"):
                if len(data) < FRAME_HEADER.size or len(data) > 1200:
                    raise RuntimeError("invalid native frame size")
                _, version, epoch, tick, index, count, total = FRAME_HEADER.unpack_from(data)
                if version != 1 or not 0 < count <= 56 or not index < count or total > 65507:
                    raise RuntimeError("invalid native frame header")
                key = (epoch, tick)
                if key not in self.pending and len(self.pending) >= 4:
                    del self.pending[min(self.pending, key=lambda old: self.pending[old][0])]
                assembly = self.pending.setdefault(key, (now, {}))[1]
                assembly[index] = data[FRAME_HEADER.size:]
                if len(assembly) != count:
                    continue
                data = b"".join(assembly[index] for index in range(count))
                del self.pending[key]
                if len(data) != total:
                    raise RuntimeError("incomplete native frame reconstruction")
            snapshot = json.loads(data)
            tick = snapshot.get("snapshot_tick", 0)
            if tick <= self.snapshot_tick:
                continue
            self.snapshot_tick = tick
            self.own = next((player for player in snapshot.get("players", [])
                             if player["id"] == snapshot.get("your_id")), None)
            self.joined = self.own is not None
        if self.own and self.own.get("hp", 0) > 0:
            dx, dz = self.target[0] - self.own["x"], self.target[1] - self.own["z"]
            distance = math.hypot(dx, dz)
            step = min(distance, 0.15)  # <=3 u/s at the 50 ms update interval.
            if distance > 0.1:
                self.send(dict(type="transform", x=self.own["x"] + dx / distance * step,
                               y=self.own.get("y", 0.5), z=self.own["z"] + dz / distance * step,
                               yaw=math.atan2(dx, dz)))

    def close(self):
        self.socket.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", type=Path)
    parser.add_argument("--client-bin", type=Path)
    parser.add_argument("--server-bin", type=Path)
    parser.add_argument("--assets", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=300)
    parser.add_argument("--bots", type=int, choices=range(5), default=2)
    args = parser.parse_args()
    package = args.package.resolve() if args.package else None
    suffix = ".exe" if os.name == "nt" else ""
    client = args.client_bin or (package / ("client" + suffix) if package else None)
    server = args.server_bin or (package / ("server" + suffix) if package else None)
    assets = args.assets or (package / "assets" if package else None)
    if not client or not server or not assets:
        parser.error("provide --package or explicit --client-bin, --server-bin and --assets")
    client, server, assets = client.resolve(), server.resolve(), assets.resolve()
    if not client.is_file() or not server.is_file() or not assets.is_dir():
        parser.error("both binaries and the explicit asset directory must exist")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if any((output / name).exists() for name in (*EXPECTED_IMAGES, "capture-run.json")):
        parser.error("output already has captures; choose a new directory to preserve evidence")
    timeout = min(660, max(60, args.timeout))
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as reservation:
        reservation.bind(("127.0.0.1", 0))
        address = reservation.getsockname()
    children, peers = [], []
    result = dict(platform=platform.platform(), machine=platform.machine(),
                  client=str(client), server=str(server), assets=str(assets),
                  binary_sha256=dict(client=sha256(client), server=sha256(server)),
                  source_identity=json.loads((package / "BUILD.json").read_text())
                  if package and (package / "BUILD.json").exists() else None,
                  capture_method="Bevy Screenshot::primary_window + save_to_disk",
                  manual_interaction_verified=False, timeout_seconds=timeout,
                  scripted_peers=args.bots, server_mode="dev; production commands; no cheats")
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="omoba-verdant-capture-") as isolated:
        result["isolated_cwd"] = isolated
        env = dict(os.environ, SERVER_ADDR=f"{address[0]}:{address[1]}",
                   GAME_SERVER_ADDR=f"{address[0]}:{address[1]}",
                   OMOBA_CLIENT_CONFIG_DIR=str(Path(isolated) / "config"),
                   OMOBA_ASSET_DIR=str(assets), OMOBA_MATCH_MODE="dev", OMOBA_TEAM_SIZE="5",
                   OMOBA_PLAYER_VISUAL_MODE="models3d", OMOBA_DEBUG_UI="0",
                   OMOBA_VISUAL_QA_DIR=str(output), OMOBA_VISUAL_QA_TIMEOUT=str(timeout - 20))
        for key in ("OMOBA_AUTOJOIN", "OMOBA_MEASURE_MODELS", "OMOBA_AVATAR_MANIFEST"):
            env.pop(key, None)
        try:
            with (output / "server.log").open("w") as log:
                server_process = subprocess.Popen([str(server)], cwd=isolated, env=env,
                                                  stdout=log, stderr=subprocess.STDOUT)
            children.append(server_process)
            deadline = time.monotonic() + 15
            while "is listening" not in (output / "server.log").read_text(errors="replace"):
                if server_process.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError("fresh native server did not report listening")
                time.sleep(0.05)
            peers = [ScenarioPeer(address, index) for index in range(args.bots)]
            with (output / "client.log").open("w") as log:
                client_process = subprocess.Popen([str(client)], cwd=isolated, env=env,
                                                  stdout=log, stderr=subprocess.STDOUT)
            children.append(client_process)
            while client_process.poll() is None and time.monotonic() - started < timeout:
                if server_process.poll() is not None:
                    raise RuntimeError("native server exited during capture")
                for peer in peers:
                    peer.update(time.monotonic())
                time.sleep(0.05)
            result["client_exit_code"] = client_process.poll()
            result["timed_out"] = client_process.poll() is None
        except Exception as error:
            result["error"] = str(error)
        finally:
            for peer in peers:
                peer.close()
            for child in reversed(children):
                if child.poll() is None:
                    child.terminate()
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
    client_log = (output / "client.log").read_text(errors="replace") if (output / "client.log").exists() else ""
    result["elapsed_seconds"] = time.monotonic() - started
    result["images"] = {name: dict(bytes=(output / name).stat().st_size, sha256=sha256(output / name))
                        for name in EXPECTED_IMAGES if (output / name).is_file()}
    result["snapshot_received"] = "First snapshot received" in client_log
    result["asset_root_confirmed"] = str(assets) in client_log
    result["errors"] = [line for line in client_log.splitlines() if any(text in line for text in
                         ("panicked at", "does not exist", "Path not found", "Downloading model", "VERDANT_QA failed"))]
    result["capture_pass"] = (result.get("client_exit_code") == 0 and not result.get("error")
                              and len(result["images"]) == len(EXPECTED_IMAGES)
                              and (output / "qa-summary.json").is_file()
                              and result["snapshot_received"] and result["asset_root_confirmed"]
                              and not result["errors"])
    (output / "capture-run.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    raise SystemExit(0 if result["capture_pass"] else 1)


if __name__ == "__main__":
    main()
