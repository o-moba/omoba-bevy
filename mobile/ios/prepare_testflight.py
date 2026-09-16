#!/usr/bin/env python3
"""Prepare a development-signed Xcode archive for subsequent App Store export.

Uses an existing device app and identity. Does not create credentials, provision,
export an IPA, upload, or claim Apple validation. The source app is never changed.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import plistlib
import re
import shutil
import sys
import tempfile

from build_device import ROOT, inspect_macho, output, resolve_identity, sha256, validate_dsym, validate_profile


def validate_build_number(value: str) -> str:
    # Conservative CFBundleVersion subset: positive four-digit integer, with
    # optional two-digit minor/patch components. No developer suffix in releases.
    if not re.fullmatch(r"[1-9][0-9]{0,3}(?:\.[0-9]{1,2}){0,2}", value):
        raise ValueError("Build number must be 1..9999, optionally followed by .0..99[.0..99].")
    return value


def validate_source(app: Path) -> tuple[dict, dict]:
    if not app.is_dir() or app.suffix != ".app" or app.is_symlink():
        raise ValueError("Supply a regular physical-device .app directory.")
    if any(path.is_symlink() for path in app.rglob("*")):
        raise ValueError("App symlinks are not supported by this archive preparer.")
    info = plistlib.loads((app / "Info.plist").read_bytes())
    # Current single-binary package contract; never interpret executable paths
    # supplied by a plist outside this bundle.
    if info.get("CFBundleExecutable") != "client":
        raise ValueError("Expected the Omoba client executable.")
    if not re.fullmatch(r"[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+", info.get("CFBundleIdentifier", "")):
        raise ValueError("Invalid app identifier.")
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", info.get("CFBundleShortVersionString", "")):
        raise ValueError("Expected a numeric three-component marketing version.")
    if not (app / "assets/legal/SOURCE-REVISION.json").is_file():
        raise ValueError("App is missing its bundled source/legal provenance.")
    for path in (app / "Frameworks", app / "PlugIns", app / "Watch"):
        if path.exists():
            raise ValueError("Nested signed bundles require a dedicated Xcode archive workflow.")
    return info, inspect_macho(app / "client")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--dsym", type=Path, help="matching .dSYM; defaults to OmobaBeta.app.dSYM beside the app")
    parser.add_argument("--build-number", required=True)
    parser.add_argument("--identity", required=True, help="existing Apple Development identity SHA-1 or exact name")
    parser.add_argument("--output", type=Path, required=True, help="fresh .xcarchive path")
    parser.add_argument("--check", action="store_true", help="read-only checks, no signing or archive writes")
    args = parser.parse_args(argv)
    build = validate_build_number(args.build_number)
    app = args.app.expanduser().absolute()
    dsym = (args.dsym.expanduser() if args.dsym else app.with_name(app.name + ".dSYM")).resolve()
    archive = args.output.expanduser().absolute()
    if archive.suffix != ".xcarchive" or archive.exists() or archive.is_symlink():
        raise ValueError("Choose a fresh .xcarchive output; existing output is never replaced.")
    if app.resolve() in archive.resolve().parents:
        raise ValueError("Archive output cannot be inside the source app.")
    if dsym.resolve() in archive.resolve().parents:
        raise ValueError("Archive output cannot be inside the source dSYM.")
    info, binary = validate_source(app)
    symbols = validate_dsym(app / "client", dsym, runner=output)
    output(["codesign", "--verify", "--strict", str(app)])
    identity = resolve_identity(args.identity, output(["security", "find-identity", "-v", "-p", "codesigning"]).decode())
    profile = plistlib.loads(output(["security", "cms", "-D", "-i", str(app / "embedded.mobileprovision")]))
    entitlements = validate_profile(profile, info["CFBundleIdentifier"], identity)
    assets = ROOT / "mobile/ios/Assets.xcassets"
    if not (assets / "AppIcon.appiconset/AppIcon.png").is_file():
        raise ValueError("Bundled 1024px beta app icon is missing.")
    manifest = ROOT / "mobile/ios/PrivacyInfo.xcprivacy"
    plistlib.loads(manifest.read_bytes())
    report = {"preflight_passed": True, "bundle_id": info["CFBundleIdentifier"],
              "version": info["CFBundleShortVersionString"], "build": build,
              "input_binary": binary, "archive_prepared": False,
              "debug_symbols": symbols,
              "distribution_signed": False, "apple_validated": False,
              "uploaded": False, "testflight_available": False, "installed": False}
    if args.check:
        print(json.dumps(report, indent=2))
        return 0
    sdk_version = output(["xcrun", "--sdk", "iphoneos", "--show-sdk-version"]).decode().strip()
    sdk_build = output(["xcrun", "--sdk", "iphoneos", "--show-sdk-build-version"]).decode().strip()
    if tuple(map(int, sdk_version.split(".")[:2])) != tuple(map(int, binary["sdk"].split(".")[:2])):
        raise ValueError("Selected SDK differs from the source executable SDK; rebuild with the selected Xcode.")
    xcode = output(["xcodebuild", "-version"]).decode()
    match = re.search(r"Xcode (\d+)\.(\d+)(?:\.(\d+))?\s+Build version (\S+)", xcode)
    if not match:
        raise ValueError("Could not read selected Xcode version.")
    major, minor, patch, xcode_build = match.groups()
    archive.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".omoba-archive-", dir=archive.parent) as temporary:
        staging = Path(temporary)
        prepared = staging / "OmobaBeta.xcarchive"
        bundle = prepared / "Products/Applications/OmobaBeta.app"
        shutil.copytree(app, bundle)
        retained_dsym = prepared / "dSYMs/OmobaBeta.app.dSYM"
        shutil.copytree(dsym, retained_dsym)
        partial = staging / "icons.plist"
        output(["xcrun", "actool", "--compile", str(bundle), "--platform", "iphoneos",
                "--minimum-deployment-target", info["MinimumOSVersion"], "--app-icon", "AppIcon",
                "--output-partial-info-plist", str(partial), "--target-device", "iphone",
                "--target-device", "ipad", str(assets)])
        info.update(plistlib.loads(partial.read_bytes()))
        info.update({"CFBundleVersion": build, "DTCompiler": "com.apple.compilers.llvm.clang.1_0",
                     "DTPlatformBuild": sdk_build, "DTPlatformVersion": sdk_version,
                     "DTSDKBuild": sdk_build, "DTSDKName": f"iphoneos{sdk_version}",
                     "DTXcode": f"{int(major):02d}{int(minor)}{int(patch or 0)}", "DTXcodeBuild": xcode_build})
        (bundle / "Info.plist").write_bytes(plistlib.dumps(info))
        shutil.copyfile(manifest, bundle / "PrivacyInfo.xcprivacy")
        entitlement_file = staging / "entitlements.plist"
        entitlement_file.write_bytes(plistlib.dumps(entitlements))
        output(["codesign", "--force", "--sign", identity, "--generate-entitlement-der",
                "--entitlements", str(entitlement_file), "--timestamp=none", str(bundle)])
        output(["codesign", "--verify", "--strict", str(bundle)])
        actual = plistlib.loads(output(["codesign", "--display", "--entitlements", ":-", str(bundle)]))
        if actual != entitlements:
            raise ValueError("Archive app signature entitlements do not match its development profile.")
        retained_symbols = validate_dsym(bundle / "client", retained_dsym, runner=output)
        if retained_symbols != symbols:
            raise ValueError("dSYM changed during archive preparation; source symbols must remain intact.")
        metadata = {"ArchiveVersion": 2, "CreationDate": datetime.now(timezone.utc).replace(tzinfo=None),
                    "Name": "Omoba Beta", "SchemeName": "OmobaBeta",
                    "ApplicationProperties": {"ApplicationPath": "Applications/OmobaBeta.app",
                        "CFBundleIdentifier": info["CFBundleIdentifier"],
                        "CFBundleShortVersionString": info["CFBundleShortVersionString"],
                        "CFBundleVersion": build, "SigningIdentity": "Apple Development",
                        "Team": entitlements["com.apple.developer.team-identifier"],
                        "Architectures": ["arm64"]}}
        (prepared / "Info.plist").write_bytes(plistlib.dumps(metadata))
        report.update({"archive_prepared": True, "development_signature_verified": True,
                       "source_binary_unchanged": sha256(app / "client") == binary["sha256"],
                       "files": [{"path": p.relative_to(prepared).as_posix(), "sha256": sha256(p)}
                                 for p in sorted(prepared.rglob("*")) if p.is_file()],
                       "notice": "Preparation only. Includes matching dSYM symbols. Requires App Store distribution export, Apple validation, compliance review and upload."})
        if not report["source_binary_unchanged"]:
            raise ValueError("Source executable changed during preparation.")
        (prepared / "preparation.json").write_text(json.dumps(report, indent=2) + "\n")
        prepared.rename(archive)
    print(json.dumps({k: v for k, v in report.items() if k != "files"}, indent=2))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, RuntimeError, plistlib.InvalidFileException) as error:
        print(f"Archive preparation failed: {error}", file=sys.stderr)
        sys.exit(1)
