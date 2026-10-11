#!/usr/bin/env python3
"""Original alternative Wildspark gun sets: repeater + launcher in one set GLB.

python3 scripts/build_wildspark_skin.py --theme all            # build, render, validate
python3 scripts/build_wildspark_skin.py --theme all --verify   # re-check committed outputs
python3 scripts/build_wildspark_skin.py --finalize export.glb --output my-set.glb --credit "..."
python3 scripts/build_wildspark_skin.py --check my-set.glb     # every gate on any set GLB

Each theme writes only assets-src/skins/wildspark-sets/<theme>/: the set GLB,
cover.png (640x480), repeater.png and launcher.png (512x512) and manifest.json.
Blender runs as a child process with --factory-startup and an empty scene; the
geometry is generated here, so assets-src/skills/wildspark-reference.blend is
never opened or saved and client/assets is never written. The same file is the
Blender-side builder/renderer (sub-commands after `--`) and, without bpy, the
orchestrator, GLB post-processor and validation gates used by the unit tests.

Set convention (C4, handheld-glb-v1 static): docs/wildspark-skin-sets.md.
Canonical frame: origin = grip, +Y up, +Z muzzle, metres. Blender coordinates
of a canonical point (x, y, z) are (x, -z, y); the exporter converts back.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import sys
import tempfile

try:  # Blender side only; the orchestrator and tests import this file without bpy.
    import bmesh
    import bpy
    from mathutils import Matrix, Vector
    import numpy
except ImportError:
    bpy = None

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = Path(__file__).resolve()
OUTPUT_ROOT = ROOT / "assets-src/skins/wildspark-sets"
FORBIDDEN_OUTPUTS = (ROOT / "client", ROOT / "assets-src/skills")
DEFAULT_BLENDER = "/Applications/Blender.app/Contents/MacOS/Blender"

SET_ROOT = "wildspark-set"
REPEATER = "WildsparkSetRepeater"
LAUNCHER = "WildsparkSetLauncher"
ROTOR = "WildsparkRotor"
SLIDE = "WildsparkSlide"
ROTOR_PIVOT = (0.0, 0.12, 0.25)
SLIDE_PIVOT = (0.0, 0.13, 0.20)
MUZZLE = {"repeater": (0.0, 0.12, 0.735), "launcher": (0.0, 0.13, 0.81)}
# Bounds of the shipped baselines (weapons/wildspark-repeater.glb, wild-launcher.glb).
BASELINE_BOUNDS = {
    "repeater": ((-0.17, -0.155, -0.3375), (0.17, 0.2575, 0.735)),
    "launcher": ((-0.1825, -0.155, -0.3375), (0.1825, 0.3705, 0.81)),
}
ENVELOPE_MARGIN = 0.10  # of the baseline extent on each axis
MUZZLE_REACH = (0.02, 0.005)  # geometry must end within [muzzle - 0.02, muzzle + 0.005] on Z
GRIP = {"bone": "rightHand", "offset": [0, 0, 0], "rotation_degrees": [0, 0, 0], "scale": 1}
SET_HINT = {"repeater": REPEATER, "launcher": LAUNCHER,
            "muzzle": {"repeater": list(MUZZLE["repeater"]), "launcher": list(MUZZLE["launcher"])}}
LICENSE = "CC-BY-4.0"
REQUIREMENT = "wildspark.handheld.set"
PROFILE = {"kind": "weapon", "project": "omoba", "platform": "desktop", "profile": "handheld-glb-v1"}
LIMITS = {"bytes": 8 * 1024 * 1024, "nodes": 64, "meshes": 16}
ALLOWED_EXTENSIONS = {"KHR_materials_unlit", "KHR_texture_transform"}
IMAGES = {"cover": (640, 480), "repeater": (512, 512), "launcher": (512, 512)}

# Materials: (label, linear base colour, metallic, roughness, emission strength <= 1).
# Emission above 1 makes Blender emit KHR_materials_emissive_strength, which Studio
# and the registry reject.
THEMES = {
    "candy-neon": {
        "title": "Candy Neon",
        "backdrop": ((0.16, 0.07, 0.19), (0.035, 0.02, 0.05)),
        "rim": (1.0, 0.45, 0.85),
        "materials": {
            "pink": ("bubblegum", (1.0, 0.16, 0.52), 0.0, 0.32, 0.0),
            "mint": ("mint", (0.02, 0.68, 0.86), 0.0, 0.3, 0.0),
            "lemon": ("lemon", (1.0, 0.72, 0.04), 0.1, 0.3, 0.0),
            "cream": ("cream", (1.0, 0.88, 0.84), 0.0, 0.38, 0.0),
            "grape": ("grape", (0.16, 0.035, 0.26), 0.0, 0.55, 0.0),
            "glow-mint": ("mint glow", (0.15, 1.0, 0.92), 0.0, 0.25, 1.0),
            "glow-pink": ("heart glow", (1.0, 0.22, 0.66), 0.0, 0.25, 1.0),
        },
    },
    "scrap-punk": {
        "title": "Scrap Punk",
        "backdrop": ((0.085, 0.1, 0.07), (0.025, 0.03, 0.022)),
        "rim": (0.55, 1.0, 0.3),
        "materials": {
            "rust": ("rust", (0.36, 0.115, 0.035), 0.15, 0.9, 0.0),
            "rust-bright": ("rust bloom", (0.6, 0.2, 0.05), 0.1, 0.92, 0.0),
            "steel": ("scrap steel", (0.3, 0.3, 0.28), 0.75, 0.55, 0.0),
            "gunmetal": ("gunmetal", (0.13, 0.14, 0.145), 0.85, 0.45, 0.0),
            "tape": ("duct tape", (0.2, 0.21, 0.2), 0.0, 0.8, 0.0),
            "bolt": ("bolts", (0.55, 0.42, 0.2), 0.9, 0.35, 0.0),
            "hazard": ("hazard green", (0.25, 0.8, 0.03), 0.0, 0.5, 0.0),
            "soot": ("soot", (0.015, 0.015, 0.013), 0.0, 0.9, 0.0),
            "glow": ("toxic glow", (0.32, 1.0, 0.1), 0.0, 0.3, 1.0),
        },
        # Tiling 256x256 PNGs generated in Blender, multiplied by each material's colour.
        "textures": {"rust": ("rust", "rust-bright"), "grime": ("steel", "gunmetal")},
    },
}


def attribution(theme):
    return (f"{THEMES[theme]['title']} Wildspark set — original geometry by OMOBA contributors "
            f"(generated with scripts/build_wildspark_skin.py); {LICENSE}")


# ---------------------------------------------------------------- GLB container

def read_glb(data):
    """Strict GLB 2.0 split: (JSON document, BIN chunk payload). Raises ValueError."""
    if len(data) < 28 or struct.unpack_from("<4sII", data) != (b"glTF", 2, len(data)):
        raise ValueError("GLB header must be glTF 2 with the exact file length")
    length, kind = struct.unpack_from("<II", data, 12)
    if kind != 0x4E4F534A or length % 4 or 20 + length + 8 > len(data):
        raise ValueError("GLB JSON chunk must be 4-byte aligned and bounded")
    document = json.loads(data[20:20 + length])
    binary_length, binary_kind = struct.unpack_from("<II", data, 20 + length)
    if binary_kind != 0x004E4942 or binary_length % 4 or 28 + length + binary_length != len(data):
        raise ValueError("GLB must end with exactly one aligned BIN chunk")
    return document, data[28 + length:]


def pack_glb(document, binary):
    encoded = json.dumps(document, separators=(",", ":")).encode()
    encoded += b" " * (-len(encoded) % 4)
    binary += b"\0" * (-len(binary) % 4)
    total = 12 + 8 + len(encoded) + 8 + len(binary)
    return (struct.pack("<4sII", b"glTF", 2, total) + struct.pack("<I4s", len(encoded), b"JSON")
            + encoded + struct.pack("<I4s", len(binary), b"BIN\0") + binary)


def finalize(raw, credit):
    """Exporter output -> C4 set: grip + set hint extras, credit, hidden launcher."""
    document, binary = read_glb(raw)
    names = [node.get("name") for node in document.get("nodes", [])]
    for name in (SET_ROOT, REPEATER, LAUNCHER):
        if names.count(name) != 1:
            raise ValueError(f"exported GLB needs exactly one {name} node")
    launcher = document["nodes"][names.index(LAUNCHER)]
    launcher.pop("matrix", None)
    launcher["scale"] = [0, 0, 0]  # hidden for consumers that do not know the convention
    asset = document.setdefault("asset", {})
    asset["copyright"] = credit
    extras = asset.get("extras") if isinstance(asset.get("extras"), dict) else {}
    extras.update({"ekza_handheld_v1": dict(GRIP), "omoba_wildspark_set_v1": json.loads(json.dumps(SET_HINT))})
    asset["extras"] = extras
    return pack_glb(document, binary)


# ---------------------------------------------------------------- pure geometry

def _quat_matrix(q):
    x, y, z, w = q
    return [[1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
            [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
            [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)]]


def local_matrix(node, scale_override=None):
    if "matrix" in node:
        m = node["matrix"]  # column-major
        return [[m[c * 4 + r] for c in range(4)] for r in range(4)]
    t = node.get("translation", [0, 0, 0])
    r = _quat_matrix(node.get("rotation", [0, 0, 0, 1]))
    s = scale_override or node.get("scale", [1, 1, 1])
    return [[r[i][0] * s[0], r[i][1] * s[1], r[i][2] * s[2], t[i]] for i in range(3)] + [[0, 0, 0, 1]]


def _mul(a, b):
    return [[sum(a[i][k] * b[k][j] for k in range(4)) for j in range(4)] for i in range(4)]


def _apply(m, p):
    return tuple(m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2] + m[i][3] for i in range(3))


def node_index(document, name):
    matches = [i for i, node in enumerate(document.get("nodes", [])) if node.get("name") == name]
    return matches[0] if len(matches) == 1 else None


def subtree(document, start):
    nodes, out, stack = document["nodes"], [], [start]
    while stack:
        index = stack.pop()
        out.append(index)
        stack.extend(nodes[index].get("children", []))
    return out


def mesh_bounds(document, start=None, unhide=()):
    """World AABB of meshes under `start` (all scene roots when None) from accessor min/max."""
    nodes = document["nodes"]
    roots = [start] if start is not None else document["scenes"][document.get("scene", 0)]["nodes"]
    lo, hi = [math.inf] * 3, [-math.inf] * 3
    stack = [(index, [[1, 0, 0, 0], [0, 1, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]]) for index in roots]
    while stack:
        index, parent = stack.pop()
        node = nodes[index]
        world = _mul(parent, local_matrix(node, [1, 1, 1] if node.get("name") in unhide else None))
        if "mesh" in node:
            for primitive in document["meshes"][node["mesh"]]["primitives"]:
                accessor = document["accessors"][primitive["attributes"]["POSITION"]]
                a, b = accessor["min"], accessor["max"]
                for corner in [(x, y, z) for x in (a[0], b[0]) for y in (a[1], b[1]) for z in (a[2], b[2])]:
                    p = _apply(world, corner)
                    lo = [min(lo[i], p[i]) for i in range(3)]
                    hi = [max(hi[i], p[i]) for i in range(3)]
        stack.extend((child, world) for child in node.get("children", []))
    return tuple(round(v, 5) for v in lo), tuple(round(v, 5) for v in hi)


def allowed_envelope(kind):
    lo, hi = BASELINE_BOUNDS[kind]
    margin = [ENVELOPE_MARGIN * (hi[i] - lo[i]) for i in range(3)]
    return (tuple(round(lo[i] - margin[i], 5) for i in range(3)),
            tuple(round(hi[i] + margin[i], 5) for i in range(3)))


def glb_stats(document):
    meshes = document.get("meshes", [])
    primitives = [p for mesh in meshes for p in mesh.get("primitives", [])]
    accessors = document.get("accessors", [])
    return {
        "scenes": len(document.get("scenes", [])),
        "nodes": len(document.get("nodes", [])),
        "meshes": len(meshes),
        "primitives": len(primitives),
        "materials": len(document.get("materials", [])),
        "vertices": sum(accessors[p["attributes"]["POSITION"]]["count"] for p in primitives),
        "triangles": sum(accessors[p["indices"]]["count"] // 3 for p in primitives if "indices" in p),
    }


# ---------------------------------------------------------------- gates

def _extension_names(value, found, depth=0):
    if depth > 64:
        raise ValueError("JSON nested too deeply")
    if isinstance(value, dict):
        for key, child in value.items():
            if key == "extensions" and isinstance(child, dict):
                found.update(child)
            _extension_names(child, found, depth + 1)
    elif isinstance(value, list):
        for child in value:
            _extension_names(child, found, depth + 1)
    return found


def budget_gate(data):
    """(d) Budgets shared by Studio, the registry and the game, plus the art rules."""
    issues = []
    try:
        document, _ = read_glb(data)
    except (ValueError, struct.error) as error:
        return {"ok": False, "issues": [str(error)]}
    stats = glb_stats(document)
    if len(data) > LIMITS["bytes"]:
        issues.append(f"{len(data)} bytes exceeds 8 MiB")
    if stats["scenes"] != 1 or document.get("scene", 0) != 0:
        issues.append("needs exactly one scene (Scene0)")
    if not 1 <= stats["nodes"] <= LIMITS["nodes"]:
        issues.append(f"{stats['nodes']} nodes (limit {LIMITS['nodes']})")
    if not 1 <= stats["meshes"] <= LIMITS["meshes"]:
        issues.append(f"{stats['meshes']} meshes (limit {LIMITS['meshes']})")
    for key in ("skins", "animations"):
        if document.get(key):
            issues.append(f"{key} present; handheld-glb-v1 is static")
    if any(node.get("skin") is not None for node in document.get("nodes", [])):
        issues.append("node.skin present")
    if len(document.get("buffers", [])) != 1 or any("uri" in b for b in document.get("buffers", [])):
        issues.append("needs one embedded buffer")
    used = set(document.get("extensionsUsed", [])) | set(document.get("extensionsRequired", []))
    used |= _extension_names(document, set())
    if "KHR_materials_emissive_strength" in used:
        issues.append("KHR_materials_emissive_strength present (Emission Strength must be <= 1)")
    if used - ALLOWED_EXTENSIONS:
        issues.append(f"unsupported extensions {sorted(used - ALLOWED_EXTENSIONS)}")
    for image in document.get("images", []):
        if "uri" in image or image.get("mimeType") not in ("image/png", "image/jpeg"):
            issues.append("images must be embedded PNG/JPEG")
    return {"ok": not issues, "issues": issues, "sizeBytes": len(data), **stats}


def _close(a, b, tolerance=1e-4):
    return len(a) == len(b) and all(abs(x - y) <= tolerance for x, y in zip(a, b))


def c4_gate(data):
    """Wildspark set convention: names, hierarchy, pivots, hidden launcher, envelopes, muzzles."""
    issues = []
    try:
        document, _ = read_glb(data)
    except (ValueError, struct.error) as error:
        return {"ok": False, "issues": [str(error)]}
    nodes = document.get("nodes", [])
    names = [node.get("name", "") for node in nodes]
    suffixed = sorted(n for n in names if re.search(r"\.\d{3}$", n))
    if suffixed:
        issues.append(f"Blender duplicate suffixes break name binding: {suffixed}")
    if len(set(names)) != len(names):
        issues.append("node names must be unique")
    index = {name: node_index(document, name) for name in (SET_ROOT, REPEATER, LAUNCHER, ROTOR, SLIDE)}
    missing = [name for name, i in index.items() if i is None]
    if missing:
        return {"ok": False, "issues": issues + [f"missing or duplicated nodes {missing}"]}
    roots = document["scenes"][document.get("scene", 0)]["nodes"]
    if roots != [index[SET_ROOT]]:
        issues.append(f"scene root must be the single {SET_ROOT!r} node")
    root = nodes[index[SET_ROOT]]
    if sorted(root.get("children", [])) != sorted([index[REPEATER], index[LAUNCHER]]):
        issues.append(f"{SET_ROOT} children must be exactly {REPEATER} and {LAUNCHER}")

    def trs(node):
        return (node.get("translation", [0, 0, 0]), node.get("rotation", [0, 0, 0, 1]),
                node.get("scale", [1, 1, 1]), "matrix" in node)

    for name, scale in ((SET_ROOT, [1, 1, 1]), (REPEATER, [1, 1, 1]), (LAUNCHER, [0, 0, 0])):
        t, r, s, matrix = trs(nodes[index[name]])
        if matrix or not _close(t, [0, 0, 0]) or not _close(r, [0, 0, 0, 1]) or not _close(s, scale, 0):
            issues.append(f"{name} must have identity transform with scale {scale}")
    for part, owner, pivot in ((ROTOR, REPEATER, ROTOR_PIVOT), (SLIDE, LAUNCHER, SLIDE_PIVOT)):
        t, r, s, matrix = trs(nodes[index[part]])
        if index[part] not in subtree(document, index[owner]):
            issues.append(f"{part} must be under {owner}")
        if matrix or not _close(t, pivot) or not _close(r, [0, 0, 0, 1]) or not _close(s, [1, 1, 1]):
            issues.append(f"{part} rest pose must be T={list(pivot)}, identity rotation and scale")
        if not any("mesh" in nodes[i] for i in subtree(document, index[part])):
            issues.append(f"{part} carries no geometry")
    bounds = {}
    for kind, owner, moving in (("repeater", REPEATER, ROTOR), ("launcher", LAUNCHER, SLIDE)):
        static = [i for i in subtree(document, index[owner]) if "mesh" in nodes[i]
                  and i not in subtree(document, index[moving])]
        if not static:
            issues.append(f"{owner} needs static receiver geometry")
        lo, hi = mesh_bounds(document, index[owner], unhide=(LAUNCHER,))
        allowed_lo, allowed_hi = allowed_envelope(kind)
        bounds[kind] = {"min": list(lo), "max": list(hi)}
        if any(lo[i] < allowed_lo[i] or hi[i] > allowed_hi[i] for i in range(3)):
            issues.append(f"{kind} bounds {lo}..{hi} outside {allowed_lo}..{allowed_hi}")
        muzzle_z = MUZZLE[kind][2]
        if not muzzle_z - MUZZLE_REACH[0] <= hi[2] <= muzzle_z + MUZZLE_REACH[1]:
            issues.append(f"{kind} front Z {hi[2]} must end at the muzzle {muzzle_z}")
    asset = document.get("asset", {})
    extras = asset.get("extras", {}) if isinstance(asset.get("extras"), dict) else {}
    if extras.get("ekza_handheld_v1") != GRIP:
        issues.append("asset.extras.ekza_handheld_v1 must be the identity rightHand grip")
    if extras.get("omoba_wildspark_set_v1") != SET_HINT:
        issues.append("asset.extras.omoba_wildspark_set_v1 does not match C4")
    if not asset.get("copyright"):
        issues.append("asset.copyright missing")
    return {"ok": not issues, "issues": issues, "bounds": bounds}


STUDIO_RUNNER = """
import fs from 'node:fs';
import { pathToFileURL } from 'node:url';
const [lib, file] = process.argv.slice(2);
try {
  const { parseHandheld } = await import(pathToFileURL(lib).href);
  const bytes = fs.readFileSync(file);
  const parsed = parseHandheld(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength), true);
  console.log(JSON.stringify({ ok: true, grip: parsed.grip }));
} catch (error) {
  console.log(JSON.stringify({ ok: false, issues: [String(error && error.message || error)] }));
}
"""

REGISTRY_RUNNER = """
import json, sys
sys.dont_write_bytecode = True
sys.path.insert(0, sys.argv[1])
from pathlib import Path
try:
    from app.services.studio import validate_model
    validate_model(Path(sys.argv[2]), asset_kind="weapon")
    print(json.dumps({"ok": True}))
except Exception as error:
    detail = getattr(error, "message", None) or getattr(error, "detail", None) or str(error)
    print(json.dumps({"ok": False, "issues": [f"{type(error).__name__}: {detail}"]}))
"""


def _git_revision(root, *paths):
    try:
        return subprocess.check_output(["git", "log", "-1", "--format=%h", "--", *paths], cwd=root,
                                       text=True, stderr=subprocess.DEVNULL).strip() or None
    except (OSError, subprocess.CalledProcessError):
        return None


def _json_line(output):
    for line in reversed(output.strip().splitlines()):
        if line.startswith("{"):
            return json.loads(line)
    raise ValueError(output.strip()[-400:] or "no output")


def studio_gate(path, registry_root, node="node"):
    """(a) Studio browser preparation: web/app/lib/handheld-glb.ts parseHandheld(final=true)."""
    lib = Path(registry_root) / "web/app/lib/handheld-glb.ts"
    if not lib.is_file():
        return {"ok": False, "issues": ["Studio validator not found under --registry-root"]}
    with tempfile.TemporaryDirectory() as scratch:
        runner = Path(scratch) / "studio_check.mjs"
        runner.write_text(STUDIO_RUNNER)
        run = subprocess.run([node, str(runner), str(lib), str(path)], capture_output=True, text=True)
    try:
        result = _json_line(run.stdout)
    except ValueError as error:
        return {"ok": False, "issues": [f"node failed: {error} {run.stderr.strip()[-300:]}"]}
    result.update({"validator": "ekza-registry web/app/lib/handheld-glb.ts parseHandheld(final=true)",
                   "revision": _git_revision(registry_root, "web/app/lib/handheld-glb.ts")})
    return result


def registry_gate(path, registry_root, python=None):
    """(b) Registry upload boundary: backend app.services.studio.validate_model(kind=weapon)."""
    backend = Path(registry_root) / "backend"
    python = python or (backend / ".venv/bin/python")
    if not (backend / "app/services/handheld.py").is_file():
        return {"ok": False, "issues": ["registry backend not found under --registry-root"]}
    env = {**os.environ, "PYTHONDONTWRITEBYTECODE": "1"}
    run = subprocess.run([str(python), "-B", "-c", REGISTRY_RUNNER, str(backend), str(path)],
                         capture_output=True, text=True, env=env, cwd=tempfile.gettempdir())
    try:
        result = _json_line(run.stdout)
    except ValueError as error:
        return {"ok": False, "issues": [f"python failed: {error} {run.stderr.strip()[-300:]}"]}
    result.update({"validator": "ekza-registry backend app.services.studio.validate_model(asset_kind='weapon') "
                                "-> handheld.py + handheld_checks.py",
                   "revision": _git_revision(registry_root, "backend/app/services/handheld.py",
                                             "backend/app/services/handheld_checks.py",
                                             "backend/app/services/studio.py")})
    return result


def primary_checkout():
    """The main omoba-bevy checkout, also from a linked worktree."""
    try:
        common = subprocess.check_output(["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
                                         cwd=ROOT, text=True, stderr=subprocess.DEVNULL).strip()
        return Path(common).parent
    except (OSError, subprocess.CalledProcessError):
        return ROOT


def target_directory():
    if os.environ.get("CARGO_TARGET_DIR"):
        return Path(os.environ["CARGO_TARGET_DIR"]).resolve()
    return primary_checkout() / "target"


GAME_VALIDATOR_SOURCES = ("passport/src/weapons.rs", "passport/src/bin/weapon-build.rs")


def _validator_sources():
    """(newest commit "time sha" of the game validator sources, sources with local edits)."""
    try:
        committed = subprocess.check_output(["git", "log", "-1", "--format=%ct %h", "--", *GAME_VALIDATOR_SOURCES],
                                            cwd=ROOT, text=True).split()
        dirty = subprocess.check_output(["git", "status", "--porcelain", "--", *GAME_VALIDATOR_SOURCES],
                                        cwd=ROOT, text=True).split()
    except (OSError, subprocess.CalledProcessError):
        return (0, None), []
    return (int(committed[0]), committed[1]) if committed else (0, None), sorted(set(dirty) & set(GAME_VALIDATOR_SOURCES))


def game_gate(path, weapon_build):
    """(c) Game validator: passport weapon-build into a fresh scratch directory."""
    binary = Path(weapon_build)
    if not binary.is_file():
        return {"ok": False, "issues": ["weapon-build binary missing; build omoba-passport --bin weapon-build"]}
    (committed, revision), edited = _validator_sources()
    stale = committed > binary.stat().st_mtime
    with tempfile.TemporaryDirectory() as scratch:
        run = subprocess.run([str(binary), "--source", str(path), "--output-dir", str(Path(scratch) / "out")],
                             capture_output=True, text=True)
    try:
        report = _json_line(run.stdout)
    except ValueError as error:
        return {"ok": False, "issues": [f"weapon-build failed: {error}"]}
    digest = hashlib.sha256(Path(path).read_bytes()).hexdigest()
    result = {"ok": bool(report.get("ok")) and report.get("sha256") == digest and run.returncode == 0,
              "validator": "omoba-bevy passport weapon-build --source --output-dir (validate_model)",
              "revision": _git_revision(ROOT, *GAME_VALIDATOR_SOURCES)}
    if report.get("issues"):
        result["issues"] = [issue.get("message", str(issue)) for issue in report["issues"]]
    # A stale binary still ran its validate_model; say so instead of hiding or failing it.
    warnings = []
    if stale:
        warnings.append(f"weapon-build predates validator commit {revision}; rebuild "
                        "omoba-passport --bin weapon-build and re-run --verify")
    if edited:
        warnings.append(f"uncommitted edits in {edited} are not in this binary")
    if warnings:
        result["warnings"] = warnings
    return result


# ---------------------------------------------------------------- orchestration

def checked_output_dir(path):
    path = Path(path).resolve()
    for forbidden in FORBIDDEN_OUTPUTS:
        if path == forbidden.resolve() or path.is_relative_to(forbidden.resolve()):
            raise SystemExit(f"Refusing to write into {forbidden.relative_to(ROOT)}: use assets-src/skins")
    return path


def png_size(path):
    data = Path(path).read_bytes()[:24]
    if data[:8] != b"\x89PNG\r\n\x1a\n" or data[12:16] != b"IHDR":
        raise ValueError(f"{path} is not a PNG")
    return struct.unpack(">II", data[16:24])


def file_record(path):
    data = Path(path).read_bytes()
    record = {"path": Path(path).name, "sha256": hashlib.sha256(data).hexdigest(), "sizeBytes": len(data)}
    if Path(path).suffix == ".png":
        record["width"], record["height"] = png_size(path)
    return record


def run_blender(blender, *command):
    cmd = [blender, "-b", "--factory-startup", "--python-exit-code", "1", "--python", str(SCRIPT), "--", *command]
    run = subprocess.run(cmd, capture_output=True, text=True)
    report = None
    for line in run.stdout.splitlines():
        if line.startswith("WILDSPARK_SKIN "):
            report = json.loads(line[len("WILDSPARK_SKIN "):])
    if run.returncode or report is None:
        sys.stderr.write(run.stdout[-3000:] + run.stderr[-3000:])
        raise SystemExit(f"Blender {command[0]} failed ({run.returncode})")
    return report


def run_gates(glb, args):
    glb = Path(glb).resolve()  # the external validators run from scratch directories
    data = glb.read_bytes()
    return {
        "budgets": budget_gate(data),
        "c4": c4_gate(data),
        "studio": studio_gate(glb, args.registry_root, args.node),
        "registry": registry_gate(glb, args.registry_root, args.registry_python),
        "game": game_gate(glb, args.weapon_build),
    }


def manifest_for(theme, directory, glb, gates, generator):
    document, _ = read_glb(glb.read_bytes())
    files = {"model": file_record(glb)}
    for image in IMAGES:
        if (directory / f"{image}.png").is_file():
            files[image] = file_record(directory / f"{image}.png")
    return {
        "schema": "omoba.wildspark-set-skin.v1",
        "theme": theme,
        "title": f"{THEMES[theme]['title']} Wildspark set",
        "requirement": REQUIREMENT,
        "upload": PROFILE,
        "license": LICENSE,
        "attribution": attribution(theme),
        "grip": GRIP,
        "set": SET_HINT,
        "files": files,
        "glb": {**glb_stats(document), "nodeNames": [n.get("name") for n in document["nodes"]],
                "materialNames": [m.get("name") for m in document.get("materials", [])]},
        "bounds": gates["c4"].get("bounds"),
        "envelopes": {kind: {"baseline": [list(v) for v in BASELINE_BOUNDS[kind]],
                             "allowed": [list(v) for v in allowed_envelope(kind)]} for kind in BASELINE_BOUNDS},
        "gates": {name: {k: v for k, v in gate.items() if k not in ("bounds",)} for name, gate in gates.items()},
        "generator": generator,
    }


def build(theme, args):
    directory = checked_output_dir(args.out_root / theme)
    directory.mkdir(parents=True, exist_ok=True)
    glb = directory / f"{theme}-wildspark-set.glb"
    with tempfile.TemporaryDirectory() as scratch:
        raw = Path(scratch) / "export.glb"
        built = run_blender(args.blender, "blender-build", "--theme", theme, "--output", str(raw))
        glb.write_bytes(finalize(raw.read_bytes(), attribution(theme)))
    if not args.skip_render:
        run_blender(args.blender, "blender-render", "--theme", theme, "--glb", str(glb), "--output-dir", str(directory))
    gates = run_gates(glb, args)
    generator = {"script": "scripts/build_wildspark_skin.py", "blender": built["blender"],
                 "objects": built["objects"]}
    manifest = manifest_for(theme, directory, glb, gates, generator)
    (directory / "manifest.json").write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n")
    return manifest


def verify(theme, args):
    directory = args.out_root / theme
    manifest = json.loads((directory / "manifest.json").read_text())
    glb = directory / manifest["files"]["model"]["path"]
    problems = [f"{name}: sha256 differs from manifest" for name, record in manifest["files"].items()
                if file_record(directory / record["path"])["sha256"] != record["sha256"]]
    gates = run_gates(glb, args)
    problems += [f"gate {name}: {gate.get('issues')}" for name, gate in gates.items() if not gate.get("ok")]
    return problems, gates


def summary(gates):
    return " ".join(f"{name}={'ok' if gate.get('ok') else 'FAIL'}" for name, gate in gates.items())


def print_findings(gates):
    for name, gate in gates.items():
        for issue in gate.get("issues", []):
            print(f"  {name}: {issue}")
        for warning in gate.get("warnings", []):
            print(f"  {name} warning: {warning}")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--theme", default="all", choices=["all", *THEMES])
    parser.add_argument("--out-root", type=Path, default=OUTPUT_ROOT)
    parser.add_argument("--blender", default=os.environ.get("BLENDER", DEFAULT_BLENDER))
    parser.add_argument("--registry-root", type=Path,
                        default=Path(os.environ.get("EKZA_REGISTRY_ROOT", primary_checkout().parent / "ekza-registry")),
                        help="ekza-registry checkout holding the Studio and registry validators")
    parser.add_argument("--registry-python", help="Python with the registry backend dependencies "
                                                  "(default: <registry-root>/backend/.venv/bin/python)")
    parser.add_argument("--node", default="node", help="Node.js >= 22.6 with TypeScript type stripping")
    parser.add_argument("--weapon-build", type=Path, default=target_directory() / "debug/weapon-build")
    parser.add_argument("--skip-render", action="store_true", help="keep existing PNGs")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--verify", action="store_true", help="re-run every gate on committed outputs only")
    mode.add_argument("--check", nargs="+", type=Path, metavar="GLB", help="run every gate on any set GLB")
    mode.add_argument("--finalize", type=Path, metavar="EXPORT", help="turn a hand-made Blender export "
                      "into a set GLB: grip and set extras, --credit as asset.copyright, launcher scale 0")
    parser.add_argument("--output", type=Path, help="set GLB written by --finalize")
    parser.add_argument("--credit", help="asset.copyright for --finalize, e.g. your attribution line")
    args = parser.parse_args(argv)
    args.out_root = args.out_root.resolve()
    if args.finalize:
        if not args.output or not args.credit:
            parser.error("--finalize needs --output and --credit")
        output = checked_output_dir(args.output.parent) / args.output.name
        output.write_bytes(finalize(args.finalize.read_bytes(), args.credit))
        args.check = [output]
    if args.check:
        failed = False
        for path in args.check:
            gates = run_gates(path, args)
            print(f"CHECK {path.name}: {summary(gates)}")
            print_findings(gates)
            failed |= not all(gate.get("ok") for gate in gates.values())
        return 1 if failed else 0
    themes = list(THEMES) if args.theme == "all" else [args.theme]
    failed = False
    for theme in themes:
        if args.verify:
            problems, gates = verify(theme, args)
            print(f"VERIFY {theme}: {summary(gates)}")
            print_findings(gates)
            for problem in problems:
                if not problem.startswith("gate "):
                    print(f"  {problem}")
            failed |= bool(problems)
        else:
            manifest = build(theme, args)
            print(f"BUILT {theme}: {manifest['files']['model']['sizeBytes']} bytes sha256 "
                  f"{manifest['files']['model']['sha256'][:12]} nodes {manifest['glb']['nodes']} "
                  f"meshes {manifest['glb']['meshes']} {summary(manifest['gates'])}")
            print_findings(manifest["gates"])
            failed |= not all(gate.get("ok") for gate in manifest["gates"].values())
    return 1 if failed else 0


# ================================================================= Blender side

SHARP_ANGLE = math.radians(38)


def _rot(axis, degrees):
    return Matrix.Rotation(math.radians(degrees), 4, axis)


def _frame(origin, normal, up=(0, 1, 0)):
    """4x4 with local Z = normal and local Y as close to `up` as possible."""
    w = Vector(normal).normalized()
    u = Vector(up).cross(w)
    if u.length < 1e-6:
        u = Vector((0, 0, 1)).cross(w)
    u.normalize()
    v = w.cross(u)
    m = Matrix((u, v, w)).transposed().to_4x4()
    m.translation = Vector(origin)
    return m


def _along(a, b):
    a, b = Vector(a), Vector(b)
    m = (b - a).to_track_quat("Z", "Y").to_matrix().to_4x4()
    m.translation = a
    return m, (b - a).length


def star(outer, inner, points=5):
    return [((outer if i % 2 == 0 else inner) * math.cos(math.pi / 2 + i * math.pi / points),
             (outer if i % 2 == 0 else inner) * math.sin(math.pi / 2 + i * math.pi / points))
            for i in range(points * 2)]


def heart(height, samples=36):
    raw = []
    for i in range(samples):
        t = math.tau * i / samples
        raw.append((16 * math.sin(t) ** 3,
                    13 * math.cos(t) - 5 * math.cos(2 * t) - 2 * math.cos(3 * t) - math.cos(4 * t)))
    lo_y, hi_y = min(p[1] for p in raw), max(p[1] for p in raw)
    scale = height / (hi_y - lo_y)
    mid = (lo_y + hi_y) / 2
    return [(x * scale, (y - mid) * scale) for x, y in raw]


class Part:
    """One exported mesh node built in canonical coordinates around its pivot."""

    def __init__(self, name, owner, pivot=(0, 0, 0)):
        self.name, self.owner, self.pivot = name, owner, Vector(pivot)
        self.bm = bmesh.new()
        self.slots = []

    def _slot(self, key):
        if key not in self.slots:
            self.slots.append(key)
        return self.slots.index(key)

    def _paint(self, verts, key):
        for face in {f for v in verts for f in v.link_faces}:
            face.material_index = self._slot(key)

    def box(self, center, size, key, bevel=0.0, segments=1, rot=None):
        m = Matrix.Translation(center) @ (rot or Matrix.Identity(4)) @ Matrix.Diagonal((*size, 1))
        verts = bmesh.ops.create_cube(self.bm, size=1.0, matrix=m)["verts"]
        self._paint(verts, key)
        if bevel:
            edges = list({e for v in verts for e in v.link_edges})
            bmesh.ops.bevel(self.bm, geom=verts + edges, offset=bevel, segments=segments, profile=0.5,
                            affect="EDGES", clamp_overlap=True, material=-1)

    def lathe(self, profile, key, n=16, frame=None, phase=0.0, closed=False, twist=None):
        """Revolve [(radius, z)] about frame-local Z. key: name or fn(segment, sector).

        twist(r, z) adds a per-ring angle, so sector-keyed stripes become smooth spirals.
        """
        frame = frame or Matrix.Identity(4)
        pick = key if callable(key) else (lambda i, j: key)
        rings = []
        for r, z in profile:
            if r <= 1e-7:
                rings.append([self.bm.verts.new(frame @ Vector((0, 0, z)))])
            else:
                start = phase + (twist(r, z) if twist else 0.0)
                rings.append([self.bm.verts.new(frame @ Vector((r * math.cos(start + math.tau * j / n),
                                                                r * math.sin(start + math.tau * j / n), z)))
                              for j in range(n)])
        count = len(rings)
        for i in range(count if closed else count - 1):
            a, b = rings[i], rings[(i + 1) % count]
            for j in range(n):
                k = (j + 1) % n
                if len(a) == 1 and len(b) == 1:
                    continue
                if len(a) == 1:
                    face = (a[0], b[j], b[k])
                elif len(b) == 1:
                    face = (a[j], a[k], b[0])
                else:
                    face = (a[j], a[k], b[k], b[j])
                self.bm.faces.new(face).material_index = self._slot(pick(i, j))
        if not closed:
            for ring, segment in ((rings[0], 0), (rings[-1], count - 2)):
                if len(ring) > 1:
                    self.bm.faces.new(ring).material_index = self._slot(pick(segment, 0))

    def cylinder(self, a, b, radius, key, n=16, chamfer=0.0):
        frame, length = _along(a, b)
        c = min(chamfer, length / 3)
        profile = ([(radius - c, 0), (radius, c), (radius, length - c), (radius - c, length)] if c
                   else [(radius, 0), (radius, length)])
        self.lathe(profile, key, n, frame)

    def tube(self, a, b, outer, inner, key, n=16, phase=0.0):
        frame, length = _along(a, b)
        self.lathe([(outer, 0), (outer, length), (inner, length), (inner, 0)], key, n, frame, phase, closed=True)

    def cone(self, a, b, radius, key, n=16, tip=0.0):
        frame, length = _along(a, b)
        self.lathe([(0, 0), (radius, 0), (tip, length)] if tip else [(0, 0), (radius, 0), (0, length)],
                   key, n, frame)

    def sphere(self, center, radius, key, n=16, rings=8):
        profile = [(0, -radius)] + [(radius * math.sin(math.pi * i / rings), -radius * math.cos(math.pi * i / rings))
                                     for i in range(1, rings)] + [(0, radius)]
        self.lathe(profile, key, n, Matrix.Translation(center))

    def torus(self, center, axis, major, minor, key, n=24, m=8):
        profile = [(major + minor * math.cos(math.tau * i / m), minor * math.sin(math.tau * i / m)) for i in range(m)]
        self.lathe(profile, key, n, _frame(center, axis), closed=True)

    def prism(self, points, depth, frame, key):
        bottom = [self.bm.verts.new(frame @ Vector((u, v, -depth / 2))) for u, v in points]
        top = [self.bm.verts.new(frame @ Vector((u, v, depth / 2))) for u, v in points]
        slot = self._slot(key)
        self.bm.faces.new(list(reversed(bottom))).material_index = slot
        self.bm.faces.new(top).material_index = slot
        for i in range(len(points)):
            j = (i + 1) % len(points)
            self.bm.faces.new((bottom[i], bottom[j], top[j], top[i])).material_index = slot

    def bolt(self, center, normal, key, radius=0.01, height=0.008):
        c, w = Vector(center), Vector(normal).normalized()
        self.cylinder(c - w * height / 2, c + w * height / 2, radius, key, n=6, chamfer=height / 4)

    def finish(self, materials, parent, uv_scale=None):
        bm = self.bm
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
        if uv_scale:  # box projection in canonical metres for the tiling textures
            bm.normal_update()
            uv = bm.loops.layers.uv.new("UVMap")
            for face in bm.faces:
                axis = max(range(3), key=lambda i: abs(face.normal[i]))
                a, b = ((1, 2), (0, 2), (0, 1))[axis]
                for loop in face.loops:
                    loop[uv].uv = (loop.vert.co[a] * uv_scale + 0.37 * axis, loop.vert.co[b] * uv_scale)
        for face in bm.faces:
            face.smooth = True
        for edge in bm.edges:
            if not edge.is_manifold or edge.calc_face_angle(math.pi) > SHARP_ANGLE:
                edge.smooth = False
        to_blender = Matrix(((1, 0, 0, 0), (0, 0, -1, 0), (0, 1, 0, 0), (0, 0, 0, 1)))
        bmesh.ops.transform(bm, matrix=to_blender @ Matrix.Translation(-self.pivot), verts=bm.verts[:])
        mesh = bpy.data.meshes.new(self.name)
        bm.to_mesh(mesh)
        bm.free()
        for key in self.slots:
            mesh.materials.append(materials[key])
        obj = bpy.data.objects.new(self.name, mesh)
        bpy.context.scene.collection.objects.link(obj)
        obj.parent = parent
        obj.location = to_blender @ self.pivot
        return obj


# ---------------------------------------------------------------- Candy Neon

def _candy_chassis(part, shell, trim, decal):
    """Toy pistol frame shared by both guns; colours swap between repeater and launcher."""
    rake = _rot("X", 8)
    part.box((0, -0.05, -0.012), (0.074, 0.2, 0.09), "grape", bevel=0.03, segments=3, rot=rake)
    axis = (rake @ Vector((0, 1, 0, 0))).to_3d()
    bottom = Vector((0, -0.05, -0.012)) - axis * 0.1
    part.cylinder(bottom - axis * 0.004, bottom - axis * 0.03, 0.043, "lemon", n=20, chamfer=0.009)
    part.torus((0, -0.012, 0.062), (1, 0, 0), 0.037, 0.0095, "lemon", n=24, m=8)
    part.box((0, -0.004, 0.056), (0.012, 0.036, 0.014), "cream", bevel=0.004, rot=_rot("X", -15))
    part.box((0, 0.115, 0.03), (0.17, 0.165, 0.34), shell, bevel=0.042, segments=4)
    part.box((0, 0.057, 0.035), (0.178, 0.045, 0.28), trim, bevel=0.018, segments=3)
    part.box((0, 0.1955, 0.03), (0.05, 0.012, 0.25), "cream", bevel=0.005, segments=2)
    for side in (-1, 1):
        part.prism(star(0.036, 0.016), 0.012, _frame((side * 0.087, 0.118, 0.045), (side, 0, 0)), decal)
    # Candy-cane stock ending in a swirl lollipop butt.
    stick = [(0.027, -0.11 - 0.0088 * i) for i in range(21)]
    part.lathe(stick, lambda i, j: "cream" if (j // 4) % 2 else shell, 16, Matrix.Translation((0, 0.112, 0)),
               twist=lambda r, z: z * 48)
    rim = [(0.091, -0.29), (0.097, -0.305), (0.091, -0.32)]
    swirl = ([(0.0915 * i / 8, -0.282 - 0.002 * i / 8) for i in range(8)] + rim
             + [(0.0915 * (8 - i) / 8, -0.326 - 0.002 * i / 8) for i in range(1, 9)])
    part.lathe(swirl, lambda i, j: "cream" if (j // 3) % 2 else shell, 24, Matrix.Translation((0, 0.112, 0)),
               twist=lambda r, z: r * 42)


def candy_neon():
    repeater = Part("WildsparkRepeaterReceiver", REPEATER)
    rotor = Part(ROTOR, REPEATER, ROTOR_PIVOT)
    launcher = Part("WildsparkLauncherReceiver", LAUNCHER)
    slide = Part(SLIDE, LAUNCHER, SLIDE_PIVOT)

    # Repeater: pink toy frame, gumball hopper, heart sight, donut drum, pinwheel rotor.
    _candy_chassis(repeater, "pink", "mint", "lemon")
    repeater.prism(star(0.036, 0.016), 0.01, _frame((0, 0.2, 0.1), (0, 1, 0), (0, 0, 1)), "lemon")
    repeater.cylinder((0, 0.185, -0.05), (0, 0.212, -0.05), 0.046, "lemon", n=24, chamfer=0.006)
    for center, radius, key in [((0, 0.238, -0.05), 0.034, "glow-mint"), ((0.032, 0.228, -0.028), 0.025, "pink"),
                                ((-0.032, 0.23, -0.074), 0.025, "cream"), ((-0.03, 0.228, -0.02), 0.023, "lemon"),
                                ((0.03, 0.231, -0.078), 0.024, "glow-pink"), ((0.0, 0.266, -0.05), 0.024, "pink")]:
        repeater.sphere(center, radius, key, n=14, rings=7)
    repeater.box((0, 0.21, 0.168), (0.012, 0.03, 0.012), "lemon")
    repeater.prism(heart(0.06), 0.018, _frame((0, 0.25, 0.168), (0, 0, 1)), "glow-pink")
    repeater.cylinder((0, 0.12, 0.185), (0, 0.12, 0.246), 0.086, "mint", n=24, chamfer=0.008)
    donut = (0, -0.036, 0.185)
    repeater.torus(donut, (1, 0, 0), 0.041, 0.024, lambda i, j: "pink" if i in (4, 5, 6, 7) else "cream", n=24, m=8)
    repeater.box((0, 0.012, 0.185), (0.026, 0.05, 0.03), "mint", bevel=0.006)
    for angle in range(0, 360, 45):
        a = math.radians(angle)
        repeater.box((-0.025, donut[1] + 0.041 * math.sin(a), donut[2] + 0.041 * math.cos(a)), (0.006, 0.006, 0.014),
                     ["lemon", "glow-mint", "cream"][angle // 45 % 3], rot=_rot("X", angle + 30))
    x, y, z = ROTOR_PIVOT
    rotor.lathe([(0, z), (0.084, z), (0.098, z + 0.012), (0.098, z + 0.04), (0.084, z + 0.052), (0, z + 0.052)],
                "lemon", 28, Matrix.Translation((x, y, 0)))
    for k in range(3):
        a = math.radians(90 + 120 * k)
        cx, cy = 0.056 * math.cos(a), y + 0.056 * math.sin(a)
        zs = [0.3 + 0.025 * i for i in range(17)]
        outer = [(0.036, zz) for zz in zs]
        rotor.lathe(outer + [(0.021, zs[-1]), (0.021, zs[0])],
                    lambda i, j: ("cream" if (j // 2) % 2 else "pink") if i < len(zs) - 1 else "grape",
                    12, Matrix.Translation((cx, cy, 0)), closed=True, twist=lambda r, z: z * 20)
        rotor.lathe([(0.039, 0.69), (0.045, 0.7), (0.045, 0.726), (0.039, 0.735), (0.022, 0.735), (0.022, 0.69)],
                    lambda i, j: "grape" if i == 4 else "lemon", 12, Matrix.Translation((cx, cy, 0)), closed=True)
    rotor.prism(star(0.124, 0.06), 0.03, _frame((x, y, 0.535), (0, 0, 1)), "mint")
    rotor.cylinder((x, y, 0.3), (x, y, 0.69), 0.02, "glow-mint", n=12)
    rotor.sphere((x, y, 0.69), 0.024, "glow-mint", n=12, rings=6)

    # Launcher: mint toy frame, heart core, spare candy rocket, striped finned rocket pod.
    _candy_chassis(launcher, "mint", "pink", "lemon")
    launcher.prism(heart(0.085), 0.034, _frame((0, 0.243, -0.03), (1, 0, 0)), "glow-pink")
    launcher.box((0, 0.205, -0.03), (0.05, 0.02, 0.07), "lemon", bevel=0.008, segments=2)
    rx, ry = -0.118, 0.09
    launcher.lathe([(0.026, -0.07), (0.026, -0.03), (0.026, 0.01), (0.026, 0.05), (0.026, 0.09)],
                   lambda i, j: "pink" if i % 2 else "cream", 14, Matrix.Translation((rx, ry, 0)))
    launcher.cone((rx, ry, 0.09), (rx, ry, 0.15), 0.026, "pink", n=14, tip=0.004)
    for k in range(3):
        a = math.radians(90 + 120 * k)
        launcher.prism([(0.02, -0.075), (0.045, -0.085), (0.045, -0.06), (0.02, -0.035)], 0.008,
                       Matrix.Translation((rx, ry, 0)) @ _rot("Z", math.degrees(a))
                       @ Matrix(((1, 0, 0, 0), (0, 0, -1, 0), (0, 1, 0, 0), (0, 0, 0, 1))), "lemon")
    launcher.box((-0.092, ry, 0.02), (0.03, 0.02, 0.05), "pink", bevel=0.006)
    launcher.cylinder((0, 0.13, 0.17), (0, 0.13, 0.205), 0.07, "lemon", n=24, chamfer=0.006)
    x, y, z = SLIDE_PIVOT
    centre = Matrix.Translation((x, y, 0))
    body = [0.275 + 0.02425 * i for i in range(21)]
    shell = ([(0, 0.212), (0.07, 0.212), (0.115, 0.222), (0.138, 0.245)] + [(0.145, zz) for zz in body]
             + [(0.113, body[-1]), (0.113, 0.47), (0, 0.47)])
    last_stripe = 3 + len(body) - 1

    def pod_key(i, j):
        if i < 3:
            return "lemon"
        if i < last_stripe:
            return "cream" if (j // 4) % 2 else "pink"
        return "grape"

    slide.lathe(shell, pod_key, 32, centre, twist=lambda r, z: z * 7 if r > 0.14 else 0.0)
    slide.torus((x, y, 0.783), (0, 0, 1), 0.133, 0.027, "lemon", n=32, m=10)
    slide.cylinder((x, y, 0.47), (x, y, 0.6), 0.084, "cream", n=20)
    slide.cone((x, y, 0.6), (x, y, 0.745), 0.084, "glow-pink", n=20, tip=0.006)
    for angle in (45, 135, 225, 315):
        a = math.radians(angle)
        radial, axial = Vector((math.cos(a), math.sin(a), 0)), Vector((0, 0, 1))
        m = Matrix((radial, axial, radial.cross(axial))).transposed().to_4x4()
        m.translation = (x, y, 0)
        slide.prism([(0.125, 0.225), (0.2, 0.236), (0.214, 0.25), (0.214, 0.305), (0.2, 0.322), (0.125, 0.43)],
                    0.022, m, "lemon")
    slide.cylinder((x, 0.27, 0.68), (x, 0.305, 0.68), 0.008, "lemon", n=8)
    slide.prism(star(0.036, 0.016), 0.014, _frame((x, 0.322, 0.68), (0, 0, 1)), "glow-mint")
    for side in (-1, 1):
        slide.prism(heart(0.075), 0.012, _frame((side * 0.142, y, 0.56), (side, 0, 0)), "glow-mint")
    return [repeater, rotor, launcher, slide]


# ---------------------------------------------------------------- Scrap Punk

def _scrap_chassis(part):
    """Welded junk frame shared by both guns: taped pipe grip, bolted plates, toxic cell."""
    rake = _rot("X", 8)
    axis = (rake @ Vector((0, 1, 0, 0))).to_3d()
    base = Vector((0, -0.05, -0.012))
    part.cylinder(base - axis * 0.105, base + axis * 0.1, 0.032, "gunmetal", n=10)
    for t, tilt in ((-0.07, 0), (-0.025, 6), (0.022, -4)):
        frame = _rot("X", 8 + tilt)
        tape_axis = (frame @ Vector((0, 1, 0, 0))).to_3d()
        c = base + axis * t
        part.cylinder(c - tape_axis * 0.018, c + tape_axis * 0.018, 0.0355, "tape", n=10)
    part.cylinder(base - axis * 0.105, base - axis * 0.125, 0.042, "bolt", n=6, chamfer=0.004)
    part.box((0, -0.066, 0.075), (0.012, 0.012, 0.088), "gunmetal")
    part.box((0, -0.029, 0.114), (0.012, 0.075, 0.012), "gunmetal")
    part.box((0, -0.018, 0.058), (0.01, 0.04, 0.012), "bolt", rot=_rot("X", -12))
    part.box((0, 0.11, 0.035), (0.15, 0.16, 0.31), "rust", bevel=0.006)
    for side in (-1, 1):
        part.box((side * 0.08, 0.118, 0.03), (0.01, 0.105, 0.2), "steel", bevel=0.002, rot=_rot("X", side * 3))
        for dy in (-0.038, 0.038):
            for dz in (-0.085, 0.085):
                part.bolt((side * 0.086, 0.118 + dy, 0.03 + dz), (side, 0, 0), "bolt", 0.009)
    part.box((0, 0.197, 0.05), (0.11, 0.014, 0.24), "gunmetal", bevel=0.003, rot=_rot("X", -2))
    # Strapped toxic vial on top, so the green reads from the overhead camera.
    part.cylinder((0.034, 0.216, -0.03), (0.034, 0.216, 0.13), 0.013, "glow", n=10)
    for z0, z1, key, radius in ((-0.045, -0.03, "gunmetal", 0.016), (0.13, 0.145, "gunmetal", 0.016),
                                (0.04, 0.062, "tape", 0.0155)):
        part.cylinder((0.034, 0.216, z0), (0.034, 0.216, z1), radius, key, n=10)
    # Taped toxic cell on the right side.
    cell = Matrix.Translation((0.1, 0.115, -0.095))
    profile = [(0, 0), (0.027, 0), (0.032, 0.008), (0.032, 0.03), (0.026, 0.034), (0.026, 0.146),
               (0.032, 0.15), (0.032, 0.172), (0.027, 0.18), (0, 0.18)]
    part.lathe(profile, lambda i, j: "glow" if i == 5 else "gunmetal", 14, cell)
    for dz in (0.065, 0.115):
        part.cylinder((0.1, 0.115, -0.095 + dz - 0.01), (0.1, 0.115, -0.095 + dz + 0.01), 0.0295, "tape", n=14)
    # Pipe skeleton stock with a rubber pad.
    part.cylinder((0, 0.165, -0.1), (0, 0.165, -0.3), 0.015, "gunmetal", n=8)
    part.cylinder((0, 0.055, -0.1), (0, 0.085, -0.3), 0.015, "gunmetal", n=8)
    part.cylinder((0, 0.165, -0.185), (0, 0.165, -0.225), 0.019, "tape", n=8)
    part.box((0, 0.122, -0.315), (0.075, 0.15, 0.04), "soot", bevel=0.01, segments=2)
    part.box((0, 0.212, -0.06), (0.01, 0.025, 0.01), "gunmetal")
    part.torus((0, 0.243, -0.06), (0, 0, 1), 0.02, 0.005, "bolt", n=16, m=6)


def scrap_punk():
    repeater = Part("WildsparkRepeaterReceiver", REPEATER)
    rotor = Part(ROTOR, REPEATER, ROTOR_PIVOT)
    launcher = Part("WildsparkLauncherReceiver", LAUNCHER)
    slide = Part(SLIDE, LAUNCHER, SLIDE_PIVOT)
    x, y, z = ROTOR_PIVOT

    # Repeater: paint-can drum, welded half shroud over mismatched spinning pipes.
    _scrap_chassis(repeater)
    can = _frame((-0.075, 0.095, 0.03), (-1, 0, 0))
    profile = [(0.078, 0.0), (0.078, 0.012), (0.084, 0.016), (0.084, 0.024), (0.078, 0.028), (0.078, 0.07),
               (0.084, 0.074), (0.084, 0.082), (0.078, 0.086), (0.078, 0.094), (0.072, 0.1), (0.0, 0.1)]
    repeater.lathe(profile, lambda i, j: ("hazard" if (j // 2) % 2 else "soot") if i == 10
                   else "bolt" if i in (1, 2, 3, 6, 7, 8) else "gunmetal", 24, can)
    repeater.lathe([(0.079, 0.0), (0.079, 0.012)], "tape", 24, can @ Matrix.Translation((0, 0, 0.04)))
    repeater.tube((x, y, 0.19), (x, y, 0.232), 0.132, 0.116, "gunmetal", n=20)
    for k, angle in enumerate((25, 57.5, 90, 122.5, 155)):
        a = math.radians(angle)
        radial = Vector((math.cos(a), math.sin(a), 0))
        repeater.box((0.124 * radial.x, y + 0.124 * radial.y, 0.35), (0.068, 0.01, 0.25),
                     "rust" if k % 2 else "steel", bevel=0.002, rot=_rot("Z", angle - 90))
        for zz in (0.24, 0.46):
            repeater.bolt((0.13 * radial.x, y + 0.13 * radial.y, zz), radial, "bolt", 0.008)
    repeater.box((0, y + 0.1305, 0.425), (0.058, 0.003, 0.07), "soot")
    for k in range(4):
        repeater.box((0, y + 0.1325, 0.4 + 0.017 * k), (0.05, 0.003, 0.008), "hazard", rot=_rot("Y", 35))
    repeater.sphere((0, y + 0.138, 0.455), 0.008, "glow", n=8, rings=4)
    repeater.box((0, y + 0.133, 0.455), (0.006, 0.01, 0.006), "gunmetal")
    rotor.box((x, y, 0.272), (0.15, 0.15, 0.036), "gunmetal", bevel=0.004, rot=_rot("Z", 45))
    for angle in (0, 90, 180, 270):
        a = math.radians(angle)
        rotor.bolt((0.072 * math.cos(a), y + 0.072 * math.sin(a), 0.292), (0, 0, 1), "bolt", 0.01)
    for k, (radius, tip, key) in enumerate(((0.031, 0.735, "gunmetal"), (0.025, 0.712, "rust"),
                                             (0.029, 0.724, "steel"), (0.024, 0.7, "gunmetal"))):
        a = math.radians(45 + 90 * k)
        cx, cy = 0.058 * math.cos(a), y + 0.058 * math.sin(a)
        rotor.lathe([(radius, 0.29), (radius, tip), (radius - 0.008, tip), (radius - 0.008, 0.29)],
                    lambda i, j, key=key: "soot" if i == 2 else key, 10, Matrix.Translation((cx, cy, 0)), closed=True)
    for z0 in (0.4, 0.615):
        rotor.tube((x, y, z0), (x, y, z0 + 0.018), 0.099, 0.088, "bolt", n=20)
        rotor.box((x, y + 0.101, z0 + 0.009), (0.02, 0.014, 0.024), "bolt")
    rotor.tube((x, y, 0.5), (x, y, 0.545), 0.095, 0.087, "tape", n=20)
    rotor.cylinder((x, y, 0.29), (x, y, 0.69), 0.017, "glow", n=10)

    # Launcher: fuel tank, square sheet-metal tube with hazard band and ladder sight.
    _scrap_chassis(launcher)
    tank = Matrix.Translation((-0.112, 0.1, -0.1))
    profile = [(0, 0), (0.03, 0.004), (0.042, 0.016), (0.045, 0.03), (0.045, 0.08), (0.045, 0.1),
               (0.045, 0.12), (0.045, 0.14), (0.045, 0.19), (0.042, 0.204), (0.03, 0.216), (0, 0.22)]
    launcher.lathe(profile, lambda i, j: {4: "hazard", 5: "soot", 6: "hazard"}.get(i, "steel"), 16, tank)
    launcher.cylinder((-0.112, 0.14, 0.0), (-0.112, 0.162, 0.0), 0.012, "bolt", n=8)
    launcher.box((-0.112, 0.165, 0.0), (0.04, 0.008, 0.01), "gunmetal")
    launcher.tube((-0.112, 0.1, -0.05), (-0.112, 0.1, -0.03), 0.048, 0.044, "tape", n=16)
    x, y, z = SLIDE_PIVOT
    centre = Matrix.Translation((x, y, 0))
    square = math.sqrt(2)
    ro, ri = 0.145 * square, 0.118 * square
    zs = [0.215, 0.33, 0.45, 0.6, 0.77]
    slide.lathe([(ro, zz) for zz in zs] + [(ri, zs[-1]), (ri, zs[0])],
                lambda i, j: ("rust-bright" if (i + j) % 3 == 0 else "rust" if (i + j) % 2 else "steel")
                if i < 4 else "soot" if i == 5 else "gunmetal",
                4, centre, math.pi / 4, closed=True)
    slide.lathe([(0.158 * square, 0.765), (0.158 * square, 0.81), (0.112 * square, 0.81), (0.112 * square, 0.765)],
                "gunmetal", 4, centre, math.pi / 4, closed=True)
    slide.lathe([(0.153 * square, 0.21), (0.153 * square, 0.25), (0.14 * square, 0.25), (0.14 * square, 0.21)],
                "gunmetal", 4, centre, math.pi / 4, closed=True)
    bands = [0.6 + 0.016 * i for i in range(6)]
    slide.lathe([(0.149 * square, zz) for zz in bands] + [(0.143 * square, bands[-1]), (0.143 * square, bands[0])],
                lambda i, j: ("hazard" if i % 2 == 0 else "soot") if i < 5 else "soot", 4, centre, math.pi / 4,
                closed=True)
    slide.lathe([(0.1485 * square, 0.43), (0.1485 * square, 0.47), (0.143 * square, 0.47), (0.143 * square, 0.43)],
                "tape", 4, centre @ _rot("Z", 4), math.pi / 4, closed=True)
    for normal in ((1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0)):
        n = Vector(normal)
        lateral = Vector((0, 0, 1)).cross(n)
        for zz in (0.29, 0.73):
            for side in (-1, 1):
                slide.bolt(Vector((x, y, zz)) + n * 0.147 + lateral * side * 0.085, n, "bolt", 0.011)
    for side in (-1, 1):  # warning signs welded to both sides
        sign = _frame((side * 0.148, y + 0.005, 0.5), (side, 0, 0))
        slide.prism([(0, 0.058), (-0.062, -0.05), (0.062, -0.05)], 0.006, sign, "hazard")
        slide.prism([(-0.006, 0.022), (0.006, 0.022), (0.004, -0.018), (-0.004, -0.018)], 0.009, sign, "soot")
        slide.prism([(-0.006, -0.026), (0.006, -0.026), (0.006, -0.037), (-0.006, -0.037)], 0.009, sign, "soot")
    patch = _rot("Y", 9)
    slide.box((0.02, y + 0.148, 0.36), (0.12, 0.007, 0.1), "steel", bevel=0.002, rot=patch)
    for dx, dz in ((-0.048, -0.038), (0.048, -0.038), (-0.048, 0.038), (0.048, 0.038)):
        offset = (patch @ Vector((dx, 0, dz, 0))).to_3d()
        slide.bolt(Vector((0.02, y + 0.152, 0.36)) + offset, (0, 1, 0), "bolt", 0.008)
    slide.cylinder((x, y, 0.44), (x, y, 0.6), 0.088, "soot", n=16)
    slide.tube((x, y, 0.595), (x, y, 0.615), 0.093, 0.07, "gunmetal", n=16)
    slide.cone((x, y, 0.61), (x, y, 0.78), 0.088, "glow", n=16, tip=0.01)
    for side in (-1, 1):
        slide.box((side * 0.028, 0.305, 0.66), (0.008, 0.06, 0.008), "gunmetal")
    for yy in (0.305, 0.33):
        slide.box((x, yy, 0.66), (0.064, 0.007, 0.007), "bolt")
    slide.sphere((x, 0.346, 0.66), 0.011, "glow", n=10, rings=5)
    for zz in (0.3, 0.46):
        slide.cylinder((x, 0.272, zz), (x, 0.31, zz), 0.012, "gunmetal", n=8)
    slide.cylinder((x, 0.31, 0.29), (x, 0.31, 0.47), 0.014, "tape", n=8, chamfer=0.004)
    return [repeater, rotor, launcher, slide]


BUILDERS = {"candy-neon": candy_neon, "scrap-punk": scrap_punk}


def _material(name, color, metallic, roughness, emission, image=None):
    m = bpy.data.materials.new(name)
    m.diffuse_color = (*color, 1)
    m.metallic, m.roughness = metallic, roughness
    m.use_nodes = True
    tree = m.node_tree
    p = tree.nodes.get("Principled BSDF")
    for key, value in [("Base Color", (*color, 1)), ("Metallic", metallic), ("Roughness", roughness),
                       ("Emission Color", (*color, 1)), ("Emission Strength", min(emission, 1.0))]:
        p.inputs[key].default_value = value
    if image:  # exported as baseColorTexture x baseColorFactor, no extension
        texture = tree.nodes.new("ShaderNodeTexImage")
        texture.image = image
        mix = tree.nodes.new("ShaderNodeMix")
        mix.data_type, mix.blend_type = "RGBA", "MULTIPLY"
        socket = {s.identifier: s for s in [*mix.inputs, *mix.outputs]}
        socket["Factor_Float"].default_value = 1.0
        socket["B_Color"].default_value = (*color, 1)
        tree.links.new(texture.outputs["Color"], socket["A_Color"])
        tree.links.new(socket["Result_Color"], p.inputs["Base Color"])
    return m


def _tiling_noise(rng, size, cells):
    grid = rng.random((cells, cells))
    t = numpy.arange(size) * cells / size
    i0 = numpy.floor(t).astype(int)
    i1 = (i0 + 1) % cells
    f = t - i0
    f = f * f * (3 - 2 * f)
    top = grid[i0][:, i0] * (1 - f)[None, :] + grid[i0][:, i1] * f[None, :]
    bottom = grid[i1][:, i0] * (1 - f)[None, :] + grid[i1][:, i1] * f[None, :]
    return top * (1 - f)[:, None] + bottom * f[:, None]


def _texture(kind, path, size=256):
    """Seeded, tiling sRGB multiplier: rust = dark scale with orange blooms; grime = stains on steel."""
    rng = numpy.random.default_rng(20261011 if kind == "rust" else 7)
    noise = sum(_tiling_noise(rng, size, cells) * weight
                for cells, weight in ((4, 0.45), (8, 0.27), (16, 0.16), (32, 0.08), (64, 0.04)))
    noise = (noise - noise.min()) / (noise.max() - noise.min())
    fine = _tiling_noise(rng, size, 64) * 0.6 + _tiling_noise(rng, size, 32) * 0.4

    def ramp(v, lo, hi):
        x = numpy.clip((v - lo) / (hi - lo), 0, 1)
        return x * x * (3 - 2 * x)

    if kind == "rust":
        base, bloom, pit = numpy.array((0.5, 0.45, 0.42)), numpy.array((0.97, 0.84, 0.72)), 0.45
        mask = ramp(noise, 0.48, 0.74)
    else:
        base, bloom, pit = numpy.array((0.97, 0.96, 0.94)), numpy.array((0.92, 0.6, 0.42)), 0.4
        mask = ramp(noise, 0.55, 0.75) * 0.85
    color = base[None, None, :] * (1 - mask[..., None]) + bloom[None, None, :] * mask[..., None]
    color *= (1 - ramp(fine, 0.68, 0.85) * pit)[..., None] * (0.9 + 0.1 * fine)[..., None]
    pixels = numpy.ones((size, size, 4), dtype=numpy.float32)
    pixels[..., :3] = color.clip(0, 1)
    image = bpy.data.images.new(f"wildspark {kind}", size, size, alpha=False)
    image.pixels[:] = pixels.ravel()
    image.filepath_raw, image.file_format = str(path), "PNG"
    image.save()
    image = bpy.data.images.load(str(path))
    image.name = f"wildspark {kind}"
    image.pack()
    return image


def blender_build(theme, output):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    spec = THEMES[theme]
    textures = {key: _texture(kind, Path(output).with_name(f"{kind}.png"))
                for kind, keys in spec.get("textures", {}).items() for key in keys}
    materials = {key: _material(f"{spec['title']} | {label}", color, metallic, roughness, glow, textures.get(key))
                 for key, (label, color, metallic, roughness, glow) in spec["materials"].items()}
    root = bpy.data.objects.new(SET_ROOT, None)
    scene.collection.objects.link(root)
    owners = {}
    for name in (REPEATER, LAUNCHER):
        owners[name] = bpy.data.objects.new(name, None)
        scene.collection.objects.link(owners[name])
        owners[name].parent = root
    uv_scale = 5.0 if textures else None
    parts = [part.finish(materials, owners[part.owner], uv_scale) for part in BUILDERS[theme]()]
    names = {o.name for o in scene.objects}
    expected = {SET_ROOT, REPEATER, LAUNCHER, ROTOR, SLIDE}
    if not expected <= names or any(re.search(r"\.\d{3}$", n) for n in names):
        raise RuntimeError(f"unexpected object names {sorted(names)}")
    bpy.ops.export_scene.gltf(filepath=str(output), export_format="GLB", use_selection=False,
                              export_yup=True, export_apply=True, export_animations=False, export_skins=False,
                              export_morph=False, export_cameras=False, export_lights=False, export_extras=False,
                              export_draco_mesh_compression_enable=False, export_materials="EXPORT")
    print("WILDSPARK_SKIN " + json.dumps({"blender": bpy.app.version_string, "objects": len(scene.objects),
                                          "vertices": sum(len(o.data.vertices) for o in parts)}))


def _look(camera, target, direction, distance):
    camera.location = Vector(target) + Vector(direction).normalized() * distance
    camera.rotation_euler = (Vector(target) - camera.location).to_track_quat("-Z", "Y").to_euler()


def _frame_ortho(camera, objects, aspect, padding):
    bpy.context.view_layer.update()
    corners = [o.matrix_world @ Vector(c) for o in objects for c in o.bound_box]
    lo = Vector([min(c[i] for c in corners) for i in range(3)])
    hi = Vector([max(c[i] for c in corners) for i in range(3)])
    center = (lo + hi) / 2
    _look(camera, center, camera["direction"], (hi - lo).length * 3)
    bpy.context.view_layer.update()
    inverse = camera.matrix_world.inverted()
    local = [inverse @ c for c in corners]
    width = max(c.x for c in local) - min(c.x for c in local)
    height = max(c.y for c in local) - min(c.y for c in local)
    shift = Vector(((max(c.x for c in local) + min(c.x for c in local)) / 2,
                    (max(c.y for c in local) + min(c.y for c in local)) / 2, 0))
    camera.location = camera.matrix_world @ shift
    camera.data.ortho_scale = max(width, height * aspect) * padding


def _composite(path, backdrop):
    """Alpha-over the transparent render onto a soft radial backdrop (no compositor nodes)."""
    image = bpy.data.images.load(str(path))
    w, h = image.size
    pixels = numpy.array(image.pixels[:], dtype=numpy.float32).reshape(h, w, 4)
    ys, xs = numpy.mgrid[0:h, 0:w]
    d = numpy.sqrt(((xs - w / 2) / (w / 2)) ** 2 + ((ys - h * 0.55) / (h / 2)) ** 2).clip(0, 1.4) / 1.4
    inner, outer = numpy.array(backdrop[0]), numpy.array(backdrop[1])
    background = inner[None, None, :] * (1 - d[..., None]) + outer[None, None, :] * d[..., None]
    alpha = pixels[..., 3:4]
    out = numpy.ones_like(pixels)
    out[..., :3] = pixels[..., :3] * alpha + background * (1 - alpha)
    result = bpy.data.images.new(Path(path).stem + "-final", w, h, alpha=False)
    result.pixels[:] = out.ravel()
    result.filepath_raw = str(path)
    result.file_format = "PNG"
    result.save()


def blender_render(theme, glb, output_dir):
    spec = THEMES[theme]
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=str(glb))
    scene = bpy.context.scene
    objects = {o.name: o for o in scene.objects}
    repeater, launcher = objects[REPEATER], objects[LAUNCHER]
    if tuple(launcher.scale) != (0, 0, 0):
        raise RuntimeError("launcher variant must be hidden (scale 0) in the shipped file")
    launcher.scale = (1, 1, 1)
    meshes = {kind: [o for o in owner.children_recursive if o.type == "MESH"]
              for kind, owner in (("repeater", repeater), ("launcher", launcher))}
    scene.render.engine = "BLENDER_EEVEE"
    scene.eevee.taa_render_samples = 64
    scene.render.film_transparent = True
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA"
    scene.view_settings.view_transform = "Standard"
    scene.view_settings.look = "None"
    world = bpy.data.worlds.new("studio")
    world.use_nodes = True
    world.node_tree.nodes["Background"].inputs["Color"].default_value = (0.34, 0.36, 0.4, 1)
    world.node_tree.nodes["Background"].inputs["Strength"].default_value = 0.55
    scene.world = world
    for name, location, energy, color, size in [
            ("key", (-2.2, -2.0, 2.6), 520, (1.0, 0.97, 0.93), 2.2),
            ("fill", (2.4, -1.2, 0.9), 160, (0.82, 0.88, 1.0), 2.5),
            ("rim", (0.6, 2.6, 1.8), 480, spec["rim"], 1.4)]:
        light = bpy.data.lights.new(name, "AREA")
        light.energy, light.color, light.size = energy, color, size
        obj = bpy.data.objects.new(name, light)
        scene.collection.objects.link(obj)
        _look(obj, (0, 0, 0.08), location, Vector(location).length)
    camera = bpy.data.objects.new("camera", bpy.data.cameras.new("camera"))
    camera.data.type = "ORTHO"
    camera["direction"] = (-0.82, -0.62, 0.48)
    scene.collection.objects.link(camera)
    scene.camera = camera
    written = []

    def shot(name, size, visible, padding=1.14):
        scene.render.resolution_x, scene.render.resolution_y = size
        for kind, objs in meshes.items():
            for o in objs:
                o.hide_render = kind not in visible
        _frame_ortho(camera, [o for kind in visible for o in meshes[kind]], size[0] / size[1], padding)
        scene.render.filepath = str(Path(output_dir) / f"{name}.png")
        bpy.ops.render.render(write_still=True)
        _composite(scene.render.filepath, spec["backdrop"])
        written.append(name)

    shot("repeater", IMAGES["repeater"], ["repeater"])
    shot("launcher", IMAGES["launcher"], ["launcher"])
    # Cover: both guns, repeater above-left, launcher below-right of the same view.
    forward = -Vector(camera["direction"]).normalized()
    right = forward.cross(Vector((0, 0, 1))).normalized()
    repeater.location = right * -0.12 + Vector((0, 0, 0.27))
    launcher.location = right * 0.12 + Vector((0, 0, -0.27))
    shot("cover", IMAGES["cover"], ["repeater", "launcher"], 1.08)
    print("WILDSPARK_SKIN " + json.dumps({"rendered": written}))


def blender_main(argv):
    parser = argparse.ArgumentParser(prog="build_wildspark_skin.py (Blender)")
    sub = parser.add_subparsers(dest="command", required=True)
    b = sub.add_parser("blender-build")
    b.add_argument("--theme", choices=list(THEMES), required=True)
    b.add_argument("--output", type=Path, required=True)
    r = sub.add_parser("blender-render")
    r.add_argument("--theme", choices=list(THEMES), required=True)
    r.add_argument("--glb", type=Path, required=True)
    r.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args(argv)
    if args.command == "blender-build":
        blender_build(args.theme, args.output)
    else:
        blender_render(args.theme, args.glb, checked_output_dir(args.output_dir))


if __name__ == "__main__":
    if bpy is not None:
        blender_main(sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else [])
    else:
        sys.exit(main())
