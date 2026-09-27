#!/usr/bin/env python3
"""Capture every UI screen the QA harnesses can reach, for a UI/UX review.

Runs each screen harness on the desktop profile (1280x720) and the phone
profile (844x390, the phone UI rendered by a desktop development build with
OMOBA_TOUCH_CONTROLS=1), then files the frames by area:

    <output>/<profile>/<NN-area>/<frame>.png
    <output>/index.json          every frame with its run, area and overlay note
    <output>/raw/<profile>/<run>/ harness output, logs and summaries

    python3 scripts/capture_ui_audit.py --build --output /tmp/omoba-ui-audit
    python3 scripts/capture_ui_audit.py --client-bin ... --server-bin ... --output /tmp/a --run shell

Frames are real window readbacks. Some harnesses label fixture data on the
frame itself (career, target/scoreboard fixtures, result fixture); index.json
repeats those labels. Screens no harness reaches are listed in COVERAGE_GAPS.
"""
import argparse
import datetime
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
from capture_showcase import (ROOT, base_env, free_address, run_until_exit, sha256,  # noqa: E402
                              source_identity, stop)

PROFILES = {"desktop": (1280, 720), "phone": (844, 390)}

# area: folder the frames go to. server: match mode or None. env: harness switches
# ({out} is the raw directory). note: what the frames show beyond live UI.
RUNS = {
    "shell": dict(area="01-menus", server="dev", env={
        "OMOBA_FRONTEND_QA_OUTPUT": "{out}", "OMOBA_QA_CLEAN_FRAME": "1"},
        note="Front-end shell screens driven directly; searching/loading/post-match have no real match behind them."),
    "flow": dict(area="02-matchmaking", server="practice", env={
        "OMOBA_FRONTEND_QA_OUTPUT": "{out}", "OMOBA_FRONTEND_QA_FLOW": "1"},
        note="Real button flow into a practice match: draft, countdown, loading, first in-match frame."),
    "match": dict(area="03-in-match-hud", server="dev", env={
        "OMOBA_VISUAL_QA_DIR": "{out}", "OMOBA_VISUAL_QA_SCENARIO": "beta-ui", "OMOBA_BETA_UI_EDGE": "1",
        "OMOBA_BETA_UI_CLASS": "mage", "OMOBA_QA_TEAM": "green", "OMOBA_VISUAL_QA_TIMEOUT": "220"},
        # 0.25.0: the phone edge pass stops on its radial-geometry check (see the audit),
        # so phones get the base HUD/shop/result frames only.
        phone_env={"OMOBA_BETA_UI_EDGE": "0"},
        note="HUD, help, shop and a real purchase; result, target frames and scoreboard use labelled fixtures."),
    "social": dict(area="04-social-in-match", server="practice", env={
        "OMOBA_SOCIAL_QA_OUTPUT": "{out}"}, phone_env={"OMOBA_SOCIAL_QA_SKILL_HELP": "1"},
        note="Chat, reaction wheel, confirmed reaction; phone adds the skill description on hold."),
    "career": dict(area="05-career", server=None, env={"OMOBA_CAREER_QA_OUTPUT": "{out}"},
                   note="Profile, friends, history, match detail, account and devices with labelled fixture data."),
    "supporter": dict(area="06-supporter", server=None, env={"OMOBA_SUPPORTER_QA_DIR": "{out}"},
                      note="Supporter aura panel; the harness uses its own fixed viewport sizes."),
    "offline": dict(area="07-offline-practice", server=None, env={"OMOBA_OFFLINE_SMOKE_DIR": "{out}"},
                    note="Offline practice from Home: hero picker, match, target, game menu, settings."),
}
COVERAGE_GAPS = [
    "Party lobby with more than one member and party invite notifications (needs a scripted friend).",
    "Searching with real queue data and post-match with a real result (frontend frames are layout-only).",
    "Pause-menu debug tools page, debug console and OMOBA_DEBUG_UI toggles.",
    "Gamepad focus ring and controller legend (no harness injects gamepad input).",
    "Respawn countdown, defeat and rematch states, reconnecting/offline connection status.",
    "Hero-select join rejection notice; wallet/account connect flows beyond the static buttons.",
    "Avatar collection with a live Studio registry (needs an SDK fixture service).",
    "Combat Test sandbox panel (developer tool; needs sandbox launch arguments).",
    "Live lane fight HUD: see scripts/capture_showcase.py lane scenes and the demo recordings.",
    "Phone target frames and scoreboard: the phone edge pass stops on its radial-geometry check.",
]


def capture(run_name, run, profile, binaries, assets, raw, timeout):
    width, height = PROFILES[profile]
    if raw.exists():
        shutil.rmtree(raw)
    raw.mkdir(parents=True)
    record = dict(run=run_name, profile=profile, started=datetime.datetime.now().isoformat(timespec="seconds"))
    processes = []
    with tempfile.TemporaryDirectory(prefix=f"omoba-ui-audit-{run_name}-") as workdir:
        env = base_env(dict(size=(width, height), profile=profile), assets, workdir, raw)
        switches = dict(run["env"], **(run.get("phone_env", {}) if profile == "phone" else {}))
        env.update({key: value.format(out=raw) for key, value in switches.items()})
        try:
            if run["server"]:
                host, port = free_address()
                address = f"{host}:{port}"
                server_env = dict(env, SERVER_ADDR=address, OMOBA_MATCH_MODE=run["server"], OMOBA_TEAM_SIZE="5")
                server_env.update(run.get("server_env", {}))
                for key in [k for k in server_env if k.endswith(("_QA_DIR", "_QA_OUTPUT", "_SMOKE_DIR"))]:
                    del server_env[key]
                env.update(SERVER_ADDR=address, GAME_SERVER_ADDR=address)
                log = (raw / "server.log").open("w")
                processes.append(subprocess.Popen([str(binaries["server"])], cwd=workdir, env=server_env,
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
        except Exception as error:  # recorded; the remaining runs still go ahead
            record["error"] = str(error)
        finally:
            stop(processes)
    frames = sorted(p for p in raw.rglob("*.png") if p.stat().st_size > 32)
    if run.get("keep"):
        frames = [p for p in frames if any(p.name.startswith(k) for k in run["keep"])]
    record["frames"] = [str(p.relative_to(raw)) for p in frames]
    record["pass"] = record.get("client_exit_code") == 0 and "error" not in record
    return record, frames


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--run", action="append", choices=list(RUNS), help="repeatable; default: all")
    parser.add_argument("--profile", action="append", choices=list(PROFILES), help="repeatable; default: both")
    parser.add_argument("--build", action="store_true", help="cargo build the dev workspace first")
    parser.add_argument("--client-bin", type=Path)
    parser.add_argument("--server-bin", type=Path)
    parser.add_argument("--assets", type=Path, default=ROOT / "client/assets")
    parser.add_argument("--timeout", type=int, default=260, help="seconds per run")
    args = parser.parse_args()
    if args.build:
        from package_native import build_executables
        binaries = build_executables("dev")
    elif args.client_bin and args.server_bin:
        binaries = dict(client=args.client_bin.resolve(), server=args.server_bin.resolve())
    else:
        parser.error("pass --build or both --client-bin and --server-bin (a dev build)")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    index_path = output / "index.json"
    index = json.loads(index_path.read_text()) if index_path.exists() else dict(frames=[], runs=[])
    index.update(schema_version=1, generator="scripts/capture_ui_audit.py",
                 captured_on=datetime.date.today().isoformat(), source=source_identity(),
                 profiles={k: list(v) for k, v in PROFILES.items()}, coverage_gaps=COVERAGE_GAPS,
                 phone_note="Phone frames are the phone UI rendered by a desktop development build.")
    for profile in args.profile or list(PROFILES):
        for name in args.run or list(RUNS):
            run = RUNS[name]
            print(f"[ui-audit] {profile}/{name} ...", flush=True)
            record, frames = capture(name, run, profile, binaries, args.assets.resolve(),
                                     output / "raw" / profile / name, args.timeout)
            index["runs"] = [r for r in index["runs"] if not (r["run"] == name and r["profile"] == profile)]
            index["runs"].append(record)
            index["frames"] = [f for f in index["frames"] if not (f["run"] == name and f["profile"] == profile)]
            area = output / profile / run["area"]
            area.mkdir(parents=True, exist_ok=True)
            for frame in frames:
                target = area / (run.get("prefix", "") + frame.name)
                shutil.copyfile(frame, target)
                index["frames"].append(dict(file=str(target.relative_to(output)), run=name, profile=profile,
                                            area=run["area"], note=run["note"], sha256=sha256(target)))
            status = "ok" if record["pass"] else "FAILED"
            print(f"[ui-audit] {profile}/{name}: {status}, {len(frames)} frames"
                  f"{' - ' + record['error'] if record.get('error') else ''}", flush=True)
            index_path.write_text(json.dumps(index, indent=2) + "\n")
    failed = [f"{r['profile']}/{r['run']}" for r in index["runs"] if not r["pass"]]
    print(f"[ui-audit] {len(index['frames'])} frames -> {output}"
          + (f"; failed: {', '.join(failed)} (see raw/<profile>/<run>/client.log)" if failed else ""))
    return 0


if __name__ == "__main__":
    sys.exit(main())
