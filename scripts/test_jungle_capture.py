"""Reject incomplete/misleading jungle renderer evidence."""
import copy
import unittest
from capture_verdant import verify_jungle, JUNGLE_IMAGES


class JungleCaptureEvidenceTests(unittest.TestCase):
    def fixture(self):
        mobs = [dict(id=i, hp=72, kind="Skirmisher", position=[float(i), 0, 0], foot_local_y=0, head_local_y=2) for i in range(6)]
        capture = dict(snapshot_tick=7, jungle_mobs=mobs, qa_render_fixtures=0,
                       pixels=[844, 390], ui_profile="Mobile", gameplay_allowed=True, window_focused=True,
                       primary_controls_fit=True, primary_nodes=[{}] * 6,
                       minimap=dict(camp_markers=[dict(index=i, alive=True) for i in range(6)]))
        return dict(qa_fixtures=0, captures=[dict(copy.deepcopy(capture), file=file) for file in JUNGLE_IMAGES]), [
            dict(snapshot_tick=7, neutrals=[dict(id=i, hp=72, camp_type="skirmisher", x=float(i), z=0) for i in range(6)])]

    def test_complete_independent_evidence_passes(self):
        summary, samples = self.fixture()
        self.assertTrue(verify_jungle(summary, samples, 844, 390, True)["pass"])

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


if __name__ == "__main__":
    unittest.main()
