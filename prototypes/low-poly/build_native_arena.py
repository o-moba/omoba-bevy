# SPDX-License-Identifier: MPL-2.0
"""Split the Blender ARAM export into terrain and live Bevy structure scenes."""

import json
from pathlib import Path

from export_vrm import CHUNK_HEADER, GLB_HEADER, read_glb


PROTOTYPE = Path(__file__).resolve().parent
RUNTIME = PROTOTYPE.parents[1] / "client/src/verdant3d/aram.glb"
BOARD_SCALE = 2.5
FOUNDATION_TOP = 0.27
# GLB scene order is part of the native runtime's asset contract.
STRUCTURES = [
    ("Blue Tower", "Green Tower", -11, 0.90, 1.0),
    ("Orange Tower", "Blue Tower", 11, 0.90, 1.0),
    ("Blue Nexus", "Green Nexus", -22, 1.85, 1.35),
    ("Orange Nexus", "Blue Nexus", 22, 1.85, 1.35),
]
TEAM_MATERIALS = {
    "Team Cloth": ("Green enamel", "358969", 0.14),
    "Team Trim": ("Green enamel highlight", "73C7A8", 0.14),
    "Blue tower crystal color": ("Green crystal", "42C594", 0.45),
    "Blue tower crystal light": ("Green crystal highlight", "9AF6D4", 0.60),
    "Blue tower crystal shade": ("Green crystal shade", "257757", 0.12),
    "Orange Cloth": ("Blue enamel", "4167DF", 0.14),
    "Orange Trim": ("Blue enamel highlight", "718EF1", 0.14),
    "Orange tower crystal color": ("Blue crystal", "4274EF", 0.45),
    "Orange tower crystal light": ("Blue crystal highlight", "8BDCFF", 0.60),
    "Orange tower crystal shade": ("Blue crystal shade", "2543B2", 0.12),
}


def linear_rgb(hex_color):
    srgb = [int(hex_color[i:i + 2], 16) / 255 for i in (0, 2, 4)]
    return [c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4 for c in srgb]


def put_group(nodes, name, children, scale, translation):
    index = len(nodes)
    nodes.append({"name": name, "children": children,
                  "translation": translation, "scale": scale})
    return index


def build():
    source, binary = read_glb(PROTOTYPE / "aram-arena.glb")
    nodes = source["nodes"]
    roots = source["scenes"][source.get("scene", 0)]["nodes"]
    if len(roots) != len(nodes) or any("children" in nodes[i] for i in roots):
        raise ValueError("Export each ARAM mesh as an individual root node")

    live = set()
    scenes = []
    # The browser file uses baked world positions. Pivot each live group at its
    # own foundation and scale its XZ footprint to the native collision radius.
    for prefix, name, x, width, height in STRUCTURES:
        parts = [i for i in roots if nodes[i]["name"].startswith(prefix + " ")]
        if not parts:
            raise ValueError(f"Missing live structure: {prefix}")
        live.update(parts)
        for i in parts:
            nodes[i]["name"] = name + nodes[i]["name"][len(prefix):]
        root = put_group(nodes, name, parts, [width, height, width],
                         [-x * width, -FOUNDATION_TOP * height, 0])
        scenes.append({"name": name, "nodes": [root]})

    terrain = [i for i in roots if i not in live]
    for prefix, name, x, width, _ in STRUCTURES:
        color, kind = prefix.split(" ")
        footprint = f"{kind} stone footprint • {color.lower()}"
        (part,) = [i for i in terrain if nodes[i]["name"] == footprint]
        fit = width / BOARD_SCALE
        terrain.remove(part)
        terrain.append(put_group(nodes, f"{name} platform", [part],
                                 [fit, 1, fit], [x * (1 - fit), 0, 0]))
    root = put_group(nodes, "ARAM battlefield", terrain,
                     [BOARD_SCALE, 1, BOARD_SCALE], [0, 0, 0])
    source["scenes"] = [{"name": "ARAM battlefield", "nodes": [root]}, *scenes]
    source["scene"] = 0

    missing = set(TEAM_MATERIALS)
    for material in source["materials"]:
        key = material["name"]
        if key not in TEAM_MATERIALS:
            continue
        missing.remove(key)
        name, hex_color, glow = TEAM_MATERIALS[key]
        rgb = linear_rgb(hex_color)
        material["name"] = name
        material["pbrMetallicRoughness"]["baseColorFactor"] = [*rgb, 1]
        material["emissiveFactor"] = [component * glow for component in rgb]
    if missing:
        raise ValueError(f"Missing team materials: {', '.join(sorted(missing))}")

    payload = json.dumps(source, separators=(",", ":"), allow_nan=False).encode()
    payload += b" " * (-len(payload) % 4)
    chunks = (CHUNK_HEADER.pack(len(payload), b"JSON") + payload
              + CHUNK_HEADER.pack(len(binary), b"BIN\0") + binary)
    RUNTIME.parent.mkdir(parents=True, exist_ok=True)
    RUNTIME.write_bytes(GLB_HEADER.pack(b"glTF", 2, GLB_HEADER.size + len(chunks)) + chunks)


if __name__ == "__main__":
    build()
