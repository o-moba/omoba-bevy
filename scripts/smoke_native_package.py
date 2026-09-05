#!/usr/bin/env python3
"""Attempt isolated packaged native startup; does not certify visual gameplay."""
import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", type=Path)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--seconds", type=int, default=20)
    args = parser.parse_args()
    package = args.package.resolve()
    evidence = args.evidence.resolve()
    evidence.mkdir(parents=True, exist_ok=True)
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as reservation:
        reservation.bind(("127.0.0.1", 0))
        address = "127.0.0.1:" + str(reservation.getsockname()[1])
    with tempfile.TemporaryDirectory(prefix="omoba-native-smoke-") as isolated:
        env = dict(os.environ, SERVER_ADDR=address, GAME_SERVER_ADDR=address,
                   OMOBA_CLIENT_CONFIG_DIR=isolated, OMOBA_MATCH_MODE="dev")
        env.pop("OMOBA_MEASURE_MODELS", None)
        children = []
        try:
            for name in ("server", "client"):
                with (evidence / f"packaged-{name}.log").open("w") as log:
                    child = subprocess.Popen([str(package / f"launch-{name}.sh")],
                                             cwd=isolated, env=env, stdout=log, stderr=subprocess.STDOUT)
                children.append(child)
                if name == "server":
                    deadline = time.monotonic() + 10
                    while time.monotonic() < deadline and child.poll() is None:
                        if "is listening" in (evidence / "packaged-server.log").read_text():
                            break
                        time.sleep(0.1)
            time.sleep(max(1, args.seconds))
            server_alive, client_alive = [child.poll() is None for child in children]
        finally:
            for child in reversed(children):
                if child.poll() is None:
                    child.terminate()
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
        log = (evidence / "packaged-client.log").read_text()
        findings = dict(server_alive=server_alive, client_alive=client_alive,
                        seconds=args.seconds, source_independent_cwd=isolated,
                        isolated_config=True, snapshot_received="First snapshot received" in log,
                        package_asset_root=str(package / "assets") in log,
                        panic="panicked at" in log,
                        missing_assets="does not exist" in log or "Path not found" in log,
                        remote_model_fetch="Downloading" in log or "Downloading model" in log,
                        visual_gameplay="NOT_OBSERVED: startup and logs only")
        findings["startup_pass"] = all(findings[k] for k in
            ("server_alive", "client_alive", "snapshot_received", "package_asset_root")) and not any(
                findings[k] for k in ("panic", "missing_assets", "remote_model_fetch"))
        (evidence / "package-smoke.json").write_text(json.dumps(findings, indent=2) + "\n")
        print(json.dumps(findings))
        raise SystemExit(0 if findings["startup_pass"] else 1)


if __name__ == "__main__":
    main()
