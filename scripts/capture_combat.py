#!/usr/bin/env python3
"""Capture actual native combat, preserving independent UDP and pixel evidence.

One ordinary Q per champion scenario; normal mixed waves and production movement
in --waves. No synthetic damage, held VFX, browser, or physical-device claims.
Run --self-test for bounded verifier checks without launching the game.
"""
import argparse
import copy
import json
import math
import os
from pathlib import Path
import platform
import socket
import subprocess
import sys
import tempfile
import time

from capture_verdant import FRAME_HEADER, ScenarioPeer, SnapshotObserver, sha256, verify_beta_ui_profile

IMAGES = ("01-combat-ready.png", "02-projectile-flight.png", "03-confirmed-impact.png")
STYLES = {"warrior": "crescent", "mage": "arcane", "ranger": "arrow", "cleric": "holy"}


class PassiveTarget(ScenarioPeer):
    """Ordinary admitted opponent. Never sends movement, casts or attacks."""
    def send(self, data):
        if data.get("type") == "transform":
            return
        if data.get("type") == "join":
            data = dict(data, team="blue", session_id=f"combat-qa-{os.getpid()}-{self.index}")
        if data.get("type") not in ("hello", "join"):
            raise RuntimeError("Passive target attempted a gameplay command")
        super().send(data)


class CombatObserver(SnapshotObserver):
    """Keep the original hello-only endpoint but retain full combat fields.

    Frame bounds match capture_verdant. No response packet is rewritten, and the
    JSONL contains original complete snapshots, including event/actor identities.
    """
    def update(self, now):
        if now - self.last_hello >= 1.0:
            self.last_hello = now
            self.socket.send(b'{"type":"hello","protocol_version":2}')
        self.pending = {key: value for key, value in self.pending.items() if now - value[0] < 2.0}
        # Bound one poll even if a broken server floods this read-only endpoint.
        for _ in range(2048):
            try:
                data = self.socket.recv(65536)
            except BlockingIOError:
                return
            if data.startswith(b"OMB1"):
                if not FRAME_HEADER.size <= len(data) <= 1200:
                    raise RuntimeError("Invalid observer frame length")
                _, version, epoch, tick, index, count, total = FRAME_HEADER.unpack_from(data)
                if version != 2 or not 0 < count <= 56 or not index < count or total > 65507:
                    raise RuntimeError("Invalid observer frame header")
                key = (epoch, tick)
                if key not in self.pending and len(self.pending) >= 4:
                    del self.pending[min(self.pending, key=lambda old: self.pending[old][0])]
                chunks = self.pending.setdefault(key, (now, {}))[1]
                chunks[index] = data[FRAME_HEADER.size:]
                if len(chunks) != count:
                    continue
                data = b"".join(chunks[i] for i in range(count))
                del self.pending[key]
                if len(data) != total:
                    raise RuntimeError("Incomplete observer frame reconstruction")
            packet = json.loads(data)
            if packet.get("type") == "snapshot":
                self.samples.append(packet)
                self.output.write(json.dumps(packet, separators=(",", ":")) + "\n")
                self.output.flush()


def finite_number(value):
    return type(value) in (int, float) and math.isfinite(value)


def combat_events_match(actual, observed):
    """Keep identity exact, allowing only f32-to-JSON numeric round trips."""
    if not isinstance(actual, dict) or not isinstance(observed, dict):
        return False
    for key, expected_type in (("id", int), ("style", str), ("killed", bool)):
        if type(actual.get(key)) is not expected_type or type(observed.get(key)) is not expected_type or actual[key] != observed[key]:
            return False
    if "action_slot" not in actual or "action_slot" not in observed:
        return False
    if type(actual["action_slot"]) not in (int, type(None)) or type(actual["action_slot"]) is not type(observed["action_slot"]) or actual["action_slot"] != observed["action_slot"]:
        return False
    for key in ("source", "target"):
        left, right = actual.get(key), observed.get(key)
        if not isinstance(left, dict) or not isinstance(right, dict):
            return False
        if type(left.get("id")) is not int or type(right.get("id")) is not int or left["id"] != right["id"]:
            return False
        if type(left.get("kind")) is not str or type(right.get("kind")) is not str or left["kind"] != right["kind"]:
            return False
    return all(finite_number(actual.get(key)) and finite_number(observed.get(key))
               and math.isclose(actual[key], observed[key], rel_tol=0.0, abs_tol=1e-5)
               for key in ("amount", "x", "y", "z"))


def verify_combat(summary, snapshots, mobile, class_name, mode, waves):
    """A client success flag alone is insufficient: link visible objects to UDP."""
    summary = summary if isinstance(summary, dict) else {}
    profile = verify_beta_ui_profile(summary, mobile, IMAGES)
    errors = list(profile["errors"])
    if summary.get("scenario") != "combat" or summary.get("pass") is not True:
        errors.append("Missing successful native combat summary")
    for field, expected in (("scripted_commands", True), ("synthetic_damage", False),
                            ("manual_interaction_verified", False), ("physical_device_verified", False)):
        if summary.get(field) is not expected:
            errors.append(f"Incorrect or missing provenance: {field}")
    if summary.get("class") != class_name or summary.get("waves") is not waves:
        errors.append("Class/wave scenario differs from request")
    fixture = ("normal development waves; production route to midlane" if waves else
               "development server initial player placement; ambient AI disabled")
    if summary.get("setup_fixture") != fixture:
        errors.append("Missing explicit fixture provenance")
    captures = {record.get("stage"): record for record in summary.get("captures", []) if isinstance(record, dict)}
    if len(captures) != 3:
        errors.append("Expected three distinct actual capture stages")
    required_mode = "Sprite2d" if mode == "sprite2d" else "Models3d"
    for capture in captures.values():
        if capture.get("visual_mode") != required_mode:
            errors.append("Actual renderer mode differs from request")
        if capture.get("pixels") != summary.get("pixels"):
            errors.append("Per-frame viewport differs from native readback")
        if mobile:
            shown = {node.get("name") for node in capture.get("nodes", [])
                     if node.get("visible") is True and isinstance(node.get("size"), list)
                     and len(node["size"]) == 2 and all(finite_number(v) and v > 0 for v in node["size"])}
            if not {"MobileJoystick", "MobileAttack", *(f"MobileAbility-{i}" for i in range(4))} <= shown:
                errors.append("Mobile capture does not show all primary controls")
    style = "caster_bolt" if waves else STYLES[class_name]
    flight, impact = captures.get(1, {}), captures.get(2, {})
    identity = (flight.get("server_epoch"), flight.get("match_id"))
    relevant = [sample for sample in snapshots if sample.get("server_epoch") == identity[0]
                and sample.get("round_id", sample.get("match_id")) == identity[1]]
    server_projectiles = {p["id"]: p for sample in relevant for p in sample.get("projectiles", [])}
    server_events = {e["id"]: e for sample in relevant for e in sample.get("combat_events", [])}
    flights = flight.get("projectiles", [])
    if not flights:
        errors.append("Flight frame has no visible live projectile")
    for projectile in flights:
        source = server_projectiles.get(projectile.get("id"), {})
        for field in ("id", "owner_id", "source_kind", "style", "action_slot"):
            if projectile.get(field) != source.get(field):
                errors.append(f"Projectile {projectile.get('id')} differs from observer {field}")
        if projectile.get("style") != style or not isinstance(projectile.get("rendered_drawables"), int) or projectile["rendered_drawables"] <= 0:
            errors.append("Flight has wrong style or no visible drawable descendants")
    events = {e.get("id"): e for e in impact.get("combat_events", [])}
    numbers = impact.get("damage_numbers", [])
    if not numbers:
        errors.append("Impact frame has no visible confirmed damage number")
    for number in numbers:
        event = events.get(number.get("event_id"), {})
        observed = server_events.get(number.get("event_id"), {})
        if not combat_events_match(event, observed) or event.get("style") != style or event["amount"] <= 0:
            errors.append("Visible number does not match a positive authoritative event")
            continue
        if number.get("visible") is not True:
            errors.append("Damage number is hidden")
        try:
            if abs(float(number["text"]) - event["amount"]) > .51:
                errors.append("Visible number text differs from actual damage")
        except (KeyError, ValueError, TypeError):
            errors.append("Damage number text is invalid")
        center, size, pixels = number.get("center", []), number.get("size", []), impact.get("pixels", [])
        if any(not isinstance(v, list) or len(v) != 2 for v in (center, size, pixels)) or not all(finite_number(v) for pair in (center, size, pixels) for v in pair):
            errors.append("Damage number lacks finite layout bounds")
        elif min(size) <= 0 or any(c - s / 2 < 0 or c + s / 2 > p for c, s, p in zip(center, size, pixels)):
            errors.append("Damage number is outside the captured viewport")
    requests = summary.get("requests", [])
    if waves:
        roles = {m.get("kind") for m in captures.get(0, {}).get("minions", [])}
        if not {"melee", "caster"} <= roles:
            errors.append("Ready frame lacks both live minion roles")
        if not any(r.get("kind") == "navigation" for r in requests):
            errors.append("Missing production midlane route request")
        actual_minions = {m["id"]: m for sample in relevant for m in sample.get("minions", [])}
        for minion in captures.get(0, {}).get("minions", []):
            source = actual_minions.get(minion.get("id"), {})
            if any(minion.get(key) != source.get(key) for key in ("id", "kind", "team")):
                errors.append("Rendered wave role/team does not match independent server")
    else:
        casts = [r for r in requests if r.get("kind") == "cast"]
        target = {"kind": "player", "id": summary.get("target_id")}
        if len(casts) != 1 or casts[0].get("slot") != 0 or casts[0].get("target") != target:
            errors.append("Champion scenario requires exactly one ordinary Q request")
        if any(e.get("source") != {"kind": "player", "id": summary.get("player_id")} or e.get("target") != target for e in events.values()):
            errors.append("Captured event did not hit the requested opponent")
        hp = [player["hp"] for sample in sorted(relevant, key=lambda s: s["snapshot_tick"])
              for player in sample.get("players", []) if player.get("id") == target["id"]]
        if len(hp) < 2 or not any(b < a for a, b in zip(hp, hp[1:])):
            errors.append("Independent observer saw no target HP decrease")
    return dict(pass_=not errors, errors=errors, ui_profile=profile, observer_snapshots=len(relevant),
                captured_projectile_ids=[p.get("id") for p in flights],
                captured_event_ids=[n.get("event_id") for n in numbers],
                synthetic_damage=False, physical_device_verified=False)


def self_test():
    """Verifier unit fixtures are data only; never used as capture evidence."""
    event = dict(id=4, source=dict(kind="player", id=1), target=dict(kind="player", id=2), amount=12.0, x=-4.0, y=1.05, z=-8.0, style="arrow", action_slot=0, killed=False)
    projectile = dict(id=3, owner_id=1, source_kind="player", style="arrow", action_slot=0)
    captures = [dict(stage=i, file=name, pixels=[1280,720], mobile_controls=False,
                     visual_mode="Models3d", server_epoch=7, match_id=8) for i,name in enumerate(IMAGES)]
    captures[1]["projectiles"] = [dict(projectile, rendered_drawables=2)]
    captures[2].update(combat_events=[event], damage_numbers=[dict(event_id=4,text="12",visible=True,size=[25,28],center=[600,300])])
    summary = dict(scenario="combat", **{"pass":True}, scripted_commands=True, synthetic_damage=False,
                   manual_interaction_verified=False, physical_device_verified=False, **{"class":"ranger"}, waves=False,
                   setup_fixture="development server initial player placement; ambient AI disabled", pixels=[1280,720],
                   player_id=1,target_id=2,captures=captures,requests=[dict(kind="cast",slot=0,target=dict(kind="player",id=2))])
    samples = [dict(server_epoch=7,round_id=8,snapshot_tick=i,projectiles=[projectile],combat_events=[event],players=[dict(id=2,hp=hp)]) for i,hp in [(1,100),(2,88)]]
    check = lambda data, wire=samples: verify_combat(data,wire,False,"ranger","models3d",False)["pass_"]
    assert check(summary)
    assert not check(summary, [])
    broken = copy.deepcopy(summary); broken["captures"][1]["projectiles"][0]["rendered_drawables"] = 0
    assert not check(broken)
    broken = copy.deepcopy(summary); broken["captures"][2]["damage_numbers"][0]["event_id"] = 99
    assert not check(broken)
    broken = copy.deepcopy(summary); broken["captures"][2]["damage_numbers"][0]["center"] = [-50,20]
    assert not check(broken)
    broken = copy.deepcopy(summary); broken["physical_device_verified"] = True
    assert not check(broken)
    rounded = copy.deepcopy(summary)
    rounded["captures"][2]["combat_events"][0]["y"] = 1.0499999523162842
    assert check(rounded)
    checks = 7
    for field, value in (("amount", 12.001), ("x", -4.001), ("y", 1.051),
                         ("z", -8.001), ("amount", float("nan")), ("y", float("inf")),
                         ("id", True), ("killed", 0), ("action_slot", False), ("style", "holy")):
        broken = copy.deepcopy(summary)
        broken["captures"][2]["combat_events"][0][field] = value
        assert not check(broken), f"Changed {field} must fail"
        checks += 1
    broken = copy.deepcopy(summary)
    broken["captures"][2]["combat_events"][0]["source"]["id"] = True
    assert not check(broken)
    checks += 1
    print(f"PASS: {checks} combat capture verifier checks")


def verify_existing(output):
    """Re-read saved evidence only; preserve the first run report byte-for-byte."""
    initial_path = output / "capture-run.initial.json"
    current_path = output / "capture-run.json"
    initial_bytes = (initial_path if initial_path.exists() else current_path).read_bytes()
    original = json.loads(initial_bytes)
    summary = json.loads((output / "qa-summary.json").read_text())
    with (output / "authoritative-snapshots.jsonl").open() as source:
        snapshots = [json.loads(line) for line in source if line.strip()]
    proof = verify_combat(summary, snapshots, original["requested_profile"] == "mobile",
                          original["class_name"], original["mode"], original["waves"])
    proof["pass"] = proof.pop("pass_")
    images = {name: dict(bytes=(output / name).stat().st_size, sha256=sha256(output / name))
              for name in IMAGES if (output / name).is_file()}
    unchanged_images = images == original.get("images") and len(images) == len(IMAGES)
    result = copy.deepcopy(original)
    result["verification"] = proof
    result["capture_pass"] = bool(original.get("client_exit_code") == 0
                                  and not original.get("timed_out") and not original.get("error")
                                  and original.get("binary_unchanged") is True
                                  and not original.get("runtime_errors")
                                  and unchanged_images and proof["pass"])
    if not initial_path.exists():
        with initial_path.open("xb") as initial:
            initial.write(initial_bytes)
    result["reverification"] = dict(
        native_relaunched=False, images_unchanged=unchanged_images,
        binary_hashes_source="original native run; binaries were not rebuilt or relaunched",
        initial_report=initial_path.name, initial_report_sha256=sha256(initial_path),
        verifier_sha256=sha256(Path(__file__)),
        summary_sha256=sha256(output / "qa-summary.json"),
        observer_sha256=sha256(output / "authoritative-snapshots.jsonl"),
        numeric_event_comparison="finite amount/x/y/z; abs_tol=1e-5, rel_tol=0; identity/style/action/killed exact")
    current_path.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    return result["capture_pass"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("client-bin", "server-bin", "assets", "output"):
        parser.add_argument(f"--{name}", type=Path, required=name == "output")
    parser.add_argument("--verify-existing", action="store_true",
                        help="Reverify --output without launching; preserve capture-run.initial.json")
    parser.add_argument("--class", dest="class_name", choices=STYLES, default="ranger")
    parser.add_argument("--mode", choices=("models3d", "sprite2d"), default="models3d")
    parser.add_argument("--waves", action="store_true")
    parser.add_argument("--touch-controls", action="store_true")
    parser.add_argument("--width", type=int, default=1280)
    parser.add_argument("--height", type=int, default=720)
    parser.add_argument("--timeout", type=int, default=180)
    args = parser.parse_args()
    if args.verify_existing:
        raise SystemExit(0 if verify_existing(args.output.resolve()) else 1)
    if any(value is None for value in (args.client_bin, args.server_bin, args.assets)):
        parser.error("--client-bin, --server-bin and --assets are required for a new capture")
    client, server, assets, output = (p.resolve() for p in (args.client_bin,args.server_bin,args.assets,args.output))
    if not client.is_file() or not server.is_file() or not assets.is_dir(): parser.error("Native binaries and assets must exist")
    if not 320 <= args.width <= 3840 or not 320 <= args.height <= 2160: parser.error("Unsupported viewport")
    if output.exists() and any(output.iterdir()): parser.error("Output must be empty; preserve previous evidence")
    output.mkdir(parents=True, exist_ok=True)
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as reservation:
        reservation.bind(("127.0.0.1",0)); address=reservation.getsockname()
    timeout=max(45,min(args.timeout,600))
    result=dict(scenario="combat", class_name=args.class_name, mode=args.mode, waves=args.waves,
                binary_sha256=dict(client=sha256(client),server=sha256(server)), platform=platform.platform(),
                pixels=[args.width,args.height], requested_profile="mobile" if args.touch_controls else "desktop",
                scripted_commands=True, synthetic_damage=False, physical_device_verified=False)
    children, peers, observer = [], [], None
    started=time.monotonic()
    with tempfile.TemporaryDirectory(prefix="omoba-combat-qa-") as isolated:
        env=dict(os.environ, SERVER_ADDR=f"{address[0]}:{address[1]}",GAME_SERVER_ADDR=f"{address[0]}:{address[1]}",
                 OMOBA_MATCH_MODE="dev",OMOBA_TEAM_SIZE="5",OMOBA_CLIENT_CONFIG_DIR=str(Path(isolated)/"config"),
                 OMOBA_ASSET_DIR=str(assets),OMOBA_PLAYER_VISUAL_MODE=args.mode,OMOBA_DEBUG_UI="0",
                 OMOBA_QA_WIDTH=str(args.width),OMOBA_QA_HEIGHT=str(args.height),
                 OMOBA_TOUCH_CONTROLS="1" if args.touch_controls else "0",OMOBA_VISUAL_QA_DIR=str(output),
                 OMOBA_VISUAL_QA_SCENARIO="combat",OMOBA_COMBAT_QA_CLASS=args.class_name,
                 OMOBA_COMBAT_QA_WAVES="1" if args.waves else "0",OMOBA_VISUAL_QA_TIMEOUT=str(timeout-10))
        for key in ("OMOBA_AUTOJOIN","OMOBA_MEASURE_MODELS","OMOBA_AVATAR_MANIFEST","OMOBA_TARGETING_QA", "OMOBA_TARGETING_QA_SCENARIO"):
            env.pop(key,None)
        if not args.waves: env["OMOBA_TARGETING_QA"]="1"
        try:
            with (output/"server.log").open("w") as log:
                proc=subprocess.Popen([str(server)],cwd=isolated,env=env,stdout=log,stderr=subprocess.STDOUT)
            children.append(proc)
            deadline=time.monotonic()+15
            while "is listening" not in (output/"server.log").read_text(errors="replace"):
                if proc.poll() is not None or time.monotonic()>deadline: raise RuntimeError("Server failed to start")
                time.sleep(.05)
            peers=[PassiveTarget(address,0)]
            observer=CombatObserver(address,output/"authoritative-snapshots.jsonl")
            with (output/"client.log").open("w") as log:
                native=subprocess.Popen([str(client)],cwd=isolated,env=env,stdout=log,stderr=subprocess.STDOUT)
            children.append(native)
            while native.poll() is None and time.monotonic()-started<timeout:
                if proc.poll() is not None: raise RuntimeError("Server exited during scenario")
                now=time.monotonic()
                for peer in peers: peer.update(now)
                observer.update(now)
                time.sleep(.015)
            result["client_exit_code"]=native.poll()
            result["timed_out"]=native.poll() is None
            if observer: observer.update(time.monotonic())
        except Exception as error:
            result["error"]=str(error)
        finally:
            for peer in peers: peer.close()
            if observer: observer.close()
            for child in reversed(children):
                if child.poll() is None: child.terminate()
                try: child.wait(timeout=5)
                except subprocess.TimeoutExpired: child.kill(); child.wait()
    try: summary=json.loads((output/"qa-summary.json").read_text())
    except (OSError,ValueError): summary={}
    proof=verify_combat(summary,observer.samples if observer else [],args.touch_controls,args.class_name,args.mode,args.waves)
    proof["pass"]=proof.pop("pass_")
    result["verification"]=proof
    result["images"]={name:dict(bytes=(output/name).stat().st_size,sha256=sha256(output/name)) for name in IMAGES if (output/name).is_file()}
    result["binary_unchanged"]=(result["binary_sha256"]==dict(client=sha256(client),server=sha256(server)))
    log=(output/"client.log").read_text(errors="replace") if (output/"client.log").exists() else ""
    result["runtime_errors"]=[line for line in log.splitlines() if any(marker in line for marker in ("panicked at","COMBAT_QA failed","Path not found","does not exist"))]
    result["elapsed_seconds"]=time.monotonic()-started
    result["capture_pass"]=bool(result.get("client_exit_code")==0 and not result.get("error") and proof["pass"]
                                and len(result["images"])==3 and result["binary_unchanged"] and not result["runtime_errors"])
    (output/"capture-run.json").write_text(json.dumps(result,indent=2)+"\n")
    print(json.dumps(result,indent=2))
    raise SystemExit(0 if result["capture_pass"] else 1)

if __name__=="__main__":
    if sys.argv[1:]==["--self-test"]: self_test()
    else: main()
