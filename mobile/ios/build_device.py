#!/usr/bin/env python3
"""Build/package a physical iPhone app; optionally sign with existing credentials.

No provisioning, certificate creation, installation, or private-key export occurs.
--check is read-only. --binary packages an existing executable without Cargo.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from package_licenses import collect_legal_notices, copy_legal_notices

TARGET = "aarch64-apple-ios"
BUNDLE_ID = "space.ekza.omoba.beta"
DEPLOYMENT_TARGET = "15.0"


def output(command: list[str], *, cwd: Path | None = None) -> bytes:
    result = subprocess.run(command, cwd=cwd, capture_output=True, check=False)
    if result.returncode:
        # Some Apple tools include personal profile/device data in stderr.
        raise RuntimeError(f"{Path(command[0]).name} failed (exit {result.returncode}); inspect locally.")
    return result.stdout


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def version_string(packed: int) -> str:
    return f"{packed >> 16}.{(packed >> 8) & 255}.{packed & 255}"


def inspect_macho(path: Path) -> dict:
    """Require the thin arm64 MH_EXECUTE emitted by our physical Rust target.

    Simulator arm64 is a different platform despite the identical CPU type.
    Read only the bounded load-command table, never the whole executable.
    """
    with path.open("rb") as source:
        header = source.read(32)
        if len(header) != 32:
            raise ValueError("Executable has a truncated Mach-O header.")
        magic, cpu, _subtype, kind, count, size, _flags, _reserved = struct.unpack("<8I", header)
        if magic != 0xFEEDFACF or cpu != 0x0100000C or kind != 2:
            raise ValueError("Expected a thin arm64 Mach-O executable, not a library or universal/simulator wrapper.")
        if not count or size > 16 * 1024 * 1024 or count > size // 8:
            raise ValueError("Invalid Mach-O load-command table.")
        commands = source.read(size)
    if len(commands) != size:
        raise ValueError("Truncated Mach-O load commands.")
    cursor = 0
    versions = []
    for _ in range(count):
        if cursor + 8 > size:
            raise ValueError("Truncated Mach-O load command.")
        command, length = struct.unpack_from("<2I", commands, cursor)
        if length < 8 or cursor + length > size:
            raise ValueError("Invalid Mach-O load-command size.")
        if command == 0x32:  # LC_BUILD_VERSION
            if length < 24:
                raise ValueError("Truncated Mach-O platform metadata.")
            platform, minimum, sdk, tools = struct.unpack_from("<4I", commands, cursor + 8)
            if platform != 2 or 24 + tools * 8 > length:
                raise ValueError("Mach-O is not a physical iOS executable.")
            versions.append((minimum, sdk))
        elif command == 0x25:  # LC_VERSION_MIN_IPHONEOS, older Apple linker
            if length < 16:
                raise ValueError("Truncated iOS deployment metadata.")
            versions.append(struct.unpack_from("<2I", commands, cursor + 8))
        elif command in (0x24, 0x2F, 0x30):
            raise ValueError("Executable declares another Apple operating system.")
        cursor += length
    if cursor != size or len(versions) != 1:
        raise ValueError("Executable needs exactly one physical iOS platform declaration.")
    minimum, sdk = versions[0]
    return {"architecture": "arm64", "platform": "iOS", "minimum_os": version_string(minimum),
            "sdk": version_string(sdk), "sha256": sha256(path), "bytes": path.stat().st_size}


def dwarf_uuids(path: Path, *, runner=None) -> dict[str, str]:
    text = (runner or output)(["xcrun", "dwarfdump", "--uuid", str(path)]).decode()
    records = re.findall(r"(?m)^UUID: ([0-9A-Fa-f]{8}(?:-[0-9A-Fa-f]{4}){3}-[0-9A-Fa-f]{12}) \(([^)]+)\)", text)
    if len(records) != 1 or records[0][1] != "arm64":
        raise ValueError("Expected exactly one arm64 UUID from dwarfdump; symbols cannot be matched.")
    return {arch: uuid.upper() for uuid, arch in records}


def require_dwarf_sections(path: Path) -> None:
    """Reject a UUID-only/empty dSYM: it cannot symbolicate Rust crash logs."""
    with path.open("rb") as source:
        header = source.read(32)
        if len(header) != 32:
            raise ValueError("dSYM DWARF file is truncated.")
        magic, cpu, _subtype, kind, count, size, _flags, _reserved = struct.unpack("<8I", header)
        if magic != 0xFEEDFACF or cpu != 0x0100000C or kind != 10:
            raise ValueError("Expected a thin arm64 MH_DSYM symbol file.")
        if not count or size > 16 * 1024 * 1024 or count > size // 8:
            raise ValueError("Invalid dSYM load-command table.")
        commands = source.read(size)
    if len(commands) != size:
        raise ValueError("Truncated dSYM load commands.")
    cursor, sections = 0, set()
    for _ in range(count):
        if cursor + 8 > size:
            raise ValueError("Truncated dSYM load command.")
        command, length = struct.unpack_from("<2I", commands, cursor)
        if length < 8 or cursor + length > size:
            raise ValueError("Invalid dSYM load-command size.")
        if command == 0x19:  # LC_SEGMENT_64
            if length < 72:
                raise ValueError("Truncated dSYM segment.")
            section_count = struct.unpack_from("<I", commands, cursor + 64)[0]
            if 72 + section_count * 80 > length:
                raise ValueError("Truncated dSYM section table.")
            for i in range(section_count):
                section = cursor + 72 + i * 80
                name = commands[section:section + 16].rstrip(b"\0")
                segment = commands[section + 16:section + 32].rstrip(b"\0")
                section_size, offset = struct.unpack_from("<QI", commands, section + 40)
                if segment == b"__DWARF" and section_size and offset >= 32 + size and offset + section_size <= path.stat().st_size:
                    sections.add(name)
        cursor += length
    if cursor != size or not {b"__debug_info", b"__debug_line"}.issubset(sections):
        raise ValueError("dSYM has no usable debug info/line tables. Rebuild with debug=1, strip=none, split-debuginfo=packed.")


def validate_dsym(binary: Path, dsym: Path, *, runner=None) -> dict:
    # Cargo exposes client.dSYM as a relative alias of deps/client-<hash>.dSYM.
    # Follow that entry once, then require every item inside the actual bundle
    # to be a regular path so a nested alias cannot escape the symbol package.
    dsym = dsym.resolve()
    if not dsym.is_dir() or dsym.suffix != ".dSYM":
        raise ValueError("Matching dSYM bundle is missing; rebuild the iPhone executable with retained debug symbols.")
    if any(path.is_symlink() for path in dsym.rglob("*")):
        raise ValueError("dSYM symlinks are not supported.")
    dwarf_dir = dsym / "Contents/Resources/DWARF"
    files = list(dwarf_dir.iterdir()) if dwarf_dir.is_dir() else []
    if len(files) != 1 or not files[0].is_file():
        raise ValueError("Expected exactly one DWARF file inside the dSYM bundle.")
    require_dwarf_sections(files[0])
    executable_uuids = dwarf_uuids(binary, runner=runner)
    symbol_uuids = dwarf_uuids(files[0], runner=runner)
    if executable_uuids != symbol_uuids:
        raise ValueError(f"dSYM UUID mismatch: executable {executable_uuids['arm64']}, symbols {symbol_uuids['arm64']}. Use symbols from this exact build.")
    return {"uuid_match_verified": True, "uuids": symbol_uuids,
            "dwarf_file": files[0].relative_to(dsym).as_posix(),
            "dwarf_sha256": sha256(files[0]), "dwarf_bytes": files[0].stat().st_size}


def ios_build_environment(cache: Path, sdk: str, server: str | None = None) -> dict[str, str]:
    env = os.environ.copy()
    env.update({"CARGO_TARGET_DIR": str(cache), "SDKROOT": sdk,
                "IPHONEOS_DEPLOYMENT_TARGET": DEPLOYMENT_TARGET, "CARGO_INCREMENTAL": "0"})
    for profile in ("DEV", "RELEASE"):
        env.update({f"CARGO_PROFILE_{profile}_DEBUG": "1",
                    f"CARGO_PROFILE_{profile}_STRIP": "none",
                    f"CARGO_PROFILE_{profile}_SPLIT_DEBUGINFO": "packed"})
    if server:
        env["OMOBA_DEFAULT_GAME_SERVER_ADDR"] = server
    return env


def resolve_identity(requested: str, listing: str) -> str:
    identities = re.findall(r'\b([0-9A-Fa-f]{40})\s+"([^"\n]+)"', listing)
    matching = {fingerprint.upper() for fingerprint, name in identities
                if requested.upper() == fingerprint.upper() or requested == name}
    if len(matching) != 1:
        raise ValueError("Choose one existing valid code-signing identity by exact name or SHA-1 fingerprint.")
    return matching.pop()


def validate_profile(profile: dict, bundle_id: str, identity: str,
                     device_udid: str | None = None, now: datetime | None = None) -> dict:
    """Validate a development profile and return only the needed app entitlements."""
    now = now or datetime.now(timezone.utc)
    expiration = profile.get("ExpirationDate")
    if not isinstance(expiration, datetime):
        raise ValueError("Provisioning profile has no expiration date.")
    if expiration.replace(tzinfo=timezone.utc) <= now.astimezone(timezone.utc):
        raise ValueError("Provisioning profile has expired.")
    if "iOS" not in profile.get("Platform", []):
        raise ValueError("Provisioning profile does not support iOS.")
    devices = profile.get("ProvisionedDevices", [])
    if not devices or profile.get("ProvisionsAllDevices"):
        raise ValueError("Use an existing device development profile, not a distribution profile.")
    if device_udid and device_udid.casefold() not in {str(value).casefold() for value in devices}:
        raise ValueError("The requested iPhone is not enrolled in this provisioning profile.")
    entitlements = profile.get("Entitlements", {})
    if entitlements.get("get-task-allow") is not True:
        raise ValueError("Use a development profile with get-task-allow enabled.")
    team = entitlements.get("com.apple.developer.team-identifier")
    if not team or team not in profile.get("TeamIdentifier", []):
        raise ValueError("Provisioning profile has inconsistent team identifiers.")
    allowed = entitlements.get("application-identifier", "")
    if not isinstance(allowed, str) or "." not in allowed:
        raise ValueError("Provisioning profile has no usable application identifier.")
    prefix, pattern = allowed.split(".", 1)
    if prefix not in profile.get("ApplicationIdentifierPrefix", []):
        raise ValueError("Provisioning profile has inconsistent application prefixes.")
    if not re.fullmatch(r"[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+", bundle_id):
        raise ValueError("Invalid application bundle identifier.")
    match = pattern == bundle_id or (
        pattern.endswith("*") and pattern.count("*") == 1 and bundle_id.startswith(pattern[:-1]))
    if not match:
        raise ValueError("Provisioning profile does not permit this bundle identifier.")
    certificates = profile.get("DeveloperCertificates", [])
    fingerprints = {hashlib.sha1(cert).hexdigest().upper() for cert in certificates if isinstance(cert, bytes)}
    if identity.upper() not in fingerprints:
        raise ValueError("Selected code-signing identity is not included in the provisioning profile.")
    return {"application-identifier": f"{prefix}.{bundle_id}",
            "com.apple.developer.team-identifier": team, "get-task-allow": True}


def tracked_assets(root: Path) -> list[tuple[Path, Path]]:
    listing = output(["git", "ls-files", "-z", "--", "client/assets"], cwd=root)
    paths = []
    for raw in listing.split(b"\0"):
        if not raw:
            continue
        relative = Path(os.fsdecode(raw))
        source = root / relative
        if source.is_symlink() or not source.is_file() or source.resolve() != source.absolute():
            raise ValueError(f"Tracked asset must be a regular, in-tree file: {relative}")
        paths.append((source, relative.relative_to("client/assets")))
    if not paths:
        raise ValueError("No tracked client assets were found.")
    return sorted(paths, key=lambda item: item[1].as_posix())


def app_info(root: Path, binary_info: dict, bundle_id: str) -> dict:
    with (root / "mobile/ios/Info.plist").open("rb") as source:
        info = plistlib.load(source)
    cargo = (root / "Cargo.toml").read_text()
    version = re.search(r'(?ms)^\[workspace\.package\]\s*\n(?:(?!^\[).)*?^version\s*=\s*"([^"\n]+)"', cargo)
    if not version:
        raise ValueError("Workspace package version is missing.")
    # Compare version tuples, never lexical strings.
    os_version = max((tuple(map(int, DEPLOYMENT_TARGET.split("."))) + (0,))[:3],
                     tuple(map(int, binary_info["minimum_os"].split("."))))
    info.update({"CFBundleIdentifier": bundle_id, "CFBundleExecutable": "client",
                 "CFBundleShortVersionString": version.group(1).split("-", 1)[0],
                 "CFBundleSupportedPlatforms": ["iPhoneOS"], "DTPlatformName": "iphoneos",
                 "UIDeviceFamily": [1, 2], "MinimumOSVersion": ".".join(map(str, os_version))})
    return info


def check_tools(names: list[str]) -> None:
    missing = [name for name in names if not shutil.which(name)]
    if missing:
        raise ValueError("Missing required tools: " + ", ".join(missing))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="read-only preflight; never build, sign or write")
    parser.add_argument("--binary", type=Path, help="existing physical arm64 iOS executable; skips Cargo")
    parser.add_argument("--dsym", type=Path, help="matching dSYM for --binary; otherwise looks beside the executable")
    parser.add_argument("--output", type=Path, default=ROOT / "builds/iphone")
    parser.add_argument("--target-dir", type=Path, default=ROOT / "target/iphone-cargo",
                        help="reusable Cargo cache; kept separate from installable builds")
    parser.add_argument("--build-profile", choices=("dev", "release"), default="dev",
                        help="dev retains workspace optimizations; both profiles retain packed dSYM symbols")
    parser.add_argument("--server", help="editable initial host:port, compiled only when building")
    parser.add_argument("--bundle-id", default=BUNDLE_ID)
    parser.add_argument("--profile", type=Path, help="existing .mobileprovision; never copied into source")
    parser.add_argument("--identity", help="existing valid signing identity SHA-1 or exact name")
    parser.add_argument("--device-udid", help="optional local-only profile membership check; does not install")
    args = parser.parse_args(argv)
    if bool(args.profile) != bool(args.identity) or args.device_udid and not args.profile:
        parser.error("--profile and --identity must be supplied together; --device-udid also requires them")
    if args.binary and args.server:
        parser.error("--server cannot change an existing --binary; enter the server in the installed app")
    if args.dsym and not args.binary:
        parser.error("--dsym accompanies --binary; a source build generates its own symbols")
    out = args.output.expanduser().resolve()
    bundle = out / "OmobaBeta.app"
    if not args.check and (bundle.exists() or (out / "device-build.json").exists() or (out / "OmobaBeta.app.dSYM").exists()):
        raise ValueError("Output app or report already exists; choose a fresh --output to preserve previous evidence.")
    binary = args.binary.expanduser().resolve() if args.binary else None
    dsym = args.dsym.expanduser().resolve() if args.dsym else None
    if binary and dsym is None and Path(str(binary) + ".dSYM").exists():
        dsym = Path(str(binary) + ".dSYM").resolve()
    if dsym and (out == dsym.resolve() or dsym.resolve() in out.parents):
        raise ValueError("Output cannot be inside the input dSYM bundle.")
    check_tools(["git"] + ([] if binary else ["cargo", "rustc", "xcrun"]) +
                (["xcrun"] if dsym and binary else []) +
                (["security", "codesign"] if args.profile else []))
    assets = tracked_assets(ROOT)
    notices = collect_legal_notices(ROOT)
    tracked = {os.fsdecode(path) for path in output(["git", "ls-files", "-z"], cwd=ROOT).split(b"\0") if path}
    if any(path != "SOURCE-REVISION.json" and path not in tracked for path in notices):
        raise ValueError("Package legal notices must be tracked source files.")
    binary_info = inspect_macho(binary) if binary else None
    symbols = validate_dsym(binary, dsym) if binary and dsym else None
    sdk = None
    if binary is None:
        sdk = output(["xcrun", "--sdk", "iphoneos", "--show-sdk-path"]).decode().strip()
        std = Path(output(["rustc", "--print", "target-libdir", "--target", TARGET]).decode().strip())
        if not list(std.glob("libstd-*.rlib")):
            raise ValueError(f"Rust standard library for {TARGET} is not installed.")
        volume = out
        while not volume.exists():
            volume = volume.parent
        if shutil.disk_usage(volume).free < 6 * 1024**3:
            raise ValueError("Reserve at least 6 GiB free for a fresh iOS cross-build.")
    profile_bytes = entitlements = identity = None
    if args.profile:
        profile_path = args.profile.expanduser().resolve()
        profile_bytes = profile_path.read_bytes()
        decoded = plistlib.loads(output(["security", "cms", "-D", "-i", str(profile_path)]))
        identity = resolve_identity(args.identity, output(["security", "find-identity", "-v", "-p", "codesigning"]).decode())
        entitlements = validate_profile(decoded, args.bundle_id, identity, args.device_udid)
    report = {"target": TARGET, "preflight_passed": True, "existing_binary": binary_info,
              "tracked_asset_count": len(assets), "signing_credentials_checked": bool(entitlements),
              "device_profile_membership_checked": bool(args.device_udid), "debug_symbols": symbols,
              "installed": False, "launched": False}
    if args.check:
        print(json.dumps(report, indent=2))
        return 0
    out.mkdir(parents=True, exist_ok=True)
    if binary is None:
        cache = args.target_dir.expanduser().resolve()
        env = ios_build_environment(cache, sdk, args.server)
        subprocess.run(["cargo", "build", "--locked", "-p", "client", "--bin", "client", "--target", TARGET,
                        "--profile", args.build_profile], cwd=ROOT, env=env, check=True)
        binary = cache / TARGET / ("debug" if args.build_profile == "dev" else "release") / "client"
        binary_info = inspect_macho(binary)
        dsym = Path(str(binary) + ".dSYM").resolve()
        symbols = validate_dsym(binary, dsym)
    bundle.mkdir()
    shutil.copy2(binary, bundle / "client")
    (bundle / "client").chmod(0o755)
    for source, relative in assets:
        destination = bundle / "assets" / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
    copy_legal_notices(notices, bundle / "assets/legal")
    with (bundle / "Info.plist").open("wb") as destination:
        plistlib.dump(app_info(ROOT, binary_info, args.bundle_id), destination)
    if entitlements:
        (bundle / "embedded.mobileprovision").write_bytes(profile_bytes)
        # Personal team/profile details remain in this transient file and the local signed app.
        with tempfile.TemporaryDirectory(prefix="omoba-ios-sign-") as temporary:
            entitlement_file = Path(temporary) / "entitlements.plist"
            entitlement_file.write_bytes(plistlib.dumps(entitlements))
            output(["codesign", "--force", "--sign", identity, "--generate-entitlement-der", "--entitlements", str(entitlement_file),
                    "--timestamp=none", str(bundle)])
        output(["codesign", "--verify", "--strict", str(bundle)])
        actual = plistlib.loads(output(["codesign", "--display", "--entitlements", ":-", str(bundle)]))
        if actual != entitlements:
            raise ValueError("Signed app entitlements do not match the validated development profile.")
    if symbols:
        retained_dsym = out / "OmobaBeta.app.dSYM"
        shutil.copytree(dsym, retained_dsym)
        symbols = validate_dsym(bundle / "client", retained_dsym)
        symbols["bundle"] = retained_dsym.name
    files = []
    for path in sorted(bundle.rglob("*")):
        if path.is_file():
            files.append({"path": path.relative_to(bundle).as_posix(), "bytes": path.stat().st_size, "sha256": sha256(path)})
    report.update({"bundle": bundle.name, "bundle_id": args.bundle_id, "binary_before_signing": binary_info,
                   "signed_and_verified": bool(entitlements), "files": files,
                   "debug_symbols": symbols,
                   "source_revision": output(["git", "rev-parse", "HEAD"], cwd=ROOT).decode().strip(),
                   "source_dirty": bool(output(["git", "status", "--porcelain"], cwd=ROOT).strip()),
                   "binary_source_match_verified": False,
                   "notice": "Local development artifact only. Build logs must independently bind binary to source. Signing is not installation or physical gameplay verification."})
    (out / "device-build.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: value for key, value in report.items() if key != "files"}, indent=2))
    print(f"Device app: {bundle}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError, plistlib.InvalidFileException) as error:
        print(f"iPhone build failed: {error}", file=sys.stderr)
        sys.exit(1)
