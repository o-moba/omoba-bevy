#!/usr/bin/env python3
"""Validate measured server combat against the frozen first-pass balance targets."""
import argparse
import json
import math
import statistics as stats

CLASSES = ('warrior', 'mage', 'ranger', 'cleric')


def read(path):
    with open(path) as stream:
        return json.load(stream)


def key(row):
    return (row['level'], row['attacker'], row['defender'], row['policy'],
            tuple(row['order']), row['reply_delay'])


def validate(baseline, candidate, repeat):
    assert baseline['setup'] == candidate['setup'] == repeat['setup'], 'scenario drift'
    expected = {(level, a, b) for level in (1, 5, 10) for a in CLASSES for b in CLASSES}
    for data in (baseline, candidate, repeat):
        assert len(data['matrix']) == 48
        assert {(r['level'], r['attacker'], r['defender']) for r in data['matrix']} == expected
        assert all(r['ttk'] is not None and math.isfinite(r['ttk']) for r in data['matrix'])
        assert {(r['level'], r['attacker'], r['policy']) for r in data['supplements']
                if r['policy'] in ('basic_only', 'q_only')} == {
                    (l, c, p) for l in (1, 10) for c in CLASSES for p in ('basic_only', 'q_only')}
    levels = {l: [r['ttk'] for r in candidate['matrix'] if r['level'] == l] for l in (1, 5, 10)}
    assert min(levels[1]) >= 6, ('early minimum', min(levels[1]))
    assert 8 <= stats.median(levels[1]) <= 14, ('early median', stats.median(levels[1]))
    assert stats.median(levels[1]) > stats.median(r['ttk'] for r in baseline['matrix'] if r['level'] == 1)
    assert stats.mean(levels[10]) <= .8 * stats.mean(levels[1]), 'late mean must improve by 20%'
    assert min(levels[10]) >= 2, 'late minimum'
    for c in CLASSES:
        early = [r for r in candidate['matrix'] if r['level'] == 1 and r['attacker'] == c]
        late = [r for r in candidate['matrix'] if r['level'] == 10 and r['attacker'] == c]
        assert stats.mean(r['ttk'] for r in late) < stats.mean(r['ttk'] for r in early), c
        a, b = early[0], late[0]
        assert 1.2 <= b['move_speed'] / a['move_speed'] <= 1.3, c
        assert 1.5 - 1e-5 <= a['basic_interval'] / b['basic_interval'] <= 1.8 + 1e-5, c
        assert b['basic_damage'] / a['basic_damage'] >= 1.25, c
    # Re-run all primary and supplemental scenarios, including timeouts.
    for group in ('matrix', 'supplements'):
        left, right = ({key(r): r for r in d[group]} for d in (candidate, repeat))
        assert left.keys() == right.keys()
        for k, row in left.items():
            other = right[k]
            if row['ttk'] is None or other['ttk'] is None:
                assert row['ttk'] == other['ttk'], k
            else:
                assert abs(row['ttk'] - other['ttk']) <= candidate['setup']['dt'], k
            for field in ('attacker_hp', 'defender_hp', 'mana', 'ranks', 'basic_damage',
                          'basic_interval', 'move_speed', 'attacker_alive', 'defender_alive'):
                assert row[field] == other[field], (k, field)
    assert len(candidate['sustain']) == 4
    for row in candidate['sustain']:
        hero = next(r for r in candidate['matrix'] if r['attacker'] == row['class'] and r['level'] == 10)
        assert row['seconds'] == 30 and 0 < row['final_hp'] <= hero['attacker_hp']
        assert 0 <= row['final_mana'] <= hero['mana'] and row['hp_restored'] >= 0
    print('PASS: 48 primary encounters; early/late targets; class growth; repeatability; sustain bounds.')
    print('\n| Level | Baseline mean | Current mean | Current median | Current min–max |')
    print('|---|---:|---:|---:|---:|')
    for level, values in levels.items():
        old = [r['ttk'] for r in baseline['matrix'] if r['level'] == level]
        print(f'| {level} | {stats.mean(old):.2f}s | {stats.mean(values):.2f}s | '
              f'{stats.median(values):.2f}s | {min(values):.2f}–{max(values):.2f}s |')
    print('\nCurrent ordered matrix (rows = attacker; columns = defender):')
    for level in levels:
        print(f'\nLevel {level}\n\n| Attacker | Warrior | Mage | Ranger | Cleric |\n|---|---:|---:|---:|---:|')
        for c in CLASSES:
            values = [next(r['ttk'] for r in candidate['matrix'] if
                           (r['level'], r['attacker'], r['defender']) == (level, c, d)) for d in CLASSES]
            print(f'| {c} | ' + ' | '.join(f'{v:.2f}s' for v in values) + ' |')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('baseline')
    parser.add_argument('candidate')
    parser.add_argument('repeat')
    args = parser.parse_args()
    validate(read(args.baseline), read(args.candidate), read(args.repeat))
