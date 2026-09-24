#!/usr/bin/env python3
"""Launch an isolated, unranked Combat Test server and client (or a direct duel peer)."""
import argparse
import ipaddress
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import time
import uuid

from beta_launcher import address, stop

ROOT = Path(__file__).resolve().parents[1]


def arguments(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    mode = p.add_mutually_exclusive_group()
    mode.add_argument('--connect', type=address, help='join an existing Combat Test host')
    mode.add_argument('--server-only', action='store_true', help='host without opening a client')
    p.add_argument('--bind', type=address, default='127.0.0.1:4040')
    p.add_argument('--hero', choices=('warrior', 'mage', 'ranger', 'cleric', 'warden'), help='skip the hero picker')
    p.add_argument('--avatar', help='shipped avatar slug; choose from the picker if omitted')
    p.add_argument('--preset', help='duel, late-game, dps, animation, or a saved JSON file')
    p.add_argument('--client-binary', type=Path, help='explicit prebuilt client; otherwise build locked sources')
    p.add_argument('--server-binary', type=Path, help='explicit prebuilt server; otherwise build locked sources')
    args = p.parse_args(argv)
    endpoint = args.connect or args.bind
    if not ipaddress.ip_address(endpoint.rsplit(":", 1)[0]).is_loopback:
        p.error("Combat Test is a local development host; use a loopback address")
    return args


def session_environment(parent, run_dir, endpoint, bind):
    # A shell configured for a live service must not give this test its database,
    # worker reservation, account or public matchmaking configuration.
    env = {k: v for k, v in parent.items()
           if not k.startswith(('OMOBA_', 'EKZA_')) and k not in ('SERVER_ADDR', 'GAME_SERVER_ADDR')}
    env.update(OMOBA_COMBAT_SANDBOX='1', OMOBA_MATCH_MODE='dev', OMOBA_TEAM_SIZE='1',
               OMOBA_DEBUG_UI='0', OMOBA_PLAYER_VISUAL_MODE='models3d', OMOBA_ASSET_DIR=str(ROOT / 'client/assets'),
               OMOBA_CLIENT_CONFIG_DIR=str(run_dir / 'client-data'),
               GAME_SERVER_ADDR=endpoint, SERVER_ADDR=bind)
    return env


def build(names):
    if not names:
        return {}
    command = ['cargo', 'build', '--locked', '--message-format=json-render-diagnostics']
    for name in sorted(names):
        command.extend(['-p', name, '--bin', name])
    result = {}
    with subprocess.Popen(command, cwd=ROOT, stdout=subprocess.PIPE, text=True) as process:
        try:
            for line in process.stdout:
                event = json.loads(line)
                if event.get('reason') == 'compiler-artifact' and event.get('executable'):
                    result[event['target']['name']] = Path(event['executable'])
            if process.wait() != 0 or not names <= result.keys():
                raise RuntimeError('Combat Test build failed; see compiler output above')
        finally:
            stop([process])
    return result


def run(args):
    needed = {'server'} if args.server_only else {'client'} if args.connect else {'server', 'client'}
    binaries = {name: getattr(args, name + '_binary') for name in needed}
    binaries.update(build({name for name, path in binaries.items() if path is None}))
    binaries = {name: path.resolve(strict=True) for name, path in binaries.items()}
    host, port = args.bind.rsplit(':', 1)
    if args.connect is None:
        # Never stop another user's process to make room for our test.
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
            probe.bind((host, int(port)))
    endpoint = args.connect or (f'127.0.0.1:{port}' if host == '0.0.0.0' else args.bind)
    if endpoint.startswith('0.0.0.0:'):
        raise RuntimeError('--connect needs the host address, not 0.0.0.0')
    run_dir = ROOT / 'target/combat-test' / (time.strftime('%Y%m%d-%H%M%S-') + uuid.uuid4().hex[:8])
    run_dir.mkdir(parents=True)
    env = session_environment(os.environ, run_dir, endpoint, args.bind)
    children, logs = [], []

    def launch(name, flags=()):
        log = (run_dir / (name + '.log')).open('w')
        logs.append(log)
        process = subprocess.Popen([str(binaries[name]), *flags], cwd=ROOT, env=env,
                                   stdout=log, stderr=subprocess.STDOUT)
        children.append(process)
        return process

    try:
        print(f'Combat Test: {endpoint}\nLogs: {run_dir}', flush=True)
        server = None
        if 'server' in needed:
            server = launch('server')
            deadline = time.monotonic() + 20
            while 'is listening' not in (run_dir / 'server.log').read_text(errors='replace'):
                if server.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError(f'Server failed to start; see {run_dir / "server.log"}')
                time.sleep(0.1)
        client = None
        if 'client' in needed:
            flags = ['--combat-test']
            for flag, value in (('--hero', args.hero), ('--avatar', args.avatar), ('--sandbox-preset', args.preset)):
                if value:
                    flags.extend([flag, value])
            client = launch('client', flags)
            print('Choose a hero, then Enter Combat Test. F6 toggles the Dev Panel.', flush=True)
        else:
            print(f'Host ready. Second client: python3 scripts/combat_test.py --connect 127.0.0.1:{port}', flush=True)
        while True:
            if server and server.poll() is not None:
                raise RuntimeError(f'Server exited; see {run_dir / "server.log"}')
            if client and client.poll() is not None:
                return client.returncode
            time.sleep(0.1)
    finally:
        stop(children)
        for log in logs:
            log.close()


def main(argv=None):
    args = arguments(argv)
    def interrupt(_signum, _frame):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, interrupt)
    try:
        return run(args)
    except KeyboardInterrupt:
        return 130
    except (OSError, RuntimeError) as error:
        print(f'Combat Test: {error}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
