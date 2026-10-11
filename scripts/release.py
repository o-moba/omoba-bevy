#!/usr/bin/env python3
"""Turnkey OMOBA release builds.

One entry point for every platform package a playtest release ships:

    python3 scripts/release.py check                       # what this host can build
    python3 scripts/release.py build macos --server H:P    # one platform
    python3 scripts/release.py build all --server H:P      # everything this host can build
    python3 scripts/release.py notes                       # release notes from CHANGELOG.md
    python3 scripts/release.py draft                       # draft GitHub release + upload dist/
    python3 scripts/release.py ci --ref v0.24.0            # run the GitHub release workflow
    python3 scripts/release.py publish                     # pin dist/ packages to IPFS (Pinata)

Platforms: macos (Apple silicon .app), windows (x64 zip; on macOS cross-built
with cargo-xwin, otherwise Windows / GitHub Actions), linux (x64 client +
server tarball; Docker on macOS, offline when the registry is unreachable),
android (universal APK, needs SDK/NDK), ios (Xcode archive / TestFlight, needs
the Apple team). `make release-all` builds all five on an Apple-silicon Mac.

Artifacts land in dist/v<version>/ with SHA256SUMS.txt. `--server host:port`
compiles the initial server address into every client; players can change it
in the party lobby (desktop) or the SERVER keypad (phone).

Nothing here publishes on its own: `draft` creates an unpublished draft
release, `publish` pins packages to IPFS, `ios --upload` is the only step that
sends a build to App Store Connect, and each must be asked for explicitly.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import plistlib
import re
import shutil
import subprocess
import sys
import tempfile
import time
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

PLATFORMS = ("macos", "windows", "linux", "android", "ios")
DESKTOP_BINARIES = ("client", "server")
MAC_BUNDLE_ID = "space.ekza.omoba.desktop"
ANDROID_KEYSTORE = Path.home() / ".config/omoba/android-playtest.keystore"
REPO = "o-moba/omoba-bevy"


def android_universal() -> bool:
    """arm64-v8a + armeabi-v7a + x86_64 in one APK (make release-all); CI keeps arm64."""
    return os.environ.get("OMOBA_ANDROID_UNIVERSAL") == "1"


# ---------------------------------------------------------------- helpers

def workspace_version(root: Path = ROOT) -> str:
    text = (root / "Cargo.toml").read_text()
    match = re.search(r'(?ms)^\[workspace\.package\]\s*\n(?:(?!^\[).)*?^version\s*=\s*"([^"\n]+)"', text)
    if not match:
        raise SystemExit("Cargo.toml has no [workspace.package] version")
    return match.group(1)


def tag_for(version: str) -> str:
    return f"v{version}"


def android_version_code(version: str) -> int:
    """Monotonic Android versionCode: 0.24.0 -> 240009, 0.24.1-rc.1 -> 240010.

    A final release ends in 9 so it sorts after its own pre-releases.
    """
    core, _, pre = version.partition("-")
    major, minor, patch = (int(part) for part in core.split("."))
    return ((major * 1000 + minor) * 1000 + patch) * 10 + (0 if pre else 9)


def release_notes(version: str, changelog: str) -> str:
    """The CHANGELOG section for `version`, else the [Unreleased] section."""
    heading = re.compile(r"(?m)^## \[?([^\]\s]+)\]?.*$")
    sections = {}
    matches = list(heading.finditer(changelog))
    for index, match in enumerate(matches):
        end = matches[index + 1].start() if index + 1 < len(matches) else len(changelog)
        sections[match.group(1)] = changelog[match.end():end].strip()
    body = sections.get(version) or sections.get("Unreleased") or ""
    return body.strip() + "\n"


def artifact_name(version: str, target: str) -> str:
    return {
        "macos": f"Omoba-{version}-macos-arm64.zip",
        "windows": f"Omoba-{version}-windows-x64.zip",
        "linux": f"omoba-{version}-linux-x64.tar.gz",
        "android": f"Omoba-{version}-android-{'universal' if android_universal() else 'arm64'}.apk",
        "ios": f"Omoba-{version}-ios.ipa",
    }[target]


def host_platform() -> str:
    return {"Darwin": "macos", "Windows": "windows", "Linux": "linux"}.get(platform.system(), "unknown")


def run(*args, env=None, cwd=ROOT, capture=False):
    printable = " ".join(str(a) for a in args)
    print(f"$ {printable}", flush=True)
    if capture:
        return subprocess.check_output([str(a) for a in args], cwd=cwd, env=env, text=True).strip()
    subprocess.check_call([str(a) for a in args], cwd=cwd, env=env)
    return ""


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_checksums(directory: Path) -> Path:
    lines = [f"{sha256(p)}  {p.name}" for p in sorted(directory.iterdir())
             if p.is_file() and p.name != "SHA256SUMS.txt"]
    target = directory / "SHA256SUMS.txt"
    target.write_text("\n".join(lines) + "\n")
    return target


def shared_target() -> Path:
    """Cache A in the primary checkout, shared by every worktree."""
    common = subprocess.check_output(["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
                                     cwd=ROOT, text=True).strip()
    return Path(common).parent / "target"


def free_gib(path: Path = ROOT) -> float:
    return shutil.disk_usage(path).free / (1 << 30)


def xwin_env() -> dict | None:
    """cargo-xwin + LLVM for cross-building the Windows package on macOS, or None."""
    search = [os.environ.get("OMOBA_XWIN_TOOLS"), str(Path.home() / ".cargo/bin"),
              "/tmp/omoba-xwin-tools/bin", "/opt/homebrew/opt/llvm/bin", "/usr/local/opt/llvm/bin"]
    path = os.pathsep.join([d for d in search if d and Path(d).is_dir()] + [os.environ.get("PATH", "")])
    if not shutil.which("cargo-xwin", path=path) or not shutil.which("clang-cl", path=path):
        return None
    env = os.environ.copy()
    env["PATH"] = path
    env.setdefault("XWIN_CACHE_DIR", str(shared_target() / "xwin"))
    env["CARGO_INCREMENTAL"] = "0"
    env["RUSTFLAGS"] = (env.get("RUSTFLAGS", "") + " -C target-feature=+crt-static").strip()
    return env


def android_ndk() -> Path | None:
    candidates = [os.environ.get("ANDROID_NDK_HOME"), "/opt/homebrew/share/android-ndk"]
    sdk = os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
    if sdk and Path(sdk, "ndk").is_dir():
        candidates += [str(p) for p in sorted(Path(sdk, "ndk").iterdir(), reverse=True)]
    for candidate in candidates:
        if candidate and Path(candidate, "toolchains/llvm/prebuilt").is_dir():
            return Path(candidate)
    return None


def client_env(server: str | None) -> dict:
    env = os.environ.copy()
    if server:
        env["OMOBA_DEFAULT_GAME_SERVER_ADDR"] = server
    else:
        env.pop("OMOBA_DEFAULT_GAME_SERVER_ADDR", None)
    return env


def validate_server(server: str | None) -> str | None:
    if server is None:
        return None
    if not re.fullmatch(r"[A-Za-z0-9.\-]+:\d{1,5}", server) or not 0 < int(server.rsplit(":", 1)[1]) < 65536:
        raise SystemExit(f"--server must be host:port, got {server!r}")
    return server


def player_readme(version: str, target: str, server: str | None) -> str:
    server_line = (f"This build connects to {server} by default." if server else
                   "This build starts on 127.0.0.1:4000 (a server on the same computer).")
    common = f"""OMOBA {version} — playtest build
{'=' * (len(version) + 25)}

{server_line}

PLAY WITH A FRIEND
1. One of you hosts a practice server (see HOSTING below) or you both use
   a shared server address.
2. Both start the game. Home -> "Party & friends". On a computer, press
   "Change" under SERVER and type the host's address (host:port, for example
   192.168.1.20:4000). It is remembered for the next start.
3. Invite your friend from "Online on this server"; they press Accept.
4. The leader presses PLAY VS BOTS. Pick heroes. You play on one team and
   bots take the other seats.

HOSTING A PRACTICE SERVER
The host's UDP port 4000 must be reachable: same Wi-Fi/LAN, a VPN such as
Tailscale/ZeroTier, or a router port forward for UDP 4000.
"""
    specific = {
        "macos": """
MACOS
- Unzip and move Omoba.app to Applications (or anywhere).
- The build is not notarized. First start: right-click Omoba.app -> Open ->
  Open. If macOS still refuses, run in Terminal:
      xattr -dr com.apple.quarantine /path/to/Omoba.app
- Host a server: double-click "Host Practice Server.command" (same folder).
""",
        "windows": """
WINDOWS
- Unzip the whole folder, then run Omoba.exe.
- SmartScreen may warn about an unknown publisher: More info -> Run anyway.
- Host a server: double-click "Host Practice Server.bat". Allow it through
  Windows Firewall (private networks) when asked.
""",
        "linux": """
LINUX
- Play: ./omoba            (client)
- Host: ./host-practice.sh (practice server with bots on 0.0.0.0:4000)
- A long-running server: see omoba-practice.service (systemd example).
""",
        "android": """
ANDROID
- Allow installs from your browser/file manager, open the APK, install.
- Tap SERVER on the home screen to enter the host's address.
""",
    }.get(target, "")
    return common + specific + "\nLicenses and attributions: see the legal/ folder and ATTRIBUTION.md.\n"


# ---------------------------------------------------------------- checks

def check_report() -> dict:
    host = host_platform()
    have = lambda name: shutil.which(name) is not None
    rust_targets = set()
    if have("rustup"):
        rust_targets = set(subprocess.run(["rustup", "target", "list", "--installed"],
                                          capture_output=True, text=True).stdout.split())
    sdk = os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
    android_ready = bool(sdk and Path(sdk, "build-tools/35.0.0").is_dir()
                         and Path(sdk, "platforms/android-35").is_dir()
                         and android_ndk() is not None and have("java")
                         and ({"aarch64-linux-android", "armv7-linux-androideabi", "x86_64-linux-android"}
                              if android_universal() else {"aarch64-linux-android"}) <= rust_targets)
    ios_ready = host == "macos" and have("xcodebuild") and "aarch64-apple-ios" in rust_targets
    report = {
        "version": workspace_version(),
        "host": host,
        "git_dirty": bool(subprocess.run(["git", "status", "--porcelain"], cwd=ROOT,
                                         capture_output=True, text=True).stdout.strip()),
        "platforms": {
            "macos": host == "macos" and platform.machine() == "arm64",
            "windows": host == "windows" or (host == "macos" and xwin_env() is not None
                                              and "x86_64-pc-windows-msvc" in rust_targets),
            "linux": host == "linux" or (host == "macos" and have("docker")),
            "android": android_ready,
            "ios": ios_ready,
        },
        "gh": have("gh"),
        "free_gib": round(free_gib(), 1),
        "notes": {
            "windows": "on macOS: cargo install --locked cargo-xwin, brew install llvm, "
                       "rustup target add x86_64-pc-windows-msvc; else GitHub Actions (release.yml)",
            "linux": "on macOS, built inside Docker (rust:bookworm); offline fallback reuses "
                     "target/linux-docker/toolchain-<rust> when the registry is unreachable",
            "android": "needs ANDROID_HOME with build-tools 35.0.0, platforms android-35, an NDK "
                       "(ANDROID_NDK_HOME or brew android-ndk), java and the three Android rust targets",
            "ios": "needs Xcode signed in to the Apple team (mobile/ios/Omoba.local.xcconfig)",
        },
    }
    return report


# ---------------------------------------------------------------- desktop

def cargo_release(binaries, env, target=None, command=("cargo", "build")) -> dict[str, Path]:
    args = [*command, "--locked", "--release", "--message-format=json-render-diagnostics"]
    for name in binaries:
        args += ["-p", name]
    if target:
        args += ["--target", target]
    print("$ " + " ".join(args), flush=True)
    process = subprocess.Popen(args, cwd=ROOT, env=env, stdout=subprocess.PIPE, text=True)
    found = {}
    for line in process.stdout:
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if event.get("reason") == "compiler-artifact" and event.get("executable"):
            found[event["target"]["name"]] = Path(event["executable"])
    if process.wait() != 0:
        raise SystemExit("cargo build --release failed")
    missing = set(binaries) - found.keys()
    if missing:
        raise SystemExit("cargo produced no executable for: " + ", ".join(sorted(missing)))
    return found


def stage_common(destination: Path, version: str, target: str, server: str | None, asset_dir: Path):
    """Assets (tracked only, content-gated), legal notices and the README."""
    from package_licenses import collect_legal_notices, copy_legal_notices
    from validate_candidate_assets import validate
    tracked = subprocess.check_output(["git", "ls-files", "client/assets"], cwd=ROOT, text=True).splitlines()
    for relative in tracked:
        source = ROOT / relative
        output = asset_dir / source.relative_to(ROOT / "client/assets")
        output.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, output)
    gate = validate(asset_dir)
    if gate["status"] != "PASS":
        raise SystemExit("packaged asset content gate failed: " + "; ".join(gate["errors"]))
    copy_legal_notices(collect_legal_notices(ROOT), destination / "legal")
    shutil.copy2(ROOT / "ATTRIBUTION.md", destination / "ATTRIBUTION.md")
    (destination / "README.txt").write_text(player_readme(version, target, server))


def zip_tree(source: Path, archive: Path, root_name: str):
    """Zip preserving executable bits (Info-ZIP attributes) for macOS/Linux."""
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as bundle:
        for path in sorted(source.rglob("*")):
            relative = Path(root_name) / path.relative_to(source)
            if path.is_symlink():
                continue
            info = zipfile.ZipInfo.from_file(path, str(relative))
            if path.is_dir():
                bundle.writestr(info, b"")
                continue
            info.compress_type = zipfile.ZIP_DEFLATED
            with path.open("rb") as handle:
                bundle.writestr(info, handle.read())


def mac_icon(resources: Path):
    source = ROOT / "mobile/ios/Assets.xcassets/AppIcon.appiconset/AppIcon.png"
    if not source.is_file() or not shutil.which("iconutil"):
        return None
    with tempfile.TemporaryDirectory() as temporary:
        iconset = Path(temporary) / "Omoba.iconset"
        iconset.mkdir()
        for size in (16, 32, 128, 256, 512):
            for scale in (1, 2):
                name = f"icon_{size}x{size}{'@2x' if scale == 2 else ''}.png"
                subprocess.check_call(["sips", "-z", str(size * scale), str(size * scale), str(source),
                                       "--out", str(iconset / name)], stdout=subprocess.DEVNULL)
        subprocess.check_call(["iconutil", "-c", "icns", str(iconset), "-o", str(resources / "Omoba.icns")])
    return "Omoba.icns"


def build_macos(version: str, out: Path, server: str | None) -> Path:
    if host_platform() != "macos":
        raise SystemExit("macos packages are built on macOS (or by the GitHub release workflow)")
    binaries = cargo_release(DESKTOP_BINARIES, client_env(server))
    with tempfile.TemporaryDirectory() as temporary:
        stage = Path(temporary) / "Omoba"
        app = stage / "Omoba.app"
        macos_dir = app / "Contents/MacOS"
        resources = app / "Contents/Resources"
        macos_dir.mkdir(parents=True)
        resources.mkdir(parents=True)
        shutil.copy2(binaries["client"], macos_dir / "client")
        shutil.copy2(binaries["server"], macos_dir / "server")
        stage_common(stage, version, "macos", server, resources / "assets")
        icon = mac_icon(resources)
        info = {
            "CFBundleName": "Omoba", "CFBundleDisplayName": "OMOBA",
            "CFBundleIdentifier": MAC_BUNDLE_ID, "CFBundleExecutable": "client",
            "CFBundlePackageType": "APPL", "CFBundleShortVersionString": version.split("-")[0],
            "CFBundleVersion": version, "LSMinimumSystemVersion": "12.0",
            "NSHighResolutionCapable": True, "LSApplicationCategoryType": "public.app-category.games",
        }
        if icon:
            info["CFBundleIconFile"] = icon
        with (app / "Contents/Info.plist").open("wb") as handle:
            plistlib.dump(info, handle)
        host = stage / "Host Practice Server.command"
        host.write_text("""#!/bin/sh
# Practice server with bots for you and your friends (UDP 4000).
DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
export SERVER_ADDR="${SERVER_ADDR:-0.0.0.0:4000}"
export OMOBA_MATCH_MODE=practice
export OMOBA_ASSET_DIR="$DIR/Omoba.app/Contents/Resources/assets"
echo "OMOBA practice server on $SERVER_ADDR. Friends connect to this computer's IP, port 4000."
echo "Close this window to stop the server."
exec "$DIR/Omoba.app/Contents/MacOS/server"
""")
        host.chmod(0o755)
        # Ad-hoc signature: required on Apple silicon; not a notarized identity.
        run("codesign", "--force", "--deep", "--sign", "-", app)
        archive = out / artifact_name(version, "macos")
        # ditto keeps the bundle layout and signature intact.
        run("ditto", "-c", "-k", "--keepParent", stage, archive)
        return archive


def stage_windows(stage: Path, version: str, server: str | None, client: Path, server_binary: Path):
    stage.mkdir(parents=True)
    for binary, name in ((client, "Omoba.exe"), (server_binary, "omoba-server.exe")):
        raw = binary.read_bytes()[:4096]
        offset = int.from_bytes(raw[60:64], "little")
        if raw[:2] != b"MZ" or raw[offset:offset + 6] != b"PE\x00\x00\x64\x86":
            raise SystemExit(f"{binary} is not an x64 Windows executable")
        shutil.copy2(binary, stage / name)
    stage_common(stage, version, "windows", server, stage / "assets")
    (stage / "Host Practice Server.bat").write_text(
        "@echo off\r\n"
        "rem Practice server with bots for you and your friends (UDP 4000).\r\n"
        "cd /d \"%~dp0\"\r\n"
        "if \"%SERVER_ADDR%\"==\"\" set SERVER_ADDR=0.0.0.0:4000\r\n"
        "set OMOBA_MATCH_MODE=practice\r\n"
        "set OMOBA_ASSET_DIR=%~dp0assets\r\n"
        "echo OMOBA practice server on %SERVER_ADDR%. Friends connect to this PC's IP, port 4000.\r\n"
        "omoba-server.exe\r\n"
        "pause\r\n")


def build_windows(version: str, out: Path, server: str | None) -> Path:
    host = host_platform()
    if host == "windows":
        binaries = cargo_release(DESKTOP_BINARIES, client_env(server))
    elif host == "macos" and (env := xwin_env()):
        # Cross-build with the Microsoft CRT/SDK that cargo-xwin caches; static CRT.
        if server:
            env["OMOBA_DEFAULT_GAME_SERVER_ADDR"] = server
        else:
            env.pop("OMOBA_DEFAULT_GAME_SERVER_ADDR", None)
        binaries = cargo_release(DESKTOP_BINARIES, env, target="x86_64-pc-windows-msvc",
                                 command=("cargo", "xwin", "build"))
    else:
        raise SystemExit("windows packages need Windows, cargo-xwin on macOS (see `check`), "
                         "or the GitHub release workflow (python3 scripts/release.py ci)")
    with tempfile.TemporaryDirectory() as temporary:
        stage = Path(temporary) / "Omoba"
        stage_windows(stage, version, server, binaries["client"], binaries["server"])
        archive = out / artifact_name(version, "windows")
        zip_tree(stage, archive, "Omoba")
        return archive


def stage_linux(stage: Path, version: str, server: str | None, client: Path, server_binary: Path):
    stage.mkdir(parents=True)
    shutil.copy2(client, stage / "omoba")
    shutil.copy2(server_binary, stage / "omoba-server")
    stage_common(stage, version, "linux", server, stage / "assets")
    host = stage / "host-practice.sh"
    host.write_text("""#!/bin/sh
# Practice server with bots (UDP 4000 by default).
set -eu
DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
export SERVER_ADDR="${SERVER_ADDR:-0.0.0.0:4000}"
export OMOBA_MATCH_MODE="${OMOBA_MATCH_MODE:-practice}"
export OMOBA_ASSET_DIR="$DIR/assets"
exec "$DIR/omoba-server" "$@"
""")
    host.chmod(0o755)
    (stage / "omoba-practice.service").write_text("""# systemd example: copy the package to /opt/omoba, then
#   sudo cp omoba-practice.service /etc/systemd/system/
#   sudo systemctl enable --now omoba-practice
# and open UDP 4000 in the firewall (ufw allow 4000/udp).
[Unit]
Description=OMOBA practice server
After=network-online.target

[Service]
User=omoba
WorkingDirectory=/opt/omoba
ExecStart=/opt/omoba/host-practice.sh
Restart=on-failure
Environment=SERVER_ADDR=0.0.0.0:4000

[Install]
WantedBy=multi-user.target
""")


LINUX_DEBS = ("libasound2", "libasound2-data", "libasound2-dev", "libudev1", "libudev-dev")


def linux_toolchain(cache: Path, toolchain: str) -> Path | None:
    """A Linux Rust toolchain unpacked under the cache, named by full or major.minor version."""
    for name in (toolchain, ".".join(toolchain.split(".")[:2])):
        if (cache / f"toolchain-{name}/bin/cargo").is_file():
            return cache / f"toolchain-{name}"
    return None


def linux_offline_prepare(cache: Path, toolchain: str) -> Path:
    """Fetch crates and the ALSA/udev -dev packages on the host for a --network none build.

    Docker Desktop sends container and registry traffic through the system proxy;
    when that proxy is down the host can still reach crates.io and deb.debian.org.
    """
    installed = linux_toolchain(cache, toolchain)
    if not installed:
        raise SystemExit(f"Docker cannot reach the registry and {cache}/toolchain-{toolchain} is missing; "
                         "fix Docker's proxy or install that Rust toolchain (x86_64-unknown-linux-gnu) there")
    env = os.environ.copy()
    env["CARGO_HOME"] = str(cache / "cargo-home")
    run("cargo", "fetch", "--locked", env=env)
    debs = cache / "debs"
    debs.mkdir(parents=True, exist_ok=True)
    if all(any(debs.glob(f"{name}_*.deb")) for name in LINUX_DEBS):
        return installed
    import lzma
    import urllib.request
    index = lzma.decompress(urllib.request.urlopen(
        "http://deb.debian.org/debian/dists/bookworm/main/binary-amd64/Packages.xz", timeout=120).read()).decode()
    for block in index.split("\n\n"):
        fields = dict(line.split(": ", 1) for line in block.splitlines() if ": " in line and not line.startswith(" "))
        if fields.get("Package") in LINUX_DEBS:
            target = debs / Path(fields["Filename"]).name
            data = urllib.request.urlopen("http://deb.debian.org/debian/" + fields["Filename"], timeout=120).read()
            if hashlib.sha256(data).hexdigest() != fields["SHA256"]:
                raise SystemExit(f"checksum mismatch for {target.name}")
            target.write_bytes(data)
    return installed


def build_linux(version: str, out: Path, server: str | None) -> Path:
    archive = out / artifact_name(version, "linux")
    if host_platform() == "linux":
        binaries = cargo_release(DESKTOP_BINARIES, client_env(server))
        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary) / f"omoba-{version}"
            stage_linux(stage, version, server, binaries["client"], binaries["server"])
            run("tar", "-czf", archive, "-C", stage.parent, stage.name)
        return archive
    if not shutil.which("docker"):
        raise SystemExit("linux packages need a Linux host, Docker, or the GitHub release workflow")
    # Build inside the pinned toolchain image, then package here.
    toolchain = re.search(r'channel\s*=\s*"([^"]+)"', (ROOT / "rust-toolchain.toml").read_text()).group(1)
    common = shared_target().parent
    target_dir = common / "target/linux-docker"
    image = f"rust:{toolchain}-bookworm"
    env_args = ["-e", f"OMOBA_DEFAULT_GAME_SERVER_ADDR={server}"] if server else []
    mounts = ["-v", f"{ROOT}:/src", "-v", f"{target_dir}:/cache", "-w", "/src"]
    # A successful pull also proves the container can reach apt.
    if subprocess.run(["docker", "pull", "--platform", "linux/amd64", image]).returncode == 0:
        script = ("apt-get update -qq && apt-get install -y -qq --no-install-recommends "
                  "libasound2-dev libudev-dev pkg-config >/dev/null && "
                  "cargo build --locked --release -p client -p server")
        run("docker", "run", "--rm", "--platform", "linux/amd64", *mounts,
            "-e", "CARGO_TARGET_DIR=/cache", *env_args, image, "sh", "-c", script)
    else:
        print("!! Docker registry unreachable; building offline from the local cache", flush=True)
        installed = linux_offline_prepare(target_dir, toolchain)
        base = os.environ.get("OMOBA_LINUX_IMAGE") or next(
            (line.split()[1] for line in subprocess.run(
                ["docker", "images", "--format", "{{.Tag}} {{.ID}}", "rust"],
                capture_output=True, text=True).stdout.splitlines() if "alpine" not in line), None)
        if not base:
            raise SystemExit("no local glibc rust image for the offline Linux build (set OMOBA_LINUX_IMAGE)")
        script = ("dpkg -i /cache/debs/*.deb >/dev/null && "
                  "cargo build --offline --locked --release -p client -p server")
        run("docker", "run", "--rm", "--network", "none", "--platform", "linux/amd64", *mounts,
            "-e", "CARGO_TARGET_DIR=/cache", "-e", "CARGO_HOME=/cache/cargo-home",
            "-e", f"PATH=/cache/{installed.name}/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
            *env_args, base, "sh", "-c", script)
    release = target_dir / "release"
    with tempfile.TemporaryDirectory() as temporary:
        stage = Path(temporary) / f"omoba-{version}"
        stage_linux(stage, version, server, release / "client", release / "server")
        run("tar", "-czf", archive, "-C", stage.parent, stage.name)
    return archive


# ---------------------------------------------------------------- mobile

def ensure_android_keystore() -> Path:
    """One stable playtest key, so an update installs over the previous APK."""
    encoded = os.environ.get("OMOBA_ANDROID_KEYSTORE_BASE64")
    if encoded:
        import base64
        path = Path(tempfile.mkdtemp()) / "playtest.keystore"
        path.write_bytes(base64.b64decode(encoded))
        return path
    if not ANDROID_KEYSTORE.exists():
        ANDROID_KEYSTORE.parent.mkdir(parents=True, exist_ok=True)
        run("keytool", "-genkeypair", "-keystore", ANDROID_KEYSTORE, "-storepass", "android",
            "-keypass", "android", "-alias", "androiddebugkey", "-keyalg", "RSA", "-keysize", "2048",
            "-validity", "10000", "-dname", "CN=Omoba Playtest,O=Omoba,C=US")
    return ANDROID_KEYSTORE


def build_android(version: str, out: Path, server: str | None) -> Path:
    # Reuse the one Android cargo cache; build.py signs with <output>/local-debug.keystore.
    work = shared_target() / "mobile/android"
    work.mkdir(parents=True, exist_ok=True)
    shutil.copy2(ensure_android_keystore(), work / "local-debug.keystore")
    abi = "universal" if android_universal() else "arm64"
    args = [sys.executable, ROOT / "mobile/android/build.py", "--output", work,
            "--version-code", str(android_version_code(version))]
    if android_universal():
        args.append("--universal")
    if ndk := android_ndk():
        args += ["--ndk", ndk]
    if server:
        args += ["--server", server]
    run(*args)
    apk = work / f"omoba-{version}-android-{abi}-debug.apk"
    if not apk.is_file():
        raise SystemExit(f"Android build produced no {apk.name}")
    for leftover in ("omoba-unaligned.apk", "omoba-unsigned.apk"):
        (work / leftover).unlink(missing_ok=True)
    target = out / artifact_name(version, "android")
    shutil.copy2(apk, target)
    return target


def local_xcconfig() -> dict:
    path = ROOT / "mobile/ios/Omoba.local.xcconfig"
    if not path.is_file():
        main_checkout = Path(subprocess.check_output(
            ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"], cwd=ROOT, text=True).strip()).parent
        candidate = main_checkout / "mobile/ios/Omoba.local.xcconfig"
        if candidate.is_file():
            shutil.copy2(candidate, path)
    values = {}
    if path.is_file():
        for line in path.read_text().splitlines():
            if "=" in line and not line.strip().startswith("//"):
                key, value = line.split("=", 1)
                values[key.strip()] = value.strip()
    return values


def remember_ios_build(number: str) -> None:
    """Record the last used build number in the ignored local config (and the primary checkout's)."""
    path = ROOT / "mobile/ios/Omoba.local.xcconfig"
    text = re.sub(r"(?m)^CURRENT_PROJECT_VERSION\s*=.*$", f"CURRENT_PROJECT_VERSION = {number}", path.read_text())
    path.write_text(text)
    main_copy = Path(subprocess.check_output(
        ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"], cwd=ROOT, text=True).strip()
    ).parent / "mobile/ios/Omoba.local.xcconfig"
    if main_copy != path and main_copy.is_file():
        main_copy.write_text(text)


def build_ios(version: str, out: Path, server: str | None, build_number: str | None, upload: bool) -> Path:
    if host_platform() != "macos":
        raise SystemExit("iOS builds need macOS with Xcode")
    config = local_xcconfig()
    team = config.get("DEVELOPMENT_TEAM")
    if not team:
        raise SystemExit("Set DEVELOPMENT_TEAM in mobile/ios/Omoba.local.xcconfig (see mobile/ios/TESTFLIGHT.md)")
    number = build_number or str(int(config.get("CURRENT_PROJECT_VERSION", "1")) + 1)
    work = ROOT / "target/release-ios"
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)
    archive = work / "Omoba.xcarchive"
    settings = [f"DEVELOPMENT_TEAM={team}", f"CURRENT_PROJECT_VERSION={number}",
                "OMOBA_CARGO_PROFILE=release", "CODE_SIGN_STYLE=Automatic"]
    if server:
        settings.append(f"OMOBA_GAME_SERVER={server}")
    run("xcodebuild", "-project", ROOT / "mobile/ios/Omoba.xcodeproj", "-scheme", "Omoba",
        "-configuration", "Release", "-destination", "generic/platform=iOS",
        "-archivePath", archive, "-allowProvisioningUpdates", "archive", *settings)
    options = work / "ExportOptions.plist"
    with options.open("wb") as handle:
        plistlib.dump({"method": "app-store-connect", "teamID": team, "signingStyle": "automatic",
                       "destination": "upload" if upload else "export",
                       "manageAppVersionAndBuildNumber": False,
                       "uploadSymbols": True}, handle)
    export = work / "export"
    kept = out / f"Omoba-{version}-ios-{number}.xcarchive"
    shutil.rmtree(kept, ignore_errors=True)
    run("ditto", archive, kept)
    try:
        run("xcodebuild", "-exportArchive", "-archivePath", archive, "-exportPath", export,
            "-exportOptionsPlist", options, "-allowProvisioningUpdates")
    except subprocess.CalledProcessError:
        # Command-line export needs an Apple ID in Xcode → Settings → Accounts ("Failed to
        # Use Accounts" otherwise). The signed archive is kept and handed to Organizer.
        organizer = Path.home() / "Library/Developer/Xcode/Archives" / time.strftime("%Y-%m-%d")
        organizer.mkdir(parents=True, exist_ok=True)
        listed = organizer / f"Omoba {version} ({number}).xcarchive"
        shutil.rmtree(listed, ignore_errors=True)
        run("ditto", archive, listed)
        subprocess.run(["open", "-a", "Xcode", str(listed)])
        # This number is now taken by a signed archive; the next run uses a fresh one.
        remember_ios_build(number)
        print(f"!! iOS export/upload was refused; signed archive {kept.name} is in Xcode Organizer: "
              "Distribute App → App Store Connect → Upload", file=sys.stderr)
        return kept
    if upload:
        print(f"Uploaded build {version} ({number}) to App Store Connect; assign it to testers in TestFlight.")
        remember_ios_build(number)
    ipas = list(export.glob("*.ipa"))
    target = out / artifact_name(version, "ios")
    if ipas:
        shutil.copy2(ipas[0], target)
        return target
    return archive


# ---------------------------------------------------------------- commands

def dist_dir(version: str, base: Path | None) -> Path:
    directory = (base or ROOT / "dist") / tag_for(version)
    directory.mkdir(parents=True, exist_ok=True)
    return directory


def write_compatibility_manifest(out: Path, version: str) -> Path:
    """Keep the exact shared Rust contract beside the artifacts being built."""
    common = Path(run("git", "rev-parse", "--path-format=absolute", "--git-common-dir", capture=True))
    env = dict(os.environ)
    env.setdefault("CARGO_TARGET_DIR", str(common.parent / "target"))
    raw = run("cargo", "run", "--locked", "--quiet", "-p", "shared", "--example",
              "compatibility", "--", "manifest", env=env, capture=True)
    manifest = json.loads(raw)
    if manifest.get("release") != version:
        raise SystemExit("Compatibility manifest does not match the artifact release")
    path = out / "compatibility.json"
    path.write_text(json.dumps(manifest, indent=2) + "\n")
    return path


def command_build(args) -> int:
    version = workspace_version()
    server = validate_server(args.server)
    out = dist_dir(version, args.out)
    report = check_report()
    minimum = float(os.environ.get("OMOBA_MIN_FREE_GIB", "20"))
    if args.platform == "all" and report["free_gib"] < minimum:
        raise SystemExit(f"only {report['free_gib']} GiB free; a full release needs ~{minimum:g} GiB "
                         "(set OMOBA_MIN_FREE_GIB to override)")
    write_compatibility_manifest(out, version)
    targets = [p for p in PLATFORMS if report["platforms"][p]] if args.platform == "all" else [args.platform]
    if args.platform == "all":
        # TestFlight upload never happens implicitly.
        print("Building on this host: " + ", ".join(targets))
    built, failed = [], {}
    for target in targets:
        try:
            if target == "macos":
                built.append(build_macos(version, out, server))
            elif target == "windows":
                built.append(build_windows(version, out, server))
            elif target == "linux":
                built.append(build_linux(version, out, server))
            elif target == "android":
                built.append(build_android(version, out, server))
            elif target == "ios":
                built.append(build_ios(version, out, server, args.ios_build, args.upload))
        except (SystemExit, subprocess.CalledProcessError) as error:
            failed[target] = str(error)
            print(f"!! {target} failed: {error}", file=sys.stderr)
            if args.platform != "all":
                raise
    write_checksums(out)
    summary = {"version": version, "server": server, "dist": str(out),
               "built": [str(p) for p in built], "failed": failed}
    print(json.dumps(summary, indent=2))
    return 1 if failed and not built else 0


def pinata_pin(path: Path, token: str) -> str:
    """Pin one file wrapped in a directory (so the URL keeps its name); return the CID."""
    if any(c in token for c in '\r\n"\\'):
        raise SystemExit("PINATA_JWT contains unexpected characters")
    print(f"$ pin {path.name} to Pinata", flush=True)
    # The token goes through curl's stdin config so it never appears in argv or logs.
    result = subprocess.run(
        ["curl", "--silent", "--show-error", "--fail-with-body", "--retry", "3", "--config", "-",
         "--request", "POST", "https://api.pinata.cloud/pinning/pinFileToIPFS",
         "--form", f"file=@{path};filename={path.name}",
         "--form", 'pinataOptions={"cidVersion":1,"wrapWithDirectory":true}',
         "--form", "pinataMetadata=" + json.dumps({"name": path.name})],
        input=f'header = "Authorization: Bearer {token}"\n', capture_output=True, text=True)
    if result.returncode:
        raise SystemExit(f"Pinata rejected {path.name}: {result.stderr.strip() or result.stdout[:300]}")
    return json.loads(result.stdout)["IpfsHash"]


def command_publish(args) -> int:
    """Pin every built package to IPFS, re-download it and record the verified links."""
    token = os.environ.get("PINATA_JWT")
    if not token:
        raise SystemExit("Set PINATA_JWT to pin packages")
    version = workspace_version()
    out = dist_dir(version, args.out)
    receipt_path = out / "ipfs.json"
    receipt = json.loads(receipt_path.read_text()) if receipt_path.is_file() else {}
    for target in PLATFORMS:
        path = out / artifact_name(version, target)
        if not path.is_file():
            continue
        digest = sha256(path)
        entry = receipt.get(target)
        if entry and entry.get("sha256") == digest:
            continue
        cid = pinata_pin(path, token)
        url = f"{args.gateway.rstrip('/')}/ipfs/{cid}/{path.name}"
        downloaded = Path(tempfile.mkdtemp()) / path.name
        run("curl", "--silent", "--show-error", "--fail", "--location", "--retry", "3",
            "--output", downloaded, url)
        if sha256(downloaded) != digest:
            raise SystemExit(f"gateway bytes for {path.name} do not match the local package")
        downloaded.unlink()
        receipt[target] = {"file": path.name, "bytes": path.stat().st_size, "sha256": digest,
                           "cid": cid, "url": url}
        receipt_path.write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps(receipt, indent=2))
    return 0


def command_notes(args) -> int:
    version = workspace_version()
    print(release_notes(version, (ROOT / "CHANGELOG.md").read_text()))
    return 0


def command_draft(args) -> int:
    version = workspace_version()
    tag = args.tag or tag_for(version)
    out = dist_dir(version, args.out)
    files = sorted(p for p in out.iterdir() if p.is_file())
    if not files:
        raise SystemExit(f"{out} is empty; build first")
    write_checksums(out)
    files = sorted(p for p in out.iterdir() if p.is_file())
    notes = out.parent / f"{tag}-notes.md"
    notes.write_text(release_notes(version, (ROOT / "CHANGELOG.md").read_text()))
    exists = subprocess.run(["gh", "release", "view", tag, "--repo", REPO],
                            capture_output=True).returncode == 0
    if exists:
        run("gh", "release", "upload", tag, *files, "--clobber", "--repo", REPO)
    else:
        run("gh", "release", "create", tag, *files, "--repo", REPO, "--draft", "--prerelease",
            "--title", f"OMOBA {version} (playtest)", "--notes-file", notes,
            "--target", args.target)
    print(f"Draft release {tag} is ready. Review it on GitHub; publish with: "
          f"gh release edit {tag} --draft=false --repo {REPO}")
    return 0


def command_ci(args) -> int:
    fields = ["-f", f"tag={args.ref}"]
    if args.server:
        fields += ["-f", f"server={validate_server(args.server)}"]
    run("gh", "workflow", "run", "release.yml", "--repo", REPO, "--ref", args.branch, *fields)
    print("Started. Watch with: gh run watch --repo " + REPO)
    return 0


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("check", help="what this host can build")
    build = sub.add_parser("build", help="build platform packages into dist/v<version>/")
    build.add_argument("platform", choices=PLATFORMS + ("all",))
    build.add_argument("--server", help="initial game server host:port compiled into clients")
    build.add_argument("--out", type=Path, help="dist base directory (default: dist/)")
    build.add_argument("--ios-build", help="iOS build number (default: local xcconfig + 1)")
    build.add_argument("--upload", action="store_true", help="iOS: upload to App Store Connect / TestFlight")
    sub.add_parser("notes", help="print release notes for the current version")
    publish = sub.add_parser("publish", help="pin dist/ packages to IPFS (Pinata) and verify the gateway copy")
    publish.add_argument("--out", type=Path)
    publish.add_argument("--gateway", default="https://ekza.mypinata.cloud")
    draft = sub.add_parser("draft", help="create or update a DRAFT GitHub release with dist/ files")
    draft.add_argument("--tag")
    draft.add_argument("--target", default="main", help="commit/branch the new tag points at")
    draft.add_argument("--out", type=Path)
    ci = sub.add_parser("ci", help="run the GitHub release workflow (all platforms)")
    ci.add_argument("--ref", default=tag_for(workspace_version()), help="release tag to create/update")
    ci.add_argument("--branch", default="main", help="branch whose workflow and code are built")
    ci.add_argument("--server")
    args = parser.parse_args(argv)
    if args.command == "check":
        print(json.dumps(check_report(), indent=2))
        return 0
    return {"build": command_build, "notes": command_notes, "draft": command_draft, "ci": command_ci,
            "publish": command_publish}[args.command](args)


if __name__ == "__main__":
    sys.exit(main())
