#!/usr/bin/env python3
"""Native map proof: original UDP receipts and actual rendered mesh buffers.

Use --map-config examples/maps/two-tier.json for the contributor example.
3D stages inspect the same non-solid prop A→B→A→B; 2D stages inspect actual
configured structure sprites. Scripted camera/cosmetic changes are declared.
No browser, fabricated gameplay state, or physical-device claims.
"""
import argparse
import copy
import json
import math
import os
from pathlib import Path
import platform
import socket
import struct
import subprocess
import sys
import tempfile
import time

from capture_combat import CombatObserver, finite_number
from capture_verdant import sha256, verify_beta_ui_profile

ROOT = Path(__file__).resolve().parent.parent
FILES_3D = ("01-map-overview.png", "02-prop-a.png", "03-prop-b.png", "04-prop-a-restored.png", "05-prop-b-repeated.png")
FILES_2D = ("01-map-overview.png", "02-tower-detail.png")
MODELS = ("map-props/lantern.glb#Scene0", "map-props/flowering_shrub.glb#Scene0")
GEOMETRY_ID = "verdant-confluence-v1"
# Pinned authored v1 geometry; bridge is terrain, not a replaceable prop.
STATIC_PROPS = {
    "environment.glb:banner_blue": 2,
    "environment.glb:banner_green": 2,
    "environment.glb:lantern": 19,
    "environment.glb:rock_stratified_outcrop": 5,
    "environment.glb:ruin_wall": 15,
    "environment.glb:ruined_arch": 3,
    "foliage.glb:boulder_moss_flat": 39,
    "foliage.glb:boulder_moss_tall": 64,
    "foliage.glb:fallen_log": 5,
    "foliage.glb:fern_cluster": 249,
    "foliage.glb:flowering_shrub": 121,
    "foliage.glb:grass_fan": 241,
    "foliage.glb:river_reeds": 60,
    "foliage.glb:rooted_stump": 4,
    "foliage.glb:tree_cypress_spire": 22,
    "foliage.glb:tree_jade_canopy": 23,
    "foliage.glb:tree_river_pine": 22,
    "foliage.glb:tree_sage_elder": 23,
    "foliage.glb:tree_windswept_oak": 23
}


def near(a, b):
    # World coordinates pass through f32 on the wire and in native ECS.
    return finite_number(a) and finite_number(b) and math.isclose(a, b, rel_tol=0, abs_tol=1e-4)


def expected_map(config):
    """Independent v1 arc-length placement mirror, not a server validator.

    The server performs full schema/collision validation. This simple mirror
    checks its externally visible placements from the saved startup JSON.
    """
    if config["format_version"] != 1 or config["geometry_id"] != GEOMETRY_ID:
        raise ValueError("Verifier only supports the fixed Verdant v1 geometry")
    half = 225 / math.sqrt(2) * .5
    edge = half + 46 * .5 + 6 - 6 - 12 * .5
    home, away = [-half, -half], [half, half]
    lanes = dict(mid=[home, away], top=[home, [-edge, -half], [-edge, edge], [edge, edge], [half, edge], away],
                 bot=[home, [-half, -edge], [-edge, -edge], [edge, -edge], [edge, half], away])

    def sample(lane, progress):
        points = lanes[lane]
        lengths = [math.dist(a, b) for a, b in zip(points, points[1:])]
        remaining = sum(lengths) * progress
        for a, b, length in zip(points, points[1:], lengths):
            if remaining <= length:
                fraction = remaining / length
                return [x + (y - x) * fraction for x, y in zip(a, b)]
            remaining -= length
        return away

    structures = []
    for item in config["structures"]:
        tower = item["kind"] == "tower"
        position = sample(item["lane"], item["t"]) if tower else (home if item["team"] == "green" else away)
        offset = item.get("offset", [0, 0]) if tower else [0, 0]
        stats = dict(config["profiles"][item["profile"]], **item.get("overrides", {}))
        tier = 0
        if tower:
            peers = sorted([other for other in config["structures"] if other["kind"] == "tower"
                            and other["team"] == item["team"] and other["lane"] == item["lane"]],
                           key=lambda other: other["t"], reverse=item["team"] == "green")
            tier = next(index for index, other in enumerate(peers) if other["id"] == item["id"])
        structures.append(dict(id=item["id"], map_key=item["key"], visual_profile=item["visual_profile"],
                               kind="tower" if tower else "base_tower", team=item["team"], lane=item.get("lane"), tier=tier,
                               position=[position[0] + offset[0], 3 if tower else 4, position[1] + offset[1]],
                               max_hp=stats["max_hp"]))
    if len({item["id"] for item in structures}) != len(structures):
        raise ValueError("Duplicate fixture IDs")
    return dict(geometry_id=config["geometry_id"], map_profile=config["map_profile"], structures=structures)


def valid_geometry(value):
    if not isinstance(value, dict):
        return False
    keys = ("vertices", "indices", "mesh_count")
    if any(type(value.get(key)) is not int or value[key] <= 0 for key in keys):
        return False
    signatures = value.get("mesh_geometry_signatures", [])
    low, high = value.get("world_min"), value.get("world_max")
    return (isinstance(signatures, list) and len(signatures) == value["mesh_count"]
            and all(isinstance(s, str) and s for s in signatures)
            and isinstance(low, list) and isinstance(high, list) and len(low) == len(high) == 3
            and all(finite_number(n) for pair in (low, high) for n in pair)
            and all(a <= b for a, b in zip(low, high)) and math.dist(low, high) > .01)


def verify_map(summary, snapshots, mobile, mode, expected):
    summary = summary if isinstance(summary, dict) else {}
    files = FILES_3D if mode == "models3d" else FILES_2D
    profile = verify_beta_ui_profile(summary, mobile, files)
    errors = list(profile["errors"])
    if summary.get("pass") is not True or summary.get("scenario") != "map":
        errors.append("Missing successful native map summary")
    for key, value in (("scripted_camera", True), ("scripted_cosmetic_registry", mode == "models3d"),
                       ("synthetic_structures", False), ("physical_device_verified", False), ("manual_interaction_verified", False)):
        if summary.get(key) is not value:
            errors.append(f"Missing precise provenance: {key}")
    for field in ("geometry_id", "map_profile"):
        if summary.get(field) != expected[field]:
            errors.append(f"Summary {field} differs from startup JSON")
    records = {frame.get("stage"): frame for frame in summary.get("captures", []) if isinstance(frame, dict)}
    expected_by_id = {item["id"]: item for item in expected["structures"]}
    matched = []
    for index, frame in records.items():
        if frame.get("visual_mode") != ("Models3d" if mode == "models3d" else "Sprite2d"):
            errors.append(f"Stage {index}: wrong actual renderer")
        if frame.get("pixels") != summary.get("pixels"):
            errors.append(f"Stage {index}: viewport differs from native pixels")
        for field in ("geometry_id", "map_profile"):
            if frame.get(field) != expected[field]:
                errors.append(f"Stage {index}: incompatible {field}")
        matching = [s for s in snapshots if s.get("server_epoch") == frame.get("server_epoch")
                    and s.get("round_id", s.get("match_id")) == frame.get("match_id")
                    and s.get("snapshot_tick") == frame.get("snapshot_tick")]
        if not matching:
            errors.append(f"Stage {index}: no independent UDP receipt for exact epoch/round/tick")
            continue
        packet = matching[-1]
        if any(packet.get(k) != expected[k] for k in ("geometry_id", "map_profile")):
            errors.append(f"Stage {index}: observer map identity differs from startup")
        server = {s.get("id"): s for s in packet.get("structures", [])}
        actual = {s.get("id"): s for s in frame.get("structures", [])}
        if set(actual) != set(expected_by_id) or set(server) != set(expected_by_id) or len(actual) != len(frame.get("structures", [])) or len(server) != len(packet.get("structures", [])):
            errors.append(f"Stage {index}: missing, duplicate or extra authoritative structure")
        for identity, wanted in expected_by_id.items():
            native, wire = actual.get(identity, {}), server.get(identity, {})
            for field in ("id", "map_key", "visual_profile", "team", "kind", "lane", "tier"):
                if native.get(field) != wanted[field] or wire.get(field) != wanted[field]:
                    errors.append(f"Stage {index} structure {identity}: wrong {field}")
            position = native.get("position", [])
            if len(position) != 3 or any(not near(a, b) or not near(b, wire.get(axis))
                                         for a, b, axis in zip(position, wanted["position"], ("x", "y", "z"))):
                errors.append(f"Stage {index} structure {identity}: position differs from config/UDP")
            if not near(native.get("max_hp"), wanted["max_hp"]) or not near(wire.get("max_hp"), wanted["max_hp"]) or not near(native.get("hp"), wire.get("hp")):
                errors.append(f"Stage {index} structure {identity}: configured max HP/current UDP HP mismatch")
            if type(native.get("protected")) is not bool or native.get("protected") != wire.get("protected"):
                errors.append(f"Stage {index} structure {identity}: protection differs from authority")
            if mode == "models3d":
                geometry = native.get("model_geometry", [])
                if len(geometry) != 1 or not valid_geometry(geometry[0]):
                    errors.append(f"Stage {index} structure {identity}: missing actual 3D mesh buffers")
            else:
                sprites = native.get("sprite_geometry", [])
                if len(sprites) != 1 or sprites[0].get("profile_key") != wanted["visual_profile"] or not sprites[0].get("sprite_key") or type(sprites[0].get("visible_drawables")) is not int or sprites[0]["visible_drawables"] <= 0:
                    errors.append(f"Stage {index} structure {identity}: missing actual profile-bound sprite")
        if mode == "sprite2d":
            ground = frame.get("world_2d") or {}
            if any(ground.get(key) != 3025 for key in ("tile_count", "loaded_atlas_tiles", "tiles_inside_depth")) or type(ground.get("ground_tiles_on_screen")) is not int or ground["ground_tiles_on_screen"] <= 0:
                errors.append(f"Stage {index}: missing loaded terrain atlas or ground clipped/outside viewport")
            camera = ground.get("camera_position", [])
            bands = ground.get("ground_z", [])
            near_plane, far_plane = ground.get("camera_near"), ground.get("camera_far")
            if len(camera) != 3 or len(bands) != 2 or not all(finite_number(v) for v in [*camera, *bands, near_plane, far_plane]) or not near_plane < far_plane or any(not near_plane <= camera[2]-z <= far_plane for z in bands):
                errors.append(f"Stage {index}: actual 2D camera does not cover the ground depth bands")
        if mode == "models3d" and (frame.get("static_props_by_archetype") != STATIC_PROPS or frame.get("ready_static_props") != 942):
            errors.append(f"Stage {index}: incomplete authored prop inventory or missing real drawables")
        if mobile:
            nodes = {node.get("name") for node in frame.get("nodes", []) if node.get("visible") is True
                     and isinstance(node.get("size"), list) and len(node["size"]) == 2
                     and all(finite_number(v) and v > 0 for v in node["size"])}
            if not {"MobileJoystick", "MobileAttack", *(f"MobileAbility-{i}" for i in range(4))} <= nodes:
                errors.append(f"Stage {index}: phone primary controls are missing")
        matched.append(dict(stage=index, snapshot_tick=frame.get("snapshot_tick"), structure_count=len(actual)))
    if mode == "models3d" and len(records) == 5:
        props = [records[i].get("selected_prop") or {} for i in range(5)]
        if len({p.get("entity") for p in props}) != 1 or len({p.get("key") for p in props}) != 1:
            errors.append("Prop owner/key changed during repeated substitution")
        for i, prop in enumerate(props):
            if not prop.get("entity") or not prop.get("key") or prop.get("solid") is not False or prop.get("geometry_ready") is not True or not valid_geometry(prop.get("geometry")):
                errors.append(f"Stage {i}: prop has no actual ready non-solid geometry")
            if i and (prop.get("active_model") != MODELS[(i-1) % 2] or prop.get("desired_model") != prop.get("active_model")):
                errors.append(f"Stage {i}: incorrect loaded replacement model")
        signatures = [p.get("geometry", {}).get("mesh_geometry_signatures") for p in props]
        if signatures[1] != signatures[3] or signatures[2] != signatures[4] or signatures[1] == signatures[2]:
            errors.append("Actual vertex/index signatures do not prove A→B→A→B")
        if len({records[i].get("prop_instances") for i in range(5)}) != 1:
            errors.append("Prop instance ownership/count grew during swaps")
        for field in ("scene_roots", "mesh_assets"):
            if len({records[i].get(field) for i in (2, 3, 4)}) != 1:
                errors.append(f"Repeated loaded assets are not bounded: {field}")
        cache_counts = [records[i].get("map_visual_cache_counts") for i in (2, 3, 4)]
        if any(not isinstance(counts, list) or len(counts) != 2 or any(type(n) is not int or n < 0 for n in counts) for counts in cache_counts) or cache_counts[0] != cache_counts[1] or cache_counts[1] != cache_counts[2]:
            errors.append("Repeated production model/material cache sizes are not bounded")
        operations = summary.get("operations", [])
        if len(operations) != 4 or any(op.get("stage") != i or op.get("key") != props[i].get("key") or str(op.get("model")) + "#Scene0" != MODELS[(i-1) % 2] for i, op in enumerate(operations, 1)):
            errors.append("Missing exact declared registry operation sequence")
    return {"pass": not errors, "errors": errors, "ui_profile": profile, "matched_receipts": matched,
            "observer_snapshots": len(snapshots), "physical_device_verified": False,
            "geometry_scope": "actual inherited-visible mesh buffers; screenshot inspection separately confirms framing"}


def image_proof(output, files):
    images = {}
    for name in files:
        path = output / name
        if not path.is_file():
            continue
        with path.open("rb") as source:
            header = source.read(24)
        if len(header) != 24 or header[:8] != b"\x89PNG\r\n\x1a\n":
            continue
        images[name] = dict(bytes=path.stat().st_size, sha256=sha256(path), pixels=list(struct.unpack(">II", header[16:24])))
    return images


def verify_existing(output):
    report = output / "capture-run.json"
    initial = report.read_bytes()
    result = json.loads(initial)
    summary = json.loads((output / "qa-summary.json").read_text())
    samples = [json.loads(line) for line in (output / "authoritative-snapshots.jsonl").read_text().splitlines() if line]
    expected = expected_map(json.loads((output / "map-config.json").read_text()))
    result["verification"] = verify_map(summary, samples, result["requested_profile"] == "mobile", result["mode"], expected)
    files = FILES_3D if result["mode"] == "models3d" else FILES_2D
    images = image_proof(output, files)
    unchanged = images == result.get("images") and len(images) == len(files)
    result["capture_pass"] = bool(result.get("client_exit_code") == 0 and not result.get("timed_out") and not result.get("error") and result.get("binary_unchanged") is True and not result.get("runtime_errors") and unchanged and result["verification"]["pass"])
    initial_path = output / "capture-run.initial.json"
    if not initial_path.exists():
        initial_path.write_bytes(initial)
    result["reverification"] = dict(native_relaunched=False, images_unchanged=unchanged,
                                    initial_report_sha256=sha256(initial_path), verifier_sha256=sha256(Path(__file__)))
    report.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    return result["capture_pass"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("client-bin", "server-bin", "assets", "output", "map-config"):
        parser.add_argument(f"--{name}", type=Path, required=name == "output")
    parser.add_argument("--verify-existing", action="store_true")
    parser.add_argument("--mode", choices=("models3d", "sprite2d"), default="models3d")
    parser.add_argument("--touch-controls", action="store_true")
    parser.add_argument("--width", type=int, default=1280)
    parser.add_argument("--height", type=int, default=720)
    parser.add_argument("--timeout", type=int, default=180)
    args = parser.parse_args()
    if args.verify_existing:
        return verify_existing(args.output.resolve())
    if any(p is None for p in (args.client_bin, args.server_bin, args.assets)):
        parser.error("Native binaries and assets are required")
    client, server, assets, output = [p.resolve() for p in (args.client_bin, args.server_bin, args.assets, args.output)]
    if not client.is_file() or not server.is_file() or not assets.is_dir():
        parser.error("Native binaries/assets do not exist")
    if not 320 <= args.width <= 3840 or not 320 <= args.height <= 2160:
        parser.error("Unsupported capture size")
    if output.exists() and any(output.iterdir()):
        parser.error("Output must be empty; preserve previous evidence")
    config_source = (args.map_config or ROOT / "shared/assets/maps/verdant.json").resolve()
    config_bytes = config_source.read_bytes()
    expected = expected_map(json.loads(config_bytes))
    output.mkdir(parents=True, exist_ok=True)
    config = output / "map-config.json"
    config.write_bytes(config_bytes)
    (output / "expected-map.json").write_text(json.dumps(expected, indent=2) + "\n")
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as reservation:
        reservation.bind(("127.0.0.1", 0))
        address = reservation.getsockname()
    timeout = max(45, min(args.timeout, 600))
    result = dict(scenario="map", mode=args.mode, pixels=[args.width, args.height],
                  binary_sha256=dict(client=sha256(client), server=sha256(server)), platform=platform.platform(),
                  requested_profile="mobile" if args.touch_controls else "desktop", map_config_sha256=sha256(config),
                  map_profile=expected["map_profile"], geometry_id=expected["geometry_id"],
                  expected_structure_count=len(expected["structures"]), verifier_sha256=sha256(Path(__file__)),
                  scripted_camera=True, scripted_cosmetic_registry=args.mode == "models3d",
                  synthetic_structures=False, physical_device_verified=False, manual_interaction_verified=False)
    children, observer = [], None
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="omoba-map-qa-") as isolated:
        env = dict(os.environ, SERVER_ADDR=f"{address[0]}:{address[1]}", GAME_SERVER_ADDR=f"{address[0]}:{address[1]}",
                   OMOBA_MATCH_MODE="dev", OMOBA_CLIENT_CONFIG_DIR=str(Path(isolated) / "config"), OMOBA_ASSET_DIR=str(assets),
                   OMOBA_MAP_CONFIG=str(config), OMOBA_PLAYER_VISUAL_MODE=args.mode, OMOBA_DEBUG_UI="0",
                   OMOBA_QA_WIDTH=str(args.width), OMOBA_QA_HEIGHT=str(args.height), OMOBA_TOUCH_CONTROLS="1" if args.touch_controls else "0",
                   OMOBA_VISUAL_QA_DIR=str(output), OMOBA_VISUAL_QA_SCENARIO="map", OMOBA_VISUAL_QA_TIMEOUT=str(timeout - 10),
                   OMOBA_MAP_QA_EXPECTED_STRUCTURES=str(len(expected["structures"])))
        for key in ("OMOBA_AUTOJOIN", "OMOBA_MEASURE_MODELS", "OMOBA_AVATAR_MANIFEST", "OMOBA_TARGETING_QA", "OMOBA_TARGETING_QA_SCENARIO", "OMOBA_MAP_QA_PROP_KEY"):
            env.pop(key, None)
        try:
            with (output / "server.log").open("w") as log:
                process = subprocess.Popen([str(server)], cwd=isolated, env=env, stdout=log, stderr=subprocess.STDOUT)
            children.append(process)
            deadline = time.monotonic() + 15
            while "is listening" not in (output / "server.log").read_text(errors="replace"):
                if process.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError("Server failed startup configuration validation; inspect server.log")
                time.sleep(.05)
            observer = CombatObserver(address, output / "authoritative-snapshots.jsonl")
            with (output / "client.log").open("w") as log:
                native = subprocess.Popen([str(client)], cwd=isolated, env=env, stdout=log, stderr=subprocess.STDOUT)
            children.append(native)
            while native.poll() is None and time.monotonic() - started < timeout:
                if process.poll() is not None:
                    raise RuntimeError("Server exited during native scenario")
                observer.update(time.monotonic())
                time.sleep(.015)
            result["client_exit_code"] = native.poll()
            result["timed_out"] = native.poll() is None
            observer.update(time.monotonic())
        except Exception as error:
            result["error"] = str(error)
        finally:
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
    try:
        summary = json.loads((output / "qa-summary.json").read_text())
    except (OSError, ValueError):
        summary = {}
    result["verification"] = verify_map(summary, observer.samples if observer else [], args.touch_controls, args.mode, expected)
    files = FILES_3D if args.mode == "models3d" else FILES_2D
    result["images"] = image_proof(output, files)
    result["binary_unchanged"] = result["binary_sha256"] == dict(client=sha256(client), server=sha256(server))
    log = (output / "client.log").read_text(errors="replace") if (output / "client.log").exists() else ""
    result["runtime_errors"] = [line for line in log.splitlines() if any(marker in line for marker in ("panicked at", "MAP_QA failed", "Path not found", "does not exist"))]
    result["elapsed_seconds"] = time.monotonic() - started
    result["capture_pass"] = bool(result.get("client_exit_code") == 0 and not result.get("error") and result["verification"]["pass"]
                                  and len(result["images"]) == len(files) and all(image["pixels"] == result["pixels"] for image in result["images"].values())
                                  and result["binary_unchanged"] and not result["runtime_errors"])
    (output / "capture-run.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    return result["capture_pass"]


def self_test():
    """Data-only verifier fixtures, never native capture or gameplay evidence."""
    config = json.loads((ROOT / "shared/assets/maps/verdant.json").read_text())
    expected = expected_map(config)
    mesh = dict(mesh_geometry_signatures=["abc:3:3"], mesh_count=1, vertices=3, indices=3, world_min=[0, 0, 0], world_max=[1, 1, 1])
    structures = [dict(s, hp=s["max_hp"], protected=s["kind"] == "base_tower", model_geometry=[mesh],
                       sprite_geometry=[dict(entity=str(s["id"]), owner=str(s["id"]), sprite_key="tower", profile_key=s["visual_profile"], visible_drawables=1)]) for s in expected["structures"]]
    frames = []
    operations = []
    for i, filename in enumerate(FILES_3D):
        geometry = dict(mesh, mesh_geometry_signatures=["a:3:3" if i in (0, 1, 3) else "b:3:3"])
        active = MODELS[max(0, i - 1) % 2]
        prop = dict(entity="prop", key="environment.glb:lantern / 0001", solid=False, geometry_ready=True, geometry=geometry,
                    desired_model=active, active_model=active)
        frames.append(dict(stage=i, file=filename, pixels=[1280, 720], mobile_controls=False, visual_mode="Models3d", snapshot_tick=i,
                           server_epoch=7, match_id=9, geometry_id=expected["geometry_id"], map_profile=expected["map_profile"],
                           structures=structures, selected_prop=prop, prop_instances=950, scene_roots=12, mesh_assets=50, map_visual_cache_counts=[2, 5], static_props_by_archetype=STATIC_PROPS, ready_static_props=942))
        if i:
            operations.append(dict(stage=i, key=prop["key"], model=active.removesuffix("#Scene0")))
    summary = dict(scenario="map", **{"pass": True}, scripted_camera=True, scripted_cosmetic_registry=True, synthetic_structures=False,
                   manual_interaction_verified=False, physical_device_verified=False, pixels=[1280, 720], captures=frames, operations=operations,
                   geometry_id=expected["geometry_id"], map_profile=expected["map_profile"])
    wire = [dict(s, x=s["position"][0], y=s["position"][1], z=s["position"][2]) for s in structures]
    samples = [dict(server_epoch=7, round_id=9, snapshot_tick=i, geometry_id=expected["geometry_id"], map_profile=expected["map_profile"], structures=wire) for i in range(5)]
    check = lambda s, p=samples, mobile=False, mode="models3d": verify_map(s, p, mobile, mode, expected)["pass"]
    assert check(summary)
    checks = 1
    mutations = [lambda s: s.update(physical_device_verified=True),
                 lambda s: s["captures"][0].update(snapshot_tick=999),
                 lambda s: s["captures"][0].update(mobile_controls=True),
                 lambda s: s["captures"][0]["structures"][0].update(hp=111),
                 lambda s: s["captures"][0]["structures"][0].update(position=[999, 3, 1]),
                 lambda s: s["captures"][0]["structures"][0].update(map_key="wrong"),
                 lambda s: s["captures"][0]["structures"].pop(),
                 lambda s: s["captures"][0]["structures"][0].update(model_geometry=[]),
                 lambda s: s["captures"][2]["selected_prop"]["geometry"].update(mesh_geometry_signatures=["a:3:3"]),
                 lambda s: s["captures"][3]["selected_prop"].update(entity="other"),
                 lambda s: s["captures"][4].update(scene_roots=999),
                 lambda s: s["captures"][4].update(mesh_assets=999),
                 lambda s: s["captures"][2]["selected_prop"].update(active_model=MODELS[0]),
                 lambda s: s["operations"].pop(),
                 lambda s: s["captures"][4].update(map_visual_cache_counts=[999, 999]),
                 lambda s: s["captures"][0].update(static_props_by_archetype={}),
                 lambda s: s["captures"][0].update(ready_static_props=941)]
    for mutate in mutations:
        fixture = copy.deepcopy(summary)
        mutate(fixture)
        assert not check(fixture)
        checks += 1
    rounded = copy.deepcopy(summary)
    rounded["captures"][0]["structures"][0]["position"][0] += .00001
    assert check(rounded)
    checks += 1
    assert not check(summary, [])
    checks += 1
    flat = copy.deepcopy(summary)
    flat.update(scripted_cosmetic_registry=False, captures=flat["captures"][:2], operations=[])
    for i, frame in enumerate(flat["captures"]):
        frame.update(file=FILES_2D[i], visual_mode="Sprite2d", selected_prop=None, world_2d=dict(tile_count=3025, loaded_atlas_tiles=3025, tiles_inside_depth=3025, ground_tiles_on_screen=100, camera_position=[0,0,0], camera_near=-1000, camera_far=1000, ground_z=[-100,-80]))
    assert check(flat, mode="sprite2d")
    checks += 1
    bad_flat = copy.deepcopy(flat)
    bad_flat["captures"][0]["structures"][0]["sprite_geometry"][0]["visible_drawables"] = 0
    assert not check(bad_flat, mode="sprite2d")
    checks += 1
    for mutate in [lambda s: s["captures"][0].pop("world_2d"),
                   lambda s: s["captures"][0]["world_2d"].update(loaded_atlas_tiles=0),
                   lambda s: s["captures"][0]["world_2d"].update(tiles_inside_depth=0),
                   lambda s: s["captures"][0]["world_2d"].update(ground_tiles_on_screen=0),
                   lambda s: s["captures"][0]["world_2d"].update(camera_position=[0,0,999])]:
        invalid = copy.deepcopy(flat)
        mutate(invalid)
        assert not check(invalid, mode="sprite2d")
        checks += 1
    phone = copy.deepcopy(summary)
    for frame in phone["captures"]:
        frame.update(mobile_controls=True, nodes=[dict(name=name, visible=True, size=[40, 40]) for name in ["MobileJoystick", "MobileAttack", *(f"MobileAbility-{i}" for i in range(4))]])
    assert check(phone, mobile=True)
    checks += 1
    phone["captures"][0]["nodes"].pop()
    assert not check(phone, mobile=True)
    checks += 1
    custom = expected_map(json.loads((ROOT / "examples/maps/two-tier.json").read_text()))
    assert len(custom["structures"]) == 10 and next(s for s in custom["structures"] if s["id"] == 9)["max_hp"] == 420
    checks += 1
    print(f"PASS {checks} map verifier checks; synthetic unit fixtures only, no native capture claims")


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        self_test()
    else:
        raise SystemExit(0 if main() else 1)
