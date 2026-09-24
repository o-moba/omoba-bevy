#!/usr/bin/env python3
"""Native 3D visibility transitions with admitted observers and original UDP evidence.

A dev-only initial placement fixture starts ordinary heroes beside authored brush.
All transitions use regular movement; screenshots are native Bevy readbacks.
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

from capture_combat import CombatObserver
from capture_verdant import sha256, verify_beta_ui_profile

IMAGES = ("01-visible-outside-brush.png", "02-hidden-in-brush.png", "03-same-brush-reveal.png",
          "04-concealed-again.png", "05-exited-brush.png", "06-outside-team-sight.png")



class AdmittedObserver(CombatObserver):
    """An ordinary joined endpoint, retaining every full received snapshot."""
    def __init__(self, address, output, team):
        super().__init__(address, output)
        self.team = team
        self.last_join = -1.
        self.last_move = -1.
        self.own = None
        self.destination = None
        self.joined = False

    def send(self, data):
        self.socket.send(json.dumps(data, separators=(",", ":")).encode())

    def update(self, now):
        if not self.joined and now - self.last_join >= 1.:
            self.last_join = now
            self.send(dict(type="hello", protocol_version=2))
            self.send(dict(type="join", team=self.team, character="cube", hero_class="ranger",
                           avatar="agnes", session_id=f"vision-qa-{self.team}-{os.getpid()}"))
        super().update(now)
        if self.samples:
            latest = self.samples[-1]
            self.own = next((p for p in latest.get("players", []) if p["id"] == latest.get("your_id")), None)
            self.joined = self.own is not None
        if self.own and self.destination and now - self.last_move >= .05:
            self.last_move = now
            dx, dz = self.destination[0] - self.own["x"], self.destination[1] - self.own["z"]
            distance = math.hypot(dx, dz)
            if distance > .12:
                step = min(.2, distance)
                self.send(dict(type="transform", x=self.own["x"]+dx/distance*step,
                               y=self.own.get("y", .5), z=self.own["z"]+dz/distance*step,
                               yaw=math.atan2(dx, dz)))

    def arrived(self):
        return self.own and self.destination and math.hypot(self.own["x"]-self.destination[0], self.own["z"]-self.destination[1]) < .25


def stage_waypoints(command):
    """Ordinary movement avoids pushing stationary solid allied heroes."""
    destination = command.get("enemy_destination")
    if destination is None:
        return []
    if command["stage"] != 5:
        return [destination]
    x, z = command["brush"]["center"]
    return [[x + 4., z + 2.], [x + 12., z + 2.], destination]


def verify(summary, green, blue, mobile):
    errors = verify_beta_ui_profile(summary, mobile, IMAGES)["errors"]
    if summary.get("scenario") != "team-vision" or summary.get("pass") is not True:
        errors.append("Missing successful native team-vision summary")
    if summary.get("synthetic_visibility") is not False or summary.get("physical_device_verified") is not False:
        errors.append("Incorrect provenance")
    target = summary.get("target_id")
    captures = summary.get("captures", [])
    if len(captures) != 6 or {c.get("stage") for c in captures} != set(range(6)):
        errors.append("Expected six distinct native stages")
    baseline = next((c for c in captures if c.get("stage") == 0), {})
    budgets = {"mesh_assets": baseline.get("mesh_assets"),
               "material_assets": baseline.get("material_assets"),
               "total_entities": baseline.get("total_entities")}
    budget_records = []
    for capture in captures:
        stage = capture.get("stage")
        hidden = stage in (1, 3, 5)
        counters = {key: capture.get(key) for key in budgets}
        budget_ok = True
        for key in ("mesh_assets", "material_assets"):
            if (type(budgets[key]) is not int or budgets[key] <= 0
                    or type(counters[key]) is not int or counters[key] > budgets[key]):
                errors.append(f"Stage {stage}: {key} grew or lacks measured baseline")
                budget_ok = False
        if hidden or stage == 4:
            if (type(budgets["total_entities"]) is not int or budgets["total_entities"] <= 0
                    or type(counters["total_entities"]) is not int
                    or counters["total_entities"] > budgets["total_entities"] + 16):
                errors.append(f"Stage {stage}: entity count exceeds initial visible frame plus 16")
                budget_ok = False
        budget_records.append(dict(stage=stage, counters=counters, **{"pass": budget_ok}))
        tick = capture.get("snapshot_tick")
        match = lambda samples: next((s for s in samples if s.get("snapshot_tick") == tick and s.get("server_epoch") == capture.get("server_epoch")), {})
        observed, opponent = match(green), match(blue)
        if not observed or not opponent:
            errors.append(f"Stage {stage}: missing exact-tick admitted observer snapshots")
        enemy = next((p for p in opponent.get("players", []) if p["id"] == target), None)
        wire_enemy = any(p["id"] == target for p in observed.get("players", []))
        if not enemy or wire_enemy == hidden or capture.get("enemy_present") == hidden:
            errors.append(f"Stage {stage}: authoritative presence differs from rendering")
        if capture.get("visual_mode") != "Models3d":
            errors.append(f"Stage {stage}: wrong renderer")
        if hidden:
            if any(capture.get(k) != v for k, v in (("enemy_entities",0),("rendered_enemy_drawables",0),
                   ("previous_actor_entities_remaining",0),("target_cleared",True),("enemy_combat_bars",0))):
                errors.append(f"Stage {stage}: stale hidden entity, descendant, or target")
            if capture.get("minimap", {}).get("hero_markers", {}).get("enemy") != 0:
                errors.append(f"Stage {stage}: stale minimap enemy")
        elif capture.get("enemy_entities") != 1 or capture.get("rendered_enemy_drawables",0) <= 0:
            errors.append(f"Stage {stage}: no unique visible drawable enemy")
        vision = capture.get("vision", {})
        if stage == 2 and vision.get("local_brush") != 1:
            errors.append("Same-brush frame lacks local concealment status")
        if stage == 5 and enemy:
            if any(math.hypot(enemy["x"]-s["position"][0], enemy["z"]-s["position"][1]) <= s["radius"]
                   for s in observed.get("vision", {}).get("sources", [])):
                errors.append("Final enemy is still inside shared radial sight")
        fog = capture.get("fog", {})
        if not fog.get("active") or fog.get("darkest",0) <= 0 or fog.get("clearest") != 0:
            errors.append(f"Stage {stage}: missing live fog alpha mask")
        if stage == 2 and not any(n.get("name") == "BrushStatus" and n.get("visible")
                                  and "BRUSH" in (n.get("text") or "") for n in capture.get("nodes", [])):
            errors.append("Same-brush frame lacks rendered brush status")
        if not any(n.get("name") == "Team fog of war" and n.get("visible") for n in capture.get("nodes", [])):
            errors.append(f"Stage {stage}: fog UI is not rendered")
        if mobile:
            shown = {n.get("name") for n in capture.get("nodes", []) if n.get("visible") and n.get("size") and min(n["size"]) > 0}
            if not {"MobileJoystick","MobileAttack",*(f"MobileAbility-{i}" for i in range(4))} <= shown:
                errors.append(f"Stage {stage}: primary mobile controls absent")
    return {"pass": not errors, "errors": errors, "green_snapshots":len(green),"blue_snapshots":len(blue),
            "bounded_rendering": {"baseline": budgets, "mesh_material_growth_allowed": 0,
                "hidden_and_reacquired_entity_growth_allowed": 16, "stages": budget_records}}


def self_test():
    summary = {"scenario":"team-vision","pass":True,"synthetic_visibility":False,"physical_device_verified":False,
               "target_id":2,"pixels":[1280,720],"captures":[]}
    green, blue = [], []
    for stage, name in enumerate(IMAGES):
        hidden = stage in (1,3,5)
        enemy = dict(id=2,x=32. if stage==5 else -12.,z=-6.)
        vision = dict(sources=[dict(position=[-4.,-6.],radius=32.)],local_brush=1 if stage==2 else None,local_hidden=stage==2)
        summary["captures"].append(dict(stage=stage,file=name,pixels=[1280,720],mobile_controls=False,
            visual_mode="Models3d",snapshot_tick=stage+1,server_epoch=1,enemy_present=not hidden,
            enemy_entities=0 if hidden else 1,rendered_enemy_drawables=0 if hidden else 2,
            previous_actor_entities_remaining=0,target_cleared=True,enemy_combat_bars=0,minimap={"hero_markers":{"enemy":0 if hidden else 1}},
            vision=vision,fog=dict(active=True,darkest=190,clearest=0),
            mesh_assets=562,material_assets=346,total_entities=1000 if hidden else 1080,
            nodes=[dict(name="Team fog of war",visible=True),dict(name="BrushStatus",visible=True,text="BRUSH · REVEALED")]))
        green.append(dict(snapshot_tick=stage+1,server_epoch=1,players=[] if hidden else [enemy],vision=vision))
        blue.append(dict(snapshot_tick=stage+1,server_epoch=1,players=[enemy]))
    assert verify(summary,green,blue,False)["pass"]
    assert not verify(summary,[],blue,False)["pass"]
    broken=copy.deepcopy(summary); broken["captures"][1]["enemy_entities"]=1
    assert not verify(broken,green,blue,False)["pass"]
    broken=copy.deepcopy(green); broken[1]["players"]=[dict(id=2)]
    assert not verify(summary,broken,blue,False)["pass"]
    broken=copy.deepcopy(summary); broken["captures"][2]["vision"]["local_brush"]=None
    assert not verify(broken,green,blue,False)["pass"]
    broken=copy.deepcopy(summary); broken["captures"][3]["fog"]["darkest"]=0
    assert not verify(broken,green,blue,False)["pass"]
    for field, stage, value in (("mesh_assets", 2, 563), ("material_assets", 3, 347),
                                ("total_entities", 4, 1097), ("total_entities", 5, 1097)):
        broken=copy.deepcopy(summary); broken["captures"][stage][field]=value
        assert not verify(broken,green,blue,False)["pass"]
    assert stage_waypoints(dict(stage=5,brush=dict(center=[-22.,-8.]),enemy_destination=[22.,-8.])) == [[-18.,-6.],[-10.,-6.],[22.,-8.]]
    print("PASS: ten verifier checks and derived waypoint route; synthetic test data is never capture evidence")


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ("client-bin","server-bin","assets","output"):
        parser.add_argument(f"--{name}",type=Path,required=True)
    parser.add_argument("--touch-controls",action="store_true")
    parser.add_argument("--width",type=int,default=1280)
    parser.add_argument("--height",type=int,default=720)
    parser.add_argument("--timeout",type=int,default=120)
    args=parser.parse_args()
    client,server,assets,output=(p.resolve() for p in (args.client_bin,args.server_bin,args.assets,args.output))
    if not client.is_file() or not server.is_file() or not assets.is_dir(): parser.error("Native binaries and assets must exist")
    if output.exists() and any(output.iterdir()): parser.error("Output must be empty")
    if not 320<=args.width<=3840 or not 320<=args.height<=2160: parser.error("Unsupported viewport")
    output.mkdir(parents=True,exist_ok=True)
    with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as reservation:
        reservation.bind(("127.0.0.1",0)); address=reservation.getsockname()
    timeout=max(45,min(args.timeout,120))
    result=dict(scenario="team-vision",binary_sha256=dict(client=sha256(client),server=sha256(server)),
                platform=platform.platform(),pixels=[args.width,args.height],
                requested_profile="mobile" if args.touch_controls else "desktop",physical_device_verified=False)
    children,observers=[],[]
    started=time.monotonic()
    with tempfile.TemporaryDirectory(prefix="omoba-vision-qa-") as isolated:
        env=dict(os.environ,SERVER_ADDR=f"{address[0]}:{address[1]}",GAME_SERVER_ADDR=f"{address[0]}:{address[1]}",
            OMOBA_MATCH_MODE="dev",OMOBA_TEAM_SIZE="5",OMOBA_CLIENT_CONFIG_DIR=str(Path(isolated)/"config"),
            OMOBA_ASSET_DIR=str(assets),OMOBA_PLAYER_VISUAL_MODE="models3d",OMOBA_DEBUG_UI="0",OMOBA_VISION_QA="1",
            OMOBA_QA_WIDTH=str(args.width),OMOBA_QA_HEIGHT=str(args.height),
            OMOBA_TOUCH_CONTROLS="1" if args.touch_controls else "0",OMOBA_VISUAL_QA_DIR=str(output),
            OMOBA_VISUAL_QA_SCENARIO="team-vision",OMOBA_VISUAL_QA_TIMEOUT=str(timeout-5))
        for key in ("OMOBA_AUTOJOIN","OMOBA_MEASURE_MODELS","OMOBA_AVATAR_MANIFEST","OMOBA_TARGETING_QA","OMOBA_TARGETING_QA_SCENARIO"):
            env.pop(key,None)
        try:
            with (output/"server.log").open("w") as log:
                proc=subprocess.Popen([str(server)],cwd=isolated,env=env,stdout=log,stderr=subprocess.STDOUT)
            children.append(proc)
            deadline=time.monotonic()+15
            while "is listening" not in (output/"server.log").read_text(errors="replace"):
                if proc.poll() is not None or time.monotonic()>deadline: raise RuntimeError("Server failed to start")
                time.sleep(.05)
            green=AdmittedObserver(address,output/"green-snapshots.jsonl","green")
            blue=AdmittedObserver(address,output/"blue-snapshots.jsonl","blue")
            observers=[green,blue]
            with (output/"client.log").open("w") as log:
                native=subprocess.Popen([str(client)],cwd=isolated,env=env,stdout=log,stderr=subprocess.STDOUT)
            children.append(native)
            ready_stage=-1
            route_stage=-1
            route=[]
            route_index=0
            command={"stage":0}
            route_log=[]
            while native.poll() is None and time.monotonic()-started<timeout:
                if proc.poll() is not None: raise RuntimeError("Server exited during scenario")
                now=time.monotonic()
                try:
                    command=json.loads((output/"stage-command.json").read_text())
                except (OSError,ValueError):
                    pass
                stage=command["stage"]
                if stage != route_stage:
                    route_stage=stage
                    route=stage_waypoints(command)
                    route_index=0
                blue.destination=route[route_index] if route else None
                for peer in observers: peer.update(now)
                if blue.arrived():
                    if route_index + 1 < len(route):
                        route_log.append(dict(stage=stage,waypoint=route[route_index],
                            snapshot_tick=blue.samples[-1]["snapshot_tick"],enemy=blue.own))
                        route_index+=1
                        (output/"movement-waypoints.json").write_text(json.dumps(route_log,indent=2))
                    elif stage!=ready_stage:
                        (output/"peer-ready.json").write_text(json.dumps(dict(stage=stage,
                            snapshot_tick=blue.samples[-1]["snapshot_tick"],enemy=blue.own)))
                        ready_stage=stage
                time.sleep(.015)
            result["client_exit_code"]=native.poll()
            result["timed_out"]=native.poll() is None
            for peer in observers: peer.update(time.monotonic())
        except Exception as error: result["error"]=str(error)
        finally:
            for peer in observers: peer.close()
            for child in reversed(children):
                if child.poll() is None: child.terminate()
                try: child.wait(timeout=5)
                except subprocess.TimeoutExpired: child.kill(); child.wait()
    try: summary=json.loads((output/"qa-summary.json").read_text())
    except (OSError,ValueError): summary={}
    result["verification"]=verify(summary,observers[0].samples if observers else [],observers[1].samples if len(observers)>1 else [],args.touch_controls)
    result["images"]={name:dict(bytes=(output/name).stat().st_size,sha256=sha256(output/name)) for name in IMAGES if (output/name).is_file()}
    result["binary_unchanged"]=result["binary_sha256"]==dict(client=sha256(client),server=sha256(server))
    log=(output/"client.log").read_text(errors="replace") if (output/"client.log").exists() else ""
    result["runtime_errors"]=[line for line in log.splitlines() if any(s in line for s in ("panicked at","TEAM_VISION_QA failed","Path not found","does not exist"))]
    result["elapsed_seconds"]=time.monotonic()-started
    result["capture_pass"]=bool(result.get("client_exit_code")==0 and not result.get("error") and result["verification"]["pass"]
        and len(result["images"])==6 and result["binary_unchanged"] and not result["runtime_errors"])
    (output/"capture-run.json").write_text(json.dumps(result,indent=2)+"\n")
    print(json.dumps(result,indent=2))
    raise SystemExit(0 if result["capture_pass"] else 1)

if __name__=="__main__":
    if sys.argv[1:]==["--self-test"]: self_test()
    else: main()
