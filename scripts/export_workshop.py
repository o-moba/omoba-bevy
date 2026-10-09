#!/usr/bin/env python3
"""Export compiled game-owned workshop rules with reproducible source provenance."""
import argparse
import json
import os
from pathlib import Path
import subprocess

from export_asset_catalog import canonical, digest

ROOT = Path(__file__).resolve().parents[1]


def target_directory():
    if os.environ.get("CARGO_TARGET_DIR"):
        return Path(os.environ["CARGO_TARGET_DIR"]).resolve()
    common = subprocess.check_output(
        ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"], cwd=ROOT, text=True).strip()
    return Path(common).parent / "target"


def rust_command(*flags):
    return ["cargo", "run", "--quiet", "--locked", "-p", "shared", "--example", "export_workshop", "--", *flags]


def rust_environment():
    return {**os.environ, "CARGO_TARGET_DIR": str(target_directory())}


def source_commit(root, paths):
    """A tracked snapshot or unrelated docs commit cannot stale its own provenance."""
    relative = [str(Path(path).relative_to(root)) for path in paths]
    revision = subprocess.check_output(
        ["git", "log", "-1", "--format=%H", "--", *relative], cwd=root, text=True).strip()
    if not revision:
        raise ValueError("No committed source revision exists for the workshop inputs")
    return revision


def export_catalog():
    data = json.loads(subprocess.check_output(rust_command(), cwd=ROOT, env=rust_environment(), text=True))
    paths = [*sorted((ROOT / "shared/src").rglob("*.rs")), ROOT / "shared/examples/export_workshop.rs",
             ROOT / "shared/Cargo.toml", ROOT / "Cargo.toml", ROOT / "Cargo.lock", Path(__file__),
             ROOT / "scripts/export_asset_catalog.py",
             *(ROOT / "shared/assets/catalog" / f"{name}.json" for name in ("heroes", "skills", "items"))]
    inputs = {str(path.relative_to(ROOT)): digest(path.read_bytes()) for path in paths}
    data["source"] = {"commit": source_commit(ROOT, paths),
                      "inputsSha256": digest(canonical(inputs))}
    data = json.loads(canonical(data))
    data["revision"] = "sha256-" + digest(canonical(data))
    return data


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "shared/assets/catalog/workshop.json")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args(argv)
    try:
        data = export_catalog()
        encoded = json.dumps(data, ensure_ascii=False, indent=2) + "\n"
        if args.check:
            if args.output.read_text() != encoded:
                parser.exit(1, "Workshop snapshot is stale; regenerate from this game checkout.\n")
        else:
            args.output.write_text(encoded)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Workshop export failed: {error}\n")
    print(f"{'Verified' if args.check else 'Exported'} {len(data['cores'])} cores / {len(data['skills'])} skills: {data['revision']}")


if __name__ == "__main__":
    main()
