#!/usr/bin/env python3
"""Build the conformance fixtures of OMOBA's game contract profile (v1), deterministically.

The profile fixtures of `omoba.character.v1` (Ekza Game Protocol 1.0; the protocol's core
fixtures live in its own repository). Every implementation of the contract's rules runs
them: Studio's TypeScript checks, OMOBA's passport (`validate-motion-pack`, C3), the Hub
registry (`pack.*` only). Each package case is a small synthetic VRM (a 15-bone T-pose rig,
17 bones for VRM 0.x) with a few clips and an `ekza_package_v1` manifest, built from a valid
baseline with one change, so the expected verdict follows from the construction; no rule
is evaluated here. Contract cases are documents a reader must accept or refuse.

    python3 scripts/build_game_contract_fixtures.py            # rebuild
    python3 scripts/build_game_contract_fixtures.py --check    # verify the committed files
    python3 scripts/build_game_contract_fixtures.py --refreeze # new fixture contract (v-bump)

`contract.json` is frozen: packages name its revision, so a change of the live contract
does not move the fixtures. `--refreeze` cuts it again from the committed contract.
Standard library only; CC0-1.0. Reference: docs/game-contract.md.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import math
from pathlib import Path
import struct
import sys

from export_asset_catalog import CatalogError, canonical, load_json
from export_game_contract import (ContractError, PACKAGE_SCHEMA_FILE, ROOT, read_contract, render, revision_of,
                                  schema_errors)

FIXTURES = "shared/assets/catalog/game-contract-fixtures/v1"
CONTRACT = "shared/assets/catalog/game-contract.json"
TARGETS = ("any", "warden", "dawnweaver", "wildspark")
GENERATOR = "omoba-bevy scripts/build_game_contract_fixtures.py"
PROFILE = "omoba.character.v1"
LEG = 0.9      # hips (0.98 m) above the feet (0.08 m): the unit of the in-place limits
HZ = 30


def digest(data):
    return hashlib.sha256(data).hexdigest()


# ---------------------------------------------------------------------------------------
# Fixture contracts
# ---------------------------------------------------------------------------------------

def freeze(live):
    """The fixture contract: the live contract cut to four targets, and a retired revision."""
    targets = [target for target in live["targets"] if target["id"] in TARGETS]
    keep = {slot for target in targets for slot in target["slots"]} | {"base.walk"}
    trimmed = copy.deepcopy(live)
    trimmed["targets"] = copy.deepcopy(targets)
    trimmed["slots"] = {}
    for slot_id, slot in live["slots"].items():
        if slot_id in keep:
            slot = copy.deepcopy(slot)
            for link in ("sharesDefaultWith", "alternatesWith"):
                slot[link] = [other for other in slot[link] if other in keep]
            slot["usedBy"] = [user for user in slot["usedBy"] if user == "*" or user in TARGETS]
            trimmed["slots"][slot_id] = slot
    retired = copy.deepcopy(trimmed)
    retired["previousRevision"] = None
    for rule in retired["rules"]:
        if rule["id"] == "clip.in-place":
            rule["params"]["maxHorizontal"] = 0.2
    retired["revision"] = revision_of(retired)
    trimmed["previousRevision"] = retired["revision"]
    trimmed["revision"] = revision_of(trimmed)
    return trimmed, retired


def contract_cases(contract):
    """Documents a reader must accept or refuse (GAME-PROTOCOL-SPEC §3.3, §3.5), each with one
    change; `expect` is `ok` or `refused: <reader check>`."""
    def case(name, expect, mutate, keep_revision=False):
        document = copy.deepcopy(contract)
        mutate(document)
        if not keep_revision:
            document["revision"] = revision_of(document)
        return name, expect, document

    def newer_minor(document):
        document["schemaVersion"] = "1.9"
        document["slots"]["base.run"]["animation"]["footsteps"] = ["footstep_l", "footstep_r"]

    def foreign_profile(document, required):
        document["profiles"]["used"].append("acme.extra.v1")
        if required:
            document["profiles"]["required"].append("acme.extra.v1")
        document["slots"]["base.run"]["ext"]["acme.extra.v1"] = {"tempo": 1}

    def offer_without_packaging(document):
        target = next(t for t in document["targets"] if t["id"] == "wildspark")
        target["accepts"].append({"package": "asset-pack", "requirementId": "wildspark.handheld.set",
                                  "label": "Wildspark gear set", "slots": [], "families": [],
                                  "listing": "filled-slots-only", "mustFill": []})

    def dangling_slot(document):
        target = next(t for t in document["targets"] if t["id"] == "wildspark")
        target["sections"][-1]["slots"].append("skill.not_a_skill.release")
        target["slots"].append("skill.not_a_skill.release")

    return [
        ("contract-ok-fixture", "ok", copy.deepcopy(contract)),
        case("contract-schema-1.9-min-reader-1.0", "ok", newer_minor),
        case("contract-unknown-namespaced-rule", "ok", lambda d: d["rules"].append(
            {"id": "omoba.clip.foot-slide", "severity": "warning", "scope": "animation", "params": {},
             "checkedBy": ["tool", "builder"]})),
        case("contract-unknown-profile-ext", "ok", lambda d: foreign_profile(d, False)),
        case("contract-unknown-key", "refused: schema", lambda d: d["slots"]["base.run"].update(tempo=1)),
        case("contract-revision-mismatch", "refused: revision",
             lambda d: d["slots"]["base.run"]["animation"]["window"].update(maxSecs=1.1), keep_revision=True),
        case("contract-dangling-slot-ref", "refused: reference", dangling_slot),
        case("contract-offer-kind-mismatch", "refused: reference", offer_without_packaging),
        case("contract-required-unknown-profile", "refused: profile", lambda d: foreign_profile(d, True)),
        case("contract-unknown-core-rule", "refused: unknown-rule", lambda d: d["rules"].append(
            {"id": "clip.foot-slide", "severity": "error", "scope": "animation", "params": {},
             "checkedBy": ["tool", "builder", "game"]})),
        case("contract-min-reader-1.9", "refused: reader-version",
             lambda d: d.update(schemaVersion="1.9", minReaderVersion="1.9")),
        case("contract-bad-profile-ext", "refused: profile",
             lambda d: d["slots"]["base.run"]["ext"][PROFILE].update(phase="sprint")),
    ]


# ---------------------------------------------------------------------------------------
# Synthetic VRM packages
# ---------------------------------------------------------------------------------------

def rig(vrm=1, hips_root=False, accessory=False):
    """Bones of a T-pose rig: (name, parent, world position). VRM 1.0 faces +Z with the
    character's left at +X; VRM 0.x faces -Z with its left at -X."""
    left = 1.0 if vrm == 1 else -1.0
    torso = "spine" if vrm == 1 else "chest"
    bones = [("hips", None if hips_root else "Root", (0.0, 0.98, 0.0)),
             ("spine", "hips", (0.0, 1.13, 0.0))]
    if vrm == 0:
        bones += [("chest", "spine", (0.0, 1.28, 0.0)), ("neck", "chest", (0.0, 1.45, 0.0))]
    bones.append(("head", "spine" if vrm == 1 else "neck", (0.0, 1.55, 0.0)))
    for side, sign in (("left", left), ("right", -left)):
        bones += [(f"{side}UpperArm", torso, (sign * 0.18, 1.42, 0.0)),
                  (f"{side}LowerArm", f"{side}UpperArm", (sign * 0.46, 1.42, 0.0)),
                  (f"{side}Hand", f"{side}LowerArm", (sign * 0.72, 1.42, 0.0))]
    for side, sign in (("left", left), ("right", -left)):
        bones += [(f"{side}UpperLeg", "hips", (sign * 0.09, 0.93, 0.0)),
                  (f"{side}LowerLeg", f"{side}UpperLeg", (sign * 0.09, 0.52, 0.0)),
                  (f"{side}Foot", f"{side}LowerLeg", (sign * 0.09, 0.08, 0.0))]
    extra = [("Accessory", "head", (0.0, 1.7, 0.05))] if accessory else []
    return bones, extra


def quat(axis, degrees):
    half = math.radians(degrees) / 2
    s = math.sin(half)
    return [s if axis == "x" else 0.0, s if axis == "y" else 0.0, s if axis == "z" else 0.0, math.cos(half)]


class Clip:
    """One custom clip: `kind` loop (closes) or action; changes name the case's defect."""

    def __init__(self, name, kind="action", duration=0.4, hz=HZ, contact=0.1, meta_contact="same", **change):
        self.name, self.kind, self.duration, self.hz = name, kind, duration, hz
        self.contact = contact
        self.meta_contact = contact if meta_contact == "same" else meta_contact
        self.change = change

    def times(self):
        count = round(self.duration * self.hz) + 1
        times = [index / self.hz for index in range(count)]
        times[-1] = self.duration
        if "keys" in self.change:
            count = self.change["keys"]
            times = [self.duration * index / (count - 1) for index in range(count)]
        if "starts_at" in self.change:
            times = [time + self.change["starts_at"] for time in times]
        if self.change.get("unsorted"):
            times[3], times[4] = times[4], times[3]
        return times

    def channels(self, hips_rest):
        """[(node, path, values, interpolation)] over `times()`."""
        times = self.times()
        count, end = len(times), times[-1]
        phase = [index / (count - 1) for index in range(count)]
        if self.kind == "loop":
            arm = [quat("z", 30 * math.sin(2 * math.pi * p)) for p in phase]
            legs = [quat("x", 20 * math.sin(2 * math.pi * p)) for p in phase]
            arm[-1], legs[-1] = list(arm[0]), list(legs[0])
            if "seam_degrees" in self.change:
                arm[-1] = quat("z", self.change["seam_degrees"])
        else:
            arm = [quat("z", -60 * math.sin(math.pi * p)) for p in phase]
            legs = [quat("x", 10 * math.sin(math.pi * p)) for p in phase]
        drift = self.change.get("hips_drift", 0.0) * LEG
        hips = [[hips_rest[0] + drift * (1 - abs(2 * p - 1)), hips_rest[1], hips_rest[2]] for p in phase]
        if "nan" in self.change:
            arm[2] = [arm[2][0], arm[2][1], float("nan"), arm[2][3]]
        if "quat_scale" in self.change:
            arm[2] = [component * self.change["quat_scale"] for component in arm[2]]
        interpolation = "CUBICSPLINE" if self.change.get("cubic") else "LINEAR"
        if interpolation == "CUBICSPLINE":
            zero = [0.0, 0.0, 0.0, 0.0]
            arm = [value for key in arm for value in (zero, key, zero)]
        channels = [("hips", "translation", hips, "LINEAR"),
                    ("rightUpperArm", "rotation", arm, interpolation),
                    ("leftUpperLeg", "rotation", legs, "LINEAR"),
                    ("rightUpperLeg", "rotation", [quat("x", -q) for q in
                                                   (2 * math.degrees(math.asin(l[0])) for l in legs)], "LINEAR")]
        for node, path in self.change.get("extra_channels", ()):
            if path == "rotation":
                channels.append((node, path, [quat("y", 5 * math.sin(math.pi * p)) for p in phase], "LINEAR"))
            elif path == "scale":
                channels.append((node, path, [[1.0, 1.0, 1.0] for _ in phase], "LINEAR"))
            elif path == "translation":
                channels.append((node, path, [[0.0, 0.15, 0.0] for _ in phase], "LINEAR"))
            elif path == "weights":
                channels.append((node, path, [[0.5 * math.sin(math.pi * p)] for p in phase], "LINEAR"))
        for node in self.change.get("bulk_channels", ()):
            channels.append((node, "rotation", [[0.0, 0.0, 0.0, 1.0] for _ in phase], "LINEAR"))
        return times, channels, end


class Buffer:
    def __init__(self):
        self.data = bytearray()
        self.views, self.accessors = [], []

    def add(self, payload, component, kind, count, bounds=None):
        while len(self.data) % 4:
            self.data.append(0)
        self.views.append({"buffer": 0, "byteOffset": len(self.data), "byteLength": len(payload)})
        self.data += payload
        accessor = {"bufferView": len(self.views) - 1, "componentType": component, "count": count, "type": kind}
        if bounds:
            accessor["min"], accessor["max"] = bounds
        self.accessors.append(accessor)
        return len(self.accessors) - 1

    def floats(self, rows, kind, bounds=False):
        flat = [value for row in rows for value in (row if isinstance(row, (list, tuple)) else [row])]
        payload = struct.pack(f"<{len(flat)}f", *flat)
        width = {"SCALAR": 1, "VEC3": 3, "VEC4": 4, "MAT4": 16}[kind]
        limits = None
        if bounds:
            columns = [flat[index::width] for index in range(width)]
            limits = ([min(c) for c in columns], [max(c) for c in columns])
        return self.add(payload, 5126, kind, len(flat) // width, limits)


def package(case, contract_revision):
    """The GLB bytes of a package case."""
    vrm = case.get("vrm", 1)
    bones, extra = rig(vrm, case.get("hips_root", False), case.get("accessory", False))
    world = {name: position for name, _, position in bones + extra}
    world["Root"] = (0.0, 0.0, 0.0)
    order = ([] if case.get("hips_root") else ["Root"]) + [name for name, _, _ in bones + extra] + ["Body"]
    index = {name: position for position, name in enumerate(order)}
    parent = {name: up for name, up, _ in bones + extra}
    nodes = []
    for name in order:
        node = {"name": name}
        if name == "Body":
            node.update(mesh=0, skin=0)
        elif name != "Root":
            up = parent[name]
            base = world[up] if up else (0.0, 0.0, 0.0)
            node["translation"] = [round(a - b, 6) for a, b in zip(world[name], base)]
        children = [index[child] for child, up, _ in bones + extra if up == name]
        if name == "Root":
            children.append(index["Body"])
        if children:
            node["children"] = children
        nodes.append(node)
    roots = [index["Root"]] if "Root" in index else [index["hips"], index["Body"]]

    buffer = Buffer()
    triangle = [[0.0, 0.9, 0.0], [0.1, 1.0, 0.0], [-0.1, 1.0, 0.0]]
    attributes = {"POSITION": buffer.floats(triangle, "VEC3", bounds=True)}
    attributes["JOINTS_0"] = buffer.add(bytes([0, 0, 0, 0] * 3), 5121, "VEC4", 3)
    attributes["WEIGHTS_0"] = buffer.floats([[1.0, 0.0, 0.0, 0.0]] * 3, "VEC4")
    primitive = {"attributes": attributes, "mode": 4}
    mesh = {"name": "Body", "primitives": [primitive]}
    if case.get("morph"):
        primitive["targets"] = [{"POSITION": buffer.floats([[0.0, 0.05, 0.0]] * 3, "VEC3", bounds=True)}]
        mesh["weights"] = [0.0]
    joints = [index[name] for name, _, _ in bones]
    matrices = [[1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, -world[name][0], -world[name][1], -world[name][2], 1]
                for name, _, _ in bones]
    skin = {"joints": joints, "inverseBindMatrices": buffer.floats(matrices, "MAT4")}

    hips_rest = list(world["hips"]) if case.get("hips_root") else [0.0, 0.98, 0.0]
    animations, metas = [], []
    for clip_index, clip in enumerate(case.get("clips", [])):
        times, channels, end = clip.times(), *clip.channels(hips_rest)[1:]
        times_accessor = buffer.floats(times, "SCALAR", bounds=True)
        animation = {"name": clip.name, "channels": [], "samplers": []}
        for node, path, values, interpolation in channels:
            kind = {"rotation": "VEC4", "translation": "VEC3", "scale": "VEC3", "weights": "SCALAR"}[path]
            output = buffer.floats(values, kind)
            animation["samplers"].append({"input": times_accessor, "interpolation": interpolation, "output": output})
            animation["channels"].append({"sampler": len(animation["samplers"]) - 1,
                                          "target": {"node": index[node], "path": path}})
        animations.append(animation)
        metas.append({"index": clip_index, "name": clip.name, "slot": clip.name, "loop": clip.kind == "loop",
                      "fps": clip.hz, "durationSeconds": round(end, 6), "rootMotion": "in-place",
                      "contactSeconds": clip.meta_contact, "tags": [], "derivedFrom": [], "redistributable": "yes"})

    if vrm == 1:
        human = {name: {"node": index[name]} for name, _, _ in bones}
        extension = {"VRMC_vrm": {"specVersion": "1.0", "meta": {
            "name": f"Contract fixture {case['id']}", "version": "1", "authors": ["Open Moba contributors"],
            "licenseUrl": "https://vrm.dev/licenses/1.0/", "avatarPermission": "everyone",
            "commercialUsage": "corporation", "creditNotation": "unnecessary", "allowRedistribution": True,
            "modification": "allowModificationRedistribution",
            "otherLicenseUrl": "https://creativecommons.org/publicdomain/zero/1.0/"},
            "humanoid": {"humanBones": human}}}
    else:
        extension = {"VRM": {"specVersion": "0.0", "exporterVersion": GENERATOR, "meta": {
            "title": f"Contract fixture {case['id']}", "version": "1", "author": "Open Moba contributors",
            "allowedUserName": "Everyone", "violentUssageName": "Allow", "sexualUssageName": "Allow",
            "commercialUssageName": "Allow", "licenseName": "CC0"},
            "humanoid": {"humanBones": [{"bone": name, "node": index[name], "useDefaultValues": True}
                                        for name, _, _ in bones]}}}

    extras = {}
    if case.get("manifest") is not None:
        manifest = copy.deepcopy(case["manifest"])
        manifest.setdefault("contract", {"schema": "ekza.game-contract.v1", "game": "omoba",
                                         "revision": contract_revision})
        extras["ekza_package_v1"] = {key: manifest[key] for key in
                                     ("schema", "schemaVersion", "kind", "contract", "target", "slots", "requires",
                                      "provenance")}
    if metas:
        extras["ekza_animation_v1"] = {"schema": 1, "rig": "vrm-humanoid", "clips": metas, "provenance": {
            "tool": "ekza-animator", "toolVersion": "0.0.0-fixture", "projectSha256": None, "baseSha256": None,
            "derivedFrom": []}}
    asset = {"version": "2.0", "generator": GENERATOR}
    if extras:
        asset["extras"] = extras
    gltf = {"asset": asset, "extensionsUsed": list(extension), "extensions": extension, "scene": 0,
            "scenes": [{"nodes": roots}], "nodes": nodes, "meshes": [mesh], "skins": [skin]}
    if animations:
        gltf["animations"] = animations
    gltf["accessors"] = buffer.accessors
    gltf["bufferViews"] = buffer.views
    while len(buffer.data) % 4:
        buffer.data.append(0)
    gltf["buffers"] = [{"byteLength": len(buffer.data)}]
    text = json.dumps(gltf, separators=(",", ":"), ensure_ascii=False).encode()
    text += b" " * (-len(text) % 4)
    body = struct.pack("<I4s", len(text), b"JSON") + text + struct.pack("<I4s", len(buffer.data), b"BIN\0") + buffer.data
    return struct.pack("<4sII", b"glTF", 2, 12 + len(body)) + body


# ---------------------------------------------------------------------------------------
# The cases (Appendix D of the spec), each the baseline plus one change
# ---------------------------------------------------------------------------------------

WILDSPARK = ("base.idle", "base.run", "base.death", "basic.wildspark.0", "basic.wildspark.rockets.0",
             "skill.wild_switch.release", "skill.wild_zap.release", "skill.wild_traps.release",
             "skill.wild_rocket.release")


def manifest(custom, target="wildspark", view=WILDSPARK, requirement=None, **change):
    """An `ekza_package_v1` manifest of a character package listing `view`; `custom` maps
    slot -> (animation index, contact seconds or None)."""
    slots = {}
    for slot in view:
        if slot in custom:
            animation, contact = custom[slot]
            slots[slot] = {"mode": "custom", "animation": animation,
                           "markers": {} if contact is None else {"contact": contact}}
        else:
            slots[slot] = {"mode": "default"}
    result = {"schema": "ekza.package.v1", "schemaVersion": "1.0", "kind": "character",
              "target": {"id": target, "requirementId": requirement or f"character.{target}"}, "slots": slots,
              "requires": [], "provenance": {"tool": "omoba-fixtures", "toolVersion": "1"}}
    result.update(change)
    return result


def baseline(run=None, basic=None, zap=None):
    return [run or Clip("base.run", "loop", 0.6, contact=None),
            basic or Clip("basic.wildspark.0", duration=0.4, contact=0.1),
            zap or Clip("skill.wild_zap.release", duration=0.5, contact=0.12)]


THREE = {"base.run": (0, None), "basic.wildspark.0": (1, 0.1), "skill.wild_zap.release": (2, 0.12)}
PASS3 = {"base.run": "pass", "basic.wildspark.0": "pass", "skill.wild_zap.release": "pass"}


def verdict(pack="ok", slots=None, warnings=(), slot_warnings=None):
    return {"pack": pack, "warnings": list(warnings), "slots": slots if pack == "ok" else {},
            "slotWarnings": slot_warnings or {}}


def with_slot(slots, slot, result):
    return {**slots, slot: result}


def cases(contract, retired):
    dawnweaver = next(target for target in contract["targets"] if target["id"] == "dawnweaver")["slots"]
    many = {slot: (0, None) for slot in WILDSPARK}
    many.update({f"skill.fixture_{index:02d}.release": (0, None) for index in range(49 - len(WILDSPARK))})
    # 33000 keys on 8 rotation channels and the hips: 4.53 MiB of accessor data.
    bulk = Clip("base.run", "loop", 0.6, contact=None, keys=33_000,
                bulk_channels=("spine", "head", "leftUpperArm", "rightLowerArm", "leftLowerLeg"))
    rows = [
        ("ok-vrm1-three-slots", None, "VRM 1.0 rig; custom run, basic attack and Shockline",
         dict(clips=baseline(), manifest=manifest(THREE)), verdict(slots=PASS3)),
        ("ok-vrm0-three-slots", None, "the same on a VRM 0.x rig (facing -Z, 17 bones)",
         dict(vrm=0, clips=baseline(), manifest=manifest(THREE)), verdict(slots=PASS3)),
        ("ok-shared-animation", None, "one animation serves the rocket basic attack and Last Spark",
         dict(clips=baseline() + [Clip("basic.wildspark.rockets.0", duration=0.4, contact=0.13)],
              manifest=manifest({**THREE, "basic.wildspark.rockets.0": (3, 0.13),
                                 "skill.wild_rocket.release": (3, 0.13)})),
         verdict(slots={**PASS3, "basic.wildspark.rockets.0": "pass", "skill.wild_rocket.release": "pass"})),
        ("ok-all-default", None, "manifest only: every slot plays OMOBA's default",
         dict(clips=[], manifest=manifest({})), verdict(slots={})),
        ("pack-missing-manifest", "pack.manifest", "clips without asset.extras.ekza_package_v1",
         dict(clips=baseline(), manifest=None), verdict(["pack.manifest"])),
        ("pack-bad-schema", "pack.manifest", "manifest schema ekza.package.v2",
         dict(clips=baseline(), manifest=manifest(THREE, schema="ekza.package.v2")), verdict(["pack.manifest"])),
        ("pack-unknown-revision", "pack.contract", "a revision no registry lists",
         dict(clips=baseline(), manifest=manifest(THREE, contract={
             "schema": "ekza.game-contract.v1", "game": "omoba",
             "revision": "sha256-" + digest(b"unknown revision")})), verdict(["pack.contract"])),
        ("pack-retired-revision", "pack.contract", "made against contract-retired.json",
         dict(clips=baseline(), manifest=manifest(THREE, contract={
             "schema": "ekza.game-contract.v1", "game": "omoba", "revision": retired["revision"]})),
         verdict(["pack.contract"])),
        ("pack-unknown-slot", "pack.slot-known", "a custom slot the target does not have",
         dict(clips=baseline(), manifest=manifest({**THREE, "skill.wild_dance.release": (2, 0.12)},
                                                   view=WILDSPARK + ("skill.wild_dance.release",))),
         verdict(["pack.slot-known"])),
        ("pack-slot-not-listed", "pack.slot-known", "Snapline is missing from the manifest (every-customizable-slot)",
         dict(clips=baseline(), manifest=manifest(THREE, view=tuple(s for s in WILDSPARK
                                                                      if s != "skill.wild_traps.release"))),
         verdict(["pack.slot-known"])),
        ("pack-windup-custom", "pack.slot-known", "Dawn Ray's wind-up opens in profile 1.1, not 1",
         dict(clips=[Clip("base.run", "loop", 0.6, contact=None),
                     Clip("skill.dawn_ray.windup", "loop", 1.0, contact=None)],
              manifest=manifest({"base.run": (0, None), "skill.dawn_ray.windup": (1, None)}, target="dawnweaver",
                                view=tuple(dawnweaver))),
         verdict(["pack.slot-known"])),
        ("pack-unknown-target", "pack.target", "a target the contract does not have",
         dict(clips=baseline(), manifest=manifest(THREE, target="wildsparkle")), verdict(["pack.target"])),
        ("pack-gear-requirement-on-character", "pack.target",
         "a character package naming the gear requirement wildspark.handheld.set",
         dict(clips=baseline(), manifest=manifest(THREE, requirement="wildspark.handheld.set")),
         verdict(["pack.target"])),
        ("pack-bad-index", "pack.ref", "the basic attack names animation 7 of 3",
         dict(clips=baseline(), manifest=manifest({**THREE, "basic.wildspark.0": (7, 0.1)})),
         verdict(["pack.ref"])),
        ("pack-name-mismatch", "pack.ref", "the basic attack's animation is named attack",
         dict(clips=baseline(basic=Clip("attack", duration=0.4, contact=0.1)), manifest=manifest(THREE)),
         verdict(["pack.ref"])),
        ("pack-duplicate-names", "pack.ref", "two animations named skill.wild_zap.release",
         dict(clips=baseline() + [Clip("skill.wild_zap.release", duration=0.5, contact=0.12)],
              manifest=manifest(THREE)), verdict(["pack.ref"])),
        ("pack-unreferenced-animation", "pack.unreferenced", "an animation no slot names (warning)",
         dict(clips=baseline() + [Clip("spare_wave", duration=0.5, contact=0.1)], manifest=manifest(THREE)),
         verdict(slots=PASS3, warnings=["pack.unreferenced"])),
        ("pack-49-custom-slots", "pack.bytes", "49 custom slots (packaging.character.maxFills is 48; checked before the slots)",
         dict(clips=[Clip("base.run", "loop", 0.6, contact=None)],
              manifest=manifest(many, view=tuple(many))), verdict(["pack.bytes"])),
        ("pack-animation-bytes-over", "pack.bytes", "a custom clip with 4.53 MiB of accessor data (the cap is 4 MiB)",
         dict(clips=[bulk], manifest=manifest({"base.run": (0, None)})), verdict(["pack.bytes"])),
    ]

    def clip_case(name, rule, note, slot, clips, result="fail", custom=THREE, **rig):
        expected = with_slot(PASS3, slot, [rule] if result == "fail" else "pass") if result != "warn" else PASS3
        warnings = {slot: [rule]} if result == "warn" else {}
        if custom is not THREE:
            expected = {key: expected.get(key, "pass") for key in custom}
        rows.append((name, rule, note, dict(clips=clips, manifest=manifest(custom), **rig),
                     verdict(slots=expected, slot_warnings=warnings)))

    basic = "basic.wildspark.0"
    clip_case("clip-scale-channel", "clip.channels", "a scale channel on the hips", "base.run",
              baseline(run=Clip("base.run", "loop", 0.6, contact=None, extra_channels=[("hips", "scale")])))
    clip_case("clip-morph-weights", "clip.channels", "a morph weights channel on the mesh", basic,
              baseline(basic=Clip(basic, extra_channels=[("Body", "weights")])), morph=True)
    clip_case("clip-root-node", "clip.channels", "a rotation channel on the scene root", basic,
              baseline(basic=Clip(basic, extra_channels=[("Root", "rotation")])))
    clip_case("clip-accessory-node", "clip.channels", "a rotation channel on a non-humanoid accessory", basic,
              baseline(basic=Clip(basic, extra_channels=[("Accessory", "rotation")])), accessory=True)
    clip_case("clip-translation-on-spine", "clip.channels", "a translation channel on the spine", basic,
              baseline(basic=Clip(basic, extra_channels=[("spine", "translation")])))
    clip_case("clip-cubicspline", "clip.sampler", "CUBICSPLINE interpolation", basic,
              baseline(basic=Clip(basic, cubic=True)))
    clip_case("clip-nan-key", "clip.sampler", "a NaN in a rotation key", basic,
              baseline(basic=Clip(basic, nan=True)))
    clip_case("clip-unsorted-times", "clip.sampler", "key times 4 and 5 swapped", basic,
              baseline(basic=Clip(basic, unsorted=True)))
    clip_case("clip-starts-late", "clip.sampler", "the first key at 0.05 s", basic,
              baseline(basic=Clip(basic, duration=0.35, starts_at=0.05)))
    clip_case("clip-non-unit-quat", "clip.sampler", "a rotation key of length 1.01", basic,
              baseline(basic=Clip(basic, quat_scale=1.01)))
    clip_case("clip-4097-keys", "clip.keys", "4097 keys in one second", "base.run",
              baseline(run=Clip("base.run", "loop", 1.0, contact=None, keys=4097)))
    clip_case("clip-120hz", "clip.keys", "sampled at 120 Hz", basic,
              baseline(basic=Clip(basic, hz=120)))
    clip_case("clip-too-short", "clip.duration", "a 0.1 s basic attack (window 0.15-1.2 s)", basic,
              baseline(basic=Clip(basic, duration=0.1, contact=0.05)), custom={**THREE, basic: (1, 0.05)})
    clip_case("clip-too-long", "clip.duration", "a 1.5 s Shockline release", "skill.wild_zap.release",
              baseline(zap=Clip("skill.wild_zap.release", duration=1.5, contact=0.12)))
    clip_case("clip-run-cycle-1.3s", "clip.duration", "a 1.3 s run cycle (window 0.4-1.2 s)", "base.run",
              baseline(run=Clip("base.run", "loop", 1.3, contact=None)))
    clip_case("clip-hips-drift-at-cap", "clip.in-place", "hips 0.149 leg lengths off rest (passes)", "base.run",
              baseline(run=Clip("base.run", "loop", 0.6, contact=None, hips_drift=0.149)), result="pass")
    clip_case("clip-hips-drift-over", "clip.in-place", "hips 0.151 leg lengths off rest", "base.run",
              baseline(run=Clip("base.run", "loop", 0.6, contact=None, hips_drift=0.151)))
    clip_case("clip-loop-open-3deg", "clip.loop-closure", "the run's last pose 3 degrees off its first",
              "base.run", baseline(run=Clip("base.run", "loop", 0.6, contact=None, seam_degrees=3.0)))
    clip_case("clip-loop-closed-1deg", "clip.loop-closure", "a 1 degree seam (passes)", "base.run",
              baseline(run=Clip("base.run", "loop", 0.6, contact=None, seam_degrees=1.0)), result="pass")
    clip_case("clip-contact-0.149", "clip.markers", "contact 0.149 s after the cast (passes)", basic,
              baseline(basic=Clip(basic, contact=0.149)), result="pass",
              custom={**THREE, basic: (1, 0.149)})
    clip_case("clip-contact-0.151", "clip.markers", "contact 0.151 s after the cast", basic,
              baseline(basic=Clip(basic, contact=0.151)), custom={**THREE, basic: (1, 0.151)})
    clip_case("clip-contact-missing", "clip.markers", "no contact time on an edge-released slot", basic,
              baseline(basic=Clip(basic, contact=None)), custom={**THREE, basic: (1, None)})
    clip_case("clip-contact-after-end", "clip.markers", "contact 0.45 s in a 0.4 s clip", basic,
              baseline(basic=Clip(basic, contact=0.45)), custom={**THREE, basic: (1, 0.45)})
    clip_case("clip-contact-mismatch-meta", "clip.markers",
              "the manifest says 0.1 s, ekza_animation_v1 says 0.12 s", basic,
              baseline(basic=Clip(basic, contact=0.1, meta_contact=0.12)))
    clip_case("clip-basic-longer-than-interval", "clip.min-interval",
              "a 0.6 s basic attack; the fastest basic attacks come 0.4375 s apart (warning)", basic,
              baseline(basic=Clip(basic, duration=0.6, contact=0.1)), result="warn")
    clip_case("rig-hips-scene-root-vrm1", None,
              "VRM 1.0 whose hips are a scene root (the builder inserts a parent before turning it)",
              "base.run", baseline(), result="pass", hips_root=True)
    return rows


# ---------------------------------------------------------------------------------------
# Output
# ---------------------------------------------------------------------------------------

def build(root=ROOT, refreeze=False):
    """{relative path: bytes} of the fixture directory."""
    root = Path(root)
    directory = root / FIXTURES
    if refreeze or not (directory / "contract.json").is_file():
        live = read_contract((root / CONTRACT).read_bytes(), root)
        contract, retired = freeze(live)
    else:
        contract = read_contract((directory / "contract.json").read_bytes(), root)
        retired = read_contract((directory / "contract-retired.json").read_bytes(), root)
    files = {"contract.json": render(contract), "contract-retired.json": render(retired)}
    contract_rows = []
    for name, expect, document in contract_cases(contract):
        path = f"contracts/{name}.json"
        files[path] = render(document)
        files[f"expected/{name}.json"] = render({"case": name, "document": expect})
        contract_rows.append({"id": name, "file": path, "expected": f"expected/{name}.json", "document": expect})
    package_rows = []
    for name, rule, note, spec, expected in cases(contract, retired):
        spec = {"id": name, **spec}
        files[f"packages/{name}.glb"] = package(spec, contract["revision"])
        target = (spec.get("manifest") or {}).get("target", {}).get("id", "wildspark")
        files[f"expected/{name}.json"] = render({"case": name, "contract": contract["revision"],
                                                 "target": target, "document": "ok", **expected, "note": note})
        package_rows.append({"id": name, "rule": rule, "file": f"packages/{name}.glb",
                             "expected": f"expected/{name}.json", "note": note})
    manifest = {
        "schema": "omoba.game-contract-fixtures.v1",
        "profile": PROFILE,
        "license": "CC0-1.0",
        "generator": "scripts/build_game_contract_fixtures.py",
        "contracts": [{"file": "contract.json", "revision": contract["revision"], "status": "active"},
                      {"file": "contract-retired.json", "revision": retired["revision"], "status": "retired"}],
        "verdicts": "gate",
        "packageCases": package_rows,
        "contractCases": contract_rows,
        "files": {path: {"sha256": digest(data), "bytes": len(data)} for path, data in sorted(files.items())},
    }
    files["manifest.json"] = json.dumps(manifest, indent=2, ensure_ascii=False).encode() + b"\n"
    return files


def manifest_of(glb):
    length = struct.unpack_from("<I", glb, 12)[0]
    document = json.loads(glb[20:20 + length])
    return document.get("asset", {}).get("extras", {}).get("ekza_package_v1")


def verify(root, files):
    """Self-checks of the construction: the reader accepts or refuses each contract case as
    labelled, and every manifest a case does not mean to break matches ekza.package.v1."""
    for name, data in files.items():
        if name.startswith("contracts/"):
            expect = next(row["document"] for row in json.loads(files["manifest.json"])["contractCases"]
                          if row["file"] == name)
            try:
                read_contract(data, root)
                verdict = "ok"
            except ContractError as error:
                verdict = f"refused: {error.code}"
            if verdict != expect:
                raise CatalogError(f"{name}: reader says {verdict}, expected {expect}")
        if name.startswith("packages/"):
            case = name[len("packages/"):-len(".glb")]
            manifest = manifest_of(data)
            if manifest is not None and case != "pack-bad-schema":
                problems = schema_errors(root, manifest, PACKAGE_SCHEMA_FILE)
                if problems:
                    raise CatalogError(f"{name}: manifest breaks its schema: {problems[:3]}")


def write(root, files, check=False):
    directory = Path(root) / FIXTURES
    if check:
        present = {str(path.relative_to(directory)) for path in directory.rglob("*") if path.is_file()}
        if present != set(files):
            raise CatalogError(f"Fixture files differ: extra {sorted(present - set(files))}, "
                               f"missing {sorted(set(files) - present)}")
        for name, data in files.items():
            if (directory / name).read_bytes() != data:
                raise CatalogError(f"Stale fixture {name}; rebuild without --check")
        return
    for name, data in files.items():
        path = directory / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--check", action="store_true", help="Verify the committed fixtures without writing")
    parser.add_argument("--refreeze", action="store_true", help="Cut contract.json again from the live contract")
    args = parser.parse_args(argv)
    try:
        files = build(args.root, args.refreeze and not args.check)
        verify(args.root, files)
        write(args.root, files, args.check)
    except (ContractError, CatalogError, OSError, KeyError, StopIteration) as error:
        parser.exit(1, f"Game contract fixtures failed: {error}\n")
    manifest = json.loads(files["manifest.json"])
    print(f"{'Verified' if args.check else 'Built'} {len(manifest['packageCases'])} package cases and "
          f"{len(manifest['contractCases'])} contract cases against {manifest['contracts'][0]['revision']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
