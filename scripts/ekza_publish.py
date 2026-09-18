#!/usr/bin/env python3
"""Publish an on-chain Ekza avatar template into a registry catalogue for Omoba.

The storefront lets an artist upload a VRM and create a template on Solana
(`initialize_avatar`). Nothing turned that template into a registry catalogue
entry, and nothing produced the `desktop/humanoid-glb-v1` rendition Omoba
loads. This operator tool closes that gap:

    chain template -> metadata -> source VRM (verified)
                   -> Omoba GLB with idle/walk/attack/cast/death clips
                   -> catalogue item + immutable content-addressed assets
                   -> explicit operator approval record

It never signs or sends a transaction, never invents approval (the operator
names the projects and signs the record with --reviewed-by), and never deploys:
the output is a catalogue file and an asset directory that the Ekza registry
(`ekza-mirror/backend`) serves through EKZA_CATALOG_PATH / EKZA_ASSET_DIR.

    # what is on chain
    python3 scripts/ekza_publish.py list

    # publish template #18 for Omoba (and its source VRM for Ekza Space)
    python3 scripts/ekza_publish.py publish --index 18 \
        --catalog demo/ekza-registry/catalog.json \
        --asset-dir demo/ekza-registry/assets \
        --approve omoba --approve ekza-space --reviewed-by "Dima"

Standard library only.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import http.client
import json
import os
import re
import struct
import sys
import tempfile
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
REPO = SCRIPTS.parent
sys.path.insert(0, str(SCRIPTS))

MINTER_PROGRAM = "29KLLArkfCfRGPgTh4k4qzXvR2JkkXfRnnNZTKn54TKz"
DEFAULT_RPC = "https://api.devnet.solana.com"
NETWORK = "solana-devnet"
CATALOG_SCHEMA = "ekza.avatar.catalog.v1"
APPROVAL_SCHEMA = "ekza.passport.approval.v1"
# AvatarData: 8 discriminator + (4 + 64) uri + 32 creator + 5 * u64 + bump.
AVATAR_DATA_SIZE = 8 + 4 + 64 + 32 + 8 * 5 + 1
AVATAR_DATA_DISCRIMINATOR = hashlib.sha256(b"account:AvatarData").digest()[:8]
MAX_MODEL_BYTES = 50 * 1024 * 1024
MAX_METADATA_BYTES = 1024 * 1024
IPFS_GATEWAYS = (
    "https://ekza.mypinata.cloud/ipfs/",
    "https://gateway.pinata.cloud/ipfs/",
)
REQUIRED_CLIPS = ("idle", "walk", "attack", "cast", "death")
# project id -> the exact selector that project's runtime loads.
PROJECT_SELECTORS = {
    "omoba": ("desktop", "humanoid-glb-v1"),
    "ekza-space": ("universal", None),  # profile follows the VRM version
}
B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"


class PublishError(Exception):
    pass


# --- small helpers -------------------------------------------------------------


def b58encode(data: bytes) -> str:
    number = int.from_bytes(data, "big")
    out = ""
    while number:
        number, rem = divmod(number, 58)
        out = B58[rem] + out
    return "1" * (len(data) - len(data.lstrip(b"\0"))) + out


def b58decode(text: str) -> bytes:
    number = 0
    for char in text:
        number = number * 58 + B58.index(char)
    body = number.to_bytes((number.bit_length() + 7) // 8, "big")
    return b"\0" * (len(text) - len(text.lstrip("1"))) + body


_P = 2**255 - 19
_D = (-121665 * pow(121666, _P - 2, _P)) % _P


def on_ed25519_curve(point: bytes) -> bool:
    """True when 32 bytes decompress to a curve point (so they are NOT a PDA)."""
    y = int.from_bytes(point, "little") & ((1 << 255) - 1)
    if y >= _P:
        return False
    x2 = ((y * y - 1) * pow(_D * y * y + 1, _P - 2, _P)) % _P
    return x2 == 0 or pow(x2, (_P - 1) // 2, _P) == 1


def find_program_address(seeds: list, program: str) -> str:
    program_id = b58decode(program)
    for bump in range(255, -1, -1):
        digest = hashlib.sha256(
            b"".join(seeds) + bytes([bump]) + program_id + b"ProgramDerivedAddress"
        ).digest()
        if not on_ed25519_curve(digest):
            return b58encode(digest)
    raise PublishError("no program address for these seeds")


def template_pda(index: int, program: str) -> str:
    return find_program_address([b"avatar_v1", struct.pack("<Q", index)], program)


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


class _Http10Connection(http.client.HTTPSConnection):
    # Some proxies corrupt long chunked transfers for Python's HTTP/1.1 client.
    # HTTP/1.0 responses are never chunked. Truncation stays detectable: GLB
    # files declare their length and every JSON body must parse.
    _http_vsn = 10
    _http_vsn_str = "HTTP/1.0"


class _Http10Handler(urllib.request.HTTPSHandler):
    def https_open(self, req):
        return self.do_open(_Http10Connection, req)


OPENER = urllib.request.build_opener(_Http10Handler)
NETWORK_ERRORS = (urllib.error.URLError, TimeoutError, http.client.HTTPException, OSError)


def with_retries(what: str, attempt):
    """Public gateways and RPC nodes drop connections; five tries, then fail."""
    last = None
    for delay in (0, 2, 4, 8, 15):
        time.sleep(delay)
        try:
            return attempt()
        except urllib.error.HTTPError as error:
            if error.code not in (429, 500, 502, 503, 504):
                raise PublishError(f"{what}: HTTP {error.code}") from error
            last = error
        except NETWORK_ERRORS as error:
            last = error
    raise PublishError(f"{what}: {last}")


def glb_complete(data: bytes) -> bool:
    return len(data) >= 12 and len(data) >= struct.unpack_from("<I", data, 8)[0]


def json_complete(data: bytes) -> bool:
    try:
        json.loads(data)
    except ValueError:
        return False
    return True


def http_get(url: str, limit: int, complete) -> bytes:
    """Bounded HTTPS download. Gateways cut large transfers and HTTP/1.0 cannot
    signal that, so `complete` decides from the bytes themselves; a short body
    resumes with a Range request instead of starting over."""
    if not url.startswith("https://"):
        raise PublishError(f"refusing non-HTTPS download: {url}")
    data = bytearray()
    for _ in range(12):
        headers = {"User-Agent": "ekza-publish/1"}
        if data:
            headers["Range"] = f"bytes={len(data)}-"
        request = urllib.request.Request(url, headers=headers)

        def attempt():
            with OPENER.open(request, timeout=120) as response:
                if data and response.status != 206:
                    del data[:]  # server ignored Range; start over
                try:
                    while len(data) <= limit:
                        chunk = response.read(256 * 1024)
                        if not chunk:
                            break
                        data.extend(chunk)
                except http.client.IncompleteRead as error:
                    data.extend(error.partial)

        with_retries(f"download failed: {url}", attempt)
        if len(data) > limit:
            raise PublishError(f"{url} exceeds {limit} bytes")
        if complete(bytes(data)):
            return bytes(data)
    raise PublishError(f"download kept breaking: {url}")


def rpc(url: str, method: str, params: list):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    request = urllib.request.Request(
        url,
        data=body,
        headers={
            "Content-Type": "application/json",
            "User-Agent": "ekza-publish/1",
            "Connection": "close",
        },
    )

    def attempt():
        with OPENER.open(request, timeout=60) as response:
            try:
                return json.loads(response.read())
            except json.JSONDecodeError as error:
                # A cut connection can surface as a short, unparsable body.
                raise OSError(f"truncated RPC response: {error}") from error

    reply = with_retries("Solana RPC unreachable", attempt)
    if "error" in reply:
        raise PublishError(f"Solana RPC error: {reply['error'].get('message', reply['error'])}")
    return reply["result"]


def canonical_uri(url: str) -> str:
    """https gateway URL -> ar:// or ipfs:// identity when recognisable."""
    match = re.fullmatch(r"https://arweave\.net/([A-Za-z0-9_-]{43})", url)
    if match:
        return f"ar://{match.group(1)}"
    match = re.fullmatch(r"https://[^/]+/ipfs/([A-Za-z0-9]{46,64})", url)
    if match:
        return f"ipfs://{match.group(1)}"
    return url


def fetch_uri(uri: str, limit: int, complete) -> bytes:
    if uri.startswith("ar://"):
        return http_get("https://arweave.net/" + uri[5:], limit, complete)
    if uri.startswith("ipfs://"):
        last = None
        for gateway in IPFS_GATEWAYS:
            try:
                return http_get(gateway + uri[7:], limit, complete)
            except PublishError as error:
                last = error
        raise last or PublishError(f"no IPFS gateway for {uri}")
    return http_get(uri, limit, complete)


# --- chain ---------------------------------------------------------------------


def decode_avatar_data(pubkey: str, raw: bytes) -> dict:
    if len(raw) < 8 + 4 or raw[:8] != AVATAR_DATA_DISCRIMINATOR:
        raise PublishError(f"{pubkey} is not an AvatarData account")
    (length,) = struct.unpack_from("<I", raw, 8)
    if length > 64 or len(raw) < 12 + length + 32 + 41:
        raise PublishError(f"{pubkey} has a malformed AvatarData layout")
    offset = 12
    uri = raw[offset : offset + length].decode("utf-8")
    offset += length
    creator = b58encode(raw[offset : offset + 32])
    offset += 32
    max_supply, current_supply, fee, unclaimed, index = struct.unpack_from("<5Q", raw, offset)
    return {
        "pda": pubkey,
        "uri": uri,
        "creator": creator,
        "maxSupply": max_supply,
        "currentSupply": current_supply,
        "mintingFeeLamports": fee,
        "index": index,
    }


def load_templates(rpc_url: str, program: str) -> list[dict]:
    accounts = rpc(
        rpc_url,
        "getProgramAccounts",
        [program, {"encoding": "base64", "filters": [{"dataSize": AVATAR_DATA_SIZE}]}],
    )
    templates = []
    for account in accounts:
        raw = base64.b64decode(account["account"]["data"][0])
        if raw[:8] != AVATAR_DATA_DISCRIMINATOR:
            continue
        templates.append(decode_avatar_data(account["pubkey"], raw))
    return sorted(templates, key=lambda template: template["index"])


# --- model checks --------------------------------------------------------------


def glb_document(data: bytes) -> dict:
    if len(data) < 20 or struct.unpack_from("<4sII", data) != (b"glTF", 2, len(data)):
        raise PublishError("model is not a complete binary glTF v2 file")
    json_length, chunk_type = struct.unpack_from("<II", data, 12)
    if chunk_type != 0x4E4F534A or 20 + json_length > len(data):
        raise PublishError("model has an invalid glTF JSON chunk")
    return json.loads(data[20 : 20 + json_length])


def vrm_format(document: dict) -> str:
    extensions = document.get("extensions", {})
    if "VRMC_vrm" in extensions:
        return "vrm1"
    if "VRM" in extensions:
        return "vrm0"
    raise PublishError("source model carries no VRM humanoid metadata")


def validate_omoba_profile(data: bytes) -> None:
    """Mirror of omoba_passport::validate_humanoid_profile."""
    document = glb_document(data)
    nodes = document.get("nodes") or []
    skins = document.get("skins") or []
    if not nodes or not skins:
        raise PublishError("Omoba rendition has no skinned skeleton")
    for skin in skins:
        joints = skin.get("joints") or []
        if not joints or any(not isinstance(j, int) or j >= len(nodes) for j in joints):
            raise PublishError("Omoba rendition has invalid joint references")
    for field in ("buffers", "images"):
        if any("uri" in entry for entry in document.get(field, [])):
            raise PublishError("Omoba rendition must embed every buffer and texture")
    animations = {clip.get("name"): clip for clip in document.get("animations", [])}
    for name in REQUIRED_CLIPS:
        channels = (animations.get(name) or {}).get("channels") or []
        if not channels or any(
            not isinstance(c.get("target", {}).get("node"), int) or c["target"]["node"] >= len(nodes)
            for c in channels
        ):
            raise PublishError(f"Omoba rendition is missing a valid '{name}' clip")


def build_omoba_rendition(source: bytes, verbose: bool) -> bytes:
    """VRM -> GLB with Omoba's five retargeted clips (scripts/retarget_animations.py)."""
    import retarget_animations as retarget

    library = REPO / retarget.SOURCE_GLTF
    if not library.is_file():
        raise PublishError(f"animation library missing: {library}")
    rig = retarget.SourceRig(retarget.Gltf.from_gltf_file(library))
    clips = {name: rig.clip(name) for name, _ in retarget.CLIP_MAP}
    with tempfile.TemporaryDirectory(prefix="ekza-publish-") as directory:
        path = Path(directory) / "avatar.glb"
        path.write_bytes(source)
        try:
            mapped = retarget.retarget_avatar(rig, clips, path, verbose=verbose)
        except (ValueError, KeyError, OSError, struct.error) as error:
            raise PublishError(f"animation retarget failed: {error}") from error
        data = path.read_bytes()
    print(f"  retargeted {len(REQUIRED_CLIPS)} clips onto {mapped} bones")
    validate_omoba_profile(data)
    return data


# --- catalogue -----------------------------------------------------------------


def rendition_entry(data: bytes, platform: str, profile: str, fmt: str, canonical: str | None):
    sha = sha256_hex(data)
    extension = "vrm" if fmt in ("vrm0", "vrm1") else fmt
    return {
        "assetPath": f"/v1/assets/{sha}.{extension}",
        "canonicalUri": canonical or f"urn:sha256:{sha}",
        "downloadUrl": None,
        "format": fmt,
        "id": f"sha256:{sha}",
        "mediaType": "model/vrm" if extension == "vrm" else "model/gltf-binary",
        "minOsVersion": None,
        "platform": platform,
        "profile": profile,
        "rigFit": None,
        "sha256": sha,
        "sizeBytes": len(data),
        "status": "ready",
        "units": "meters",
    }, f"{sha}.{extension}"


def store_asset(asset_dir: Path, name: str, data: bytes) -> None:
    asset_dir.mkdir(parents=True, exist_ok=True)
    destination = asset_dir / name
    if destination.is_symlink():
        raise PublishError(f"immutable asset destination is a symlink: {destination}")
    if destination.exists():
        if destination.read_bytes() != data:
            raise PublishError(f"immutable asset exists with different bytes: {destination}")
        return
    temporary = destination.with_name(f".{name}.part-{os.getpid()}")
    temporary.write_bytes(data)
    temporary.replace(destination)


def license_from_metadata(metadata: dict, override: str | None) -> dict:
    if override:
        return {"spdx": override, "attribution": None}
    for attribute in metadata.get("attributes", []):
        if str(attribute.get("trait_type", "")).lower() == "license":
            value = str(attribute.get("value", "")).strip()
            if value.upper().replace(" ", "").replace("-", "") in ("CC0", "CC01.0", "CC010"):
                return {"spdx": "CC0-1.0", "attribution": None}
            if value:
                return {"spdx": value.replace(" ", "-"), "attribution": None}
    raise PublishError("template metadata states no license; pass --license <SPDX id>")


def model_uri_from_metadata(metadata: dict) -> str:
    for entry in (metadata.get("properties") or {}).get("files", []):
        if "vrm" in str(entry.get("type", "")).lower() or str(entry.get("uri", "")).endswith(".vrm"):
            return entry["uri"]
    if metadata.get("animation_url"):
        return metadata["animation_url"]
    raise PublishError("template metadata references no VRM model")


def display_name(metadata: dict) -> str:
    name = str(metadata.get("name", "")).strip()
    # Storefront NFT names look like "Ekza Avatar — Robert".
    return re.sub(r"^Ekza Avatar\s*[—-]\s*", "", name) or "Ekza avatar"


def publish(args) -> int:
    # One small account read; the address of template N is derived locally.
    pda = args.pda or template_pda(args.index, args.program)
    info = rpc(args.rpc, "getAccountInfo", [pda, {"encoding": "base64"}])["value"]
    if info is None or info["owner"] != args.program:
        raise PublishError("no such template on chain; run `list` to see indexes")
    template = decode_avatar_data(pda, base64.b64decode(info["data"][0]))
    if args.index is not None and template["index"] != args.index:
        raise PublishError("derived account does not carry the requested index")
    print(f"Template #{template['index']} {template['pda']} by {template['creator']}")

    metadata_uri = template["uri"] if "://" in template["uri"] else f"ipfs://{template['uri']}"
    metadata_bytes = fetch_uri(metadata_uri, MAX_METADATA_BYTES, json_complete)
    metadata = json.loads(metadata_bytes)
    name = display_name(metadata)
    source_url = model_uri_from_metadata(metadata)
    source_uri = canonical_uri(source_url)
    print(f"  {name}: source {source_uri}")

    source = fetch_uri(source_uri, MAX_MODEL_BYTES, glb_complete)
    fmt = vrm_format(glb_document(source))
    print(f"  source VRM verified: {fmt}, {len(source)} bytes, sha256 {sha256_hex(source)}")

    approvals = []
    renditions = []
    assets = []
    for project in dict.fromkeys(args.approve):
        if project not in PROJECT_SELECTORS:
            raise PublishError(f"unknown project '{project}' (known: {', '.join(PROJECT_SELECTORS)})")
        platform, profile = PROJECT_SELECTORS[project]
        if project == "omoba":
            data = build_omoba_rendition(source, args.verbose)
            entry, asset = rendition_entry(data, platform, profile, "glb", None)
        else:
            profile = "vrm-humanoid-v0" if fmt == "vrm0" else "vrm-humanoid-v1"
            data = source
            entry, asset = rendition_entry(data, platform, profile, fmt, source_uri)
        renditions.append(entry)
        assets.append((asset, data))
        approvals.append(
            {"platform": platform, "profile": profile, "projectId": project, "status": "approved"}
        )
        print(f"  {project}: {platform}/{profile} {entry['format']} {entry['sizeBytes']} bytes")

    item = {
        "id": f"solana:devnet:avatar-data:{template['pda']}",
        "license": license_from_metadata(metadata, args.license),
        "name": name,
        "projectSupport": approvals,
        "provenance": {
            "avatarDataIndex": template["index"],
            "avatarDataPda": template["pda"],
            "creator": template["creator"],
            "metadataSha256": sha256_hex(metadata_bytes),
            "metadataUri": metadata_uri,
            "network": NETWORK,
            "program": args.program,
            "sourceModelUri": source_uri,
        },
        "renditions": renditions,
        "thumbnailUrl": metadata.get("image") if str(metadata.get("image", "")).startswith("https://") else None,
    }

    catalog_path: Path = args.catalog
    if catalog_path.exists():
        catalog = json.loads(catalog_path.read_text())
        if catalog.get("schema") != CATALOG_SCHEMA or catalog.get("program") != args.program:
            raise PublishError(f"{catalog_path} is not a catalogue of this program")
    else:
        catalog = {"schema": CATALOG_SCHEMA, "network": NETWORK, "program": args.program, "items": []}
    previous = next((i for i in catalog["items"] if i["id"] == item["id"]), None)
    if previous is not None:
        # Keep renditions/approvals this run did not touch (e.g. the iOS USDZ).
        touched = {(r["platform"], r["profile"]) for r in renditions}
        item["renditions"] += [
            r for r in previous["renditions"] if (r["platform"], r["profile"]) not in touched
        ]
        named = {a["projectId"] for a in approvals}
        kept = {(r["platform"], r["profile"]) for r in item["renditions"]}
        item["projectSupport"] += [
            a
            for a in previous.get("projectSupport", [])
            if a["projectId"] not in named and (a["platform"], a["profile"]) in kept
        ]
    catalog["items"] = [i for i in catalog["items"] if i["id"] != item["id"]] + [item]
    catalog["generatedAt"] = datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")

    if args.dry_run:
        print(json.dumps(item, indent=2))
        return 0
    for asset, data in assets:
        store_asset(args.asset_dir, asset, data)
    catalog_path.parent.mkdir(parents=True, exist_ok=True)
    temporary = catalog_path.with_name(f".{catalog_path.name}.part-{os.getpid()}")
    temporary.write_text(json.dumps(catalog, indent=2, sort_keys=True) + "\n")
    temporary.replace(catalog_path)

    record = {
        "schema": APPROVAL_SCHEMA,
        "avatarId": item["id"],
        "sourceModelUri": source_uri,
        "sourceSha256": sha256_hex(source),
        "renditions": {r["platform"] + "/" + r["profile"]: r["sha256"] for r in renditions},
        "support": approvals,
        "reviewedBy": args.reviewed_by,
        "reviewedAt": catalog["generatedAt"],
        "priceLamports": template["mintingFeeLamports"],
    }
    approvals_dir = catalog_path.parent / "approvals"
    approvals_dir.mkdir(exist_ok=True)
    (approvals_dir / f"{template['pda']}.json").write_text(json.dumps(record, indent=2) + "\n")
    print(f"Published {name} -> {catalog_path} ({len(catalog['items'])} item(s))")
    print(f"Price on the storefront: {template['mintingFeeLamports'] / 1e9:g} SOL")
    return 0


def list_templates(args) -> int:
    templates = load_templates(args.rpc, args.program)
    print(f"{len(templates)} template(s) in {args.program}")
    for template in templates:
        print(
            f"  #{template['index']:<4} {template['pda']}  "
            f"{template['currentSupply']}/{template['maxSupply']} minted  "
            f"{template['mintingFeeLamports'] / 1e9:g} SOL  {template['uri']}"
        )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--rpc", default=os.environ.get("OMOBA_SOLANA_RPC_URL", DEFAULT_RPC))
    parser.add_argument("--program", default=MINTER_PROGRAM)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("list", help="show avatar templates on chain")
    pub = commands.add_parser("publish", help="add one template to a registry catalogue")
    target = pub.add_mutually_exclusive_group(required=True)
    target.add_argument("--index", type=int)
    target.add_argument("--pda")
    pub.add_argument("--catalog", type=Path, required=True)
    pub.add_argument("--asset-dir", type=Path, required=True)
    pub.add_argument("--approve", action="append", required=True, metavar="PROJECT")
    pub.add_argument("--reviewed-by", required=True, help="operator who reviewed and approves")
    pub.add_argument("--license", help="SPDX id when the metadata states none")
    pub.add_argument("--dry-run", action="store_true")
    pub.add_argument("--verbose", action="store_true")
    args = parser.parse_args()
    try:
        return list_templates(args) if args.command == "list" else publish(args)
    except PublishError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
