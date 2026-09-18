#!/usr/bin/env python3
"""Local demo of the Ekza avatar loop: publish -> buy -> wear in Omoba.

Devnet chain, local services. Nothing here deploys, signs or spends:
publishing reads the chain, buying happens in your own browser wallet.

    python3 scripts/ekza_demo.py publish --index 18 --reviewed-by "Dima"
    python3 scripts/ekza_demo.py serve        # registry + storefront/passport + game server
    python3 scripts/ekza_demo.py client       # one game window (run twice for two players)

Flow to show:
  1. artist: http://127.0.0.1:5191/deployer -> upload VRM -> template on devnet
  2. operator: `publish --index N` -> catalogue entry + Omoba rendition + approval
     (restart `serve` so the registry loads the new catalogue)
  3. buyer: http://127.0.0.1:5191/minter -> mint the avatar
  4. player: `client` -> "Connect Ekza wallet" -> approve in the browser ->
     the avatar appears under "Your Ekza avatars" -> pick it -> join
  5. second `client`: sees the first player wearing it (downloaded on demand)

Needs sibling checkouts `../ekza-mirror` (backend/.venv) and
`../solana-avatars` (app built with `npm run build`). Standard library only.
"""

from __future__ import annotations

import argparse
import os
import signal
import socket
import subprocess
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
UMBRELLA = REPO.parent
STATE = Path(os.environ.get("OMOBA_EKZA_DEMO_DIR", REPO / ".ekza-demo"))
REGISTRY_PORT = int(os.environ.get("EKZA_DEMO_REGISTRY_PORT", "8029"))
STORE_PORT = int(os.environ.get("EKZA_DEMO_STORE_PORT", "5191"))
GAME_ADDR = os.environ.get("EKZA_DEMO_GAME_ADDR", "127.0.0.1:4028")
REGISTRY = f"http://127.0.0.1:{REGISTRY_PORT}"
STORE = f"http://127.0.0.1:{STORE_PORT}"
RPC = os.environ.get("OMOBA_SOLANA_RPC_URL", "https://api.devnet.solana.com")


def game_env() -> dict:
    return {
        **os.environ,
        "OMOBA_PASSPORT_URL": f"{STORE}/api/passport",
        "OMOBA_REGISTRY_URL": REGISTRY,
    }


def port_free(port: int) -> bool:
    with socket.socket() as probe:
        return probe.connect_ex(("127.0.0.1", port)) != 0


def publish(args) -> int:
    command = [
        sys.executable, str(REPO / "scripts/ekza_publish.py"), "--rpc", RPC, "publish",
        "--catalog", str(STATE / "registry/catalog.json"),
        "--asset-dir", str(STATE / "registry/assets"),
        "--reviewed-by", args.reviewed_by,
    ]
    command += ["--index", str(args.index)] if args.index is not None else ["--pda", args.pda]
    for project in args.approve or ["omoba", "ekza-space"]:
        command += ["--approve", project]
    return subprocess.call(command, cwd=REPO)


def serve(_args) -> int:
    catalog = STATE / "registry/catalog.json"
    python = UMBRELLA / "ekza-mirror/backend/.venv/bin/python"
    store = UMBRELLA / "solana-avatars/app"
    problems = []
    if not catalog.is_file():
        problems.append("no catalogue yet: run `publish --index N --reviewed-by NAME` first")
    if not python.is_file():
        problems.append(f"registry environment missing: {python} (cd ekza-mirror/backend && uv sync)")
    if not (store / "build/server").is_dir():
        problems.append(f"storefront is not built: cd {store} && npm run build")
    for port in (REGISTRY_PORT, STORE_PORT):
        if not port_free(port):
            problems.append(f"port {port} is in use; stop the earlier demo first")
    if problems:
        print("\n".join(f"error: {p}" for p in problems), file=sys.stderr)
        return 1
    for name in ("published", "quarantine"):
        (STATE / "registry" / name).mkdir(parents=True, exist_ok=True)

    children = []

    def launch(command, cwd, env):
        child = subprocess.Popen(command, cwd=cwd, env={**os.environ, **env}, start_new_session=True)
        children.append(child)
        return child

    launch(
        [str(python), "-m", "uvicorn", "app.main:app", "--host", "127.0.0.1", "--port", str(REGISTRY_PORT)],
        UMBRELLA / "ekza-mirror/backend",
        {
            "EKZA_CATALOG_PATH": str(catalog),
            "EKZA_ASSET_DIR": str(STATE / "registry/assets"),
            "EKZA_PUBLISHED_DIR": str(STATE / "registry/published"),
            "EKZA_QUARANTINE_DIR": str(STATE / "registry/quarantine"),
            "EKZA_PUBLIC_BASE_URL": REGISTRY,
        },
    )
    launch(
        ["npm", "start"],
        store,
        {
            "PORT": str(STORE_PORT), "HOST": "127.0.0.1",
            "EKZA_PASSPORT_ORIGIN": STORE, "EKZA_PASSPORT_ALLOW_LOCALHOST": "1",
            "EKZA_PASSPORT_RPC_URL": RPC,
            "EKZA_PASSPORT_REGISTRY_URL": f"{REGISTRY}/v1/avatars",
        },
    )
    launch(
        ["cargo", "run", "-p", "server"],
        REPO,
        {**game_env(), "SERVER_ADDR": GAME_ADDR, "OMOBA_MATCH_MODE": "dev"},
    )
    print(
        f"\nRegistry  {REGISTRY}/v1/avatars\n"
        f"Storefront {STORE}  (publish: /deployer, buy: /minter)\n"
        f"Game server udp://{GAME_ADDR}\n"
        "Now run: python3 scripts/ekza_demo.py client   (Ctrl+C here stops everything)\n"
    )

    def stop(*_):
        for child in children:
            if child.poll() is None:
                try:
                    os.killpg(child.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass

    signal.signal(signal.SIGINT, stop)
    signal.signal(signal.SIGTERM, stop)
    try:
        while all(child.poll() is None for child in children):
            time.sleep(0.5)
    finally:
        stop()
    return 0


def client(args) -> int:
    env = {**game_env(), "GAME_SERVER_ADDR": GAME_ADDR}
    # Separate settings (and Ekza store) per player, so player two really has
    # to download player one's avatar.
    env["OMOBA_CLIENT_CONFIG_DIR"] = str(STATE / f"player-{args.player}")
    return subprocess.call(["cargo", "run", "-p", "client"], cwd=REPO, env=env)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    pub = commands.add_parser("publish", help="chain template -> demo registry catalogue")
    target = pub.add_mutually_exclusive_group(required=True)
    target.add_argument("--index", type=int)
    target.add_argument("--pda")
    pub.add_argument("--approve", action="append", metavar="PROJECT")
    pub.add_argument("--reviewed-by", required=True)
    commands.add_parser("serve", help="registry + storefront/passport + game server")
    play = commands.add_parser("client", help="one game window")
    play.add_argument("--player", type=int, default=1)
    args = parser.parse_args()
    return {"publish": publish, "serve": serve, "client": client}[args.command](args)


if __name__ == "__main__":
    sys.exit(main())
