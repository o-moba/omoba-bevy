#!/usr/bin/env python3
"""Fail-closed content/provenance check of the actual candidate assets directory.

The repository policy is the review authority, including for copied packages.
This checks known content and embedded metadata; it is not legal certification.
"""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import struct

ROOT = Path(__file__).resolve().parents[1]
POLICY = ROOT / "client/assets/config/asset_policy.json"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def safe_path(root, relative):
    path = PurePosixPath(relative)
    if path.is_absolute() or ".." in path.parts or "\\" in relative:
        raise ValueError(f"unsafe asset reference: {relative}")
    candidate = root / path
    if not candidate.resolve().is_relative_to(root.resolve()):
        raise ValueError(f"asset reference escapes root: {relative}")
    return candidate


def glb_json(path):
    data = path.read_bytes()
    if len(data) < 20 or struct.unpack_from("<4sII", data) != (b"glTF", 2, len(data)):
        raise ValueError(f"invalid GLB: {path.name}")
    length, kind = struct.unpack_from("<II", data, 12)
    if kind != 0x4E4F534A or length > len(data) - 20:
        raise ValueError(f"invalid GLB JSON: {path.name}")
    return json.loads(data[20:20 + length])


def embedded_conflicts(gltf):
    meta = gltf.get("extensions", {}).get("VRM", {}).get("meta", {})
    errors = []
    required = {"licenseName": "CC0", "allowedUserName": "Everyone", "commercialUssageName": "Allow"}
    for key, expected in required.items():
        if meta.get(key) != expected:
            errors.append(f"embedded {key} must be {expected}, got {meta.get(key)!r}")
    for key in ("allowCommercialUse", "allowRedistribution"):
        if key in meta and meta[key] is not True:
            errors.append(f"embedded {key} restricts use")
    if "modification" in meta and meta["modification"] != "ALLOW":
        errors.append("embedded modification restricts use")
    for collection in ("buffers", "images"):
        for entry in gltf.get(collection, []):
            uri = entry.get("uri", "")
            if uri and not uri.startswith("data:"):
                errors.append(f"external {collection} dependency: {uri}")
    return errors


def validate(asset_root, policy_path=POLICY):
    root = Path(asset_root).resolve()
    policy_path = Path(policy_path).resolve()
    errors, checks = [], []
    try:
        policy = json.loads(policy_path.read_text())
        if policy.get("schema_version") != 1:
            raise ValueError("unsupported policy schema")
        packaged_policy = root / "config/asset_policy.json"
        if not packaged_policy.is_file() or sha(packaged_policy) != sha(policy_path):
            errors.append("asset policy differs from the reviewed repository policy")
        denied = policy["denied_files"]
        denied_hashes = {entry["sha256"] for entry in denied.values()}
        denied_stems = {Path(path).stem for path in denied}
        actual_models = set()
        for path in sorted(root.rglob("*")):
            if not path.is_file():
                continue
            relative = path.relative_to(root).as_posix()
            safe_path(root, relative)
            digest = sha(path)
            if digest in denied_hashes or path.stem.lower() in denied_stems:
                errors.append(f"denied content: {relative}")
            with path.open("rb") as stream:
                is_glb = stream.read(4) == b"glTF"
            if is_glb or path.suffix.lower() in (".glb", ".gltf", ".vrm", ".fbx", ".obj"):
                actual_models.add(relative)
        actors = policy["approved_actor_models"]
        referenced = set()
        for directory, expected_count, procedural_ids in (
                ("avatars", 15, set()), ("bosses", 1, {"wendigo-boss"}),
                ("minions", 0, {"minion-green", "minion-blue"})):
            manifest = json.loads((root / directory / "manifest.json").read_text())
            entries = manifest["avatars"]
            if len(entries) != expected_count:
                errors.append(f"unexpected {directory} roster size: {len(entries)}")
            procedural = manifest.get("procedural", [])
            if {item["id"] for item in procedural} != procedural_ids:
                errors.append(f"unreviewed {directory} procedural roster")
            for item in procedural:
                if (item.get("source") != "client/src/creatures3d.rs"
                        or item.get("provenance") != "original project-authored geometry"):
                    errors.append(f"missing procedural provenance: {item['id']}")
            for entry in entries:
                relative = f"{directory}/{entry['slug']}.glb"
                if relative in referenced:
                    errors.append(f"duplicate manifest reference: {relative}")
                referenced.add(relative)
                approved = actors.get(relative)
                if not approved:
                    errors.append(f"unapproved actor reference: {relative}")
                    continue
                if entry.get("license") != approved["license"] or entry.get("source_url") != approved["source_url"]:
                    errors.append(f"manifest provenance mismatch: {relative}")
                if entry.get("thumbnail") != approved.get("thumbnail"):
                    errors.append(f"unreviewed actor preview reference: {relative}")
                if entry.get("thumbnail"):
                    preview = safe_path(root, f"{directory}/{entry['thumbnail']}")
                    if not preview.is_file():
                        errors.append(f"missing actor preview: {relative}")
        if referenced != set(actors):
            errors.append("actor references differ from reviewed model inventory")
        for relative, approved in actors.items():
            path = safe_path(root, relative)
            if not path.is_file():
                errors.append(f"missing approved model: {relative}")
                continue
            if sha(path) != approved["sha256"]:
                errors.append(f"unreviewed model hash: {relative}")
            errors.extend(f"{relative}: {issue}" for issue in embedded_conflicts(glb_json(path)))
        environment_path = safe_path(root, policy["environment_manifest"])
        if sha(environment_path) != policy["environment_manifest_sha256"]:
            errors.append("unreviewed environment manifest hash")
        environment = json.loads(environment_path.read_text())
        if environment.get("provenance") != policy["environment_provenance"]:
            errors.append("missing environment provenance")
        expected_environment = {"environment", "foliage", "watchtower_green", "watchtower_blue",
                                "sanctuary_green", "sanctuary_blue"}
        if {entry["id"] for entry in environment["files"]} != expected_environment or len(environment["files"]) != 6:
            errors.append("unreviewed environment scene inventory")
        environment_models = set()
        for entry in environment["files"]:
            relative = f"verdant/{entry['path']}"
            environment_models.add(relative)
            path = safe_path(root, relative)
            if not path.is_file():
                errors.append(f"missing environment model: {relative}")
                continue
            if sha(path) != entry["sha256"]:
                errors.append(f"unreviewed environment model hash: {relative}")
        expected_models = set(actors) | environment_models
        for relative in sorted(actual_models - expected_models):
            errors.append(f"unknown model: {relative}")
        checks = ["denied filenames and hashes across every packaged file", "reviewed model inventory and SHA-256",
                  "manifest references, previews and provenance", "embedded actor permissions", "reviewed original environment manifest"]
    # JSON with unexpected object/array shapes must still return a closed gate.
    except (OSError, ValueError, KeyError, TypeError, AttributeError, struct.error) as error:
        errors.append(f"invalid or incomplete asset inventory: {error}")
    return dict(status="FAIL" if errors else "PASS", asset_root=str(root), policy_sha256=sha(policy_path) if policy_path.is_file() else None,
                checks=checks, errors=errors)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assets", type=Path, default=ROOT / "client/assets")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    result = validate(args.assets)
    text = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.write_text(text)
    print(text, end="")
    raise SystemExit(0 if result["status"] == "PASS" else 1)


if __name__ == "__main__":
    main()
