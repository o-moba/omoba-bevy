#!/usr/bin/env python3
"""Check the demo encoder's timing math and clip table without ffmpeg or a GPU."""
from pathlib import Path
import tempfile
import unittest

from record_demo import (CLIPS, PROFILES, V2_CLIPS, V2_FPS, clip_environment, escape, finish_stepped, speed_at,
                         stepped_frames_complete, warp)

LEGACY_CLASSES = ("warrior", "mage", "ranger", "cleric", "warden")
STANDARD_CLASSES = ("dawnweaver", "wildspark", "cinderforge", "edgeweaver", "stormfist", "veilstalker",
                    "emberveil", "orbitwright", "riftshot", "chainkeeper", "frostguard", "adventurer")


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
                self.assertIn(clip["hero"], LEGACY_CLASSES)
                self.assertEqual(clip["touch"], clip["script"] == "phone")
                self.assertTrue(clip["label"])


class TrailerV2ProfileTest(unittest.TestCase):
    def test_profiles_keep_the_v1_table_and_add_v2(self):
        self.assertIs(PROFILES["v1"], CLIPS)
        self.assertIs(PROFILES["v2"], V2_CLIPS)
        self.assertEqual(set(CLIPS), {"desktop", "jungle", "phone"})

    def test_v2_clips_are_stepped_offline_captures_at_60_fps(self):
        for name, clip in V2_CLIPS.items():
            with self.subTest(clip=name):
                self.assertIn(clip["script"], ("lane", "showcase", "jungle"))
                self.assertIn(clip["hero"], LEGACY_CLASSES + STANDARD_CLASSES)
                self.assertTrue(clip["offline"])
                self.assertEqual(clip["fps"], V2_FPS)
                self.assertEqual(V2_FPS, 60)
                width, height = clip["size"]
                # x264 4:2:0 needs even sizes; the capture is at least Full HD wide.
                self.assertEqual((width % 2, height % 2), (0, 0))
                self.assertGreaterEqual(width, 1920)
                self.assertEqual(clip["touch"], name.startswith("phone-"))
                self.assertIn(clip.get("lane", "mid"), ("top", "mid", "bot"))
                self.assertTrue(clip["label"])

    def test_v2_is_phone_first_with_wildspark_and_four_classes(self):
        heroes = {clip["hero"] for clip in V2_CLIPS.values()}
        self.assertIn("wildspark", heroes)
        self.assertGreaterEqual(len(heroes), 4)
        phones = [clip for clip in V2_CLIPS.values() if clip["touch"]]
        self.assertGreater(len(phones), len(V2_CLIPS) / 2)
        for clip in phones:
            # The phone layout is 844x390 points whatever the pixel size.
            self.assertEqual([side / clip["scale"] for side in clip["size"]], [844, 390])

    def test_offline_clip_environment_steps_the_offline_practice(self):
        env = clip_environment(V2_CLIPS["phone-wildspark"], Path("/raw"), "127.0.0.1:1", "/work", "/assets",
                               pace=12, window_at="2860,1800")
        self.assertEqual(env["OMOBA_RECORD_WINDOW_AT"], "2860,1800")
        self.assertEqual(env["OMOBA_OFFLINE_PRACTICE"], "1")
        self.assertEqual(env["OMOBA_RECORD_STEP"], "1")
        self.assertEqual(env["OMOBA_QA_SYNTHETIC_FOCUS"], "1")
        self.assertEqual(env["OMOBA_RECORD_PACE"], "12")
        self.assertEqual(env["OMOBA_RECORD_FPS"], "60")
        self.assertEqual(env["OMOBA_QA_SCALE"], "3")
        self.assertEqual((env["OMOBA_QA_WIDTH"], env["OMOBA_QA_HEIGHT"]), ("2532", "1170"))
        self.assertEqual(env["OMOBA_DEMO_CLASS"], "wildspark")
        self.assertEqual(env["OMOBA_DEMO_LANE"], "mid")
        self.assertEqual(env["OMOBA_TOUCH_CONTROLS"], "1")
        self.assertEqual(env["OMOBA_RECORD_DIR"], "/raw/frames")

    def test_v1_clip_environment_stays_real_time_against_a_server(self):
        env = clip_environment(CLIPS["desktop"], Path("/raw"), "127.0.0.1:1", "/work", "/assets")
        for key in ("OMOBA_OFFLINE_PRACTICE", "OMOBA_RECORD_STEP", "OMOBA_RECORD_PACE",
                    "OMOBA_RECORD_WINDOW_AT", "OMOBA_QA_SYNTHETIC_FOCUS", "OMOBA_DEMO_ZOOM"):
            self.assertNotIn(key, env)
        self.assertEqual(env["OMOBA_RECORD_FPS"], "30")
        self.assertEqual(env["OMOBA_QA_SCALE"], "1")
        self.assertEqual(env["GAME_SERVER_ADDR"], "127.0.0.1:1")

    def test_finishing_again_without_raw_frames_keeps_the_encoded_clip(self):
        with tempfile.TemporaryDirectory() as folder:
            output = Path(folder)
            (output / "raw/phone-wildspark/frames").mkdir(parents=True)
            (output / "clips").mkdir()
            clip = output / "clips/phone-wildspark.mp4"
            clip.write_bytes(b"encoded earlier")
            video = dict(file="clips/phone-wildspark.mp4", bytes=15, sha256="kept")
            manifest = dict(profile="v2", clips=[dict(clip="phone-wildspark", frames=[], events=[], video=video,
                                                      **{"pass": True, "fps": 60})])
            # No PNG frames are left, so ffmpeg is not run and nothing is overwritten.
            self.assertEqual(finish_stepped(manifest, output, ["phone-wildspark"], keep_frames=False), 0)
            self.assertEqual(clip.read_bytes(), b"encoded earlier")
            self.assertEqual(manifest["clips"][0]["video"], video)
            self.assertTrue((output / "demo-summary.json").is_file())

    def test_stepped_log_must_be_gapless_and_on_the_frame_grid(self):
        rows = [dict(index=i, file=f"frame-{i:06}.png", seconds=i / 60, skipped_before=0) for i in range(5)]
        self.assertTrue(stepped_frames_complete(rows, rows, 60))
        self.assertFalse(stepped_frames_complete(rows, rows[:-1], 60))  # a frame was not saved
        late = [dict(row, seconds=row["seconds"] + 0.01) if row["index"] == 3 else row for row in rows]
        self.assertFalse(stepped_frames_complete(late, late, 60))
        skipped = [dict(row, skipped_before=1) if row["index"] == 4 else row for row in rows]
        self.assertFalse(stepped_frames_complete(skipped, skipped, 60))


if __name__ == "__main__":
    unittest.main()
