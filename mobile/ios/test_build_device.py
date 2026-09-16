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


UUID = "079D4CF5-594E-3DD6-A192-D183AB5EB30B"


def dsym_fixture(root: Path) -> Path:
    """Small structurally valid section table; Apple UUID tools are mocked."""
    bundle = root / "client.dSYM"
    directory = bundle / "Contents/Resources/DWARF"
    directory.mkdir(parents=True)
    size = 72 + 2 * 80
    command = struct.pack("<II16sQQQQIIII", 0x19, size, b"__DWARF", 0, 2, 32 + size, 2, 1, 1, 2, 0)
    for i, name in enumerate((b"__debug_info", b"__debug_line")):
        command += struct.pack("<16s16sQQIIIIIIII", name, b"__DWARF", i, 1, 32 + size + i, 0, 0, 0, 0, 0, 0, 0)
    header = struct.pack("<8I", 0xFEEDFACF, 0x0100000C, 0, 10, 1, len(command), 0, 0)
    (directory / "client").write_bytes(header + command + b"DL")
    (bundle / "Contents/Info.plist").write_bytes(plistlib.dumps({"CFBundlePackageType": "dSYM"}))
    return bundle


def uuid_output(command):
    assert command[:3] == ["xcrun", "dwarfdump", "--uuid"], command
    return f"UUID: {UUID} (arm64) {command[-1]}\n".encode()


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

    def test_ios_profiles_retain_packed_debug_symbols(self):
        with patch.dict(device.os.environ, {"CARGO_PROFILE_DEV_DEBUG": "0", "CARGO_PROFILE_RELEASE_STRIP": "symbols"}):
            env = device.ios_build_environment(self.root, "/sdk", "192.0.2.1:4000")
        for profile in ("DEV", "RELEASE"):
            self.assertEqual(env[f"CARGO_PROFILE_{profile}_DEBUG"], "1")
            self.assertEqual(env[f"CARGO_PROFILE_{profile}_STRIP"], "none")
            self.assertEqual(env[f"CARGO_PROFILE_{profile}_SPLIT_DEBUGINFO"], "packed")
        self.assertEqual(env["CARGO_INCREMENTAL"], "0")
        self.assertEqual(env["OMOBA_DEFAULT_GAME_SERVER_ADDR"], "192.0.2.1:4000")

    def test_matching_dsym_uuid_and_real_sections_required(self):
        dsym = dsym_fixture(self.root)
        report = device.validate_dsym(self.binary, dsym, runner=uuid_output)
        self.assertTrue(report["uuid_match_verified"])
        self.assertEqual(report["uuids"], {"arm64": UUID})
        (dsym / report["dwarf_file"]).write_bytes(macho(kind=10))
        with self.assertRaisesRegex(ValueError, "debug info/line tables"):
            device.validate_dsym(self.binary, dsym, runner=uuid_output)

    def test_missing_and_mismatched_dsym_rejected(self):
        with self.assertRaisesRegex(ValueError, "missing"):
            device.validate_dsym(self.binary, self.root / "missing.dSYM")
        dsym = dsym_fixture(self.root)
        def mismatch(command):
            return uuid_output(command).replace(UUID.encode(), b"FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF") if command[-1] == str(self.binary) else uuid_output(command)
        with self.assertRaisesRegex(ValueError, "UUID mismatch"):
            device.validate_dsym(self.binary, dsym, runner=mismatch)
        for bad in (b"", b"UUID: invalid (arm64)", uuid_output(["xcrun", "dwarfdump", "--uuid", "x"]) * 2,
                    uuid_output(["xcrun", "dwarfdump", "--uuid", "x"]).replace(b"arm64", b"x86_64")):
            with self.assertRaisesRegex(ValueError, "exactly one arm64"):
                device.dwarf_uuids(self.binary, runner=lambda _: bad)

    def test_dsym_symlink_rejected_without_reading_destination(self):
        dsym = dsym_fixture(self.root)
        (dsym / "escape").symlink_to(self.root)
        with self.assertRaisesRegex(ValueError, "symlinks"):
            device.validate_dsym(self.binary, dsym, runner=uuid_output)

    def test_cargo_dsym_root_alias_is_allowed_but_nested_alias_is_not(self):
        dsym = dsym_fixture(self.root)
        target = self.root / "deps/client-0123456789.dSYM"
        target.parent.mkdir()
        dsym.rename(target)
        dsym.symlink_to(Path("deps") / target.name, target_is_directory=True)
        self.assertTrue(device.validate_dsym(self.binary, dsym, runner=uuid_output)["uuid_match_verified"])
        (target / "nested").symlink_to(self.root)
        with self.assertRaisesRegex(ValueError, "symlinks"):
            device.validate_dsym(self.binary, dsym, runner=uuid_output)

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

    def test_existing_binary_package_retains_verified_symbols(self):
        self.repository()
        dsym = dsym_fixture(self.root)
        target = self.root / "deps/client-0123456789.dSYM"
        target.parent.mkdir()
        dsym.rename(target)
        alias = Path(str(self.binary) + ".dSYM")
        alias.symlink_to(Path("deps") / target.name, target_is_directory=True)
        real_output = device.output
        def tool(command, **kwargs):
            return uuid_output(command) if command[:3] == ["xcrun", "dwarfdump", "--uuid"] else real_output(command, **kwargs)
        for explicit in (False, True):
            with self.subTest(explicit=explicit):
                out = self.root / f"symbols-out-{explicit}"
                with patch.object(device, "ROOT", self.root), patch.object(device, "collect_legal_notices", return_value={"LICENSE": b"synthetic notice"}), \
                        patch.object(device, "output", side_effect=tool), patch.object(device, "check_tools"), redirect_stdout(io.StringIO()):
                    args = ["--binary", str(self.binary), "--output", str(out)]
                    if explicit:
                        args.extend(["--dsym", str(alias)])
                    self.assertEqual(device.main(args + ["--check"]), 0)
                    self.assertFalse(out.exists())
                    self.assertEqual(device.main(args), 0)
                report = json.loads((out / "device-build.json").read_text())
                symbols = report["debug_symbols"]
                self.assertTrue(symbols["uuid_match_verified"])
                self.assertEqual(symbols["uuids"], {"arm64": UUID})
                retained = out / symbols["bundle"] / symbols["dwarf_file"]
                self.assertFalse((out / symbols["bundle"]).is_symlink())
                self.assertEqual(retained.read_bytes(), (target / symbols["dwarf_file"]).read_bytes())
                self.assertEqual(device.sha256(retained), symbols["dwarf_sha256"])


if __name__ == "__main__":
    unittest.main()
