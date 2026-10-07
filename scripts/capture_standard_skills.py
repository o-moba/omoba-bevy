#!/usr/bin/env python3
"""One English desktop viewport: live pilot, full-roster casts or three stills per skill.

No synthetic damage: a local dev server runs the Combat Sandbox and the client
sends ordinary cast commands. Examples (binaries are a dev build of this tree):

    python3 scripts/capture_standard_skills.py --phases --offscreen \\
        --client-bin target/debug/client --server-bin target/debug/server \\
        --output /tmp/skills --hero stormfist
    python3 scripts/build_skill_contact_sheets.py /tmp/skills

`--phases` writes `0-idle.png` and, per skill, `<n>-<key>-1-windup.png`,
`-2-release.png` and `-3-impact.png` (or `-3-settled.png` when no damage receipt
arrived). `--offscreen` hides the window and renders to an image, so a locked
or covered desktop still gives frames. `--skillfx` / `--combat-visuals` overlay
one registry file on a private copy of the asset `config/` directory. The
output directory receives one folder per class and a merged `manifest.json`.

For a look at motion clips, `--avatar SLUG` stages the hero on another shipped
rig and `--release-at contact|SECONDS` moves the release still of skills
without a telegraph to the clip's contact time or to a fixed time after the
accepted cast. For a look at projectile bodies, `--flight` stands the target of
every unit-target ability far enough for its projectile to be in flight at the
release still and adds `5-basic-flight.png`, the basic attack's projectile on
its way (not for a melee core, which throws nothing). Every still lists the
hero's projectiles and what stands for each of them.
"""
import argparse
import datetime
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import time

from capture_showcase import base_env, free_address, run_until_exit, sha256, source_identity, stop

# Catalog order (shared/assets/catalog/heroes.json).
HEROES = ["warrior", "mage", "ranger", "cleric", "warden", "dawnweaver", "wildspark", "cinderforge",
          "edgeweaver", "stormfist", "veilstalker", "emberveil", "orbitwright", "riftshot", "chainkeeper",
          "frostguard", "adventurer"]
PILOT_HEROES = ["dawnweaver", "wildspark"]
# The single-frame roster run never included the Adventurer (its own harness covers it).
ROSTER_HEROES = ["chainkeeper", "frostguard", "orbitwright", "cinderforge", "edgeweaver", "stormfist",
                 "veilstalker", "emberveil", "riftshot", "warrior", "mage", "ranger", "cleric", "warden"]
SLOT_KEYS = ["q", "w", "e", "r"]
OVERLAYS = {"skillfx": "skills.skillfx", "combat_visuals": "combat_visuals.json"}
ERROR_MARKERS = ("panicked at", "does not exist", "Path not found", "STANDARD_KITS_QA stage=",
                 "Skill presentation unavailable")
# Per-still fields the merged manifest keeps (the full state stays in qa-summary.json).
STILL_FIELDS = ("file", "phase", "gate", "slot", "skill", "since_edge_secs", "animation", "profile",
                "particles", "receipt", "damage_numbers", "effect_parts", "effect_visible_parts",
                "effect_lights", "registry_profiles", "mean_pixel", "hero_pixels", "target_pixels",
                "projectiles")


def overlay_assets(assets, workdir, overlays):
    """A private asset root: links to `assets`, with `config/` copied and the given files replaced."""
    root = Path(workdir) / "assets"
    root.mkdir()
    for entry in sorted(Path(assets).iterdir()):
        if entry.name == "config":
            shutil.copytree(entry, root / "config")
        else:
            os.symlink(entry, root / entry.name, target_is_directory=entry.is_dir())
    for name, source in overlays.items():
        shutil.copyfile(source, root / "config" / OVERLAYS[name])
    return root


def phase_problems(summary, directory):
    """Why a phase run is not usable evidence (empty list = fine)."""
    problems = []
    stills = [c for c in summary.get("captures", []) if c.get("phase")]
    for still in stills:
        path = Path(directory) / still["file"]
        if not path.is_file() or path.stat().st_size <= 32:
            problems.append(f"{still['file']} is missing")
        if not (still.get("mean_pixel") or 0) > 0:
            problems.append(f"{still['file']} is black or was not read back")
    if [s["file"] for s in stills if s["phase"] == "idle"] != ["0-idle.png"]:
        problems.append("expected exactly one idle baseline")
    for slot, key in enumerate(SLOT_KEYS):
        phases = [s["phase"] for s in stills if s["phase"] != "idle" and s.get("slot") == slot]
        if sorted(phases) not in (["impact", "release", "windup"], ["release", "settled", "windup"]):
            problems.append(f"slot {key} has stills {phases}, expected windup, release and impact or settled")
    if len(summary.get("skills", [])) != len(SLOT_KEYS):
        problems.append("expected four skill records")
    # A flight look ends with the basic attack; its record names the still.
    flights = [s["file"] for s in stills if s["phase"] == "flight"]
    if flights != (summary.get("basic") or {}).get("stills", []):
        problems.append(f"flight stills {flights} do not match the basic attack record")
    return problems


def capture(hero, client, server, assets, output, timeout=135, roster=False, handhelds=False, sdk_weapon=None,
            phases=False, offscreen=False, visual_mode="models3d", overlays=None, avatar=None, release_at=None,
            flight=False):
    output.mkdir(parents=True)
    children = []
    result = dict(hero=hero, source=source_identity(), client_sha256=sha256(client),
                  locale="en", viewport=[1280, 720], physical_device_verified=False,
                  phases=phases, offscreen=offscreen, visual_mode=visual_mode,
                  avatar=avatar, release_at=release_at, flight=flight)
    with tempfile.TemporaryDirectory(prefix="omoba-skill-pilot-") as isolated:
        if overlays:
            assets = overlay_assets(assets, isolated, overlays)
        env = base_env(dict(size=(1280, 720), profile="desktop"), assets, isolated, output)
        host, port = free_address()
        env.update(SERVER_ADDR=f"{host}:{port}", GAME_SERVER_ADDR=f"{host}:{port}",
                   OMOBA_MATCH_MODE="dev", OMOBA_TEAM_SIZE="5", OMOBA_LANGUAGE="en",
                   OMOBA_COMBAT_SANDBOX="1", OMOBA_PLAYER_VISUAL_MODE=visual_mode)
        try:
            with (output / "server.log").open("w") as log:
                children.append(subprocess.Popen([str(server)], cwd=isolated, env=env,
                                                  stdout=log, stderr=subprocess.STDOUT))
            deadline = time.monotonic() + 20
            while "is listening" not in (output / "server.log").read_text(errors="replace"):
                if children[0].poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError("Local server did not start")
                time.sleep(.05)
            env.update(OMOBA_STANDARD_QA_DIR=str(output), OMOBA_STANDARD_QA_CLASS=hero)
            if phases:
                env["OMOBA_STANDARD_QA_PHASES"] = "1"
                if avatar:
                    env["OMOBA_STANDARD_QA_AVATAR"] = avatar
                if release_at:
                    env["OMOBA_STANDARD_QA_RELEASE_AT"] = release_at
                if flight:
                    env["OMOBA_STANDARD_QA_FLIGHT"] = "1"
            elif roster or handhelds:
                env["OMOBA_ROSTER_SKILLS_QA"] = "1"
            if offscreen:
                env["OMOBA_STANDARD_QA_OFFSCREEN"] = "1"
            if handhelds:
                env["OMOBA_HANDHELD_QA"] = "1"
                if sdk_weapon:
                    env["OMOBA_HANDHELD_QA_IMPORT"] = sdk_weapon
            with (output / "client.log").open("w") as log:
                process = subprocess.Popen([str(client)], cwd=isolated, env=env,
                                           stdout=log, stderr=subprocess.STDOUT)
                children.append(process)
            result["client_exit_code"] = run_until_exit(children, process, timeout)
            if result["client_exit_code"] is None:
                result["error"] = f"Client did not exit within {timeout:g}s"
        except Exception as error:
            result["error"] = str(error)
        finally:
            stop(children)
    summary_path = output / "qa-summary.json"
    summary = json.loads(summary_path.read_text()) if summary_path.exists() else {}
    log = (output / "client.log").read_text(errors="replace") if (output / "client.log").exists() else ""
    result["errors"] = [line for line in log.splitlines() if any(marker in line for marker in ERROR_MARKERS)]
    if phases and summary:
        result["errors"] += phase_problems(summary, output)
        result["registry_profiles"] = summary.get("registry_profiles")
        result["skills"] = summary.get("skills", [])
        result["basic"] = summary.get("basic")
        result["stills"] = [{field: capture.get(field) for field in STILL_FIELDS}
                            for capture in summary.get("captures", []) if capture.get("phase")]
    result["pass"] = (result.get("client_exit_code") == 0 and summary.get("pass") is True
                      and not result["errors"])
    (output / "capture-run.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


def merge_manifest(path, header, results):
    """Add this run's classes to the output directory's manifest and return it."""
    manifest = json.loads(path.read_text()) if path.exists() else dict(schema_version=1, classes={})
    manifest.update(header)
    manifest["classes"].update({result["hero"]: result for result in results})
    manifest["pass"] = all(result["pass"] for result in manifest["classes"].values())
    path.write_text(json.dumps(manifest, indent=2) + "\n")
    return manifest


def avatar_slugs(assets):
    """Slugs of the rigs shipped in an asset root."""
    manifest = json.loads((Path(assets) / "avatars" / "manifest.json").read_text())
    return [avatar["slug"] for avatar in manifest["avatars"]]


def release_time(value):
    """`contact`, or seconds after the accepted cast."""
    if value != "contact" and not 0 <= float(value) <= 2:
        raise argparse.ArgumentTypeError("expected `contact` or 0..2 seconds")
    return value


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--client-bin", type=Path, required=True)
    parser.add_argument("--server-bin", type=Path, required=True)
    parser.add_argument("--assets", type=Path, default=Path("client/assets"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--hero", action="append", choices=HEROES)
    parser.add_argument("--handhelds", action="store_true", help="Warrior: equip sword/hammer/scepter, run, attack, swap avatar, unequip")
    parser.add_argument("--roster", action="store_true", help="Capture all remaining classes, each Q/W/E/R accepted by live server")
    parser.add_argument("--phases", action="store_true",
                        help="Three state-gated stills per skill (windup, release, impact or settled); default: all 17 classes")
    parser.add_argument("--offscreen", action="store_true",
                        help="Hidden window, main camera rendered to an image (works on a locked desktop)")
    parser.add_argument("--visual-mode", choices=["models3d", "sprite2d"], default="models3d")
    parser.add_argument("--skillfx", type=Path, help="Overlay this file as config/skills.skillfx")
    parser.add_argument("--combat-visuals", type=Path, help="Overlay this file as config/combat_visuals.json")
    parser.add_argument("--avatar", help="With --phases: the hero's rig (a shipped avatar slug; default agnes)")
    parser.add_argument("--release-at", type=release_time, metavar="contact|SECONDS",
                        help="With --phases: take the release still of a skill without a telegraph at its "
                             "clip's contact time, or this long after the accepted cast (default: 0.10 s)")
    parser.add_argument("--flight", action="store_true",
                        help="With --phases: unit-target abilities are cast from lane distance and the basic "
                             "attack gets a still of its projectile in flight")
    parser.add_argument("--timeout", type=float,
                        help="Per-client deadline in seconds (default: 135, or 300 with --phases)")
    args = parser.parse_args(argv)
    if args.phases and (args.roster or args.handhelds):
        parser.error("--phases replaces --roster and --handhelds")
    if (args.avatar or args.release_at or args.flight) and not args.phases:
        parser.error("--avatar, --release-at and --flight need --phases")
    if args.timeout is None:
        args.timeout = 300 if args.phases else 135
    if not 0 < args.timeout <= 600:
        parser.error("--timeout must be between 0 and 600 seconds")
    if args.phases and args.timeout < 300:
        parser.error("--phases needs --timeout of at least 300 seconds (the harness stops itself at 280)")
    client, server, assets, output = [p.resolve() for p in
                                    (args.client_bin, args.server_bin, args.assets, args.output)]
    if not client.is_file() or not server.is_file() or not assets.is_dir():
        parser.error("Binaries and assets must exist")
    overlays = {name: path.resolve() for name, path in
                (("skillfx", args.skillfx), ("combat_visuals", args.combat_visuals)) if path}
    for path in overlays.values():
        if not path.is_file():
            parser.error(f"Overlay file not found: {path}")
    if args.avatar and args.avatar not in avatar_slugs(assets):
        parser.error(f"Unknown avatar: {args.avatar}")
    if args.handhelds:
        heroes = ["warrior"]
    elif args.phases:
        heroes = args.hero or HEROES
    else:
        heroes = args.hero or (ROSTER_HEROES if args.roster else PILOT_HEROES)
        if "adventurer" in heroes:
            parser.error("adventurer needs --phases (the single-frame runs do not cover it)")
    if any((output / hero).exists() for hero in heroes):
        parser.error("Use a new output directory to preserve evidence")
    output.mkdir(parents=True, exist_ok=True)
    # The shared cargo cache can be rebuilt by another worktree during a long
    # run: every class of this run uses the same private copies.
    with tempfile.TemporaryDirectory(prefix="binaries-", dir=output) as private:
        binaries = {name: Path(shutil.copy2(source, Path(private) / name))
                    for name, source in (("client", client), ("server", server))}
        header = dict(generator="scripts/capture_standard_skills.py",
                      captured_on=datetime.date.today().isoformat(), source=source_identity(),
                      host=dict(os=platform.platform(), machine=platform.machine()),
                      binaries={name: dict(source=str(source), sha256=sha256(binaries[name]))
                                for name, source in (("client", client), ("server", server))},
                      assets=str(assets),
                      overlays={OVERLAYS[name]: dict(path=str(path), sha256=sha256(path))
                                for name, path in overlays.items()},
                      locale="en", viewport=[1280, 720], phases=args.phases, offscreen=args.offscreen,
                      visual_mode=args.visual_mode, avatar=args.avatar, release_at=args.release_at,
                      flight=args.flight)
        results = [capture(hero, binaries["client"], binaries["server"], assets, output / hero, args.timeout,
                           args.roster or hero not in PILOT_HEROES, args.handhelds, phases=args.phases,
                           offscreen=args.offscreen, visual_mode=args.visual_mode, overlays=overlays,
                           avatar=args.avatar, release_at=args.release_at, flight=args.flight)
                   for hero in heroes]
    merge_manifest(output / "manifest.json", header, results)
    print(json.dumps([{key: result.get(key) for key in ("hero", "pass", "client_exit_code", "error", "errors")}
                      for result in results], indent=2))
    raise SystemExit(0 if all(r["pass"] for r in results) else 1)


if __name__ == "__main__":
    main()
