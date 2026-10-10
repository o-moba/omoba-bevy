#!/usr/bin/env python3
"""Collect OMOBA launch evidence from one USB-debugging-authorized Android device.

Read-only unless --launch is requested. Never clears logcat, reinstalls the app,
reads app storage, or records the device serial. Uses the existing Android adb.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
import zipfile

PACKAGE = "space.ekza.omoba.beta"


def apk_abis(path):
    with zipfile.ZipFile(path) as archive:
        return sorted({name.split("/")[1] for name in archive.namelist()
                       if re.fullmatch(r"lib/[^/]+/libclient\.so", name)})


def crash_excerpt(log):
    """Keep only crash-buffer PID groups explicitly naming our app, not other apps."""
    groups = {}
    for line in log.splitlines():
        match = re.match(r"\d\d-\d\d\s+[\d:.]+\s+(\d+)\s+\d+\s+[VDIWEF]\s", line)
        if match:
            groups.setdefault(match[1], []).append(line)
    selected = []
    for lines in groups.values():
        if not any(re.search(r"(?:Process:|Cmdline:)\s*" + re.escape(PACKAGE)
                             + r"(?:\s|,|$)", line) for line in lines):
            continue
        selected.extend(line for line in lines if not re.search(
            r"authorization|password|private.?key|profile.?key|bearer|token", line, re.I))
    return "\n".join(selected)[-32000:]


def assess(device, candidate_abis, crash, exits):
    findings = []
    sdk = device.get("sdk", "")
    if sdk.isdigit() and int(sdk) < 26:
        findings.append("Android API is below the APK minimum (26 / Android 8).")
    abis = set(device.get("abis", "").split(",")) - {""}
    if candidate_abis and abis and not abis.intersection(candidate_abis):
        findings.append("The supplied APK has no native ABI supported by this device.")
    features = device.get("features", "")
    if features and "android.hardware.vulkan.level" not in features:
        findings.append("Device does not advertise Vulkan level support; current APK requires it.")
    level = re.search(r"android\.hardware\.vulkan\.level=(\d+)", features)
    if level and int(level[1]) < 1:
        findings.append("Advertised Vulkan feature level is below the APK's declared level 1 requirement.")
    if re.search(r"UnsatisfiedLinkError|dlopen failed|cannot locate symbol", crash, re.I):
        findings.append("Retained crash log contains a native library loader failure.")
    if re.search(r"no suitable.*adapter|request.?device|Vulkan|VK_ERROR|wgpu", crash, re.I):
        findings.append("Retained crash log contains graphics initialization evidence; inspect the excerpt.")
    if re.search(r"LOW_MEMORY|low memory", exits, re.I):
        findings.append("Android records a historical low-memory termination.")
    if not findings:
        findings.append("No cause established. OS version alone cannot certify GPU/driver or launch compatibility.")
    return findings


def find_adb():
    found = shutil.which("adb")
    if found:
        return found
    for base in (os.getenv("ANDROID_HOME"), os.getenv("ANDROID_SDK_ROOT"),
                 str(Path.home() / "Library/Android/sdk"), str(Path.home() / "Android/Sdk")):
        if base and (Path(base) / "platform-tools/adb").is_file():
            return str(Path(base) / "platform-tools/adb")
    raise RuntimeError("adb is missing; use existing Android SDK platform-tools or --adb PATH.")


def collect(adb, serial=None, launch=False, apk=None):
    def command(args):
        result = subprocess.run([adb, *args], text=True, capture_output=True, timeout=30)
        if result.returncode:
            raise RuntimeError("adb command failed: " + result.stderr.strip()[:300])
        return result.stdout.strip()

    rows = [row.split() for row in command(["devices"]).splitlines()[1:] if row.strip()]
    available = [row[0] for row in rows if len(row) >= 2 and row[1] == "device"]
    if serial:
        if serial not in available:
            raise RuntimeError("Selected device is not connected and USB-debugging-authorized.")
    elif len(available) == 1:
        serial = available[0]
    else:
        raise RuntimeError("Connect and authorize exactly one Android device, or select one with --serial.")

    errors = []

    def shell(*args):
        try:
            return command(["-s", serial, "shell", *args])
        except (RuntimeError, subprocess.TimeoutExpired):
            errors.append("Unavailable: " + " ".join(args[:3]))
            return ""

    properties = {"manufacturer": "ro.product.manufacturer", "model": "ro.product.model",
                  "android": "ro.build.version.release", "sdk": "ro.build.version.sdk",
                  "security_patch": "ro.build.version.security_patch", "abis": "ro.product.cpu.abilist"}
    device = {key: shell("getprop", prop) for key, prop in properties.items()}
    device["features"] = "\n".join(line for line in shell("pm", "list", "features").splitlines()
                                   if any(term in line.lower() for term in ("vulkan", "opengles")))
    device["gpu"] = "\n".join(line.strip() for line in shell("dumpsys", "SurfaceFlinger").splitlines()
                              if line.strip().startswith("GLES:"))
    package = shell("dumpsys", "package", PACKAGE)
    package_summary = "\n".join(line.strip() for line in package.splitlines()
                                if re.search(r"versionName=|versionCode=|primaryCpuAbi=|secondaryCpuAbi=", line))
    launch_result = None
    if launch:
        launch_result = shell("am", "start", "-W", "-n", PACKAGE + "/android.app.NativeActivity")
        time.sleep(3)
    exits = "\n".join(line.strip() for line in shell("dumpsys", "activity", "exit-info", PACKAGE).splitlines()
                      if re.search(r"timestamp=|reason=|description=|status=|pss=|rss=", line))[-8000:]
    crash = crash_excerpt(shell("logcat", "-d", "-b", "crash", "-v", "threadtime", "-t", "2000"))
    candidate = apk_abis(apk) if apk else []
    return {"package": PACKAGE, "device": device, "installed_package": package_summary,
            "candidate_apk_abis": candidate, "launch_requested": launch, "launch_result": launch_result,
            "exit_history": exits, "crash_excerpt": crash, "collection_errors": errors,
            "findings": assess(device, candidate, crash, exits),
            "limits": "Crash/exit history may predate this attempt. Empty logs do not prove successful launch. "
                      "Candidate APK is not necessarily the installed APK. Review before sharing."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--adb")
    parser.add_argument("--serial", help="select one already authorized device; not saved in report")
    parser.add_argument("--apk", type=Path, help="optionally compare a local APK's native slices")
    parser.add_argument("--launch", action="store_true", help="explicitly start OMOBA before collecting evidence")
    parser.add_argument("--output", type=Path, default=Path("builds/android-diagnosis.json"))
    args = parser.parse_args()
    try:
        if args.output.exists():
            raise RuntimeError("Output exists; choose a new --output to preserve earlier evidence.")
        report = collect(args.adb or find_adb(), args.serial, args.launch, args.apk)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        with args.output.open("x") as output:
            json.dump(report, output, indent=2)
            output.write("\n")
        print(args.output)
        print("\n".join(report["findings"]))
        return 0
    except (OSError, RuntimeError, subprocess.TimeoutExpired, zipfile.BadZipFile) as error:
        print(str(error))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
