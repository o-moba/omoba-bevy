#!/usr/bin/env python3
"""Export one engine-owned semantic motion library without changing avatar bytes.

The checked-in CC0 Quaternius UAL is sampled against A_TPose. Quaternion
channels are world-space deltas, in glTF [x, y, z, w] order; hips channels are
world-space position deltas in source metres. Target runtime binding supplies
rest transforms, bone indices, facing alignment and proportion scaling.

    python3 scripts/export_humanoid_motion.py
    python3 scripts/export_humanoid_motion.py --check --audit /tmp/motion-audit.json

Only Python's standard library is required. The older retarget module supplies
source parsing and quaternion math; its GLB-writing functions are never called.
"""

import argparse
import hashlib
import json
import math
from pathlib import Path

import retarget_animations as legacy

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / legacy.SOURCE_GLTF
OUTPUT = ROOT / "client/assets/animations/humanoid-motion-v1.json"
CLIPS = (
    ("idle", "Idle_Loop", True),
    ("walk", "Walk_Loop", True),
    ("run", "Sprint_Loop", True),
    ("attack", "Sword_Attack", False),
    ("cast", "Spell_Simple_Shoot", False),
    ("death", "Death01", False),
)
REQUIRED = (
    "hips", "spine", "head", "leftUpperArm", "leftLowerArm", "leftHand",
    "rightUpperArm", "rightLowerArm", "rightHand", "leftUpperLeg",
    "leftLowerLeg", "leftFoot", "rightUpperLeg", "rightLowerLeg", "rightFoot",
)
LEGS = (
    "leftUpperLeg", "leftLowerLeg", "leftFoot",
    "rightUpperLeg", "rightLowerLeg", "rightFoot",
)


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def rounded(value):
    """Seven decimals retain sub-micrometre positions without float64 noise."""
    if isinstance(value, (list, tuple)):
        return [rounded(item) for item in value]
    number = round(value, 7)
    return 0.0 if number == 0 else number


def angular_distance(a, b):
    a, b = legacy.qnorm(a), legacy.qnorm(b)
    dot = abs(sum(x * y for x, y in zip(a, b)))
    return 2.0 * math.acos(min(1.0, dot))


def reference_facing(left_hand, right_hand):
    # (left - right) cross world up, with numerical vertical noise discarded.
    across = [a - b for a, b in zip(left_hand, right_hand)]
    facing = [-across[2], 0.0, across[0]]
    length = math.sqrt(sum(v * v for v in facing))
    if length < 1e-5:
        raise ValueError("reference hands do not define a horizontal facing")
    return tuple(v / length for v in facing)


def semantic_source_bones(source):
    names = dict(legacy.BONE_MAP)
    names.update({
        "DEF-spine.001": "spine",
        "DEF-spine.002": "chest",
        "DEF-spine.003": "upperChest",
    })
    return dict(sorted((semantic, source.name_to_node[rig]) for rig, semantic in names.items()))


def export_library(source_path=SOURCE):
    source_path = Path(source_path)
    source = legacy.SourceRig(legacy.Gltf.from_gltf_file(source_path))
    for index, node in enumerate(source.doc.nodes):
        if any(abs(v - 1.0) > 1e-5 for v in node.get("scale", [1, 1, 1])):
            raise ValueError(f"source node {index} has unsupported non-unit scale")
    for animation in source.anims.values():
        for sampler in animation.get("samplers", []):
            if sampler.get("interpolation", "LINEAR") not in ("LINEAR", "STEP"):
                raise ValueError("source requires exact LINEAR/STEP sampling; rebake splines first")
    bones = semantic_source_bones(source)
    facing = reference_facing(
        source.ref_world[source.left_hand][0],
        source.ref_world[source.name_to_node["DEF-hand.R"]][0],
    )
    buffer_path = source_path.parent / source.doc.js["buffers"][0]["uri"]
    library = {
        "schema_version": 1,
        "source": {
            "author": "Quaternius",
            "license": "CC0-1.0",
            "reference_clip": "A_TPose",
            "gltf": source_path.name,
            "gltf_sha256": sha256(source_path),
            "buffer": buffer_path.name,
            "buffer_sha256": sha256(buffer_path),
            "coordinate_system": "glTF right-handed Y-up; quaternions xyzw",
            "loop_processing": "Remove endpoint hips drift and close final pose to first pose",
        },
        "source_hips_height": rounded(source.hips_height),
        "source_hips_to_feet_distance": rounded(source.hips_height - sum(
            source.ref_world[source.name_to_node[name]][0][1]
            for name in ("DEF-foot.L", "DEF-foot.R")
        ) / 2),
        "source_left_hand_x": rounded(source.left_hand_x),
        "source_reference_facing": rounded(facing),
        "bones": list(bones),
        "clips": {},
    }
    for name, source_name, looping in CLIPS:
        clip = source.clip(source_name)
        if not clip.timeline or clip.timeline[0] != 0:
            raise ValueError(f"{source_name}: timeline must start at zero")
        duration = clip.timeline[-1]
        rotations = {semantic: [] for semantic in bones}
        hips = []
        for time in clip.timeline:
            world = source.doc.world_pose(clip.pose_at(time, source.rest_pose))
            for semantic, index in bones.items():
                delta = legacy.qnorm(legacy.qmul(
                    world[index][1], legacy.qconj(source.ref_world[index][1])
                ))
                stream = rotations[semantic]
                if stream and sum(a * b for a, b in zip(stream[-1], delta)) < 0:
                    delta = tuple(-v for v in delta)
                stream.append(delta)
            hips.append(tuple(
                a - b for a, b in zip(world[source.hips][0], source.ref_world[source.hips][0])
            ))
        if looping:
            # UAL has tiny export seams (Sprint max 0.014 rad). Correct that
            # seam at its final key, retaining all actual intermediate poses.
            drift = tuple(b - a for a, b in zip(hips[0], hips[-1]))
            hips = [tuple(v - d * time / duration for v, d in zip(pos, drift))
                    for time, pos in zip(clip.timeline, hips)]
            hips[-1] = hips[0]
            for stream in rotations.values():
                first = stream[0]
                if sum(a * b for a, b in zip(stream[-2], first)) < 0:
                    first = tuple(-v for v in first)
                stream[-1] = first
        library["clips"][name] = {
            "source_clip": source_name,
            "duration": rounded(duration),
            "looping": looping,
            "times": rounded(clip.timeline),
            "world_rotation_deltas": {key: rounded(value) for key, value in rotations.items()},
            "hips_world_deltas": rounded(hips),
        }
    validate_library(library)
    return library


def validate_library(library):
    """Reject malformed or drifting motion before it becomes a runtime asset."""
    if library.get("schema_version") != 1:
        raise ValueError("unsupported motion schema")
    height = library["source_hips_height"]
    if not math.isfinite(height) or height <= 0:
        raise ValueError("source hips height must be positive and finite")
    span = library["source_hips_to_feet_distance"]
    if not math.isfinite(span) or span <= 0:
        raise ValueError("source hips-to-feet distance must be positive and finite")
    facing = library["source_reference_facing"]
    if len(facing) != 3 or not all(math.isfinite(v) for v in facing):
        raise ValueError("reference facing must have three finite components")
    if abs(sum(v * v for v in facing) - 1) > 1e-5:
        raise ValueError("reference facing must be unit length")
    bones = library["bones"]
    if len(bones) != len(set(bones)) or not set(REQUIRED) <= set(bones):
        raise ValueError("duplicate or missing required semantic bones")
    if set(library["clips"]) != {name for name, _, _ in CLIPS}:
        raise ValueError("motion library must contain all six states")
    for name, clip in library["clips"].items():
        times = clip["times"]
        if len(times) < 2 or times[0] != 0 or times[-1] != clip["duration"]:
            raise ValueError(f"{name}: invalid duration/timeline endpoints")
        if not all(math.isfinite(t) for t in times) or any(a >= b for a, b in zip(times, times[1:])):
            raise ValueError(f"{name}: timeline must be finite and strictly increasing")
        streams = clip["world_rotation_deltas"]
        if set(streams) != set(bones):
            raise ValueError(f"{name}: incomplete semantic bone channels")
        for bone, stream in streams.items():
            if len(stream) != len(times):
                raise ValueError(f"{name}/{bone}: rotation sample count mismatch")
            for quat in stream:
                if len(quat) != 4 or not all(math.isfinite(v) for v in quat):
                    raise ValueError(f"{name}/{bone}: invalid quaternion")
                if abs(sum(v * v for v in quat) - 1) > 1e-5:
                    raise ValueError(f"{name}/{bone}: quaternion is not normalized")
            if any(sum(x * y for x, y in zip(a, b)) < -1e-6 for a, b in zip(stream, stream[1:])):
                raise ValueError(f"{name}/{bone}: discontinuous quaternion sign")
            if clip["looping"] and angular_distance(stream[0], stream[-1]) > 1e-6:
                raise ValueError(f"{name}/{bone}: loop endpoint discontinuity")
        hips = clip["hips_world_deltas"]
        if len(hips) != len(times) or any(
            len(v) != 3 or not all(math.isfinite(c) for c in v) for v in hips
        ):
            raise ValueError(f"{name}: invalid hips samples")
        if clip["looping"] and hips[0] != hips[-1]:
            raise ValueError(f"{name}: loop has root displacement")


def encode_library(library):
    return (json.dumps(library, separators=(",", ":"), allow_nan=False) + "\n").encode()


def target_channels(clip, humanoid):
    """A shorter torso still receives the top source spine's accumulated delta."""
    return {
        node: clip["world_rotation_deltas"][
            "upperChest" if bone == "chest" and "upperChest" not in humanoid else bone
        ]
        for bone, node in humanoid.items()
        if bone in clip["world_rotation_deltas"]
    }


def retarget_in_memory(library, doc, humanoid, clip_name):
    """Independent numeric audit of target poses; never serializes avatar GLBs."""
    rest = doc.rest_local_pose()
    rest_world = doc.world_pose(rest)
    hips = humanoid["hips"]
    facing = reference_facing(rest_world[humanoid["leftHand"]][0], rest_world[humanoid["rightHand"]][0])
    source_facing = library["source_reference_facing"]
    angle = math.atan2(facing[0], facing[2]) - math.atan2(source_facing[0], source_facing[2])
    align = (0, math.sin(angle / 2), 0, math.cos(angle / 2))
    inverse_align = legacy.qconj(align)
    feet_height = sum(rest_world[humanoid[bone]][0][1] for bone in ("leftFoot", "rightFoot")) / 2
    ratio = (rest_world[hips][0][1] - feet_height) / library["source_hips_to_feet_distance"]
    if ratio <= 0:
        raise ValueError("target hips height must be positive")
    clip = library["clips"][clip_name]
    channels = target_channels(clip, humanoid)
    poses = []
    for frame in range(len(clip["times"])):
        local = {}
        world_rot = {}
        for node in doc.topo:
            parent = doc.parent.get(node)
            parent_rotation = world_rot[parent] if parent is not None else (0, 0, 0, 1)
            rotation = rest[node][1]
            if node in channels:
                delta = legacy.qmul(align, legacy.qmul(channels[node][frame], inverse_align))
                desired = legacy.qnorm(legacy.qmul(delta, rest_world[node][1]))
                rotation = legacy.qnorm(legacy.qmul(legacy.qconj(parent_rotation), desired))
            world_rot[node] = legacy.qnorm(legacy.qmul(parent_rotation, rotation))
            translation = rest[node][0]
            if node == hips:
                delta = tuple(v * ratio for v in legacy.qrot(align, clip["hips_world_deltas"][frame]))
                desired = tuple(a + b for a, b in zip(rest_world[hips][0], delta))
                if parent is None:
                    translation = desired
                else:
                    position, parent_rest_rotation = rest_world[parent]
                    translation = legacy.qrot(legacy.qconj(parent_rest_rotation), tuple(a - b for a, b in zip(desired, position)))
            local[node] = (translation, rotation)
        poses.append(local)
    return poses


def audit_roster(library, avatars_dir=ROOT / legacy.AVATARS_DIR):
    entries = json.loads((avatars_dir / "manifest.json").read_text())["avatars"]
    results = []
    for entry in entries:
        path = avatars_dir / (entry["slug"] + ".glb")
        before = sha256(path)
        doc = legacy.Gltf.from_glb(path)
        for node in doc.nodes:
            if any(abs(s - 1) > 1e-5 for s in node.get("scale", [1, 1, 1])):
                raise ValueError(f"{entry['slug']}: audit expects unit-scale shipped rig")
        humanoid = {bone["bone"]: bone["node"] for bone in doc.js["extensions"]["VRM"]["humanoid"]["humanBones"]}
        if not set(REQUIRED) <= set(humanoid):
            raise ValueError(f"{entry['slug']}: incomplete humanoid")
        rest = doc.rest_local_pose()
        poses = retarget_in_memory(library, doc, humanoid, "run")
        all_values = [v for pose in poses for translation, rotation in pose.values() for v in (*translation, *rotation)]
        if not all(math.isfinite(v) for v in all_values):
            raise ValueError(f"{entry['slug']}: non-finite target pose")
        joint_excursions = {
            bone: max(angular_distance(a[humanoid[bone]][1], b[humanoid[bone]][1]) for a in poses for b in poses)
            for bone in (*LEGS, "hips")
        }
        if any(v < 0.02 for v in joint_excursions.values()):
            raise ValueError(f"{entry['slug']}: run must move every leg joint and hips")
        hips_positions = [pose[humanoid["hips"]][0] for pose in poses]
        hips_range = [max(p[i] for p in hips_positions) - min(p[i] for p in hips_positions) for i in range(3)]
        if max(hips_range) < 0.02:
            raise ValueError(f"{entry['slug']}: missing running hips motion")
        world = [doc.world_pose(pose) for pose in poses]
        foot_ranges = {
            bone: [max(p[humanoid[bone]][0][i] for p in world) - min(p[humanoid[bone]][0][i] for p in world) for i in range(3)]
            for bone in ("leftFoot", "rightFoot")
        }
        translation_error = max(
            abs(a - b)
            for pose in poses for node, (translation, _) in pose.items() if node != humanoid["hips"]
            for a, b in zip(translation, rest[node][0])
        )
        seam = max(angular_distance(poses[0][node][1], poses[-1][node][1]) for node in rest)
        if seam > 1e-6 or translation_error > 1e-8 or hips_positions[0] != hips_positions[-1]:
            raise ValueError(f"{entry['slug']}: failed loop/limb-translation invariants")
        if before != sha256(path):
            raise ValueError(f"{entry['slug']}: avatar bytes changed during read-only audit")
        results.append({
            "slug": entry["slug"], "model_sha256": before,
            "nodes": len(doc.nodes), "mapped_channels": len(target_channels(library["clips"]["run"], humanoid)),
            "leg_and_hips_rotation_excursion_rad": joint_excursions,
            "hips_local_range_m": hips_range, "foot_world_ranges_m": foot_ranges,
            "max_loop_rotation_error_rad": seam,
            "max_non_hips_translation_error_m": translation_error,
            "loop_hips_endpoint_error_m": 0.0, "finite": True, "source_bytes_unchanged": True,
        })
    run = library["clips"]["run"]
    return {
        "status": "PASS", "asset": str(OUTPUT.relative_to(ROOT)),
        "asset_sha256": hashlib.sha256(encode_library(library)).hexdigest(),
        "source": library["source"], "rig_count": len(results),
        "run_source": run["source_clip"], "run_duration_s": run["duration"],
        "run_frames": len(run["times"]), "walk_duration_s": library["clips"]["walk"]["duration"],
        "scope": "Read-only numeric retarget audit; native runtime rendering is verified separately",
        "rigs": results,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=SOURCE)
    parser.add_argument("--output", type=Path, default=OUTPUT)
    parser.add_argument("--check", action="store_true", help="fail if the committed output differs; never write it")
    parser.add_argument("--audit", type=Path, help="write a numeric read-only retarget report for all shipped rigs")
    args = parser.parse_args()
    library = export_library(args.source)
    output = encode_library(library)
    if args.check:
        if not args.output.exists() or args.output.read_bytes() != output:
            raise SystemExit(f"Motion output is stale: run {Path(__file__).name}")
    else:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_bytes(output)
    if args.audit:
        audit = audit_roster(library)
        args.audit.parent.mkdir(parents=True, exist_ok=True)
        args.audit.write_text(json.dumps(audit, indent=2, allow_nan=False) + "\n")
    print(f"{'Verified' if args.check else 'Wrote'} {args.output}: {len(output)} bytes, "
          f"{len(library['bones'])} semantic bones, {len(library['clips'])} motions, "
          f"sha256 {hashlib.sha256(output).hexdigest()}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
