#!/usr/bin/env python3
"""Contract tests for the Ekza rendition builder; standard library only.

The registry runs scripts/ekza_build_rendition.py as an external command, so these
tests pin what it may rely on: one JSON document on stdout, exit 0 / 2, a
content-addressed output file, and an output that passes Omoba's own profile check.
Both VRM 0.x (`VRM`) and VRM 1.0 (`VRMC_vrm`) sources are covered.
"""
from __future__ import annotations

import hashlib
import json
import math
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ekza_publish
import retarget_animations as retarget

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/ekza_build_rendition.py"
# A shipped avatar keeps its VRM humanoid metadata, so it doubles as a real source.
SOURCE = ROOT / "client/assets/avatars/anna.glb"
# Optional real VRM 1.0 export (Ekza Studio fixture, VRM Public License 1.0, credit
# required). It is read in place and never copied into this repository; the test
# skips without it.
SEED_SAN = Path(
    os.environ.get(
        "OMOBA_VRM1_SAMPLE",
        Path.home() / "git/ekza/ekza-studio/fixtures/models/seed-san.vrm",
    )
)


def glb(document: dict) -> bytes:
    body = json.dumps(document).encode()
    body += b" " * (-len(body) % 4)
    return struct.pack("<4sII", b"glTF", 2, 20 + len(body)) + struct.pack("<II", len(body), 0x4E4F534A) + body


def glb_with_bin(document: dict, binary: bytes) -> bytes:
    doc = retarget.Gltf(document, binary)
    return doc.to_glb_bytes()


def studio_vrm1() -> dict:
    """A minimal VRM 1.0 rig shaped like an Ekza Studio export.

    Identity rest rotations except a 2 degree elbow/knee flexion, +Z facing (left
    hand at +X), no upperChest, VRM 1.0 thumb names, KHR_materials_unlit only.
    """
    nodes: list[dict] = []
    bones: dict[str, int] = {}

    def bone(name, translation, parent=None, rotation=None):
        index = len(nodes)
        node = {"name": f"J_{name}", "translation": list(translation)}
        if rotation:
            node["rotation"] = list(rotation)
        nodes.append(node)
        bones[name] = index
        if parent is not None:
            nodes[bones[parent]].setdefault("children", []).append(index)
        return name

    flex = math.radians(2.0)
    bone("hips", (0.0, 0.9, 0.0))
    bone("spine", (0.0, 0.1, 0.0), "hips")
    bone("chest", (0.0, 0.15, 0.0), "spine")
    bone("neck", (0.0, 0.2, 0.0), "chest")
    bone("head", (0.0, 0.1, 0.0), "neck")
    for side, x in (("left", 1.0), ("right", -1.0)):
        bone(f"{side}Shoulder", (0.04 * x, 0.15, 0.0), "chest")
        bone(f"{side}UpperArm", (0.1 * x, 0.0, 0.0), f"{side}Shoulder")
        # Elbow flexion bends the forearm toward the front (+Z).
        yaw = (0.0, -x * math.sin(flex / 2), 0.0, math.cos(flex / 2))
        bone(f"{side}LowerArm", (0.25 * x, 0.0, 0.0), f"{side}UpperArm", yaw)
        bone(f"{side}Hand", (0.25 * x, 0.0, 0.0), f"{side}LowerArm")
        bone(f"{side}ThumbMetacarpal", (0.02 * x, 0.0, 0.02), f"{side}Hand")
        bone(f"{side}ThumbProximal", (0.03 * x, 0.0, 0.0), f"{side}ThumbMetacarpal")
        bone(f"{side}ThumbDistal", (0.02 * x, 0.0, 0.0), f"{side}ThumbProximal")
        bone(f"{side}IndexProximal", (0.08 * x, 0.0, 0.01), f"{side}Hand")
        bone(f"{side}IndexIntermediate", (0.03 * x, 0.0, 0.0), f"{side}IndexProximal")
        bone(f"{side}IndexDistal", (0.02 * x, 0.0, 0.0), f"{side}IndexIntermediate")
        bone(f"{side}UpperLeg", (0.1 * x, -0.05, 0.0), "hips")
        # Knee flexion bends the shin toward the back (-Z).
        pitch = (math.sin(flex / 2), 0.0, 0.0, math.cos(flex / 2))
        bone(f"{side}LowerLeg", (0.0, -0.4, 0.0), f"{side}UpperLeg", pitch)
        bone(f"{side}Foot", (0.0, -0.4, 0.0), f"{side}LowerLeg")
        bone(f"{side}Toes", (0.0, -0.05, 0.1), f"{side}Foot")
    mesh = len(nodes)
    nodes.append({"name": "Body", "mesh": 0, "skin": 0})
    return {
        "asset": {"version": "2.0", "generator": "synthetic Ekza Studio-like VRM 1.0"},
        "extensionsUsed": ["KHR_materials_unlit", "VRMC_vrm"],
        "extensions": {
            "VRMC_vrm": {
                "specVersion": "1.0",
                "meta": {"name": "synthetic", "authors": ["omoba tests"], "licenseUrl": "https://vrm.dev/licenses/1.0/"},
                "humanoid": {"humanBones": {name: {"node": node} for name, node in bones.items()}},
            }
        },
        "materials": [{"extensions": {"KHR_materials_unlit": {}}}],
        "meshes": [{"primitives": []}],
        "nodes": nodes,
        "scene": 0,
        "scenes": [{"nodes": [bones["hips"], mesh]}],
        "skins": [{"joints": [bones[name] for name in bones]}],
        "buffers": [{"byteLength": 4}],
    }, bones


def vrm1_from_vrm0(data: bytes) -> bytes:
    """Re-declare a shipped VRM 0.x avatar as VRM 1.0 and turn it to face +Z."""
    doc = retarget.Gltf.from_glb_bytes(data)
    vrm0 = doc.js["extensions"].pop("VRM")
    bones = {}
    for entry in vrm0["humanoid"]["humanBones"]:
        name = entry["bone"]
        if name.endswith("ThumbProximal"):
            name = name.replace("ThumbProximal", "ThumbMetacarpal")
        elif name.endswith("ThumbIntermediate"):
            name = name.replace("ThumbIntermediate", "ThumbProximal")
        bones[name] = {"node": entry["node"]}
    doc.js["extensions"]["VRMC_vrm"] = {"specVersion": "1.0", "humanoid": {"humanBones": bones}}
    doc.js["extensionsUsed"] = ["VRMC_vrm" if e == "VRM" else e for e in doc.js["extensionsUsed"]]
    retarget.turn_roots_180(doc)
    return doc.to_glb_bytes()


def rest_world(data: bytes):
    doc = retarget.Gltf.from_glb_bytes(data)
    return doc, doc.world_pose(doc.rest_local_pose())


def clip_streams(data: bytes, clip: str) -> dict:
    doc = retarget.Gltf.from_glb_bytes(data)
    animation = next(a for a in doc.js["animations"] if a["name"] == clip)
    return {
        (channel["target"]["node"], channel["target"]["path"]): doc.read_accessor(
            animation["samplers"][channel["sampler"]]["output"]
        )
        for channel in animation["channels"]
    }


def run(source: Path, output: Path) -> tuple[int, dict, str]:
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "--source", str(source), "--output-dir", str(output)],
        capture_output=True,
        text=True,
        timeout=120,
    )
    return result.returncode, json.loads(result.stdout), result.stdout


class BuildRenditionContract(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory(prefix="ekza-build-test-")
        self.output = Path(self.directory.name)

    def tearDown(self) -> None:
        self.directory.cleanup()

    def test_real_avatar_builds_a_content_addressed_playable_rendition(self) -> None:
        code, result, stdout = run(SOURCE, self.output)
        self.assertEqual(code, 0, stdout)
        self.assertEqual(stdout.count("\n"), 1, "stdout must be exactly one JSON line")
        self.assertTrue(result["ok"])
        self.assertEqual(result["profile"], "desktop/humanoid-glb-v1")
        self.assertEqual(result["format"], "glb")
        data = Path(result["assetPath"]).read_bytes()
        self.assertEqual(Path(result["assetPath"]).name, result["sha256"] + ".glb")
        self.assertEqual(hashlib.sha256(data).hexdigest(), result["sha256"])
        self.assertEqual(len(data), result["sizeBytes"])
        self.assertEqual(result["report"]["clips"], ["idle", "walk", "attack", "cast", "death"])
        self.assertEqual(result["report"]["sourceSha256"], hashlib.sha256(SOURCE.read_bytes()).hexdigest())
        ekza_publish.validate_omoba_profile(data)  # the same rule the game enforces
        self.assertEqual([p.name for p in self.output.iterdir()], [result["sha256"] + ".glb"])

    def test_repeated_build_is_idempotent(self) -> None:
        first = run(SOURCE, self.output)[1]
        second = run(SOURCE, self.output)[1]
        self.assertEqual(first["sha256"], second["sha256"])
        self.assertEqual(len(list(self.output.iterdir())), 1)

    def test_unusable_sources_are_reported_not_crashed(self) -> None:
        cases = {
            "source_container": b"this is not a model, it is a sentence of plain text",
            "source_not_vrm": glb({"asset": {"version": "2.0"}, "nodes": [{}]}),
            "build_failed": glb(
                {"asset": {"version": "2.0"}, "nodes": [{}], "extensions": {"VRM": {"humanoid": {"humanBones": []}}}}
            ),
        }
        for expected, payload in cases.items():
            with self.subTest(expected):
                source = self.output / f"{expected}.vrm"
                source.write_bytes(payload)
                code, result, stdout = run(source, self.output / "out")
                self.assertEqual(code, 2, stdout)
                self.assertFalse(result["ok"])
                self.assertEqual(result["issues"][0]["code"], expected, stdout)
                self.assertTrue(result["issues"][0]["message"])
                self.assertFalse((self.output / "out").exists() and any((self.output / "out").iterdir()))

    def test_conflicting_vrm_versions_are_refused(self) -> None:
        document, _ = studio_vrm1()
        document["extensions"]["VRM"] = {"humanoid": {"humanBones": []}}
        source = self.output / "both.vrm"
        source.write_bytes(glb_with_bin(document, b"\0" * 4))
        code, result, stdout = run(source, self.output / "out")
        self.assertEqual(code, 2, stdout)
        self.assertEqual(result["issues"][0]["code"], "build_failed")
        self.assertIn("conflicting", result["issues"][0]["message"])

    def test_limits_match_the_published_profile(self) -> None:
        self.assertEqual(ekza_publish.MAX_MODEL_BYTES, 50 * 1024 * 1024)
        self.assertEqual(ekza_publish.REQUIRED_CLIPS, ("idle", "walk", "attack", "cast", "death"))
        self.assertEqual(ekza_publish.PROJECT_SELECTORS["omoba"], ("desktop", "humanoid-glb-v1"))


class Vrm1Renditions(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory(prefix="ekza-build-vrm1-")
        self.output = Path(self.directory.name)

    def tearDown(self) -> None:
        self.directory.cleanup()

    def build(self, name: str, data: bytes) -> tuple[dict, bytes]:
        source = self.output / name
        source.write_bytes(data)
        code, result, stdout = run(source, self.output / "out")
        self.assertEqual(code, 0, stdout)
        self.assertTrue(result["ok"])
        built = Path(result["assetPath"]).read_bytes()
        ekza_publish.validate_omoba_profile(built)
        self.assertEqual(result["report"]["clips"], ["idle", "walk", "attack", "cast", "death"])
        return result, built

    def test_studio_shaped_vrm1_builds_and_faces_like_vrm0(self) -> None:
        document, bones = studio_vrm1()
        result, built = self.build("studio.vrm", glb_with_bin(document, b"\0" * 4))
        self.assertEqual(result["report"]["sourceFormat"], "vrm1")
        doc, world = rest_world(built)
        # VRM 1.0 metadata and the unlit material extension survive the bake.
        self.assertIn("VRMC_vrm", doc.js["extensions"])
        self.assertNotIn("VRM", doc.js["extensions"])
        self.assertEqual(doc.js["extensionsUsed"], ["KHR_materials_unlit", "VRMC_vrm"])
        # The rest now faces -Z: the character's left hand is at -X, like VRM 0.x.
        self.assertLess(world[bones["leftHand"]][0][0], -0.3)
        self.assertGreater(world[bones["rightHand"]][0][0], 0.3)
        self.assertTrue(doc.js["asset"]["extras"]["omoba_rest_yaw180"])
        # Only the scene roots were turned; child rest transforms are untouched.
        self.assertEqual(doc.nodes[bones["leftLowerArm"]]["rotation"], document["nodes"][bones["leftLowerArm"]]["rotation"])
        self.assertEqual(doc.nodes[bones["spine"]]["translation"], document["nodes"][bones["spine"]]["translation"])
        for clip in ("idle", "walk", "attack", "cast", "death"):
            streams = clip_streams(built, clip)
            # VRM 1.0 thumbs are driven: metacarpal by the source thumb root.
            for name in ("leftThumbMetacarpal", "leftThumbProximal", "leftThumbDistal", "rightThumbMetacarpal"):
                self.assertIn((bones[name], "rotation"), streams, (clip, name))
            self.assertIn((bones["hips"], "translation"), streams)
            for name in ("leftUpperLeg", "rightLowerLeg", "leftUpperArm", "head", "chest"):
                self.assertIn((bones[name], "rotation"), streams, (clip, name))
        # The legs really move during walk.
        thigh = clip_streams(built, "walk")[(bones["leftUpperLeg"], "rotation")]
        spread = max(abs(sum(a * b for a, b in zip(thigh[0], q))) for q in thigh)
        least = min(abs(sum(a * b for a, b in zip(thigh[0], q))) for q in thigh)
        self.assertGreater(spread - least, 0.01, "walk thigh rotation must vary")

    def test_rebake_of_a_vrm1_rendition_does_not_turn_it_again(self) -> None:
        document, bones = studio_vrm1()
        _, built = self.build("studio.vrm", glb_with_bin(document, b"\0" * 4))
        path = self.output / "again.glb"
        path.write_bytes(built)
        rig = retarget.SourceRig(retarget.Gltf.from_gltf_file(ROOT / retarget.SOURCE_GLTF))
        clips = {name: rig.clip(name) for name, _ in retarget.CLIP_MAP}
        retarget.retarget_avatar(rig, clips, path)
        self.assertEqual(path.read_bytes(), built)

    def test_vrm1_matches_the_same_rig_declared_as_vrm0(self) -> None:
        """Facing and thumb renaming are exact: VRM0 and VRM1 bakes agree per node."""
        vrm0 = SOURCE.read_bytes()
        _, built0 = self.build("anna0.vrm", vrm0)
        result1, built1 = self.build("anna1.vrm", vrm1_from_vrm0(vrm0))
        self.assertEqual(result1["report"]["sourceFormat"], "vrm1")
        for clip in ("idle", "walk", "attack", "cast", "death"):
            streams0, streams1 = clip_streams(built0, clip), clip_streams(built1, clip)
            self.assertEqual(set(streams0), set(streams1), clip)
            worst = 0.0
            for key, values0 in streams0.items():
                for a, b in zip(values0, streams1[key]):
                    if key[1] == "rotation":
                        worst = max(worst, 1.0 - abs(sum(x * y for x, y in zip(a, b))))
                    else:
                        worst = max(worst, max(abs(x - y) for x, y in zip(a, b)))
            self.assertLess(worst, 1e-5, clip)

    def test_vrm0_rendition_is_never_turned(self) -> None:
        _, built = self.build("anna.vrm", SOURCE.read_bytes())
        source = retarget.Gltf.from_glb_bytes(SOURCE.read_bytes())
        doc = retarget.Gltf.from_glb_bytes(built)
        self.assertEqual(doc.nodes, source.nodes)
        self.assertNotIn("omoba_rest_yaw180", doc.js["asset"]["extras"])

    @unittest.skipUnless(SEED_SAN.is_file(), "optional VRM 1.0 sample not present")
    def test_real_vrm1_export_builds(self) -> None:
        result, built = self.build("seed-san.vrm", SEED_SAN.read_bytes())
        self.assertEqual(result["report"]["sourceFormat"], "vrm1")
        doc, world = rest_world(built)
        humanoid = retarget.humanoid_bone_map(doc.js)[1]
        self.assertLess(world[humanoid["leftHand"]][0][0], 0.0)
        self.assertIn((humanoid["leftThumbProximal"], "rotation"), clip_streams(built, "attack"))


if __name__ == "__main__":
    unittest.main()
