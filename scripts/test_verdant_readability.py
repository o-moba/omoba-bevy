#!/usr/bin/env python3
"""Check shipped plant proportions and ground contact using actual GLB vertices.

Uses the independent art reader, not the staging transform implementation.
These checks protect the visual height hierarchy without changing world XZ.
"""
from collections import Counter
import unittest

import validate_verdant_assets as assets

reader = assets.ART_READER


def plant_bounds(glb):
    result = {}
    for root in glb.doc["scenes"][glb.doc.get("scene", 0)]["nodes"]:
        node = glb.doc["nodes"][root]
        kind = node.get("extras", {}).get("asset_id")
        if kind not in {"river_reeds", "grass_fan"}:
            continue
        points = []

        def walk(index, parent):
            child = glb.doc["nodes"][index]
            world = reader["matmul"](parent, reader["node_matrix"](child))
            if "mesh" in child:
                for primitive in glb.doc["meshes"][child["mesh"]]["primitives"]:
                    points.extend(assets.transform(world, p) for p in
                                  glb.accessor(primitive["attributes"]["POSITION"]))
            for descendant in child.get("children", []):
                walk(descendant, world)

        walk(root, reader["IDENTITY"])
        result[node["name"]] = {
            "kind": kind,
            "min": tuple(min(p[axis] for p in points) for axis in range(3)),
            "max": tuple(max(p[axis] for p in points) for axis in range(3)),
            "origin": tuple(node.get("translation", [0, 0, 0])),
        }
    return result


class PlantReadabilityTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.source = plant_bounds(assets.GLB(assets.ART / "exports/verdant-confluence.glb"))
        cls.runtime = plant_bounds(assets.GLB(assets.RUNTIME / "foliage.glb"))

    def test_all_plants_retain_original_ground_contact_and_world_footprint(self):
        self.assertEqual(self.source.keys(), self.runtime.keys())
        self.assertEqual(Counter(p["kind"] for p in self.runtime.values()),
                         {"river_reeds": 60, "grass_fan": 241})
        for name, after in self.runtime.items():
            before = self.source[name]
            with self.subTest(plant=name):
                self.assertEqual(after["origin"], before["origin"])
                self.assertAlmostEqual(after["min"][1], before["min"][1], places=5)
                self.assertAlmostEqual(after["min"][1], after["origin"][1], places=5)
                for axis in (0, 2):
                    self.assertAlmostEqual(after["min"][axis], before["min"][axis], places=5)
                    self.assertAlmostEqual(after["max"][axis], before["max"][axis], places=5)

    def test_ordinary_plants_stay_below_hero_height_without_flattening(self):
        for name, plant in self.runtime.items():
            height = plant["max"][1] - plant["min"][1]
            low, high = (.9, 1.8) if plant["kind"] == "river_reeds" else (.5, 1.2)
            with self.subTest(plant=name):
                self.assertGreaterEqual(height, low)
                self.assertLessEqual(height, high)


if __name__ == "__main__":
    unittest.main()
