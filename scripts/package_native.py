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

ROOT = Path(__file__).resolve().parents[1]


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
    for line in process.stdout:
        event = json.loads(line)
        if event.get("reason") == "compiler-artifact" and event.get("executable"):
            binaries[event["target"]["name"]] = Path(event["executable"])
    if process.wait() != 0:
        raise RuntimeError("locked native workspace build failed")
    missing = {"client", "server", "bots"} - binaries.keys()
    if missing:
        raise RuntimeError("Cargo did not emit required executables: " + ", ".join(sorted(missing)))
    return binaries


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--profile", choices=("dev", "release"), default="dev")
    args = parser.parse_args()
    source_gate = validate(ROOT / "client/assets")
    if source_gate["status"] != "PASS":
        parser.error("asset content gate failed: " + "; ".join(source_gate["errors"]))
    destination = args.output.resolve()
    if destination.exists():
        parser.error("output must be a new directory; existing packages are never overwritten")
    source_state = source_identity()
    binaries = build_executables(args.profile)
    destination.mkdir(parents=True)
    suffix = ".exe" if os.name == "nt" else ""
    for name in ("client", "server", "bots"):
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
    shutil.copy2(ROOT / "ATTRIBUTION.md", destination / "ATTRIBUTION.md")
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
