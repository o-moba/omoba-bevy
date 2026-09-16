#!/usr/bin/env python3
"""Run the bundled Mac practice server and print its current LAN addresses."""
import argparse
import ipaddress
import os
from pathlib import Path
import socket
import subprocess
import sys

sys.path.append(str(Path(__file__).resolve().parents[2] / 'scripts'))
import beta_launcher


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--kit',type=Path,default=Path(__file__).resolve().parent,
                        help='Directory containing the compiled server')
    parser.add_argument('--port',type=int,default=4000)
    args = parser.parse_args(argv)
    if not 1024 <= args.port <= 65535:
        parser.error('Choose a UDP port between 1024 and 65535.')
    addresses = set()
    for _, interface in socket.if_nameindex():
        if not interface.startswith('en'): continue
        result = subprocess.run(['/usr/sbin/ipconfig','getifaddr',interface],capture_output=True,text=True)
        try: address = ipaddress.IPv4Address(result.stdout.strip())
        except ValueError: continue
        if not address.is_loopback and not address.is_link_local: addresses.add(str(address))
    print('\nConnect Mac and iPhone to the same home Wi-Fi.',flush=True)
    for address in sorted(addresses):
        print(f'On iPhone tap SERVER, enter {address}:{args.port}, then CONNECT.',flush=True)
    if not addresses:
        print(f'No active Ethernet/Wi-Fi IPv4 address found. Connect Wi-Fi, then restart this helper. Use the Mac LAN IP with :{args.port}.',flush=True)
    print('Allow local-network access on iPhone. Keep this window open; Ctrl+C stops only this server.\n',flush=True)
    for key in list(os.environ):
        if key.startswith('OMOBA_') or key == 'GAME_SERVER_ADDR': os.environ.pop(key)
    options = beta_launcher.arguments(['practice-server','--bind',f'0.0.0.0:{args.port}'])
    try:
        return beta_launcher.run(options,args.kit)
    except KeyboardInterrupt:
        print('Practice server stopped.')
        return 0


if __name__ == '__main__':
    try: raise SystemExit(main())
    except (OSError,RuntimeError) as error:
        print('Practice server:',error,file=sys.stderr)
        raise SystemExit(1)
