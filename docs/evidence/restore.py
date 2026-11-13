#!/usr/bin/env python3
"""Verify and restore the external evidence archive without Git payload storage."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import subprocess
import tarfile
import tempfile

HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parents[1]
MANIFEST = HERE / "ARCHIVE.json"
PREFIX = PurePosixPath("docs/evidence")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def members(archive: tarfile.TarFile) -> list[tarfile.TarInfo]:
    selected = []
    for member in archive.getmembers():
        path = PurePosixPath(member.name)
        if path.is_absolute() or ".." in path.parts or PREFIX not in path.parents:
            raise SystemExit(f"Unsafe archive path: {member.name}")
        if member.issym() or member.islnk() or not (member.isfile() or member.isdir()):
            raise SystemExit(f"Unsupported archive member: {member.name}")
        if member.isfile():
            selected.append(member)
    return selected


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, help="Use an existing archive instead of gh download")
    parser.add_argument("--check-only", action="store_true", help="Verify without extracting")
    parser.add_argument("--overwrite", action="store_true", help="Allow replacing differing restored files")
    args = parser.parse_args()
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    with tempfile.TemporaryDirectory(prefix="omoba-evidence-") as temporary:
        archive_path = args.archive
        if archive_path is None:
            archive_path = Path(temporary) / manifest["asset"]
            subprocess.run([
                "gh", "release", "download", manifest["release_tag"],
                "--repo", manifest["archive_repository"],
                "--pattern", manifest["asset"],
                "--dir", str(archive_path.parent),
            ], check=True)
        archive_path = archive_path.resolve()
        if archive_path.stat().st_size != manifest["archive_bytes"]:
            raise SystemExit("Archive size does not match ARCHIVE.json")
        actual = sha256(archive_path)
        if actual != manifest["archive_sha256"]:
            raise SystemExit(f"Archive SHA-256 mismatch: {actual}")
        with tarfile.open(archive_path, "r:gz") as archive:
            files = members(archive)
            if len(files) != manifest["archived_files"]:
                raise SystemExit(f"Archive file count mismatch: {len(files)}")
            if args.check_only:
                print(f"PASS archive={archive_path} files={len(files)} sha256={actual}")
                return
            tracked = set(subprocess.check_output([
                "git", "ls-files", "docs/evidence"
            ], cwd=REPO_ROOT, text=True).splitlines())
            conflicts = []
            restored_files = []
            for member in files:
                if member.name in tracked:
                    continue
                restored_files.append(member)
                target = REPO_ROOT / PurePosixPath(member.name)
                if target.exists() and target.is_file():
                    if sha256(target) == hashlib.sha256(archive.extractfile(member).read()).hexdigest():
                        continue
                    if not args.overwrite:
                        conflicts.append(str(target.relative_to(REPO_ROOT)))
            if conflicts:
                raise SystemExit("Refusing to overwrite differing files:\n" + "\n".join(conflicts[:20]))
            selected = [
                member for member in archive.getmembers()
                if member.isdir() or member.name not in tracked
            ]
            archive.extractall(REPO_ROOT, members=selected, filter="data")
        print(
            f"PASS restored={len(restored_files)} skipped_tracked={len(files) - len(restored_files)} "
            f"archive={archive_path} destination={REPO_ROOT}"
        )


if __name__ == "__main__":
    main()
