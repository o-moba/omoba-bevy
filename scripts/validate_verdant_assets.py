#!/usr/bin/env python3
"""Validate staged Verdant GLBs against their actual world-space triangles.

No rendering, Blender mutation, or third-party Python packages. The imported
art validator supplies an independent strict GLB reader; this script does not
use the staging script's mesh reader or its height mutation implementation.
"""
import argparse
from collections import defaultdict
import datetime
import hashlib
import json
import math
from pathlib import Path
import runpy
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
ART = ROOT / "art/verdant-confluence"
RUNTIME = ROOT / "client/assets/verdant"
RAW = ROOT / ".agent/tasks/VERDANT-3D-RELEASE-2026-09-05/raw"
INPUT_INVENTORY = ROOT / "scripts/verdant_source_inventory.json"
ART_READER = runpy.run_path(str(ART / "scripts/validate_art.py"), run_name="verdant_art_reader")
GLB = ART_READER["GLB"]
transform = ART_READER["transform"]
require = ART_READER["require"]
ASSETS = {"environment", "foliage", "watchtower_green", "watchtower_blue", "sanctuary_green", "sanctuary_blue"}
DYNAMIC_ROLES = {"base_landmark", "lane_tower"}
TOLERANCE = 0.05


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def runtime_ground(x, z):
    """Client terrain_height_3d contract; agreement is tested against GLB rays.

    These are the unchanged runtime layout constants, not values trusted from
    the manifest being validated. Rust tests exercise the actual client method.
    """
    center = 225 / math.sqrt(2) / 2
    return max(0.7 * max(0.0, min(1.0, (29 - max(abs(x - c), abs(z - c))) / 6))
               for c in (-center, center))


def iter_triangles(glb, node_filter=lambda node: True):
    for node, matrix in glb.world_nodes():
        if "mesh" not in node or not node_filter(node):
            continue
        for primitive in glb.doc["meshes"][node["mesh"]]["primitives"]:
            vertices = [transform(matrix, point) for point in glb.accessor(primitive["attributes"]["POSITION"])]
            require(all(math.isfinite(v) for point in vertices for v in point), "Non-finite world geometry")
            ids = glb.indices(primitive, len(vertices))
            for offset in range(0, len(ids), 3):
                yield node.get("name", "unnamed"), tuple(vertices[index] for index in ids[offset:offset + 3])


def projected_height(triangle, x, z):
    """Barycentric ray intersection in XZ, interpolating actual vertex Y."""
    a, b, c = triangle
    determinant = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2])
    if abs(determinant) < 1e-10:
        return None
    u = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / determinant
    v = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / determinant
    w = 1 - u - v
    if min(u, v, w) < -2e-5:
        return None
    return u * a[1] + v * b[1] + w * c[1]


class SurfaceIndex:
    def __init__(self, triangles):
        self.triangles = list(triangles)
        self.cells = defaultdict(list)
        for index, (_, triangle) in enumerate(self.triangles):
            if abs((triangle[1][0] - triangle[0][0]) * (triangle[2][2] - triangle[0][2])
                   - (triangle[1][2] - triangle[0][2]) * (triangle[2][0] - triangle[0][0])) < 1e-10:
                continue
            for gx in range(math.floor(min(p[0] for p in triangle) / 8), math.floor(max(p[0] for p in triangle) / 8) + 1):
                for gz in range(math.floor(min(p[2] for p in triangle) / 8), math.floor(max(p[2] for p in triangle) / 8) + 1):
                    self.cells[(gx, gz)].append(index)

    def sample(self, x, z):
        hits = []
        for index in self.cells.get((math.floor(x / 8), math.floor(z / 8)), []):
            name, triangle = self.triangles[index]
            height = projected_height(triangle, x, z)
            if height is not None:
                hits.append((height, name))
        return max(hits) if hits else None


def walkable_node(node):
    name = node.get("name", "")
    # Geometry roles, never Y-based filtering: raised decks/ramp regressions
    # must fail instead of being dropped in favor of the ground beneath them.
    return (name.startswith(("Landscape / meadow", "River /", "Route", "Base / walk-up ramp",
                             "Base / ceremonial", "Base / inner circuit", "Base / radial inlay",
                             "River crossing / corner watergate", "River crossing / watergate inlay",
                             "Objective / tower"))
            or (name.startswith("Base ") and ("46m pad" in name or "/ paving" in name))
            or node.get("name", "").startswith("bridge"))


def static_exclusions(glbs, source, manifest):
    expected = [n["name"] for n, _ in source.world_nodes() if n.get("extras", {}).get("role") in DYNAMIC_ROLES]
    require(len(expected) == 8 and len(set(expected)) == 8, "Source must contain eight distinct live structure roots")
    require(set(manifest["excluded_live_structure_roots"]) == set(expected), "Excluded root inventory differs from source")
    for kind in ("environment", "foliage"):
        for node, _ in glbs[kind].world_nodes():
            extras = node.get("extras", {})
            require(extras.get("role") not in DYNAMIC_ROLES, f"Baked live structure role in {kind}")
            require(not extras.get("asset_id", "").startswith(("watchtower_", "sanctuary_")), f"Baked structure asset in {kind}")
            require(node.get("name") not in expected, f"Baked structure name in {kind}")
    return {"status": "PASS", "excluded_source_roots": expected, "static_scene_baked_structures": 0}


def surface_checks(environment, structures):
    index = SurfaceIndex(iter_triangles(environment, walkable_node))
    require(index.triangles, "No walkable runtime triangles")
    samples = []
    joins = []

    def sample(label, x, z, custom_index=None, expected=None):
        hit = (custom_index or index).sample(x, z)
        predicted = runtime_ground(x, z) if expected is None else expected
        actual, node = hit if hit else (None, None)
        error = abs(actual - predicted) if actual is not None else None
        result = {"label": label, "xz": [x, z], "client_height": predicted, "triangle_height": actual,
                  "surface_node": node, "error_m": error, "status": "PASS" if error is not None and error <= TOLERANCE else "FAIL"}
        samples.append(result)
        return result

    center = 225 / math.sqrt(2) / 2
    for base in (-center, center):
        for tag, dx, dz in [("center", 0, 0), ("top", 13, 8), ("edge", 23, 0),
                            ("corner_inside_join", 22.999, 22.999), ("corner_outside_join", 23.001, 23.001),
                            ("side_mid", 26, 0), ("side_end", 29, 0), ("unequal_corner", 24, 26),
                            ("unequal_corner_transposed", 26, 24), ("diagonal_limit", 29, 29)]:
            for sx, sz in ((1, 1), (-1, 1), (1, -1), (-1, -1)):
                sample(f"pad_{base:+.1f}/{tag}/{sx},{sz}", base + sx * dx, base + sz * dz)
        # Dense rays on the actual shared miter edges and side walk-off line.
        inward = -math.copysign(1, base)
        for direction in ((inward, 0), (inward, inward), (inward, inward * .93)):
            previous = None
            for step in range(121):
                d = 22 + step / 15
                item = sample(f"pad_{base:+.1f}/join_{direction}/{step}", base + d * direction[0], base + d * direction[1])
                if previous and item["triangle_height"] is not None and previous["triangle_height"] is not None:
                    # Remove the legitimate continuous ramp slope; detect extra
                    # steps at pad/skirt/road joins beyond the same 0.05 budget.
                    residual = abs((item["triangle_height"] - previous["triangle_height"])
                                   - (item["client_height"] - previous["client_height"]))
                    joins.append({"label": item["label"], "residual_step_m": residual,
                                  "status": "PASS" if residual <= TOLERANCE else "FAIL"})
                previous = item
    # Paver longitudinal joins occur at integer local-X positions; include
    # strip centers and cross-lane strips inside the unchanged 12m width.
    for across in (-5.5, -3, 0, 3, 5.5):
        previous = None
        for step in range(181):
            along = -18 + step * .2
            x, z = (along + across) / math.sqrt(2), (along - across) / math.sqrt(2)
            item = sample(f"central_bridge/strip_{across}/{along:.1f}", x, z)
            if previous and item["triangle_height"] is not None and previous["triangle_height"] is not None:
                delta = abs(item["triangle_height"] - previous["triangle_height"])
                joins.append({"label": item["label"], "residual_step_m": delta,
                              "status": "PASS" if delta <= TOLERANCE else "FAIL"})
            previous = item
    edge = center + 29 - 12
    for sign in (-1, 1):
        cx, cz = sign * edge, -sign * edge
        for direction in ((-sign, 0), (0, sign)):
            previous = None
            for step in range(91):
                d = step * .2
                item = sample(f"outer_{sign}/route_{direction}/{d:.1f}", cx + direction[0] * d, cz + direction[1] * d)
                if previous and item["triangle_height"] is not None and previous["triangle_height"] is not None:
                    delta = abs(item["triangle_height"] - previous["triangle_height"])
                    joins.append({"label": item["label"], "residual_step_m": delta,
                                  "status": "PASS" if delta <= TOLERANCE else "FAIL"})
                previous = item
    # Foundation top just outside tower collision radius; core masonry is an
    # authoritative obstacle, not a claimed walkable surface.
    for x, source_y in ART_READER["expected_layout"]()["towers_blender"]:
        for dx, dz in ((3.8, 0), (-3.8, 0), (0, 3.8), (0, -3.8)):
            sample("tower_foundation", x + dx, -source_y + dz)
    # All triangles of each rotated sanctuary are tested at the actual spawn
    # approach, so a retained buttress or raised dais can obstruct/fail the ray.
    for name, glb in structures.items():
        if not name.startswith("sanctuary"):
            continue
        rotated = []
        for node, triangle in iter_triangles(glb):
            rotated.append((node, tuple(((x + z) / math.sqrt(2), y, (-x + z) / math.sqrt(2)) for x, y, z in triangle)))
        sanctuary = SurfaceIndex(rotated)
        for sign in (-1, 1):
            for radius in (6.8, 7, 7.2, 8, 8.8, 9.0):
                d = sign * radius / math.sqrt(2)
                sample(f"{name}/spawn_floor/{sign}/{radius}", d, d, sanctuary, 0.0)
    failures = [s for s in samples if s["status"] != "PASS"] + [j for j in joins if j["status"] != "PASS"]
    return {"status": "PASS" if not failures else "FAIL", "raycast_space": "exported glTF/Bevy world XZ with Y interpolated from triangles",
            "walkable_triangle_count": len(index.triangles), "sample_count": len(samples), "join_count": len(joins),
            "tolerance_m": TOLERANCE, "maximum_error_m": max((s["error_m"] or 0 for s in samples), default=0),
            "maximum_join_residual_m": max((j["residual_step_m"] for j in joins), default=0),
            "failures": failures, "samples": samples, "joins": joins,
            "scope": "Pads/ramps, lane crossings, tower foundation edges and rotated sanctuary spawn floors. Altar and tower core are authoritative obstacles; garden planting and railings are not claimed walking decks."}


def validate(runtime, raw, inventory, original_art=None):
    report = {"status": "FAIL", "checked_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "runtime": str(runtime), "errors": [], "files": {},
              "grounding_implementation": {"path": "client/src/maps.rs", "function": "MapLayout::terrain_height_3d",
                                           "sha256": sha(ROOT / "client/src/maps.rs"),
                                           "oracle": "Preserved MapLayout constants and miter formula, separately exercised by Rust map/player regression tests."}}
    raw.mkdir(parents=True, exist_ok=True)
    try:
        source_inventory = json.loads(inventory.read_text())
        require(len(source_inventory) == 40, "Expected original 40-file input identity")
        source_hashes = {row["path"]: sha(ROOT / row["path"]) for row in source_inventory}
        require(all(source_hashes[row["path"]] == row["sha256"] for row in source_inventory), "Imported source differs from frozen input inventory")
        report["source_identity"] = {"status": "PASS", "files": len(source_inventory), "inventory": str(inventory)}
        original_hashes = {}
        if original_art is not None:
            for row in source_inventory:
                path = original_art / Path(row["path"]).relative_to("art/verdant-confluence")
                original_hashes[path] = sha(path)
                require(original_hashes[path] == row["sha256"], "Original checkout source changed: " + str(path))
            report["original_source_identity"] = {"status": "PASS", "root": str(original_art), "files": len(original_hashes)}
        manifest = json.loads((runtime / "manifest.json").read_text())
        require({p.stem for p in runtime.glob("*.glb")} == ASSETS, "Runtime GLB inventory must contain exactly six assets")
        require({row["id"] for row in manifest["files"]} == ASSETS and len(manifest["files"]) == 6, "Manifest asset inventory mismatch")
        require(manifest["source_sha256"] == sha(ART / "exports/verdant-confluence.glb"), "Source export hash mismatch")
        require(manifest["source_blend_sha256"] == sha(ART / "verdant-confluence.blend"), "Source Blender hash mismatch")
        assimp = shutil.which("assimp")
        require(assimp, "Independent assimp importer is required")
        glbs = {}
        for row in manifest["files"]:
            path = runtime / row["path"]
            require(path.parent == runtime and path.name == row["id"] + ".glb", "Unsafe/noncanonical runtime asset path")
            require(row["sha256"] == sha(path) and row["bytes"] == path.stat().st_size, "Runtime file hash/length mismatch: " + path.name)
            glb = GLB(path)
            glbs[row["id"]] = glb
            # Exercise all instantiated world transforms, not just unique mesh buffers.
            count = sum(1 for _ in iter_triangles(glb))
            require(count > 0, "No reachable scene geometry")
            for material in glb.doc.get("materials", []):
                def finite_json(value):
                    if isinstance(value, (int, float)):
                        return math.isfinite(value)
                    if isinstance(value, dict):
                        return all(finite_json(v) for v in value.values())
                    if isinstance(value, list):
                        return all(finite_json(v) for v in value)
                    return True
                require(finite_json(material), "Non-finite material values")
            result = subprocess.run([assimp, "info", str(path)], capture_output=True, text=True, timeout=60)
            (raw / ("verdant-assimp-" + row["id"] + ".log")).write_text(result.stdout + result.stderr)
            require(result.returncode == 0, "Independent assimp import failed: " + path.name)
            report["files"][path.name] = dict(glb.summary, sha256=sha(path), world_triangles=count, assimp="PASS")
        report["static_exclusions"] = static_exclusions(glbs, GLB(ART / "exports/verdant-confluence.glb"), manifest)
        report["surfaces"] = surface_checks(glbs["environment"], glbs)
        if report["surfaces"]["status"] != "PASS":
            report["errors"].append("Actual runtime walkable geometry disagrees with client grounding")
        with tempfile.TemporaryDirectory(prefix="verdant-repeat-", dir=raw) as repeat:
            run = subprocess.run([sys.executable, str(ROOT / "scripts/stage_verdant.py"), "--output", repeat],
                                 capture_output=True, text=True, timeout=60)
            (raw / "verdant-second-derivation.log").write_text(run.stdout + run.stderr)
            require(run.returncode == 0, "Second derivation failed")
            comparisons = []
            for name in sorted([n + ".glb" for n in ASSETS] + ["manifest.json"]):
                comparisons.append({"path": name, "candidate_sha256": sha(runtime / name), "repeat_sha256": sha(Path(repeat) / name)})
            require(all(row["candidate_sha256"] == row["repeat_sha256"] for row in comparisons), "Second derivation is not byte-identical")
            report["determinism"] = {"status": "PASS", "scope": "All six GLBs and manifest are byte-identical; no variable exporter metadata.", "files": comparisons}
        require(all(sha(ROOT / name) == digest for name, digest in source_hashes.items()), "Source mutated during verification/second derivation")
        require(all(sha(path) == digest for path, digest in original_hashes.items()), "Original checkout source mutated during verification")
    except (ValueError, KeyError, TypeError, IndexError, OSError, subprocess.TimeoutExpired) as error:
        report["errors"].append(str(error))
    report["status"] = "PASS" if not report["errors"] else "FAIL"
    return report


class GeometryRegressionTests(unittest.TestCase):
    def test_ray_interpolates_a_translated_sloped_triangle(self):
        tri = ((10, 0, 20), (16, .7, 20), (10, 0, 26))
        self.assertAlmostEqual(projected_height(tri, 13, 21), .35)
        self.assertIsNone(projected_height(tri, 17, 27))
        self.assertIsNone(projected_height(((0, 0, 0), (0, 1, 0), (0, 1, 1)), 0, .5))

    def test_raised_deck_is_detected_above_underlying_ground(self):
        triangle = ((-1, 0, -1), (1, 0, -1), (0, 0, 1))
        index = SurfaceIndex([("ground", triangle), ("bad_deck", tuple((x, y + .6, z) for x, y, z in triangle))])
        self.assertEqual(index.sample(0, 0), (.6, "bad_deck"))
        self.assertGreater(abs(index.sample(0, 0)[0] - runtime_ground(0, 0)), TOLERANCE)

    def test_original_crowned_bridge_fails_the_new_flat_ground_contract(self):
        glb = GLB(ART / "exports/verdant-confluence.glb")
        index = SurfaceIndex(iter_triangles(glb, walkable_node))
        # Interior of an actual source paver, away from parapets and seams.
        along, across = 1.0, 1.5
        x, z = (along + across) / math.sqrt(2), (along - across) / math.sqrt(2)
        height, name = index.sample(x, z)
        self.assertTrue(name.startswith("bridge"), name)
        self.assertGreater(height - runtime_ground(x, z), .5)

    def test_original_baked_structure_scene_is_rejected_as_static_runtime(self):
        source = GLB(ART / "exports/verdant-confluence.glb")
        manifest = json.loads((RUNTIME / "manifest.json").read_text())
        with self.assertRaisesRegex(ValueError, "Baked live structure role"):
            static_exclusions({"environment": source, "foliage": source}, source, manifest)

    def test_nonfinite_geometry_and_missing_material_are_rejected(self):
        # Mutate real runtime GLB input in a temporary output, without touching
        # either the staged candidate or the original art package.
        import struct
        data = (RUNTIME / "watchtower_green.glb").read_bytes()
        length = struct.unpack_from("<I", data, 12)[0]
        document = json.loads(data[20:20 + length])
        binary = bytearray(data[28 + length:])
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "bad.glb"
            def write(doc, buffer):
                encoded = json.dumps(doc, separators=(",", ":")).encode()
                encoded += b" " * (-len(encoded) % 4)
                path.write_bytes(struct.pack("<4sII", b"glTF", 2, 28 + len(encoded) + len(buffer))
                                 + struct.pack("<II", len(encoded), 0x4E4F534A) + encoded
                                 + struct.pack("<II", len(buffer), 0x004E4942) + buffer)
            primitive = document["meshes"][0]["primitives"][0]
            del primitive["material"]
            write(document, binary)
            with self.assertRaisesRegex(ValueError, "material"):
                GLB(path)
            primitive["material"] = 0
            accessor = document["accessors"][primitive["attributes"]["POSITION"]]
            view = document["bufferViews"][accessor["bufferView"]]
            offset = view.get("byteOffset", 0) + accessor.get("byteOffset", 0)
            struct.pack_into("<f", binary, offset, float("nan"))
            write(document, binary)
            with self.assertRaisesRegex(ValueError, "Non-finite"):
                GLB(path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime", type=Path, default=RUNTIME)
    parser.add_argument("--raw-dir", type=Path, default=RAW)
    parser.add_argument("--input-inventory", type=Path, default=INPUT_INVENTORY)
    parser.add_argument("--original-art", type=Path, help="Also verify the untouched source checkout against its frozen hashes")
    parser.add_argument("--output", type=Path, default=RAW / "verdant-runtime-validation.json")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(GeometryRegressionTests))
        return 0 if result.wasSuccessful() else 1
    report = validate(args.runtime.resolve(), args.raw_dir.resolve(), args.input_inventory.resolve(),
                      args.original_art.resolve() if args.original_art else None)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    summary = {key: value for key, value in report.items() if key not in ("surfaces", "files")}
    if "surfaces" in report:
        summary["surfaces"] = {key: value for key, value in report["surfaces"].items() if key not in ("samples", "joins")}
    print(json.dumps(summary, indent=2))
    return 0 if report["status"] == "PASS" else 1


if __name__ == "__main__":
    sys.exit(main())
