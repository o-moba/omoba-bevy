#!/usr/bin/env python3
"""Stage and capture showcase shots for the website, stores and README.

Every scene is a real native capture: a local dev server and client run from
source, the client presses production buttons and gives the hero ordinary move
orders, and Bevy reads the window back. Nothing is painted or composited.

Scenes are declared in SCENES below (desktop and phone profiles, hero class,
lane point, hold time and which harness frames to keep). Examples:

    python3 scripts/capture_showcase.py --list
    python3 scripts/capture_showcase.py --build --output /tmp/omoba-showcase
    python3 scripts/capture_showcase.py --build --output /tmp/s --scene mobile-lane

The output directory receives the curated PNGs, `showcase-manifest.json`
(source commit, version, binaries, scene parameters, hashes) and `raw/<scene>/`
with every harness frame and log. Phone scenes are previews of the phone UI
rendered by a desktop development build (OMOBA_TOUCH_CONTROLS=1), not device
screenshots; the manifest says so.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import socket
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(Path(__file__).resolve().parent))

DESKTOP = dict(profile="desktop", size=(1280, 720))
PHONE = dict(profile="phone", size=(844, 390))

# harness: "match" drives the beta-ui harness against a live server; "shell"
# captures the offline front-end screens. `lane` is "<top|mid|bot>:<0..1>"
# measured from the hero's own base; `hold` is seconds of live match before
# the shot, so waves and bots have time to meet. `keep` maps harness frames
# ({h} = viewport height) to published file names.
SCENES = {
    "desktop-lane": dict(
        DESKTOP, harness="match", mode="practice", hero="warden", lane="mid:0.5", hold=45,
        keep={"03-gameplay-{h}p.png": "desktop-gameplay.png"},
        caption="Desktop gameplay: a practice match on the mid lane."),
    "desktop-shop": dict(
        DESKTOP, harness="match", mode="dev", hero="mage",
        keep={"04-shop-{h}p.png": "desktop-shop.png"},
        caption="Desktop sanctuary shop."),
    "desktop-hero-select": dict(
        DESKTOP, harness="shell",
        keep={"04-hero-select.png": "desktop-hero-select.png", "01-home.png": "desktop-home.png"},
        caption="Desktop hero selection with all five classes."),
    "phone-lane": dict(
        PHONE, harness="match", mode="practice", hero="ranger", lane="mid:0.5", hold=45,
        keep={"03-gameplay-{h}p.png": "mobile-gameplay-preview.png"},
        caption="Phone interface preview on the mid lane."),
    "phone-shop": dict(
        PHONE, harness="match", mode="dev", hero="cleric",
        keep={"04-shop-{h}p.png": "mobile-shop-preview.png"},
        caption="Phone touch shop preview."),
    "phone-hero-select": dict(
        PHONE, harness="shell",
        keep={"04-hero-select.png": "mobile-hero-select.png"},
        caption="Phone hero selection preview."),
    "hero-banner": dict(
        profile="desktop", size=(1600, 1000), harness="match", mode="practice", hero="warrior",
        lane="mid:0.5", hold=45,
        keep={"03-gameplay-{h}p.png": "beta-gameplay.png"},
        caption="Heroes and minions meet on the mid lane."),
}


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as file:
        for chunk in iter(lambda: file.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def git(*args):
    return subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True).stdout.strip()


def source_identity():
    version = next((line.split('"')[1] for line in (ROOT / "Cargo.toml").read_text().splitlines()
                    if line.startswith("version = ")), None)
    return dict(commit=git("rev-parse", "HEAD"), dirty=bool(git("status", "--porcelain")),
                version=version)


def free_address():
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as reservation:
        reservation.bind(("127.0.0.1", 0))
        return reservation.getsockname()


def base_env(scene, assets, workdir, output):
    width, height = scene["size"]
    env = dict(os.environ, OMOBA_CLIENT_CONFIG_DIR=str(Path(workdir) / "config"),
               OMOBA_ASSET_DIR=str(assets), OMOBA_PLAYER_VISUAL_MODE="models3d", OMOBA_DEBUG_UI="0",
               OMOBA_QA_WIDTH=str(width), OMOBA_QA_HEIGHT=str(height),
               OMOBA_TOUCH_CONTROLS="1" if scene["profile"] == "phone" else "0")
    for key in list(env):
        # Any other harness switch would take the client over.
        if key.startswith("OMOBA_") and ("_QA" in key or key in ("OMOBA_AUTOJOIN", "OMOBA_AVATAR_MANIFEST")) \
                and key not in ("OMOBA_QA_WIDTH", "OMOBA_QA_HEIGHT"):
            del env[key]
    return env


def run_until_exit(processes, client, timeout):
    started = time.monotonic()
    while client.poll() is None and time.monotonic() - started < timeout:
        for process in processes:
            if process is not client and process.poll() is not None:
                raise RuntimeError("server exited during capture")
        time.sleep(0.1)
    return client.poll()


def stop(processes):
    for process in reversed(processes):
        if process.poll() is None:
            process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def capture_scene(name, scene, binaries, assets, raw, timeout):
    raw.mkdir(parents=True, exist_ok=True)
    record = dict(scene=name, started_at=datetime.datetime.now(datetime.timezone.utc).isoformat())
    processes = []
    with tempfile.TemporaryDirectory(prefix=f"omoba-showcase-{name}-") as workdir:
        env = base_env(scene, assets, workdir, raw)
        try:
            # Shell scenes get a live server too, so the menus show a
            # connected client rather than "Connecting...".
            host, port = free_address()
            address = f"{host}:{port}"
            env.update(SERVER_ADDR=address, GAME_SERVER_ADDR=address, OMOBA_MATCH_MODE=scene.get("mode", "dev"),
                       OMOBA_TEAM_SIZE="5")
            if scene["harness"] == "shell":
                env.update(OMOBA_FRONTEND_QA_OUTPUT=str(raw), OMOBA_QA_CLEAN_FRAME="1")
            else:
                env.update(OMOBA_QA_TEAM="green", OMOBA_VISUAL_QA_DIR=str(raw),
                           OMOBA_VISUAL_QA_SCENARIO="beta-ui", OMOBA_VISUAL_QA_TIMEOUT=str(timeout - 20),
                           OMOBA_BETA_UI_CLASS=scene["hero"])
                if scene.get("lane"):
                    env.update(OMOBA_BETA_UI_SCENE_LANE=scene["lane"],
                               OMOBA_BETA_UI_SCENE_HOLD=str(scene.get("hold", 30)))
            log = (raw / "server.log").open("w")
            processes.append(subprocess.Popen([str(binaries["server"])], cwd=workdir, env=env,
                                              stdout=log, stderr=subprocess.STDOUT))
            deadline = time.monotonic() + 20
            while "is listening" not in (raw / "server.log").read_text(errors="replace"):
                if processes[0].poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError("server did not report listening")
                time.sleep(0.05)
            log = (raw / "client.log").open("w")
            client = subprocess.Popen([str(binaries["client"])], cwd=workdir, env=env,
                                      stdout=log, stderr=subprocess.STDOUT)
            processes.append(client)
            record["client_exit_code"] = run_until_exit(processes, client, timeout)
        except Exception as error:  # recorded, the remaining scenes still run
            record["error"] = str(error)
        finally:
            stop(processes)
    height = scene["size"][1]
    record["frames"] = {}
    for source, published in scene["keep"].items():
        frame = raw / source.format(h=height)
        record["frames"][published] = frame.name if frame.is_file() and frame.stat().st_size > 32 else None
    record["pass"] = (record.get("client_exit_code") == 0 and "error" not in record
                      and all(record["frames"].values()))
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--scene", action="append", choices=sorted(SCENES), help="repeatable; default: all")
    parser.add_argument("--list", action="store_true", help="print the scenes and exit")
    parser.add_argument("--build", action="store_true", help="cargo build the dev workspace first")
    parser.add_argument("--client-bin", type=Path)
    parser.add_argument("--server-bin", type=Path)
    parser.add_argument("--assets", type=Path, default=ROOT / "client/assets")
    parser.add_argument("--timeout", type=int, default=240, help="seconds per scene")
    parser.add_argument("--lane", help="override the lane point of the selected match scenes, e.g. mid:0.5")
    parser.add_argument("--attempts", type=int, default=2, help="tries per scene")
    parser.add_argument("--hold", type=int, help="override the hold seconds of the selected lane scenes")
    args = parser.parse_args()
    if args.list:
        for name, scene in SCENES.items():
            width, height = scene["size"]
            print(f"{name:22} {scene['profile']:8} {width}x{height} {scene['harness']:6} "
                  f"hero={scene.get('hero', '-'):8} lane={scene.get('lane', '-'):9} -> {', '.join(scene['keep'].values())}")
        return 0
    if not args.output:
        parser.error("--output is required")
    if args.build:
        from package_native import build_executables
        binaries = build_executables("dev")
    else:
        if not args.client_bin or not args.server_bin:
            parser.error("pass --build or both --client-bin and --server-bin (a dev build)")
        binaries = dict(client=args.client_bin.resolve(), server=args.server_bin.resolve())
    assets = args.assets.resolve()
    if not assets.is_dir():
        parser.error(f"asset directory not found: {assets}")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)

    manifest = dict(schema_version=1, generator="scripts/capture_showcase.py",
                    captured_on=datetime.date.today().isoformat(), source=source_identity(),
                    host=dict(os=platform.platform(), machine=platform.machine()),
                    binaries={name: dict(path=str(path), sha256=sha256(path))
                              for name, path in binaries.items() if name in ("client", "server")},
                    method="Bevy Screenshot::primary_window + save_to_disk; production buttons and ordinary move orders",
                    images=[], scenes=[])
    for name in args.scene or list(SCENES):
        scene = dict(SCENES[name])
        if args.lane and scene["harness"] == "match":
            scene["lane"] = args.lane
        if args.hold is not None and scene.get("lane"):
            scene["hold"] = args.hold
        print(f"[showcase] {name}: {scene['profile']} {scene['size'][0]}x{scene['size'][1]} ...", flush=True)
        # A live match can kill or body-block the hero; one clean retry keeps a
        # full run unattended. Each attempt keeps its own raw folder.
        for attempt in range(1, args.attempts + 1):
            raw = output / "raw" / (name if attempt == 1 else f"{name}-attempt{attempt}")
            record = capture_scene(name, scene, binaries, assets, raw, args.timeout)
            record["attempt"] = attempt
            if record["pass"]:
                break
            print(f"[showcase] {name}: attempt {attempt} failed", flush=True)
        manifest["scenes"].append(record)
        print(f"[showcase] {name}: {'ok' if record['pass'] else 'FAILED'}"
              f"{' - ' + record['error'] if record.get('error') else ''}", flush=True)
        for published, frame in record["frames"].items():
            if not frame:
                continue
            target = output / published
            shutil.copyfile(raw / frame, target)
            width, height = scene["size"]
            manifest["images"].append(dict(
                file=published, scene=name, profile=scene["profile"], width=width, height=height,
                hero_class=scene.get("hero"), lane=scene.get("lane"), hold_seconds=scene.get("hold"),
                match_mode=scene.get("mode"), harness=scene["harness"], caption=scene["caption"],
                phone_preview_on_desktop=scene["profile"] == "phone",
                offline_shell=scene["harness"] == "shell",
                bytes=target.stat().st_size, sha256=sha256(target)))
    (output / "showcase-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    failed = [record["scene"] for record in manifest["scenes"] if not record["pass"]]
    print(f"[showcase] {len(manifest['images'])} images -> {output}"
          + (f"; failed: {', '.join(failed)} (see raw/<scene>/client.log)" if failed else ""))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
