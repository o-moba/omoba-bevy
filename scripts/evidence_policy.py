#!/usr/bin/env python3
"""Keep bulky evidence payloads out of the main Git index."""

from __future__ import annotations

import argparse
from pathlib import Path
import subprocess

MAX_JSON = 16 * 1024
MAX_MANIFEST = 64 * 1024
MAX_VERIFIER = 128 * 1024


def violation(name: str, size: int) -> str | None:
    path = Path(name)
    if not name.startswith("docs/evidence/"):
        return None
    if name in {
        "docs/evidence/README.md",
        "docs/evidence/ARCHIVE.json",
        "docs/evidence/restore.py",
    }:
        return None
    if path.suffix == ".md":
        return None
    if path.name.startswith("verify") and path.suffix == ".py":
        return f"{name}: verifier exceeds 128 KiB" if size > MAX_VERIFIER else None
    if path.name == "manifest.json":
        return f"{name}: manifest exceeds 64 KiB" if size > MAX_MANIFEST else None
    if path.suffix == ".json":
        return f"{name}: summary JSON exceeds 16 KiB; archive the payload" if size > MAX_JSON else None
    return f"{name}: bulky evidence type {path.suffix or '(none)'}; use the external archive"


def tracked_files(repo: Path) -> list[str]:
    return subprocess.check_output(
        ["git", "ls-files", "docs/evidence"], cwd=repo, text=True
    ).splitlines()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    repo = args.repo.resolve()
    errors = []
    total = 0
    for name in tracked_files(repo):
        size = len((repo / name).read_bytes())
        total += 1
        error = violation(name, size)
        if error:
            errors.append(error)
    if errors:
        raise SystemExit("Evidence policy failed:\n" + "\n".join(errors))
    print(f"PASS tracked_evidence={total}; bulky payloads remain external")


if __name__ == "__main__":
    main()
