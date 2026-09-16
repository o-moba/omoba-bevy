#!/usr/bin/env python3
"""Pure package tests; no Apple credentials, device, Cargo, or signing is used."""
from contextlib import redirect_stdout
from datetime import datetime, timedelta, timezone
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import plistlib
import struct
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("build_device", Path(__file__).with_name("build_device.py"))
device = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(device)


def macho(platform=2, cpu=0x0100000C, kind=2, minimum=0x000F0000):
    command = struct.pack("<6I", 0x32, 24, platform, minimum, 0x001A0200, 0)
    return struct.pack("<8I", 0xFEEDFACF, cpu, 0, kind, 1, len(command), 0, 0) + command


class DeviceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.binary = self.root / "client"
        self.binary.write_bytes(macho())
        self.cert = b"synthetic certificate bytes for profile contract only"
        self.identity = hashlib.sha1(self.cert).hexdigest().upper()
        self.now = datetime.now(timezone.utc)
        self.profile = {
            "Platform": ["iOS"], "ExpirationDate": self.now + timedelta(days=1),
            "ProvisionedDevices": ["TEST-DEVICE"], "TeamIdentifier": ["TESTTEAM00"],
            "ApplicationIdentifierPrefix": ["TESTPREFIX"], "DeveloperCertificates": [self.cert],
            "Entitlements": {"application-identifier": "TESTPREFIX.*",
                             "com.apple.developer.team-identifier": "TESTTEAM00", "get-task-allow": True},
        }

    def validate(self, **kwargs):
        return device.validate_profile(self.profile, device.BUNDLE_ID, self.identity, now=self.now, **kwargs)

    def test_physical_macho_platform_and_versions(self):
        result = device.inspect_macho(self.binary)
        self.assertEqual(result["minimum_os"], "15.0.0")
        self.assertEqual(result["sdk"], "26.2.0")
        self.assertEqual(result["sha256"], hashlib.sha256(macho()).hexdigest())

    def test_same_arm64_simulator_is_rejected(self):
        self.binary.write_bytes(macho(platform=7))
        with self.assertRaisesRegex(ValueError, "physical iOS"):
            device.inspect_macho(self.binary)

    def test_macos_library_and_wrong_architecture_are_rejected(self):
        for arguments in ({"platform": 1}, {"kind": 6}, {"cpu": 0x01000007}):
            with self.subTest(arguments=arguments):
                self.binary.write_bytes(macho(**arguments))
                with self.assertRaises(ValueError):
                    device.inspect_macho(self.binary)

    def test_truncated_or_invalid_load_commands_are_rejected(self):
        for data in (b"bad", macho()[:-1], macho()[:32] + struct.pack("<2I", 0x32, 2) + b"\0" * 16):
            self.binary.write_bytes(data)
            with self.assertRaises(ValueError):
                device.inspect_macho(self.binary)

    def test_wildcard_profile_and_distinct_team_prefix(self):
        actual = self.validate(device_udid="test-device")
        self.assertEqual(actual, {"application-identifier": "TESTPREFIX.space.ekza.omoba.beta",
                                 "com.apple.developer.team-identifier": "TESTTEAM00", "get-task-allow": True})

    def test_exact_profile_allowed_and_wrong_app_rejected(self):
        self.profile["Entitlements"]["application-identifier"] = "TESTPREFIX." + device.BUNDLE_ID
        self.validate()
        self.profile["Entitlements"]["application-identifier"] = "TESTPREFIX.some.other.app"
        with self.assertRaisesRegex(ValueError, "bundle identifier"):
            self.validate()

    def test_expired_and_unenrolled_device_rejected(self):
        with self.assertRaisesRegex(ValueError, "not enrolled"):
            self.validate(device_udid="OTHER-DEVICE")
        self.profile["ExpirationDate"] = self.now - timedelta(seconds=1)
        with self.assertRaisesRegex(ValueError, "expired"):
            self.validate()

    def test_wrong_certificate_and_inconsistent_prefix_rejected(self):
        self.profile["DeveloperCertificates"] = [b"different cert"]
        with self.assertRaisesRegex(ValueError, "identity"):
            self.validate()
        self.profile["DeveloperCertificates"] = [self.cert]
        self.profile["ApplicationIdentifierPrefix"] = ["OTHERPREFIX"]
        with self.assertRaisesRegex(ValueError, "prefix"):
            self.validate()

    def test_distribution_profile_rejected(self):
        self.profile["Entitlements"]["get-task-allow"] = False
        with self.assertRaisesRegex(ValueError, "development"):
            self.validate()

    def test_identity_names_are_exact_and_unambiguous(self):
        listing = f'  1) {self.identity} "Apple Development: Test"\n'
        self.assertEqual(device.resolve_identity("Apple Development: Test", listing), self.identity)
        self.assertEqual(device.resolve_identity(self.identity.lower(), listing), self.identity)
        with self.assertRaises(ValueError):
            device.resolve_identity("Apple Development", listing)
        with self.assertRaises(ValueError):
            device.resolve_identity("Apple Development: Test", listing + '  2) ' + "A" * 40 + ' "Apple Development: Test"\n')

    def repository(self):
        subprocess.run(["git", "init", "--quiet", str(self.root)], check=True)
        assets = self.root / "client/assets"
        # Move synthetic executable away from the source directory's name.
        self.binary.rename(self.root / "device-binary")
        self.binary = self.root / "device-binary"
        assets.mkdir(parents=True)
        (assets / "tracked.ogg").write_bytes(b"audio fixture")
        (assets / "private-local.json").write_text("untracked must not ship")
        ios = self.root / "mobile/ios"
        ios.mkdir(parents=True)
        (ios / "Info.plist").write_bytes(plistlib.dumps({"CFBundleVersion": "1"}))
        (self.root / "Cargo.toml").write_text('[workspace.package]\nversion = "0.19.0-rc.8"\n')
        (self.root / "LICENSE").write_text("synthetic notice")
        subprocess.run(["git", "add", "client/assets/tracked.ogg", "mobile", "Cargo.toml", "LICENSE"], cwd=self.root, check=True)
        subprocess.run(["git", "-c", "user.name=Package Test", "-c", "user.email=package@example.invalid",
                        "commit", "--quiet", "-m", "test fixture"], cwd=self.root, check=True)

    def test_tracked_assets_exclude_untracked_and_reject_symlinks(self):
        self.repository()
        self.assertEqual([relative.as_posix() for _, relative in device.tracked_assets(self.root)], ["tracked.ogg"])
        asset = self.root / "client/assets/tracked.ogg"
        asset.unlink()
        asset.symlink_to(self.binary)
        with self.assertRaisesRegex(ValueError, "regular"):
            device.tracked_assets(self.root)

    def test_existing_binary_unsigned_package_manifest_and_notice(self):
        self.repository()
        out = self.root / "out"
        with patch.object(device, "ROOT", self.root), patch.object(device, "collect_legal_notices", return_value={"LICENSE": b"synthetic notice"}), redirect_stdout(io.StringIO()):
            self.assertEqual(device.main(["--binary", str(self.binary), "--output", str(out), "--check"]), 0)
            self.assertFalse(out.exists(), "preflight must not write")
            self.assertEqual(device.main(["--binary", str(self.binary), "--output", str(out)]), 0)
            with self.assertRaisesRegex(ValueError, "already exists"):
                device.main(["--binary", str(self.binary), "--output", str(out)])
        app = out / "OmobaBeta.app"
        self.assertFalse((app / "assets/private-local.json").exists())
        self.assertEqual((app / "assets/legal/LICENSE").read_bytes(), b"synthetic notice")
        info = plistlib.loads((app / "Info.plist").read_bytes())
        self.assertEqual(info["CFBundleSupportedPlatforms"], ["iPhoneOS"])
        self.assertEqual(info["MinimumOSVersion"], "15.0.0")
        report = json.loads((out / "device-build.json").read_text())
        self.assertFalse(report["signed_and_verified"])
        self.assertFalse(report["installed"])
        self.assertFalse(report["binary_source_match_verified"])
        for entry in report["files"]:
            self.assertEqual(entry["sha256"], device.sha256(app / entry["path"]))
        app.rename(out / "archived.app")
        with patch.object(device, "ROOT", self.root):
            with self.assertRaisesRegex(ValueError, "report already exists"):
                device.main(["--binary", str(self.binary), "--output", str(out)])

    def test_untracked_legal_notice_rejected(self):
        self.repository()
        with patch.object(device, "ROOT", self.root), patch.object(device, "collect_legal_notices", return_value={"LICENSES/private.txt": b"untracked"}):
            with self.assertRaisesRegex(ValueError, "tracked source"):
                device.main(["--binary", str(self.binary), "--check"])


if __name__ == "__main__":
    unittest.main()
