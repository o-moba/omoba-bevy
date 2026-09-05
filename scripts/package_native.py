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
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def run(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--profile", choices=("dev", "release"), default="dev")
    parser.add_argument("--internal-review", action="store_true",
                        help="acknowledge unresolved asset provenance; do not distribute this package")
    args = parser.parse_args()
    if not args.internal_review:
        parser.error("asset provenance conflicts block distribution; use --internal-review only for local review (see TESTING guide RG5)")
    destination = args.output.resolve()
    if destination.exists():
        parser.error("output must be a new directory; existing packages are never overwritten")
    subprocess.run(["cargo", "build", "--workspace", "--locked", "--profile", args.profile],
                   cwd=ROOT, check=True)
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    if not target.is_absolute():
        target = ROOT / target
    binaries = target / ("debug" if args.profile == "dev" else "release")
    destination.mkdir(parents=True)
    suffix = ".exe" if os.name == "nt" else ""
    for name in ("client", "server", "bots"):
        shutil.copy2(binaries / (name + suffix), destination / (name + suffix))
    # Only versioned assets: no developer SDK/cache/Arena artifacts.
    for relative in run("git", "ls-files", "client/assets").splitlines():
        source = ROOT / relative
        output = destination / "assets" / source.relative_to(ROOT / "client/assets")
        output.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, output)
    for name in ("avatars", "bosses", "minions"):
        manifest = json.loads((destination / "assets" / name / "manifest.json").read_text())
        for avatar in manifest["avatars"]:
            if not (destination / "assets" / name / (avatar["slug"] + ".glb")).is_file():
                raise RuntimeError(f"missing required {name} model: {avatar['slug']}")
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
    shutil.copy2(ROOT / "docs/progress/2026-09-05-release-test-guide.md", destination / "TESTING.md")
    shutil.copy2(ROOT / "docs/progress/2026-09-05-distribution-review.md", destination / "2026-09-05-distribution-review.md")
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    files = {str(p.relative_to(destination)): hashlib.sha256(p.read_bytes()).hexdigest()
             for p in sorted(destination.rglob("*")) if p.is_file()}
    identity = dict(version=version, source_revision=run("git", "rev-parse", "HEAD"),
                    source_dirty=bool(run("git", "status", "--porcelain")),
                    source_diff_sha256=hashlib.sha256(run("git", "diff", "HEAD").encode()).hexdigest(),
                    platform=platform.platform(), machine=platform.machine(), profile=args.profile,
                    certification="INTERNAL REVIEW ONLY: asset provenance and external release gates BLOCKED", sha256=files)
    (destination / "BUILD.json").write_text(json.dumps(identity, indent=2) + "\n")
    print(json.dumps({"package": str(destination), "version": version, "files": len(files),
                      "bytes": sum(p.stat().st_size for p in destination.rglob("*") if p.is_file())}))


if __name__ == "__main__":
    main()
