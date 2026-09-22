#!/usr/bin/env python3
"""Build a source-independent native Models3d playtest package (host OS only)."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess

from validate_candidate_assets import validate
from package_licenses import collect_legal_notices, copy_legal_notices

ROOT = Path(__file__).resolve().parents[1]
REQUIRED_EXECUTABLES = ("client", "server", "bots", "migrate-career", "omoba-account-api")


def run(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def source_identity():
    untracked = subprocess.check_output(
        ["git", "ls-files", "--others", "--exclude-standard", "-z"], cwd=ROOT
    ).decode().split("\0")
    identity = dict(source_revision=run("git", "rev-parse", "HEAD"),
                    source_dirty=bool(run("git", "status", "--porcelain")),
                    source_diff_sha256=hashlib.sha256(subprocess.check_output(
                        ["git", "diff", "--binary", "HEAD"], cwd=ROOT)).hexdigest(),
                    untracked_source_sha256={name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest()
                                             for name in untracked if name})
    identity["source_identity_sha256"] = hashlib.sha256(
        json.dumps(identity, sort_keys=True).encode()).hexdigest()
    return identity


def build_executables(profile):
    # Cargo emits the actual executable paths, respecting target-dir and target
    # configuration. Never guess a path that could contain an older binary.
    process = subprocess.Popen(
        ["cargo", "build", "--workspace", "--locked", "--profile", profile,
         "--message-format=json-render-diagnostics"], cwd=ROOT, stdout=subprocess.PIPE, text=True)
    binaries = {}
    try:
        for line in process.stdout:
            event = json.loads(line)
            if event.get("reason") == "compiler-artifact" and event.get("executable"):
                binaries[event["target"]["name"]] = Path(event["executable"])
        if process.wait() != 0:
            raise RuntimeError("locked native workspace build failed")
    finally:
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
        process.stdout.close()
    missing = set(REQUIRED_EXECUTABLES) - binaries.keys()
    if missing:
        raise RuntimeError("Cargo did not emit required executables: " + ", ".join(sorted(missing)))
    return binaries


def write_lobby_launcher(destination, suffix=""):
    """Keep mutable worker manifests, receipts and logs outside the package."""
    launcher = destination / "launch-lobby.sh"
    launcher.write_text('''#!/bin/sh
set -eu
umask 077
PACKAGE_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
: "${OMOBA_DATABASE_URL:?Set OMOBA_DATABASE_URL to the migrated career database runtime connection}"
: "${OMOBA_MATCH_ROOT:?Set OMOBA_MATCH_ROOT to an absolute persistent directory outside this package}"
: "${SERVER_ADDR:?Set SERVER_ADDR to the lobby UDP bind address, for example 127.0.0.1:4000}"
: "${OMOBA_MATCH_PUBLIC_HOST:?Set OMOBA_MATCH_PUBLIC_HOST to the worker host reachable by players}"
: "${OMOBA_MATCH_BIND_IP:?Set OMOBA_MATCH_BIND_IP to the local worker bind IP}"
: "${OMOBA_MATCH_FIRST_PORT:?Set OMOBA_MATCH_FIRST_PORT to the first worker UDP port}"
export OMOBA_MATCH_CAPACITY="${OMOBA_MATCH_CAPACITY:-16}"
case "$OMOBA_MATCH_FIRST_PORT" in
    ''|0*|*[!0-9]*|??????*) echo "OMOBA_MATCH_FIRST_PORT must be a UDP port from 1 to 65535" >&2; exit 2 ;;
esac
case "$OMOBA_MATCH_CAPACITY" in
    ''|0*|*[!0-9]*|????*) echo "OMOBA_MATCH_CAPACITY must be from 1 to 100" >&2; exit 2 ;;
esac
if [ "$OMOBA_MATCH_FIRST_PORT" -lt 1 ] || [ "$OMOBA_MATCH_FIRST_PORT" -gt 65535 ] ||
   [ "$OMOBA_MATCH_CAPACITY" -lt 1 ] || [ "$OMOBA_MATCH_CAPACITY" -gt 100 ] ||
   [ "$((OMOBA_MATCH_FIRST_PORT + OMOBA_MATCH_CAPACITY))" -gt 65535 ]; then
    echo "The configured worker capacity and UDP port range are invalid" >&2
    exit 2
fi
case "$OMOBA_MATCH_ROOT" in
    /*) ;;
    *) echo "OMOBA_MATCH_ROOT must be an absolute persistent directory" >&2; exit 2 ;;
esac
case "$OMOBA_MATCH_ROOT/" in
    "$PACKAGE_DIR/"*) echo "OMOBA_MATCH_ROOT must be outside the package directory" >&2; exit 2 ;;
esac
mkdir -p -- "$OMOBA_MATCH_ROOT"
OMOBA_MATCH_ROOT=$(CDPATH= cd -- "$OMOBA_MATCH_ROOT" && pwd -P)
case "$OMOBA_MATCH_ROOT/" in
    "$PACKAGE_DIR/"*) echo "OMOBA_MATCH_ROOT must resolve outside the package directory" >&2; exit 2 ;;
esac
export OMOBA_MATCH_ROOT
export OMOBA_DATABASE_URL SERVER_ADDR OMOBA_MATCH_PUBLIC_HOST OMOBA_MATCH_BIND_IP OMOBA_MATCH_FIRST_PORT
export OMOBA_SERVER_ROLE=lobby
export OMOBA_MATCH_EXECUTABLE="$PACKAGE_DIR/@SERVER_BINARY@"
export OMOBA_CAREER_OUTBOX="$OMOBA_MATCH_ROOT/lobby-outbox"
export OMOBA_ASSET_DIR="$PACKAGE_DIR/assets"
export OMOBA_MATCH_MODE=release
export OMOBA_TEAM_SIZE=5
unset OMOBA_MATCH_ALLOCATION OMOBA_MATCH_RECOVERY OMOBA_MAP_CONFIG OMOBA_TARGETING_QA
cd -- "$OMOBA_MATCH_ROOT"
exec "$OMOBA_MATCH_EXECUTABLE" "$@"
'''.replace("@SERVER_BINARY@", "server" + suffix))
    launcher.chmod(0o755)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--profile", choices=("dev", "release"), default="dev")
    args = parser.parse_args()
    legal_notices = collect_legal_notices(ROOT)
    source_gate = validate(ROOT / "client/assets")
    if source_gate["status"] != "PASS":
        parser.error("asset content gate failed: " + "; ".join(source_gate["errors"]))
    destination = args.output.resolve()
    if destination.exists():
        parser.error("output must be a new directory; existing packages are never overwritten")
    source_state = source_identity()
    binaries = build_executables(args.profile)
    if source_identity() != source_state:
        raise RuntimeError("source changed during build; choose a fresh output and rebuild from a stable revision")
    destination.mkdir(parents=True)
    suffix = ".exe" if os.name == "nt" else ""
    for name in REQUIRED_EXECUTABLES:
        shutil.copy2(binaries[name], destination / (name + suffix))
    # Only versioned assets: no developer SDK/cache/Arena artifacts.
    for relative in run("git", "ls-files", "client/assets").splitlines():
        source = ROOT / relative
        output = destination / "assets" / source.relative_to(ROOT / "client/assets")
        output.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, output)
    # Inspect the bytes that will actually ship, using the repository review policy.
    asset_gate = validate(destination / "assets")
    (destination / "ASSET-REVIEW.json").write_text(json.dumps(asset_gate, indent=2) + "\n")
    if asset_gate["status"] != "PASS":
        raise RuntimeError("packaged asset content gate failed: " + "; ".join(asset_gate["errors"]))
    for kind in ("client", "server", "bots"):
        launcher = destination / f"launch-{kind}.sh"
        launcher.write_text('''#!/bin/sh
set -eu
PACKAGE_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
export OMOBA_ASSET_DIR="$PACKAGE_DIR/assets"
unset OMOBA_AVATAR_MANIFEST
export OMOBA_PLAYER_VISUAL_MODE=models3d
export OMOBA_DEBUG_UI=0
export OMOBA_CLIENT_CONFIG_DIR="${OMOBA_CLIENT_CONFIG_DIR:-$PACKAGE_DIR/user-data}"
export OMOBA_MATCH_MODE="${OMOBA_MATCH_MODE:-release}"
export OMOBA_TEAM_SIZE="${OMOBA_TEAM_SIZE:-5}"
exec "$PACKAGE_DIR/''' + kind + '''" "$@"
''')
        launcher.chmod(0o755)
    write_lobby_launcher(destination, suffix)
    shutil.copy2(ROOT / "scripts/beta_launcher.py", destination / "beta.py")
    for name, action in (("join-server", "join"), ("practice", "practice"), ("host", "host")):
        launcher = destination / f"{name}.sh"
        launcher.write_text('''#!/bin/sh
set -eu
PACKAGE_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec python3 "$PACKAGE_DIR/beta.py" ''' + action + ''' "$@"
''')
        launcher.chmod(0o755)
    shutil.copy2(ROOT / "docs/progress/2026-09-07-beta-test-guide.md", destination / "TESTING.md")
    shutil.copy2(ROOT / "docs/public-mvp.md", destination / "PUBLIC-MVP.md")
    shutil.copy2(ROOT / "ATTRIBUTION.md", destination / "ATTRIBUTION.md")
    # Keep server/LICENSE under legal/: the package root contains a server executable.
    copy_legal_notices(legal_notices, destination / "legal")
    shutil.copy2(ROOT / "art/verdant-confluence/PROVENANCE.md", destination / "VERDANT-PROVENANCE.md")
    shutil.copy2(ROOT / "assets-src/animations/README.md", destination / "ANIMATION-ATTRIBUTION.md")
    shutil.copy2(ROOT / "docs/progress/2026-09-05-distribution-review.md", destination / "2026-09-05-distribution-review.md")
    metadata = json.loads(run("cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"))
    version = next(item["version"] for item in metadata["packages"] if item["name"] == "client")
    files = {str(p.relative_to(destination)): hashlib.sha256(p.read_bytes()).hexdigest()
             for p in sorted(destination.rglob("*")) if p.is_file()}
    if source_identity() != source_state:
        raise RuntimeError("source changed while packaging; choose a fresh output and rebuild from a stable revision")
    identity = dict(version=version, **source_state,
                    platform=platform.platform(), machine=platform.machine(), profile=args.profile,
                    asset_content_gate="PASS",
                    certification="Controlled native beta candidate; see TESTING.md for measured checks and remaining human, platform/network and dependency limitations",
                    sha256=files)
    (destination / "BUILD.json").write_text(json.dumps(identity, indent=2) + "\n")
    print(json.dumps({"package": str(destination), "version": version, "files": len(files),
                      "bytes": sum(p.stat().st_size for p in destination.rglob("*") if p.is_file())}))


if __name__ == "__main__":
    main()
