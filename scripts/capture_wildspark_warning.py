#!/usr/bin/env python3
"""Record Last Spark from the defending native client in normal team vision.

The dev fixture places the two actors and unlocks the caster's R. The caster
sends an ordinary UDP cast and the defender receives unmodified server packets.
Review the native frames alongside the timed packet evidence.
"""

import argparse
from collections import deque
from contextlib import ExitStack
import json
import math
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time

import catalog
from capture_verdant import FRAME_HEADER, sha256
from qa_account_fixture import prepare_capture_account
from snapshot_payload import decode_snapshot_payload


def local_udp():
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind(('127.0.0.1', 0))
    sock.setblocking(False)
    return sock


def address_text(address):
    return f'{address[0]}:{address[1]}'


def datagrams(sock):
    while True:
        try:
            yield sock.recvfrom(65536)
        except BlockingIOError:
            return


class Snapshots:
    def __init__(self):
        self.protocol = catalog.protocol_version()
        self.pending = {}
        self.history = deque(maxlen=512)

    def accept(self, data, now):
        if data.startswith(b'OMB1'):
            _, protocol, epoch, tick, index, count, total = FRAME_HEADER.unpack_from(data)
            if protocol != self.protocol or not 0 <= index < count <= 56 or total > 65507:
                raise RuntimeError('Invalid local server snapshot frame')
            key = (epoch, tick)
            chunks = self.pending.setdefault(key, {})
            chunks[index] = data[FRAME_HEADER.size:]
            while len(self.pending) > 8:
                del self.pending[min(self.pending)]
            if len(chunks) != count:
                return
            del self.pending[key]
            data = b''.join(chunks[i] for i in range(count))
            if len(data) != total:
                raise RuntimeError('Incomplete snapshot payload')
        packet = json.loads(decode_snapshot_payload(data))
        if packet.get('type') == 'snapshot':
            self.history.append((now, packet))

    def own(self):
        if not self.history:
            return None
        packet = self.history[-1][1]
        return next((actor for actor in packet['players'] if actor['id'] == packet['your_id']), None)


def warning(packet, kind):
    return next((effect for effect in packet.get('skill_effects', [])
                 if effect['skill'] == 'wild_rocket' and effect['kind'] == kind), None)


def position(actor):
    return [actor['x'], actor['z']]


def clip_matches_sight(full, clipped, sight):
    def visible(point):
        return any(math.dist(point, zone['position']) <= zone['radius'] + .02 for zone in sight)

    origin, end = full['position'], full['end']
    dx, dz = end[0] - origin[0], end[1] - origin[1]
    length = math.hypot(dx, dz)
    if length <= .02 or visible(origin):
        return False

    def visible_on_segment(point):
        x, z = point[0] - origin[0], point[1] - origin[1]
        along = (x * dx + z * dz) / length
        across = abs(x * dz - z * dx) / length
        return -.02 <= along <= length + .02 and across < .02 and visible(point)

    return all(visible_on_segment(clipped[key]) for key in ('position', 'end'))


def verify(caster, defender, after, source, target):
    """Match both perspectives by authoritative tick and effect ID."""
    caster_ticks = {(s['server_epoch'], s['snapshot_tick']): s for t, s in caster if t >= after}
    result = {}
    for received, snap in defender:
        if received < after:
            continue
        effect = warning(snap, 'beam_warning')
        rocket = warning(snap, 'rocket')
        if effect and 'warning' not in result:
            original = caster_ticks.get((snap['server_epoch'], snap['snapshot_tick']))
            full = warning(original or {}, 'beam_warning')
            sight = (snap.get('vision') or {}).get('sources', [])
            if (full and full['id'] == effect['id'] and full['owner_id'] == source['id']
                    and effect['owner_id'] == 0 and not effect['armed']
                    and all(p['id'] != source['id'] for p in snap['players'])
                    and math.dist(full['position'], position(source)) < .05
                    and clip_matches_sight(full, effect, sight)):
                result['warning'] = dict(received=received, tick=snap['snapshot_tick'],
                                         full=full, defender=effect, sight=sight)
        if rocket and 'warning' in result and 'release' not in result:
            if (snap['snapshot_tick'] > result['warning']['tick']
                    and rocket['id'] == result['warning']['defender']['id'] and rocket['armed']):
                result['release'] = dict(received=received, tick=snap['snapshot_tick'], defender=rocket)
        if 'release' in result and snap['snapshot_tick'] >= result['release']['tick'] and 'hit' not in result:
            for event in snap.get('combat_events', []):
                if (event.get('target') == {'kind': 'player', 'id': target['id']}
                        and (event.get('area_impact') or {}).get('id') == result['release']['defender']['id']
                        and event.get('action_slot') == 3 and event.get('amount', 0) > 0):
                    result['hit'] = dict(received=received, tick=snap['snapshot_tick'], event=event)
                    break
    return result


def run(args):
    with ExitStack() as stack:
        isolated = Path(stack.enter_context(tempfile.TemporaryDirectory(prefix='omoba-warning-qa-')))
        prepare_capture_account(isolated)
        (isolated / 'config' / 'client_preferences.json').write_text(
            json.dumps({'beginner_tips': {'enabled': False}})
        )
        with local_udp() as reserved:
            address = reserved.getsockname()
        proxy = stack.enter_context(local_udp())
        green_socket = stack.enter_context(local_udp())
        blue_socket = stack.enter_context(local_udp())
        for sock in (green_socket, blue_socket):
            sock.connect(address)

        frames = args.output / 'frames'
        env = dict(os.environ, OMOBA_MATCH_MODE='dev', SERVER_ADDR=address_text(address),
                   OMOBA_TEAM_SIZE='1', OMOBA_VISION_QA='1',
                   OMOBA_VISION_QA_SCENARIO='wildspark-warning')
        env.pop('OMOBA_COMBAT_SANDBOX', None)
        client_env = dict(env, GAME_SERVER_ADDR=address_text(proxy.getsockname()),
                          OMOBA_ASSET_DIR=str(args.assets), OMOBA_CLIENT_CONFIG_DIR=str(isolated / 'config'),
                          OMOBA_AUTOJOIN='cleric:agnes:green', OMOBA_LANGUAGE='en', OMOBA_TOUCH_CONTROLS='0',
                          OMOBA_QA_OFFSCREEN='1', OMOBA_QA_WIDTH='1280', OMOBA_QA_HEIGHT='720',
                          OMOBA_RECORD_DIR=str(frames), OMOBA_RECORD_FPS='30', OMOBA_RECORD_STEP='0')

        def start(name, binary, environment):
            log = stack.enter_context((args.output / f'{name}.log').open('w'))
            child = subprocess.Popen([str(binary)], cwd=isolated, env=environment, stdout=log,
                                     stderr=subprocess.STDOUT)

            def stop():
                if child.poll() is None:
                    child.terminate()
                    try:
                        child.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        child.kill()
                        child.wait()
            stack.callback(stop)
            return child

        server = start('server', args.server_bin, env)
        deadline = time.monotonic() + 15
        while 'is listening' not in (args.output / 'server.log').read_text(errors='replace'):
            if server.poll() is not None or time.monotonic() >= deadline:
                raise RuntimeError('Local server did not start')
            time.sleep(.02)
        client = start('client', args.client_bin, client_env)
        blue, green = Snapshots(), Snapshots()
        native = frame_log = stable_since = source = target = cast = None
        last_keepalive = time.monotonic() - 2
        marks = []
        deadline = time.monotonic() + 40
        while time.monotonic() < deadline and client.poll() is None and server.poll() is None:
            now = time.monotonic()
            for data, native in datagrams(proxy):
                green_socket.send(data)
            for data, _ in datagrams(green_socket):
                proxy.sendto(data, native)
                green.accept(data, now)
            for data, _ in datagrams(blue_socket):
                blue.accept(data, now)
            if now - last_keepalive > 1:
                if blue.own() is None:
                    blue_socket.send(json.dumps(dict(type='hello', protocol_version=blue.protocol)).encode())
                    blue_socket.send(json.dumps(dict(type='join', team='blue', character='cube',
                                                    hero_class='wildspark', session_id='rocket-warning-qa')).encode())
                else:
                    blue_socket.send(b'{"type":"ping"}')
                last_keepalive = now
            if frame_log is None and (frames / 'frames.jsonl').is_file():
                frame_log = stack.enter_context((frames / 'frames.jsonl').open())
            if frame_log:
                while True:
                    offset = frame_log.tell()
                    line = frame_log.readline()
                    if not line.endswith('\n'):
                        frame_log.seek(offset)
                        break
                    marks.append(dict(json.loads(line), observed=now))
            # Start only after the native render cadence settles. Asset loading
            # can pause the game between its first few screenshot requests.
            frames_ready = (len(marks) >= 20 and now - marks[-1]['observed'] < .08
                            and .35 < marks[-1]['observed'] - marks[-20]['observed'] < 1.0)
            if frames_ready:
                if stable_since is None:
                    stable_since = now
            else:
                stable_since = None
            if cast is None and blue.own() and green.own() and stable_since is not None and now - stable_since >= 2:
                source, target = blue.own(), green.own()
                if source['level'] != 6 or not 32 < math.dist(position(source), position(target)) < 60:
                    raise RuntimeError(f'Unexpected fixture placement or caster level: {position(source)} / {position(target)} / {source["level"]}')
                state = blue.history[-1][1]
                packet = dict(type='cast_skill', slot=3, aim=position(target), request_id=1,
                              server_epoch=state['server_epoch'], match_id=state['match_id'])
                blue_socket.send(json.dumps(packet).encode())
                cast = now
            if cast is not None and now - cast >= 3:
                break
            time.sleep(.005)

        if cast is None:
            raise RuntimeError('Caster and defending client did not reach the fixture')
        phases = verify(blue.history, green.history, cast, source, target)
        blue_release = next((dict(received=received, tick=snap['snapshot_tick'])
                             for received, snap in blue.history
                             if received >= cast and warning(snap, 'rocket')), None)
        end = phases.get('hit', {}).get('received', cast + 2)
        gaps = [b['observed'] - a['observed'] for a, b in zip(marks, marks[1:])
                if b['observed'] >= cast and a['observed'] <= end]
        evidence = dict(pass_wire=len(phases) == 3, phases=phases, cast_observed=cast,
                        source=source, target=target, frames=marks,
                        blue_first_release=blue_release, pixels=[1280, 720],
                        max_frame_request_gap_secs=max(gaps, default=None),
                        physical_device_verified=False, locale='en')
        return evidence


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('client-bin', 'server-bin', 'assets', 'output'):
        parser.add_argument(f'--{name}', required=True, type=lambda path: Path(path).resolve())
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    if any(args.output.iterdir()):
        parser.error('Use an empty output directory')
    report = run(args)
    report['binaries_sha256'] = {name: sha256(path) for name, path in
                                (('client', args.client_bin), ('server', args.server_bin))}
    (args.output / 'capture.json').write_text(json.dumps(report, indent=2) + '\n')
    print(f'{args.output}: opponent wire phases {list(report["phases"])}')
    raise SystemExit(0 if report['pass_wire'] else 1)


if __name__ == '__main__':
    main()
