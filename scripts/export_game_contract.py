#!/usr/bin/env python3
"""Finish, check and export OMOBA's game contract (Ekza Game Protocol 1.0).

The document is `ekza.game-contract.v1` with the profile `omoba.character.v1`. The Rust
generator (client/src/game_contract.rs) writes its body into
shared/assets/catalog/game-contract.json:

    OMOBA_WRITE_GAME_CONTRACT=1 cargo test -p client --lib game_contract_is_current

This script adds what the body cannot know - `source` (commit, exporter, input hashes), the
profile's provenance (gameplay revision, presentation and motion library hashes, protocol
version), `previousRevision` and the canonical `revision` - checks the result the way every
reader of the protocol does (GAME-PROTOCOL-SPEC §3.5) and writes it:

    python3 scripts/export_game_contract.py --output shared/assets/catalog
    python3 scripts/export_game_contract.py --output shared/assets/catalog --check

A body that has not changed keeps its header, so the revision moves only with data. Standard
library only. Reference: docs/game-contract.md.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import math
from pathlib import Path
import re
import subprocess
import sys

from export_asset_catalog import CatalogError, canonical, load_json

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = "ekza.game-contract.v1"
PROFILE = "omoba.character.v1"
READER_VERSION = (1, 0)
CATALOG = "shared/assets/catalog"
CONTRACT = f"{CATALOG}/game-contract.json"
SCHEMA_FILE = f"{CATALOG}/ekza.game-contract.v1.schema.json"
PROFILE_SCHEMA_FILE = f"{CATALOG}/omoba.character.v1.schema.json"
PACKAGE_SCHEMA_FILE = f"{CATALOG}/ekza.package.v1.schema.json"
EXPORTER = "scripts/export_game_contract.py"
PRESENTATION = "client/assets/config/skills.skillfx"
MOTION = "client/assets/animations/humanoid-motion-v1.json"
SKILLS = "shared/assets/catalog/skills.json"
PROTOCOL = "shared/src/protocol.rs"
# Every file the body and its header are cut from.
INPUTS = (
    PRESENTATION,
    MOTION,
    "shared/assets/catalog/heroes.json",
    SKILLS,
    f"{CATALOG}/game-contract-rules.json",
    "client/src/game_contract.rs",
    EXPORTER,
)
HEADER = ("source", "previousRevision", "revision")
PROVENANCE = ("gameplayRevision", "presentationSha256", "motionLibrary", "protocolVersion")
TOP_LEVEL = ("schema", "schemaVersion", "minReaderVersion", "game", "revision", "previousRevision", "license",
             "source", "profiles", "rigs", "sockets", "packaging", "rules", "slots", "families", "targets",
             "retired", "ext")
# The core rule vocabulary of EGP 1.0 (§7.2). A contract naming another core id is refused
# (fail closed); namespaced ids (`<game>.<scope>.<name>`) are kept and deferred.
CORE_RULES = ("pack.manifest", "pack.contract", "pack.target", "pack.slot-known", "pack.ref", "pack.unreferenced",
              "pack.bytes", "pack.requires", "rig.humanoid", "clip.channels", "clip.sampler", "clip.keys",
              "clip.duration", "clip.in-place", "clip.loop-closure", "clip.markers", "clip.min-interval",
              "asset.container", "asset.nodes", "asset.budget", "asset.bounds", "asset.anchors", "asset.materials",
              "asset.motion", "socket.node", "rights.license", "rights.derivation")
KNOWN_PROFILES = (PROFILE,)
MAX_BYTES = 1024 * 1024
REVISION = re.compile(r"sha256-[0-9a-f]{64}\Z")


class ContractError(ValueError):
    """A contract document is refused; `code` names the reader check that failed."""

    def __init__(self, code, message):
        super().__init__(f"{code}: {message}")
        self.code = code


def require(condition, code, message):
    if not condition:
        raise ContractError(code, message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def revision_of(document):
    """`sha256-` over the canonical JSON of the document without `revision`."""
    return "sha256-" + digest(canonical({k: v for k, v in document.items() if k != "revision"}))


def version(text):
    return tuple(int(part) for part in text.split("."))


# ---------------------------------------------------------------------------------------
# A JSON Schema (draft 2020-12) validator for the keywords these schemas use. An unknown
# keyword is an error, so a schema edit cannot silently weaken the check.
# ---------------------------------------------------------------------------------------

ANNOTATIONS = {"$schema", "$id", "title", "description"}
UNKNOWN_KEY = "unknown key"


class SchemaValidator:
    def __init__(self, schema):
        self.root = schema

    def errors(self, instance, schema=None):
        found = []
        self._check(instance, self.root if schema is None else schema, "$", found)
        return found

    def _resolve(self, ref):
        if not ref.startswith("#/"):
            raise CatalogError(f"Unsupported $ref {ref}")
        node = self.root
        for part in ref[2:].split("/"):
            node = node[part]
        return node

    @staticmethod
    def _type_ok(value, kind):
        if kind == "null":
            return value is None
        if kind == "boolean":
            return isinstance(value, bool)
        if kind == "string":
            return isinstance(value, str)
        if kind == "array":
            return isinstance(value, list)
        if kind == "object":
            return isinstance(value, dict)
        number = isinstance(value, (int, float)) and not isinstance(value, bool)
        if kind == "number":
            return number and math.isfinite(value)
        if kind == "integer":
            return number and math.isfinite(value) and float(value).is_integer()
        raise CatalogError(f"Unsupported type {kind}")

    def _valid(self, instance, schema):
        found = []
        self._check(instance, schema, "$", found)
        return not found

    def _evaluated(self, value, schema):
        """Property names a schema evaluates on `value` (for unevaluatedProperties)."""
        if not isinstance(schema, dict):
            return set()
        names = set(schema.get("properties", {}))
        if "additionalProperties" in schema:
            names |= set(value)
        if "$ref" in schema:
            names |= self._evaluated(value, self._resolve(schema["$ref"]))
        for option in schema.get("allOf", []):
            names |= self._evaluated(value, option)
        for option in schema.get("oneOf", []) + schema.get("anyOf", []):
            if self._valid(value, option):
                names |= self._evaluated(value, option)
        if "if" in schema and self._valid(value, schema["if"]):
            names |= self._evaluated(value, schema.get("then", {}))
        return names

    def prune(self, value, schema=None, path="$", removed=None):
        """A copy of `value` without the keys `schema` does not know (a newer minor's
        additions); their paths are appended to `removed`."""
        schema = self.root if schema is None else schema
        removed = [] if removed is None else removed
        value = copy.deepcopy(value)
        if not isinstance(schema, dict):
            return value
        if "$ref" in schema:
            value = self.prune(value, self._resolve(schema["$ref"]), path, removed)
        for option in schema.get("allOf", []):
            value = self.prune(value, option, path, removed)
        for option in schema.get("oneOf", []):
            trial = []
            candidate = self.prune(value, option, path, trial)
            if self._valid(candidate, option):
                value = candidate
                removed.extend(trial)
                break
        if "if" in schema and "then" in schema and self._valid(value, schema["if"]):
            value = self.prune(value, schema["then"], path, removed)
        if isinstance(value, dict):
            known = schema.get("properties", {})
            for name, child in known.items():
                if name in value:
                    value[name] = self.prune(value[name], child, f"{path}.{name}", removed)
            extra = schema.get("additionalProperties")
            for name in [name for name in value if name not in known]:
                if extra is False:
                    removed.append(f"{path}.{name}")
                    del value[name]
                elif isinstance(extra, dict):
                    value[name] = self.prune(value[name], extra, f"{path}.{name}", removed)
            if schema.get("unevaluatedProperties") is False:
                evaluated = self._evaluated(value, schema)
                for name in [name for name in value if name not in evaluated]:
                    removed.append(f"{path}.{name}")
                    del value[name]
        if isinstance(value, list) and isinstance(schema.get("items"), dict):
            start = len(schema.get("prefixItems", []))
            value[start:] = [self.prune(item, schema["items"], f"{path}[{index}]", removed)
                             for index, item in enumerate(value[start:], start)]
        return value

    def _check(self, value, schema, path, found):
        if schema is True:
            return
        if schema is False:
            found.append(f"{path}: not allowed")
            return
        for key in schema:
            if key in ANNOTATIONS or key in ("$defs", "then"):
                continue
            rule = schema[key]
            if key == "$ref":
                self._check(value, self._resolve(rule), path, found)
            elif key == "type":
                kinds = rule if isinstance(rule, list) else [rule]
                if not any(self._type_ok(value, kind) for kind in kinds):
                    found.append(f"{path}: expected {'/'.join(kinds)}")
                    return
            elif key == "const":
                if not (value == rule and isinstance(value, bool) == isinstance(rule, bool)):
                    found.append(f"{path}: expected {rule!r}")
            elif key == "enum":
                if not any(value == option and isinstance(value, bool) == isinstance(option, bool)
                           for option in rule):
                    found.append(f"{path}: {value!r} is not one of {rule}")
            elif key == "pattern":
                if isinstance(value, str) and re.search(rule, value) is None:
                    found.append(f"{path}: {value!r} does not match {rule}")
            elif key in ("minLength", "maxLength"):
                if isinstance(value, str) and not (len(value) >= rule if key == "minLength" else len(value) <= rule):
                    found.append(f"{path}: length outside {key} {rule}")
            elif key in ("minimum", "maximum", "exclusiveMinimum"):
                if self._type_ok(value, "number"):
                    ok = {"minimum": value >= rule, "maximum": value <= rule, "exclusiveMinimum": value > rule}[key]
                    if not ok:
                        found.append(f"{path}: {value} breaks {key} {rule}")
            elif key == "required":
                if isinstance(value, dict):
                    for name in rule:
                        if name not in value:
                            found.append(f"{path}: missing {name}")
            elif key == "properties":
                if isinstance(value, dict):
                    for name, child in rule.items():
                        if name in value:
                            self._check(value[name], child, f"{path}.{name}", found)
            elif key == "additionalProperties":
                if isinstance(value, dict):
                    known = schema.get("properties", {})
                    for name, child in value.items():
                        if name in known:
                            continue
                        if rule is False:
                            found.append(f"{path}: {UNKNOWN_KEY} {name}")
                        else:
                            self._check(child, rule, f"{path}.{name}", found)
            elif key == "unevaluatedProperties":
                if isinstance(value, dict):
                    evaluated = self._evaluated(value, schema)
                    for name in value:
                        if name not in evaluated:
                            if rule is False:
                                found.append(f"{path}: {UNKNOWN_KEY} {name}")
                            else:
                                self._check(value[name], rule, f"{path}.{name}", found)
            elif key == "propertyNames":
                if isinstance(value, dict):
                    for name in value:
                        self._check(name, rule, f"{path}[{name!r}]", found)
            elif key in ("minProperties", "maxProperties"):
                if isinstance(value, dict) and not (len(value) >= rule if key == "minProperties" else len(value) <= rule):
                    found.append(f"{path}: {len(value)} properties breaks {key} {rule}")
            elif key == "items":
                if isinstance(value, list):
                    start = len(schema.get("prefixItems", []))
                    for index, child in enumerate(value[start:], start):
                        self._check(child, rule, f"{path}[{index}]", found)
            elif key == "prefixItems":
                if isinstance(value, list):
                    for index, (child, item) in enumerate(zip(value, rule)):
                        self._check(child, item, f"{path}[{index}]", found)
            elif key in ("minItems", "maxItems"):
                if isinstance(value, list) and not (len(value) >= rule if key == "minItems" else len(value) <= rule):
                    found.append(f"{path}: {len(value)} items breaks {key} {rule}")
            elif key == "uniqueItems":
                if rule and isinstance(value, list):
                    keys = [canonical(item) for item in value]
                    if len(set(keys)) != len(keys):
                        found.append(f"{path}: duplicate items")
            elif key == "oneOf":
                matches = sum(self._valid(value, option) for option in rule)
                if matches != 1:
                    found.append(f"{path}: matches {matches} of oneOf")
            elif key == "allOf":
                for option in rule:
                    self._check(value, option, path, found)
            elif key == "if":
                if self._valid(value, rule) and "then" in schema:
                    self._check(value, schema["then"], path, found)
            else:
                raise CatalogError(f"Unsupported JSON Schema keyword {key}")


def load_schema(root, schema_file):
    return SchemaValidator(load_json((Path(root) / schema_file).read_bytes(), schema_file))


def schema_errors(root, document, schema_file=SCHEMA_FILE, definition=None):
    validator = load_schema(root, schema_file)
    return validator.errors(document, validator.root["$defs"][definition] if definition else None)


# ---------------------------------------------------------------------------------------
# The reader checks, in the order of GAME-PROTOCOL-SPEC §3.5
# ---------------------------------------------------------------------------------------

def read_contract(data, root=ROOT):
    """Parse and check a contract document the way Studio, Hub and the passport read one;
    raise ContractError with the failed check's code."""
    require(len(data) <= MAX_BYTES, "size", f"{len(data)} bytes exceed the 1 MiB reader cap")
    try:
        document = load_json(data, "contract")
    except CatalogError as error:
        raise ContractError("json", str(error)) from error
    require(isinstance(document, dict) and document.get("schema") == SCHEMA, "schema", f"not an {SCHEMA} document")
    minimum = document.get("minReaderVersion")
    stated = document.get("schemaVersion")
    require(isinstance(minimum, str) and re.fullmatch(r"1\.\d{1,4}", minimum) is not None
            and isinstance(stated, str) and re.fullmatch(r"1\.\d{1,4}", stated) is not None,
            "schema", "schemaVersion and minReaderVersion must be 1.N")
    require(version(minimum) <= READER_VERSION, "reader-version",
            f"minReaderVersion {minimum} needs a newer reader (this one reads {'.'.join(map(str, READER_VERSION))})")
    try:
        canonical(document)
    except CatalogError as error:
        raise ContractError("schema", str(error)) from error
    validator = load_schema(root, SCHEMA_FILE)
    received = document
    if version(stated) > READER_VERSION:
        # A newer minor: known keys stay strict, keys this reader does not know are dropped.
        ignored = []
        document = validator.prune(document, removed=ignored)
        for path in ignored:
            print(f"game contract {stated}: ignored {path}", file=sys.stderr)
    problems = validator.errors(document)
    require(not problems, "schema", "; ".join(problems[:5]))
    # The revision covers the document as received, unknown keys included.
    require(received["revision"] == revision_of(received), "revision",
            f"stated {received['revision']}, recomputed {revision_of(received)}")
    check_references(document)
    for rule in document["rules"]:
        namespaced = rule["id"].count(".") > 1
        require(namespaced or rule["id"] in CORE_RULES, "unknown-rule",
                f"core rule {rule['id']} needs a newer reader")
        require(not namespaced or rule["checkedBy"] != ["tool"], "reference",
                f"namespaced rule {rule['id']} checked only by tools is never enforced")
    check_profiles(document, root)
    return document


def check_references(document):
    def fail(message):
        raise ContractError("reference", message)

    slots, targets = document["slots"], document["targets"]
    families, packaging = document["families"], document["packaging"]
    profiles = document["profiles"]
    if not set(profiles["required"]) <= set(profiles["used"]):
        fail("profiles.required must be a subset of profiles.used")
    for socket_id, socket in document["sockets"].items():
        if socket["rig"] not in document["rigs"]:
            fail(f"socket {socket_id} names unknown rig {socket['rig']}")
    for kind, retired in document["retired"].items():
        live = {"slots": slots, "families": families, "targets": {t["id"] for t in targets}}[kind]
        for retired_id in retired:
            if retired_id in live:
                fail(f"{kind} {retired_id} is both live and retired")
    ids = [rule["id"] for rule in document["rules"]]
    if len(set(ids)) != len(ids):
        fail("duplicate rule ids")
    by_id = {}
    for target in targets:
        if target["id"] in by_id:
            fail(f"duplicate target {target['id']}")
        by_id[target["id"]] = target
    for slot_id, slot in slots.items():
        if slot["kind"] == "animation" and slot["animation"]["rig"] not in document["rigs"]:
            fail(f"{slot_id} names unknown rig")
        if slot["kind"] == "asset" and slot["asset"]["socket"] not in (None, *document["sockets"]):
            fail(f"{slot_id} names unknown socket")
        if slot["customizable"] and slot["reserved"]:
            fail(f"{slot_id}: a reserved slot is not customizable")
        if slot["kind"] == "animation":
            window = slot["animation"]["window"]
            if not window["minSecs"] < window["maxSecs"]:
                fail(f"{slot_id}: empty window")
        for link in ("sharesDefaultWith", "alternatesWith"):
            for other in slot[link]:
                if other not in slots or other == slot_id or slots[other]["kind"] != slot["kind"]:
                    fail(f"{slot_id}.{link} names {other}")
                if slot_id not in slots[other][link]:
                    fail(f"{slot_id}.{link} is not mutual with {other}")
        for user in slot["usedBy"]:
            if user != "*" and (user not in by_id or slot_id not in by_id[user]["slots"]):
                fail(f"{slot_id}.usedBy names {user}")
    for target in targets:
        listed = target["slots"]
        if [slot for section in target["sections"] for slot in section["slots"]] != listed:
            fail(f"target {target['id']}: slots must be its sections in order")
        for slot in listed:
            if slot not in slots:
                fail(f"target {target['id']} names unknown slot {slot}")
        for family in target["families"]:
            if family not in families:
                fail(f"target {target['id']} names unknown family {family}")
        for offer in target["accepts"]:
            kind = offer["package"]
            if kind not in packaging:
                fail(f"target {target['id']}: no packaging for {kind}")
            for slot in offer["slots"] + offer["mustFill"]:
                if slot not in listed:
                    fail(f"target {target['id']}: offer {offer['requirementId']} names {slot}")
                if slots[slot]["kind"] not in packaging[kind]["slotKinds"]:
                    fail(f"target {target['id']}: {slot} cannot be filled by a {kind} package")
            for slot in offer["mustFill"]:
                if slot not in offer["slots"]:
                    fail(f"target {target['id']}: mustFill {slot} is not in its offer")
            for family in offer["families"]:
                if family not in target["families"]:
                    fail(f"target {target['id']}: offer names family {family}")


ANIMATION_SLOT = re.compile(r"(base\.(idle|run|walk|death)|basic\.[a-z][a-z0-9_]{0,31}\.([0-9]|[a-z][a-z0-9_]{0,15}\.[0-9])"
                            r"|skill\.[a-z][a-z0-9_]{0,47}\.(release|recast|windup))\Z")


def check_profiles(document, root):
    """Apply known profiles to `ext`; refuse a required profile this reader does not know."""
    for profile in document["profiles"]["required"]:
        require(profile in KNOWN_PROFILES, "profile", f"the game requires profile {profile}")
    if PROFILE not in document["profiles"]["used"]:
        return
    validator = load_schema(root, PROFILE_SCHEMA_FILE)

    def check(value, definition, where):
        problems = validator.errors(value, validator.root["$defs"][definition])
        require(not problems, "profile", f"{where}: " + "; ".join(problems[:3]))

    def ext(holder):
        return holder["ext"].get(PROFILE)

    require(ext(document) is not None, "profile", f"document has no ext {PROFILE}")
    check(ext(document), "document", "document")
    for target in document["targets"]:
        require(ext(target) is not None, "profile", f"target {target['id']} has no ext")
        check(ext(target), "target", f"target {target['id']}")
        require(all(offer["requirementId"] == f"character.{target['id']}" for offer in target["accepts"]
                    if offer["package"] == "character"), "profile",
                f"target {target['id']}: a character offer is character.{target['id']}")
    require(document["targets"][0]["id"] == "any", "profile", "the first target is `any`")
    for slot_id, slot in document["slots"].items():
        data = ext(slot)
        require(data is not None, "profile", f"{slot_id} has no ext")
        check(data, "animationSlot" if slot["kind"] == "animation" else "assetSlot", slot_id)
        if slot["kind"] != "animation":
            continue
        require(ANIMATION_SLOT.match(slot_id) is not None, "profile", f"{slot_id} breaks the slot grammar")
        parts = slot_id.split(".")
        require(data["group"] == parts[0], "profile", f"{slot_id}: group {data['group']}")
        if parts[0] == "skill":
            require(data["role"] == parts[2] and data.get("skill") == parts[1], "profile", f"{slot_id}: role or skill")
        elif parts[0] == "basic":
            require(data["role"] == "basic" and data.get("class") == parts[1], "profile", f"{slot_id}: role or class")
        else:
            require(data["role"] == parts[1], "profile", f"{slot_id}: role")


# ---------------------------------------------------------------------------------------
# Finishing: the header
# ---------------------------------------------------------------------------------------

def read_source(root, relative):
    path = Path(root) / relative
    if not path.is_file():
        raise CatalogError(f"Missing source file: {relative}")
    return path.read_bytes()


def header_of(root, commit=None):
    """`source` and the profile provenance of the current checkout."""
    hashes = {path: digest(read_source(root, path)) for path in INPUTS}
    motion = load_json(read_source(root, MOTION), MOTION)
    skills = load_json(read_source(root, SKILLS), SKILLS)
    versions = re.findall(rb"pub const PROTOCOL_VERSION: u16 = (\d+);", read_source(root, PROTOCOL))
    if len(versions) != 1:
        raise CatalogError(f"Cannot read PROTOCOL_VERSION from {PROTOCOL}")
    if commit is None:
        commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
    if re.fullmatch(r"[0-9a-f]{40}", commit) is None:
        raise CatalogError("Invalid source commit")
    source = {"commit": commit, "exporter": EXPORTER, "inputsSha256": digest(canonical(hashes))}
    provenance = {
        "gameplayRevision": skills["revision"],
        "presentationSha256": hashes[PRESENTATION],
        "motionLibrary": {"schemaVersion": motion["schema_version"], "sha256": hashes[MOTION],
                          "clips": len(motion["clips"])},
        "protocolVersion": int(versions[0]),
    }
    return source, provenance


def finish(root, document, commit=None):
    """The document with its header: kept while the body is unchanged, else renewed."""
    stated = document.get("revision")
    profile = document.get("ext", {}).get(PROFILE, {})
    consistent = (all(key in document for key in HEADER) and all(key in profile for key in PROVENANCE)
                  and isinstance(stated, str) and stated == revision_of(document))
    if not consistent:
        document = {k: v for k, v in document.items() if k not in HEADER}
        source, provenance = header_of(root, commit)
        document["source"] = source
        document["previousRevision"] = stated if isinstance(stated, str) and REVISION.match(stated) else None
        document.setdefault("ext", {}).setdefault(PROFILE, {}).update(provenance)
    # Canonical numbers (1.0 -> 1) before the hash, as omoba.asset-catalog.v1 does.
    document = json.loads(canonical(document))
    document["revision"] = revision_of(document)
    ordered = {key: document[key] for key in TOP_LEVEL if key in document}
    ordered.update({key: value for key, value in document.items() if key not in TOP_LEVEL})
    return ordered


def render(document):
    return json.dumps(document, indent=2, ensure_ascii=False, allow_nan=False).encode() + b"\n"


def write_output(output, data, check=False):
    output = Path(output).absolute()
    if output.is_symlink():
        raise CatalogError("Output directory cannot be a symlink")
    destination = output / "game-contract.json"
    if check:
        if not destination.is_file() or destination.read_bytes() != data:
            raise CatalogError(f"Stale export file: {destination}; run without --check")
        return
    output.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(destination.name + ".tmp")
    if temporary.exists():
        raise CatalogError(f"Temporary file already exists: {temporary}")
    with temporary.open("xb") as handle:
        handle.write(data)
    temporary.replace(destination)


def export(root=ROOT, output=None, check=False, commit=None):
    root = Path(root).resolve()
    body = load_json(read_source(root, CONTRACT), CONTRACT)
    document = finish(root, body, commit)
    data = render(document)
    read_contract(data, root)
    write_output(output if output is not None else root / CATALOG, data, check)
    return document


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--output", type=Path, required=True,
                        help="Directory for game-contract.json (shared/assets/catalog is the committed one)")
    parser.add_argument("--root", type=Path, default=ROOT, help="Explicit game checkout")
    parser.add_argument("--source-commit", help="Commit to record instead of the checkout's HEAD")
    parser.add_argument("--check", action="store_true", help="Verify an existing export without writing")
    args = parser.parse_args(argv)
    try:
        document = export(args.root, args.output, args.check, args.source_commit)
    except (ContractError, CatalogError, OSError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Game contract export failed: {error}\n")
    slots = document["slots"].values()
    print(f"{'Verified' if args.check else 'Exported'} {len(document['slots'])} slots "
          f"({sum(slot['customizable'] for slot in slots)} customizable), {len(document['targets'])} targets: "
          f"{document['revision']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
