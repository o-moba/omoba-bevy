#!/usr/bin/env python3
"""Collect project notices for native and mobile packages using only the stdlib."""
from __future__ import annotations

import json
from pathlib import Path
import subprocess
import zipfile


REQUIRED_FILES = (
    "LICENSE",
    "LICENSING.md",
    "MISSION.md",
    "CONTRIBUTING.md",
    "TRADEMARKS.md",
    "SOURCE.md",
    "ATTRIBUTION.md",
    "server/LICENSE",
    "LICENSES/AGPL-3.0-only.txt",
    "LICENSES/MPL-2.0.txt",
    "LICENSES/CC-BY-4.0.txt",
)


def collect_legal_notices(root: Path) -> dict[str, bytes]:
    """Validate every required notice before any package is built or written."""
    paths = set(REQUIRED_FILES)
    paths.update(path.relative_to(root).as_posix() for path in (root / "LICENSES").glob("*.txt"))
    notices = {}
    for relative in sorted(paths):
        source = root / relative
        if not source.is_file():
            raise RuntimeError(f"Required legal notice is missing: {source}")
        contents = source.read_bytes()
        try:
            text = contents.decode("utf-8")
        except UnicodeDecodeError as error:
            raise RuntimeError(f"Legal notice is not UTF-8 text: {source}") from error
        if not text.strip():
            raise RuntimeError(f"Required legal notice is empty: {source}")
        notices[relative] = contents
    revision = subprocess.check_output(
        ["git", "rev-parse", "--verify", "HEAD"], cwd=root, text=True
    ).strip()
    dirty = bool(subprocess.check_output(
        ["git", "status", "--porcelain"], cwd=root, text=True
    ).strip())
    notices["SOURCE-REVISION.json"] = (json.dumps({
        "source_revision": revision,
        "source_dirty": dirty,
        "source_publication_verified": False,
        "notice": (
            "Informational checkout identity recorded before the build. A revision alone "
            "does not identify uncommitted changes. This metadata does not verify source "
            "publication, dependency-license compliance, or store approval. See SOURCE.md "
            "for the exact matching source and other requirements before public release."
        ),
    }, indent=2) + "\n").encode("utf-8")
    return notices


def copy_legal_notices(notices: dict[str, bytes], destination: Path) -> None:
    """Copy the collected snapshot, retaining the source-relative notice paths."""
    for relative, contents in notices.items():
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(contents)


def add_legal_notices_to_zip(notices: dict[str, bytes], archive: zipfile.ZipFile) -> None:
    """Add APK notices before alignment/signing; reject ambiguous duplicate entries."""
    members = {f"assets/legal/{relative}": contents for relative, contents in notices.items()}
    duplicates = set(members).intersection(archive.namelist())
    if duplicates:
        raise RuntimeError("Archive already contains legal notices: " + ", ".join(sorted(duplicates)))
    for name, contents in members.items():
        archive.writestr(name, contents, compress_type=zipfile.ZIP_DEFLATED)
