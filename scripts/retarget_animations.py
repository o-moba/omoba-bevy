#!/usr/bin/env python3
"""Retarget Quaternius UAL animation clips onto staged VRM avatar GLBs.

Bakes the base clip set (idle, walk, attack, cast, death) from the CC0
Quaternius Universal Animation Library (Blender Rigify DEF-* skeleton,
`assets-src/animations/AnimationLibrary_Godot_Standard.gltf`) into every
avatar GLB listed in `client/assets/avatars/manifest.json`, so the runtime
keeps using the stock Bevy glTF loader with embedded clips.

Method (world-space delta retarget, both rigs T-pose):
  * Source reference pose: the UAL `A_TPose` clip sampled at its first key.
  * Target reference pose: the VRM rest pose (VRM 0.x mandates T-pose rest).
  * Source bones are resolved by Rigify name; target bones through the VRM
    humanoid bone map (`extensions.VRM.humanoid.humanBones`), which is
    authoritative even when node names are nonstandard.
  * Per frame, for every mapped bone:
        delta      = R_src_world(t) * inv(R_src_world_ref)
        R_tgt_world = A * delta * inv(A) * R_tgt_world_ref
    where A is a yaw alignment (identity or 180 degrees) chosen from the rest
    facing of each rig (sign of the left hand's world X in T-pose). Local
    rotations are recovered root-to-leaf through the animated parent chain.
  * Hips translation: world-space hips delta, rotated by A and scaled by
    hipsHeight_target / hipsHeight_source, applied on top of the target
    hips rest position.

Output: animations named exactly `idle`, `walk`, `attack`, `cast`, `death`
appended to each GLB (new accessors/buffer views appended to the BIN chunk;
existing bytes untouched). Idempotent: a marker in `asset.extras` records the
pre-injection array sizes and BIN length, and a re-run first rolls the file
back to that baseline before re-injecting.

Pure Python (stdlib only). Run from the repo root:

    python3 scripts/retarget_animations.py [--only SLUG] [--verbose]
"""

import argparse
import json
import math
import struct
import sys
from pathlib import Path

SOURCE_GLTF = Path("assets-src/animations/AnimationLibrary_Godot_Standard.gltf")
AVATARS_DIR = Path("client/assets/avatars")
MARKER_KEY = "omoba_retarget"

FLOAT = 5126  # glTF component type for float32
COMPONENT_COUNT = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4, "MAT4": 16}

# (source clip in the UAL library, clip name injected into the avatar GLB)
CLIP_MAP = [
    ("Idle_Loop", "idle"),
    ("Walk_Loop", "walk"),
    ("Sword_Attack", "attack"),
    ("Spell_Simple_Shoot", "cast"),
    ("Death01", "death"),
]

# Rigify DEF-* bone -> VRM humanoid bone. Spine chain handled separately.
BONE_MAP = {
    "DEF-hips": "hips",
    "DEF-neck": "neck",
    "DEF-head": "head",
    "DEF-shoulder.L": "leftShoulder",
    "DEF-upper_arm.L": "leftUpperArm",
    "DEF-forearm.L": "leftLowerArm",
    "DEF-hand.L": "leftHand",
    "DEF-thigh.L": "leftUpperLeg",
    "DEF-shin.L": "leftLowerLeg",
    "DEF-foot.L": "leftFoot",
    "DEF-toe.L": "leftToes",
    "DEF-shoulder.R": "rightShoulder",
    "DEF-upper_arm.R": "rightUpperArm",
    "DEF-forearm.R": "rightLowerArm",
    "DEF-hand.R": "rightHand",
    "DEF-thigh.R": "rightUpperLeg",
    "DEF-shin.R": "rightLowerLeg",
    "DEF-foot.R": "rightFoot",
    "DEF-toe.R": "rightToes",
}
# Fingers (optional on the target; skipped when the humanoid map lacks them).
for side, vside in (("L", "left"), ("R", "right")):
    for rig, vrm in (
        ("f_index", "Index"),
        ("f_middle", "Middle"),
        ("f_ring", "Ring"),
        ("f_pinky", "Little"),
        ("thumb", "Thumb"),
    ):
        for seg, vseg in (("01", "Proximal"), ("02", "Intermediate"), ("03", "Distal")):
            BONE_MAP[f"DEF-{rig}.{seg}.{side}"] = f"{vside}{vrm}{vseg}"

# ---------------------------------------------------------------------------
# Quaternion / vector helpers (glTF order: [x, y, z, w])
# ---------------------------------------------------------------------------


def qmul(a, b):
    ax, ay, az, aw = a
    bx, by, bz, bw = b
    return (
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    )


def qconj(q):
    return (-q[0], -q[1], -q[2], q[3])


def qnorm(q):
    n = math.sqrt(sum(c * c for c in q))
    if n == 0.0:
        return (0.0, 0.0, 0.0, 1.0)
    return tuple(c / n for c in q)


def qrot(q, v):
    x, y, z, w = q
    vx, vy, vz = v
    tx = 2.0 * (y * vz - z * vy)
    ty = 2.0 * (z * vx - x * vz)
    tz = 2.0 * (x * vy - y * vx)
    return (
        vx + w * tx + (y * tz - z * ty),
        vy + w * ty + (z * tx - x * tz),
        vz + w * tz + (x * ty - y * tx),
    )


def qslerp(a, b, t):
    dot = sum(x * y for x, y in zip(a, b))
    if dot < 0.0:
        b = tuple(-c for c in b)
        dot = -dot
    if dot > 0.9995:
        return qnorm(tuple(x + t * (y - x) for x, y in zip(a, b)))
    theta = math.acos(max(-1.0, min(1.0, dot)))
    sin_theta = math.sin(theta)
    wa = math.sin((1.0 - t) * theta) / sin_theta
    wb = math.sin(t * theta) / sin_theta
    return qnorm(tuple(wa * x + wb * y for x, y in zip(a, b)))


def vlerp(a, b, t):
    return tuple(x + t * (y - x) for x, y in zip(a, b))


# ---------------------------------------------------------------------------
# Minimal glTF document
# ---------------------------------------------------------------------------

GLB_MAGIC = b"glTF"
CHUNK_JSON = 0x4E4F534A
CHUNK_BIN = 0x004E4942


class Gltf:
    def __init__(self, js, binary):
        self.js = js
        self.bin = binary
        self.nodes = js.get("nodes", [])
        self.parent = {}
        for i, node in enumerate(self.nodes):
            for child in node.get("children", []):
                self.parent[child] = i
        # Topological order: parents before children.
        self.topo = []
        roots = [i for i in range(len(self.nodes)) if i not in self.parent]
        stack = list(reversed(roots))
        while stack:
            i = stack.pop()
            self.topo.append(i)
            stack.extend(reversed(self.nodes[i].get("children", [])))

    @classmethod
    def from_gltf_file(cls, path: Path):
        js = json.loads(path.read_text())
        buffers = js.get("buffers", [])
        binary = b""
        if buffers:
            uri = buffers[0].get("uri")
            if uri is None:
                raise ValueError("external .gltf without buffer uri")
            binary = (path.parent / uri).read_bytes()
        return cls(js, binary)

    @classmethod
    def from_glb(cls, path: Path):
        data = path.read_bytes()
        magic, version, length = struct.unpack("<4sII", data[:12])
        if magic != GLB_MAGIC or version != 2:
            raise ValueError(f"{path}: not a GLB v2 file")
        offset = 12
        js = None
        binary = b""
        while offset + 8 <= length:
            clen, ctype = struct.unpack("<II", data[offset : offset + 8])
            offset += 8
            chunk = data[offset : offset + clen]
            offset += clen
            if ctype == CHUNK_JSON:
                js = json.loads(chunk)
            elif ctype == CHUNK_BIN:
                binary = chunk
        if js is None:
            raise ValueError(f"{path}: missing JSON chunk")
        return cls(js, binary)

    def to_glb_bytes(self) -> bytes:
        self.js.setdefault("buffers", [{}])
        self.js["buffers"][0].pop("uri", None)
        self.js["buffers"][0]["byteLength"] = len(self.bin)
        js_bytes = json.dumps(self.js, separators=(",", ":")).encode("utf-8")
        js_pad = (4 - len(js_bytes) % 4) % 4
        js_bytes += b" " * js_pad
        bin_bytes = self.bin + b"\x00" * ((4 - len(self.bin) % 4) % 4)
        total = 12 + 8 + len(js_bytes) + 8 + len(bin_bytes)
        out = bytearray()
        out += struct.pack("<4sII", GLB_MAGIC, 2, total)
        out += struct.pack("<II", len(js_bytes), CHUNK_JSON)
        out += js_bytes
        out += struct.pack("<II", len(bin_bytes), CHUNK_BIN)
        out += bin_bytes
        return bytes(out)

    def read_accessor(self, index):
        acc = self.js["accessors"][index]
        if acc["componentType"] != FLOAT:
            raise ValueError(f"accessor {index}: unsupported componentType")
        n = COMPONENT_COUNT[acc["type"]]
        bv = self.js["bufferViews"][acc["bufferView"]]
        start = bv.get("byteOffset", 0) + acc.get("byteOffset", 0)
        count = acc["count"]
        values = struct.unpack_from(f"<{count * n}f", self.bin, start)
        if n == 1:
            return list(values)
        return [tuple(values[i * n : (i + 1) * n]) for i in range(count)]

    def rest_trs(self, index):
        node = self.nodes[index]
        if "matrix" in node:
            raise ValueError(f"node {index} uses matrix transform (unsupported)")
        return (
            tuple(node.get("translation", (0.0, 0.0, 0.0))),
            qnorm(tuple(node.get("rotation", (0.0, 0.0, 0.0, 1.0)))),
        )

    def world_pose(self, local_pose):
        """local_pose: {node: (T, R)} -> {node: (T_world, R_world)} (scale ignored)."""
        world = {}
        for i in self.topo:
            t, r = local_pose[i]
            p = self.parent.get(i)
            if p is None:
                world[i] = (t, r)
            else:
                pt, pr = world[p]
                world[i] = (
                    tuple(a + b for a, b in zip(pt, qrot(pr, t))),
                    qnorm(qmul(pr, r)),
                )
        return world

    def rest_local_pose(self):
        return {i: self.rest_trs(i) for i in range(len(self.nodes))}


# ---------------------------------------------------------------------------
# Source animation sampling
# ---------------------------------------------------------------------------


class Channel:
    def __init__(self, times, values, interpolation, is_rotation):
        self.times = times
        self.values = values
        self.interpolation = interpolation
        self.is_rotation = is_rotation

    def sample(self, t):
        times = self.times
        if t <= times[0]:
            return self.values[0]
        if t >= times[-1]:
            return self.values[-1]
        lo, hi = 0, len(times) - 1
        while hi - lo > 1:
            mid = (lo + hi) // 2
            if times[mid] <= t:
                lo = mid
            else:
                hi = mid
        if self.interpolation == "STEP":
            return self.values[lo]
        span = times[hi] - times[lo]
        f = 0.0 if span <= 0.0 else (t - times[lo]) / span
        if self.is_rotation:
            return qslerp(self.values[lo], self.values[hi], f)
        return vlerp(self.values[lo], self.values[hi], f)


class SourceClip:
    """Sampled channels of one UAL animation, keyed by (node, path)."""

    def __init__(self, doc: Gltf, anim):
        self.channels = {}
        self.timeline = set()
        for ch in anim["channels"]:
            path = ch["target"]["path"]
            if path not in ("translation", "rotation"):
                continue
            sampler = anim["samplers"][ch["sampler"]]
            times = doc.read_accessor(sampler["input"])
            values = doc.read_accessor(sampler["output"])
            interp = sampler.get("interpolation", "LINEAR")
            if interp == "CUBICSPLINE":
                # Keep only the in-between vertex values.
                values = values[1::3]
            is_rot = path == "rotation"
            if is_rot:
                values = [qnorm(v) for v in values]
            self.channels[(ch["target"]["node"], path)] = Channel(
                times, values, interp, is_rot
            )
            self.timeline.update(times)
        self.timeline = sorted(self.timeline)

    def pose_at(self, t, rest_pose):
        pose = {}
        for i, (rt, rr) in rest_pose.items():
            ct = self.channels.get((i, "translation"))
            cr = self.channels.get((i, "rotation"))
            pose[i] = (
                ct.sample(t) if ct else rt,
                qnorm(cr.sample(t)) if cr else rr,
            )
        return pose


# ---------------------------------------------------------------------------
# Retarget core
# ---------------------------------------------------------------------------


class SourceRig:
    def __init__(self, doc: Gltf):
        self.doc = doc
        self.name_to_node = {}
        for i, node in enumerate(doc.nodes):
            self.name_to_node[node.get("name", f"#{i}")] = i
        self.rest_pose = doc.rest_local_pose()
        anims = {a["name"]: a for a in doc.js.get("animations", [])}
        if "A_TPose" not in anims:
            raise ValueError("source library is missing the A_TPose reference clip")
        tpose = SourceClip(doc, anims["A_TPose"])
        self.ref_pose_local = tpose.pose_at(tpose.timeline[0], self.rest_pose)
        self.ref_world = doc.world_pose(self.ref_pose_local)
        self.anims = anims
        self.hips = self.name_to_node["DEF-hips"]
        self.left_hand = self.name_to_node["DEF-hand.L"]
        self.hips_height = self.ref_world[self.hips][0][1]
        self.left_hand_x = self.ref_world[self.left_hand][0][0]

    def clip(self, name):
        return SourceClip(self.doc, self.anims[name])


def build_bone_mapping(src: SourceRig, humanoid: dict, node_count: int):
    """Return {src_node: tgt_node}. `humanoid` maps VRM bone name -> node index."""
    mapping = {}
    # Spine chain: prefer 1:1 when upperChest exists, else map the top spine
    # bone to chest so the accumulated world delta still lands on the torso.
    if "upperChest" in humanoid:
        spine_pairs = [
            ("DEF-spine.001", "spine"),
            ("DEF-spine.002", "chest"),
            ("DEF-spine.003", "upperChest"),
        ]
    else:
        spine_pairs = [("DEF-spine.001", "spine"), ("DEF-spine.003", "chest")]
    for rig_name, vrm_bone in list(BONE_MAP.items()) + spine_pairs:
        src_node = src.name_to_node.get(rig_name)
        tgt_node = humanoid.get(vrm_bone)
        if src_node is None or tgt_node is None:
            continue
        if not 0 <= tgt_node < node_count:
            continue
        mapping[src_node] = tgt_node
    return mapping


def retarget_clip(src, clip, tgt_doc, mapping, align, ratio, tgt_rest_world):
    """Return (times, {tgt_node: [quat, ...]}, [hips_translation, ...])."""
    align_inv = qconj(align)
    tgt_rest_local = tgt_doc.rest_local_pose()
    tgt_hips = mapping[src.hips]
    hips_parent = tgt_doc.parent.get(tgt_hips)
    hips_rest_world_t = tgt_rest_world[tgt_hips][0]
    src_hips_ref_t = src.ref_world[src.hips][0]

    times = clip.timeline
    rotations = {tgt: [] for tgt in mapping.values()}
    translations = []

    for t in times:
        src_world = src.doc.world_pose(clip.pose_at(t, src.rest_pose))
        # Desired world rotation per mapped target node.
        desired = {}
        for s_node, t_node in mapping.items():
            delta = qmul(src_world[s_node][1], qconj(src.ref_world[s_node][1]))
            delta = qmul(align, qmul(delta, align_inv))
            desired[t_node] = qnorm(qmul(delta, tgt_rest_world[t_node][1]))
        # Root-to-leaf pass on the target: recover local rotations.
        world_rot = {}
        for i in tgt_doc.topo:
            p = tgt_doc.parent.get(i)
            parent_rot = world_rot[p] if p is not None else (0.0, 0.0, 0.0, 1.0)
            if i in desired:
                local = qnorm(qmul(qconj(parent_rot), desired[i]))
                world_rot[i] = desired[i]
                rotations[i].append(local)
            else:
                world_rot[i] = qnorm(qmul(parent_rot, tgt_rest_local[i][1]))
        # Hips translation: scaled world-space delta on top of the target rest.
        delta_p = tuple(a - b for a, b in zip(src_world[src.hips][0], src_hips_ref_t))
        delta_p = tuple(c * ratio for c in qrot(align, delta_p))
        desired_p = tuple(a + b for a, b in zip(hips_rest_world_t, delta_p))
        if hips_parent is None:
            local_p = desired_p
        else:
            pt, pr = tgt_rest_world[hips_parent]
            local_p = qrot(qconj(pr), tuple(a - b for a, b in zip(desired_p, pt)))
        translations.append(local_p)

    # Keep quaternion streams continuous (avoid sign flips during playback).
    for stream in rotations.values():
        for k in range(1, len(stream)):
            if sum(a * b for a, b in zip(stream[k - 1], stream[k])) < 0.0:
                stream[k] = tuple(-c for c in stream[k])
    return times, rotations, translations


# ---------------------------------------------------------------------------
# GLB injection (idempotent append)
# ---------------------------------------------------------------------------


def rollback_to_baseline(doc: Gltf):
    extras = doc.js.get("asset", {}).get("extras", {})
    marker = extras.get(MARKER_KEY)
    if marker:
        doc.js["animations"] = doc.js.get("animations", [])[: marker["baseAnimations"]]
        doc.js["accessors"] = doc.js.get("accessors", [])[: marker["baseAccessors"]]
        doc.js["bufferViews"] = doc.js.get("bufferViews", [])[: marker["baseBufferViews"]]
        doc.bin = doc.bin[: marker["baseBinLength"]]
        if not doc.js["animations"]:
            del doc.js["animations"]
    else:
        marker = {
            "baseAnimations": len(doc.js.get("animations", [])),
            "baseAccessors": len(doc.js.get("accessors", [])),
            "baseBufferViews": len(doc.js.get("bufferViews", [])),
            "baseBinLength": len(doc.bin),
        }
        doc.js.setdefault("asset", {}).setdefault("extras", {})[MARKER_KEY] = marker


class BinAppender:
    def __init__(self, doc: Gltf):
        self.doc = doc
        pad = (4 - len(doc.bin) % 4) % 4
        doc.bin += b"\x00" * pad
        self.blob = bytearray()

    def add_accessor(self, values, gltf_type, with_min_max=False):
        n = COMPONENT_COUNT[gltf_type]
        flat = []
        if n == 1:
            flat = list(values)
        else:
            for v in values:
                flat.extend(v)
        offset = len(self.doc.bin) + len(self.blob)
        self.blob += struct.pack(f"<{len(flat)}f", *flat)
        bv_index = len(self.doc.js["bufferViews"])
        self.doc.js["bufferViews"].append(
            {"buffer": 0, "byteOffset": offset, "byteLength": len(flat) * 4}
        )
        accessor = {
            "bufferView": bv_index,
            "componentType": FLOAT,
            "count": len(values),
            "type": gltf_type,
        }
        if with_min_max:
            accessor["min"] = [min(values)]
            accessor["max"] = [max(values)]
        index = len(self.doc.js["accessors"])
        self.doc.js["accessors"].append(accessor)
        return index

    def commit(self):
        self.doc.bin += bytes(self.blob)


def inject_clips(doc: Gltf, clips):
    """clips: [(name, times, {node: quats}, hips_node, translations)]."""
    appender = BinAppender(doc)
    doc.js.setdefault("animations", [])
    for name, times, rotations, hips_node, translations in clips:
        input_acc = appender.add_accessor(times, "SCALAR", with_min_max=True)
        samplers = []
        channels = []
        for node in sorted(rotations):
            output_acc = appender.add_accessor(rotations[node], "VEC4")
            samplers.append(
                {"input": input_acc, "interpolation": "LINEAR", "output": output_acc}
            )
            channels.append(
                {
                    "sampler": len(samplers) - 1,
                    "target": {"node": node, "path": "rotation"},
                }
            )
        output_acc = appender.add_accessor(translations, "VEC3")
        samplers.append(
            {"input": input_acc, "interpolation": "LINEAR", "output": output_acc}
        )
        channels.append(
            {"sampler": len(samplers) - 1, "target": {"node": hips_node, "path": "translation"}}
        )
        doc.js["animations"].append(
            {"name": name, "samplers": samplers, "channels": channels}
        )
    appender.commit()


# ---------------------------------------------------------------------------
# Per-avatar pipeline
# ---------------------------------------------------------------------------


def retarget_avatar(src: SourceRig, source_clips, glb_path: Path, verbose=False):
    doc = Gltf.from_glb(glb_path)
    rollback_to_baseline(doc)

    vrm = doc.js.get("extensions", {}).get("VRM")
    if not vrm:
        raise ValueError("no VRM extension (cannot resolve humanoid bone map)")
    humanoid = {
        b["bone"]: b["node"]
        for b in vrm.get("humanoid", {}).get("humanBones", [])
        if "bone" in b and "node" in b
    }
    for required in ("hips", "head", "leftHand", "leftUpperLeg"):
        if required not in humanoid:
            raise ValueError(f"humanoid map is missing required bone {required!r}")

    mapping = build_bone_mapping(src, humanoid, len(doc.nodes))
    tgt_rest_world = doc.world_pose(doc.rest_local_pose())

    tgt_hips_height = tgt_rest_world[humanoid["hips"]][0][1]
    if not (0.05 < tgt_hips_height < 3.0):
        raise ValueError(f"implausible target hips rest height {tgt_hips_height:.3f}")
    ratio = tgt_hips_height / src.hips_height

    # Facing alignment: compare the rest-pose left-hand world X signs.
    tgt_left_x = tgt_rest_world[humanoid["leftHand"]][0][0]
    if tgt_left_x * src.left_hand_x < 0.0:
        align = (0.0, 1.0, 0.0, 0.0)  # 180-degree yaw
    else:
        align = (0.0, 0.0, 0.0, 1.0)

    if verbose:
        print(
            f"    mapped bones: {len(mapping)}, hips height {tgt_hips_height:.3f} "
            f"(ratio {ratio:.3f}), yaw180={align[1] == 1.0}"
        )

    clips = []
    for source_name, clip_name in CLIP_MAP:
        clip = source_clips[source_name]
        times, rotations, translations = retarget_clip(
            src, clip, doc, mapping, align, ratio, tgt_rest_world
        )
        clips.append((clip_name, times, rotations, humanoid["hips"], translations))
        if verbose:
            print(
                f"    {clip_name:<6} <- {source_name}: {len(times)} keys, "
                f"{times[-1]:.2f}s, {len(rotations)} rotation channels"
            )
    inject_clips(doc, clips)
    glb_path.write_bytes(doc.to_glb_bytes())
    return len(mapping)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=SOURCE_GLTF)
    parser.add_argument("--avatars-dir", type=Path, default=AVATARS_DIR)
    parser.add_argument("--only", action="append", help="restrict to these slugs")
    parser.add_argument("--verbose", action="store_true")
    args = parser.parse_args()

    src = SourceRig(Gltf.from_gltf_file(args.source))
    source_clips = {name: src.clip(name) for name, _ in CLIP_MAP}

    manifest = json.loads((args.avatars_dir / "manifest.json").read_text())
    failures = []
    done = 0
    for entry in manifest["avatars"]:
        slug = entry["slug"]
        if args.only and slug not in args.only:
            continue
        glb_path = args.avatars_dir / f"{slug}.glb"
        print(f"Retargeting {slug} ...")
        try:
            mapped = retarget_avatar(src, source_clips, glb_path, verbose=args.verbose)
        except (ValueError, KeyError, OSError, struct.error) as err:
            failures.append((slug, str(err)))
            print(f"  FAIL: {err}")
            continue
        done += 1
        print(f"  OK ({mapped} bones mapped)")

    print(f"\nRetargeted {done} avatar(s); {len(failures)} failure(s).")
    for slug, err in failures:
        print(f"  FAILED {slug}: {err}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
