#!/usr/bin/env python3
"""Run the real, existing local Studio rehearsal and Omoba on a private LAN.

Uses only the isolated Supabase project and its existing rehearsal accounts.
Does not deploy, migrate, reset a database, rotate credentials or perform purchases.
"""
from __future__ import annotations
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
import urllib.request

REPO = Path(__file__).resolve().parents[1]


def private_host(value):
    address = ipaddress.IPv4Address(value)
    if not any(address in ipaddress.ip_network(cidr) for cidr in ('10.0.0.0/8', '172.16.0.0/12', '192.168.0.0/16')):
        raise argparse.ArgumentTypeError('Use this Mac’s private LAN IPv4 address.')
    return str(address)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--host', required=True, type=private_host)
    parser.add_argument('--registry-repo', type=Path, default=REPO.parent/'ekza-registry')
    parser.add_argument('--studio-web', type=Path, help='Existing compiled Studio web directory')
    parser.add_argument('--rehearsal-state', type=Path, help='Existing local rehearsal private.json')
    parser.add_argument('--config-repo', type=Path, default=REPO, help='Omoba checkout opened in Xcode')
    parser.add_argument('--game-binary', type=Path, required=True, help='Current debug server executable')
    parser.add_argument('--api-port', type=int, default=8018)
    parser.add_argument('--web-port', type=int, default=5188)
    parser.add_argument('--game-port', type=int, default=4030)
    args = parser.parse_args()
    registry = args.registry_repo.resolve()
    web = (args.studio_web or registry/'web').resolve()
    state_path = args.rehearsal_state or registry/'.build/cross-app-20260922/private.json'
    private = json.loads(state_path.read_text())
    environment = private['env']
    if environment.get('EKZA_SUPABASE_URL') != 'http://127.0.0.1:55431':
        parser.error('Only the isolated local Studio Supabase project is supported.')
    python = registry/'backend/.venv/bin/python'
    if not python.is_file() or not (web/'build/server').is_dir() or not args.game_binary.is_file():
        parser.error('Build the existing Registry, Studio and debug game server first; see docs/ekza-lan.md.')
    for port, kind in ((args.api_port,socket.SOCK_STREAM),(args.web_port,socket.SOCK_STREAM),(args.game_port,socket.SOCK_DGRAM)):
        if not 1024 <= port <= 65535: parser.error('Use ports between 1024 and 65535.')
        with socket.socket(socket.AF_INET,kind) as check:
            check.bind(('0.0.0.0',port))
    api = f'http://{args.host}:{args.api_port}'
    studio = f'http://{args.host}:{args.web_port}'
    state = args.config_repo.resolve()/'.ekza-lan'
    state.mkdir(exist_ok=True,mode=0o700)
    config = {'development_host':args.host,'registry':api,'studio':studio,'game_server':f'{args.host}:{args.game_port}'}
    (state/'client.json').write_text(json.dumps(config,indent=2)+'\n')
    # Human-readable access to the existing local test accounts; never served
    # over HTTP or included in the committed game/configuration artifacts.
    descriptor = os.open(state/'test-accounts.txt', os.O_CREAT | os.O_WRONLY | os.O_TRUNC, 0o600)
    with os.fdopen(descriptor, 'w') as output:
        output.write('LOCAL TEST ACCOUNTS ONLY — Studio ' + studio + '/studio\n\n')
        for role in ('creator', 'reviewer', 'owner'):
            account = private['accounts'][role]
            output.write(f"{role}\nEmail: {account['email']}\nPassword: {account['password']}\n\n")
    # Credentials stay only in the API/worker environment. Neither Studio's
    # browser-facing process nor the game receives the Supabase service key.
    backend_env = {**environment,'EKZA_PUBLIC_BASE_URL':api,'EKZA_STUDIO_WEB_URL':studio,
                   'EKZA_STUDIO_DEV_HTTP_HOST':args.host}
    children=[]
    logs=[]
    base_env={k:v for k,v in os.environ.items() if not k.startswith(('EKZA_','SUPABASE_','OMOBA_'))}
    def start(name, command, cwd, extra):
        log=(state/f'{name}.log').open('a');logs.append(log)
        child=subprocess.Popen(command,cwd=cwd,env={**base_env,**extra},stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
        children.append(child)
    def stop(*_): raise KeyboardInterrupt
    signal.signal(signal.SIGINT,stop);signal.signal(signal.SIGTERM,stop)
    try:
        start('registry',[str(python),'-m','uvicorn','app.main:app','--host','0.0.0.0','--port',str(args.api_port)],registry/'backend',backend_env)
        start('studio',['npm','start'],web,{'PORT':str(args.web_port),'HOST':'0.0.0.0',
              'EKZA_STUDIO_API_URL':f'http://127.0.0.1:{args.api_port}/v1/studio','EKZA_STUDIO_PUBLIC_API_URL':api+'/v1/studio'})
        start('worker',[str(python),'-m','app.studio_cli','worker'],registry/'backend',backend_env)
        start('game',[str(args.game_binary.resolve())],args.config_repo.resolve(),{
              'SERVER_ADDR':f'0.0.0.0:{args.game_port}','OMOBA_MATCH_MODE':'practice',
              'OMOBA_REGISTRY_URL':api,'EKZA_DEV_HTTP_HOST':args.host})
        ready=False
        for _ in range(60):
            if any(child.poll() is not None for child in children): raise RuntimeError('A local service exited; inspect .ekza-lan logs.')
            try:
                with urllib.request.urlopen(api+'/v2/avatars?project=omoba&platform=desktop&profile=humanoid-glb-v1',timeout=1) as response:
                    json.load(response)
                with urllib.request.urlopen(studio+'/api/studio/status',timeout=1) as response:
                    status=json.load(response)
                if status.get('enabled') and status.get('database')=='postgresql': ready=True;break
            except (OSError,ValueError): pass
            time.sleep(1)
        if not ready: raise RuntimeError('Studio did not become ready; inspect .ekza-lan logs.')
        print(f'READY Studio: {studio}/studio\nRegistry: {api}\nOmoba: {args.host}:{args.game_port}\nXcode Debug config: {state / "client.json"}',flush=True)
        print('Existing local account credentials remain in the private rehearsal file. Ctrl+C stops only these services.',flush=True)
        while all(child.poll() is None for child in children): time.sleep(1)
        raise RuntimeError('A local service exited; inspect .ekza-lan logs.')
    except KeyboardInterrupt:
        pass
    finally:
        for child in children:
            if child.poll() is None: os.killpg(child.pid,signal.SIGTERM)
        for child in children:
            try: child.wait(timeout=5)
            except subprocess.TimeoutExpired: os.killpg(child.pid,signal.SIGKILL);child.wait()
        for log in logs: log.close()

if __name__=='__main__': main()
