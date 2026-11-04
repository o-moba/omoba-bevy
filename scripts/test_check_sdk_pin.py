#!/usr/bin/env python3
"""The SDK pin check reads the pin from both Cargo files and finds it on branches."""
import contextlib
import io
import subprocess
import tempfile
import unittest
from pathlib import Path

import check_sdk_pin

URL = "https://github.com/ekza-space/ekza-bevy-sdk"
GIT = [
    "git",
    "-c", "user.name=t",
    "-c", "user.email=t@example.invalid",
    "-c", "commit.gpgsign=false",
    "-c", "core.hooksPath=/dev/null",
]


def git(repo, *args):
    return subprocess.run(
        [*GIT, "-C", str(repo), *args], check=True, capture_output=True, text=True
    ).stdout.strip()


def manifest(rev, url=URL):
    return (
        "[workspace.dependencies]\n"
        'bevy = "0.19.1"\n'
        f'ekza-bevy-sdk = {{ git = "{url}", rev = "{rev}", default-features = false }}\n'
    )


def lock(rev, url=URL):
    return (
        "[[package]]\n"
        'name = "ekza-bevy-macros"\n'
        'version = "0.1.0"\n'
        'source = "registry+https://github.com/rust-lang/crates.io-index"\n'
        "\n[[package]]\n"
        'name = "ekza-bevy-sdk"\n'
        'version = "0.9.0"\n'
        f'source = "git+{url}?rev={rev}#{rev}"\n'
        "dependencies = [\n"
        ' "serde",\n'
        "]\n"
    )


class ParseTests(unittest.TestCase):
    REV = "d18ec7227a8c72c4dedfb8aca825bd656bf6d02c"

    def test_manifest_and_lock_name_the_url_and_commit(self):
        self.assertEqual(check_sdk_pin.manifest_pin(manifest(self.REV)), (URL, self.REV))
        self.assertEqual(check_sdk_pin.lock_pin(lock(self.REV)), (URL, self.REV))

    def test_short_or_missing_revisions_are_rejected(self):
        with self.assertRaises(check_sdk_pin.PinError):
            check_sdk_pin.manifest_pin(manifest("d18ec72"))
        with self.assertRaises(check_sdk_pin.PinError):
            check_sdk_pin.manifest_pin(f'ekza-bevy-sdk = {{ git = "{URL}", branch = "dev" }}\n')
        with self.assertRaises(check_sdk_pin.PinError):
            check_sdk_pin.lock_pin(lock(self.REV).replace("ekza-bevy-sdk", "other"))

    def test_this_repository_pins_one_sdk_commit_in_both_files(self):
        url, rev = check_sdk_pin.read_pin(check_sdk_pin.ROOT)
        self.assertEqual(url, URL)
        self.assertRegex(rev, r"^[0-9a-f]{40}$")


class BranchTests(unittest.TestCase):
    """A local SDK repository stands in for GitHub: `main` and `dev`, a feature
    branch with one more commit, and a commit no branch contains."""

    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.tmp = Path(scratch.name)
        self.sdk = self.tmp / "sdk"
        subprocess.run(["git", "init", "-q", "-b", "main", str(self.sdk)], check=True)
        (self.sdk / "lib.rs").write_text("// base\n")
        git(self.sdk, "add", "lib.rs")
        git(self.sdk, "commit", "-q", "-m", "base")
        self.base = git(self.sdk, "rev-parse", "HEAD")
        git(self.sdk, "branch", "dev")
        git(self.sdk, "switch", "-q", "-c", "feat/library-assets")
        (self.sdk / "lib.rs").write_text("// feature\n")
        git(self.sdk, "commit", "-q", "-am", "feature")
        self.feature = git(self.sdk, "rev-parse", "HEAD")
        tree = git(self.sdk, "rev-parse", "HEAD^{tree}")
        self.orphan = git(self.sdk, "commit-tree", tree, "-p", self.feature, "-m", "only in a cache")
        self.root = self.tmp / "game"
        self.root.mkdir()

    def pin(self, rev):
        (self.root / "Cargo.toml").write_text(manifest(rev))
        (self.root / "Cargo.lock").write_text(lock(rev))

    def run_check(self, *extra, remote=None):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = check_sdk_pin.main(
                ["--root", str(self.root), "--remote", str(remote or self.sdk), *extra]
            )
        return code, out.getvalue() + err.getvalue()

    def test_commit_on_main_and_dev_passes_the_dev_requirement(self):
        self.pin(self.base)
        self.assertEqual(
            check_sdk_pin.branches_containing(str(self.sdk), self.base),
            ["dev", "feat/library-assets", "main"],
        )
        code, text = self.run_check("--branch", "dev")
        self.assertEqual(code, 0, text)

    def test_feature_branch_commit_is_published_but_not_merged(self):
        self.pin(self.feature)
        code, text = self.run_check()
        self.assertEqual(code, 0, text)
        self.assertIn("feat/library-assets", text)
        code, text = self.run_check("--branch", "dev")
        self.assertEqual(code, 1)
        self.assertIn("merge commit", text)

    def test_commit_on_no_branch_fails(self):
        self.pin(self.orphan)
        self.assertEqual(check_sdk_pin.branches_containing(str(self.sdk), self.orphan), [])
        code, text = self.run_check()
        self.assertEqual(code, 1)
        self.assertIn("on no branch", text)

    def test_deleting_the_feature_branch_unpublishes_its_commit(self):
        self.pin(self.feature)
        git(self.sdk, "switch", "-q", "main")
        git(self.sdk, "branch", "-q", "-D", "feat/library-assets")
        code, text = self.run_check()
        self.assertEqual(code, 1, text)

    def test_cargo_lock_out_of_step_with_cargo_toml_fails(self):
        (self.root / "Cargo.toml").write_text(manifest(self.base))
        (self.root / "Cargo.lock").write_text(lock(self.feature))
        code, text = self.run_check()
        self.assertEqual(code, 1)
        self.assertIn("cargo update -p ekza-bevy-sdk", text)

    def test_unreadable_remote_fails(self):
        self.pin(self.base)
        code, text = self.run_check(remote=self.tmp / "missing")
        self.assertEqual(code, 1)
        self.assertIn("could not read the branches", text)


if __name__ == "__main__":
    unittest.main()
