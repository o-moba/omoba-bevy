#!/usr/bin/env python3
"""Cold-cache SDK installs, real UDP admission and focused native pack captures."""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import time

import catalog
from capture_showcase import base_env, free_address, run_until_exit, sha256, source_identity, stop
from capture_verdant import FRAME_HEADER
from sdk_avatar_pack import start_catalog, write_json


class Peer:
    def __init__(self, address):
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.connect(address)
        self.sock.settimeout(.1)
        self.pending = {}

    def send(self, data):
        self.sock.send(json.dumps(data).encode())

    def packet(self):
        try:
            data = self.sock.recv(65536)
        except socket.timeout:
            return None
        if data.startswith(b'OMB1'):
            _, version, epoch, tick, index, count, total = FRAME_HEADER.unpack_from(data)
            assert version == catalog.protocol_version() and 0 <= index < count <= 56 and total <= 65507
            if len(self.pending) > 4:
                self.pending.clear()
            parts = self.pending.setdefault((epoch, tick), {})
            parts[index] = data[FRAME_HEADER.size:]
            if len(parts) != count:
                return None
            data = b''.join(parts[i] for i in range(count))
            del self.pending[(epoch, tick)]
            assert len(data) == total
        return json.loads(data)

    def join(self, slug, index, accepted=True):
        deadline, last_send = time.monotonic() + 15, 0
        try:
            while time.monotonic() < deadline:
                if time.monotonic() - last_send > .5:
                    self.send(dict(type='hello', protocol_version=catalog.protocol_version()))
                    self.send(dict(type='join', team='green', character='cube', hero_class='warrior',
                                   avatar=slug, session_id=f'sdk-pack-{os.getpid()}-{index}'))
                    last_send = time.monotonic()
                packet = self.packet()
                if not packet or packet.get('type') != 'snapshot':
                    continue
                own = next((p for p in packet.get('players', []) if p['id'] == packet.get('your_id')), None)
                if accepted and own and own.get('avatar') == slug:
                    assert own['hero_class'] == 'warrior'
                    return dict(slug=slug, admitted=True, player_id=own['id'], tick=packet['snapshot_tick'])
                if not accepted and packet.get('join_error') == 'avatar_not_authorized':
                    assert own is None
                    return dict(slug=slug, admitted=False, rejection=packet['join_error'])
            raise AssertionError(f'Admission timeout for {slug}')
        finally:
            self.send(dict(type='leave'))
            self.sock.close()


def verify_native(directory, entries, match_index, previews, weapon='forge-sword', remote_weapon=None):
    summary = json.loads((directory / 'qa-summary.json').read_text())
    frames = summary['captures']
    assert summary['pass'] and len(frames) == previews + 1 + int(remote_weapon is not None)
    preview_frames = frames[:previews]
    if previews:
        assert {f['slug'] for f in preview_frames} == {e['slug'] for e in entries}
        for frame in preview_frames:
            assert frame['store_ready'] and frame['scene_loaded'] and frame['preview_bound']
            assert frame['animation_advance_secs'] >= .15 and frame['bone_rotation_delta'] >= .0001
            assert frame['model'] == f"ekza://avatars/{frame['slug']}.glb"
            assert {'idle', 'walk', 'attack', 'cast', 'death'} <= set(frame['clips'])
    game = frames[previews]
    assert game['slug'] == entries[match_index]['slug'] and game['bound_to_model']
    assert game['scene_loaded'] and game['server_admitted'] and game['animation'] == 'Run'
    assert game['weapon'] == weapon and game['attachment_error'] < 1e-4 and game['distance'] >= .2
    result = dict(previews=previews, gameplay_avatar=game['name'], attachment_error=game['attachment_error'])
    if remote_weapon is not None:
        peer=frames[-1]
        assert peer['remote_player_id'] != peer['local_player_id']
        assert peer['weapon']==remote_weapon and peer['replicated_selection_verified'] and peer['scene_loaded'] and peer['bound_to_model']
        assert peer['model']==f"ekza://avatars/{peer['slug']}.glb"
        assert peer['weapon_model']==f"ekza://weapons/{remote_weapon}.glb"
        assert peer['animation_advance_secs'] >= .15 and peer['bone_rotation_delta'] >= .0001 and peer['distance'] >= .1
        assert peer['attachment_error'] < 1e-4
        result['remote']=peer
    for frame in frames:
        assert (directory / frame['file']).is_file()
    return result


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ('pack', 'output', 'client-bin', 'server-bin', 'import-bin'):
        p.add_argument('--' + name, type=Path, required=True)
    p.add_argument('--assets', type=Path, default=Path('client/assets'))
    p.add_argument('--data-only', action='store_true', help='Verify SDK/server paths only; leaves visual verification pending')
    args = p.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    pack = args.pack.resolve()
    entries = json.loads((pack / 'pack.json').read_text())['items']
    requests = []
    service, thread = start_catalog(pack, requests=requests)
    registry = f'http://127.0.0.1:{service.server_port}'
    env = dict(os.environ, OMOBA_REGISTRY_URL=registry)
    children = []
    result = dict(data_pass=False, visual_status='pending', source=source_identity(), local_developer_catalog=True,
                  public_studio=False, client_sha256=sha256(args.client_bin))
    result['pass'] = False
    try:
        with (output / 'install.log').open('w') as log:
            subprocess.run([str(args.import_bin.resolve()), str(output / 'sdk-cold-store'), str(output / 'installed.json')],
                           env=env, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=120)
        installed = json.loads((output / 'installed.json').read_text())['items']
        assert len(installed) == 20
        assert {(i['slug'], i['sha256']) for i in installed} == {(i['slug'], i['sha256']) for i in entries}
        for item in entries:
            assert f"/files/{item['sha256']}.glb" in requests
        host, port = free_address()
        game_env = dict(env, SERVER_ADDR=f'{host}:{port}', GAME_SERVER_ADDR=f'{host}:{port}',
                        OMOBA_MATCH_MODE='dev', OMOBA_ASSET_DIR=str(args.assets.resolve()))
        # No manifest override: admission comes from the server's SDK registry read.
        game_env.pop('OMOBA_AVATAR_MANIFEST', None)
        with (output / 'server.log').open('w') as log:
            children.append(subprocess.Popen([str(args.server_bin.resolve())], env=game_env, stdout=log, stderr=subprocess.STDOUT))
        deadline = time.monotonic() + 15
        while 'is listening' not in (output / 'server.log').read_text(errors='replace'):
            if children[0].poll() is not None or time.monotonic() > deadline:
                raise RuntimeError('Game server failed to start')
            time.sleep(.05)
        admitted = [Peer((host, port)).join(e['slug'], i) for i, e in enumerate(entries)]
        rejected = Peer((host, port)).join('ekza-' + 'a' * 64, 'unknown', accepted=False)
        # A fresh server cannot retain a formerly approved entry from its TTL cache.
        withdrawn, withdrawn_thread = start_catalog(pack, excluded_ids=[entries[0]['avatar_id']])
        revoked_process = None
        try:
            revoke_host, revoke_port = free_address()
            revoke_env = dict(game_env, SERVER_ADDR=f'{revoke_host}:{revoke_port}',
                              OMOBA_REGISTRY_URL=f'http://127.0.0.1:{withdrawn.server_port}')
            with (output / 'withdrawn-server.log').open('w') as log:
                revoked_process = subprocess.Popen([str(args.server_bin.resolve())], env=revoke_env, stdout=log, stderr=subprocess.STDOUT)
            deadline = time.monotonic() + 15
            while 'is listening' not in (output / 'withdrawn-server.log').read_text(errors='replace'):
                if revoked_process.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError('Withdrawal test server failed to start')
                time.sleep(.05)
            removed = Peer((revoke_host, revoke_port)).join(entries[0]['slug'], 'withdrawn', accepted=False)
        finally:
            if revoked_process:
                stop([revoked_process])
            withdrawn.shutdown(); withdrawn.server_close(); withdrawn_thread.join()
        write_json(output / 'admission.json', dict(accepted=admitted, unknown=rejected, withdrawn_on_fresh_server=removed))
        result.update(data_pass=True, sdk_installs=len(installed), admitted=len(admitted),
                      unknown_rejected=True, withdrawn_rejected=True)
        native = []
        for ordinal, match_index in enumerate(() if args.data_only else (0, 6, 17)):
            directory = output / f'native-{match_index:02}'
            directory.mkdir()
            native_env = base_env(dict(size=(1280, 720), profile='desktop'), args.assets.resolve(), directory, directory)
            native_env.update(OMOBA_REGISTRY_URL=registry, GAME_SERVER_ADDR=f'{host}:{port}',
                              OMOBA_LANGUAGE='en', OMOBA_SDK_PACK_QA_OUTPUT=str(directory),
                              OMOBA_SDK_PACK_QA_MANIFEST=str(pack / 'pack.json'),
                              OMOBA_SDK_PACK_QA_MATCH=str(match_index))
            if ordinal:
                native_env['OMOBA_SDK_PACK_QA_MATCH_ONLY'] = '1'
            with (directory / 'client.log').open('w') as log:
                process = subprocess.Popen([str(args.client_bin.resolve())], env=native_env, stdout=log, stderr=subprocess.STDOUT)
                children.append(process)
                # Previous native clients have exited successfully; only the
                # current client and its live server belong to this capture.
                assert run_until_exit([children[0], process], process, 260) == 0, directory
            native.append(verify_native(directory, entries, match_index, 20 if ordinal == 0 else 0))
        result.update(native=native, visual_status='pending' if args.data_only else 'pass')
        result['pass'] = not args.data_only
    except Exception as error:
        result['error'] = str(error)
        raise
    finally:
        stop(children)
        service.shutdown()
        service.server_close()
        thread.join()
        result['http_requests'] = requests
        write_json(output / 'evidence.json', result)
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
