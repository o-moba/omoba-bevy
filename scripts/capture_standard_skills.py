#!/usr/bin/env python3
"""One English desktop viewport: live pilot or full-roster casts, no synthetic damage."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
import time

from capture_showcase import base_env, free_address, run_until_exit, sha256, source_identity, stop


def capture(hero, client, server, assets, output, timeout=135, roster=False):
    output.mkdir(parents=True)
    children = []
    result = dict(hero=hero, source=source_identity(), client_sha256=sha256(client),
                  locale="en", viewport=[1280, 720], physical_device_verified=False)
    with tempfile.TemporaryDirectory(prefix="omoba-skill-pilot-") as isolated:
        env = base_env(dict(size=(1280, 720), profile="desktop"), assets, isolated, output)
        host, port = free_address()
        env.update(SERVER_ADDR=f"{host}:{port}", GAME_SERVER_ADDR=f"{host}:{port}",
                   OMOBA_MATCH_MODE="dev", OMOBA_TEAM_SIZE="5", OMOBA_LANGUAGE="en",
                   OMOBA_COMBAT_SANDBOX="1")
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
            if roster:
                env["OMOBA_ROSTER_SKILLS_QA"] = "1"
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
    result["errors"] = [line for line in log.splitlines() if any(marker in line for marker in
                        ("panicked at", "does not exist", "Path not found", "STANDARD_KITS_QA stage="))]
    result["pass"] = (result.get("client_exit_code") == 0 and summary.get("pass") is True
                      and not result["errors"])
    (output / "capture-run.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--client-bin", type=Path, required=True)
    parser.add_argument("--server-bin", type=Path, required=True)
    parser.add_argument("--assets", type=Path, default=Path("client/assets"))
    parser.add_argument("--output", type=Path, required=True)
    roster_heroes = ["chainkeeper", "frostguard", "orbitwright", "cinderforge", "edgeweaver", "stormfist", "veilstalker", "emberveil", "riftshot", "warrior", "mage", "ranger", "cleric", "warden"]
    parser.add_argument("--hero", action="append", choices=["dawnweaver", "wildspark"] + roster_heroes)
    parser.add_argument("--roster", action="store_true", help="Capture all remaining classes, each Q/W/E/R accepted by live server")
    parser.add_argument("--timeout", type=float, default=135,
                        help="Per-client deadline in seconds (default: 135)")
    args = parser.parse_args()
    if not 0 < args.timeout <= 600:
        parser.error("--timeout must be between 0 and 600 seconds")
    client, server, assets, output = [p.resolve() for p in
                                    (args.client_bin, args.server_bin, args.assets, args.output)]
    if not client.is_file() or not server.is_file() or not assets.is_dir():
        parser.error("Binaries and assets must exist")
    heroes = args.hero or (roster_heroes if args.roster else ["dawnweaver", "wildspark"])
    if any((output / hero).exists() for hero in heroes):
        parser.error("Use a new output directory to preserve evidence")
    results = [capture(hero, client, server, assets, output / hero, args.timeout, args.roster or hero not in ["dawnweaver", "wildspark"]) for hero in heroes]
    print(json.dumps(results, indent=2))
    raise SystemExit(0 if all(r["pass"] for r in results) else 1)


if __name__ == "__main__":
    main()
