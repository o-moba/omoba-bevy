#!/usr/bin/env python3
"""Check the demo encoder's timing math and clip table without ffmpeg or a GPU."""
import unittest

from record_demo import CLIPS, escape, speed_at, warp


class DemoTimingTest(unittest.TestCase):
    PACE = [{"seconds": 10.0, "speed": 4.0}, {"seconds": 50.0, "speed": 1.0}]

    def test_timelapse_compresses_only_marked_segments(self):
        self.assertEqual(warp(self.PACE, 0.0, 10.0), 10.0)
        self.assertEqual(warp(self.PACE, 0.0, 50.0), 20.0)
        self.assertEqual(warp(self.PACE, 0.0, 60.0), 30.0)
        self.assertEqual(warp(self.PACE, 30.0, 60.0), 15.0)
        self.assertEqual(warp([], 2.0, 7.5), 5.5)

    def test_speed_follows_the_latest_marker(self):
        self.assertEqual(speed_at(self.PACE, 5.0), 1.0)
        self.assertEqual(speed_at(self.PACE, 10.0), 4.0)
        self.assertEqual(speed_at(self.PACE, 49.9), 4.0)
        self.assertEqual(speed_at(self.PACE, 50.0), 1.0)

    def test_drawtext_escaping_keeps_filter_syntax_intact(self):
        self.assertEqual(escape("a:b,c'd"), "a\\:b\\,c’d")

    def test_clips_name_a_director_script_class_and_profile(self):
        for name, clip in CLIPS.items():
            with self.subTest(clip=name):
                self.assertIn(clip["script"], ("desktop", "jungle", "phone"))
                self.assertIn(clip["hero"], ("warrior", "mage", "ranger", "cleric", "warden"))
                self.assertEqual(clip["touch"], clip["script"] == "phone")
                self.assertTrue(clip["label"])


if __name__ == "__main__":
    unittest.main()
