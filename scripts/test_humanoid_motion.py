#!/usr/bin/env python3
"""Shared-motion source, continuity, provenance and all-roster retarget checks."""

import copy
import hashlib
import json
import math
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import export_humanoid_motion as motion
import retarget_animations as legacy


# SHA-256 of each clip object that existed before the derived table, as it is
# written to the library (compact JSON). They prove that adding motions leaves
# the bytes of the kept clips alone. `dagger_backstab` was re-keyed with the
# derived table and is pinned to its new keys.
EXISTING_CLIP_SHA256 = {
    "idle": "af71fb2c3ff625f37a19cac0c87d7a82cb701d565fa6a9c2b410498ef0dad3b9",
    "walk": "1aa74874a7f8e984eb75981559b30669b0c07cda83289760f273b8bc45f55952",
    "run": "ef37702dedce3e9412e3c9ab50384037e941100f08ce4ceca392d760297f2c7e",
    "attack": "ad3241d41e03d6e41e8ac7fa90c1c7f5fa66eef722c307321f9a8e78053896f2",
    "cast": "5f52d18b0dde372d0618a1493123e97962bfd613813458650f3de0551c5177c4",
    "death": "ba041e629066eecd925feda50f7ec3e3598bfd5048ddec3f23e0d1baa2cfb348",
    "spell_prepare": "d984ca2432c9edb005f65127f681bf22f199377cf100d30757c43c0e73f2c45c",
    "spell_finish": "3e49d19255aa7a4ca8a06a8c2dc47b16a83b45f97229d7fe38bc26dcf64ba81e",
    "pistol_shoot": "91b2e270d1a43f7215cb1e0493d293ec27909ff1b39463c69470e1205e91d516",
    "pistol_reload": "ec25ea24c2fbfa14f3f8b4d3429db8d28f7821ac34a05e3fba1530905a3ba7b9",
    "pistol_aim": "b3bb31ef89250b6591f0b724d5292dbe89f969d17432f4665e356feb75531a73",
    "interact": "7f3b237f4d8f5f1ad8cdc239c72b5e98c0c5b6e537d9bda5cfd0500b9a705567",
    "punch": "9c0dfc50c2b4ef087158f324637418b18c965257039dc931a329515c166f6a6d",
    "guard": "7ec398646076a39a4c7a7f45e3f8a1f5331857aa542e9c5faa62c0d2e810fe3c",
    "shoulder_drive": "66627717894feca8d9782ef479f547441ff0c38e0aed792e542bca7059b6328d",
    "roll": "588361a3c64687fd7b0b2b1d5777dbdc00fa77d2d9ff4322a2b9023e9b79cd30",
    "dagger_stab": "9d3e11c9f101fe593ab25ff546e0591e0eb60f936954eb23e16510b193cb2957",
    "dagger_feint": "ae1d165bdd300a2e3809cde0149b7afc1c4120cddcfdb9bb33fab7e1ba97d9dd",
    "dagger_backstab": "a473b4af6bea0061aba65b934611447651ad103ce625f70496b4373d14bd973f",
    "dagger_heavy_thrust": "5ca871eff3ff2123ea9bac3e2cfb923a6171288397fc721d92c52705be250c6e",
}
# The motion IDs the skill vocabulary needs beyond the clips above.
NEW_MOTIONS = (
    "slash_down", "slash_rising", "cleave_slam", "spin_cleave", "slash_down_m", "slash_rising_m",
    "blade_flourish", "blade_ready_loop", "thrust_lunge", "jab_cross", "fist_guard_loop", "ground_pound",
    "flying_knee", "leap_land", "dive_lunge", "vault_flip", "backflip_retreat", "dance_twirl",
    "overhead_plant", "rally_raise", "raise_from_earth", "two_hand_push", "shot_heavy", "burst_fire",
    "sky_shot", "ground_shot", "reload_snap", "aim_hold_loop", "aim_loose_r", "cast_thrust_r",
    "point_command", "hover_pulse", "levitate_loop", "draw_in", "hurl_overhand", "toss_underhand",
    "place_quick", "kneel_plant",
)
FINGERS = ("Index", "Middle", "Ring", "Little", "Thumb")


class SharedHumanoidMotionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.library = motion.export_library()
        cls.table = motion.derived_table()
        cls.source = legacy.SourceRig(legacy.Gltf.from_gltf_file(motion.SOURCE))
        cls.bones = motion.semantic_source_bones(cls.source)
        cls.document = legacy.Gltf.from_glb(motion.ROOT / legacy.AVATARS_DIR / "agnes.glb")
        cls.humanoid = {
            item["bone"]: item["node"]
            for item in cls.document.js["extensions"]["VRM"]["humanoid"]["humanBones"]
        }

    def joints(self, clip_name, library=None):
        """World positions of the mapped joints of one rig at every key of a clip."""
        poses = motion.retarget_in_memory(library or self.library, self.document, self.humanoid, clip_name)
        return [
            {bone: world[node][0] for bone, node in self.humanoid.items()}
            for world in map(self.document.world_pose, poses)
        ]

    def derive(self, name, without):
        """The row built again with one operation left out."""
        row = copy.deepcopy(self.table[name])
        del row["ops"][without]
        return motion.derive_clip(name, row, self.source, self.bones)

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

    def test_dagger_skill_clips_are_distinct_in_place_and_retarget_to_vrm(self):
        clips = [self.library['clips'][name] for name in motion.DAGGER_MOTIONS]
        self.assertEqual(len({clip['duration'] for clip in clips}), 4)
        for name, clip in zip(motion.DAGGER_MOTIONS, clips):
            self.assertFalse(clip['looping'])
            self.assertTrue(all(p[0] == p[2] == 0 for p in clip['hips_world_deltas']))
            poses = motion.retarget_in_memory(self.library, self.document, self.humanoid, name)
            hand = self.humanoid['rightHand']
            # A stab can keep the wrist rigid; test the actual right-hand path
            # after its shoulder/elbow chain, not arbitrary local wrist flexion.
            points = [self.document.world_pose(p)[hand][0] for p in poses]
            excursion = max(sum((v - start) ** 2 for v, start in zip(point, points[0])) ** .5 for point in points)
            self.assertGreater(excursion, .1)

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

    def test_library_is_the_three_tables_and_kept_clips_keep_their_bytes(self):
        kept = [name for name, _, _ in motion.CLIPS] + list(motion.DAGGER_MOTIONS)
        self.assertEqual(list(EXISTING_CLIP_SHA256), kept)
        self.assertEqual(list(self.library["clips"]), kept + list(NEW_MOTIONS))
        self.assertEqual(set(self.table), set(NEW_MOTIONS) | {"dagger_backstab"})
        self.assertLessEqual(len(self.library["clips"]), 64)
        for name, digest in EXISTING_CLIP_SHA256.items():
            encoded = json.dumps(self.library["clips"][name], separators=(",", ":"), allow_nan=False).encode()
            self.assertEqual(hashlib.sha256(encoded).hexdigest(), digest, name)
            self.assertIn(encoded, motion.OUTPUT.read_bytes(), name)

    def test_every_derived_clip_is_in_place_with_its_declared_timing(self):
        for name, row in self.table.items():
            with self.subTest(name):
                clip = self.library["clips"][name]
                count = len(row["keys"])
                self.assertEqual(clip["duration"], row["duration"])
                self.assertEqual(clip["looping"], row["looping"])
                uniform = motion.rounded([row["duration"] * i / (count - 1) for i in range(count)])
                self.assertEqual(clip["times"], row.get("times", uniform))
                self.assertTrue(all(p[0] == p[2] == 0 for p in clip["hips_world_deltas"]))
                for key in row["keys"]:
                    self.assertIn(key["clip"], clip["source_clip"])
                self.assertIn(f"Open Moba {name}", clip["source_clip"])

    def test_every_derived_clip_interpolates_the_short_way_on_a_rig(self):
        for name in self.table:
            with self.subTest(name):
                for stream in self.library["clips"][name]["world_rotation_deltas"].values():
                    for a, b in zip(stream, stream[1:]):
                        self.assertGreaterEqual(sum(x * y for x, y in zip(a, b)), 0)
                # The runtime interpolates local joint rotations: no step may
                # come near the half turn where the short way is undefined.
                poses = motion.retarget_in_memory(self.library, self.document, self.humanoid, name)
                step = max(
                    motion.angular_distance(a[node][1], b[node][1])
                    for a, b in zip(poses, poses[1:]) for node in self.humanoid.values()
                )
                self.assertLess(math.degrees(step), 170)

    def test_every_derived_clip_leaves_the_idle_pose_and_actions_move(self):
        idle = self.joints("idle")[0]
        for name, row in self.table.items():
            with self.subTest(name):
                frames = self.joints(name)
                away = max(
                    math.dist(frame[bone], idle[bone])
                    for frame in frames for bone in ("leftHand", "rightHand", "hips")
                )
                self.assertGreater(away, 0.1)
                if not row["looping"]:
                    streams = self.library["clips"][name]["world_rotation_deltas"]
                    turned = max(
                        motion.angular_distance(stream[0], pose)
                        for bone, stream in streams.items() if not any(f in bone for f in FINGERS)
                        for pose in stream
                    )
                    self.assertGreater(math.degrees(turned), 10)

    def test_contacts_cover_action_clips_and_lie_inside_them(self):
        clips, contacts = self.library["clips"], self.library["contacts"]
        self.assertEqual(
            list(contacts), [name for name, clip in clips.items() if not clip["looping"] and name != "death"])
        for name, contact in contacts.items():
            with self.subTest(name):
                self.assertGreaterEqual(contact, 0)
                self.assertLessEqual(contact, clips[name]["duration"])
                self.assertEqual(contact, self.table[name]["contact"] if name in self.table else motion.CONTACTS[name])
        for name, row in self.table.items():
            self.assertEqual("contact" in row, not row["looping"], name)

    def test_baked_mirrors_reflect_the_base_clip(self):
        mirrors = [name for name in self.table if name.endswith("_m")]
        self.assertEqual(mirrors, ["slash_down_m", "slash_rising_m"])
        rest = self.document.world_pose(self.document.rest_local_pose())
        centre = rest[self.humanoid["hips"]][0]
        across = [a - b for a, b in zip(rest[self.humanoid["leftHand"]][0], rest[self.humanoid["rightHand"]][0])]
        across[1] = 0.0
        normal = [v / math.hypot(*across) for v in across]

        def reflect(point):
            depth = sum((p - c) * n for p, c, n in zip(point, centre, normal))
            return [p - 2 * depth * n for p, n in zip(point, normal)]

        for name in mirrors:
            with self.subTest(name):
                base, row = self.table[name[:-2]], self.table[name]
                self.assertEqual(row["ops"], {"mirror": True})
                self.assertNotIn("ops", base)
                self.assertEqual({k: v for k, v in row.items() if k != "ops"}, base)
                plain = self.library["clips"][name[:-2]]["world_rotation_deltas"]
                for bone, stream in self.library["clips"][name]["world_rotation_deltas"].items():
                    for mirrored, source in zip(stream, plain[motion.opposite(bone)]):
                        self.assertLess(motion.angular_distance(
                            mirrored, (source[0], -source[1], -source[2], source[3])), 1e-6)
                for hand, other in (("leftHand", "rightHand"), ("rightHand", "leftHand")):
                    for mirrored, source in zip(self.joints(name), self.joints(name[:-2])):
                        self.assertLess(math.dist(mirrored[hand], reflect(source[other])), 0.005)
        # A mirrored thrust drives the hand that holds the weapon.
        thrust = self.joints("thrust_lunge")
        reach = {hand: max(math.dist(frame[hand], thrust[0][hand]) for frame in thrust)
                 for hand in ("leftHand", "rightHand")}
        self.assertGreater(reach["rightHand"], 0.4)
        self.assertGreater(reach["rightHand"], 2 * reach["leftHand"])

    def test_baked_spins_turn_the_whole_body_and_return_to_the_start_heading(self):
        spins = {name: row["ops"]["spin_deg"] for name, row in self.table.items() if "spin_deg" in row.get("ops", {})}
        self.assertEqual(sorted(spins), ["dance_twirl", "spin_cleave"])
        for name, degrees in spins.items():
            with self.subTest(name):
                self.assertEqual(degrees[0], 0)
                self.assertEqual(degrees[-1], 360)
                spun = self.library["clips"][name]["world_rotation_deltas"]
                plain = self.derive(name, "spin_deg")["world_rotation_deltas"]
                for bone in spun:
                    self.assertLess(motion.angular_distance(spun[bone][-1], plain[bone][-1]), 1e-6)
                    for turned, straight, angle in zip(spun[bone], plain[bone], degrees):
                        half = math.radians(angle) / 2
                        expected = legacy.qmul((0.0, math.sin(half), 0.0, math.cos(half)), straight)
                        self.assertLess(motion.angular_distance(turned, expected), 1e-6)
                half_way = degrees.index(180) if 180 in degrees else None
                if half_way is not None:
                    self.assertAlmostEqual(
                        motion.angular_distance(spun["hips"][half_way], plain["hips"][half_way]), math.pi, places=5)

    def test_hops_lift_only_the_hips_and_end_on_the_ground(self):
        hops = {name: row["ops"]["hips_y"] for name, row in self.table.items() if "hips_y" in row.get("ops", {})}
        self.assertEqual(sorted(hops), ["flying_knee", "hover_pulse", "leap_land", "levitate_loop"])
        for name, lift in hops.items():
            with self.subTest(name):
                clip, plain = self.library["clips"][name], self.derive(name, "hips_y")
                self.assertEqual(clip["world_rotation_deltas"], plain["world_rotation_deltas"])
                for lifted, grounded, height in zip(clip["hips_world_deltas"], plain["hips_world_deltas"], lift):
                    self.assertAlmostEqual(lifted[1] - grounded[1], height, places=6)
                self.assertGreater(max(lift), 0.15)
                self.assertEqual(lift[-1], lift[0] if clip["looping"] else 0)

    def test_malformed_derived_rows_are_rejected_before_sampling(self):
        good = self.table["flying_knee"]
        loop = self.table["blade_ready_loop"]
        spin = self.table["dance_twirl"]
        cases = (
            ("extra", dict(good, family="knee"), "unknown field"),
            ("idle", good, "already a full-rate or dagger clip"),
            ("Bad-ID", good, "ID must match"),
            ("no_contact", {k: v for k, v in good.items() if k != "contact"}, "contact must lie inside"),
            ("late_contact", dict(good, contact=0.71), "contact must lie inside"),
            ("times", dict(good, times=[0, 0.08, 0.08, 0.25, 0.36, 0.48, 0.7]), "times must rise"),
            ("short", dict(good, times=[0, 0.08, 0.17, 0.25, 0.36, 0.48, 0.6]), "times must rise"),
            ("hop", dict(good, ops={"hips_y": [0, 0.3, 0.5, 0.42, 0, 0, 0.1]}), "a hop ends on the ground"),
            ("ops", dict(good, ops={"exaggerate": 2.5}), "unknown op"),
            ("phase", dict(good, keys=good["keys"][:-1] + [{"clip": "Jump_Land", "phase": 1.5}]), "normalised"),
            ("root_motion", dict(good, keys=good["keys"][:-1] + [{"clip": "Roll_RM", "phase": 1}]), "root motion"),
            ("one_key", dict(good, keys=good["keys"][:1]), "at least two keys"),
            ("loop_contact", dict(loop, contact=0.1), "a loop is never released"),
            ("open_loop", dict(loop, keys=loop["keys"][:-1] + [{"clip": "Sword_Idle", "phase": 0.9}]), "a loop repeats"),
            ("half_turn", dict(spin, ops={"spin_deg": [0, 45, 90, 150, 210, 270, 315, 350]}), "a baked spin"),
            ("fast_turn", dict(spin, ops={"spin_deg": [0, 0, 0, 0, 120, 240, 360, 360]}), "a baked spin"),
        )
        for name, row, message in cases:
            with self.subTest(name), self.assertRaisesRegex(ValueError, message):
                motion.check_row(name, copy.deepcopy(row))
        with self.assertRaisesRegex(ValueError, "unknown source clip"):
            motion.derive_clip("ghost", dict(good, keys=[{"clip": "Ghost", "phase": 0}] * 7), self.source, self.bones)

    def test_validator_rejects_missing_contacts_and_moving_skill_motions(self):
        bad = copy.deepcopy(self.library)
        del bad["contacts"]["cast"]
        with self.assertRaisesRegex(ValueError, "contacts must cover"):
            motion.validate_library(bad)
        bad = copy.deepcopy(self.library)
        bad["contacts"]["cast"] = 0.6
        with self.assertRaisesRegex(ValueError, "contact must lie inside"):
            motion.validate_library(bad)
        bad = copy.deepcopy(self.library)
        bad["clips"]["leap_land"]["hips_world_deltas"][1][2] = 0.2
        with self.assertRaisesRegex(ValueError, "must stay in place"):
            motion.validate_library(bad)
        bad = copy.deepcopy(self.library)
        del bad["clips"]["kneel_plant"]
        with self.assertRaisesRegex(ValueError, "all declared"):
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
