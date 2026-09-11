#!/usr/bin/env python3
"""Notice preservation and pre-sign packaging regressions; no native builds needed."""
from __future__ import annotations

import contextlib
import importlib.util
import io
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parent))
import package_licenses


ROOT = Path(__file__).resolve().parents[1]


def load_packager(name, relative):
    spec = importlib.util.spec_from_file_location(name, ROOT / relative)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


NATIVE = load_packager("native_packager", "scripts/package_native.py")
ANDROID = load_packager("android_packager", "mobile/android/build.py")
IOS = load_packager("ios_packager", "mobile/ios/build_simulator.py")


class PackageLicenseTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.root = self.base / "source"
        self.root.mkdir()
        self.source_notices = {
            name: f"Complete fixture notice: {name}\r\nUnicode attribution: café\r\n".encode()
            for name in (*package_licenses.REQUIRED_FILES, "LICENSES/EXTRA.txt")
        }
        for relative, contents in self.source_notices.items():
            self.write(relative, contents)
        self.local_notices = {
            "fonts/fixture/OFL.txt": b"Fixture font license\n",
            "models/fixture/LICENSE.md": b"Fixture asset license\n",
        }
        for relative, contents in self.local_notices.items():
            self.write("client/assets/" + relative, contents)
        for relative in (
            "scripts/beta_launcher.py", "docs/progress/2026-09-07-beta-test-guide.md",
            "art/verdant-confluence/PROVENANCE.md", "assets-src/animations/README.md",
            "docs/progress/2026-09-05-distribution-review.md",
        ):
            self.write(relative, b"Fixture packaging input\n")
        self.write("Cargo.toml", b'[workspace.package]\nversion = "0.1.0-test"\n')
        for name in ("client", "server", "bots"):
            self.write("fixture-binaries/" + name, b"Fixture executable\n")
        self.git("init", "--quiet")
        self.git("add", ".")
        self.git("-c", "user.name=Packaging test", "-c", "user.email=test@example.invalid",
                 "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null",
                 "commit", "--quiet", "-m", "Fixture")
        self.revision = self.git("rev-parse", "HEAD")

    def write(self, relative, contents):
        target = self.root / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(contents)

    def git(self, *arguments):
        return subprocess.check_output(["git", *arguments], cwd=self.root, text=True).strip()

    def assert_notices(self, read):
        for relative, expected in self.source_notices.items():
            self.assertEqual(read(relative), expected, relative)
        identity = json.loads(read("SOURCE-REVISION.json"))
        self.assertEqual(identity["source_revision"], self.revision)
        self.assertIs(identity["source_publication_verified"], False)
        self.assertIn("does not verify", identity["notice"])

    def test_helper_preserves_all_notice_bytes_in_directory_and_zip(self):
        notices = package_licenses.collect_legal_notices(self.root)
        self.assertIs(json.loads(notices["SOURCE-REVISION.json"])["source_dirty"], False)
        directory = self.base / "legal"
        package_licenses.copy_legal_notices(notices, directory)
        self.assert_notices(lambda name: (directory / name).read_bytes())
        archive_path = self.base / "fixture.apk"
        with zipfile.ZipFile(archive_path, "w") as archive:
            archive.writestr("assets/fonts/fixture/OFL.txt", b"Existing font notice")
            package_licenses.add_legal_notices_to_zip(notices, archive)
        with zipfile.ZipFile(archive_path) as archive:
            self.assert_notices(lambda name: archive.read("assets/legal/" + name))
            self.assertEqual(archive.read("assets/fonts/fixture/OFL.txt"), b"Existing font notice")
            self.assertEqual(len(archive.namelist()), len(notices) + 1)

    def test_revision_marks_uncommitted_changes_without_claiming_publication(self):
        self.write("uncommitted.txt", b"Not part of HEAD\n")
        notices = package_licenses.collect_legal_notices(self.root)
        self.assert_notices(notices.__getitem__)
        self.assertIs(json.loads(notices["SOURCE-REVISION.json"])["source_dirty"], True)

    def test_every_required_notice_must_exist_and_contain_text(self):
        # Reject incomplete inputs before Git or any external build command is needed.
        with mock.patch.object(package_licenses.subprocess, "check_output",
                               side_effect=AssertionError("External command before validation")):
            for relative in package_licenses.REQUIRED_FILES:
                path = self.root / relative
                original = path.read_bytes()
                for contents in (None, b" \r\n\t", b"\xff"):
                    with self.subTest(relative=relative, contents=contents):
                        if contents is None:
                            path.unlink()
                        else:
                            path.write_bytes(contents)
                        with self.assertRaises(RuntimeError):
                            package_licenses.collect_legal_notices(self.root)
                        path.write_bytes(original)

    def test_duplicate_apk_notice_fails_before_adding_any_members(self):
        notices = package_licenses.collect_legal_notices(self.root)
        with zipfile.ZipFile(self.base / "duplicate.apk", "w") as archive:
            archive.writestr("assets/legal/LICENSE", b"Unexpected existing text")
            before = archive.namelist()
            with self.assertRaisesRegex(RuntimeError, "already contains"):
                package_licenses.add_legal_notices_to_zip(notices, archive)
            self.assertEqual(archive.namelist(), before)

    def test_all_packagers_fail_closed_before_build_on_missing_notice(self):
        (self.root / "LICENSES/MPL-2.0.txt").unlink()
        for module in (NATIVE, ANDROID, IOS):
            with self.subTest(packager=module.__name__), \
                    mock.patch.object(module, "ROOT", self.root), \
                    mock.patch.object(sys, "argv", [module.__file__, "--output", str(self.base / "output")]), \
                    mock.patch.object(subprocess, "run", side_effect=AssertionError("Build must not run")):
                with self.assertRaisesRegex(RuntimeError, "MPL-2.0.txt"):
                    module.main()
                self.assertFalse((self.base / "output").exists())

    def test_native_package_includes_legal_snapshot_and_local_asset_notices(self):
        output = self.base / "native"
        original_run = NATIVE.run

        def fake_run(*arguments):
            if arguments[0] == "cargo":
                self.assertEqual(arguments[1], "metadata")
                return json.dumps({"packages": [{"name": "client", "version": "0.1.0-test"}]})
            return original_run(*arguments)

        with mock.patch.object(NATIVE, "ROOT", self.root), \
                mock.patch.object(sys, "argv", [NATIVE.__file__, "--output", str(output)]), \
                mock.patch.object(NATIVE, "run", side_effect=fake_run), \
                mock.patch.object(NATIVE, "validate", return_value={"status": "PASS", "errors": []}), \
                mock.patch.object(NATIVE, "build_executables", return_value={
                    name: self.root / "fixture-binaries" / name for name in ("client", "server", "bots")
                }), contextlib.redirect_stdout(io.StringIO()):
            NATIVE.main()
        self.assert_notices(lambda name: (output / "legal" / name).read_bytes())
        self.assertTrue((output / "server").is_file())
        for relative, expected in self.local_notices.items():
            self.assertEqual((output / "assets" / relative).read_bytes(), expected)
        identity = json.loads((output / "BUILD.json").read_text())
        self.assertIn("legal/LICENSES/AGPL-3.0-only.txt", identity["sha256"])

    def test_mobile_packages_include_notices_before_signing(self):
        for module in (ANDROID, IOS):
            with self.subTest(packager=module.__name__):
                self.check_mobile_packager(module)

    def check_mobile_packager(self, module):
        output = self.base / module.__name__
        sdk = self.base / "sdk"
        ndk = self.base / "ndk"
        stdlib = self.base / "stdlib"
        stdlib.mkdir(exist_ok=True)
        (stdlib / "libstd-fixture.rlib").touch()
        for relative in ("platforms/android-35/android.jar", "build-tools/35.0.0/aapt2",
                         "build-tools/35.0.0/zipalign", "build-tools/35.0.0/apksigner"):
            path = sdk / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.touch()
        for name in ("aarch64-linux-android26-clang", "aarch64-linux-android26-clang++", "llvm-ar", "llvm-strip"):
            path = ndk / "toolchains/llvm/prebuilt/linux-x86_64/bin" / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.touch()
        real_run = subprocess.run
        signed = {}

        def fake_run(command, **kwargs):
            command = list(map(str, command))
            tool = Path(command[0]).name
            if tool == "git":
                return real_run(command, **kwargs)
            stdout = ""
            if tool == "rustc":
                stdout = str(stdlib)
            elif tool == "xcrun":
                stdout = str(sdk)
            elif tool == "cargo":
                relative = (f"cargo/{ANDROID.TARGET}/debug/libclient.so" if module is ANDROID
                            else f"cargo/{IOS.TARGET}/debug/client")
                binary = output / relative
                binary.parent.mkdir(parents=True)
                binary.write_bytes(b"Fixture native binary")
            elif tool == "aapt2":
                with zipfile.ZipFile(command[-1], "w") as archive:
                    for path in (self.root / "client/assets").rglob("*"):
                        if path.is_file():
                            archive.write(path, "assets/" + path.relative_to(self.root / "client/assets").as_posix())
            elif tool == "zipalign":
                if "-c" not in command:
                    shutil.copyfile(command[-2], command[-1])
            elif tool == "keytool":
                Path(command[command.index("-keystore") + 1]).touch()
            elif tool == "apksigner":
                if command[1] == "sign":
                    with zipfile.ZipFile(command[-1]) as archive:
                        self.assert_notices(lambda name: archive.read("assets/legal/" + name))
                        for relative, expected in self.local_notices.items():
                            self.assertEqual(archive.read("assets/" + relative), expected)
                    artifact = Path(command[command.index("--out") + 1])
                    shutil.copyfile(command[-1], artifact)
                    signed[artifact] = artifact.read_bytes()
                else:
                    self.assertEqual(command[1], "verify")
            elif tool == "codesign":
                bundle = Path(command[-1])
                self.assert_notices(lambda name: (bundle / "assets/legal" / name).read_bytes())
                for relative, expected in self.local_notices.items():
                    self.assertEqual((bundle / "assets" / relative).read_bytes(), expected)
                signed.update({path: path.read_bytes() for path in bundle.rglob("*") if path.is_file()})
            else:
                self.assertEqual(tool, "llvm-strip", f"Unexpected external command: {command}")
            return subprocess.CompletedProcess(command, 0, stdout=stdout, stderr="")

        arguments = [module.__file__, "--output", str(output)]
        if module is ANDROID:
            arguments.extend(["--sdk", str(sdk), "--ndk", str(ndk)])
        with mock.patch.object(module, "ROOT", self.root), \
                mock.patch.object(sys, "argv", arguments), \
                mock.patch.object(shutil, "which", side_effect=lambda name: "/fixture/bin/" + name), \
                mock.patch.object(shutil, "disk_usage", return_value=SimpleNamespace(free=20 * 1024**3)), \
                mock.patch.object(subprocess, "run", side_effect=fake_run), \
                mock.patch("platform.system", return_value="Linux"), \
                contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(module.main(), 0)
        self.assertTrue(signed, "The packaging path must reach signing")
        for path, contents in signed.items():
            self.assertEqual(path.read_bytes(), contents, "Package mutated after signing")


if __name__ == "__main__":
    unittest.main()
