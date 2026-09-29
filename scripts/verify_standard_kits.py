#!/usr/bin/env python3
"""Bounded, local two-client UDP proof for Dawnweaver and Wildspark.

Starts the supplied server on a fresh loopback port in the existing combat lab.
Sandbox packets only arrange level/resources/positions; tested combat always uses
ordinary CastSkill / BasicAttack packets. This is not a production balance, 5v5,
native-input or visual-UX test. No build, database or external network is used.
Raw received snapshots, sent requests and server output accompany result.json.
"""
from __future__ import annotations

import argparse
from collections import deque
import copy
import hashlib
import json
import os
from pathlib import Path
import select
import socket
import struct
import subprocess
import time


PROTOCOL = 3
HEADER = struct.Struct('<4sHQQHHI')
MAX_SNAPSHOT = 65507
CHUNK = 1170


def require(value, message):
    if not value:
        raise AssertionError(message)


class Peer:
    def __init__(self, address, name, team, hero, transcript):
        self.address, self.name, self.team, self.hero = address, name, team, hero
        self.session = 'standard-kits-' + name
        self.transcript = transcript
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.bind(('127.0.0.1', 0))
        self.sock.connect(address)
        self.sock.setblocking(False)
        self.last = None
        self.history = deque(maxlen=256)
        self.fragments = {}
        self.request = 0
        self.sandbox_request = 0
        self.basic_request = 0
        self.closed = False

    def close(self):
        if not self.closed:
            self.sock.close()
            self.closed = True

    def send(self, packet):
        self.transcript.write(json.dumps({'peer': self.name, 'direction': 'send',
                                         'packet': packet}) + '\n')
        self.transcript.flush()
        self.sock.send(json.dumps(packet, allow_nan=False).encode())

    def join(self, protocol=PROTOCOL, hero=None):
        self.send({'type': 'hello', 'protocol_version': protocol})
        self.send({'type': 'join', 'team': self.team, 'hero_class': hero or self.hero,
                   'character': 'cube', 'session_id': self.session})

    def receive(self):
        data = self.sock.recv(65536)
        if data.startswith(b'OMB1'):
            require(30 < len(data) <= 1200, 'Malformed snapshot frame size')
            _, version, epoch, tick, index, count, total = HEADER.unpack(data[:30])
            require(version == PROTOCOL and epoch > 0, 'Wrong framed protocol/epoch')
            require(0 < total <= MAX_SNAPSHOT and count == (total + CHUNK - 1) // CHUNK,
                    'Malformed framed snapshot total/count')
            require(index < count, 'Invalid fragment index')
            require(len(data[30:]) == min(CHUNK, total - index * CHUNK),
                    'Invalid fragment payload length')
            now = time.monotonic()
            self.fragments = {key: value for key, value in self.fragments.items()
                              if now - value[0] < 2}
            key = (epoch, tick)
            if key not in self.fragments and len(self.fragments) >= 4:
                del self.fragments[min(self.fragments, key=lambda k: self.fragments[k][0])]
            assembly = self.fragments.setdefault(key, (now, total, [None] * count))
            require(assembly[1] == total and len(assembly[2]) == count,
                    'Conflicting snapshot assembly')
            previous = assembly[2][index]
            require(previous is None or previous == data[30:], 'Conflicting fragment')
            assembly[2][index] = data[30:]
            if any(part is None for part in assembly[2]):
                return
            data = b''.join(assembly[2])
            del self.fragments[key]
        require(len(data) <= MAX_SNAPSHOT, 'Oversized snapshot')
        packet = json.loads(data)
        self.transcript.write(json.dumps({'peer': self.name, 'direction': 'receive',
                                         'packet': packet}) + '\n')
        if packet.get('type') != 'snapshot':
            return
        require(packet['protocol_version'] == PROTOCOL, 'Unexpected snapshot protocol')
        if self.last:
            require(packet['server_epoch'] == self.last['server_epoch'], 'Server restarted')
            if packet['snapshot_tick'] <= self.last['snapshot_tick']:
                return
        self.last = packet
        self.history.append(packet)

    def player(self, player_id=None):
        if not self.last:
            return None
        player_id = self.last['your_id'] if player_id is None else player_id
        return next((p for p in self.last['players'] if p['id'] == player_id), None)

    def envelope(self):
        require(self.last is not None, 'No authoritative match envelope')
        return {key: self.last[key] for key in ('server_epoch', 'match_id')}

    def cast(self, slot, aim, **overrides):
        self.request += 1
        packet = dict(type='cast_skill', slot=slot, aim=aim,
                      request_id=self.request, **self.envelope())
        packet.update(overrides)
        self.send(packet)
        return packet


class Proof:
    def __init__(self, process, peers, timeout):
        self.process, self.peers = process, peers
        self.deadline = time.monotonic() + timeout
        self.last_ping = 0.0
        self.rows = []

    def pump(self, seconds=.1, predicate=None, description='state change'):
        deadline = min(time.monotonic() + seconds, self.deadline)
        while time.monotonic() < deadline:
            require(self.process.poll() is None, 'Server exited during proof')
            live = [p for p in self.peers if not p.closed]
            now = time.monotonic()
            if now - self.last_ping >= .25:
                for peer in live:
                    peer.send({'type': 'ping'})
                self.last_ping = now
            ready, _, _ = select.select([p.sock for p in live], [], [], .01)
            for peer in live:
                if peer.sock in ready:
                    for _ in range(256):
                        try:
                            peer.receive()
                        except BlockingIOError:
                            break
                        except ConnectionRefusedError:
                            break  # Startup is bounded by the outer admission deadline.
            if predicate and predicate():
                return
        require(time.monotonic() < self.deadline, 'Overall proof deadline exceeded')
        if predicate:
            state = {p.name: p.player() for p in self.peers if not p.closed}
            raise AssertionError(f'Timed out: {description}; players={state}')

    def wait(self, predicate, description, timeout=4):
        self.pump(timeout, predicate, description)

    def passed(self, name, **evidence):
        self.rows.append(dict(name=name, status='PASS', **evidence))
        print('PASS: ' + name, flush=True)

    def sandbox(self, peer, command):
        peer.sandbox_request += 1
        request = dict(command=command, request_id=peer.sandbox_request, **peer.envelope())
        peer.send(dict(type='sandbox', request=request))
        self.wait(lambda: ((peer.last.get('sandbox') or {}).get('ack') or {}).get('request_id')
                  == request['request_id'], 'sandbox setup acknowledgement')
        ack = peer.last['sandbox']['ack']
        require(ack['accepted'], 'Sandbox setup rejected: ' + ack['message'])

    def prepare(self, peer):
        self.sandbox(peer, dict(action='refill', actor='player'))
        self.sandbox(peer, dict(action='reset_cooldowns', actor='player'))
        self.pump(.15)

    def accepted(self, peer, packet):
        self.wait(lambda: peer.player()['loadout']['cast_request_id'] == packet['request_id'],
                  f'{peer.hero} cast slot {packet["slot"]} accepted')

    def rejected(self, peer, packet, label):
        before = peer.player()
        mark = (before['action_sequence'], before['loadout']['weapon_mode'])
        peer.send(packet)
        self.pump(.35)
        after = peer.player()
        require((after['action_sequence'], after['loadout']['weapon_mode']) == mark,
                f'{label} executed an action or changed weapon mode')
        require(after['mana'] >= before['mana'] - .05, f'{label} spent mana')
        # Same-round attempts consume sequence even when they fail validation,
        # preventing delayed retry after cooldown, movement or respawn. Keep our
        # next fresh request above the server's processed-request high-water.
        peer.request = max(peer.request, after['loadout']['cast_request_id'])
        self.passed(label)

    def visible_effect(self, kind, skill, after_tick):
        return all(any(snapshot['snapshot_tick'] > after_tick and
                       any(effect['kind'] == kind and effect['skill'] == skill
                           for effect in snapshot.get('skill_effects', []))
                       for snapshot in peer.history)
                   for peer in self.peers if not peer.closed)

    def same_state(self, player_id):
        self.pump(.15)
        a, b = [p for p in self.peers if not p.closed]
        amap = {s['snapshot_tick']: s for s in a.history}
        common = [s for s in b.history if s['snapshot_tick'] in amap]
        require(common, 'Two UDP clients have no common snapshot tick')
        right = common[-1]
        left = amap[right['snapshot_tick']]
        pa = next(p for p in left['players'] if p['id'] == player_id)
        pb = next(p for p in right['players'] if p['id'] == player_id)
        for key in ('hero_class', 'hp', 'mana', 'action_sequence'):
            require(pa[key] == pb[key], f'Peer disagreement at tick {right["snapshot_tick"]}: {key}')
        # Recipe, mode and combat statuses are public, while processed request
        # counters belong only to the owning peer and must be redacted elsewhere.
        public_loadouts = []
        for peer, snapshot, player in ((a, left, pa), (b, right, pb)):
            loadout = player['loadout']
            require(loadout is not None, 'Expected standard-kit loadout state')
            expected_request = peer.request if snapshot['your_id'] == player_id else 0
            require(loadout['cast_request_id'] == expected_request,
                    f'Wrong private cast counter for {peer.name} at tick {snapshot["snapshot_tick"]}')
            public_loadouts.append({key: value for key, value in loadout.items()
                                    if key != 'cast_request_id'})
        require(public_loadouts[0] == public_loadouts[1],
                f'Public loadout disagreement at tick {right["snapshot_tick"]}')
        return right['snapshot_tick']


def actor(hero, position):
    return dict(hero=hero, avatar=None, level=6, xp=0, ranks=[1, 1, 1, 1],
                unlock_all=False, max_hp=1000.0, armor=0.0, resistance=0.0,
                move_speed=1.0, attack_speed=1.0, damage_multiplier=1.0,
                god_mode=False, infinite_resource=False, no_cooldowns=False,
                inventory=[], position=position)


def configure(proof, peer, position):
    config = dict(version=1, player=actor(peer.hero, position),
                  enemy=dict(enabled=False, actor=actor('warrior', [12.0, 0.0]),
                             behavior='stationary', aggression_range=20.0,
                             attack_distance=2.0, auto_respawn=False),
                  dummy=dict(enabled=False, max_hp=10000.0, armor=0.0, resistance=0.0,
                             infinite_hp=False, moving=False, position=[0.0, 12.0]),
                  environment=dict(minions=False, minions_paused=False,
                                   time_scale=1.0, paused=False))
    proof.sandbox(peer, dict(action='apply_config', config=config))
    proof.sandbox(peer, dict(action='teleport', actor='player', position=position))
    proof.prepare(peer)


def run(proof):
    dawn, wild = proof.peers
    dawn.join(protocol=PROTOCOL - 1)
    proof.wait(lambda: dawn.last is not None and dawn.last.get('join_error') == 'protocol_mismatch',
               'previous protocol rejected', 8)
    require(dawn.player() is None, 'Incompatible protocol admitted a player')
    proof.passed('previous_protocol_admission_rejected')
    for peer in proof.peers:
        peer.join()
        proof.wait(lambda: peer.player() is not None, f'{peer.hero} admission', 8)
        require(peer.last.get('join_error') is None, 'Admission rejected')
        require(peer.player()['hero_class'] == peer.hero, 'Class fell back to another preset')
        require(peer.player().get('loadout', {}).get('recipe'), 'Preset has no authoritative recipe')
    proof.wait(lambda: all(len(p.last['players']) == 2 for p in proof.peers), 'two joined peers')
    require(dawn.player()['team'] != wild.player()['team'], 'Expected opposing teams')
    proof.passed('two_fixed_presets_admitted', player_ids=[p.last['your_id'] for p in proof.peers])
    configure(proof, dawn, [-3.0, 0.0])
    configure(proof, wild, [3.0, 0.0])
    did, wid = dawn.last['your_id'], wild.last['your_id']

    # Invalid requests must not execute. Stale envelopes do not consume sequence;
    # same-round invalid attempts may advance the processed-request high-water.
    base = dict(type='cast_skill', slot=0, aim=[3.0, 0.0], request_id=900, **dawn.envelope())
    proof.rejected(dawn, dict(base, server_epoch=base['server_epoch'] + 1), 'stale_epoch_rejected')
    proof.rejected(dawn, dict(base, match_id=base['match_id'] + 1), 'stale_match_rejected')
    proof.rejected(dawn, dict(base, slot=4), 'invalid_slot_rejected')
    proof.rejected(dawn, dict(base, request_id=dawn.request + 1, aim=[1e9, 0.0]),
                   'unbounded_aim_rejected')

    hp = wild.player()['hp']
    tick = dawn.last['snapshot_tick']
    packet = dawn.cast(0, [3.0, 0.0])
    proof.accepted(dawn, packet)
    proof.wait(lambda: wild.player()['hp'] < hp and
               wild.player()['loadout']['root_remaining_secs'] > 0 and
               wild.player()['loadout']['mark_remaining_secs'] > 0, 'snare hit/root/mark')
    proof.wait(lambda: proof.visible_effect('bolt', 'dawn_bind', tick), 'snare visible to both peers')
    proof.passed('dawn_snare_damage_root_mark_and_replicated_bolt', hp_lost=hp-wild.player()['hp'],
                 matching_tick=proof.same_state(wid))
    proof.sandbox(dawn, dict(action='reset_cooldowns', actor='player'))
    proof.rejected(dawn, packet, 'duplicate_cast_rejected')

    hp = wild.player()['hp']
    dawn.basic_request += 1
    dawn.send(dict(type='basic_attack', target=dict(kind='player', id=wid),
                   request_id=dawn.basic_request, **dawn.envelope()))
    proof.wait(lambda: wild.player()['hp'] < hp and
               wild.player()['loadout']['mark_remaining_secs'] == 0,
               'basic attack consumes Radiance mark')
    proof.passed('dawn_basic_consumes_mark', hp_lost=hp-wild.player()['hp'])

    proof.prepare(dawn)
    hp = wild.player()['hp']
    packet = dawn.cast(0, [-3.0, 14.0])
    proof.accepted(dawn, packet)
    proof.pump(1.2)
    require(wild.player()['hp'] == hp, 'Aimed miss damaged off-axis target')
    proof.passed('dawn_snare_aimed_miss')

    proof.prepare(dawn)
    tick = dawn.last['snapshot_tick']
    packet = dawn.cast(1, [12.0, 0.0])
    proof.accepted(dawn, packet)
    proof.wait(lambda: dawn.player()['loadout']['shield_hp'] > 0, 'returning barrier self shield')
    proof.wait(lambda: proof.visible_effect('barrier', 'dawn_barrier', tick), 'barrier visible')
    proof.passed('dawn_barrier_shield_and_visible_travel', matching_tick=proof.same_state(did))
    proof.pump(2.8)  # Let both passes finish before later combat outcomes.

    proof.prepare(dawn)
    tick = dawn.last['snapshot_tick']
    packet = dawn.cast(2, [3.0, 0.0])
    proof.accepted(dawn, packet)
    proof.wait(lambda: dawn.player()['loadout']['slots'][2]['can_recast'] and
               wild.player()['loadout']['slow_multiplier'] < 1, 'field recast and slow')
    proof.wait(lambda: proof.visible_effect('field', 'dawn_field', tick), 'field visible')
    proof.wait(lambda: dawn.player()['skill_recovery_remaining_secs'] <= 0,
               'field initial shared recovery completed')
    mana, hp = dawn.player()['mana'], wild.player()['hp']
    packet = dawn.cast(2, [3.0, 0.0])
    proof.accepted(dawn, packet)
    proof.wait(lambda: not dawn.player()['loadout']['slots'][2]['can_recast'] and
               wild.player()['hp'] < hp, 'free field recast detonation')
    require(dawn.player()['mana'] >= mana - .05, 'Field recast consumed mana')
    proof.passed('dawn_field_slow_free_recast_and_damage', hp_lost=hp-wild.player()['hp'])
    proof.rejected(dawn, packet, 'duplicate_recast_rejected')

    proof.prepare(dawn)
    tick, hp = dawn.last['snapshot_tick'], wild.player()['hp']
    packet = dawn.cast(3, [3.0, 0.0])
    proof.accepted(dawn, packet)
    proof.wait(lambda: proof.visible_effect('beam_warning', 'dawn_ray', tick), 'warned beam')
    proof.wait(lambda: wild.player()['hp'] < hp and
               wild.player()['loadout']['mark_remaining_secs'] > 0, 'beam damage/reapplied mark')
    proof.passed('dawn_ray_warning_damage_and_mark', matching_tick=proof.same_state(wid))

    # Wildspark toggles, packet-stable rocket basic, targeted shot and armed trap.
    proof.prepare(wild)
    packet = wild.cast(0, [-3.0, 0.0])
    proof.accepted(wild, packet)
    proof.wait(lambda: wild.player()['loadout']['weapon_mode'] == 'rockets', 'rocket mode')
    require(wild.player()['loadout']['basic_attack_range'] > 11.5, 'Rocket range did not increase')
    proof.passed('wild_switch_to_rockets', matching_tick=proof.same_state(wid))
    proof.pump(.5)  # Replay gate is tested after the toggle's legitimate cooldown.
    proof.rejected(wild, packet, 'duplicate_toggle_rejected')
    require(wild.player()['loadout']['weapon_mode'] == 'rockets', 'Duplicate toggle changed mode')
    proof.pump(1.0)  # Expire any remaining returning-barrier shield.
    mana, hp = wild.player()['mana'], dawn.player()['hp']
    wild.basic_request += 1
    wild.send(dict(type='basic_attack', target=dict(kind='player', id=did),
                   request_id=wild.basic_request, **wild.envelope()))
    proof.wait(lambda: wild.player()['basic_attack_request_id'] == wild.basic_request,
               'rocket-mode basic attack accepted')
    # Observe the debit at acceptance: regeneration during projectile travel is
    # legitimate and must not turn a correct shot into a flaky mana assertion.
    require(wild.player()['mana'] < mana - 1, 'Rocket basic attack did not spend mana')
    proof.wait(lambda: dawn.player()['hp'] < hp, 'rocket-mode basic attack hits')
    proof.passed('wild_rocket_basic_damage_and_mana', hp_lost=hp-dawn.player()['hp'])

    proof.prepare(wild)
    tick, hp = wild.last['snapshot_tick'], dawn.player()['hp']
    packet = wild.cast(1, [-3.0, 0.0])
    proof.accepted(wild, packet)
    proof.wait(lambda: dawn.player()['hp'] < hp and
               dawn.player()['loadout']['slow_multiplier'] < 1, 'shockline damage/slow')
    proof.wait(lambda: proof.visible_effect('bolt', 'wild_zap', tick), 'shockline visible')
    proof.passed('wild_shockline_damage_slow_and_visible_bolt')

    proof.prepare(wild)
    tick, hp = wild.last['snapshot_tick'], dawn.player()['hp']
    packet = wild.cast(2, [-3.0, 0.0])
    proof.accepted(wild, packet)
    proof.wait(lambda: proof.visible_effect('trap', 'wild_traps', tick), 'traps visible')
    require(dawn.player()['hp'] == hp, 'Traps damaged before arming')
    proof.wait(lambda: dawn.player()['hp'] < hp and
               dawn.player()['loadout']['root_remaining_secs'] > 0, 'armed trap damage/root')
    proof.passed('wild_traps_arm_then_damage_root', matching_tick=proof.same_state(did))

    proof.prepare(wild)
    tick, hp = wild.last['snapshot_tick'], dawn.player()['hp']
    packet = wild.cast(3, [-3.0, 0.0])
    proof.accepted(wild, packet)
    proof.wait(lambda: proof.visible_effect('rocket', 'wild_rocket', tick), 'ultimate rocket visible')
    proof.wait(lambda: dawn.player()['hp'] < hp, 'ultimate rocket hero impact')
    proof.passed('wild_ultimate_visible_rocket_and_hero_damage', hp_lost=hp-dawn.player()['hp'])

    # Reconnect must restore accepted combat state, even if Join requests another kit.
    before = copy.deepcopy(wild.player())
    session = wild.session
    wild.close()
    proof.pump(5.8)
    replacement = Peer(dawn.address, 'wild-reconnect', 'blue', 'wildspark', dawn.transcript)
    replacement.session = session
    proof.peers[1] = replacement
    replacement.join(hero='warrior')
    proof.wait(lambda: replacement.player() is not None and
               replacement.player()['id'] == wid, 'session reconnect', 5)
    restored = replacement.player()
    # The existing sandbox protocol preserves its own per-player request
    # high-water across endpoint changes, independently of the cast sequence.
    replacement.sandbox_request = replacement.last['sandbox']['last_request_id']
    replacement.request = restored['loadout']['cast_request_id']
    replacement.basic_request = restored['basic_attack_request_id']
    require(restored['hero_class'] == 'wildspark', 'Reconnect replaced committed kit')
    for key in ('recipe', 'weapon_mode', 'cast_request_id'):
        require(restored['loadout'][key] == before['loadout'][key],
                f'Reconnect lost authoritative {key}')
    proof.passed('reconnect_retains_kit_mode_and_replay_state', matching_tick=proof.same_state(wid))
    proof.sandbox(replacement, dict(action='reset_cooldowns', actor='player'))
    packet['aim'] = [-3.0, 0.0]
    proof.rejected(replacement, packet, 'reconnect_duplicate_cast_rejected')


def stop(process):
    if process is not None and process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--server-binary', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--timeout', type=int, default=120)
    args = parser.parse_args()
    if not 45 <= args.timeout <= 240:
        parser.error('--timeout must be 45..240 seconds')
    if not args.server_binary.is_file():
        parser.error('Build the server binary before running this verifier')
    evidence = args.evidence.resolve()
    if evidence.exists() and any(evidence.iterdir()):
        parser.error('Evidence directory must be new or empty; retain previous failures')
    evidence.mkdir(parents=True, exist_ok=True)
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as reserved:
        reserved.bind(('127.0.0.1', 0))
        address = ('127.0.0.1', reserved.getsockname()[1])
    # Inherit no account, worker, database, proxy or other OMOBA production settings.
    env = {key: value for key, value in os.environ.items()
           if key in ('PATH', 'HOME', 'TMPDIR', 'SYSTEMROOT', 'LANG')}
    env.update(SERVER_ADDR=f'{address[0]}:{address[1]}', OMOBA_MATCH_MODE='dev',
               OMOBA_TEAM_SIZE='1', OMOBA_COMBAT_SANDBOX='1')
    result = dict(status='FAIL', protocol_version=PROTOCOL, participants=2,
                  mode='local_combat_sandbox', ordinary_cast_packets=True,
                  normal_balance=False, scope='transport and authoritative combat smoke',
                  address=list(address), checks=[],
                  checker_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                  server_sha256=hashlib.sha256(args.server_binary.read_bytes()).hexdigest())
    process, proof = None, None
    started = time.monotonic()
    try:
        with (evidence / 'server.log').open('w') as log, \
             (evidence / 'transport.jsonl').open('w') as raw:
            process = subprocess.Popen([str(args.server_binary.resolve())], cwd=evidence,
                                       env=env, stdout=log, stderr=subprocess.STDOUT)
            deadline = time.monotonic() + 8
            while 'Combat Sandbox enabled' not in (evidence / 'server.log').read_text():
                require(process.poll() is None, 'Server exited before local sandbox startup')
                require(time.monotonic() < deadline, 'Local sandbox startup timed out')
                time.sleep(.05)
            peers = [Peer(address, 'dawn', 'green', 'dawnweaver', raw),
                     Peer(address, 'wild', 'blue', 'wildspark', raw)]
            proof = Proof(process, peers, args.timeout)
            run(proof)
            result['status'] = 'PASS'
    except (AssertionError, OSError, ValueError, KeyError, TypeError, StopIteration) as error:
        result['error'] = f'{type(error).__name__}: {error}'
        print('FAIL: ' + result['error'], flush=True)
    finally:
        if proof:
            result['checks'] = proof.rows
            for peer in proof.peers:
                peer.close()
        stop(process)
        result['elapsed_seconds'] = round(time.monotonic() - started, 3)
        (evidence / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))
    return 0 if result['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
