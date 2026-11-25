"""Game-owned Classic hero bindings; authoring validation, never game approval.

The companion document reuses the game contract's bounded JSON/canonical revision
rules. Until the game team assigns a licensed base, a class is explicitly blocked.
Consumers must obtain the selected policy from the trusted game-owned catalogue;
a self-hashed document supplied by an uploader is not an approval authority.
"""
from __future__ import annotations

import copy
import hashlib
import re
import subprocess
from pathlib import Path

from export_asset_catalog import CatalogError, canonical, load_json

SCHEMA = "omoba.hero-designs.v1"
SOURCE = "shared/assets/catalog/hero-designs.source.json"
EXPORT = "hero-designs.json"
HEROES = "shared/assets/catalog/heroes.json"
IMPLEMENTATION = "scripts/hero_designs.py"
INPUTS = (SOURCE, HEROES, IMPLEMENTATION)
MAX_BYTES = 64 * 1024
SHA = re.compile(r"[a-f0-9]{64}\Z")
REVISION = re.compile(r"sha256-[a-f0-9]{64}\Z")
ID = re.compile(r"[a-z][a-z0-9._-]{0,95}\Z")
BINDING_FIELDS = {
    "gameId", "modeScope", "classId", "heroDesignId", "baseRevision", "baseSha256",
    "designContractRevision", "gameContractRevision", "heroDesignsRevision",
}
ROW_FIELDS = {"classId", "status", "heroDesignId", "baseRevision", "baseSha256", "designContractRevision"}


class HeroDesignError(CatalogError):
    """An authoring policy or Classic binding is not valid for this game snapshot."""


def require(condition, message):
    if not condition:
        raise HeroDesignError(message)


def exact_keys(value, keys, label):
    require(isinstance(value, dict) and set(value) == set(keys), f"{label}: invalid fields")


def parse(data, label):
    require(isinstance(data, bytes) and len(data) <= MAX_BYTES, f"{label}: exceeds 64 KiB")
    try:
        return load_json(data, label)
    except CatalogError as error:
        raise HeroDesignError(str(error)) from error


def revision_of(document):
    return "sha256-" + hashlib.sha256(canonical({k: v for k, v in document.items() if k != "revision"})).hexdigest()


def class_ids(root):
    document = load_json((Path(root) / HEROES).read_bytes(), HEROES)
    ids = [row["id"] for row in document["classes"]]
    require(len(ids) == len(set(ids)) and ids and "any" not in ids, "Invalid canonical class IDs")
    return ids


def check_rows(rows, known_classes):
    require(isinstance(rows, list) and len(rows) == len(known_classes), "Exactly one design row per class required")
    ids = []
    designs = []
    for row in rows:
        exact_keys(row, ROW_FIELDS, "hero design")
        require(isinstance(row["classId"], str) and row["classId"] in known_classes, "Unknown hero class")
        ids.append(row["classId"])
        require(row["status"] in ("configured", "unconfigured"), "Invalid hero design status")
        values = [row[key] for key in ("heroDesignId", "baseRevision", "baseSha256", "designContractRevision")]
        if row["status"] == "unconfigured":
            require(all(value is None for value in values), "Unconfigured class cannot declare a base")
            continue
        require(isinstance(row["heroDesignId"], str) and ID.fullmatch(row["heroDesignId"]), "Invalid hero design ID")
        require(isinstance(row["baseSha256"], str) and SHA.fullmatch(row["baseSha256"]), "Invalid base SHA-256")
        for key in ("baseRevision", "designContractRevision"):
            require(isinstance(row[key], str) and REVISION.fullmatch(row[key]), f"Invalid {key}")
        designs.append(row["heroDesignId"])
    require(ids == list(known_classes), "Duplicate, missing or reordered class IDs")
    require(len(designs) == len(set(designs)), "A canonical hero design cannot belong to two classes")


def read_source(data, known_classes):
    document = parse(data, SOURCE)
    exact_keys(document, {"schema", "gameId", "modeScope", "classes"}, "hero design source")
    require(document["schema"] == SCHEMA and document["gameId"] == "omoba"
            and document["modeScope"] == "classic", "Unsupported hero design policy")
    check_rows(document["classes"], known_classes)
    return document


def read_export(data, known_classes, expected_contract_revision):
    document = parse(data, EXPORT)
    exact_keys(document, {"schema", "gameId", "modeScope", "classes", "gameContractRevision", "source", "revision"}, EXPORT)
    source = document["source"]
    exact_keys(source, {"commit", "exporter", "inputsSha256"}, "source provenance")
    require(isinstance(source["commit"], str) and re.fullmatch(r"[0-9a-f]{40}", source["commit"]), "Invalid source commit")
    require(source["exporter"] == "scripts/export_game_contract.py", "Invalid exporter")
    require(isinstance(source["inputsSha256"], str) and SHA.fullmatch(source["inputsSha256"]), "Invalid input hash")
    read_source(canonical({key: document[key] for key in ("schema", "gameId", "modeScope", "classes")}), known_classes)
    require(document["gameContractRevision"] == expected_contract_revision
            and isinstance(expected_contract_revision, str) and REVISION.fullmatch(expected_contract_revision),
            "Stale game contract revision")
    require(document["revision"] == revision_of(document), "Hero design policy checksum mismatch")
    return document


def build_export(root, game_contract_revision, source_commit, previous=None):
    root = Path(root)
    raw = (root / SOURCE).read_bytes()
    document = copy.deepcopy(read_source(raw, class_ids(root)))
    document["gameContractRevision"] = game_contract_revision
    hashes = {name: hashlib.sha256((root / name).read_bytes()).hexdigest() for name in INPUTS}
    inputs_hash = hashlib.sha256(canonical(hashes)).hexdigest()
    # Unrelated commits do not move a content revision. Preserve provenance only
    # after verifying the previous complete document, never merely its stated hash.
    if previous is not None:
        try:
            old = read_export(previous, class_ids(root), game_contract_revision)
            if all(old[key] == value for key, value in document.items()) and old["source"]["inputsSha256"] == inputs_hash:
                return old
        except HeroDesignError:
            pass
    if source_commit is None:
        source_commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
    document["source"] = {"commit": source_commit, "exporter": "scripts/export_game_contract.py", "inputsSha256": inputs_hash}
    document["revision"] = revision_of(document)
    return read_export(canonical(document), class_ids(root), game_contract_revision)


def validate_classic_binding(binding, *, policy, target, game_contract_revision):
    """Check authoring identity against a trusted selected policy, not an upload's
    own policy. Passing is necessary, but not sufficient, for game-owner approval.
    The native server must implement and enforce this additional admission gate.
    """
    # Validate the complete policy again against game-owned class IDs. A valid
    # checksum establishes integrity, not authority; callers still pin selection.
    policy = read_export(canonical(policy), class_ids(Path(__file__).resolve().parents[1]), game_contract_revision)
    exact_keys(binding, BINDING_FIELDS, "Classic binding")
    require(isinstance(target, str) and target != "any", "Classic requires a concrete hero target")
    require(policy.get("schema") == SCHEMA and policy.get("gameId") == "omoba"
            and policy.get("modeScope") == "classic", "Unsupported hero design policy")
    require(policy.get("gameContractRevision") == game_contract_revision, "Stale game contract revision")
    require(policy.get("revision") == revision_of(policy), "Hero design policy checksum mismatch")
    rows = policy.get("classes")
    require(isinstance(rows, list), "Missing hero design policy classes")
    candidates = [row for row in rows if isinstance(row, dict) and row.get("classId") == target]
    require(len(candidates) == 1, "Unknown hero target")
    row = candidates[0]
    require(row.get("status") == "configured", "Canonical hero base is unconfigured: Classic authoring blocked")
    # Validate the assigned row as well, so callers cannot accidentally treat a
    # partial policy as configured. read_export additionally validates all rows.
    check_rows([row], [target])
    expected = {"gameId": "omoba", "modeScope": "classic", "classId": target,
                "gameContractRevision": game_contract_revision, "heroDesignsRevision": policy["revision"],
                **{key: row[key] for key in ("heroDesignId", "baseRevision", "baseSha256", "designContractRevision")}}
    require(binding == expected, "Classic binding does not match selected hero/design/base/contract")
    return expected
