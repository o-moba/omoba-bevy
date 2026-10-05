"""Reject incomplete/misleading jungle renderer evidence."""
import copy
import unittest
import catalog
from capture_verdant import verify_jungle, verify_quality, JUNGLE_IMAGES, QUALITY_IMAGES


class JungleCaptureEvidenceTests(unittest.TestCase):
    def fixture(self):
        mobs = [dict(id=i, hp=72, kind="Skirmisher", position=[float(i), 0, 0], foot_local_y=0, head_local_y=2) for i in range(catalog.map_tuning()['camp_count'])]
        capture = dict(snapshot_tick=7, jungle_mobs=mobs, qa_render_fixtures=0,
                       pixels=[844, 390], ui_profile="Mobile", gameplay_allowed=True, window_focused=True,
                       primary_controls_fit=True, primary_nodes=[{}] * 6,
                       minimap=dict(camp_markers=[dict(index=i, alive=True) for i in range(catalog.map_tuning()['camp_count'])]))
        return dict(qa_fixtures=0, captures=[dict(copy.deepcopy(capture), file=file) for file in JUNGLE_IMAGES]), [
            dict(snapshot_tick=7, neutrals=[dict(id=i, hp=72, camp_type="skirmisher", x=float(i), z=0) for i in range(catalog.map_tuning()['camp_count'])])]

    def test_complete_independent_evidence_passes(self):
        summary, samples = self.fixture()
        self.assertTrue(verify_jungle(summary, samples, 844, 390, True)["pass"])

    def test_legacy_six_camps_are_incomplete_even_when_all_endpoints_agree(self):
        summary, samples = self.fixture()
        for capture in summary['captures']:
            capture['jungle_mobs'] = capture['jungle_mobs'][:6]
            capture['minimap']['camp_markers'] = capture['minimap']['camp_markers'][:6]
        samples[0]['neutrals'] = samples[0]['neutrals'][:6]
        self.assertFalse(verify_jungle(summary, samples, 844, 390, True)['pass'])

    def test_server_ids_hp_and_render_fixtures_must_match(self):
        for field, value in [("id", 42), ("hp", 1), ("position", [0, 2, 0])]:
            summary, samples = self.fixture()
            summary["captures"][0]["jungle_mobs"][0][field] = value
            self.assertFalse(verify_jungle(summary, samples, 844, 390, True)["pass"])
        summary, samples = self.fixture()
        summary["qa_fixtures"] = 1
        self.assertFalse(verify_jungle(summary, samples, 844, 390, True)["pass"])

    def test_wrong_device_or_missing_markers_or_server_evidence_fails(self):
        summary, samples = self.fixture()
        self.assertFalse(verify_jungle(summary, samples, 1280, 720, False)["pass"])
        self.assertFalse(verify_jungle(summary, [], 844, 390, True)["pass"])
        summary["captures"][0]["minimap"]["camp_markers"].pop()
        self.assertFalse(verify_jungle(summary, samples, 844, 390, True)["pass"])


class QualityCaptureEvidenceTests(unittest.TestCase):
    def fixture(self):
        capture = dict(snapshot_tick=19, qa_render_fixtures=0, pixels=[852, 393], ui_profile='Mobile',
                       authoritative_bosses=[dict(id=11, kind='KingMutatioBoss', hp=1500)],
                       nexus_rings=[dict(name=f'NexusOrbit-{i}') for i in range(3)])
        summary = dict(qa_fixtures=0, captures=[dict(copy.deepcopy(capture), file=f) for f in QUALITY_IMAGES])
        samples = [dict(snapshot_tick=19, neutrals=[dict(id=11, camp_type='king_mutatio_boss', hp=1500)])]
        return summary, samples

    def test_exact_two_real_views_and_matching_boss_pass(self):
        summary, samples = self.fixture()
        self.assertTrue(verify_quality(summary, samples, 852, 393, True)['pass'])

    def test_missing_boss_ring_or_replaced_view_fails(self):
        for change in [lambda s: s['captures'][0].update(authoritative_bosses=[]),
                       lambda s: s['captures'][1].update(nexus_rings=[]),
                       lambda s: s['captures'][0].update(file=QUALITY_IMAGES[1]),
                       lambda s: s.update(qa_fixtures=1)]:
            summary, samples = self.fixture()
            change(summary)
            self.assertFalse(verify_quality(summary, samples, 852, 393, True)['pass'])
        summary, samples = self.fixture()
        self.assertFalse(verify_quality(summary, [], 852, 393, True)['pass'])


if __name__ == "__main__":
    unittest.main()
