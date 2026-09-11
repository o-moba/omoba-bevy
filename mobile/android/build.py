#!/usr/bin/env python3
"""Build the existing Bevy NativeActivity client without Gradle or cargo-apk.

Requires Python 3.9+, Rust Android std, JDK17+, Android SDK35/build-tools35,
NDK r27+ and (on Apple Silicon) Rosetta for Google's macOS x86_64 SDK binaries.
Toolchains are supplied by the caller: this script never installs or upgrades them.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import platform
import shutil
import shlex
import subprocess
import sys
import re
import zipfile

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from package_licenses import add_legal_notices_to_zip, collect_legal_notices

TARGET = "aarch64-linux-android"
API = 26


def run(command, *, env=None):
    print("+ " + " ".join(map(str, command)), flush=True)
    subprocess.run([str(item) for item in command], check=True, cwd=ROOT, env=env)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="read-only prerequisite report")
    parser.add_argument("--sdk", type=Path, default=os.getenv("ANDROID_HOME") or os.getenv("ANDROID_SDK_ROOT"))
    parser.add_argument("--ndk", type=Path, default=os.getenv("ANDROID_NDK_HOME"))
    parser.add_argument("--output", type=Path, default=ROOT / "target/mobile/android")
    parser.add_argument("--unsigned", action="store_true", help="skip local debug signing; output is not installable")
    parser.add_argument("--server", help="optional real host:port compiled as initial address; editable in game")
    parser.add_argument("--version-code", type=int, default=1)
    args = parser.parse_args()
    if args.version_code < 1:
        parser.error("--version-code must be positive")
    legal_notices = None if args.check else collect_legal_notices(ROOT)
    sdk = args.sdk.expanduser().resolve() if args.sdk else None
    ndk = args.ndk.expanduser().resolve() if args.ndk else None
    if ndk is None and sdk and (sdk / "ndk").is_dir():
        candidates = sorted((sdk / "ndk").iterdir(), key=lambda p: tuple(int(n) for n in p.name.split(".") if n.isdigit()))
        ndk = candidates[-1] if candidates else None
    host = {"Darwin": "darwin-x86_64", "Linux": "linux-x86_64"}.get(platform.system())
    problems = []
    if not host:
        problems.append("This packaging script supports macOS and Linux hosts.")
    tools = {name: shutil.which(name) for name in ["cargo", "rustc", "java", "keytool"]}
    for name, found in tools.items():
        if not found and (name != "keytool" or not args.unsigned):
            problems.append(f"Missing executable: {name}")
    if tools["rustc"]:
        result = subprocess.run([tools["rustc"], "--print", "target-libdir", "--target", TARGET], text=True, capture_output=True)
        library_dir = Path(result.stdout.strip())
        if result.returncode or not library_dir.is_dir() or not list(library_dir.glob("libstd-*.rlib")):
            problems.append(f"Rust standard library for {TARGET} is not installed in the selected toolchain.")
    build_tools = sdk / "build-tools/35.0.0" if sdk else Path("/__missing_sdk__")
    android_jar = sdk / "platforms/android-35/android.jar" if sdk else Path("/__missing_sdk__/android.jar")
    ndk_bin = ndk / f"toolchains/llvm/prebuilt/{host}/bin" if ndk else Path("/__missing_ndk__")
    paths = {
        "android.jar": android_jar,
        "aapt2": build_tools / "aapt2",
        "zipalign": build_tools / "zipalign",
        "clang": ndk_bin / f"aarch64-linux-android{API}-clang",
        "clang++": ndk_bin / f"aarch64-linux-android{API}-clang++",
        "llvm-ar": ndk_bin / "llvm-ar",
        "llvm-strip": ndk_bin / "llvm-strip",
    }
    if not args.unsigned:
        paths["apksigner"] = build_tools / "apksigner"
    for name, path in paths.items():
        if not path.is_file():
            problems.append(f"Missing {name}: {path}")
    # --output may be on a different disk; inspect its closest existing parent.
    output_volume = args.output.expanduser().resolve()
    while not output_volume.exists():
        output_volume = output_volume.parent
    free = shutil.disk_usage(output_volume).free
    if free < 6 * 1024**3:
        problems.append(f"Only {free / 1024**3:.2f} GiB free; reserve at least 6 GiB for a fresh Android build (more may be needed).")
    report = {
        "target": TARGET, "min_sdk": API, "target_sdk": 35,
        "sdk": str(sdk) if sdk else None, "ndk": str(ndk) if ndk else None,
        "free_bytes": free, "ready_to_build": not problems,
        "problems": problems,
        "distribution": "local debug APK; no public endpoint, publication or release signing",
    }
    print(json.dumps(report, indent=2))
    if problems:
        return 1
    if args.check:
        return 0
    out = args.output.expanduser().resolve()
    out.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env.update({
        "CARGO_TARGET_DIR": str(out / "cargo"),
        "CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER": str(paths["clang"]),
        "CC_aarch64_linux_android": str(paths["clang"]),
        "CXX_aarch64_linux_android": str(paths["clang++"]),
        "AR_aarch64_linux_android": str(paths["llvm-ar"]),
        "CARGO_PROFILE_DEV_DEBUG": "0", "CARGO_INCREMENTAL": "0",
    })
    # CARGO_ENCODED_RUSTFLAGS/RUSTFLAGS override target-specific flags in Cargo.
    # Preserve caller flags and append the 16 KiB ABI requirement at highest precedence.
    if "CARGO_ENCODED_RUSTFLAGS" in env:
        rustflags = [flag for flag in env["CARGO_ENCODED_RUSTFLAGS"].split("\x1f") if flag]
    else:
        rustflags = shlex.split(env.get("RUSTFLAGS", env.get("CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS", "")))
    env["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join(rustflags + ["-C", "link-arg=-Wl,-z,max-page-size=16384"])
    if args.server:
        env["OMOBA_DEFAULT_GAME_SERVER_ADDR"] = args.server
    run([tools["cargo"], "rustc", "--locked", "-p", "client", "--lib", "--target", TARGET,
         "--crate-type", "cdylib"], env=env)
    library = out / f"cargo/{TARGET}/debug/libclient.so"
    if not library.is_file():
        raise RuntimeError(f"Cargo produced no expected native library: {library}")
    packaged_lib = out / "libclient.so"
    shutil.copy2(library, packaged_lib)
    run([paths["llvm-strip"], "--strip-debug", packaged_lib])
    version = re.search(r'(?ms)^\[workspace\.package\]\s*\n(?:(?!^\[).)*?^version\s*=\s*"([^"\n]+)"', (ROOT / "Cargo.toml").read_text()).group(1)
    manifest_text = Path(__file__).with_name("AndroidManifest.xml").read_text()
    manifest_text = manifest_text.replace("__VERSION__", version).replace('android:versionCode="1"', f'android:versionCode="{args.version_code}"')
    manifest = out / "AndroidManifest.xml"
    manifest.write_text(manifest_text)
    unaligned = out / "omoba-unaligned.apk"
    unsigned = out / "omoba-unsigned.apk"
    run([paths["aapt2"], "link", "-I", android_jar, "--manifest", manifest,
         "-A", ROOT / "client/assets", "-o", unaligned])
    with zipfile.ZipFile(unaligned, "a") as apk:
        apk.write(packaged_lib, "lib/arm64-v8a/libclient.so", compress_type=zipfile.ZIP_STORED)
        add_legal_notices_to_zip(legal_notices, apk)
    run([paths["zipalign"], "-P", "16", "-f", "4", unaligned, unsigned])
    artifact = unsigned
    if not args.unsigned:
        keystore = out / "local-debug.keystore"
        if not keystore.exists():
            run([tools["keytool"], "-genkeypair", "-keystore", keystore,
                 "-storepass", "android", "-keypass", "android", "-alias", "androiddebugkey",
                 "-keyalg", "RSA", "-keysize", "2048", "-validity", "3650",
                 "-dname", "CN=Omoba Local Debug,O=Development,C=US"])
        artifact = out / f"omoba-{version}-android-arm64-debug.apk"
        run([paths["apksigner"], "sign", "--ks", keystore, "--ks-key-alias", "androiddebugkey",
             "--ks-pass", "pass:android", "--key-pass", "pass:android", "--out", artifact, unsigned])
        run([paths["apksigner"], "verify", "--verbose", artifact])
    # Check the final archive as well as the pre-sign input; never modify it after signing.
    run([paths["zipalign"], "-c", "-P", "16", "4", artifact])
    print(f"Artifact: {artifact}")
    print("Debug test artifact only. Validate startup/assets, two-thumb controls, reconnect and a full PC/phone match on a physical device.")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"Android build failed: {error}", file=sys.stderr)
        sys.exit(1)
