#!/usr/bin/env python3
"""Wildspark set GLBs: committed outputs, C4 convention gates and post-processing (no Blender)."""
import copy
import hashlib
import json
from pathlib import Path
import struct
import unittest

import build_wildspark_skin as skin

SHIPPED = skin.ROOT / "client/assets/weapons"


def committed(theme):
    directory = skin.OUTPUT_ROOT / theme
    manifest = json.loads((directory / "manifest.json").read_text())
    return directory, manifest, (directory / manifest["files"]["model"]["path"]).read_bytes()


def mutate(data, change):
    document, binary = skin.read_glb(data)
    document = copy.deepcopy(document)
    change(document)
    return skin.pack_glb(document, binary)


def named(document, name):
    return document["nodes"][skin.node_index(document, name)]


class CommittedSetTests(unittest.TestCase):
    def test_every_theme_matches_its_manifest_and_passes_the_offline_gates(self):
        for theme in skin.THEMES:
            with self.subTest(theme=theme):
                directory, manifest, data = committed(theme)
                for name, record in manifest["files"].items():
                    path = directory / record["path"]
                    self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), record["sha256"], name)
                    self.assertEqual(path.stat().st_size, record["sizeBytes"], name)
                for image, size in skin.IMAGES.items():
                    self.assertEqual(skin.png_size(directory / f"{image}.png"), size, image)
                self.assertEqual(manifest["files"]["model"]["path"], f"{theme}-wildspark-set.glb")
                self.assertEqual(manifest["license"], "CC-BY-4.0")
                self.assertEqual(manifest["attribution"], skin.attribution(theme))
                self.assertEqual(manifest["grip"], skin.GRIP)
                self.assertEqual(manifest["requirement"], "wildspark.handheld.set")
                for gate in ("budgets", "c4", "studio", "registry", "game"):
                    self.assertTrue(manifest["gates"][gate]["ok"], f"{gate} recorded as failing")
                prepared = manifest["gates"]["studio"]["prepared"]
                self.assertRegex(prepared["sha256"], r"^[0-9a-f]{64}$")
                self.assertLess(abs(prepared["sizeBytes"] - len(data)), 64)
                budgets, c4 = skin.budget_gate(data), skin.c4_gate(data)
                self.assertTrue(budgets["ok"], budgets["issues"])
                self.assertTrue(c4["ok"], c4["issues"])
                self.assertEqual(c4["bounds"], manifest["bounds"])
                document, _ = skin.read_glb(data)
                self.assertEqual(document["asset"]["copyright"], skin.attribution(theme))
                stats = skin.glb_stats(document)
                self.assertEqual({k: manifest["glb"][k] for k in stats}, stats)

    def test_baseline_envelopes_are_the_shipped_wildspark_bounds(self):
        for kind, path in (("repeater", "wildspark-repeater.glb"), ("launcher", "wild-launcher.glb")):
            document, _ = skin.read_glb((SHIPPED / path).read_bytes())
            lo, hi = skin.mesh_bounds(document)
            expected_lo, expected_hi = skin.BASELINE_BOUNDS[kind]
            for actual, expected in zip(lo + hi, expected_lo + expected_hi):
                self.assertAlmostEqual(actual, expected, delta=1e-3, msg=kind)

    def test_a_single_gun_is_not_a_set(self):
        result = skin.c4_gate((SHIPPED / "wildspark-repeater.glb").read_bytes())
        self.assertFalse(result["ok"])


class PostProcessTests(unittest.TestCase):
    def setUp(self):
        _, _, self.data = committed("candy-neon")

    def test_finalize_writes_extras_credit_padding_and_hides_the_launcher(self):
        def raw_export(document):
            named(document, skin.LAUNCHER).pop("scale")
            document["asset"] = {"version": "2.0", "generator": "Khronos glTF Blender I/O"}

        raw = mutate(self.data, raw_export)
        self.assertFalse(skin.c4_gate(raw)["ok"])
        final = skin.finalize(raw, "Example set — by someone; CC-BY-4.0")
        magic, version, length = struct.unpack_from("<4sII", final)
        self.assertEqual((magic, version, length), (b"glTF", 2, len(final)))
        self.assertEqual(struct.unpack_from("<I", final, 12)[0] % 4, 0)
        document, _ = skin.read_glb(final)
        self.assertEqual(named(document, skin.LAUNCHER)["scale"], [0, 0, 0])
        self.assertEqual(document["asset"]["extras"]["ekza_handheld_v1"], skin.GRIP)
        self.assertEqual(document["asset"]["extras"]["omoba_wildspark_set_v1"], skin.SET_HINT)
        self.assertEqual(document["asset"]["copyright"], "Example set — by someone; CC-BY-4.0")
        self.assertEqual(document["asset"]["generator"], "Khronos glTF Blender I/O")
        self.assertTrue(skin.c4_gate(final)["ok"])
        self.assertTrue(skin.budget_gate(final)["ok"])

    def test_finalize_needs_the_set_nodes(self):
        broken = mutate(self.data, lambda d: named(d, skin.REPEATER).update(name="Repeater"))
        with self.assertRaises(ValueError):
            skin.finalize(broken, "credit")

    def test_container_errors_are_rejected(self):
        with self.assertRaises(ValueError):
            skin.read_glb(self.data[:-4])
        self.assertFalse(skin.budget_gate(self.data + b"\0\0\0\0")["ok"])


class ConventionGateTests(unittest.TestCase):
    def setUp(self):
        _, _, self.data = committed("scrap-punk")

    def assertRejected(self, change, gate=skin.c4_gate):
        result = gate(mutate(self.data, change))
        self.assertFalse(result["ok"])
        self.assertTrue(result["issues"])

    def test_names_and_hierarchy(self):
        self.assertRejected(lambda d: named(d, skin.ROTOR).update(name="WildsparkRotor.001"))
        self.assertRejected(lambda d: named(d, skin.SLIDE).update(name="Slide"))
        self.assertRejected(lambda d: named(d, skin.REPEATER).update(name="WildsparkSetRepeater.001"))

        def rotor_under_launcher(document):
            rotor = skin.node_index(document, skin.ROTOR)
            named(document, skin.REPEATER)["children"].remove(rotor)
            named(document, skin.LAUNCHER)["children"].append(rotor)

        self.assertRejected(rotor_under_launcher)

    def test_pivots_rest_pose_and_hidden_launcher(self):
        self.assertRejected(lambda d: named(d, skin.SLIDE).update(translation=[0, 0.13, 0.25]))
        self.assertRejected(lambda d: named(d, skin.ROTOR).update(rotation=[0, 0, 0.3826834, 0.9238795]))
        self.assertRejected(lambda d: named(d, skin.LAUNCHER).update(scale=[1, 1, 1]))
        self.assertRejected(lambda d: named(d, skin.REPEATER).update(translation=[0, 0.01, 0]))

    def test_envelope_and_muzzle(self):
        def stretch(document, axis, value):
            node = named(document, "WildsparkRepeaterReceiver")
            accessor = document["meshes"][node["mesh"]]["primitives"][0]["attributes"]["POSITION"]
            document["accessors"][accessor]["max"][axis] = value

        self.assertRejected(lambda d: stretch(d, 0, 0.3))  # wider than the repeater envelope
        self.assertRejected(lambda d: stretch(d, 2, 0.8))  # geometry past the muzzle tip

    def test_extras(self):
        self.assertRejected(lambda d: d["asset"]["extras"].pop("omoba_wildspark_set_v1"))
        self.assertRejected(lambda d: d["asset"]["extras"]["ekza_handheld_v1"].update(scale=1.5))

    def test_budgets(self):
        self.assertRejected(lambda d: d.setdefault("extensionsUsed", []).append("KHR_materials_emissive_strength"),
                            skin.budget_gate)
        self.assertRejected(lambda d: d["materials"][0].setdefault("extensions", {}).update(
            KHR_materials_emissive_strength={"emissiveStrength": 4}), skin.budget_gate)
        self.assertRejected(lambda d: d.update(animations=[{"channels": [], "samplers": []}]), skin.budget_gate)
        self.assertRejected(lambda d: d["nodes"].extend({"name": f"extra{i}"} for i in range(60)), skin.budget_gate)
        self.assertRejected(lambda d: d["scenes"].append({"nodes": []}), skin.budget_gate)


class OutputGuardTests(unittest.TestCase):
    def test_outputs_never_go_into_client_assets_or_the_reference_blend_folder(self):
        for path in ("client/assets/weapons", "client", "assets-src/skills"):
            with self.assertRaises(SystemExit):
                skin.checked_output_dir(skin.ROOT / path)
        self.assertEqual(skin.checked_output_dir(skin.OUTPUT_ROOT / "candy-neon"),
                         (skin.OUTPUT_ROOT / "candy-neon").resolve())


if __name__ == "__main__":
    unittest.main()
