#!/usr/bin/env python3
"""Derive shared XZ collision from shipped GLBs without Blender or dependencies.

Run normally to regenerate, or with --check to validate without writing files.
The saved .blend and source art export are deliberately never opened.
"""
import argparse
from collections import Counter, deque
import hashlib
import json
import math
from pathlib import Path

from stage_verdant import accessor_values, read_glb, triangles

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / "client/assets/verdant"
OUTPUT = ROOT / "shared/assets/verdant-collision.json"
EXPECTED = {
    "tree_jade_canopy": 23, "tree_sage_elder": 23, "tree_windswept_oak": 23,
    "tree_cypress_spire": 22, "tree_river_pine": 22,
    "boulder_moss_tall": 64, "boulder_moss_flat": 39,
    "rock_stratified_outcrop": 5, "ruin_wall": 15,
}
STONE = {"VC / slate strata", "VC / cut limestone", "VC / weathered stone",
         "VC / ivory limestone"}
TRUNK_SLAB = (.75, 1.25)
IDENTITY = (1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1)
AGENT_CLEARANCE = .55


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def multiply(a, b):
    return tuple(sum(a[k * 4 + row] * b[col * 4 + k] for k in range(4))
                 for col in range(4) for row in range(4))


def transform(matrix, point):
    return tuple(sum(matrix[k * 4 + row] * point[k] for k in range(3)) + matrix[12 + row]
                 for row in range(3))


def node_matrix(node):
    if "matrix" in node:
        require(not any(key in node for key in ("translation", "rotation", "scale")),
                "GLB node mixes matrix and TRS")
        return tuple(node["matrix"])
    t = node.get("translation", (0, 0, 0))
    x, y, z, w = node.get("rotation", (0, 0, 0, 1))
    s = node.get("scale", (1, 1, 1))
    r = (1-2*y*y-2*z*z, 2*x*y+2*z*w, 2*x*z-2*y*w, 0,
         2*x*y-2*z*w, 1-2*x*x-2*z*z, 2*y*z+2*x*w, 0,
         2*x*z+2*y*w, 2*y*z-2*x*w, 1-2*x*x-2*y*y, 0,
         *t, 1)
    return tuple(r[i] * s[i // 4] if i < 12 else r[i] for i in range(16))


def clip_height(polygon, height, above):
    """Clip triangles, retaining crossings even when no original vertex is in the slab."""
    result = []
    for a, b in zip(polygon, polygon[1:] + polygon[:1]):
        ai = a[1] >= height if above else a[1] <= height
        bi = b[1] >= height if above else b[1] <= height
        if ai:
            result.append(a)
        if ai != bi:
            t = (height - a[1]) / (b[1] - a[1])
            result.append(tuple(a[k] + t * (b[k] - a[k]) for k in range(3)))
    return result


def cross(a, b, c):
    return (b[0]-a[0])*(c[1]-a[1]) - (b[1]-a[1])*(c[0]-a[0])


def hull(points):
    points = sorted(set(points))
    lower, upper = [], []
    for chain, sequence in ((lower, points), (upper, reversed(points))):
        for p in sequence:
            while len(chain) > 1 and cross(chain[-2], chain[-1], p) <= 1e-10:
                chain.pop()
            chain.append(p)
    return lower[:-1] + upper[:-1]


def edges(polygon):
    return zip(polygon, polygon[1:] + polygon[:1])


def point_segment_distance(p, a, b):
    dx, dz = b[0]-a[0], b[1]-a[1]
    squared = dx*dx + dz*dz
    t = max(0., min(1., ((p[0]-a[0])*dx + (p[1]-a[1])*dz)/squared)) if squared else 0.
    return math.hypot(p[0]-a[0]-t*dx, p[1]-a[1]-t*dz)


def point_polygon_distance(p, polygon):
    if all(cross(a, b, p) >= -1e-9 for a, b in edges(polygon)):
        return 0.
    return min(point_segment_distance(p, a, b) for a, b in edges(polygon))


def segment_distance(a, b, c, d):
    distances = (point_segment_distance(a, c, d), point_segment_distance(b, c, d),
                 point_segment_distance(c, a, b), point_segment_distance(d, a, b))
    if min(distances) < 1e-9:
        return 0.
    if cross(a, b, c)*cross(a, b, d) < 0 and cross(c, d, a)*cross(c, d, b) < 0:
        return 0.
    return min(distances)


def polygon_segment_distance(polygon, a, b):
    if point_polygon_distance(a, polygon) == 0 or point_polygon_distance(b, polygon) == 0:
        return 0.
    return min(segment_distance(a, b, c, d) for c, d in edges(polygon))


def extract(gltf, binary, root_index, filename):
    root = gltf["nodes"][root_index]
    asset = root.get("extras", {}).get("asset_id")
    tree = asset.startswith("tree_")
    points = []

    def walk(index, parent):
        node = gltf["nodes"][index]
        world = multiply(parent, node_matrix(node))
        if "mesh" in node:
            for primitive in gltf["meshes"][node["mesh"]]["primitives"]:
                material = gltf["materials"][primitive["material"]]["name"]
                selected_material = material == "VC / warm heartwood" if tree else material in STONE
                if not selected_material:
                    continue
                values = accessor_values(gltf, binary, primitive["attributes"]["POSITION"])
                if tree:
                    for indices in triangles(gltf, binary, primitive):
                        polygon = [values[i] for i in indices]
                        polygon = clip_height(polygon, TRUNK_SLAB[0], True)
                        polygon = clip_height(polygon, TRUNK_SLAB[1], False)
                        points.extend(transform(world, p) for p in polygon)
                else:
                    points.extend(transform(world, p) for p in values)
        for child in node.get("children", []):
            walk(child, world)

    walk(root_index, IDENTITY)
    require(points, f"No reviewed solid geometry for {root['name']}")
    xz = [(p[0], p[2]) for p in points]
    provenance = {"file": filename, "root": root["name"], "asset_id": asset,
                  "role": root.get("extras", {}).get("role")}
    if tree:
        origin = transform(node_matrix(root), (0, 0, 0))
        center = (origin[0], origin[2])
        radius = max(math.dist(center, p) for p in xz)
        # A polygon inscribed in the trunk circle would permit corner clipping.
        # Circumscribe it and retain enough extra distance for six-decimal rounding.
        outer_radius = radius / math.cos(math.pi / 16) + .000003
        vertices = [(center[0] + outer_radius*math.cos(i*math.tau/16),
                     center[1] + outer_radius*math.sin(i*math.tau/16)) for i in range(16)]
        provenance.update(trunk_center_xz=[round(v, 6) for v in center],
                          trunk_radius=round(radius, 6))
    else:
        vertices = hull(xz)
    # Re-hull after quantization to remove collinear/duplicate export coordinates.
    vertices = hull([tuple(round(v, 6) for v in p) for p in vertices])
    return {"id": filename + ":" + root["name"],
            "kind": "tree_trunk" if tree else "wall" if asset == "ruin_wall" else "rock",
            "vertices": vertices, "source": provenance}


def validate(document, layout):
    obstacles = document["obstacles"]
    require(len(obstacles) == sum(EXPECTED.values()), "Reviewed obstacle count changed")
    require(len({o["id"] for o in obstacles}) == len(obstacles), "Duplicate obstacle ids")
    counts = Counter(o["source"]["asset_id"] for o in obstacles)
    require(counts == Counter(EXPECTED), f"Reviewed root selection changed: {dict(counts)}")
    lo, hi = document["bounds"]["min"], document["bounds"]["max"]
    for obstacle in obstacles:
        polygon = obstacle["vertices"]
        require(len(polygon) >= 3 and len(set(polygon)) == len(polygon), "Invalid polygon vertices")
        require(all(math.isfinite(v) for p in polygon for v in p), "Non-finite polygon")
        require(all(lo[k] <= p[k] <= hi[k] for p in polygon for k in range(2)),
                f"Obstacle outside shared bounds: {obstacle['id']}")
        require(all(cross(polygon[i-2], polygon[i-1], polygon[i]) > 1e-10
                    for i in range(len(polygon))), f"Polygon is not strictly convex CCW: {obstacle['id']}")
        if obstacle["kind"] == "tree_trunk":
            source = obstacle["source"]
            center, radius = source["trunk_center_xz"], source["trunk_radius"]
            require(min(point_segment_distance(center, a, b) for a, b in edges(polygon))
                    >= radius - .000001, "Tree polygon does not contain its trunk circle")
    convert = lambda p: (p[0], -p[1])
    lanes = [[convert(p) for p in lane] for lane in layout["lanes_blender"]]
    lane_segments = [(a, b) for lane in lanes for a, b in zip(lane, lane[1:])]
    minimum_lane = min((polygon_segment_distance(o["vertices"], a, b), o["id"])
                       for o in obstacles for a, b in lane_segments)
    require(minimum_lane[0] >= layout["lane_width"]/2 + AGENT_CLEARANCE,
            f"Full lane corridor obstructed: {minimum_lane}")
    bases = [convert(p) for p in layout["base_centers_blender"]]
    spawns = [(x-math.copysign(7/math.sqrt(2), x), z-math.copysign(7/math.sqrt(2), z))
              for x, z in bases]
    minimum_spawn = min(point_polygon_distance(p, o["vertices"]) for p in spawns for o in obstacles)
    require(minimum_spawn > 2 + AGENT_CLEARANCE, "Spawn area obstructed")
    objectives = [convert(p) for key in ("camps_blender", "bosses_blender", "towers_blender")
                  for p in layout[key]]
    minimum_objective = min(point_polygon_distance(p, o["vertices"])
                            for p in objectives for o in obstacles)
    require(minimum_objective > 3 + AGENT_CLEARANCE, "Objective approach area obstructed")
    access = connected_access(obstacles, lo, hi, spawns + objectives)
    return {"obstacle_count": len(obstacles), "selected_roots_by_asset": dict(sorted(counts.items())),
            "minimum_lane_boundary_distance": round(minimum_lane[0], 6),
            "limiting_lane_obstacle": minimum_lane[1],
            "validated_lane_half_width": layout["lane_width"]/2,
            "agent_clearance": AGENT_CLEARANCE,
            "minimum_spawn_boundary_distance": round(minimum_spawn, 6),
            "minimum_objective_boundary_distance": round(minimum_objective, 6),
            "static_access": access}


def connected_access(obstacles, lo, hi, endpoints):
    """Conservative 1m cells prove continuous shared access for all reviewed anchors.

    Inflating by the cell half-diagonal certifies the entire traversed cell, so
    four-neighbor flood fill cannot jump through thin walls or diagonal corners.
    Live structures are deliberately outside this static access proof.
    """
    width = math.floor(hi[0]-lo[0])
    height = math.floor(hi[1]-lo[1])
    inflation = AGENT_CLEARANCE + math.sqrt(.5)
    blocked = set()
    for obstacle in obstacles:
        polygon = obstacle["vertices"]
        x0 = max(0, math.floor(min(p[0] for p in polygon)-inflation-lo[0]-.5))
        x1 = min(width-1, math.ceil(max(p[0] for p in polygon)+inflation-lo[0]-.5))
        z0 = max(0, math.floor(min(p[1] for p in polygon)-inflation-lo[1]-.5))
        z1 = min(height-1, math.ceil(max(p[1] for p in polygon)+inflation-lo[1]-.5))
        for z in range(z0, z1+1):
            for x in range(x0, x1+1):
                if point_polygon_distance((lo[0]+x+.5, lo[1]+z+.5), polygon) <= inflation:
                    blocked.add((x, z))
    for z in range(height):
        for x in (0, width-1):
            blocked.add((x, z))
    for x in range(width):
        for z in (0, height-1):
            blocked.add((x, z))
    cells = [(math.floor(p[0]-lo[0]), math.floor(p[1]-lo[1])) for p in endpoints]
    require(all(c not in blocked for c in cells), "Spawn/objective occupies uncertified cell")
    visited, queue = {cells[0]}, deque([cells[0]])
    while queue:
        x, z = queue.popleft()
        for q in ((x-1, z), (x+1, z), (x, z-1), (x, z+1)):
            if 0 <= q[0] < width and 0 <= q[1] < height and q not in blocked and q not in visited:
                visited.add(q)
                queue.append(q)
    require(all(c in visited for c in cells), "Static collision isolates a spawn/objective")
    return {"grid_cell_meters": 1, "cell_inflation": round(inflation, 6),
            "connected_anchors": len(cells), "reachable_certified_cells": len(visited),
            "scope": "Both hero spawns, three camps, two bosses and six tower anchors; static geometry only"}


def generate():
    manifest_path = ASSETS / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    inventory = {row["path"]: row for row in manifest["files"]}
    sources, obstacles = [], []
    for filename in ("environment.glb", "foliage.glb"):
        path = ASSETS / filename
        sha256 = digest(path)
        require(sha256 == inventory[filename]["sha256"], f"Shipped source hash mismatch: {filename}")
        gltf, binary = read_glb(path)
        rooted_assets = {gltf["nodes"][i].get("extras", {}).get("asset_id", "")
                         for i in gltf["scenes"][gltf.get("scene", 0)]["nodes"]}
        require(not {asset for asset in rooted_assets if asset.startswith("tree_") and asset not in EXPECTED},
                "Unreviewed shipped tree asset; extend the explicit collision selection")
        selected = [i for i in gltf["scenes"][gltf.get("scene", 0)]["nodes"]
                    if gltf["nodes"][i].get("extras", {}).get("asset_id") in EXPECTED]
        obstacles.extend(extract(gltf, binary, i, filename) for i in selected)
        sources.append({"path": "client/assets/verdant/" + filename, "sha256": sha256,
                        "selected_roots": len(selected)})
    half = manifest["layout"]["half_extent"]
    document = {
        "format_version": 1, "bounds": {"min": [-half, -half], "max": [half, half]},
        "obstacles": sorted(obstacles, key=lambda obstacle: obstacle["id"]),
        "provenance": {"generator": "scripts/generate_verdant_collision.py", "sources": sources,
                       "runtime_manifest_sha256": digest(manifest_path)},
        "rules": {"coordinates": "physical XZ = fully composed shipped glTF world (x,z), meters; no axis conversion",
                  "tree_material": "VC / warm heartwood", "tree_local_y_slab": list(TRUNK_SLAB),
                  "tree_shape": "16-gon circumscribed around root-centered enclosing circle of clipped bark slab; 0.000003m quantization guard",
                  "stone_materials": sorted(STONE),
                  "stone_shape": "Convex XZ hull of selected stone geometry, including embedded stone; no canopy/moss decoration",
                  "excluded": "Canopy, grass, flowers, reeds, logs/stumps, arches, lanterns, banners, bridge meshes, terrain, roads, base surfaces and live structures",
                  "rounding_decimals": 6,
                  "clearance": "Geometry is uninflated; client/server navigation applies actor clearance once"},
    }
    document["validation"] = validate(document, manifest["layout"])
    return document


def encoded(document):
    return json.dumps(document, indent=2, ensure_ascii=True, allow_nan=False) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="validate sources, geometry and committed artifact without writing")
    args = parser.parse_args()
    document = generate()
    content = encoded(document)
    if args.check:
        require(OUTPUT.is_file() and OUTPUT.read_text() == content,
                "Collision artifact differs; regenerate with python3 scripts/generate_verdant_collision.py")
    else:
        OUTPUT.parent.mkdir(parents=True, exist_ok=True)
        OUTPUT.write_text(content)
    print(json.dumps({"status": "PASS", "mode": "check" if args.check else "generate",
                      **document["validation"]}, sort_keys=True))


if __name__ == "__main__":
    main()
