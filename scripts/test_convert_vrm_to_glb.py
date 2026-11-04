#!/usr/bin/env python3
"""Local VRM import: VRM 0.x is copied byte for byte, VRM 1.0 is turned to face -Z."""
from __future__ import annotations

from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import convert_vrm_to_glb as convert
import retarget_animations as retarget
from test_ekza_build_rendition import SEED_SAN, SOURCE, glb_with_bin, rest_world, studio_vrm1

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/convert_vrm_to_glb.py"


def left_hand_x(data: bytes) -> float:
    doc, world = rest_world(data)
    return world[retarget.humanoid_bone_map(doc.js)[1]["leftHand"]][0][0]


class LocalVrmImport(unittest.TestCase):
    def test_vrm0_is_copied_byte_for_byte(self) -> None:
        source = SOURCE.read_bytes()
        staged, turned = convert.stage(source)
        self.assertFalse(turned)
        self.assertEqual(staged, source)

    def test_vrm1_is_turned_once_like_the_rendition_builder(self) -> None:
        document, bones = studio_vrm1()
        source = glb_with_bin(document, b"\0" * 4)
        self.assertGreater(left_hand_x(source), 0.3)
        staged, turned = convert.stage(source)
        self.assertTrue(turned)
        self.assertLess(left_hand_x(staged), -0.3)
        doc = retarget.Gltf.from_glb_bytes(staged)
        self.assertTrue(doc.js["asset"]["extras"]["omoba_rest_yaw180"])
        self.assertIn("VRMC_vrm", doc.js["extensions"])
        # Only the scene roots (hips and the skinned body node) change.
        changed = [i for i, (a, b) in enumerate(zip(doc.nodes, document["nodes"])) if a != b]
        self.assertEqual(changed, sorted(document["scenes"][0]["nodes"]))
        again, turned_again = convert.stage(staged)
        self.assertFalse(turned_again)
        self.assertEqual(again, staged)

    def test_command_line_writes_the_staged_model(self) -> None:
        document, _ = studio_vrm1()
        with tempfile.TemporaryDirectory(prefix="vrm-import-") as directory:
            source = Path(directory) / "studio.vrm"
            source.write_bytes(glb_with_bin(document, b"\0" * 4))
            output = Path(directory) / "assets/studio.glb"
            result = subprocess.run(
                [sys.executable, str(SCRIPT), str(source), str(output)],
                capture_output=True,
                text=True,
                timeout=60,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("turned to face -Z", result.stdout)
            self.assertEqual(output.read_bytes(), convert.stage(source.read_bytes())[0])

    def test_conflicting_vrm_versions_fail_without_output(self) -> None:
        document, _ = studio_vrm1()
        document["extensions"]["VRM"] = {"humanoid": {"humanBones": []}}
        with tempfile.TemporaryDirectory(prefix="vrm-import-") as directory:
            source = Path(directory) / "both.vrm"
            source.write_bytes(glb_with_bin(document, b"\0" * 4))
            output = Path(directory) / "both.glb"
            result = subprocess.run(
                [sys.executable, str(SCRIPT), str(source), str(output)],
                capture_output=True,
                text=True,
                timeout=60,
            )
            self.assertEqual(result.returncode, 1)
            self.assertIn("conflicting", result.stderr)
            self.assertFalse(output.exists())

    @unittest.skipUnless(SEED_SAN.is_file(), "optional VRM 1.0 sample not present")
    def test_real_vrm1_export_is_turned(self) -> None:
        source = SEED_SAN.read_bytes()
        self.assertGreater(left_hand_x(source), 0.0)
        staged, turned = convert.stage(source)
        self.assertTrue(turned)
        self.assertLess(left_hand_x(staged), 0.0)
        self.assertEqual(convert.stage(staged), (staged, False))


if __name__ == "__main__":
    unittest.main()
