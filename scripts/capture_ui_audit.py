#!/usr/bin/env python3
"""Capture every mapped UI screen for a UI/UX review, and keep the map honest.

The screen map, scripts/ui_screen_map.json, lists every UI screen: how a player
reaches it, which harness run captures it (with its env), the frame name and
the profiles it exists on, plus the screens no harness reaches yet (gaps).

    # capture all runs on both profiles into a dated folder
    python3 scripts/capture_ui_audit.py --build --output ../omoba-ui/captures/2026-09-27
    # one run / one profile with prebuilt binaries
    python3 scripts/capture_ui_audit.py --client-bin ... --server-bin ... --output /tmp/a --run shell --profile phone
    # static check, no game needed: new AppScreens or harness frames missing from the map
    python3 scripts/capture_ui_audit.py --check
    # regenerate the human-readable map
    python3 scripts/capture_ui_audit.py --write-doc

Output: <output>/<profile>/<NN-area>/<frame>.png, <output>/index.json (every
frame with its screen id, title, route and fixture note) and raw harness output
under <output>/raw/<profile>/<run>/. Phone frames are the phone UI rendered by a
desktop development build (OMOBA_TOUCH_CONTROLS=1).
"""
import argparse
import datetime
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
from capture_showcase import (ROOT, base_env, free_address, run_until_exit, sha256,  # noqa: E402
                              source_identity, stop)

MAP_PATH = ROOT / "scripts/ui_screen_map.json"
DOC_PATH = ROOT / "docs/ui-screens.md"
FRAME_LITERAL = re.compile(r'"([a-z0-9][a-z0-9-]*\.png)"')


def load_map(path=MAP_PATH):
    return json.loads(Path(path).read_text())


def frame_name(template, profile, screen_map):
    return template.replace("{h}", str(screen_map["profiles"][profile][1]))


def expected_frames(screen_map, run_name, profile):
    """Frame file name -> screen for one run on one profile."""
    return {frame_name(s["frame"], profile, screen_map): s for s in screen_map["screens"]
            if s["run"] == run_name and profile in s["profiles"]}


def check_map(screen_map, root=ROOT):
    """Static consistency check; returns a list of problems (empty = fine)."""
    problems = []
    runs, screens = screen_map["runs"], screen_map["screens"]
    ids = [s["id"] for s in screens] + [g["id"] for g in screen_map["gaps"]]
    for dup in sorted({i for i in ids if ids.count(i) > 1}):
        problems.append(f"duplicate id {dup!r}")
    for s in screens:
        if s["run"] not in runs:
            problems.append(f"screen {s['id']!r} uses unknown run {s['run']!r}")
        if not set(s["profiles"]) <= set(screen_map["profiles"]) or not s["profiles"]:
            problems.append(f"screen {s['id']!r} has invalid profiles {s['profiles']}")
    for entry in screens + screen_map["gaps"]:
        if not (root / entry["source"]).exists():
            problems.append(f"{entry['id']!r} points at missing source {entry['source']}")

    # Every AppScreen variant must be a mapped screen or a known gap.
    enum = (root / "client/src/frontend/mod.rs").read_text()
    body = enum[enum.index("pub enum AppScreen"):]
    body = body[:body.index("}")]
    variants = re.findall(r"^\s+([A-Z][A-Za-z]+),", body, re.M)
    mapped = {s.get("app_screen") for s in screens}
    for variant in variants:
        if variant not in mapped:
            problems.append(f"AppScreen::{variant} has no screen in the map")

    # Every frame literal in a run's harness sources is mapped (or ignored), and
    # every mapped frame still exists in those sources.
    for run_name, run in runs.items():
        literals = set()
        for source in run["sources"]:
            path = root / source
            if not path.exists():
                problems.append(f"run {run_name!r} source missing: {source}")
                continue
            literals |= {name.replace("720p", "{h}p") for name in FRAME_LITERAL.findall(path.read_text())}
        mapped_frames = {s["frame"] for s in screens if s["run"] == run_name}
        ignored = set(run.get("ignore", {}))
        for name in sorted(literals - mapped_frames - ignored):
            problems.append(f"run {run_name!r}: harness frame {name} is not in the map")
        for name in sorted(mapped_frames - literals):
            problems.append(f"run {run_name!r}: mapped frame {name} no longer appears in {run['sources']}")

    # Every QA module is either a run source or explained in other_harnesses.
    known = {src for run in runs.values() for src in run["sources"]} | set(screen_map["other_harnesses"])
    for path in sorted((root / "client/src/qa").glob("*.rs")):
        rel = str(path.relative_to(root))
        if rel.endswith("/mod.rs"):
            continue
        if rel not in known:
            problems.append(f"QA module {rel} is neither a run source nor listed in other_harnesses")
    return problems


def render_doc(screen_map):
    lines = ["# UI screens", "",
             "Generated from `scripts/ui_screen_map.json` by `python3 scripts/capture_ui_audit.py --write-doc`;",
             "edit the JSON, not this file. Capture them all with `make ui-audit`.", ""]
    for run_name, run in screen_map["runs"].items():
        lines += [f"## {run['area']} · `{run_name}`", "", run["note"], "",
                  f"Harness: `{run['harness']}`; server: {run['server'] or 'none'}; "
                  f"env: `{' '.join(f'{k}={v}' for k, v in run['env'].items())}`"
                  + (f"; phone adds `{' '.join(f'{k}={v}' for k, v in run['phone_env'].items())}`"
                     if run.get("phone_env") else ""), "",
                  "| Screen | Frame | Profiles | How a player gets there | Code | Note |",
                  "| --- | --- | --- | --- | --- | --- |"]
        for s in [s for s in screen_map["screens"] if s["run"] == run_name]:
            lines.append(f"| {s['title']} | `{s['frame']}` | {', '.join(s['profiles'])} | {s['route']} | "
                         f"`{s['source']}` | {s.get('fixture', '')} |")
        lines.append("")
    lines += ["## Not captured yet", "", "| Screen | How a player gets there | Code | What a capture needs |",
              "| --- | --- | --- | --- |"]
    for g in screen_map["gaps"]:
        lines.append(f"| {g['title']} | {g['route']} | `{g['source']}` | {g['needs']} |")
    lines += ["", "## Other QA harnesses (not UI screens)", ""]
    lines += [f"- `{path}`: {why}" for path, why in screen_map["other_harnesses"].items()]
    return "\n".join(lines) + "\n"


def capture(run_name, run, profile, binaries, assets, raw, timeout, screen_map):
    width, height = screen_map["profiles"][profile]
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
    expected = expected_frames(screen_map, run_name, profile)
    written = {p.name: p for p in raw.rglob("*.png") if p.stat().st_size > 32}
    record["missing"] = sorted(set(expected) - set(written))
    record["unmapped"] = sorted(set(written) - set(expected))
    record["pass"] = (record.get("client_exit_code") == 0 and "error" not in record and not record["missing"])
    return record, {name: (written[name], expected[name]) for name in expected if name in written}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--run", action="append", help="repeatable; default: every run in the map")
    parser.add_argument("--profile", action="append", choices=["desktop", "phone"], help="repeatable; default: both")
    parser.add_argument("--build", action="store_true", help="cargo build the dev workspace first")
    parser.add_argument("--client-bin", type=Path)
    parser.add_argument("--server-bin", type=Path)
    parser.add_argument("--assets", type=Path, default=ROOT / "client/assets")
    parser.add_argument("--timeout", type=int, default=260, help="seconds per run")
    parser.add_argument("--check", action="store_true", help="static map check only (no game)")
    parser.add_argument("--write-doc", action="store_true", help=f"regenerate {DOC_PATH.relative_to(ROOT)}")
    args = parser.parse_args()
    screen_map = load_map()
    if args.check or args.write_doc:
        if args.write_doc:
            DOC_PATH.write_text(render_doc(screen_map))
            print(f"[ui-audit] wrote {DOC_PATH.relative_to(ROOT)}")
        problems = check_map(screen_map)
        for problem in problems:
            print(f"[ui-audit] map: {problem}")
        print(f"[ui-audit] map: {len(screen_map['screens'])} screens, {len(screen_map['gaps'])} gaps, "
              f"{'OK' if not problems else f'{len(problems)} problem(s)'}")
        return 1 if problems else 0
    if not args.output:
        parser.error("--output is required (or use --check / --write-doc)")
    unknown = set(args.run or []) - set(screen_map["runs"])
    if unknown:
        parser.error(f"unknown run(s): {', '.join(sorted(unknown))}; see {MAP_PATH.name}")
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
    index.update(schema_version=2, generator="scripts/capture_ui_audit.py",
                 captured_on=datetime.date.today().isoformat(), source=source_identity(),
                 profiles=screen_map["profiles"], gaps=screen_map["gaps"],
                 phone_note="Phone frames are the phone UI rendered by a desktop development build.")
    for profile in args.profile or list(screen_map["profiles"]):
        for name in args.run or list(screen_map["runs"]):
            run = screen_map["runs"][name]
            print(f"[ui-audit] {profile}/{name} ...", flush=True)
            record, frames = capture(name, run, profile, binaries, args.assets.resolve(),
                                     output / "raw" / profile / name, args.timeout, screen_map)
            index["runs"] = [r for r in index["runs"] if not (r["run"] == name and r["profile"] == profile)]
            index["runs"].append(record)
            index["frames"] = [f for f in index["frames"] if not (f["run"] == name and f["profile"] == profile)]
            area = output / profile / run["area"]
            area.mkdir(parents=True, exist_ok=True)
            for frame, (source, screen) in sorted(frames.items()):
                target = area / frame
                shutil.copyfile(source, target)
                index["frames"].append(dict(
                    file=str(target.relative_to(output)), screen=screen["id"], title=screen["title"],
                    route=screen["route"], code=screen["source"], fixture=screen.get("fixture"),
                    run=name, profile=profile, area=run["area"], sha256=sha256(target)))
            status = "ok" if record["pass"] else "FAILED"
            extra = []
            if record["missing"]:
                extra.append(f"missing {', '.join(record['missing'])}")
            if record["unmapped"]:
                extra.append(f"not in the map: {', '.join(record['unmapped'])}")
            if record.get("error"):
                extra.append(record["error"])
            print(f"[ui-audit] {profile}/{name}: {status}, {len(frames)} frames"
                  + (f" ({'; '.join(extra)})" if extra else ""), flush=True)
            index_path.write_text(json.dumps(index, indent=2) + "\n")
    failed = [f"{r['profile']}/{r['run']}" for r in index["runs"] if not r["pass"]]
    print(f"[ui-audit] {len(index['frames'])} frames -> {output}"
          + (f"; incomplete: {', '.join(failed)} (see raw/<profile>/<run>/client.log)" if failed else ""))
    return 0


if __name__ == "__main__":
    sys.exit(main())
