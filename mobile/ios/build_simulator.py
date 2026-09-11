#!/usr/bin/env python3
"""Create an arm64 iOS Simulator .app. No device signing or TestFlight publishing."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
TARGET = "aarch64-apple-ios-sim"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--output", type=Path, default=ROOT / "target/mobile/ios-simulator")
    parser.add_argument("--server", help="optional real host:port compiled as the editable initial server")
    args = parser.parse_args()
    problems = []
    sdk = None
    for name in ["cargo", "rustc", "xcrun", "codesign"]:
        if not shutil.which(name):
            problems.append(f"Missing executable: {name}")
    if shutil.which("xcrun"):
        result = subprocess.run(["xcrun", "--sdk", "iphonesimulator", "--show-sdk-path"], text=True, capture_output=True)
        if result.returncode:
            problems.append("The Xcode iPhoneSimulator SDK is unavailable.")
        else:
            sdk = result.stdout.strip()
    if shutil.which("rustc"):
        result = subprocess.run(["rustc", "--print", "target-libdir", "--target", TARGET], text=True, capture_output=True)
        if result.returncode or not list(Path(result.stdout.strip()).glob("libstd-*.rlib")):
            problems.append(f"Rust standard library for {TARGET} is not installed in the selected toolchain.")
    # --output may be on a different disk; inspect its closest existing parent.
    output_volume = args.output.expanduser().resolve()
    while not output_volume.exists():
        output_volume = output_volume.parent
    free = shutil.disk_usage(output_volume).free
    if free < 6 * 1024**3:
        problems.append(f"Only {free / 1024**3:.2f} GiB free; reserve at least 6 GiB for a fresh simulator build.")
    print(json.dumps({"target": TARGET, "sdk": sdk, "ready_to_build": not problems, "problems": problems,
                      "distribution": "local Simulator bundle only; physical iPhone signing remains a release gate"}, indent=2))
    if problems:
        return 1
    if args.check:
        return 0
    out = args.output.expanduser().resolve()
    out.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env.update({"CARGO_TARGET_DIR": str(out / "cargo"), "IPHONEOS_DEPLOYMENT_TARGET": "15.0",
                "CARGO_PROFILE_DEV_DEBUG": "0", "CARGO_INCREMENTAL": "0", "SDKROOT": sdk})
    if args.server:
        env["OMOBA_DEFAULT_GAME_SERVER_ADDR"] = args.server
    subprocess.run(["cargo", "build", "--locked", "-p", "client", "--bin", "client", "--target", TARGET],
                   cwd=ROOT, env=env, check=True)
    bundle = out / "OmobaBeta.app"
    bundle.mkdir(exist_ok=True)
    shutil.copy2(out / f"cargo/{TARGET}/debug/client", bundle / "client")
    shutil.copytree(ROOT / "client/assets", bundle / "assets", dirs_exist_ok=True)
    with Path(__file__).with_name("Info.plist").open("rb") as source:
        info = plistlib.load(source)
    version = re.search(r'(?ms)^\[workspace\.package\]\s*\n(?:(?!^\[).)*?^version\s*=\s*"([^"\n]+)"', (ROOT / "Cargo.toml").read_text()).group(1)
    info["CFBundleShortVersionString"] = version.split("-", 1)[0]
    info["CFBundleSupportedPlatforms"] = ["iPhoneSimulator"]
    with (bundle / "Info.plist").open("wb") as destination:
        plistlib.dump(info, destination)
    subprocess.run(["codesign", "--force", "--sign", "-", bundle], check=True)
    print(f"Simulator artifact: {bundle}")
    print(f"Install into a running simulator: xcrun simctl install booted {bundle}")
    print("Launch: xcrun simctl launch booted space.ekza.omoba.beta")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, subprocess.CalledProcessError) as error:
        print(f"iOS Simulator build failed: {error}", file=sys.stderr)
        sys.exit(1)
