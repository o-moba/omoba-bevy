#!/usr/bin/env python3
"""Check showcase scene declarations without starting binaries or a renderer."""
import unittest

from capture_showcase import SCENES

CLASSES = {"warrior", "mage", "ranger", "cleric", "warden"}


class ShowcaseSceneTest(unittest.TestCase):
    def test_every_scene_is_complete_and_publishes_unique_files(self):
        published = [name for scene in SCENES.values() for name in scene["keep"].values()]
        self.assertEqual(len(published), len(set(published)))
        for name, scene in SCENES.items():
            with self.subTest(scene=name):
                self.assertIn(scene["profile"], ("desktop", "phone"))
                self.assertIn(scene["harness"], ("match", "shell"))
                width, height = scene["size"]
                self.assertTrue(320 <= width <= 3840 and 320 <= height <= 2160)
                self.assertTrue(scene["caption"].strip())
                self.assertTrue(all(file.endswith(".png") for file in scene["keep"].values()))
                if scene["harness"] == "match":
                    self.assertIn(scene["hero"], CLASSES)
                    self.assertIn(scene["mode"], ("dev", "practice"))

    def test_lane_scenes_use_the_client_lane_format_and_bounded_holds(self):
        for name, scene in SCENES.items():
            if not scene.get("lane"):
                continue
            with self.subTest(scene=name):
                lane, progress = scene["lane"].split(":")
                self.assertIn(lane, ("top", "mid", "bot"))
                self.assertTrue(0.0 <= float(progress) <= 1.0)
                self.assertTrue(0 < scene["hold"] <= 300)
                # Lane scenes need server bots; dev mode starts an empty map.
                self.assertEqual(scene["mode"], "practice")

    def test_both_profiles_cover_lane_shop_and_hero_select(self):
        for profile in ("desktop", "phone"):
            kinds = {"lane" if scene.get("lane") else scene["harness"]
                     for scene in SCENES.values() if scene["profile"] == profile}
            self.assertEqual(kinds, {"lane", "match", "shell"}, profile)

if __name__ == "__main__":
    unittest.main()
