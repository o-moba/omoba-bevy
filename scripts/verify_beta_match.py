#!/usr/bin/env python3
"""Run two ordinary 5v5 UDP bot rounds, keeping raw transport and server evidence.

No game-rule overrides: production release mode, five players per team, normal
structure health, waves, clock and movement. Bots use the shipped fill-bot brain.
This proves automated lifecycle play, not human UX or Internet performance.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import time

STARTING_GOLD = 80
ITEM_COSTS = dict(ember_blade=80, swift_grip=80, trail_boots=80,
                  vitality_gem=80, focus_charm=100, guardian_crest=120)
BASE_ITEM_BONUSES = dict(damage_multiplier=1.0, attack_speed_multiplier=1.0,
                        move_speed_multiplier=1.0, spell_haste_multiplier=1.0,
                        max_hp=0.0, max_mana=0.0)


class TelemetryLineBuffer:
    """A growing regular file can expose one stdout line across several reads."""
    def __init__(self):
        self.pending = ''

    def push(self, chunk: str) -> str | None:
        self.pending += chunk
        assert len(self.pending) <= 262144, 'Unbounded or corrupted telemetry line'
        if not self.pending.endswith('\n'):
            return None
        complete, self.pending = self.pending, ''
        return complete


if not __debug__:
    raise SystemExit("Full-match verification requires assertions: remove Python -O/PYTHONOPTIMIZE")


class MatchProof:
    def __init__(self, timeout: float):
        self.timeout = timeout
        self.rounds: list[dict] = []
        self.roster: list[int] | None = None
        self.last_tick: dict[int, int] = {}
        self.samples = 0
        self.max_snapshot_age_ms = 0
        self.initial_structures: dict[int, dict] = {}
        self.epoch = None

    def observe(self, sample: dict) -> bool:
        self.samples += 1
        meta = sample['meta']
        match_id, epoch, tick = meta['match_id'], meta['server_epoch'], meta['snapshot_tick']
        assert meta.get('protocol_version', 1) == 1, 'Unexpected snapshot protocol'
        if self.epoch is None:
            self.epoch = epoch
        assert epoch == self.epoch, 'Server epoch changed unexpectedly'
        assert tick > self.last_tick.get(match_id, -1), 'Snapshot tick did not advance'
        self.last_tick[match_id] = tick
        assert sample['join_error'] == 'None', sample['join_error']
        if sample['phase'] not in ('starting', 'running', 'victory'):
            return False
        players = sample['players']
        ids = sorted(player['id'] for player in players)
        assert len(players) == 10 and len(set(ids)) == 10, f'Expected exactly ten unique admitted players: {ids}'
        for team in ('Some(Blue)', 'Some(Green)'):
            assert sum(p['team'] == team for p in players) == 5, f'Wrong roster: {players}'
        if self.roster is None:
            self.roster = ids
        assert self.roster == ids, 'Admitted identity changed/disconnected'
        peers = sample['peers']
        assert len(peers) == 10, 'Driver does not have ten independent peers'
        # The first snapshot can arrive before the other socket readers run.
        if sample['elapsed_secs'] > 2:
            assert sorted(p['id'] for p in peers if p['id'] is not None) == ids, 'A UDP peer lost admission'
        for peer in peers:
            if peer['snapshot_age_ms'] is not None:
                self.max_snapshot_age_ms = max(self.max_snapshot_age_ms, peer['snapshot_age_ms'])
                assert peer['snapshot_age_ms'] < 5000, f'Sustained peer snapshot stall: {peer}'
        if not self.rounds or self.rounds[-1]['match_id'] != match_id:
            if self.rounds:
                assert self.rounds[-1]['victory'] is not None, 'Reset before victory'
                assert match_id > self.rounds[-1]['match_id'], 'Match id regressed'
            assert sample['phase'] == 'starting', 'Did not observe real countdown before Running'
            assert len(sample['structures']) == 8, 'Expected six towers and two bases'
            if not self.initial_structures:
                self.initial_structures = {s['id']: s.copy() for s in sample['structures']}
            assert sum(s['protected'] for s in sample['structures']) == 2, 'Bases must start protected'
            assert all(s['hp'] == self.initial_structures[s['id']]['hp'] for s in sample['structures']), 'Structures were not reset to production HP'
            assert sample['minions'] == 0 and sample['buffs'] == 0, 'Round retained waves or team buffs'
            for player in players:
                assert player['level'] == 1 and player['xp'] == 0 and player['gold'] == STARTING_GOLD, 'Progression leaked into rematch'
                assert player['inventory'] == [] and player['last_purchase'] is None, 'Equipment or purchase receipt leaked into rematch'
                assert player['item_bonuses'] == BASE_ITEM_BONUSES, 'Item bonuses leaked into rematch'
                assert player['ranks'] == [1, 1, 1, 1], 'Ability ranks leaked into rematch'
                assert player['hp'] == player['max_hp'] == 100 and player['mana'] == player['max_mana'] == 100, 'Resources not reset'
            self.rounds.append(dict(match_id=match_id, countdown=sample['elapsed_secs'], running=None,
                                    victory=None, winner=None, objectives=[], progression={},
                                    offensive_slots=[], purchases=[], clean_reset=True, final_structures=None))
        current = self.rounds[-1]
        if sample['phase'] == 'running' and current['running'] is None:
            current['running'] = sample['elapsed_secs']
            assert current['running'] - current['countdown'] >= 1.5, 'Countdown unexpectedly accelerated'
            print(f'Round {match_id}: ordinary release 5v5 Running', flush=True)
        if current['running'] is not None:
            elapsed = sample['elapsed_secs'] - current['running']
            if current['victory'] is None:
                assert elapsed <= self.timeout, f'Round {match_id} timed out after {elapsed:.1f}s'
            else:
                assert sample['elapsed_secs'] - current['victory'] <= 30, 'Rematch did not start within thirty seconds'
            for level in (2, 4, 6):
                for which, reached in [('first', any(p['level'] >= level for p in players)),
                                       ('all', all(p['level'] >= level for p in players))]:
                    key = f'level_{level}_{which}'
                    if reached and key not in current['progression']:
                        current['progression'][key] = elapsed
            for player in players:
                inventory = player['inventory']
                assert len(inventory) <= 6 and len(set(inventory)) == len(inventory), 'Invalid inventory capacity or duplicate items'
                assert all(item in ITEM_COSTS for item in inventory), 'Unknown replicated item'
                receipt = player['last_purchase']
                if receipt and receipt['error'] is None:
                    assert receipt['match_id'] == match_id, 'Purchase receipt is from the wrong round'
                    assert receipt['item_id'] in inventory, 'Successful purchase is absent from inventory'
                    key = (player['id'], receipt['request_id'])
                    if not any((p['player_id'], p['request_id']) == key for p in current['purchases']):
                        current['purchases'].append(dict(player_id=player['id'], request_id=receipt['request_id'],
                            item_id=receipt['item_id'], cost=ITEM_COSTS[receipt['item_id']], elapsed_secs=elapsed))
                offensive = player['action_slot'] == 0 or (
                    player.get('class') in ('warrior', 'mage', 'ranger')
                    and player['action_slot'] in (2, 3))
                if player['action_sequence'] and offensive:
                    if player['action_slot'] not in current['offensive_slots']:
                        current['offensive_slots'].append(player['action_slot'])
            # Production snapshots omit destroyed structures. Disappearance
            # from a fresh complete snapshot is the authoritative death signal.
            present = {structure['id']: structure for structure in sample['structures']}
            structures = [present.get(sid, dict(initial, hp=0))
                          for sid, initial in self.initial_structures.items()]
            for structure in structures:
                initial = self.initial_structures[structure['id']]
                if initial['protected'] and structure['hp'] < initial['hp']:
                    assert any(s['team'] == structure['team'] and s['hp'] <= 0 and not self.initial_structures[s['id']]['protected'] for s in structures), 'Base damaged before a defending lane tower fell'
                if structure['hp'] <= 0 and not any(o['id'] == structure['id'] for o in current['objectives']):
                    current['objectives'].append(dict(id=structure['id'], team=structure['team'], base=initial['protected'], elapsed_secs=elapsed))
                    print(f'Round {match_id}: objective {structure["id"]} destroyed at {elapsed:.1f}s', flush=True)
        if sample['phase'] == 'victory' and current['victory'] is None:
            assert current['running'] is not None, 'Victory without Running'
            assert any(o['base'] for o in current['objectives']), 'Victory without destroyed base'
            assert {p['player_id'] for p in current['purchases']} == set(self.roster), 'Not every player completed an authoritative purchase'
            assert len(current['purchases']) > 10, 'No later base purchase was observed after starter items'
            current['victory'] = sample['elapsed_secs']
            current['duration_secs'] = current['victory'] - current['running']
            current['winner'] = sample['state']
            current['final_structures'] = sample['structures']
            print(f'Round {match_id}: {sample["state"]} after {current["duration_secs"]:.1f}s', flush=True)
        return len(self.rounds) == 2 and all(r['victory'] is not None for r in self.rounds)


def stop(process: subprocess.Popen | None) -> None:
    if process is not None and process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--server-binary', type=Path, required=True)
    parser.add_argument('--bots-binary', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--round-timeout', type=int, default=1800, help='Real seconds per round, 60..2400')
    args = parser.parse_args()
    if not 60 <= args.round_timeout <= 2400:
        parser.error('--round-timeout must be 60..2400 seconds')
    for binary in (args.server_binary, args.bots_binary):
        if not binary.is_file():
            parser.error(f'Binary does not exist: {binary}')
    out = args.evidence.resolve()
    out.mkdir(parents=True, exist_ok=True)
    if (out / 'result.json').exists():
        parser.error('Evidence already exists; choose a fresh directory to retain earlier attempts')
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as reserved:
        reserved.bind(('127.0.0.1', 0))
        address = f'127.0.0.1:{reserved.getsockname()[1]}'
    env = dict(os.environ, SERVER_ADDR=address, OMOBA_MATCH_MODE='release', OMOBA_TEAM_SIZE='5')
    server_log, bots_log = out / 'server.log', out / 'bots.log'
    server = bots = None
    proof = MatchProof(args.round_timeout)
    result = dict(status='FAIL', mode='release', team_size=5, participants=10, rounds_required=2,
                  normal_rules=True, accelerated_clock=False, address=address,
                  checker_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                  round_timeout_seconds=args.round_timeout,
                  server_sha256=hashlib.sha256(args.server_binary.read_bytes()).hexdigest(),
                  bots_sha256=hashlib.sha256(args.bots_binary.read_bytes()).hexdigest())
    started = time.monotonic()
    try:
        with server_log.open('w') as server_output, bots_log.open('w') as bots_output:
            server = subprocess.Popen([str(args.server_binary.resolve())], cwd=out, env=env,
                                      stdout=server_output, stderr=subprocess.STDOUT)
            deadline = time.monotonic() + 10
            while 'listening' not in server_log.read_text(errors='replace').lower():
                assert server.poll() is None, 'Server exited during startup'
                assert time.monotonic() < deadline, 'Server did not bind within ten seconds'
                time.sleep(.1)
            bots = subprocess.Popen([str(args.bots_binary.resolve()), '--server', address, '--count', '10', '--telemetry'],
                                    cwd=out, env=env, stdout=bots_output, stderr=subprocess.STDOUT)
            last_sample = time.monotonic()
            telemetry_lines = TelemetryLineBuffer()
            with bots_log.open() as stream, (out / 'samples.jsonl').open('w') as raw:
                while True:
                    assert server.poll() is None, 'Server exited during the match'
                    assert bots.poll() is None, 'Bots exited during the match'
                    now = time.monotonic()
                    assert now - last_sample < 10, 'No fresh telemetry/snapshots for ten seconds'
                    assert now - started < args.round_timeout * 2 + 60, 'Two-round lifecycle deadline exceeded'
                    assert proof.rounds or now - started < 30, 'Ten-player formation/countdown not observed within thirty seconds'
                    line = telemetry_lines.push(stream.readline())
                    if not line:
                        time.sleep(.1)
                        continue
                    if line.startswith('BOT_SAMPLE '):
                        sample = json.loads(line.removeprefix('BOT_SAMPLE '))
                        raw.write(json.dumps(sample) + '\n')
                        raw.flush()
                        last_sample = now
                        if proof.observe(sample):
                            break
            result['status'] = 'PASS'
    except (AssertionError, OSError, ValueError, KeyError) as error:
        result['error'] = str(error)
        print(f'FAIL: {error}', flush=True)
    finally:
        stop(bots)
        stop(server)
        metrics = [line for line in server_log.read_text(errors='replace').splitlines() if 'MATCH_METRIC ' in line]
        (out / 'metrics.log').write_text('\n'.join(metrics) + '\n')
        if result['status'] == 'PASS':
            for event, minimum in [('round_start', 2), ('victory', 2), ('round_reset', 1), ('objective', 4), ('purchase', 22)]:
                if sum(f'event={event} ' in line for line in metrics) < minimum:
                    result.update(status='FAIL', error=f'Missing authoritative {event} metrics')
            if any('event=disconnect ' in line or 'event=abandoned ' in line for line in metrics):
                result.update(status='FAIL', error='Unexpected disconnect/abandonment in server metrics')
        result.update(elapsed_seconds=time.monotonic() - started, rounds=proof.rounds,
                      samples=proof.samples, max_peer_snapshot_age_ms=proof.max_snapshot_age_ms,
                      roster=proof.roster, server_epoch=proof.epoch)
        (out / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps(result, indent=2), flush=True)
    return 0 if result['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
