#!/usr/bin/env python3
"""Run a packaged OMOBA beta. Uses only Python 3.9+ standard library."""
import argparse
import ipaddress
import os
from pathlib import Path
import re
import signal
import socket
import subprocess
import sys
import time
import uuid


def address(value):
    """Resolve an IPv4 UDP endpoint, matching the native transport's socket."""
    host, separator, port = value.rpartition(":")
    if not separator or not host or not port.isdecimal() or not 1 <= int(port) <= 65535:
        raise argparse.ArgumentTypeError("use HOST:PORT with a port between 1 and 65535")
    try:
        resolved = socket.getaddrinfo(host, int(port), socket.AF_INET, socket.SOCK_DGRAM)[0][4]
    except (socket.gaierror, UnicodeError) as error:
        raise argparse.ArgumentTypeError(f"cannot resolve IPv4 host {host!r}: {error}") from error
    return f"{resolved[0]}:{resolved[1]}"


def profile(value):
    if not re.fullmatch(r"[A-Za-z0-9_-]{1,48}", value):
        raise argparse.ArgumentTypeError("profile must be 1–48 letters, digits, underscores or dashes")
    return value


def arguments(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_subparsers(dest="action", required=True)
    join = actions.add_parser("join", help="join an existing host")
    join.add_argument("server", type=address, help="host's IPv4 address or hostname, e.g. 192.168.1.20:4000")
    join.add_argument("--profile", type=profile, default="player", help="use different names for simultaneous local players")
    for name, help_text in (("practice", "one local human plus nine bots"),
                            ("host", "host a release match for a known number of humans")):
        action = actions.add_parser(name, help=help_text)
        action.add_argument("--bind", type=address, default="127.0.0.1:4000" if name == "practice" else "0.0.0.0:4000")
        if name == "host":
            action.add_argument("--humans", type=int, choices=range(1, 11), required=True,
                                help="expected human players; fills remaining seats to ten with bots")
    return parser.parse_args(argv)


def stop(children):
    for child in reversed(children):
        if child.poll() is None:
            child.terminate()
    for child in reversed(children):
        try:
            child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait(timeout=5)


def run(args, package, *, executables=None, assets=None):
    """Supervise a session, optionally using source-built binaries and assets."""
    package = package.resolve()
    children, logs = [], []
    suffix = ".exe" if os.name == "nt" else ""
    required = ("client",) if args.action == "join" else ("server", "bots", "client")
    paths = ({name: package / (name + suffix) for name in required}
             if executables is None else {name: Path(path).resolve() for name, path in executables.items()})
    for binary in required:
        if binary not in paths or not paths[binary].is_file():
            if executables is not None:
                raise RuntimeError(f"missing built {binary}; Cargo must provide every required executable")
            raise RuntimeError(f"missing packaged {binary}; launch this from a complete native package")
    run_dir = package / "sessions" / (time.strftime("%Y%m%d-%H%M%S-") + uuid.uuid4().hex[:8])
    run_dir.mkdir(parents=True)
    print(f"Session logs: {run_dir}", flush=True)
    env = dict(os.environ, OMOBA_ASSET_DIR=str(Path(assets).resolve() if assets is not None else package / "assets"),
               OMOBA_PLAYER_VISUAL_MODE="models3d", OMOBA_DEBUG_UI="0",
               OMOBA_MATCH_MODE="release", OMOBA_TEAM_SIZE="5")
    # QA/developer switches must never silently change a tester's session.
    for key in ("OMOBA_AVATAR_MANIFEST", "OMOBA_VISUAL_QA_DIR", "OMOBA_BETA_UI_QA_DIR",
                "OMOBA_AUTOJOIN", "OMOBA_MEASURE_MODELS"):
        env.pop(key, None)

    def launch(name, extra=()):
        log = (run_dir / f"{name}.log").open("w")
        logs.append(log)
        child = subprocess.Popen([str(paths[name]), *extra], cwd=package,
                                 env=env, stdout=log, stderr=subprocess.STDOUT)
        children.append(child)
        return child

    try:
        if args.action == "join":
            if ipaddress.ip_address(args.server.rsplit(":", 1)[0]).is_unspecified:
                raise RuntimeError("join needs the host's real address, not 0.0.0.0")
            env["GAME_SERVER_ADDR"] = args.server
            env["OMOBA_CLIENT_CONFIG_DIR"] = str(package / "user-data" / args.profile)
            print(f"Joining {args.server}; profile {args.profile}. Choose a hero, then Join.", flush=True)
            client = launch("client")
            return client.wait()

        host, port = args.bind.rsplit(":", 1)
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
            probe.bind((host, int(port)))
        local_address = f"127.0.0.1:{port}" if host == "0.0.0.0" else args.bind
        env["SERVER_ADDR"] = args.bind
        env["GAME_SERVER_ADDR"] = local_address
        env["OMOBA_CLIENT_CONFIG_DIR"] = str(run_dir / "player")
        humans = 1 if args.action == "practice" else args.humans
        print(f"Release 5v5: waiting for {humans} human(s); starting {10 - humans} bot(s).", flush=True)
        server = launch("server")
        deadline = time.monotonic() + 15
        while "is listening" not in (run_dir / "server.log").read_text(errors="replace"):
            if server.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError(f"server did not start; see {run_dir / 'server.log'}")
            time.sleep(0.1)
        bots = launch("bots", ("--count", str(10 - humans), "--server", local_address)) if humans < 10 else None
        client = launch("client") if args.action == "practice" else None
        if not client:
            print(f"Host ready on {args.bind}. Testers: ./join-server.sh HOST_IP:{port}", flush=True)
            print("Leave this terminal open. Ctrl+C stops this host and its bots.", flush=True)
        while True:
            if server.poll() is not None or (bots and bots.poll() is not None):
                raise RuntimeError(f"server or bots exited unexpectedly; see {run_dir}")
            if client and client.poll() is not None:
                return client.returncode
            time.sleep(0.2)
    finally:
        stop(children)
        for log in logs:
            log.close()


def main():
    args = arguments()
    # On termination, unwind through cleanup instead of orphaning owned processes.
    def interrupted(_number, _frame):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, interrupted)
    try:
        return run(args, Path(__file__).resolve().parent)
    except KeyboardInterrupt:
        print("Session stopped.", file=sys.stderr)
        return 130
    except (OSError, RuntimeError) as error:
        print(f"Beta launch failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
