#!/usr/bin/env python3
"""Geometry, source-integrity and access regressions for the shared forest map."""
import copy
import json
import math
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import generate_verdant_collision as collision


class GeometryTests(unittest.TestCase):
    def test_hierarchy_applies_parent_rotation_to_child_translation(self):
        parent = collision.node_matrix({"translation": [10, 0, -5],
                                        "rotation": [0, math.sqrt(.5), 0, math.sqrt(.5)]})
        child = collision.node_matrix({"translation": [2, 3, 0]})
        actual = collision.transform(collision.multiply(parent, child), (1, 0, 0))
        for value, expected in zip(actual, (10, 3, -8)):
            self.assertAlmostEqual(value, expected)

    def test_trunk_slab_clips_edges_without_original_vertices_in_slab(self):
        triangle = [(0, 0, 0), (2, 2, 0), (-2, 2, 0)]
        clipped = collision.clip_height(collision.clip_height(triangle, .75, True), 1.25, False)
        self.assertEqual(len(clipped), 4)
        self.assertTrue(all(.75 <= p[1] <= 1.25 for p in clipped))
        self.assertEqual({round(p[0], 2) for p in clipped}, {-.75, .75, -1.25, 1.25})

    def test_hull_and_distances_retain_wall_blocking_and_open_gaps(self):
        polygon = collision.hull([(1, 1), (-1, -1), (0, 0), (1, -1), (-1, 1), (1, 1)])
        self.assertEqual(len(polygon), 4)
        self.assertEqual(collision.point_polygon_distance((0, 0), polygon), 0)
        self.assertEqual(collision.polygon_segment_distance(polygon, (-2, 0), (2, 0)), 0)
        self.assertEqual(collision.polygon_segment_distance(polygon, (-2, 2), (2, 2)), 1)
        self.assertEqual(collision.segment_distance((0, 0), (1, 0), (2, 0), (3, 0)), 1)

    def test_conservative_access_does_not_cross_thin_wall(self):
        wall = {"vertices": [(-.05, -5), (.05, -5), (.05, 5), (-.05, 5)]}
        with self.assertRaisesRegex(ValueError, "isolates"):
            collision.connected_access([wall], (-5, -5), (5, 5), [(-3, 0), (3, 0)])


class ShippedGeometryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.document = collision.generate()
        cls.layout = json.loads((collision.ASSETS / "manifest.json").read_text())["layout"]

    def test_committed_artifact_matches_reviewed_source_bytes(self):
        self.assertEqual(collision.OUTPUT.read_text(), collision.encoded(self.document))

    def test_all_trees_are_small_circumscribed_trunks_with_stable_identity(self):
        trees = [o for o in self.document["obstacles"] if o["kind"] == "tree_trunk"]
        self.assertEqual(len(trees), 113)
        for tree in trees:
            self.assertEqual(len(tree["vertices"]), 16)
            center = tree["source"]["trunk_center_xz"]
            radius = tree["source"]["trunk_radius"]
            self.assertGreater(radius, .45)
            self.assertLess(radius, .95)
            self.assertGreaterEqual(min(collision.point_segment_distance(center, a, b)
                                        for a, b in collision.edges(tree["vertices"])), radius - .000001)
        sample = next(o for o in trees if o["id"] == "foliage.glb:tree_jade_canopy / 0071")
        self.assertAlmostEqual(sample["source"]["trunk_center_xz"][0], 26.428366, places=6)
        self.assertAlmostEqual(sample["source"]["trunk_center_xz"][1], -64.347160, places=6)

    def test_missing_root_and_nonconvex_geometry_fail_validation(self):
        missing = copy.deepcopy(self.document)
        missing["obstacles"].pop()
        with self.assertRaisesRegex(ValueError, "count"):
            collision.validate(missing, self.layout)
        malformed = copy.deepcopy(self.document)
        malformed["obstacles"][0]["vertices"].reverse()
        with self.assertRaisesRegex(ValueError, "convex"):
            collision.validate(malformed, self.layout)

    def test_changed_glb_cannot_silently_reuse_manifest_provenance(self):
        with tempfile.TemporaryDirectory() as directory:
            assets = Path(directory)
            (assets / "manifest.json").write_bytes((collision.ASSETS / "manifest.json").read_bytes())
            (assets / "environment.glb").write_bytes(b"altered geometry")
            with mock.patch.object(collision, "ASSETS", assets):
                with self.assertRaisesRegex(ValueError, "source hash mismatch"):
                    collision.generate()

    def test_lane_spawn_and_objective_clearance_and_access(self):
        proof = self.document["validation"]
        self.assertGreaterEqual(proof["minimum_lane_boundary_distance"], 6 + .55)
        self.assertGreater(proof["minimum_spawn_boundary_distance"], 15)
        self.assertGreater(proof["minimum_objective_boundary_distance"], 9)
        self.assertEqual(proof["static_access"]["connected_anchors"], 13)
        self.assertGreater(proof["static_access"]["reachable_certified_cells"], 40000)


if __name__ == "__main__":
    unittest.main()
