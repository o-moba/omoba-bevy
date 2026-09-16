"""Archive input/failure boundaries without Apple accounts or signing keys."""
from contextlib import redirect_stdout
from datetime import datetime, timedelta
import hashlib
import io
import json
from pathlib import Path
import plistlib
import tempfile
import unittest
from unittest.mock import patch

import prepare_testflight as archive
from test_build_device import UUID, dsym_fixture, macho, uuid_output


class ArchiveTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.app = self.root / "OmobaBeta.app"
        (self.app / "assets/legal").mkdir(parents=True)
        (self.app / "assets/legal/SOURCE-REVISION.json").write_text("{}")
        self.info = {"CFBundleIdentifier": "space.ekza.omoba.beta", "CFBundleExecutable": "client",
                     "CFBundleShortVersionString": "0.20.0", "MinimumOSVersion": "15.0"}
        (self.app / "Info.plist").write_bytes(plistlib.dumps(self.info))
        (self.app / "client").write_bytes(macho())
        self.dsym = dsym_fixture(self.root)
        self.output = self.root / "new/OmobaBeta.xcarchive"
        self.args = ["--app", str(self.app), "--output", str(self.output),
                     "--dsym", str(self.dsym),
                     "--build-number", "2", "--identity", "A" * 40]

    def test_simulator_cannot_be_archived_for_testflight(self):
        (self.app / "client").write_bytes(macho(platform=7))
        with self.assertRaisesRegex(ValueError, "physical iOS"):
            archive.validate_source(self.app)

    def test_source_executable_cannot_escape_app(self):
        self.info["CFBundleExecutable"] = "../client"
        (self.app / "Info.plist").write_bytes(plistlib.dumps(self.info))
        with self.assertRaisesRegex(ValueError, "executable"):
            archive.validate_source(self.app)

    def test_symlinks_and_nested_code_are_rejected(self):
        link = self.app / "escape"
        link.symlink_to(self.root)
        with self.assertRaisesRegex(ValueError, "symlinks"):
            archive.validate_source(self.app)
        link.unlink()
        (self.app / "Frameworks").mkdir()
        with self.assertRaisesRegex(ValueError, "Nested"):
            archive.validate_source(self.app)

    def test_existing_archive_preserved_before_any_apple_tool(self):
        self.output.mkdir(parents=True)
        marker = self.output / "keep"
        marker.write_text("existing build")
        with patch.object(archive, "output") as tool, self.assertRaisesRegex(ValueError, "fresh"):
            archive.main(self.args)
        tool.assert_not_called()
        self.assertEqual(marker.read_text(), "existing build")

    def test_output_inside_source_rejected_without_writes(self):
        self.args[3] = str(self.app / "nested.xcarchive")
        with self.assertRaisesRegex(ValueError, "inside"):
            archive.main(self.args)
        self.assertFalse((self.app / "nested.xcarchive").exists())

    def test_build_number_apple_limits(self):
        for invalid in ("0", "10000", "1.100", "1.2.100", "1.2.3.4", "1beta", "../1", "01", ""):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                archive.validate_build_number(invalid)
        for valid in ("1", "9999", "2026.9.16", "2.0.1"):
            self.assertEqual(archive.validate_build_number(valid), valid)

    def test_missing_symbols_fail_before_signing_and_without_writes(self):
        self.args[self.args.index("--dsym") + 1] = str(self.root / "absent.dSYM")
        with patch.object(archive, "output") as tool, self.assertRaisesRegex(ValueError, "missing"):
            archive.main(self.args)
        tool.assert_not_called()
        self.assertFalse(self.output.parent.exists())

    def test_mismatched_uuid_fails_before_signing_and_without_writes(self):
        def tool(command):
            result = uuid_output(command)
            return result.replace(UUID.encode(), b"FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF") if command[-1] == str(self.app / "client") else result
        with patch.object(archive, "output", side_effect=tool), self.assertRaisesRegex(ValueError, "UUID mismatch"):
            archive.main(self.args)
        self.assertFalse(self.output.parent.exists())

    def test_check_is_read_only_and_never_claims_distribution(self):
        cert = b"synthetic development certificate"
        identity = hashlib.sha1(cert).hexdigest().upper()
        self.args[-1] = identity
        profile = {"Platform": ["iOS"], "ExpirationDate": datetime.now() + timedelta(days=1),
                   "ProvisionedDevices": ["fixture"], "TeamIdentifier": ["TESTTEAM"],
                   "ApplicationIdentifierPrefix": ["TESTTEAM"], "DeveloperCertificates": [cert],
                   "Entitlements": {"application-identifier": "TESTTEAM.*", "get-task-allow": True,
                                    "com.apple.developer.team-identifier": "TESTTEAM"}}
        def tool(command):
            if command[:3] == ["xcrun", "dwarfdump", "--uuid"]:
                return uuid_output(command)
            if command[:2] == ["codesign", "--verify"]:
                return b""
            if command[:2] == ["security", "find-identity"]:
                return f'1) {identity} "Apple Development: Fixture"'.encode()
            if command[:2] == ["security", "cms"]:
                return plistlib.dumps(profile)
            self.fail(f"Unexpected mutation/tool during --check: {command[:2]}")
        stream = io.StringIO()
        before = {p.relative_to(self.root): p.read_bytes() for p in self.root.rglob("*") if p.is_file()}
        with patch.object(archive, "output", side_effect=tool), redirect_stdout(stream):
            self.assertEqual(archive.main(self.args + ["--check"]), 0)
        after = {p.relative_to(self.root): p.read_bytes() for p in self.root.rglob("*") if p.is_file()}
        self.assertEqual(before, after)
        self.assertFalse(self.output.parent.exists())
        report = json.loads(stream.getvalue())
        self.assertTrue(report["preflight_passed"])
        self.assertTrue(report["debug_symbols"]["uuid_match_verified"])
        for key in ("archive_prepared", "distribution_signed", "apple_validated", "uploaded", "installed"):
            self.assertFalse(report[key])

    def test_archive_retains_matching_symbols_and_preserves_inputs(self):
        # This is Cargo's ordinary output shape: the CLI receives a relative
        # directory symlink, while the archive must retain a standalone copy.
        actual_dsym = self.root / "deps/client-0123456789.dSYM"
        actual_dsym.parent.mkdir()
        self.dsym.rename(actual_dsym)
        self.dsym.symlink_to(Path("deps") / actual_dsym.name, target_is_directory=True)
        cert = b"synthetic development certificate"
        identity = hashlib.sha1(cert).hexdigest().upper()
        self.args[-1] = identity
        profile = {"Platform": ["iOS"], "ExpirationDate": datetime.now() + timedelta(days=1),
                   "ProvisionedDevices": ["fixture"], "TeamIdentifier": ["TESTTEAM"],
                   "ApplicationIdentifierPrefix": ["TESTTEAM"], "DeveloperCertificates": [cert],
                   "Entitlements": {"application-identifier": "TESTTEAM.*", "get-task-allow": True,
                                    "com.apple.developer.team-identifier": "TESTTEAM"}}
        entitlements = dict(profile["Entitlements"], **{"application-identifier": "TESTTEAM.space.ekza.omoba.beta"})
        def tool(command):
            if command[:3] == ["xcrun", "dwarfdump", "--uuid"]:
                return uuid_output(command)
            if command[:2] == ["security", "find-identity"]:
                return f'1) {identity} "Apple Development: Fixture"'.encode()
            if command[:2] == ["security", "cms"]:
                return plistlib.dumps(profile)
            if command[0] == "codesign":
                return plistlib.dumps(entitlements) if "--display" in command else b""
            if command[:3] == ["xcrun", "--sdk", "iphoneos"]:
                return b"26.2" if command[-1] == "--show-sdk-version" else b"17C52"
            if command == ["xcodebuild", "-version"]:
                return b"Xcode 26.2\nBuild version 17C52\n"
            if command[:2] == ["xcrun", "actool"]:
                Path(command[command.index("--output-partial-info-plist") + 1]).write_bytes(plistlib.dumps({}))
                return b""
            self.fail(f"Unexpected tool: {command}")
        before = {p: p.read_bytes() for source in (self.app, self.dsym) for p in source.rglob("*") if p.is_file()}
        with patch.object(archive, "output", side_effect=tool), redirect_stdout(io.StringIO()):
            self.assertEqual(archive.main(self.args), 0)
        retained = self.output / "dSYMs/OmobaBeta.app.dSYM"
        self.assertTrue(retained.is_dir())
        self.assertFalse(retained.is_symlink())
        self.assertEqual((retained / "Contents/Resources/DWARF/client").read_bytes(),
                         (self.dsym / "Contents/Resources/DWARF/client").read_bytes())
        self.assertEqual(before, {p: p.read_bytes() for p in before})
        report = json.loads((self.output / "preparation.json").read_text())
        self.assertEqual(report["debug_symbols"]["uuids"], {"arm64": UUID})
        self.assertTrue(any(item["path"].startswith("dSYMs/") for item in report["files"]))
        self.assertFalse(report["uploaded"])


if __name__ == "__main__":
    unittest.main()
