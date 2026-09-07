#!/usr/bin/env python3
"""Regression checks for source identity and Cargo executable selection."""
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

import package_native


class PackagingIdentityTests(unittest.TestCase):
    def test_dirty_identity_covers_untracked_sources_but_ignores_runtime_artifacts(self):
        with tempfile.TemporaryDirectory(prefix="omoba-source-proof-") as directory:
            root = Path(directory)
            def git(*args):
                return subprocess.check_output(["git", *args], cwd=root, stderr=subprocess.DEVNULL)
            git("init")
            (root / ".gitignore").write_text("runtime/\n")
            (root / "tracked.rs").write_text("initial source\n")
            git("add", ".")
            git("-c", "user.name=Packaging test", "-c", "user.email=test@example.invalid",
                "commit", "-m", "test: initialize fixture")
            with mock.patch.object(package_native, "ROOT", root):
                clean = package_native.source_identity()
                self.assertFalse(clean["source_dirty"])
                (root / "new.rs").write_text("first implementation\n")
                first = package_native.source_identity()
                (root / "new.rs").write_text("second implementation\n")
                second = package_native.source_identity()
                self.assertEqual(first["source_diff_sha256"], second["source_diff_sha256"])
                self.assertNotEqual(first["source_identity_sha256"], second["source_identity_sha256"])
                self.assertNotEqual(first["untracked_source_sha256"], second["untracked_source_sha256"])
                (root / "runtime").mkdir()
                (root / "runtime/log.txt").write_text("not source\n")
                self.assertEqual(second, package_native.source_identity())

    def test_build_uses_cargo_emitted_paths_and_rejects_incomplete_or_failed_builds(self):
        executable_paths = {name: Path("/configured-cargo-target/custom-profile") / name
                            for name in ("client", "server", "bots")}
        events = [dict(reason="compiler-artifact", target=dict(name=name), executable=str(path))
                  for name, path in executable_paths.items()]
        def process_for(messages, returncode=0):
            process = mock.Mock()
            process.stdout = io.StringIO("".join(json.dumps(event) + "\n" for event in messages))
            process.wait.return_value = returncode
            return process
        with mock.patch.object(package_native.subprocess, "Popen", return_value=process_for(events)):
            self.assertEqual(package_native.build_executables("dev"), executable_paths)
        with mock.patch.object(package_native.subprocess, "Popen", return_value=process_for(events[:1])):
            with self.assertRaisesRegex(RuntimeError, "required executables"):
                package_native.build_executables("dev")
        with mock.patch.object(package_native.subprocess, "Popen", return_value=process_for(events, 1)):
            with self.assertRaisesRegex(RuntimeError, "build failed"):
                package_native.build_executables("dev")


if __name__ == "__main__":
    unittest.main()
