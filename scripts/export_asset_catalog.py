#!/usr/bin/env python3
"""Export the game-owned creator catalogue without third-party dependencies."""
from __future__ import annotations

import argparse
import copy
from decimal import Decimal
import hashlib
import json
import math
from pathlib import Path, PurePosixPath
import re
import struct
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = "omoba.asset-catalog.v1"
ID = re.compile(r"[a-z][a-z0-9_.-]{0,127}\Z")
SHA = re.compile(r"[0-9a-f]{64}\Z")
SLOTS = ("q", "w", "e", "r")
ROLES = {"held_weapon", "projectile", "world_prop", "engine_effect", "animation_reference"}
MAX_SOURCE_BYTES = 16 * 1024 * 1024


class CatalogError(ValueError):
    """A source or destination cannot safely represent this catalogue."""


def require(condition, message):
    if not condition:
        raise CatalogError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    """Sorted-key JSON with browser-compatible numbers for this bounded contract."""
    def encode(item):
        if item is None or isinstance(item, (str, bool)):
            return json.dumps(item, ensure_ascii=False)
        if isinstance(item, (int, float)):
            require(math.isfinite(item) and abs(item) <= 9007199254740991,
                    "Canonical number must be finite and JavaScript-safe")
            if int(item) == item:
                return str(int(item))
            require(abs(item) >= 0.000001, "Canonical fractional number is below browser-safe range")
            return format(Decimal(str(item)), "f")
        if isinstance(item, list):
            return "[" + ",".join(encode(child) for child in item) + "]"
        require(isinstance(item, dict) and all(isinstance(key, str) and key.isascii() for key in item),
                "Canonical objects require ASCII keys")
        return "{" + ",".join(encode(key) + ":" + encode(item[key]) for key in sorted(item)) + "}"
    return encode(value).encode("utf-8")


def identifier(value, context):
    require(isinstance(value, str) and ID.fullmatch(value), f"Invalid {context}: {value!r}")
    return value


def bounded_text(value, context, maximum=2000):
    require(isinstance(value, str) and bool(value.strip()) and len(value) <= maximum,
            f"Invalid {context}: expected nonempty text up to {maximum} characters")
    return value


def named_rows(rows, context):
    require(isinstance(rows, list), f"{context} must be a list")
    result = {}
    for row in rows:
        require(isinstance(row, dict), f"Invalid {context} row")
        key = identifier(row.get("id"), context)
        require(key not in result, f"Duplicate {context}: {key}")
        result[key] = row
    return result


def safe_relative(value):
    require(isinstance(value, str) and value and "\\" not in value,
            f"Unsafe source path: {value!r}")
    path = PurePosixPath(value)
    require(not path.is_absolute() and all(p not in (".", "..") for p in value.split("/")),
            f"Unsafe source path: {value}")
    return path


def png_size(data):
    require(len(data) >= 33 and data[:8] == b"\x89PNG\r\n\x1a\n"
            and data[8:16] == b"\x00\x00\x00\rIHDR", "Invalid PNG header")
    width, height = struct.unpack(">II", data[16:24])
    require(0 < width <= 8192 and 0 < height <= 8192, "PNG dimensions out of bounds")
    return width, height


def load_json(data, context):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            require(key not in result, f"Duplicate JSON key in {context}: {key}")
            result[key] = value
        return result
    try:
        return json.loads(data, object_pairs_hook=unique,
                          parse_constant=lambda value: (_ for _ in ()).throw(
                              CatalogError(f"Non-finite JSON value in {context}: {value}")))
    except (UnicodeError, json.JSONDecodeError) as error:
        raise CatalogError(f"Invalid JSON in {context}: {error}") from error


class Exporter:
    def __init__(self, root=ROOT, public_prefix="/asset-catalog", preview_dir=None):
        self.root = Path(root).resolve()
        require(re.fullmatch(r"/[a-zA-Z0-9_/-]+", public_prefix) is not None
                and "//" not in public_prefix and ".." not in public_prefix,
                "Public prefix must be a local absolute URL path")
        self.prefix = public_prefix.rstrip("/")
        self.preview_dir = Path(preview_dir).resolve() if preview_dir else None
        self.inputs = {}
        self.files = {}

    def read(self, source):
        safe_relative(source)
        path = self.root.joinpath(source)
        require(path.resolve().is_relative_to(self.root), f"Source escapes checkout: {source}")
        require(path.is_file(), f"Missing source file: {source}")
        require(path.stat().st_size <= MAX_SOURCE_BYTES, f"Source too large: {source}")
        data = path.read_bytes()
        self.inputs[source] = digest(data)
        return data

    def json(self, source, version=None):
        result = load_json(self.read(source), source)
        require(isinstance(result, dict), f"Expected JSON object: {source}")
        if version is not None:
            actual = result.get("schema_version", result.get("schemaVersion"))
            require(actual == version, f"Unsupported source schema in {source}: {actual}")
        return result

    def publish(self, data, extension):
        sha = digest(data)
        name = f"files/{sha}.{extension}"
        self.files[name] = data
        return {"url": f"{self.prefix}/{name}", "sha256": sha, "bytes": len(data)}

    def baseline(self, raw):
        require(isinstance(raw, dict), "Missing requirement baseline")
        require(raw.get("kind") in {"model", "reference", "procedural"}, "Invalid baseline kind")
        source = raw.get("sourcePath")
        data = self.read(source)
        result = copy.deepcopy(raw)
        result.update(sha256=digest(data), bytes=len(data), preview=None)
        require("url" not in raw and "preview" not in raw,
                "Authored baselines cannot inject published URLs")
        if raw["kind"] == "model":
            require(source.startswith("client/assets/weapons/") and source.endswith(".glb"),
                    "Only licensed bundled handheld GLBs are published in v1")
            require(raw.get("license") == "CC-BY-4.0"
                    and raw.get("attribution") == "Open Moba contributors", "Missing model provenance")
            require(len(data) >= 12 and data[:4] == b"glTF"
                    and struct.unpack("<II", data[4:12]) == (2, len(data)), "Invalid GLB header")
            result.update(self.publish(data, "glb"))
            if self.preview_dir:
                preview = self.preview_dir / f"{result['sha256']}.png"
                if preview.exists():
                    require(preview.is_file() and preview.resolve().is_relative_to(self.preview_dir),
                            "Unsafe preview path")
                    require(preview.stat().st_size <= MAX_SOURCE_BYTES, "Preview too large")
                    image = preview.read_bytes()
                    png_size(image)
                    result["preview"] = self.publish(image, "png")
                    self.inputs[f"preview/{result['sha256']}.png"] = digest(image)
        return result

    def build(self, source_commit=None):
        heroes = named_rows(self.json("shared/assets/catalog/heroes.json", 1)["classes"], "class")
        skill_source = self.json("shared/assets/catalog/skills.json", 1)
        reusable = named_rows(skill_source["skills"], "skill")
        presentation = self.json("client/assets/config/skills.skillfx", 2)
        combat = self.json("client/assets/config/combat_visuals.json", 1)
        motion_source = "client/assets/animations/humanoid-motion-v1.json"
        motions = self.json(motion_source, 1)
        weapon_source = self.json("client/assets/weapons/manifest.json", 1)
        weapons = named_rows(weapon_source["items"], "weapon")
        authored = self.json("shared/assets/catalog/asset-requirements.json", 1)
        icons = self.json("client/assets/ui/skills/manifest.json", 2)
        evidence = authored.get("sourceEvidence")
        require(isinstance(evidence, list) and len(evidence) == 5, "Missing authored source evidence")
        require({row.get("sourcePath") for row in evidence} == {
            "shared/src/handheld.rs", "client/src/skill_presentation/effects.rs",
            "client/src/skill_icons.rs", "shared/src/loadout.rs", "client/src/held_weapons.rs"},
            "Authored evidence must pin handheld defaults, prop resolution and semantic icon mapping")
        for row in evidence:
            require(digest(self.read(row["sourcePath"])) == row.get("sha256"),
                    f"Authored source evidence changed: {row['sourcePath']}; review bindings before repinning")
        for path in ("shared/src/handheld.rs", "shared/src/loadout.rs", "client/assets/weapons/LICENSE.md",
                     "client/assets/ui/skills/LICENSE.md", "assets-src/skills/README.md",
                     "assets-src/animations/README.md", "scripts/export_asset_catalog.py"):
            self.read(path)

        classes, skills, owners = {}, {}, {}
        for cid, hero in heroes.items():
            require(bool(hero.get("skills")) != bool(hero.get("abilities")),
                    f"Class {cid} must have skills or inline abilities, not both")
            raw_skills = hero.get("abilities")
            if raw_skills is None:
                raw_skills = []
                for sid in hero["skills"]:
                    require(sid in reusable, f"Unknown skill {sid} in class {cid}")
                    raw_skills.append(reusable[sid])
            require(len(raw_skills) == 4, f"Class {cid} must have Q/W/E/R skills")
            placements = []
            for slot, raw in zip(SLOTS, raw_skills):
                sid = identifier(raw.get("id"), "skill")
                require(sid not in [row["id"] for row in placements], f"Duplicate class skill {sid}")
                if sid in skills:
                    require(skills[sid]["name"] == raw["name"]
                            and skills[sid]["description"] == raw["description"],
                            f"Conflicting definition for {sid}")
                else:
                    skills[sid] = {"id": sid, "name": bounded_text(raw["name"], f"{sid} name", 200),
                                   "description": bounded_text(raw["description"], f"{sid} description"),
                                   "requirementIds": []}
                owners.setdefault(sid, set()).add(cid)
                placements.append({"slot": slot, "id": sid})
            classes[cid] = {"id": cid, "name": bounded_text(hero["display_name"], f"{cid} name", 200),
                            "description": bounded_text(hero["tagline"], f"{cid} description"),
                            "role": identifier(hero["role"], f"{cid} role"), "skills": placements, "requirementIds": [],
                            "basicAttack": {"id": cid, "name": f"{hero['display_name']} basic attack",
                                            "requirementIds": []}}
        require(set(reusable) <= set(skills), "Reusable skill catalogue contains unassigned skills")
        require(set(presentation["skills"]) == set(skills), "Presentation/skill coverage mismatch")
        require(set(presentation["basic_attacks"]) == set(classes), "Basic attack coverage mismatch")

        requirements = named_rows(authored["requirements"], "requirement")
        # Derived presentation references cover every skill, even when there is no GLB.
        for sid, row in presentation["skills"].items():
            require(row.get("home") in classes, f"Unknown presentation home for {sid}")
            rid = f"skill.{sid}.presentation"
            require(rid not in requirements, f"Reserved generated requirement ID: {rid}")
            requirements[rid] = {"id": rid, "name": f"{skills[sid]['name']} presentation",
                "role": "engine_effect", "status": "engine_owned", "profile": None,
                "uses": [{"skillId": sid, "phase": "presentation"}],
                "brief": "Game-owned cast, body, impact and sound presentation. This reference does not expose an upload slot or executable skill logic.",
                "baseline": {"kind": "procedural", "sourcePath": "client/assets/config/skills.skillfx",
                             "details": {"skillId": sid, "bodyArchetype": (row.get("body") or {}).get("archetype")}}}

        motion_uses = {}
        for sid, row in presentation["skills"].items():
            for phase, mid in (("release", row.get("release")), ("windup", row.get("windup")),
                               ("recast", row.get("motion", {}).get("recast"))):
                if mid:
                    motion_uses.setdefault(mid, []).append({"skillId": sid, "phase": phase})
        for cid, row in presentation["basic_attacks"].items():
            rid = f"basic.{cid}.presentation"
            require(rid not in requirements, f"Reserved generated requirement ID: {rid}")
            requirements[rid] = {"id": rid, "name": f"{classes[cid]['name']} basic attack presentation",
                "role": "engine_effect", "status": "engine_owned", "profile": None,
                "uses": [{"basicAttackId": cid, "phase": "presentation"}],
                "brief": "Engine-owned basic attack accents and impact. Visuals cannot change damage, timing, collision or projectile travel.",
                "baseline": {"kind": "procedural", "sourcePath": "client/assets/config/skills.skillfx"}}
            for state, profile in ((None, row), ("rockets", row.get("rockets"))):
                if profile:
                    for mid in profile["motions"]:
                        use = {"basicAttackId": cid, "phase": "attack"}
                        if state:
                            use["state"] = state
                        motion_uses.setdefault(mid, []).append(use)
        for mid, uses in motion_uses.items():
            identifier(mid, "motion")
            require(mid in motions["clips"], f"Unknown motion: {mid}")
            clip = motions["clips"][mid]
            rid = f"motion.{mid}"
            require(rid not in requirements, f"Reserved generated requirement ID: {rid}")
            requirements[rid] = {"id": rid, "name": mid.replace("_", " ").title(),
                "role": "animation_reference", "status": "engine_owned", "profile": None, "uses": uses,
                "brief": "Shared engine-owned humanoid motion, retargeted to compatible avatars. Semantic timing is a reference; custom animation upload and browser combat playback are not supported in this iteration.",
                "baseline": {"kind": "reference", "sourcePath": motion_source,
                    "license": motions["source"]["license"], "attribution": motions["source"]["author"],
                    "details": {"motionId": mid, "duration": clip["duration"], "looping": clip["looping"],
                                "sourceClip": clip["source_clip"], "contact": motions["contacts"].get(mid)}}}

        # Detect a stale authored assertion about the effective rocket body.
        rocket = requirements.get("wildspark.projectile.basic-rocket", {}).get("baseline", {})
        runtime_rocket = combat["profiles"]["wild_rocket"]
        require(not runtime_rocket.get("form") and rocket.get("sourcePath") ==
                "client/assets/" + runtime_rocket.get("model", {}).get("path", ""),
                "Wildspark effective rocket form changed; update its authored brief")
        # The class-default override is pinned through held_weapons.rs evidence.
        default_repeater = "client/assets/weapons/wildspark-repeater.glb"
        require(requirements.get("wildspark.handheld.repeater", {}).get("baseline", {}).get("sourcePath")
                == default_repeater, "Wildspark default repeater binding changed")
        licensed_models = {"client/assets/" + weapon["model"] for weapon in weapons.values()}
        licensed_models.add(default_repeater)
        # A symbolic runtime model is a real requirement only when its usage is
        # described explicitly, including secondary bodies such as the orb.
        for sid, row in presentation["skills"].items():
            bodies = [row.get("body") or {}, *row.get("aux", {}).values()]
            for body in bodies:
                if body.get("model"):
                    path = f"client/assets/cosmetics/standard/{identifier(body['model'], 'model')}.glb"
                    require(any(candidate.get("baseline", {}).get("sourcePath") == path
                                and any(use.get("skillId") == sid for use in candidate.get("uses", []))
                                for candidate in requirements.values()),
                            f"Missing authored model requirement for {sid}: {body['model']}")
        for rid, row in requirements.items():
            identifier(rid, "requirement")
            require(row.get("role") in ROLES, f"Invalid role for {rid}")
            require(row.get("status") in {"supported", "planned", "engine_owned"}, f"Invalid status for {rid}")
            for field in ("name", "brief"):
                bounded_text(row.get(field), f"{field} for {rid}", 200 if field == "name" else 2000)
            if row["status"] == "supported":
                require(row["role"] == "held_weapon"
                        and row.get("profile") == {"id": "handheld-glb-v1", "version": 1}
                        and row["baseline"].get("kind") == "model", f"Unsupported v1 capability: {rid}")
            else:
                require(row.get("profile") is None, f"Unimplemented profile for {rid}")
            require(isinstance(row.get("uses"), list) and row["uses"], f"Missing uses for {rid}")
            class_ids = set()
            seen = set()
            for use in row["uses"]:
                require(isinstance(use, dict) and set(use) <= {"skillId", "basicAttackId", "phase", "state"},
                        f"Invalid use edge for {rid}")
                require(("skillId" in use) != ("basicAttackId" in use), f"Use must have one target: {rid}")
                for field, value in use.items():
                    identifier(value, f"{field} in {rid}")
                key = canonical(use)
                require(key not in seen, f"Duplicate use edge for {rid}")
                seen.add(key)
                if "skillId" in use:
                    sid = use["skillId"]
                    require(sid in skills, f"Unknown skill use {sid} for {rid}")
                    class_ids.update(owners[sid])
                    skills[sid]["requirementIds"].append(rid)
                else:
                    cid = use["basicAttackId"]
                    require(cid in classes, f"Unknown basic attack {cid} for {rid}")
                    class_ids.add(cid)
                    classes[cid]["basicAttack"]["requirementIds"].append(rid)
            row["classIds"] = sorted(class_ids)
            row["uses"] = sorted(row["uses"], key=canonical)
            if row["baseline"]["kind"] == "model":
                require(row["baseline"]["sourcePath"] in licensed_models, f"Unlisted handheld model: {rid}")
            row["baseline"] = self.baseline(row["baseline"])
            for cid in class_ids:
                classes[cid]["requirementIds"].append(rid)

        # One atlas is reused by many skill sprites; never copy AI provenance prompts.
        icon_skills = set()
        icon_bindings = authored.get("iconAtlases")
        require(isinstance(icon_bindings, list), "Missing semantic icon bindings")
        atlas_bindings = {}
        for binding in icon_bindings:
            path = binding.get("path")
            safe_relative(path)
            require(path not in atlas_bindings, f"Duplicate icon atlas binding: {path}")
            require(isinstance(binding.get("skillIds"), list), "Invalid semantic icon binding")
            atlas_bindings[path] = binding["skillIds"]
        require(set(atlas_bindings) == {atlas["path"] for atlas in icons["atlases"]},
                "Semantic icon atlas coverage mismatch")
        for atlas in icons["atlases"]:
            data = self.read("client/assets/" + str(safe_relative(atlas["path"])))
            require(digest(data) == atlas["sha256"], f"Stale icon atlas hash: {atlas['path']}")
            width, height = png_size(data)
            require((width, height) == (atlas["width"], atlas["height"]), "Atlas dimensions mismatch")
            require(atlas["columns"] == 4 and len(atlas["classes"]) == atlas["rows"], "Invalid icon grid")
            require(width == 4 * icons["cell_size"] and height == atlas["rows"] * icons["cell_size"],
                    "Invalid icon cell size")
            descriptor = self.publish(data, "png")
            semantic_ids = atlas_bindings[atlas["path"]]
            require(len(semantic_ids) == atlas["rows"] * atlas["columns"], "Semantic icon cell count mismatch")
            for index, sid in enumerate(semantic_ids):
                require(sid in skills, f"Unknown semantic icon skill: {sid}")
                require(sid not in icon_skills, f"Duplicate semantic icon skill: {sid}")
                icon_skills.add(sid)
                skills[sid]["icon"] = {**descriptor, "license": icons["license"],
                    "attribution": "Open Moba contributors", "x": (index % 4) * icons["cell_size"],
                    "y": (index // 4) * icons["cell_size"], "width": icons["cell_size"], "height": icons["cell_size"],
                    "atlasWidth": width, "atlasHeight": height}
        require(icon_skills == set(skills), "Missing skill icon coverage")
        for row in list(skills.values()) + list(classes.values()):
            row["requirementIds"] = sorted(set(row["requirementIds"]))
        for row in classes.values():
            row["basicAttack"]["requirementIds"] = sorted(set(row["basicAttack"]["requirementIds"]))
        if source_commit is None:
            source_commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=self.root, text=True).strip()
        require(re.fullmatch(r"[0-9a-f]{40}", source_commit) is not None, "Invalid source commit")
        result = {"schema": SCHEMA, "schemaVersion": 1, "projectId": "omoba",
                  "source": {"commit": source_commit, "inputsSha256": digest(canonical(self.inputs)),
                             "gameplayRevision": skill_source["revision"]},
                  "classes": sorted(classes.values(), key=lambda row: row["id"]),
                  "skills": sorted(skills.values(), key=lambda row: row["id"]),
                  "requirements": sorted(requirements.values(), key=lambda row: row["id"])}
        # Parsing our canonical form normalizes e.g. duration 1.0 to 1 before
        # consumers reproduce the revision using sorted JSON.stringify.
        result = json.loads(canonical(result))
        result["revision"] = "sha256-" + digest(canonical(result))
        return result


def write_export(exporter, catalog, output, check=False):
    output = Path(output).absolute()
    require(not output.is_symlink(), "Output directory cannot be a symlink")
    output = output.resolve()
    require(output != exporter.root and not exporter.root.is_relative_to(output),
            "Output cannot replace the checkout or an ancestor")
    for source in exporter.inputs:
        require(not exporter.root.joinpath(source).is_relative_to(output), "Output overlaps source inputs")
    expected = {**exporter.files, "catalog.json": json.dumps(catalog, indent=2, ensure_ascii=False,
                                                            allow_nan=False).encode() + b"\n"}
    for name, data in expected.items():
        destination = output / name
        require(destination.resolve().is_relative_to(output) and not destination.is_symlink(),
                f"Output path escapes destination: {name}")
        if check:
            require(destination.is_file() and destination.read_bytes() == data, f"Stale export file: {name}")
        else:
            if name != "catalog.json" and destination.exists():
                require(destination.read_bytes() == data, f"Corrupt content-addressed file: {name}")
            destination.parent.mkdir(parents=True, exist_ok=True)
            temporary = destination.with_name(destination.name + ".tmp")
            require(not temporary.exists() and not temporary.is_symlink(), f"Temporary file already exists: {temporary}")
            with temporary.open("xb") as handle:
                handle.write(data)
            temporary.replace(destination)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--root", type=Path, default=ROOT, help="Explicit game checkout")
    parser.add_argument("--preview-dir", type=Path, help="Optional PNGs named <model-sha256>.png")
    parser.add_argument("--public-prefix", default="/asset-catalog")
    parser.add_argument("--check", action="store_true", help="Verify an existing export without writing")
    args = parser.parse_args(argv)
    try:
        exporter = Exporter(args.root, args.public_prefix, args.preview_dir)
        result = exporter.build()
        write_export(exporter, result, args.output, args.check)
    except (CatalogError, OSError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Asset catalogue export failed: {error}\n")
    print(f"{'Verified' if args.check else 'Exported'} {len(result['classes'])} classes, "
          f"{len(result['skills'])} skills, {len(result['requirements'])} requirements: {result['revision']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
