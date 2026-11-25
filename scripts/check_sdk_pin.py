#!/usr/bin/env python3
"""Check that a fresh clone can fetch the pinned ekza-bevy-sdk commit.

The workspace pins the SDK as a git dependency at one exact commit
(`[workspace.dependencies]` in Cargo.toml, mirrored in Cargo.lock). Cargo
fetches that commit from the SDK repository, so the commit has to be on a
branch there. A commit that exists only in a local SDK checkout, or only in
this machine's ~/.cargo/git cache, still builds here while CI and every fresh
clone fail with "revision ... not found" (`--locked` cannot fetch it).

    python3 scripts/check_sdk_pin.py               # on some branch of the SDK repo
    python3 scripts/check_sdk_pin.py --branch dev  # merged into the SDK's dev

`--branch` is the check before this repository merges into `dev` or `main`:
a squash- or rebase-merged SDK pull request leaves the pinned commit on its
feature branch only, and the pin breaks when that branch is deleted.

Needs `git` and read access to the SDK repository (`--remote` reads another
copy, such as a local mirror). Used by `make sdk-pin-check`.
"""
import argparse
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = "ekza-bevy-sdk"
SHA = re.compile(r"[0-9a-f]{40}")


class PinError(Exception):
    """The pin is missing, inconsistent or cannot be fetched."""


def manifest_pin(text):
    """(url, rev) of the SDK entry in the workspace Cargo.toml."""
    for line in text.splitlines():
        if not re.match(rf"\s*{re.escape(PACKAGE)}\s*=\s*\{{", line):
            continue
        url = re.search(r'\bgit\s*=\s*"([^"]+)"', line)
        rev = re.search(r'\brev\s*=\s*"([^"]+)"', line)
        if not url or not rev:
            raise PinError(f"Cargo.toml: {PACKAGE} is not a git dependency with a rev")
        if not SHA.fullmatch(rev.group(1)):
            raise PinError(f"Cargo.toml: {PACKAGE} rev {rev.group(1)!r} is not a full commit hash")
        return url.group(1), rev.group(1)
    raise PinError(f"Cargo.toml: no {PACKAGE} entry")


def lock_pin(text):
    """(url, commit) of the SDK package in Cargo.lock."""
    for block in text.split("[[package]]"):
        if not re.search(rf'^name = "{re.escape(PACKAGE)}"$', block, re.M):
            continue
        source = re.search(r'^source = "git\+([^"?#]+)[^"#]*#([0-9a-f]{40})"$', block, re.M)
        if not source:
            raise PinError(f"Cargo.lock: {PACKAGE} has no git source")
        return source.group(1), source.group(2)
    raise PinError(f"Cargo.lock: no {PACKAGE} package")


def read_pin(root):
    """The pinned (url, rev), after checking Cargo.toml and Cargo.lock agree."""
    url, rev = manifest_pin((root / "Cargo.toml").read_text())
    lock_url, lock_rev = lock_pin((root / "Cargo.lock").read_text())
    if (lock_url, lock_rev) != (url, rev):
        raise PinError(
            f"Cargo.lock pins {lock_url} {lock_rev[:7]}, Cargo.toml pins {url} {rev[:7]}; "
            f"run `cargo update -p {PACKAGE}` and commit both files"
        )
    return url, rev


def git(*args, cwd=None):
    return subprocess.run(["git", *args], cwd=cwd, check=False, capture_output=True, text=True)


def branches_containing(remote, rev):
    """Branches of `remote` whose history contains `rev` (empty when none does)."""
    with tempfile.TemporaryDirectory(prefix="sdk-pin-") as scratch:
        if git("init", "-q", "--bare", scratch).returncode:
            raise PinError("git init failed")
        # Commits and trees only: ancestry needs no file contents.
        fetched = git(
            "fetch", "-q", "--no-tags", "--filter=blob:none", remote, "+refs/heads/*:refs/heads/*",
            cwd=scratch,
        )
        if fetched.returncode:
            raise PinError(f"could not read the branches of {remote}: {fetched.stderr.strip()}")
        if git("cat-file", "-e", f"{rev}^{{commit}}", cwd=scratch).returncode:
            return []
        listed = git(
            "for-each-ref", "--contains", rev, "--format=%(refname:short)", "refs/heads",
            cwd=scratch,
        )
        return sorted(name for name in listed.stdout.split() if name)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--branch", help="also require the commit on this SDK branch (for example dev)")
    parser.add_argument("--remote", help="read this repository instead of the Cargo.toml git URL")
    parser.add_argument("--root", type=Path, default=ROOT, help=argparse.SUPPRESS)
    args = parser.parse_args(argv)

    try:
        url, rev = read_pin(args.root)
        remote = args.remote or url
        branches = branches_containing(remote, rev)
    except PinError as error:
        print(f"sdk-pin-check: {error}", file=sys.stderr)
        return 1

    short = rev[:7]
    if not branches:
        print(
            f"sdk-pin-check: {PACKAGE} {short} is on no branch of {remote}. CI and fresh clones "
            f"cannot fetch it; a local ~/.cargo/git cache hides this. Push the SDK branch that "
            f"has {short} (keep that sha), or repin to a published SDK commit.",
            file=sys.stderr,
        )
        return 1
    if args.branch and args.branch not in branches:
        print(
            f"sdk-pin-check: {PACKAGE} {short} is on {', '.join(branches)} of {remote}, not on "
            f"{args.branch}. Merge the SDK pull request into {args.branch} with a merge commit "
            f"(a squash or rebase merge drops {short}), or repin to the merged commit.",
            file=sys.stderr,
        )
        return 1
    print(f"sdk-pin-check: {PACKAGE} {short} is on {', '.join(branches)} of {remote}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
