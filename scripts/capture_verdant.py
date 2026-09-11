#!/usr/bin/env python3
"""Capture actual Bevy Verdant views or beta UI using isolated native binaries.

Example: python3 scripts/capture_verdant.py --package /tmp/omoba-rc2 --output /tmp/verdant-qa
Use --scenario beta-ui --bots 0 for 1280x720 entry/help/gameplay/result layouts.
Use --scenario navigation --bots 0 --timeout 110 for genuine scripted RMB input,
route detour/arrival and modal isolation, with an independent snapshot observer.
The beta result is a labeled synthetic snapshot, never complete-match evidence.
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


EXPECTED_IMAGES = ("01-overview.png", "03-river-gameplay.png", "02-sanctuary.png", "04-follow-gameplay.png", "05-orbit-zoom-gameplay.png")
BETA_IMAGES = ("01-entry-720p.png", "02-help-720p.png", "03-gameplay-720p.png", "04-shop-720p.png", "05-purchase-720p.png", "06-shop-closed-720p.png", "07-result-fixture-720p.png")
NAVIGATION_IMAGES = ("01-minimap-route.png", "02-minimap-arrival.png", "03-world-route.png", "04-navigation-complete.png")
FRAME_HEADER = struct.Struct("<4sHQQHHI")


def verify_beta_ui_profile(summary, requested_touch_controls, expected_images):
    """Require actual profile evidence for every expected beta UI capture stage."""
    requested = "mobile" if requested_touch_controls else "desktop"
    errors, stages, seen, actual_values = [], [], set(), set()
    captures = summary.get("captures") if isinstance(summary, dict) else None
    if not isinstance(captures, list):
        errors.append("qa-summary.json must contain a captures array with UI profile evidence.")
        captures = []
    for index, capture in enumerate(captures):
        if not isinstance(capture, dict):
            errors.append(f"Capture record {index} must be an object.")
            continue
        stage, filename, mobile = (capture.get(key) for key in ("stage", "file", "mobile_controls"))
        valid_mobile = type(mobile) is bool
        stages.append(dict(stage=stage, file=filename, mobile_controls=mobile,
                           actual_profile=("mobile" if mobile else "desktop") if valid_mobile else "unknown"))
        if type(stage) is not int or not 0 <= stage < len(expected_images):
            errors.append(f"Capture record {index} has an invalid stage: {stage!r}.")
        else:
            if stage in seen:
                errors.append(f"Duplicate capture stage {stage}.")
            seen.add(stage)
            if filename != expected_images[stage]:
                errors.append(f"Stage {stage} must reference {expected_images[stage]!r}, got {filename!r}.")
        if not valid_mobile:
            errors.append(f"Capture record {index} must have a boolean mobile_controls value.")
        else:
            actual_values.add(mobile)
            if mobile != requested_touch_controls:
                errors.append(f"Stage {stage!r} reports {'mobile' if mobile else 'desktop'} UI; requested {requested} UI.")
    missing = sorted(set(range(len(expected_images))) - seen)
    if missing:
        errors.append(f"Missing UI profile evidence for capture stages {missing}.")
    if actual_values - {requested_touch_controls}:
        errors.append("Desktop mobile preview requires a development/debug client build: point --client-bin "
                      "at that build and use --touch-controls. Desktop release builds ignore OMOBA_TOUCH_CONTROLS; "
                      "omit --touch-controls to capture their desktop UI. Native mobile builds always use mobile UI.")
    actual = ("mobile" if True in actual_values else "desktop") if len(actual_values) == 1 else (
        "mixed" if actual_values else "unknown")
    return {"pass": not errors, "requested_profile": requested, "requested_touch_controls": requested_touch_controls,
            "actual_profile": actual, "expected_stages": len(expected_images), "stages": stages, "errors": errors}


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
            self.send(dict(type="hello", protocol_version=2))
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
                if version != 2 or not 0 < count <= 56 or not index < count or total > 65507:
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


class SnapshotObserver:
    """Hello-only endpoint. Receives authoritative snapshots without joining."""
    def __init__(self, address, output):
        self.socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.socket.connect(address)
        self.socket.setblocking(False)
        self.last_hello = -1.0
        self.pending = {}
        self.samples = []
        self.output = output.open("w")

    def update(self, now):
        if now - self.last_hello >= 1.0:
            self.last_hello = now
            self.socket.send(b'{"type":"hello","protocol_version":2}')
        self.pending = {key: value for key, value in self.pending.items() if now - value[0] < 2.0}
        while True:
            try:
                data = self.socket.recv(65536)
            except BlockingIOError:
                break
            if data.startswith(b"OMB1"):
                if len(data) < FRAME_HEADER.size or len(data) > 1200:
                    raise RuntimeError("invalid observer snapshot frame size")
                _, version, epoch, tick, index, count, total = FRAME_HEADER.unpack_from(data)
                if version != 2 or not 0 < count <= 56 or not index < count or total > 65507:
                    raise RuntimeError("invalid observer snapshot frame header")
                key = (epoch, tick)
                if key not in self.pending and len(self.pending) >= 4:
                    del self.pending[min(self.pending, key=lambda old: self.pending[old][0])]
                chunks = self.pending.setdefault(key, (now, {}))[1]
                chunks[index] = data[FRAME_HEADER.size:]
                if len(chunks) != count:
                    continue
                data = b"".join(chunks[index] for index in range(count))
                del self.pending[key]
                if len(data) != total:
                    raise RuntimeError("incomplete observer snapshot reconstruction")
            packet = json.loads(data)
            if packet.get("type") != "snapshot":
                continue
            sample = {key: packet.get(key) for key in ("server_epoch", "round_id", "snapshot_tick", "players", "structures")}
            self.samples.append(sample)
            self.output.write(json.dumps(sample, separators=(",", ":")) + "\n")
            self.output.flush()

    def close(self):
        self.socket.close()
        self.output.close()


def verify_navigation(summary, snapshots, collision_document=None):
    """Require input telemetry plus independent server-side arrival and detour."""
    player_id = summary.get("player_id")
    samples = [(snapshot["snapshot_tick"], player) for snapshot in snapshots
               for player in snapshot.get("players", []) if player["id"] == player_id]
    events = {event["event"]: event for event in summary.get("events", [])}
    required = {"minimap_rmb_input", "minimap_order_accepted", "minimap_arrival", "world_rmb_input",
                "world_order_accepted", "world_arrival", "alt_world_blocked", "alt_minimap_blocked",
                "shop_world_blocked", "shop_minimap_blocked", "pause_world_blocked",
                "pause_minimap_blocked", "lmb_minimap_pan"}
    arrivals = {}
    for name in ("minimap_arrival", "world_arrival"):
        event = events.get(name)
        if event is None:
            continue
        target = event["detail"]["destination"]
        # Client waits 800ms here so several independently observed snapshots
        # must confirm the stopped target before the next real input order.
        distances = [math.hypot(player["x"] - target[0], player["z"] - target[2])
                     for tick, player in samples
                     if event["snapshot_tick"] <= tick <= event["snapshot_tick"] + 12 and player["hp"] > 0]
        arrivals[name] = dict(nearby_snapshots=sum(distance <= 0.7 for distance in distances),
                              minimum_distance=min(distances, default=None))
    obstacle = summary.get("obstacle", {})
    center = obstacle.get("center", [0, 0, 0])
    radius = obstacle.get("collision_radius", 3.7)
    first_tick = events.get("minimap_rmb_input", {}).get("snapshot_tick", 0)
    last_tick = events.get("minimap_arrival", {}).get("snapshot_tick", 0)
    travel = [player for tick, player in sorted(samples, key=lambda sample: sample[0]) if first_tick <= tick <= last_tick]
    traversal_length = sum(math.hypot(end["x"] - begin["x"], end["z"] - begin["z"])
                           for begin, end in zip(travel, travel[1:]))
    clearance = min((math.hypot(player["x"] - center[0], player["z"] - center[2]) for player in travel), default=None)
    start = summary.get("start", [0, 0, 0])
    waypoints = events.get("minimap_order_accepted", {}).get("detail", {}).get("waypoints", [])
    planned_length = (sum(math.hypot(end[0] - begin[0], end[2] - begin[2])
                          for begin, end in zip([start] + waypoints, waypoints)) if waypoints else None)
    target = events.get("minimap_rmb_input", {}).get("detail", {}).get("destination", start)
    dx, dz = target[0] - start[0], target[2] - start[2]
    length = math.hypot(dx, dz)
    detour = max((abs(dx * (player["z"] - start[2]) - dz * (player["x"] - start[0])) / length
                  for player in travel if length > 0.0), default=0.0)
    result = dict(observer="hello-only UDP snapshots; no Join or Transform commands", player_id=player_id,
                  authoritative_samples=len(samples), arrivals=arrivals, minimum_structure_distance=clearance,
                  collision_radius=radius, maximum_deviation_from_direct_line=detour,
                  planned_route_length=planned_length, authoritative_traversal_length=traversal_length,
                  missing_input_events=sorted(required - events.keys()))
    result["pass"] = (summary.get("pass") is True and summary.get("scripted_input") is True
                      and not result["missing_input_events"] and len(arrivals) == 2
                      and all(value["nearby_snapshots"] >= 2 for value in arrivals.values())
                      and clearance is not None and clearance >= radius - 0.1 and detour >= radius - 0.3
                      and planned_length is not None and planned_length < max(25.0, length * 1.8)
                      and traversal_length <= max(30.0, length * 2.0))
    if obstacle.get("kind") == "tree":
        # Independently test continuous authoritative segments against the
        # versioned polygons. Client route/collision claims cannot replace this.
        from generate_verdant_collision import polygon_segment_distance
        collision_document = collision_document or json.loads(
            (Path(__file__).resolve().parents[1] / "shared/assets/verdant-collision.json").read_text())
        polygons = collision_document["obstacles"]
        selected = next((item for item in polygons if item["id"] == obstacle.get("id")), None)
        minimum = min((polygon_segment_distance(item["vertices"], (a["x"], a["z"]), (b["x"], b["z"]))
                       for a,b in zip(travel, travel[1:]) for item in polygons), default=None)
        direct_blocked = selected is not None and polygon_segment_distance(
            selected["vertices"], (start[0],start[2]), (target[0],target[2])) < 0.5
        captures = [event for event in summary.get("events", [])
                    if event["event"] == "travel_capture_after_settle"]
        minimap_visible = len(captures) == 2 and all(
            event["detail"].get("minimap", {}).get("route_segments")
            and event["detail"]["minimap"].get("route_destination") for event in captures)
        minimap_cleared = all(not events[name]["detail"].get("minimap", {}).get("route_segments")
                              and not events[name]["detail"].get("minimap", {}).get("route_destination")
                              for name in ("minimap_arrival", "world_arrival") if name in events)
        result["forest"] = dict(selected_id=obstacle.get("id"), obstacles=len(polygons),
                                minimum_polygon_clearance=minimum, direct_line_blocked=direct_blocked,
                                minimap_visible=minimap_visible, minimap_cleared=minimap_cleared)
        result["pass"] = bool(result["pass"] and minimum is not None and minimum >= 0.49
                              and direct_blocked and minimap_visible and minimap_cleared
                              and summary.get("route_display") == "minimap_only"
                              and "forest_approach_input" in events)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", type=Path)
    parser.add_argument("--client-bin", type=Path)
    parser.add_argument("--server-bin", type=Path)
    parser.add_argument("--assets", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=300)
    parser.add_argument("--team", choices=("green", "blue"), default="green")
    parser.add_argument("--width", type=int, default=1280)
    parser.add_argument("--height", type=int, default=720)
    parser.add_argument("--touch-controls", action="store_true", help="Preview phone UI at the requested viewport; requires a desktop development build")
    parser.add_argument("--scenario", choices=("verdant", "beta-ui", "navigation"), default="verdant")
    parser.add_argument("--bots", type=int, choices=range(5), default=2)
    args = parser.parse_args()
    if not (320 <= args.width <= 3840 and 320 <= args.height <= 2160):
        parser.error("viewport must be 320..3840 wide and 320..2160 high")
    expected_images = (tuple(name.replace("720p", f"{args.height}p") for name in BETA_IMAGES) if args.scenario == "beta-ui"
                       else NAVIGATION_IMAGES if args.scenario == "navigation" else EXPECTED_IMAGES)
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
    if any((output / name).exists() for name in (*expected_images, "capture-run.json")):
        parser.error("output already has captures; choose a new directory to preserve evidence")
    timeout = min(660, max(60, args.timeout))
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as reservation:
        reservation.bind(("127.0.0.1", 0))
        address = reservation.getsockname()
    children, peers = [], []
    observer = None
    result = dict(platform=platform.platform(), machine=platform.machine(),
                  client=str(client), server=str(server), assets=str(assets),
                  binary_sha256=dict(client=sha256(client), server=sha256(server)),
                  source_identity=json.loads((package / "BUILD.json").read_text())
                  if package and (package / "BUILD.json").exists() else None,
                  capture_method="Bevy Screenshot::primary_window + save_to_disk",
                  manual_interaction_verified=False, timeout_seconds=timeout,
                  pixels=[args.width, args.height] if args.scenario in ("beta-ui", "navigation") else None, team=args.team, scenario=args.scenario, result_fixture=args.scenario == "beta-ui",
                  scripted_peers=args.bots, server_mode="dev; production commands; no cheats")
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="omoba-verdant-capture-") as isolated:
        result["isolated_cwd"] = isolated
        env = dict(os.environ, SERVER_ADDR=f"{address[0]}:{address[1]}",
                   GAME_SERVER_ADDR=f"{address[0]}:{address[1]}",
                   OMOBA_CLIENT_CONFIG_DIR=str(Path(isolated) / "config"),
                   OMOBA_ASSET_DIR=str(assets), OMOBA_MATCH_MODE="dev", OMOBA_TEAM_SIZE="5",
                   OMOBA_PLAYER_VISUAL_MODE="models3d", OMOBA_DEBUG_UI="0",
                   OMOBA_QA_TEAM=args.team, OMOBA_QA_WIDTH=str(args.width), OMOBA_QA_HEIGHT=str(args.height),
                   OMOBA_VISUAL_QA_DIR=str(output), OMOBA_VISUAL_QA_SCENARIO=args.scenario, OMOBA_VISUAL_QA_TIMEOUT=str(timeout - 20))
        env["OMOBA_TOUCH_CONTROLS"] = "1" if args.touch_controls else "0"
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
            if args.scenario == "navigation":
                observer = SnapshotObserver(address, output / "authoritative-snapshots.jsonl")
            with (output / "client.log").open("w") as log:
                client_process = subprocess.Popen([str(client)], cwd=isolated, env=env,
                                                  stdout=log, stderr=subprocess.STDOUT)
            children.append(client_process)
            while client_process.poll() is None and time.monotonic() - started < timeout:
                if server_process.poll() is not None:
                    raise RuntimeError("native server exited during capture")
                for peer in peers:
                    peer.update(time.monotonic())
                if observer:
                    observer.update(time.monotonic())
                time.sleep(0.05)
            result["client_exit_code"] = client_process.poll()
            result["timed_out"] = client_process.poll() is None
        except Exception as error:
            result["error"] = str(error)
        finally:
            for peer in peers:
                peer.close()
            if observer:
                observer.close()
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
                        for name in expected_images if (output / name).is_file()}
    result["snapshot_received"] = "First snapshot received" in client_log
    result["asset_root_confirmed"] = str(assets) in client_log
    result["errors"] = [line for line in client_log.splitlines() if any(text in line for text in
                         ("panicked at", "does not exist", "Path not found", "Downloading model", "VERDANT_QA failed", "BETA_UI_QA failed", "NAVIGATION_QA failed"))]
    result["capture_pass"] = (result.get("client_exit_code") == 0 and not result.get("error")
                              and len(result["images"]) == len(expected_images)
                              and (output / "qa-summary.json").is_file()
                              and result["snapshot_received"] and result["asset_root_confirmed"]
                              and not result["errors"])
    if args.scenario == "beta-ui":
        try:
            summary = json.loads((output / "qa-summary.json").read_text())
            summary_error = None
        except (OSError, UnicodeError, json.JSONDecodeError) as error:
            summary, summary_error = None, str(error)
        result["ui_profile"] = verify_beta_ui_profile(summary, args.touch_controls, expected_images)
        if summary_error:
            result["ui_profile"]["errors"].append(f"Cannot read qa-summary.json: {summary_error}")
        result["errors"].extend(result["ui_profile"]["errors"])
        result["capture_pass"] = result["capture_pass"] and result["ui_profile"]["pass"]
    if args.scenario == "navigation":
        summary_path = output / "qa-summary.json"
        result["navigation"] = verify_navigation(json.loads(summary_path.read_text()) if summary_path.is_file() else {},
                                                  observer.samples if observer else [])
        result["capture_pass"] = result["capture_pass"] and result["navigation"]["pass"]
    (output / "capture-run.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    raise SystemExit(0 if result["capture_pass"] else 1)


if __name__ == "__main__":
    main()
