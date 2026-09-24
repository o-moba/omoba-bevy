#!/usr/bin/env python3
"""Native healing butterfly lifecycle capture with independent authoritative proof.

An admitted Blue peer casts one normal Q, then stays passive. The Green native
client uses production movement to collect a pickup and wait for its respawn.
No synthetic damage/healing, particle freezing, or physical-device claims.
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

from capture_combat import CombatObserver, PassiveTarget, finite_number
from capture_verdant import sha256, verify_beta_ui_profile

IMAGES = ("01-butterfly-available.png", "02-butterfly-flapping.png", "03-butterfly-collected.png", "04-butterfly-respawned.png")


class InjuryPeer(PassiveTarget):
    def __init__(self, address, index):
        super().__init__(address, index)
        self.casts = []

    def injure_once(self, snapshots):
        if self.casts or not self.own:
            return
        for snapshot in reversed(snapshots):
            own = next((p for p in snapshot.get("players", []) if p["id"] == self.own["id"]), None)
            target = next((p for p in snapshot.get("players", []) if p.get("team") == "green" and p.get("hp", 0) > 0), None)
            if own and target and math.hypot(own["x"] - target["x"], own["z"] - target["z"]) < 9.0:
                command = dict(type="cast", slot=0, target=dict(kind="player", id=target["id"]))
                # Bypass PassiveTarget's intentional no-gameplay guard for this ONE command.
                self.socket.send(json.dumps(command, separators=(",", ":")).encode())
                self.casts.append(dict(command, source_id=own["id"], snapshot_tick=snapshot["snapshot_tick"]))
                return


def close(a, b, tolerance=1e-4):
    return finite_number(a) and finite_number(b) and math.isclose(a, b, rel_tol=0, abs_tol=tolerance)


def verify_pickups(summary, snapshots, mobile, mode, casts, output=None):
    summary = summary if isinstance(summary, dict) else {}
    profile = verify_beta_ui_profile(summary, mobile, IMAGES)
    errors = list(profile["errors"])
    if summary.get("scenario") != "forest-pickups" or summary.get("pass") is not True:
        errors.append("Missing successful native pickup summary")
    for key, expected in (("scripted_commands", True), ("synthetic_damage", False), ("synthetic_healing", False),
                          ("manual_interaction_verified", False), ("physical_device_verified", False)):
        if summary.get(key) is not expected:
            errors.append(f"Invalid provenance {key}")
    if summary.get("readbacks") != [0, 1, 2, 3]:
        errors.append("Missing completed native image readbacks")
    if summary.get("setup_fixture") != "development server initial player placement; ambient AI disabled":
        errors.append("Missing fixture provenance")
    captures = {c.get("stage"): c for c in summary.get("captures", []) if isinstance(c, dict)}
    if set(captures) != {0, 1, 2, 3}:
        return dict(pass_=False, errors=errors + ["Missing lifecycle captures"], ui_profile=profile)
    ready, flap, collected, respawn = (captures[i] for i in range(4))
    pickup_id = ready.get("pickup", {}).get("id")
    own_id = summary.get("player_id")
    identity = (ready.get("server_epoch"), ready.get("match_id"))
    samples = sorted((s for s in snapshots if (s.get("server_epoch"), s.get("round_id", s.get("match_id"))) == identity), key=lambda s:s["snapshot_tick"])
    by_tick = {s["snapshot_tick"]: s for s in samples}
    for i, c in captures.items():
        if c.get("visual_mode") != ("Sprite2d" if mode == "sprite2d" else "Models3d") or c.get("pixels") != summary.get("pixels"):
            errors.append(f"Stage {i}: wrong actual mode or viewport")
        p = c.get("pickup", {})
        if p.get("id") != pickup_id or p.get("available") is not (i != 2):
            errors.append(f"Stage {i}: wrong pickup identity or availability")
        observed = by_tick.get(c.get("snapshot_tick"), {})
        source = next((p for p in observed.get("forest_pickups", []) if p["id"] == pickup_id), {})
        if p != source:
            errors.append(f"Stage {i}: pickup does not match independent snapshot")
        actor = next((p for p in observed.get("players", []) if p["id"] == own_id), {})
        if not close(c.get("hp"), actor.get("hp")) or not close(c.get("max_hp"), actor.get("max_hp")):
            errors.append(f"Stage {i}: HP differs from authoritative snapshot")
        position = c.get("player_position", [])
        if len(position) != 3 or not all(finite_number(v) for v in position) or not actor or math.hypot(position[0]-actor.get("x", math.inf), position[2]-actor.get("z", math.inf)) > 0.5:
            errors.append(f"Stage {i}: actor position not corroborated")
        visible = [w for w in c.get("wings", []) if w.get("pickup_id") == pickup_id and w.get("visible") is True and w.get("drawable") is True and w.get("on_screen") is True]
        if (i == 2 and visible) or (i != 2 and len(visible) < 2):
            errors.append(f"Stage {i}: wrong rendered butterfly visibility")
        if mobile:
            shown = {n.get("name") for n in c.get("nodes", []) if n.get("visible") and len(n.get("size", [])) == 2 and all(finite_number(v) and v > 0 for v in n["size"])}
            if not {"MobileJoystick", "MobileAttack", *(f"MobileAbility-{j}" for j in range(4))} <= shown:
                errors.append(f"Stage {i}: missing visible mobile controls")
        if i != 2 and actor and len(p.get("position", [])) == 2:
            if math.hypot(actor["x"]-p["position"][0], actor["z"]-p["position"][1]) <= 1.5:
                errors.append(f"Stage {i}: waiting actor is inside collection radius")
        if output is not None:
            path = output / IMAGES[i]
            try:
                data = path.read_bytes()
                valid = len(data) > 32 and data[:8] == b"\x89PNG\r\n\x1a\n" and list(struct.unpack(">II", data[16:24])) == summary.get("pixels")
            except OSError:
                valid = False
            if not valid:
                errors.append(f"Stage {i}: missing PNG readback or wrong dimensions")
    before = {w.get("entity"): w for w in ready.get("wings", [])}
    if not any(w.get("entity") in before and (w.get("rotation") != before[w["entity"]].get("rotation") or w.get("position") != before[w["entity"]].get("position")) for w in flap.get("wings", [])):
        errors.append("Timed frames show no butterfly animation")
    p = collected.get("pickup", {})
    expected = min(ready.get("max_hp", 0)-ready.get("hp", 0), ready.get("max_hp", 0)*0.05)
    if expected <= 0 or not close(p.get("healed_amount"), expected) or not close(collected.get("hp", 0)-ready.get("hp", 0), expected):
        errors.append("Collection did not heal exactly capped 5% max HP")
    if p.get("last_collector_id") != own_id or p.get("collection_sequence") != ready.get("pickup", {}).get("collection_sequence", -2)+1 or respawn.get("pickup", {}).get("collection_sequence") != p.get("collection_sequence"):
        errors.append("Collection/respawn receipt identity is invalid")
    if not any(part.get("event_id") == (2**64-1-pickup_id) and finite_number(part.get("age")) and 0.06 <= part["age"] < .65 for part in collected.get("particles", [])):
        errors.append("No visible green collection burst")
    receipts = [p for s in samples for p in s.get("forest_pickups", []) if p["id"] == pickup_id]
    if not receipts or max(p["collection_sequence"] for p in receipts) != 1:
        errors.append("Expected exactly one authoritative collection")
    transition = next((s for s in samples if any(p["id"] == pickup_id and p["collection_sequence"] == 1 for p in s.get("forest_pickups", []))), None)
    if transition:
        actor = next((a for a in transition.get("players", []) if a["id"] == own_id), {})
        anchor = collected.get("pickup", {}).get("position", [math.inf, math.inf])
        if not actor or math.hypot(actor["x"]-anchor[0], actor["z"]-anchor[1]) > 1.5001:
            errors.append("Authoritative collector was outside pickup radius")
    if not close(respawn.get("hp"), collected.get("hp")):
        errors.append("HP changed during passive respawn wait")
    if respawn.get("elapsed_seconds", 0)-collected.get("elapsed_seconds", 0) < 29:
        errors.append("Respawn occurred before the 30-second lifecycle")
    if len(casts) != 1 or casts[0].get("slot") != 0 or casts[0].get("target") != dict(kind="player", id=own_id):
        errors.append("Expected exactly one ordinary opponent Q")
    else:
        hits = [e for s in samples for e in s.get("combat_events", []) if e.get("source") == dict(kind="player", id=casts[0]["source_id"]) and e.get("target") == dict(kind="player", id=own_id) and e.get("amount", 0)>0]
        if not hits:
            errors.append("No authoritative combat event proves the injury")
    if len(summary.get("requests", [])) != 3 or any(r.get("kind") != "navigation" for r in summary.get("requests", [])):
        errors.append("Expected three ordinary movement requests")
    return dict(pass_=not errors, errors=errors, ui_profile=profile, observer_snapshots=len(samples), pickup_id=pickup_id,
                healed_amount=p.get("healed_amount"), physical_device_verified=False)


def self_test():
    """Data-only verifier unit fixtures; never presented as native evidence."""
    pickup = dict(id=1, position=[-66.,45.], available=True, collection_sequence=0, last_collector_id=None, healed_amount=0.)
    captures, snapshots = [], []
    event = dict(source=dict(kind="player",id=2), target=dict(kind="player",id=1), amount=20.)
    for i in range(4):
        p = dict(pickup)
        if i >= 2:
            p.update(collection_sequence=1, last_collector_id=1, healed_amount=5., available=i == 3)
        position = [-66.,.5,45.] if i == 2 else [-63.,.5,45.]
        hp = 85. if i >= 2 else 80.
        wings = [dict(entity=str(w),pickup_id=1,visible=i!=2,drawable=True,on_screen=True,rotation=[float(i),0,0,1],position=position) for w in range(2)]
        captures.append(dict(stage=i,file=IMAGES[i],pixels=[1280,720],mobile_controls=False,visual_mode="Models3d",
                             pickup=p,player_position=position,hp=hp,max_hp=100.,snapshot_tick=i+1,server_epoch=7,match_id=8,
                             wings=wings,particles=[dict(event_id=2**64-2,age=.1)] if i==2 else [],elapsed_seconds=[10,11,12,42][i]))
        snapshots.append(dict(server_epoch=7,round_id=8,snapshot_tick=i+1,forest_pickups=[p],combat_events=[event],
                              players=[dict(id=1,hp=hp,max_hp=100.,x=position[0],z=position[2])]))
    summary = dict(scenario="forest-pickups", **{"pass":True},scripted_commands=True,synthetic_damage=False,synthetic_healing=False,
                   manual_interaction_verified=False,physical_device_verified=False,readbacks=[0,1,2,3],
                   setup_fixture="development server initial player placement; ambient AI disabled", player_id=1,pixels=[1280,720],
                   captures=captures,requests=[dict(kind="navigation") for _ in range(3)])
    casts = [dict(slot=0,target=dict(kind="player",id=1),source_id=2)]
    def check(value, observed=snapshots, commands=casts, output=None):
        return verify_pickups(value,observed,False,"models3d",commands,output)["pass_"]
    assert check(summary)
    mutations = [lambda s:s.update(readbacks=[]), lambda s:s["captures"][2].update(particles=[]),
                 lambda s:s["captures"][2].update(hp=90.), lambda s:s["captures"][0].update(wings=[]),
                 lambda s:s["captures"][3].update(elapsed_seconds=20), lambda s:s.update(physical_device_verified=True),
                 lambda s:s["captures"][1].update(wings=copy.deepcopy(s["captures"][0]["wings"])),
                 lambda s:s["captures"][2]["pickup"].update(last_collector_id=2)]
    for mutate in mutations:
        broken=copy.deepcopy(summary); mutate(broken); assert not check(broken)
    assert not check(summary, commands=[])
    assert not check(summary, observed=[])
    with tempfile.TemporaryDirectory(prefix="pickup-verifier-selftest-") as directory:
        assert not check(summary, output=Path(directory))
    print("forest pickup verifier: valid fixture + 11 negative checks passed (data-only unit fixtures)")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("client-bin", "server-bin", "assets", "output"):
        parser.add_argument(f"--{name}", type=Path, required=name == "output")
    parser.add_argument("--mode", choices=("models3d", "sprite2d"), default="models3d")
    parser.add_argument("--touch-controls", action="store_true")
    parser.add_argument("--width", type=int, default=1280)
    parser.add_argument("--height", type=int, default=720)
    parser.add_argument("--timeout", type=int, default=180)
    args = parser.parse_args()
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
    result=dict(scenario="forest-pickups", mode=args.mode,
                binary_sha256=dict(client=sha256(client),server=sha256(server)), platform=platform.platform(),
                pixels=[args.width,args.height], requested_profile="mobile" if args.touch_controls else "desktop",
                scripted_commands=True, synthetic_damage=False, physical_device_verified=False)
    children, peers, observer = [], [], None
    started=time.monotonic()
    with tempfile.TemporaryDirectory(prefix="omoba-pickup-qa-") as isolated:
        env=dict(os.environ, SERVER_ADDR=f"{address[0]}:{address[1]}",GAME_SERVER_ADDR=f"{address[0]}:{address[1]}",
                 OMOBA_MATCH_MODE="dev",OMOBA_TEAM_SIZE="5",OMOBA_CLIENT_CONFIG_DIR=str(Path(isolated)/"config"),
                 OMOBA_ASSET_DIR=str(assets),OMOBA_PLAYER_VISUAL_MODE=args.mode,OMOBA_DEBUG_UI="0",
                 OMOBA_QA_WIDTH=str(args.width),OMOBA_QA_HEIGHT=str(args.height),
                 OMOBA_TOUCH_CONTROLS="1" if args.touch_controls else "0",OMOBA_VISUAL_QA_DIR=str(output),
                 OMOBA_VISUAL_QA_SCENARIO="forest-pickups",OMOBA_VISUAL_QA_TIMEOUT=str(timeout-10))
        for key in ("OMOBA_AUTOJOIN","OMOBA_MEASURE_MODELS","OMOBA_AVATAR_MANIFEST","OMOBA_TARGETING_QA", "OMOBA_TARGETING_QA_SCENARIO"):
            env.pop(key,None)
        env["OMOBA_TARGETING_QA"]="1"
        try:
            with (output/"server.log").open("w") as log:
                proc=subprocess.Popen([str(server)],cwd=isolated,env=env,stdout=log,stderr=subprocess.STDOUT)
            children.append(proc)
            deadline=time.monotonic()+15
            while "is listening" not in (output/"server.log").read_text(errors="replace"):
                if proc.poll() is not None or time.monotonic()>deadline: raise RuntimeError("Server failed to start")
                time.sleep(.05)
            peers=[InjuryPeer(address,0)]
            observer=CombatObserver(address,output/"authoritative-snapshots.jsonl")
            with (output/"client.log").open("w") as log:
                native=subprocess.Popen([str(client)],cwd=isolated,env=env,stdout=log,stderr=subprocess.STDOUT)
            children.append(native)
            while native.poll() is None and time.monotonic()-started<timeout:
                if proc.poll() is not None: raise RuntimeError("Server exited during scenario")
                now=time.monotonic()
                for peer in peers: peer.update(now)
                observer.update(now)
                peers[0].injure_once(observer.samples)
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
    result["casts"] = peers[0].casts if peers else []
    proof=verify_pickups(summary,observer.samples if observer else [],args.touch_controls,args.mode,result["casts"],output)
    proof["pass"]=proof.pop("pass_")
    result["verification"]=proof
    result["images"]={name:dict(bytes=(output/name).stat().st_size,sha256=sha256(output/name)) for name in IMAGES if (output/name).is_file()}
    result["binary_unchanged"]=(result["binary_sha256"]==dict(client=sha256(client),server=sha256(server)))
    log=(output/"client.log").read_text(errors="replace") if (output/"client.log").exists() else ""
    result["runtime_errors"]=[line for line in log.splitlines() if any(marker in line for marker in ("panicked at","FOREST_PICKUP_QA failed","Path not found","does not exist"))]
    result["elapsed_seconds"]=time.monotonic()-started
    result["capture_pass"]=bool(result.get("client_exit_code")==0 and not result.get("error") and proof["pass"]
                                and len(result["images"])==4 and result["binary_unchanged"] and not result["runtime_errors"])
    (output/"capture-run.json").write_text(json.dumps(result,indent=2)+"\n")
    print(json.dumps(result,indent=2))
    raise SystemExit(0 if result["capture_pass"] else 1)


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        self_test()
    else:
        main()
