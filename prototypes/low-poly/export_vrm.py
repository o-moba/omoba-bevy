# SPDX-License-Identifier: MPL-2.0
"""Add the OMOBA prototype's VRM 1.0 humanoid mapping to a skinned Blender GLB."""

from __future__ import annotations

import argparse
import json
import struct
from pathlib import Path


GLB_HEADER = struct.Struct("<4sII")
CHUNK_HEADER = struct.Struct("<I4s")
EXTENSION = "VRMC_vrm"

BONE_PARENTS = {
    "hips": None,
    "spine": "hips",
    "chest": "spine",
    "upperChest": "chest",
    "neck": "upperChest",
    "head": "neck",
}
for side in ("left", "right"):
    BONE_PARENTS.update({
        f"{side}Shoulder": "upperChest",
        f"{side}UpperArm": f"{side}Shoulder",
        f"{side}LowerArm": f"{side}UpperArm",
        f"{side}Hand": f"{side}LowerArm",
        f"{side}UpperLeg": "hips",
        f"{side}LowerLeg": f"{side}UpperLeg",
        f"{side}Foot": f"{side}LowerLeg",
        f"{side}Toes": f"{side}Foot",
    })

OPTIONAL_BONES = {
    "chest", "upperChest", "neck", "leftShoulder", "rightShoulder", "leftToes", "rightToes",
}


def read_glb(path: Path) -> tuple[dict, bytes]:
    data = path.read_bytes()
    if len(data) < GLB_HEADER.size or GLB_HEADER.unpack_from(data) != (b"glTF", 2, len(data)):
        raise ValueError("Expected a glTF 2.0 binary container")

    chunks = []
    offset = GLB_HEADER.size
    for expected in (b"JSON", b"BIN\0"):
        if offset + CHUNK_HEADER.size > len(data):
            raise ValueError(f"GLB is missing its {expected!r} chunk")
        size, kind = CHUNK_HEADER.unpack_from(data, offset)
        start = offset + CHUNK_HEADER.size
        offset = start + size
        if kind != expected or size % 4 != 0 or offset > len(data):
            raise ValueError(f"Invalid GLB {expected!r} chunk")
        chunks.append(data[start:offset])
    if offset != len(data):
        raise ValueError("Expected exactly two GLB chunks")

    document = json.loads(chunks[0].decode("utf-8"))
    if not isinstance(document, dict) or document.get("asset", {}).get("version") != "2.0":
        raise ValueError("Expected a glTF 2.0 JSON document")
    binary = chunks[1]
    buffers = document.get("buffers", [])
    if (len(buffers) != 1 or "uri" in buffers[0]
            or len(binary) - buffers[0].get("byteLength", -1) not in range(4)
            or any("uri" in image for image in document.get("images", []))):
        raise ValueError("Expected one embedded buffer and embedded images")
    return document, binary


def scene_tree(nodes: list[dict], roots: list[int]) -> dict[int, int | None]:
    parents = {}
    pending = [(root, None) for root in roots]
    while pending:
        index, parent = pending.pop()
        if not 0 <= index < len(nodes) or index in parents:
            raise ValueError(f"Invalid node or multiple parents in scene 0 at index {index}")
        parents[index] = parent
        pending.extend((child, index) for child in nodes[index].get("children", []))
    return parents


def humanoid_mapping(document: dict) -> dict[str, dict[str, int]]:
    nodes = document.get("nodes", [])
    scenes = document.get("scenes", [])
    if not scenes or not scenes[0].get("nodes"):
        raise ValueError("Expected a scene 0 with a humanoid rig")
    parents = scene_tree(nodes, scenes[0]["nodes"])

    joints = set()
    skins = document.get("skins", [])
    mesh_count = len(document.get("meshes", []))
    for index in parents:
        node = nodes[index]
        if "skin" in node:
            skin = node["skin"]
            if not 0 <= skin < len(skins) or not 0 <= node.get("mesh", -1) < mesh_count:
                raise ValueError(f"Node {index} must bind a valid mesh and skin")
            joints.update(skins[skin].get("joints", []))

    by_name = {}
    for index in sorted(joints):
        if index not in parents:
            raise ValueError(f"Skin joint {index} is outside scene 0")
        bone = nodes[index].get("name")
        if bone in BONE_PARENTS:
            if bone in by_name:
                raise ValueError(f"Skin joint name {bone!r} is duplicated")
            by_name[bone] = index

    missing = BONE_PARENTS.keys() - by_name.keys() - OPTIONAL_BONES
    if missing:
        raise ValueError(f"Missing required skin joints: {', '.join(sorted(missing))}")
    if "upperChest" in by_name and "chest" not in by_name:
        raise ValueError("upperChest requires chest")

    by_index = {index: bone for bone, index in by_name.items()}
    for bone, index in by_name.items():
        expected = BONE_PARENTS[bone]
        while expected is not None and expected not in by_name:
            expected = BONE_PARENTS[expected]
        ancestor = parents[index]
        while ancestor is not None and ancestor not in by_index:
            ancestor = parents[ancestor]
        actual = by_index.get(ancestor)
        if actual != expected:
            raise ValueError(f"Expected {bone} to descend from {expected!r}, got {actual!r}")
    return {bone: {"node": index} for bone, index in by_name.items()}


def write_vrm(source_glb: str | Path, output_vrm: str | Path, *,
              name: str = "OMOBA Low-Poly Prototype") -> None:
    document, binary = read_glb(Path(source_glb))
    extensions = document.setdefault("extensions", {})
    if "VRM" in extensions or EXTENSION in extensions:
        raise ValueError("The source already contains VRM metadata")
    if not name.strip():
        raise ValueError("Expected a non-empty avatar name")

    extensions[EXTENSION] = {
        "specVersion": "1.0",
        "meta": {
            "name": name,
            "version": "0.1",
            "authors": ["Open Moba contributors"],
            "licenseUrl": "https://vrm.dev/licenses/1.0/",
        },
        "humanoid": {"humanBones": humanoid_mapping(document)},
    }
    used = document.setdefault("extensionsUsed", [])
    if EXTENSION not in used:
        used.append(EXTENSION)

    payload = json.dumps(document, separators=(",", ":"), allow_nan=False).encode("utf-8")
    payload += b" " * (-len(payload) % 4)
    chunks = CHUNK_HEADER.pack(len(payload), b"JSON") + payload
    chunks += CHUNK_HEADER.pack(len(binary), b"BIN\0") + binary
    Path(output_vrm).write_bytes(GLB_HEADER.pack(b"glTF", 2, GLB_HEADER.size + len(chunks)) + chunks)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source_glb", type=Path)
    parser.add_argument("output_vrm", type=Path)
    parser.add_argument("--name", default="OMOBA Low-Poly Prototype")
    args = parser.parse_args()
    try:
        write_vrm(args.source_glb, args.output_vrm, name=args.name)
    except (OSError, ValueError) as exc:
        parser.exit(1, f"{parser.prog}: {exc}\n")


if __name__ == "__main__":
    main()
