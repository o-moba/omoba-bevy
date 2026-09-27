#!/usr/bin/env python3
"""The UI screen map stays in step with the game: no unmapped screens or frames."""
import unittest

from capture_ui_audit import DOC_PATH, check_map, expected_frames, load_map, render_doc


class UiScreenMapTest(unittest.TestCase):
    def setUp(self):
        self.map = load_map()

    def test_map_matches_app_screens_harness_frames_and_qa_modules(self):
        # Adding an AppScreen, a harness frame or a QA module means adding it to
        # scripts/ui_screen_map.json (as a screen, an ignored frame, a gap or an
        # other_harnesses entry) and re-running --write-doc.
        self.assertEqual(check_map(self.map), [])

    def test_generated_doc_is_current(self):
        self.assertEqual(DOC_PATH.read_text(), render_doc(self.map),
                         "run: python3 scripts/capture_ui_audit.py --write-doc")

    def test_every_run_expects_frames_on_both_profiles(self):
        for run in self.map["runs"]:
            for profile in self.map["profiles"]:
                with self.subTest(run=run, profile=profile):
                    self.assertTrue(expected_frames(self.map, run, profile))

    def test_phone_hud_frames_use_the_phone_height(self):
        frames = expected_frames(self.map, "match", "phone")
        self.assertIn("03-gameplay-390p.png", frames)
        self.assertNotIn("12-scoreboard-fixture-390p.png", frames)


if __name__ == "__main__":
    unittest.main()
