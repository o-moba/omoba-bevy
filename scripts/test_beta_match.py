#!/usr/bin/env python3
"""Negative controls for the full-match evidence checker (synthetic snapshots)."""
import copy
from pathlib import Path
import subprocess
import sys
import unittest

from verify_beta_match import MatchProof


def countdown(match_id=1, elapsed=0, tick=1):
    return dict(meta=dict(match_id=match_id, server_epoch=1, snapshot_tick=tick),
                elapsed_secs=elapsed, phase='starting', state='Starting { countdown_ms: 3000 }',
                join_error='None', minions=0, buffs=0,
                players=[dict(id=i, team='Some(Blue)' if i % 2 else 'Some(Green)',
                              level=1, xp=0, gold=0, ranks=[1] * 4, hp=100, max_hp=100,
                              mana=100, max_mana=100, action_sequence=0, action_slot=0)
                         for i in range(1, 11)],
                peers=[dict(id=i, snapshot_age_ms=0) for i in range(1, 11)],
                structures=[dict(id=i, team='Some(Blue)' if i % 2 else 'Some(Green)',
                                 hp=650 if i >= 7 else 240, protected=i >= 7) for i in range(1, 9)])


class EvidenceNegativeControls(unittest.TestCase):
    def running(self):
        proof = MatchProof(60)
        sample = countdown()
        proof.observe(sample)
        sample.update(phase='running', elapsed_secs=3)
        sample['meta']['snapshot_tick'] = 2
        proof.observe(sample)
        sample['meta']['snapshot_tick'] = 3
        return proof, sample

    def test_python_optimization_cannot_disable_verification(self):
        result = subprocess.run([sys.executable, '-O', str(Path(__file__).with_name('verify_beta_match.py')), '--help'], capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('remove Python -O', result.stderr)

    def test_cleric_self_sustain_is_not_reported_as_offense(self):
        proof, sample = self.running()
        sample['players'][0].update(action_sequence=1, action_slot=3, **{'class': 'cleric'})
        proof.observe(sample)
        self.assertEqual(proof.rounds[0]['offensive_slots'], [])
        sample['meta']['snapshot_tick'] += 1
        sample['players'][0]['class'] = 'mage'
        proof.observe(sample)
        self.assertEqual(proof.rounds[0]['offensive_slots'], [3])

    def test_rejects_wrong_roster_and_stalled_peer(self):
        for mutation in ('roster', 'duplicate', 'stall', 'identity', 'tick'):
            with self.subTest(mutation=mutation):
                proof, sample = self.running()
                if mutation == 'roster':
                    sample['players'].pop()
                elif mutation == 'duplicate':
                    duplicate = dict(sample['players'][0], team=None)
                    sample['players'].append(duplicate)
                elif mutation == 'stall':
                    sample['peers'][0]['snapshot_age_ms'] = 5001
                elif mutation == 'identity':
                    sample['peers'][0]['id'] = 42
                else:
                    sample['meta']['snapshot_tick'] = 1
                with self.assertRaises(AssertionError):
                    proof.observe(sample)

    def test_rejects_base_damage_before_lane_and_round_timeout(self):
        proof, sample = self.running()
        sample['structures'][6]['hp'] = 649
        with self.assertRaisesRegex(AssertionError, 'Base damaged'):
            proof.observe(sample)
        proof, sample = self.running()
        sample['elapsed_secs'] = 65
        with self.assertRaisesRegex(AssertionError, 'timed out'):
            proof.observe(sample)

    def test_rejects_unclean_rematch_and_requires_second_victory(self):
        proof, sample = self.running()
        sample['structures'] = [s for s in sample['structures'] if s['id'] not in (1, 7)]
        sample.update(phase='victory', state='Victory { winner: Green }', elapsed_secs=20)
        self.assertFalse(proof.observe(sample), 'One victory must not satisfy two rounds')
        next_round = countdown(match_id=2, elapsed=30, tick=4)
        unclean = copy.deepcopy(next_round)
        unclean['players'][0]['xp'] = 1
        with self.assertRaisesRegex(AssertionError, 'Progression leaked'):
            proof.observe(unclean)
        next_round['meta']['snapshot_tick'] = 5
        self.assertFalse(proof.observe(next_round))
        next_round.update(phase='running', elapsed_secs=33)
        next_round['meta']['snapshot_tick'] = 6
        self.assertFalse(proof.observe(next_round))
        next_round.update(phase='victory', state='Victory { winner: Green }', elapsed_secs=50)
        next_round['structures'] = [s for s in next_round['structures'] if s['id'] not in (1, 7)]
        next_round['meta']['snapshot_tick'] = 7
        self.assertTrue(proof.observe(next_round))


if __name__ == '__main__':
    unittest.main()
