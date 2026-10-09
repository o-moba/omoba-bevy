"""Evidence guard: a rendered movie alone must not prove working mechanics."""
import copy
from pathlib import Path
import tempfile
import unittest

from record_wildspark_demo import CHAPTERS, validate


class DemoEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory()
        self.addCleanup(self.work.cleanup)
        self.raw = Path(self.work.name)
        (self.raw / 'frame.png').write_bytes(b'capture')
        chapters = []
        for name, *_ in CHAPTERS:
            splash = name == 'rockets' or name.startswith('rocket-')
            receipts = [dict(id=i, source=dict(id=1), target=dict(id=i+2),
                             amount=100 if name == 'rocket-far' else 50,
                             area_impact=dict(id=1) if name.startswith('rocket-') else None,
                             trap_triggered=name == 'traps') for i in range(2 if splash else 1)]
            state = ['slowed'] if name == 'slow' else ['rooted'] if name == 'traps' else []
            frames = [dict(file='frame.png', mean_pixel=80, receipts=receipts,
                           actors=[dict(actor='player', id=1, mana=100-n/4, position=[0, 0]),
                                   dict(actor='enemy', id=2, mana=100, position=[n/4, 0])],
                           states=[dict(hero=2, states=state)]) for n in range(20)]
            chapters.append(dict(chapter=name, frames=frames))
        self.report = dict(capture_complete=True, chapters=chapters)

    def test_complete_real_action_evidence(self):
        self.assertTrue(validate(self.report, self.raw)['pass_'])

    def test_no_splash_receipts_cannot_pass(self):
        chapter = self.report['chapters'][1]
        for frame in chapter['frames']:
            frame['receipts'] = frame['receipts'][:1]
        self.assertIn('Rocket splash/mana not demonstrated', validate(self.report, self.raw)['errors'])

    def test_stationary_opponent_cannot_prove_trap_walk(self):
        for frame in self.report['chapters'][3]['frames']:
            frame['actors'][1]['position'] = [1, 1]
        self.assertIn('traps: moving opponent not demonstrated', validate(self.report, self.raw)['errors'])

    def test_incomplete_or_missing_images_fail(self):
        truncated = copy.deepcopy(self.report)
        truncated['chapters'].pop()
        self.assertFalse(validate(truncated, self.raw)['pass_'])
        (self.raw / 'frame.png').unlink()
        self.assertFalse(validate(self.report, self.raw)['pass_'])


if __name__ == '__main__':
    unittest.main()
