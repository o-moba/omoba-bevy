#!/usr/bin/env python3
"""Deterministically derive the playable Verdant scene from the saved GLB export.

No Blender process or third-party module is needed for this derivation. The
source scene stays intact; only explicitly documented walking surfaces change.
"""
import argparse
import copy
import hashlib
import json
import math
from pathlib import Path
import struct

ROOT = Path(__file__).resolve().parents[1]
ART = ROOT / "art/verdant-confluence"
OUTPUT = ROOT / "client/assets/verdant"
STRUCTURES = ("watchtower_green", "watchtower_blue", "sanctuary_green", "sanctuary_blue")
FOLIAGE_ROLES = {"forest_tree", "understory", "forest_rock", "groundcover", "riverbank",
                 "riverbank_rock", "riverbank_shelf", "riverbank_detail", "forest_story", "base_garden"}


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def read_glb(path):
    raw = Path(path).read_bytes()
    if struct.unpack_from("<4sII", raw) != (b"glTF", 2, len(raw)):
        raise ValueError(f"invalid GLB header: {path}")
    length, kind = struct.unpack_from("<II", raw, 12)
    if kind != 0x4E4F534A:
        raise ValueError("GLB must begin with JSON")
    gltf = json.loads(raw[20:20 + length])
    offset = 20 + length
    binary_length, binary_kind = struct.unpack_from("<II", raw, offset)
    if binary_kind != 0x004E4942:
        raise ValueError("embedded geometry buffer required")
    return gltf, bytearray(raw[offset + 8:offset + 8 + binary_length])


def write_glb(path, gltf, binary):
    data = json.dumps(gltf, separators=(",", ":"), ensure_ascii=True).encode()
    data += b" " * (-len(data) % 4)
    binary = bytes(binary) + b"\0" * (-len(binary) % 4)
    raw = (struct.pack("<4sII", b"glTF", 2, 28 + len(data) + len(binary))
           + struct.pack("<II", len(data), 0x4E4F534A) + data
           + struct.pack("<II", len(binary), 0x004E4942) + binary)
    Path(path).write_bytes(raw)


def accessor_values(gltf, binary, index):
    acc = gltf["accessors"][index]
    view = gltf["bufferViews"][acc["bufferView"]]
    components = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}[acc["type"]]
    fmt = "<" + {5121: "B", 5123: "H", 5125: "I", 5126: "f"}[acc["componentType"]] * components
    stride = view.get("byteStride", struct.calcsize(fmt))
    offset = view.get("byteOffset", 0) + acc.get("byteOffset", 0)
    return [struct.unpack_from(fmt, binary, offset + i * stride) for i in range(acc["count"])]


def put_vec3(gltf, binary, index, values):
    acc = gltf["accessors"][index]
    assert acc["componentType"] == 5126 and acc["type"] == "VEC3"
    view = gltf["bufferViews"][acc["bufferView"]]
    offset = view.get("byteOffset", 0) + acc.get("byteOffset", 0)
    stride = view.get("byteStride", 12)
    for i, point in enumerate(values):
        struct.pack_into("<3f", binary, offset + i * stride, *point)
    if "min" in acc or "max" in acc:
        acc["min"] = [min(p[i] for p in values) for i in range(3)]
        acc["max"] = [max(p[i] for p in values) for i in range(3)]


def triangles(gltf, binary, primitive):
    indices = [v[0] for v in accessor_values(gltf, binary, primitive["indices"])]
    assert primitive.get("mode", 4) == 4
    return [indices[i:i + 3] for i in range(0, len(indices), 3)]


def recalculate_normals(gltf, binary, primitive, points):
    normal = primitive["attributes"].get("NORMAL")
    if normal is None:
        return
    sums = [[0., 0., 0.] for _ in points]
    for a, b, c in triangles(gltf, binary, primitive):
        u = [points[b][i] - points[a][i] for i in range(3)]
        v = [points[c][i] - points[a][i] for i in range(3)]
        n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]]
        for vertex in (a, b, c):
            for i in range(3):
                sums[vertex][i] += n[i]
    old = accessor_values(gltf, binary, normal)
    values = []
    for i, vector in enumerate(sums):
        length = math.sqrt(sum(v * v for v in vector))
        values.append(tuple(v / length for v in vector) if length > 1e-12 else old[i])
    put_vec3(gltf, binary, normal, values)


def flatten_bridge(gltf, binary, mesh_index):
    """Flatten each original paver rigidly; de-crown fascia/rails continuously."""
    changed = set()
    for primitive in gltf["meshes"][mesh_index]["primitives"]:
        index = primitive["attributes"]["POSITION"]
        if index in changed:
            continue
        changed.add(index)
        original = accessor_values(gltf, binary, index)
        # Exported disconnected faces can duplicate their boundary vertices.
        # Geometrically identical vertices are welded for component discovery only.
        parent = list(range(len(original)))
        def find(i):
            while parent[i] != i:
                parent[i] = parent[parent[i]]
                i = parent[i]
            return i
        def union(a, b):
            parent[find(a)] = find(b)
        coordinates = {}
        for i, point in enumerate(original):
            key = tuple(round(v, 5) for v in point)
            if key in coordinates:
                union(i, coordinates[key])
            else:
                coordinates[key] = i
        for a, b, c in triangles(gltf, binary, primitive):
            union(a, b)
            union(b, c)
        groups = {}
        for i in range(len(original)):
            groups.setdefault(find(i), []).append(i)
        output = list(original)
        for members in groups.values():
            low = [min(original[j][i] for j in members) for i in range(3)]
            high = [max(original[j][i] for j in members) for i in range(3)]
            size = [high[i] - low[i] for i in range(3)]
            paver = abs(size[0] - 1.98) < .01 and abs(size[1] - .34) < .01 and abs(size[2] - 2.98) < .01
            fascia = size[0] > 27 and size[2] < 1
            for j in members:
                x, y, z = original[j]
                crown = .21 + .52 * math.cos(max(-14., min(14., x)) / 14 * math.pi / 2)
                # Authored post crowns at +/-5.9 intruded into the claimed
                # 12m route. Keep every paver unchanged in XZ, shift only the
                # parapets/abutments out 0.8m, and widen the existing fascia
                # outward from its inner bearing edge to support those posts.
                if not paver:
                    z = math.copysign(5.62 + (abs(z) - 5.62) * (1.36 / .56), z) if fascia else z + math.copysign(.8, z)
                output[j] = (x, y - (high[1] - .02 if paver else crown - .02), z)
        put_vec3(gltf, binary, index, output)
        recalculate_normals(gltf, binary, primitive, output)


def square_pad_walktop(gltf, binary, mesh_index):
    """Join the full 46m top to its miter skirt, retaining the lower bevel.

    The authored upper corner bevel cut 0.18m into the walking rectangle,
    leaving a dip/hole before the skirt begins. Snap only its upper bevel ring
    to the established square datum; no pad anchor, height or reach changes.
    """
    seen = set()
    for primitive in gltf["meshes"][mesh_index]["primitives"]:
        index = primitive["attributes"]["POSITION"]
        if index in seen:
            continue
        seen.add(index)
        points = []
        for x, y, z in accessor_values(gltf, binary, index):
            if y >= .1699:
                x = math.copysign(23., x) if abs(x) > 22.8 else x
                z = math.copysign(23., z) if abs(z) > 22.8 else z
                y = .35
            points.append((x, y, z))
        put_vec3(gltf, binary, index, points)
        recalculate_normals(gltf, binary, primitive, points)


def flatten_sanctuary(gltf, binary):
    seen = set()
    for mesh in gltf["meshes"]:
        for primitive in mesh["primitives"]:
            index = primitive["attributes"]["POSITION"]
            if index in seen:
                continue
            seen.add(index)
            points = [(x, y * (.03 / 1.1) if y <= 1.1 else y - 1.07, z)
                      for x, y, z in accessor_values(gltf, binary, index)]
            put_vec3(gltf, binary, index, points)
            recalculate_normals(gltf, binary, primitive, points)


def subset(gltf, roots):
    """Prune unreachable nodes/meshes. Buffer/accessor data stay byte-stable."""
    keep = set()
    def visit(index):
        if index in keep:
            return
        keep.add(index)
        for child in gltf["nodes"][index].get("children", []):
            visit(child)
    for root in roots:
        visit(root)
    nodes = sorted(keep)
    node_map = {old: new for new, old in enumerate(nodes)}
    meshes = sorted({gltf["nodes"][i]["mesh"] for i in nodes if "mesh" in gltf["nodes"][i]})
    mesh_map = {old: new for new, old in enumerate(meshes)}
    result = copy.deepcopy(gltf)
    result["nodes"] = [copy.deepcopy(gltf["nodes"][i]) for i in nodes]
    result["meshes"] = [copy.deepcopy(gltf["meshes"][i]) for i in meshes]
    for node in result["nodes"]:
        if "children" in node:
            node["children"] = [node_map[i] for i in node["children"]]
        if "mesh" in node:
            node["mesh"] = mesh_map[node["mesh"]]
    result["scenes"] = [{"name": "Verdant Confluence runtime", "nodes": [node_map[i] for i in roots]}]
    result["scene"] = 0
    return result


def derive(output):
    output.mkdir(parents=True, exist_ok=True)
    source = ART / "exports/verdant-confluence.glb"
    gltf, binary = read_glb(source)
    static, foliage, excluded, adjustments = [], [], [], []
    for index in gltf["scenes"][0]["nodes"]:
        node = gltf["nodes"][index]
        name = node.get("name", "")
        role = node.get("extras", {}).get("role")
        if role in {"base_landmark", "lane_tower"}:
            excluded.append(name)
            continue
        dy = 0.
        if name.startswith("Landscape / meadow"):
            dy = .015
        elif name.startswith("River /"):
            dy = .585
        elif name.startswith("Route"):
            dy = -.10
        elif name.startswith("Objective / tower"):
            dy = -.14
        elif name.startswith("River crossing /"):
            dy = -.23
        elif node.get("extras", {}).get("asset_id") == "lantern" and role == "river_crossing":
            dy = -.23
        if dy:
            position = node.setdefault("translation", [0., 0., 0.])
            position[1] += dy
            adjustments.append({"node": name, "translation_y_delta": dy})
        if "46m pad" in name:
            square_pad_walktop(gltf, binary, node["mesh"])
        if node.get("extras", {}).get("asset_id") == "bridge":
            for child in node["children"]:
                flatten_bridge(gltf, binary, gltf["nodes"][child]["mesh"])
        (foliage if role in FOLIAGE_ROLES else static).append(index)
    assert len(excluded) == 8, excluded
    inventory = []
    def save(name, document, buffer, provenance_source):
        document["asset"]["extras"] = {"provenance": "Original project-authored geometry",
                                       "source": provenance_source, "derivation": "scripts/stage_verdant.py"}
        path = output / (name + ".glb")
        write_glb(path, document, buffer)
        primitive_count = sum(len(m["primitives"]) for m in document["meshes"])
        inventory.append(dict(id=name, path=path.name, sha256=sha(path), bytes=path.stat().st_size,
                              nodes=len(document["nodes"]), meshes=len(document["meshes"]), primitives=primitive_count,
                              materials=len(document.get("materials", [])), source=provenance_source))
    save("environment", subset(gltf, static), binary, "art/verdant-confluence/exports/verdant-confluence.glb")
    save("foliage", subset(gltf, foliage), binary, "art/verdant-confluence/exports/verdant-confluence.glb")
    for name in STRUCTURES:
        document, buffer = read_glb(ART / "library" / (name + ".glb"))
        if name.startswith("sanctuary"):
            flatten_sanctuary(document, buffer)
        save(name, document, buffer, "art/verdant-confluence/library/" + name + ".glb")
    manifest = dict(schema_version=1, title="Verdant Confluence runtime", seed=260905,
                    units="meters; exported glTF Y-up; no additional axis conversion",
                    provenance="Original project-authored geometry; see art/verdant-confluence/PROVENANCE.md",
                    source_sha256=sha(source), source_blend_sha256=sha(ART / "verdant-confluence.blend"),
                    files=inventory, excluded_live_structure_roots=excluded,
                    node_translation_adjustments=adjustments,
                    surfaces=dict(open_ground=0, meadow_top=-.02, road_top=0, water_top=-.015, bridge_paver_top=.02,
                                  outer_crossing_top=.02, base_pad_top=.7, pad_half_extent=23, ramp_reach=29,
                                  pad_corner_rule="0.7 * clamp((29 - max(abs(dx), abs(dz))) / 6, 0, 1)",
                                  sanctuary_floor_max=.03, height_tolerance=.05),
                    adaptations=["Flatten bridge pavers rigidly to +0.02 and de-crown fascia/rails; preserve 28x12 meter walking span.",
                                 "Move bridge parapets and abutments 0.8m outward and widen fascia bearings to clear the entire 12m deck.",
                                 "Square the base pad upper bevel ring at the existing 46m by 0.7m walktop so it meets the miter skirt without corner dips.",
                                 "Compress sanctuary bottom 1.1m to 0.03m; translate upper architecture down 1.07m to expose spawn/walk floor.",
                                 "Runtime sanctuaries rotate 45 degrees around Y to clear original diagonal spawn points.",
                                 "Two scenes split static architecture from F4-controlled foliage; eight live structures are separate assets."],
                    layout=json.loads((ART / "manifest.json").read_text())["layout"])
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=OUTPUT)
    args = parser.parse_args()
    manifest = derive(args.output)
    print(json.dumps({"files": len(manifest["files"]), "excluded_structures": len(manifest["excluded_live_structure_roots"]),
                      "bytes": sum(f["bytes"] for f in manifest["files"])}))


if __name__ == "__main__":
    main()
