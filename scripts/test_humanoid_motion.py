#!/usr/bin/env python3
"""Shared-motion source, continuity, provenance and all-roster retarget checks."""

import copy
import hashlib
import math
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import export_humanoid_motion as motion
import retarget_animations as legacy


class SharedHumanoidMotionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.library = motion.export_library()
        cls.source = legacy.SourceRig(legacy.Gltf.from_gltf_file(motion.SOURCE))
        cls.document = legacy.Gltf.from_glb(motion.ROOT / legacy.AVATARS_DIR / "agnes.glb")
        cls.humanoid = {
            item["bone"]: item["node"]
            for item in cls.document.js["extensions"]["VRM"]["humanoid"]["humanBones"]
        }

    def test_checked_in_library_is_exact_deterministic_export(self):
        self.assertEqual(motion.OUTPUT.read_bytes(), motion.encode_library(self.library))
        self.assertEqual(motion.encode_library(self.library), motion.encode_library(motion.export_library()))

    def test_sprint_is_actual_source_not_faster_walk(self):
        run, walk = (self.library["clips"][name] for name in ("run", "walk"))
        self.assertEqual(run["source_clip"], "Sprint_Loop")
        self.assertEqual(walk["source_clip"], "Walk_Loop")
        self.assertEqual(len(run["times"]), 17)
        self.assertAlmostEqual(run["duration"], 2 / 3, places=6)
        self.assertAlmostEqual(walk["duration"], 4 / 3, places=6)
        source_clip = self.source.clip("Sprint_Loop")
        node = self.source.name_to_node["DEF-shin.L"]
        frame = 4
        pose = self.source.doc.world_pose(source_clip.pose_at(source_clip.timeline[frame], self.source.rest_pose))
        source_delta = legacy.qmul(pose[node][1], legacy.qconj(self.source.ref_world[node][1]))
        self.assertLess(motion.angular_distance(source_delta, run["world_rotation_deltas"]["leftLowerLeg"][frame]), 1e-6)
        # Equal normalized phase still differs: timing is not the only change.
        run_pose = run["world_rotation_deltas"]["leftLowerLeg"][4]
        walk_pose = walk["world_rotation_deltas"]["leftLowerLeg"][8]
        self.assertGreater(motion.angular_distance(run_pose, walk_pose), 0.25)

    def test_idle_walk_run_are_closed_in_place_loops(self):
        for name in ("idle", "walk", "run"):
            clip = self.library["clips"][name]
            self.assertTrue(clip["looping"])
            self.assertEqual(clip["hips_world_deltas"][0], clip["hips_world_deltas"][-1])
            for stream in clip["world_rotation_deltas"].values():
                self.assertLess(motion.angular_distance(stream[0], stream[-1]), 1e-6)
        for name in ("attack", "cast", "death"):
            self.assertFalse(self.library["clips"][name]["looping"])
        death = self.library["clips"]["death"]["hips_world_deltas"]
        self.assertGreater(death[0][1] - death[-1][1], 0.5)

    def test_every_shipped_rig_has_finite_run_leg_and_hips_motion(self):
        report = motion.audit_roster(self.library)
        self.assertEqual(report["rig_count"], 15)
        self.assertEqual(report["status"], "PASS")
        for rig in report["rigs"]:
            self.assertTrue(rig["source_bytes_unchanged"])
            self.assertTrue(rig["finite"])
            self.assertEqual(rig["max_non_hips_translation_error_m"], 0)
            self.assertLess(rig["max_loop_rotation_error_rad"], 1e-6)
            for angle in rig["leg_and_hips_rotation_excursion_rad"].values():
                self.assertGreater(angle, 0.02)
            for spans in rig["foot_world_ranges_m"].values():
                self.assertGreater(max(spans), 0.2)

    def test_optional_upper_chest_preserves_top_spine_world_delta(self):
        clip = self.library["clips"]["run"]
        target = {"hips": 0, "spine": 1, "chest": 2, "leftEye": 3}
        channels = motion.target_channels(clip, target)
        self.assertIs(channels[2], clip["world_rotation_deltas"]["upperChest"])
        self.assertNotIn(3, channels)
        target["upperChest"] = 4
        channels = motion.target_channels(clip, target)
        self.assertIs(channels[2], clip["world_rotation_deltas"]["chest"])
        self.assertIs(channels[4], clip["world_rotation_deltas"]["upperChest"])

    def test_rotated_parent_rest_pose_recovers_equivalent_motion(self):
        original = motion.retarget_in_memory(self.library, self.document, self.humanoid, "run")
        js = copy.deepcopy(self.document.js)
        roots = [i for i in range(len(js["nodes"])) if i not in self.document.parent]
        angle = 1.1
        rotation = (0, math.sin(angle / 2), 0, math.cos(angle / 2))
        js["nodes"].append({"rotation": rotation, "children": roots})
        rotated_doc = legacy.Gltf(js, self.document.bin)
        transformed = motion.retarget_in_memory(self.library, rotated_doc, self.humanoid, "run")
        for before, after in zip(original, transformed):
            before = self.document.world_pose(before)
            after = rotated_doc.world_pose(after)
            for node in self.humanoid.values():
                expected_position = legacy.qrot(rotation, before[node][0])
                expected_rotation = legacy.qmul(rotation, before[node][1])
                self.assertLess(motion.angular_distance(expected_rotation, after[node][1]), 1e-6)
                self.assertLess(max(abs(a - b) for a, b in zip(expected_position, after[node][0])), 1e-6)

    def test_source_provenance_pins_both_gltf_and_binary(self):
        provenance = self.library["source"]
        self.assertEqual(provenance["license"], "CC0-1.0")
        self.assertEqual(provenance["gltf_sha256"], hashlib.sha256(motion.SOURCE.read_bytes()).hexdigest())
        binary = motion.SOURCE.parent / provenance["buffer"]
        self.assertEqual(provenance["buffer_sha256"], hashlib.sha256(binary.read_bytes()).hexdigest())

    def test_validator_rejects_nonfinite_rotations_and_open_loops(self):
        bad = copy.deepcopy(self.library)
        bad["clips"]["run"]["world_rotation_deltas"]["hips"][2][0] = float("nan")
        with self.assertRaisesRegex(ValueError, "invalid quaternion"):
            motion.validate_library(bad)
        bad = copy.deepcopy(self.library)
        bad["clips"]["run"]["hips_world_deltas"][-1][0] += 0.01
        with self.assertRaisesRegex(ValueError, "root displacement"):
            motion.validate_library(bad)
        bad = copy.deepcopy(self.library)
        bad["clips"]["run"]["times"][2] = bad["clips"]["run"]["times"][1]
        with self.assertRaisesRegex(ValueError, "strictly increasing"):
            motion.validate_library(bad)

    def test_check_reports_stale_output_without_rewriting(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "motion.json"
            output.write_text("stale\n")
            result = subprocess.run(
                [sys.executable, str(motion.ROOT / "scripts/export_humanoid_motion.py"), "--check", "--output", str(output)],
                capture_output=True, text=True, check=False,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Motion output is stale", result.stderr)
            self.assertEqual(output.read_text(), "stale\n")


if __name__ == "__main__":
    unittest.main()
