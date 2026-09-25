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

# ABI name -> (Rust target triple, NDK clang triple). armeabi-v7a and x86_64
# are optional extra slices for --universal; arm64-v8a alone already covers
# effectively all real phones/tablets from the last decade.
ABIS = {
    "arm64-v8a": ("aarch64-linux-android", "aarch64-linux-android"),
    "armeabi-v7a": ("armv7-linux-androideabi", "armv7a-linux-androideabi"),
    "x86_64": ("x86_64-linux-android", "x86_64-linux-android"),
}
DEFAULT_ABI = "arm64-v8a"


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
    parser.add_argument("--universal", action="store_true",
                         help="build all supported ABIs (arm64-v8a, armeabi-v7a, x86_64) into one APK "
                              "instead of arm64-v8a only; only worth it when a device's real ABI is unknown "
                              "or confirmed non-arm64, since it multiplies build time and APK size")
    args = parser.parse_args()
    if args.version_code < 1:
        parser.error("--version-code must be positive")
    abis = list(ABIS) if args.universal else [DEFAULT_ABI]
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
    for abi in abis:
        rust_target, _ = ABIS[abi]
        if tools["rustc"]:
            result = subprocess.run([tools["rustc"], "--print", "target-libdir", "--target", rust_target], text=True, capture_output=True)
            library_dir = Path(result.stdout.strip())
            if result.returncode or not library_dir.is_dir() or not list(library_dir.glob("libstd-*.rlib")):
                problems.append(f"Rust standard library for {rust_target} ({abi}) is not installed in the selected toolchain.")
    build_tools = sdk / "build-tools/35.0.0" if sdk else Path("/__missing_sdk__")
    android_jar = sdk / "platforms/android-35/android.jar" if sdk else Path("/__missing_sdk__/android.jar")
    ndk_bin = ndk / f"toolchains/llvm/prebuilt/{host}/bin" if ndk else Path("/__missing_ndk__")
    paths = {
        "android.jar": android_jar,
        "aapt2": build_tools / "aapt2",
        "zipalign": build_tools / "zipalign",
        "llvm-ar": ndk_bin / "llvm-ar",
        "llvm-strip": ndk_bin / "llvm-strip",
    }
    clang_paths = {}
    for abi in abis:
        _, clang_triple = ABIS[abi]
        clang_paths[abi] = {
            "clang": ndk_bin / f"{clang_triple}{API}-clang",
            "clang++": ndk_bin / f"{clang_triple}{API}-clang++",
        }
        paths[f"clang ({abi})"] = clang_paths[abi]["clang"]
        paths[f"clang++ ({abi})"] = clang_paths[abi]["clang++"]
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
        "target": [ABIS[abi][0] for abi in abis], "abis": abis, "min_sdk": API, "target_sdk": 35,
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
    packaged_libs = {}
    for abi in abis:
        rust_target, _ = ABIS[abi]
        env = os.environ.copy()
        target_env = rust_target.replace("-", "_")
        target_env_upper = target_env.upper()
        env.update({
            "CARGO_TARGET_DIR": str(out / "cargo"),
            f"CARGO_TARGET_{target_env_upper}_LINKER": str(clang_paths[abi]["clang"]),
            f"CC_{target_env}": str(clang_paths[abi]["clang"]),
            f"CXX_{target_env}": str(clang_paths[abi]["clang++"]),
            f"AR_{target_env}": str(paths["llvm-ar"]),
            "CARGO_PROFILE_DEV_DEBUG": "0", "CARGO_INCREMENTAL": "0",
        })
        # CARGO_ENCODED_RUSTFLAGS/RUSTFLAGS override target-specific flags in Cargo.
        # Preserve caller flags and append the 16 KiB ABI requirement at highest precedence.
        # This only matters for arm64-v8a (Android 15's page-size requirement), but is
        # harmless on the other ABIs too.
        if "CARGO_ENCODED_RUSTFLAGS" in env:
            rustflags = [flag for flag in env["CARGO_ENCODED_RUSTFLAGS"].split("\x1f") if flag]
        else:
            rustflags = shlex.split(env.get("RUSTFLAGS", env.get(f"CARGO_TARGET_{target_env_upper}_RUSTFLAGS", "")))
        env["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join(rustflags + ["-C", "link-arg=-Wl,-z,max-page-size=16384"])
        if args.server:
            env["OMOBA_DEFAULT_GAME_SERVER_ADDR"] = args.server
        run([tools["cargo"], "rustc", "--locked", "-p", "client", "--lib", "--target", rust_target,
             "--crate-type", "cdylib"], env=env)
        library = out / f"cargo/{rust_target}/debug/libclient.so"
        if not library.is_file():
            raise RuntimeError(f"Cargo produced no expected native library: {library}")
        packaged_lib = out / f"libclient-{abi}.so"
        shutil.copy2(library, packaged_lib)
        run([paths["llvm-strip"], "--strip-debug", packaged_lib])
        packaged_libs[abi] = packaged_lib
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
        for abi, packaged_lib in packaged_libs.items():
            apk.write(packaged_lib, f"lib/{abi}/libclient.so", compress_type=zipfile.ZIP_STORED)
        add_legal_notices_to_zip(legal_notices, apk)
    run([paths["zipalign"], "-P", "16", "-f", "4", unaligned, unsigned])
    artifact = unsigned
    abi_tag = "universal" if args.universal else "arm64"
    if not args.unsigned:
        keystore = out / "local-debug.keystore"
        if not keystore.exists():
            run([tools["keytool"], "-genkeypair", "-keystore", keystore,
                 "-storepass", "android", "-keypass", "android", "-alias", "androiddebugkey",
                 "-keyalg", "RSA", "-keysize", "2048", "-validity", "3650",
                 "-dname", "CN=Omoba Local Debug,O=Development,C=US"])
        artifact = out / f"omoba-{version}-android-{abi_tag}-debug.apk"
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
