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
compiles the lobby into every client and `--registry URL` the Ekza registry
origin (self-hosted builds). Production clients have no server selection: on a
desktop GAME_SERVER_ADDR / OMOBA_REGISTRY_URL override them, phones take none.

DEV-contour tester builds are the separate OMOBA Dev app on every platform. They
point at the dev lobby on the dedicated dev VPS and the dev registry (their Dev
connection profile; the in-game Connection panel also offers Local and Custom), use a
closed loopback passport (there is no dev Ekza passport yet, and avatar.ekza.io is
production), and never share state or an app identity with a production install:

    python3 scripts/release.py build macos --contour dev   # lobby.dev.omoba.io:4100 + registry.dev.ekza.io
    python3 scripts/release.py build macos --contour dev --server <dev VPS IPv4>:4100   # without DNS
    python3 scripts/release.py build android --contour dev # space.ekza.omoba.dev, "OMOBA Dev"
    python3 scripts/release.py build ios --contour dev     # space.ekza.omoba.dev archive (TestFlight: --upload)

The client resolves a host name itself (lobby and match worker), so the default
needs only the DNS A record lobby.dev.omoba.io -> dev VPS. Dev packages land in
dist/v<version>-dev/ as Omoba-<version>-dev-*.zip / omoba-<version>-dev-*.tar.gz,
keep settings, session and caches in an `omoba-bevy-dev` folder instead of
`omoba-bevy`, title their desktop window "OMOBA DEV", and the macOS app is
"Omoba Dev.app" with its own bundle id and the DEV-ribbon icon. The phone apps are
space.ekza.omoba.dev ("OMOBA Dev", scripts/contours.py), beside space.ekza.omoba.beta.
The dev Linux package also carries omoba-account-api, the dev lobby's schema
migration tool. A production build refuses dev endpoints (and $DEV_IP when set); a
dev build refuses production ones. Production clients have no server selection.
No build inherits a compile-time default (OMOBA_DEFAULT_*, OMOBA_ACCOUNT_API_URL,
OMOBA_PORTAL_ORIGIN, EKZA_DEV_HTTP_HOST) from the shell.

Markets (scripts/markets.py, shared/src/market.rs) are a second axis: a build
serves one market. `--market ru` is a free game on the Russian servers (no Ekza
ecosystem, no purchases, Russian by default, VK ID sign-in on the RU portal);
`--feature ekza=on` flips one feature for a test build without a new market:

    python3 scripts/release.py markets                      # presets, servers, platforms
    python3 scripts/release.py preflight --market ru        # are the RU servers up?
    python3 scripts/release.py build all --market ru        # Omoba-<v>-ru-* in dist/v<v>-ru/
    python3 scripts/release.py build macos --market ru --server 127.0.0.1:4000 \
        --account-api http://127.0.0.1:40550 --portal http://127.0.0.1:3010   # local RU stack

RU packages keep their settings in `omoba-bevy-ru`, the Android app is
space.ekza.omoba.ru, and the RU Linux package also carries omoba-account-api
(built for the market) for the RU servers. See docs/markets.md.

A market with its own download host (scripts/markets.py `downloads_path`, LR-23)
publishes there first and keeps IPFS as the mirror:

    python3 scripts/release.py publish --market ru --rsync-target vds-ru:/opt/omoba-ru/downloads
    python3 scripts/release.py catalog --market ru           # releases.json: host URLs + IPFS mirrors
    python3 scripts/release.py verify-downloads --market ru  # what the portal lists, byte for byte

Nothing here publishes on its own: `draft` creates an unpublished draft
release, `publish` uploads packages to the market's download host and pins them
to IPFS, `ios --upload` is the only step that sends a build to App Store
Connect, and each must be asked for explicitly.
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
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

PLATFORMS = ("macos", "windows", "linux", "android", "ios")
DESKTOP_BINARIES = ("client", "server")
MAC_BUNDLE_ID = "space.ekza.omoba.desktop"
ANDROID_KEYSTORE = Path.home() / ".config/omoba/android-playtest.keystore"
REPO = "o-moba/omoba-bevy"

# Build-time client defaults a Flavor sets (client/src/session_config.rs,
# passport/src/community.rs, passport/src/lib.rs, client/src/platform.rs) and the
# contours' identities: scripts/contours.py, shared with the mobile packagers.
from contours import (ACCOUNT_API_ENV, PORTAL_ORIGIN_ENV, DEV_ACCOUNT_API, DEV_PORTAL_ORIGIN,
                      CONFIG_SUFFIX_ENV, CONTOURS, DEV_CONFIG_SUFFIX, DEV_LOBBY,  # noqa: E402
                      DEV_PASSPORT, DEV_REGISTRY, MOBILE, PASSPORT_DEFAULT_ENV, REGISTRY_DEFAULT_ENV,
                      SERVER_DEFAULT_ENV)
from ios_release_guard import require_production_source
import markets as market_presets  # noqa: E402
from markets import FEATURE_ENVS, MARKET_ENV, MARKETS  # noqa: E402
CLIENT_DEFAULT_ENVS = (SERVER_DEFAULT_ENV, REGISTRY_DEFAULT_ENV, PASSPORT_DEFAULT_ENV, CONFIG_SUFFIX_ENV)
# Every compile-time setting the workspace (`option_env!`) or the pinned Ekza SDK
# (EKZA_DEV_HTTP_HOST, ekza-bevy-sdk src/registry.rs) reads. A build drops all of
# them, and anything else named OMOBA_DEFAULT_*, from the environment it inherits,
# then sets only its Flavor's values; scripts/test_release.py keeps this list in
# step with the sources.
BUILD_TIME_ENV_PREFIX = "OMOBA_DEFAULT_"
BUILD_TIME_ENVS = CLIENT_DEFAULT_ENVS + ("OMOBA_ACCOUNT_API_URL", "OMOBA_PORTAL_ORIGIN", "EKZA_DEV_HTTP_HOST",
                                         MARKET_ENV, *FEATURE_ENVS.values())

# Contours. Production: the public beta lobby on the production VPS and the Ekza
# registry/Studio. DEV: ops/vps/omoba-lobby-dev.service on the dedicated dev VPS
# (lobby.dev.omoba.io, address only known from $DEV_IP) and registry.dev.ekza.io,
# built as the separate OMOBA Dev app on every platform (phones: space.ekza.omoba.dev).
# The dev contour has no Ekza passport yet (DEV_PASSPORT is a closed loopback port);
# once one exists, pass --passport https://<name>.dev.ekza.io/api/passport and update
# ops/vps/dev/lobby_dev.py.
DEV_PLATFORMS = PLATFORMS
# The dev Linux package also carries the dev lobby's schema migration tool
# (ops/vps/dev/lobby_dev.py runs `omoba-account-api migrate` on the dev VPS).
DEV_LINUX_TOOLS = ("omoba-account-api",)
# A market with its own account service ships the Account API built for it in its
# Linux package: the market's servers run exactly the features its clients have.
MARKET_LINUX_TOOLS = ("omoba-account-api",)
DEV_IP_ENV = "DEV_IP"
PROD_VPS_IPS = frozenset({"77.246.105.57"})
BETA_LOBBY = "77.246.105.57:4000"
PUBLIC_ZONES = ("omoba.io", "ekza.io", "omoba.ru")
DEV_ZONES = ("dev.omoba.io", "dev.ekza.io")
DEV_TAG = re.compile(r"v\d+\.\d+\.\d+-dev\.\d+")
LOOPBACK_HOSTS = frozenset({"127.0.0.1", "localhost", "::1"})


@dataclass(frozen=True)
class Flavor:
    """What one build bakes in: contour, market, initial server, registry and passport,
    account service and portal, settings folder."""
    contour: str = "prod"
    server: str | None = None
    registry: str | None = None
    config_suffix: str | None = None
    passport: str | None = None
    market: str = "global"
    features: tuple[tuple[str, bool], ...] = ()
    """--feature overrides of the market preset, sorted."""
    account_api: str | None = None
    portal: str | None = None

    @property
    def dev(self) -> bool:
        return self.contour == "dev"

    @property
    def label(self) -> str:
        """Inserted after the version in package and dist directory names: -ru, -dev, -ru-dev."""
        return MARKETS[self.market].label + ("-dev" if self.dev else "")

    @property
    def app_name(self) -> str:
        return "Omoba Dev" if self.dev else "Omoba"

    @property
    def bundle_id(self) -> str:
        market = "" if self.market == "global" else f".{self.market}"
        return MAC_BUNDLE_ID + market + (".dev" if self.dev else "")

    @property
    def config_dir(self) -> str:
        """Mirrors client/src/platform.rs config_directory_name."""
        return market_presets.config_dir(self.market, self.config_suffix)

    @property
    def resolved_features(self) -> dict[str, bool]:
        return market_presets.resolve_features(self.market, dict(self.features))

    def client_defaults(self) -> dict[str, str]:
        values = {SERVER_DEFAULT_ENV: self.server, REGISTRY_DEFAULT_ENV: self.registry,
                  PASSPORT_DEFAULT_ENV: self.passport, CONFIG_SUFFIX_ENV: self.config_suffix,
                  ACCOUNT_API_ENV: self.account_api, PORTAL_ORIGIN_ENV: self.portal}
        if self.dev:
            values.update({ACCOUNT_API_ENV: self.account_api or DEV_ACCOUNT_API,
                           PORTAL_ORIGIN_ENV: self.portal or DEV_PORTAL_ORIGIN})
        values.update(market_presets.market_env(self.market, dict(self.features)))
        return {key: value for key, value in values.items() if value}


PROD = Flavor()


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


def artifact_name(version: str, target: str, contour: str = "prod", market: str = "global") -> str:
    """Package file name: the version, then `-<market>` (not for global) and `-dev`."""
    if contour not in CONTOURS:
        raise SystemExit(f"unknown contour {contour!r}")
    label = version + market_presets.market(market).label + ("-dev" if contour == "dev" else "")
    return {
        "macos": f"Omoba-{label}-macos-arm64.zip",
        "windows": f"Omoba-{label}-windows-x64.zip",
        "linux": f"omoba-{label}-linux-x64.tar.gz",
        "android": f"Omoba-{label}-android-{'universal' if android_universal() else 'arm64'}.apk",
        "ios": f"Omoba-{label}-ios.ipa",
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


def is_build_time_env(key: str) -> bool:
    return key in BUILD_TIME_ENVS or key.startswith(BUILD_TIME_ENV_PREFIX)


def scrub_build_env(env: dict) -> dict:
    """Drop every compile-time setting the invoking shell may have exported."""
    for key in [key for key in env if is_build_time_env(key)]:
        del env[key]
    return env


def apply_client_defaults(env: dict, flavor: Flavor) -> dict:
    """Set exactly this build's compiled-in defaults; drop any the shell exported."""
    scrub_build_env(env)
    env.update(flavor.client_defaults())
    return env


def client_env(flavor: Flavor) -> dict:
    return apply_client_defaults(os.environ.copy(), flavor)


def validate_server(server: str | None) -> str | None:
    if server is None:
        return None
    if not re.fullmatch(r"[A-Za-z0-9.\-]+:\d{1,5}", server) or not 0 < int(server.rsplit(":", 1)[1]) < 65536:
        raise SystemExit(f"--server must be host:port, got {server!r}")
    return server


def validate_registry(registry: str | None) -> str | None:
    """Same rule as the SDK's feed_url: https anywhere, http only on loopback, no credentials."""
    if registry is None:
        return None
    parts = urlsplit(registry)
    try:
        parts.port
    except ValueError:
        raise SystemExit(f"--registry has an invalid port: {registry!r}") from None
    if (parts.scheme not in ("https", "http") or not parts.hostname or parts.username is not None
            or parts.password is not None or parts.query or parts.fragment
            or any(ch.isspace() or ch in "\"'\\" for ch in registry)):
        raise SystemExit(f"--registry must be an https:// origin without credentials, got {registry!r}")
    if parts.scheme == "http" and parts.hostname not in LOOPBACK_HOSTS:
        raise SystemExit(f"--registry may use http:// only on loopback, got {registry!r}")
    return registry.rstrip("/")


def validate_passport(passport: str | None) -> str | None:
    """The SDK passport rule: an https:// (or loopback http://) URL ending in /api/passport."""
    if passport is None:
        return None
    parts = urlsplit(passport)
    try:
        parts.port
    except ValueError:
        raise SystemExit(f"--passport has an invalid port: {passport!r}") from None
    if (parts.scheme not in ("https", "http") or not parts.hostname or parts.username is not None
            or parts.password is not None or parts.query or parts.fragment
            or not parts.path.rstrip("/").endswith("/api/passport")
            or any(ch.isspace() or ch in "\"'\\" for ch in passport)):
        raise SystemExit(f"--passport must be an https:// URL ending in /api/passport without "
                         f"credentials, got {passport!r}")
    if parts.scheme == "http" and parts.hostname not in LOOPBACK_HOSTS:
        raise SystemExit(f"--passport may use http:// only on loopback, got {passport!r}")
    return passport.rstrip("/")


def endpoint_host(server: str | None) -> str | None:
    """The host of host:port, with case and a trailing dot normalised away."""
    if not server:
        return None
    return server.rpartition(":")[0].lower().rstrip(".")


def registry_host(registry: str | None) -> str | None:
    host = urlsplit(registry).hostname if registry else None
    return host.lower().rstrip(".") if host else None


def in_zone(host: str | None, zones) -> bool:
    return bool(host) and any(host == zone or host.endswith("." + zone) for zone in zones)


def is_dev_host(host: str | None) -> bool:
    """A dev-contour name (*.dev.omoba.io, *.dev.ekza.io) or the dev VPS address in $DEV_IP."""
    dev_ip = os.environ.get(DEV_IP_ENV, "").strip()
    return in_zone(host, DEV_ZONES) or bool(host and dev_ip and host == dev_ip)


def is_prod_host(host: str | None) -> bool:
    """The production VPS, or any public omoba.io / ekza.io / omoba.ru (RU) name outside the dev zones."""
    return bool(host) and (host in PROD_VPS_IPS or (in_zone(host, PUBLIC_ZONES) and not in_zone(host, DEV_ZONES)))


def validate_origin(name: str, origin: str | None) -> str | None:
    """--account-api / --portal: an exact https:// origin (http:// only on loopback), no path."""
    if origin is None:
        return None
    parts = urlsplit(origin)
    try:
        parts.port
    except ValueError:
        raise SystemExit(f"--{name} has an invalid port: {origin!r}") from None
    if (parts.scheme not in ("https", "http") or not parts.hostname or parts.username is not None
            or parts.password is not None or parts.query or parts.fragment or parts.path not in ("", "/")
            or origin.endswith("/") or any(ch.isspace() or ch in "\"'\\" for ch in origin)):
        raise SystemExit(f"--{name} must be an exact https:// origin without a path, got {origin!r}")
    if parts.scheme == "http" and parts.hostname not in LOOPBACK_HOSTS:
        raise SystemExit(f"--{name} may use http:// only on loopback, got {origin!r}")
    return origin


def other_market_zone(market: str, host: str | None) -> str | None:
    """The market whose own servers `host` belongs to, if it is not `market`."""
    for key, preset in MARKETS.items():
        if key != market and in_zone(host, preset.zones):
            return key
    return None


def resolve_flavor(contour: str = "prod", server: str | None = None, registry: str | None = None,
                   passport: str | None = None, market: str = "global",
                   features: dict[str, bool] | None = None, account_api: str | None = None,
                   portal: str | None = None) -> Flavor:
    """Validate a build's endpoints and refuse to cross contours or markets."""
    if contour not in CONTOURS:
        raise SystemExit(f"--contour must be one of {', '.join(CONTOURS)}, got {contour!r}")
    preset = market_presets.market(market)
    if preset.blocked:
        raise SystemExit(f"cannot package --market {market}: {preset.blocked}")
    resolved = market_presets.resolve_features(market, features)
    server = validate_server(server)
    registry = validate_registry(registry)
    passport = validate_passport(passport)
    account_api = validate_origin("account-api", account_api)
    portal = validate_origin("portal", portal)
    if not resolved["ekza"] and (registry or passport):
        raise SystemExit("--registry/--passport need the ekza feature; this market builds without Ekza")
    for name, value in (("--server", endpoint_host(server)), ("--account-api", registry_host(account_api)),
                        ("--portal", registry_host(portal))):
        if owner := other_market_zone(market, value):
            raise SystemExit(f"{name} {value} is a {owner} market server; build with --market {owner}")
    if (account_api is None) != (portal is None):
        raise SystemExit("--account-api and --portal go together (the portal trusts exactly one service)")
    if contour == "dev" and account_api:
        raise SystemExit("OMOBA Dev always uses the DEV Account API and portal (passport/src/web_account.rs)")
    if market != "global":
        if server and server == BETA_LOBBY:
            raise SystemExit(f"{server} is the global public lobby; a {market} build uses its market's lobby")
        overrides = tuple(sorted((features or {}).items()))
        if contour == "dev":
            flavor = resolve_flavor("dev", server, registry, passport)
            return Flavor("dev", flavor.server, flavor.registry if resolved["ekza"] else None,
                          DEV_CONFIG_SUFFIX, flavor.passport if resolved["ekza"] else None, market,
                          overrides, account_api, portal)
        return Flavor("prod", server or preset.lobby, registry, None, passport, market, overrides,
                      account_api or preset.account_api, portal or preset.portal)
    if features:
        flavor = resolve_flavor(contour, server, registry, passport)
        ekza = resolved["ekza"]
        return Flavor(flavor.contour, flavor.server, flavor.registry if ekza else None,
                      flavor.config_suffix, flavor.passport if ekza else None, "global",
                      tuple(sorted(features.items())), account_api, portal)
    if account_api:
        flavor = resolve_flavor(contour, server, registry, passport)
        return Flavor(flavor.contour, flavor.server, flavor.registry, flavor.config_suffix,
                      flavor.passport, "global", (), account_api, portal)
    if contour == "dev":
        server = server or DEV_LOBBY
        registry = registry or DEV_REGISTRY
        passport = passport or DEV_PASSPORT
        if is_prod_host(endpoint_host(server)):
            raise SystemExit(f"a dev build must not point at production: {server} "
                             "(the dev lobby runs on the dedicated dev VPS)")
        if is_prod_host(registry_host(registry)):
            raise SystemExit(f"a dev build must not use a production registry: {registry}")
        # Pairing and ticket consumption change passport state: only a dev or loopback passport.
        host = registry_host(passport)
        if not (is_dev_host(host) or host in LOOPBACK_HOSTS):
            raise SystemExit(f"a dev build must not use a non-dev Ekza passport: {passport} "
                             f"(default {DEV_PASSPORT} fails closed)")
        return Flavor("dev", server, registry, DEV_CONFIG_SUFFIX, passport)
    if is_dev_host(endpoint_host(server)):
        raise SystemExit(f"{server} is the dev lobby; build with --contour dev")
    if is_dev_host(registry_host(registry)):
        raise SystemExit(f"{registry} is the dev registry; build with --contour dev")
    if is_dev_host(registry_host(passport)):
        raise SystemExit(f"{passport} is a dev passport; build with --contour dev")
    return Flavor("prod", server, registry, None, passport)


def player_readme(version: str, target: str, flavor: Flavor = PROD) -> str:
    server = flavor.server
    server_line = (f"This build connects to {server} by default." if server else
                   "This build plays on the public OMOBA lobby.")
    title = f"OMOBA {version} — DEV test build" if flavor.dev else f"OMOBA {version} — playtest build"
    passport_notice = ("Ekza wallet pairing and purchased avatars are off in dev builds (there is\n"
                       "no dev passport yet); free and built-in avatars work.\n"
                       if flavor.passport == DEV_PASSPORT else "")
    tools_notice = ("omoba-account-api is the dev lobby's database migration tool; players do\n"
                    "not need it.\n" if target == "linux" else "")
    dev_notice = f"""
DEV TEST BUILD
This is a development test build, not the public beta. It uses the OMOBA dev
lobby ({flavor.server}) and the Ekza dev registry ({flavor.registry}).
Its window says OMOBA DEV. It keeps its settings, session and downloads in its
own "{flavor.config_dir}" folder, so a normal OMOBA install on this computer is
left untouched. Dev servers and dev accounts may be reset at any time.
{passport_notice}{tools_notice}""" if flavor.dev else ""
    join_own_server = ("""
To play on your own practice server instead: Home -> Connection -> Custom,
type the host's address (host:port, for example 192.168.1.20:4000), press Use
and restart the game. Connection -> Dev switches back.
""" if flavor.dev else """
To play on your own practice server instead (computers only): start the game
with GAME_SERVER_ADDR set to the host's address, for example
GAME_SERVER_ADDR=192.168.1.20:4000. The game has no in-game server picker.
""")
    app = f"{flavor.app_name}.app"
    app_path = f'"/path/to/{app}"' if " " in app else f"/path/to/{app}"
    common = f"""{title}
{'=' * (len(version) + 25)}

{server_line}
{dev_notice}
PLAY WITH A FRIEND
1. Both start the game. Home -> "Party & friends".
2. Invite your friend from "Online on this server"; they press Accept.
3. The leader presses PLAY VS BOTS. Pick heroes. You play on one team and
   bots take the other seats.
{join_own_server}
HOSTING A PRACTICE SERVER
The host's UDP port 4000 must be reachable: same Wi-Fi/LAN, a VPN such as
Tailscale/ZeroTier, or a router port forward for UDP 4000.
"""
    specific = {
        "macos": f"""
MACOS
- Unzip and move {app} to Applications (or anywhere).
- The build is not notarized. First start: right-click {app} -> Open ->
  Open. If macOS still refuses, run in Terminal:
      xattr -dr com.apple.quarantine {app_path}
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
        "android": f"""
ANDROID
- Allow installs from your browser/file manager, open the APK, install.
- {"OMOBA Dev installs beside OMOBA; its Connection button picks Dev, Local or Custom." if flavor.dev else "It plays on the public OMOBA lobby."}
""",
    }.get(target, "")
    return (common + specific + legal_readme(version, flavor)
            + "\nLicenses and attributions: see the legal/ folder and ATTRIBUTION.md.\n")


def legal_readme(version: str, flavor: Flavor = PROD) -> str:
    """The market's age sign, documents, source and channels (LR-06, LR-07, LR-21), from
    markets.py `Legal`; a link the market does not have is left out, a market without any
    (global) adds nothing. A dev build has no public source tag, so it names none."""
    legal = MARKETS[flavor.market].legal
    lines = []
    if legal.age_rating:
        lines.append(f"Возрастная категория / Age rating: {legal.age_rating}")
    for label, value in (("Пользовательское соглашение / Terms of use", legal.terms),
                         ("Политика конфиденциальности / Privacy policy", legal.privacy),
                         ("Исходный код этой версии / Source code of this release",
                          None if flavor.dev else legal.source_for(version)),
                         ("Поддержка / Support", legal.support_email),
                         ("Сообщество ВКонтакте / VK community", legal.community_vk),
                         ("Telegram", legal.community_telegram)):
        if value:
            lines.append(f"{label}: {value}")
    if not lines:
        return ""
    return "\nПРАВОВАЯ ИНФОРМАЦИЯ / LEGAL\n" + "\n".join(lines) + "\n"


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


def stage_common(destination: Path, version: str, target: str, flavor: Flavor, asset_dir: Path):
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
    (destination / "README.txt").write_text(player_readme(version, target, flavor), encoding="utf-8")


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


def mac_icon_source(flavor: Flavor = PROD) -> Path:
    """The phone app icon of the build's contour: the dev app has the DEV ribbon."""
    icon_set = MOBILE[flavor.contour].ios_app_icon
    return ROOT / f"mobile/ios/Assets.xcassets/{icon_set}.appiconset/AppIcon.png"


def mac_icon(resources: Path, flavor: Flavor = PROD):
    source = mac_icon_source(flavor)
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


def mac_info_plist(version: str, flavor: Flavor, icon: str | None) -> dict:
    """A dev app has its own name and bundle id, so it never replaces or updates Omoba.app."""
    info = {
        "CFBundleName": flavor.app_name, "CFBundleDisplayName": "OMOBA Dev" if flavor.dev else "OMOBA",
        "CFBundleIdentifier": flavor.bundle_id, "CFBundleExecutable": "client",
        "CFBundlePackageType": "APPL", "CFBundleShortVersionString": version.split("-")[0],
        "CFBundleVersion": version, "LSMinimumSystemVersion": "12.0",
        "NSHighResolutionCapable": True, "LSApplicationCategoryType": "public.app-category.games",
    }
    if icon:
        info["CFBundleIconFile"] = icon
    return info


def mac_practice_command(flavor: Flavor) -> str:
    app = f"{flavor.app_name}.app"
    return f"""#!/bin/sh
# Practice server with bots for you and your friends (UDP 4000).
DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
export SERVER_ADDR="${{SERVER_ADDR:-0.0.0.0:4000}}"
export OMOBA_MATCH_MODE=practice
export OMOBA_ASSET_DIR="$DIR/{app}/Contents/Resources/assets"
echo "OMOBA practice server on $SERVER_ADDR. Friends connect to this computer's IP, port 4000."
echo "Close this window to stop the server."
exec "$DIR/{app}/Contents/MacOS/server"
"""


def build_macos(version: str, out: Path, flavor: Flavor) -> Path:
    if host_platform() != "macos":
        raise SystemExit("macos packages are built on macOS (or by the GitHub release workflow)")
    binaries = cargo_release(DESKTOP_BINARIES, client_env(flavor))
    with tempfile.TemporaryDirectory() as temporary:
        stage = Path(temporary) / flavor.app_name
        app = stage / f"{flavor.app_name}.app"
        macos_dir = app / "Contents/MacOS"
        resources = app / "Contents/Resources"
        macos_dir.mkdir(parents=True)
        resources.mkdir(parents=True)
        shutil.copy2(binaries["client"], macos_dir / "client")
        shutil.copy2(binaries["server"], macos_dir / "server")
        stage_common(stage, version, "macos", flavor, resources / "assets")
        icon = mac_icon(resources, flavor)
        with (app / "Contents/Info.plist").open("wb") as handle:
            plistlib.dump(mac_info_plist(version, flavor, icon), handle)
        host = stage / "Host Practice Server.command"
        host.write_text(mac_practice_command(flavor))
        host.chmod(0o755)
        # Ad-hoc signature: required on Apple silicon; not a notarized identity.
        run("codesign", "--force", "--deep", "--sign", "-", app)
        archive = out / artifact_name(version, "macos", flavor.contour, flavor.market)
        # ditto keeps the bundle layout and signature intact.
        run("ditto", "-c", "-k", "--keepParent", stage, archive)
        return archive


def stage_windows(stage: Path, version: str, flavor: Flavor, client: Path, server_binary: Path):
    stage.mkdir(parents=True)
    for binary, name in ((client, "Omoba.exe"), (server_binary, "omoba-server.exe")):
        raw = binary.read_bytes()[:4096]
        offset = int.from_bytes(raw[60:64], "little")
        if raw[:2] != b"MZ" or raw[offset:offset + 6] != b"PE\x00\x00\x64\x86":
            raise SystemExit(f"{binary} is not an x64 Windows executable")
        shutil.copy2(binary, stage / name)
    stage_common(stage, version, "windows", flavor, stage / "assets")
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


def build_windows(version: str, out: Path, flavor: Flavor) -> Path:
    host = host_platform()
    if host == "windows":
        binaries = cargo_release(DESKTOP_BINARIES, client_env(flavor))
    elif host == "macos" and (env := xwin_env()):
        # Cross-build with the Microsoft CRT/SDK that cargo-xwin caches; static CRT.
        apply_client_defaults(env, flavor)
        binaries = cargo_release(DESKTOP_BINARIES, env, target="x86_64-pc-windows-msvc",
                                 command=("cargo", "xwin", "build"))
    else:
        raise SystemExit("windows packages need Windows, cargo-xwin on macOS (see `check`), "
                         "or the GitHub release workflow (python3 scripts/release.py ci)")
    with tempfile.TemporaryDirectory() as temporary:
        stage = Path(temporary) / flavor.app_name
        stage_windows(stage, version, flavor, binaries["client"], binaries["server"])
        archive = out / artifact_name(version, "windows", flavor.contour, flavor.market)
        zip_tree(stage, archive, flavor.app_name)
        return archive


def linux_tools(flavor: Flavor) -> tuple[str, ...]:
    """Extra binaries of a Linux package: the dev lobby's migration tool, or a market's Account API."""
    return DEV_LINUX_TOOLS if flavor.dev else MARKET_LINUX_TOOLS if flavor.market != "global" else ()


def linux_binaries(flavor: Flavor) -> tuple[str, ...]:
    """Cargo packages a Linux package is built from."""
    return DESKTOP_BINARIES + linux_tools(flavor)


def stage_linux(stage: Path, version: str, flavor: Flavor, client: Path, server_binary: Path,
                tools: dict[str, Path] | None = None):
    stage.mkdir(parents=True)
    shutil.copy2(client, stage / "omoba")
    shutil.copy2(server_binary, stage / "omoba-server")
    expected = linux_tools(flavor)
    if set(tools or {}) != set(expected):
        raise SystemExit(f"a {flavor.contour} Linux package carries exactly {list(expected)}, "
                         f"got {sorted(tools or {})}")
    for name, binary in (tools or {}).items():
        shutil.copy2(binary, stage / name)
    stage_common(stage, version, "linux", flavor, stage / "assets")
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


def linux_docker_env_args(flavor: Flavor) -> list[str]:
    """Only this build's defaults cross into the container; it never sees the host shell."""
    return [part for key, value in flavor.client_defaults().items() for part in ("-e", f"{key}={value}")]


def build_linux(version: str, out: Path, flavor: Flavor) -> Path:
    archive = out / artifact_name(version, "linux", flavor.contour, flavor.market)
    packages = linux_binaries(flavor)
    if host_platform() == "linux":
        binaries = cargo_release(packages, client_env(flavor))
        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary) / f"omoba-{version}{flavor.label}"
            stage_linux(stage, version, flavor, binaries["client"], binaries["server"],
                        {name: binaries[name] for name in packages if name not in DESKTOP_BINARIES})
            run("tar", "-czf", archive, "-C", stage.parent, stage.name)
        return archive
    if not shutil.which("docker"):
        raise SystemExit("linux packages need a Linux host, Docker, or the GitHub release workflow")
    # Build inside the pinned toolchain image, then package here.
    toolchain = re.search(r'channel\s*=\s*"([^"]+)"', (ROOT / "rust-toolchain.toml").read_text()).group(1)
    common = shared_target().parent
    target_dir = common / "target/linux-docker"
    image = f"rust:{toolchain}-bookworm"
    env_args = linux_docker_env_args(flavor)
    cargo_packages = " ".join(f"-p {name}" for name in packages)
    mounts = ["-v", f"{ROOT}:/src", "-v", f"{target_dir}:/cache", "-w", "/src"]
    # A successful pull also proves the container can reach apt.
    if subprocess.run(["docker", "pull", "--platform", "linux/amd64", image]).returncode == 0:
        script = ("apt-get update -qq && apt-get install -y -qq --no-install-recommends "
                  "libasound2-dev libudev-dev pkg-config >/dev/null && "
                  f"cargo build --locked --release {cargo_packages}")
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
                  f"cargo build --offline --locked --release {cargo_packages}")
        run("docker", "run", "--rm", "--network", "none", "--platform", "linux/amd64", *mounts,
            "-e", "CARGO_TARGET_DIR=/cache", "-e", "CARGO_HOME=/cache/cargo-home",
            "-e", f"PATH=/cache/{installed.name}/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
            *env_args, base, "sh", "-c", script)
    # target/linux-docker is shared by every contour: package it right away. The dev
    # lobby is installed from this package's omoba-server (ops/vps/dev/lobby_dev.py).
    release = target_dir / "release"
    with tempfile.TemporaryDirectory() as temporary:
        stage = Path(temporary) / f"omoba-{version}{flavor.label}"
        stage_linux(stage, version, flavor, release / "client", release / "server",
                    {name: release / name for name in packages if name not in DESKTOP_BINARIES})
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


def mobile_endpoints_check(flavor: Flavor, platform_name: str) -> None:
    """Dev phone builds (and every iOS archive) compile their contour's registry and
    passport (scripts/contours.py; production: the SDK defaults); only --server changes."""
    defaults = (DEV_REGISTRY, DEV_PASSPORT) if flavor.dev else (None, None)
    if (flavor.registry, flavor.passport) != defaults:
        raise SystemExit(f"--registry/--passport are not supported for {platform_name}: the "
                         f"{flavor.contour} app compiles its contour's own (scripts/contours.py)")


def build_android(version: str, out: Path, flavor: Flavor) -> Path:
    """The production app (space.ekza.omoba.beta) or, for --contour dev, the OMOBA Dev
    app (space.ekza.omoba.dev, DEV icon), which installs beside it."""
    if flavor.dev:
        mobile_endpoints_check(flavor, "android")
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
    if flavor.server:
        args += ["--server", flavor.server]
    args += ["--contour", flavor.contour, "--market", flavor.market]
    run(*args, env=client_env(flavor))
    apk = work / f"omoba-{version}{flavor.label}-android-{abi}-debug.apk"  # build.py apk_name
    if not apk.is_file():
        raise SystemExit(f"Android build produced no {apk.name}")
    for leftover in ("omoba-unaligned.apk", "omoba-unsigned.apk"):
        (work / leftover).unlink(missing_ok=True)
    target = out / artifact_name(version, "android", flavor.contour, flavor.market)
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
    """Record the last used number in this checkout only; other contours own theirs."""
    path = ROOT / "mobile/ios/Omoba.local.xcconfig"
    text, count = re.subn(r"(?m)^CURRENT_PROJECT_VERSION\s*=.*$",
                         f"CURRENT_PROJECT_VERSION = {number}", path.read_text())
    if not count:
        text = text.rstrip() + f"\nCURRENT_PROJECT_VERSION = {number}\n"
    path.write_text(text)


def ios_settings(flavor: Flavor, team: str, number: str) -> list[str]:
    """xcodebuild settings of one archive. The contour picks the app (Omoba.xcconfig:
    bundle id, icon; xcode_build.py: name, compiled dev profile); only --server reaches
    the archive's Cargo step, through OMOBA_GAME_SERVER.

    Both are always on the command line, which beats mobile/ios/Omoba.local.xcconfig:
    an `OMOBA_CONTOUR = dev` or a LAN `OMOBA_GAME_SERVER` left there for Xcode runs
    never reaches a release archive (a production upload stays the production app and
    keeps the public lobby; it has no server selection to recover from a LAN one)."""
    return [f"DEVELOPMENT_TEAM={team}", f"CURRENT_PROJECT_VERSION={number}",
            "OMOBA_CARGO_PROFILE=release", "CODE_SIGN_STYLE=Automatic",
            f"OMOBA_CONTOUR={flavor.contour}", f"OMOBA_GAME_SERVER={flavor.server or ''}"]


def archive_bundle_id(archive: Path) -> str | None:
    """CFBundleIdentifier of the app in an .xcarchive: the archive's own Info.plist
    (ApplicationProperties), else the archived app's Info.plist."""
    summary = archive / "Info.plist"
    if summary.is_file():
        properties = plistlib.loads(summary.read_bytes()).get("ApplicationProperties") or {}
        if properties.get("CFBundleIdentifier"):
            return properties["CFBundleIdentifier"]
    for info in sorted((archive / "Products/Applications").glob("*.app/Info.plist")):
        return plistlib.loads(info.read_bytes()).get("CFBundleIdentifier")
    return None


def check_archive_identity(archive: Path, flavor: Flavor) -> None:
    """Refuses an archive that is not the app the flavor names (the production app for
    prod, OMOBA Dev for --contour dev) before it is kept, exported or uploaded."""
    expected = MOBILE[flavor.contour].bundle_id
    found = archive_bundle_id(archive)
    if found != expected:
        raise SystemExit(f"The {flavor.contour} iOS archive is {found or 'missing its bundle id'}, "
                         f"expected {expected}; check mobile/ios/Omoba.local.xcconfig")


def ios_scheme(contour: str) -> tuple[str, str]:
    if contour not in CONTOURS:
        raise ValueError(f"unknown contour {contour!r}")
    return ("Omoba Dev", "Release Dev") if contour == "dev" else ("Omoba", "Release")


def build_ios(version: str, out: Path, flavor: Flavor, build_number: str | None, upload: bool) -> Path:
    """The production app or, for --contour dev, the OMOBA Dev app (space.ekza.omoba.dev,
    its own App Store Connect record for internal TestFlight)."""
    mobile_endpoints_check(flavor, "ios")
    try:
        source = require_production_source(ROOT) if not flavor.dev else None
    except ValueError as error:
        raise SystemExit(str(error)) from error
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
    settings = ios_settings(flavor, team, number)
    # The archive's Cargo step (mobile/ios/xcode_build.py) inherits this environment: no
    # compile-time default from the shell, so the server comes only from OMOBA_GAME_SERVER.
    env = scrub_build_env(os.environ.copy())
    scheme, configuration = ios_scheme(flavor.contour)
    run("xcodebuild", "-project", ROOT / "mobile/ios/Omoba.xcodeproj", "-scheme", scheme,
        "-configuration", configuration, "-destination", "generic/platform=iOS",
        "-archivePath", archive, "-allowProvisioningUpdates", "archive", *settings, env=env)
    check_archive_identity(archive, flavor)
    if source is not None:
        try:
            if source != require_production_source(ROOT):
                raise ValueError('Production source changed during archive; build again.')
        except ValueError as error:
            raise SystemExit(str(error)) from error
    options = work / "ExportOptions.plist"
    with options.open("wb") as handle:
        plistlib.dump({"method": "app-store-connect", "teamID": team, "signingStyle": "automatic",
                       "destination": "upload" if upload else "export",
                       "manageAppVersionAndBuildNumber": False,
                       "uploadSymbols": True}, handle)
    export = work / "export"
    kept = out / f"Omoba-{version}{flavor.label}-ios-{number}.xcarchive"
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
        listed = organizer / f"{flavor.app_name} {version} ({number}).xcarchive"
        shutil.rmtree(listed, ignore_errors=True)
        run("ditto", archive, listed)
        subprocess.run(["open", "-a", "Xcode", str(listed)])
        # This number is now taken by a signed archive; the next run uses a fresh one.
        remember_ios_build(number)
        print(f"!! iOS export/upload was refused; signed archive {kept.name} is in Xcode Organizer: "
              "Distribute App → App Store Connect → Upload", file=sys.stderr)
        return kept
    if upload:
        print(f"Uploaded {flavor.app_name} {version} ({number}) to App Store Connect "
              f"({MOBILE[flavor.contour].bundle_id}); assign it to testers in TestFlight.")
        remember_ios_build(number)
    ipas = list(export.glob("*.ipa"))
    target = out / artifact_name(version, "ios", flavor.contour, flavor.market)
    if ipas:
        shutil.copy2(ipas[0], target)
        return target
    return archive


# ---------------------------------------------------------------- commands

def dist_path(version: str, base: Path | None, contour: str = "prod", market: str = "global") -> Path:
    """dist/v<version>[-<market>][-dev]/: one folder per market and contour."""
    if contour not in CONTOURS:
        raise SystemExit(f"unknown contour {contour!r}")
    label = market_presets.market(market).label + ("-dev" if contour == "dev" else "")
    return (base or ROOT / "dist") / (tag_for(version) + label)


def dist_dir(version: str, base: Path | None, contour: str = "prod", market: str = "global") -> Path:
    directory = dist_path(version, base, contour, market)
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


def build_targets(platform_arg: str, flavor: Flavor, buildable: dict) -> list[str]:
    """Platforms one `build` invocation makes. Every platform has a dev app with its own
    identity (desktop -dev packages, phones space.ekza.omoba.dev); a market packages
    only its own platforms (scripts/markets.py)."""
    allowed = set(MARKETS[flavor.market].platforms) & (set(DEV_PLATFORMS) if flavor.dev else set(PLATFORMS))
    if platform_arg != "all":
        if flavor.dev and platform_arg not in DEV_PLATFORMS:
            raise SystemExit(f"no dev-contour app for {platform_arg}")
        if platform_arg not in allowed:
            raise SystemExit(f"the {flavor.market} market is not packaged for {platform_arg}")
        return [platform_arg]
    return [p for p in PLATFORMS if buildable[p] and p in allowed]


def build_plan(version: str, flavor: Flavor, targets: list[str], out: Path) -> dict:
    return {"version": version, "contour": flavor.contour, "market": flavor.market,
            "features": flavor.resolved_features, "server": flavor.server,
            "registry": flavor.registry, "passport": flavor.passport, "account_api": flavor.account_api,
            "portal": flavor.portal, "config_dir": flavor.config_dir,
            "compiled_defaults": flavor.client_defaults(),
            "linux_binaries": list(linux_binaries(flavor)) if "linux" in targets else [], "dist": str(out),
            "packages": [str(out / artifact_name(version, target, flavor.contour, flavor.market))
                         for target in targets]}


def command_build(args) -> int:
    version = workspace_version()
    flavor = resolve_flavor(args.contour, args.server, args.registry, args.passport, args.market,
                            market_presets.parse_feature_overrides(args.feature), args.account_api,
                            args.portal)
    report = check_report()
    targets = build_targets(args.platform, flavor, report["platforms"])
    if args.dry_run:
        print(json.dumps({"dry_run": True, **build_plan(version, flavor, targets,
                                                        dist_path(version, args.out, flavor.contour,
                                                                  flavor.market))}, indent=2))
        return 0
    out = dist_dir(version, args.out, flavor.contour, flavor.market)
    minimum = float(os.environ.get("OMOBA_MIN_FREE_GIB", "20"))
    if args.platform == "all" and report["free_gib"] < minimum:
        raise SystemExit(f"only {report['free_gib']} GiB free; a full release needs ~{minimum:g} GiB "
                         "(set OMOBA_MIN_FREE_GIB to override)")
    write_compatibility_manifest(out, version)
    if args.platform == "all":
        # TestFlight upload never happens implicitly.
        print("Building on this host: " + ", ".join(targets))
    built, failed = [], {}
    for target in targets:
        try:
            if target == "macos":
                built.append(build_macos(version, out, flavor))
            elif target == "windows":
                built.append(build_windows(version, out, flavor))
            elif target == "linux":
                built.append(build_linux(version, out, flavor))
            elif target == "android":
                built.append(build_android(version, out, flavor))
            elif target == "ios":
                built.append(build_ios(version, out, flavor, args.ios_build, args.upload))
        except (SystemExit, subprocess.CalledProcessError) as error:
            failed[target] = str(error)
            print(f"!! {target} failed: {error}", file=sys.stderr)
            if args.platform != "all":
                raise
    write_checksums(out)
    summary = {"version": version, "contour": flavor.contour, "server": flavor.server,
               "registry": flavor.registry, "passport": flavor.passport, "config_dir": flavor.config_dir,
               "dist": str(out),
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


# ---------------------------------------------------------------- download host (LR-23)

# A market with `downloads_path` (scripts/markets.py) serves its packages from its own
# host: `publish` copies them with rsync into <target>/<tag>/ and reads every one back
# from <downloads>/<tag>/<file>; IPFS stays the mirror. The RU host serves that folder
# with Caddy (ops/vps/ru/README.md#downloads).
DOWNLOADS_TARGET_ENV = "OMOBA_DOWNLOADS_RSYNC_TARGET"
RSYNC_TARGET = re.compile(r"(?:[a-z_][a-z0-9_-]*@)?(?P<host>[A-Za-z0-9][A-Za-z0-9._-]*):(?P<path>/[A-Za-z0-9._/-]*)")
RSYNC_SSH = "ssh -o BatchMode=yes -o ConnectTimeout=10"
MAX_DOWNLOAD_BYTES = 4 << 30


def download_url(market: str, version: str, name: str) -> str | None:
    """Where the market's download host serves one package: <downloads>/<tag>/<name>."""
    base = market_presets.market(market).downloads
    return f"{base}/{tag_for(version)}/{name}" if base else None


def ssh_hostname(alias: str) -> str | None:
    """`ssh -G`: the address an ssh alias connects to, from local config only (no network)."""
    try:
        result = subprocess.run(["ssh", "-G", alias], capture_output=True, text=True, timeout=10)
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return None
    for line in result.stdout.splitlines() if result.returncode == 0 else ():
        key, _, value = line.partition(" ")
        if key.lower() == "hostname":
            return value.strip().lower().rstrip(".")
    return None


def rsync_target(value: str | None, market: str, resolve=None) -> str:
    """--rsync-target: `[user@]host:/absolute/folder` of the market's download host. Never the
    production or dev VPS, a dev name or another market's or the global servers' name, by
    itself or through the ssh alias."""
    if not value:
        raise SystemExit(f"the {market} market publishes to its download host: pass --rsync-target "
                         f"[user@]host:/folder or set {DOWNLOADS_TARGET_ENV}")
    match = RSYNC_TARGET.fullmatch(value)
    if not match or ".." in match["path"].split("/") or match["path"].rstrip("/") == "":
        raise SystemExit(f"--rsync-target must be [user@]host:/absolute/folder, got {value!r}")
    host = match["host"].lower().rstrip(".")
    own = MARKETS[market].zones
    for name in filter(None, (host, (resolve or ssh_hostname)(host))):
        if name in PROD_VPS_IPS or is_dev_host(name) or other_market_zone(market, name) or \
                (in_zone(name, PUBLIC_ZONES) and not in_zone(name, own)):
            raise SystemExit(f"--rsync-target {value} reaches {name}, not the {market} download host")
    return value.rstrip("/")


def rsync_command(staging: Path, target: str) -> list[str]:
    """Copy the staged <tag>/ folder (0755, packages 0644: Caddy reads them) into the target with
    its modes. Files already there are never replaced (a published package does not change); the
    HTTPS read-back then proves the bytes. Only flags macOS's openrsync also knows."""
    return ["rsync", "-r", "-t", "-p", "--ignore-existing", "-e", RSYNC_SSH, str(staging), target + "/"]


def https_digest(url: str, limit: int = MAX_DOWNLOAD_BYTES) -> tuple[str, int]:
    """(sha256, bytes) of what an HTTPS URL serves, streamed."""
    import urllib.request
    if not url.startswith("https://"):
        raise SystemExit(f"{url} is not an https:// download")
    digest, size = hashlib.sha256(), 0
    with urllib.request.urlopen(urllib.request.Request(url, method="GET"), timeout=60) as response:
        while chunk := response.read(1 << 20):
            size += len(chunk)
            if size > limit:
                raise SystemExit(f"{url} is larger than {limit} bytes")
            digest.update(chunk)
    return digest.hexdigest(), size


def publish_to_host(out: Path, packages: list[Path], version: str, market: str, target: str,
                    run_command=None, digest=https_digest) -> dict:
    """Upload the packages to the market's download host, read each back over HTTPS and record
    the verified links in downloads.json. Packages already verified with the same hash are skipped."""
    receipt_path = out / "downloads.json"
    receipt = json.loads(receipt_path.read_text()) if receipt_path.is_file() else {}
    pending = {}
    for path in packages:
        target_name = next(t for t in PLATFORMS if artifact_name(version, t, "prod", market) == path.name)
        entry = receipt.get(target_name)
        if not (entry and entry.get("sha256") == sha256(path) and entry.get("file") == path.name):
            pending[target_name] = path
    if not pending:
        return receipt
    staging = out / ".downloads" / tag_for(version)
    shutil.rmtree(staging.parent, ignore_errors=True)
    staging.mkdir(parents=True)
    staging.chmod(0o755)
    try:
        for path in pending.values():
            try:
                os.link(path, staging / path.name)
            except OSError:
                shutil.copy2(path, staging / path.name)
            (staging / path.name).chmod(0o644)   # world-readable, also the dist file it links
        (run_command or (lambda command: run(*command)))(rsync_command(staging, target))
    finally:
        shutil.rmtree(staging.parent, ignore_errors=True)
    for target_name, path in pending.items():
        url = download_url(market, version, path.name)
        expected = sha256(path)
        served, size = digest(url)
        if served != expected or size != path.stat().st_size:
            raise SystemExit(f"{url} does not serve the local {path.name}: a published package never changes; "
                             "build a new version (or remove the stale file on the host by hand)")
        receipt[target_name] = {"file": path.name, "bytes": path.stat().st_size, "sha256": expected, "url": url}
        receipt_path.write_text(json.dumps(receipt, indent=2) + "\n")
    return receipt


def command_publish(args) -> int:
    """Upload every built package to the market's download host (when it has one) and pin it to
    IPFS, re-download each copy and record the verified links."""
    version = workspace_version()
    contour, market = args.contour, getattr(args, "market", "global")
    host = market_presets.market(market).downloads if contour == "prod" else None
    out = dist_path(version, args.out, contour, market)
    packages = [out / artifact_name(version, target, contour, market) for target in PLATFORMS
                if (out / artifact_name(version, target, contour, market)).is_file()]
    destination = getattr(args, "rsync_target", None) or os.environ.get(DOWNLOADS_TARGET_ENV)
    if host:
        destination = rsync_target(destination, market)
        # iOS is never a direct download (App Store / TestFlight only).
        packages = [path for path in packages if not path.name.endswith(".ipa")]
    if args.dry_run:
        plan = {"dry_run": True, "contour": contour, "market": market, "dist": str(out),
                "would_pin": [str(path) for path in packages]}
        if host:
            plan["download_host"] = {"rsync": rsync_command(out / ".downloads" / tag_for(version), destination),
                                     "would_verify": [download_url(market, version, path.name) for path in packages]}
        print(json.dumps(plan, indent=2))
        return 0
    token = os.environ.get("PINATA_JWT")
    if not token:
        raise SystemExit("Set PINATA_JWT to pin packages")
    out = dist_dir(version, args.out, contour, market)
    if host:
        print(json.dumps(publish_to_host(out, packages, version, market, destination), indent=2))
    receipt_path = out / "ipfs.json"
    receipt = json.loads(receipt_path.read_text()) if receipt_path.is_file() else {}
    for target in PLATFORMS:
        path = out / artifact_name(version, target, contour, market)
        if not path.is_file() or path not in packages:
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


def command_markets(args) -> int:
    print(json.dumps({key: {"features": preset.features, "locale": preset.locale,
                            "settings_folder": market_presets.config_dir(key, None),
                            "lobby": preset.lobby or BETA_LOBBY, "account_api": preset.account_api,
                            "portal": preset.portal, "platforms": list(preset.platforms),
                            "legal": {name: value for name, value in vars(preset.legal).items() if value},
                            "blocked": preset.blocked} for key, preset in MARKETS.items()},
                     indent=2, ensure_ascii=False))
    return 0


# LR-02 (ops/vps/ru/README.md#topology): through the portal, Account API routes that
# the public ingress gates answer as the API does (no session: 401, downloads: 200);
# on the public Account API name the same routes stay refused (403).
PORTAL_ROUTE_CHECKS = (
    ("portal_account", "portal", "/api/me", 401),
    ("portal_releases", "portal", "/api/releases", 200),
    ("account_api_portal_routes_closed", "account_api", "/v1/me", 403),
)
PORTAL_ROUTE_PROBLEMS = {
    "gated": "a key-gated Account API route answered 403 through the portal: the portal does not reach the "
             "Account API (LR-02; ops/vps/ru/README.md#topology)",
    "open": "the public Account API name passes a portal route; serve only the native routes and health "
            "(ops/vps/ru/Caddyfile.ru)",
}


def preflight_report(market: str, resolve=None, fetch=None) -> dict:
    """Whether a market's lobby name resolves, its Account API / portal answer over HTTPS,
    and the portal reaches the Account API while the public API name refuses portal routes."""
    import socket
    import urllib.request
    preset = market_presets.market(market)
    resolve = resolve or (lambda host: sorted({info[4][0] for info in socket.getaddrinfo(host, None)}))
    def default_fetch(url: str) -> int:
        try:
            with urllib.request.urlopen(urllib.request.Request(url, method="GET"), timeout=8) as response:
                return response.status
        except urllib.error.HTTPError as error:
            return error.code
    fetch = fetch or default_fetch
    checks = {}
    if preset.lobby:
        host = endpoint_host(preset.lobby)
        try:
            checks["lobby_dns"] = {"host": host, "ok": True, "addresses": resolve(host)}
        except OSError as error:
            checks["lobby_dns"] = {"host": host, "ok": False, "error": type(error).__name__}
    for name, url in (("account_api", preset.account_api and preset.account_api + "/health/ready"),
                      ("portal", preset.portal and preset.portal + "/")):
        if not url:
            continue
        try:
            status = fetch(url)
            checks[name] = {"url": url, "ok": 200 <= status < 400, "status": status}
        except OSError as error:
            checks[name] = {"url": url, "ok": False, "error": type(error).__name__}
    if preset.account_api and preset.portal:
        origins = {"portal": preset.portal, "account_api": preset.account_api}
        for name, origin, path, expected in PORTAL_ROUTE_CHECKS:
            url = origins[origin] + path
            try:
                status = fetch(url)
            except OSError as error:
                checks[name] = {"url": url, "ok": False, "expected": expected, "error": type(error).__name__}
                continue
            check = {"url": url, "ok": status == expected, "status": status, "expected": expected}
            if status == 403 and expected != 403:
                check["problem"] = PORTAL_ROUTE_PROBLEMS["gated"]
            elif expected == 403 and status != 403:
                check["problem"] = PORTAL_ROUTE_PROBLEMS["open"]
            checks[name] = check
    return {"market": market, "blocked": preset.blocked,
            "ready": not preset.blocked and bool(checks) and all(c["ok"] for c in checks.values()),
            "checks": checks}


def command_preflight(args) -> int:
    report = preflight_report(args.market)
    print(json.dumps(report, indent=2))
    return 0 if report["ready"] else 1


CATALOG_PLATFORMS = {"macos": ("macos", "arm64"), "windows": ("windows", "x86_64"),
                     "linux": ("linux", "x86_64"), "android": ("android", "universal")}
# The build channel of every package published here: installed from the market's own
# downloads. Store channels come with LR-68; the game reads only its own channel's
# entries (passport/src/release_catalog.rs DIRECT).
CATALOG_CHANNEL = "direct"
RELEASE_LABEL = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
                           r"(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?")
CATALOG_INSTRUCTIONS = {
    "ru": {"macos": "Распакуйте архив и перенесите Omoba.app в «Программы». Первый запуск: правый клик → Открыть.",
           "windows": "Распакуйте архив целиком и запустите Omoba.exe. SmartScreen: «Подробнее» → «Выполнить в любом случае».",
           "linux": "Распакуйте архив и запустите ./omoba.",
           "android": "Разрешите установку из браузера или файлового менеджера и откройте APK."},
}
DEFAULT_INSTRUCTIONS = {"macos": "Unzip and move Omoba.app to Applications; first start: right-click -> Open.",
                        "windows": "Unzip the whole folder and run Omoba.exe.",
                        "linux": "Unpack and run ./omoba.",
                        "android": "Allow installs from your browser or file manager, then open the APK."}


def verified_https(entry: dict, what: str) -> None:
    if not str(entry.get("url", "")).startswith("https://") or \
            not re.fullmatch(r"[0-9a-f]{64}", str(entry.get("sha256", ""))):
        raise SystemExit(f"{what}: receipt is not a verified https download")


def release_order_key(label: str) -> tuple:
    """SemVer precedence of a release label, as the game reads it
    (shared::compatibility::ReleaseVersion): 0.45.0 < 0.46.0-rc.1 < 0.46.0; `+build` ignored."""
    match = RELEASE_LABEL.fullmatch(label.split("+", 1)[0])
    if not match:
        raise ValueError(f"not a release label: {label!r}")
    core = tuple(int(part) for part in match.group(1, 2, 3))
    if match.group(4) is None:
        return core, (1,)
    return core, (0, *((0, int(part), "") if part.isdigit() else (1, 0, part)
                       for part in match.group(4).split(".")))


def release_catalog(receipt: dict, version: str, market: str, published_at: str, mirrors: dict | None = None,
                    min_release: str | None = None) -> dict:
    """The Account API download catalog (OMOBA_RELEASE_CATALOG, account-api/README.md#downloads)
    from `publish` receipts: only packages whose served bytes were verified. A market with a
    download host lists its host's URLs (downloads.json) with the IPFS copy (ipfs.json) as
    `mirror_url`; other markets list the IPFS copies. Every entry names the market's region,
    the `direct` channel and the market's release status.

    When the market has a portal, every entry also carries the portal's download page for its
    platform as `update_url` (the game's update button). `min_release` (optional) is the oldest
    release that may still play online: a game below it shows Update required even while the
    servers accept it, so it needs the same player notice as a server switch (docs/RELEASING.md)."""
    if min_release is not None:
        try:
            too_new = release_order_key(min_release) > release_order_key(version)
        except ValueError as error:
            raise SystemExit(f"--min-release: {error}") from None
        if too_new:
            raise SystemExit(f"--min-release {min_release} is newer than this release {version}")
    preset = market_presets.market(market)
    instructions = CATALOG_INSTRUCTIONS.get(market, DEFAULT_INSTRUCTIONS)
    releases = []
    for target, (platform_name, architecture) in CATALOG_PLATFORMS.items():
        entry = receipt.get(target)
        if not entry:
            continue
        verified_https(entry, target)
        if preset.downloads and entry["url"] != download_url(market, version, entry.get("file", "")):
            raise SystemExit(f"{target}: {entry['url']} is not on the {market} download host {preset.downloads}")
        release = {"platform": platform_name, "architecture": architecture, "version": version,
                   "url": entry["url"], "sha256": entry["sha256"], "size_bytes": str(entry["bytes"]),
                   "published_at": published_at, "instructions": instructions[target],
                   "test_status": preset.release_label, "region": preset.region, "channel": CATALOG_CHANNEL}
        if preset.portal:
            release["update_url"] = f"{preset.portal}/download?platform={platform_name}"
        if min_release is not None:
            release["min_release"] = min_release
        mirror = (mirrors or {}).get(target)
        if mirror:
            verified_https(mirror, f"{target} mirror")
            if mirror["sha256"] != entry["sha256"]:
                raise SystemExit(f"{target}: the IPFS mirror holds other bytes than the download host; "
                                 f"run `publish --market {market}` again")
            release["mirror_url"] = mirror["url"]
        releases.append(release)
    return {"releases": releases}


def public_source_tag(version: str, market: str) -> str | None:
    """The tag whose tree the game's About page and the README link as this release's source
    (markets.py `Legal.source`, LR-06): `v<version>-<market>`; None without a source link."""
    if not market_presets.market(market).legal.source:
        return None
    return release_tag("prod", None, version, market)


def remote_tag_listing(tag: str) -> str:
    """`git ls-remote` of one tag in the public repository (read-only)."""
    return subprocess.run(["git", "ls-remote", "--tags", f"https://github.com/{REPO}.git", f"refs/tags/{tag}"],
                          capture_output=True, text=True, timeout=60).stdout


def require_public_source_tag(version: str, market: str, listing=None) -> str | None:
    """LR-06: the public tag exists before the release is published to players, so the
    source link in the game and the README never points at nothing."""
    tag = public_source_tag(version, market)
    listing = listing or remote_tag_listing
    if tag and f"refs/tags/{tag}" not in listing(tag).split():
        raise SystemExit(f"the public tag {tag} is missing on github.com/{REPO}; the game's About page and "
                         f"the README link its source (LR-06). Publish the release first "
                         f"(`release.py draft --market {market}`, then `gh release edit {tag} --draft=false "
                         f"--repo {REPO}`) and rerun.")
    return tag


def command_catalog(args) -> int:
    version = workspace_version()
    preset = market_presets.market(args.market)
    out = dist_path(version, args.out, "prod", args.market)
    ipfs_path = out / "ipfs.json"
    receipt_path = out / "downloads.json" if preset.downloads else ipfs_path
    if not receipt_path.is_file():
        raise SystemExit(f"{receipt_path} is missing: run `publish --market {args.market}` first")
    require_public_source_tag(version, args.market)
    mirrors = json.loads(ipfs_path.read_text()) if preset.downloads and ipfs_path.is_file() else None
    published = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(receipt_path.stat().st_mtime))
    catalog = release_catalog(json.loads(receipt_path.read_text()), version, args.market, published,
                              mirrors=mirrors, min_release=args.min_release)
    missing = [entry["platform"] for entry in catalog["releases"] if preset.downloads and "mirror_url" not in entry]
    if missing:
        print(f"warning: no IPFS mirror for {', '.join(missing)} (publish pins it)", file=sys.stderr)
    target = out / "releases.json"
    target.write_text(json.dumps(catalog, indent=2, ensure_ascii=False) + "\n")
    print(f"{target}: {len(catalog['releases'])} downloads; install it as the market Account API's "
          "OMOBA_RELEASE_CATALOG (ops/vps/ru/README.md#downloads), then run verify-downloads")
    return 0


def load_catalog(source: str, fetch_bytes=None) -> dict:
    """A download catalog from an https:// URL (the portal's /api/releases) or a local file."""
    if source.startswith("https://"):
        import urllib.request

        def default_fetch(url: str) -> bytes:
            request = urllib.request.Request(url, headers={"Accept": "application/json"})
            with urllib.request.urlopen(request, timeout=15) as response:
                return response.read(131073)
        raw = (fetch_bytes or default_fetch)(source)
    else:
        raw = Path(source).read_bytes()
    if len(raw) > 131072:
        raise SystemExit(f"{source}: the catalog is larger than the Account API accepts")
    catalog = json.loads(raw)
    if not isinstance(catalog, dict) or not isinstance(catalog.get("releases"), list):
        raise SystemExit(f"{source}: not a download catalog")
    return catalog


def downloads_report(market: str, catalog: dict, source: str, digest=https_digest) -> dict:
    """Whether every listed package is on the market's download host with the market's region,
    channel and release status, and whether it (and its mirror) serves exactly its SHA-256 and size."""
    preset = market_presets.market(market)
    entries, platforms = [], set()
    for release in catalog["releases"]:
        problems = []
        url, mirror = str(release.get("url", "")), release.get("mirror_url")
        if preset.downloads and not url.startswith(preset.downloads + "/"):
            problems.append(f"not on the download host {preset.downloads}")
        for field, expected in (("region", preset.region), ("channel", CATALOG_CHANNEL),
                                ("test_status", preset.release_label)):
            if release.get(field) != expected:
                problems.append(f"{field} is {release.get(field)!r}, not {expected!r}")
        for label, link in (("download", url), ("mirror", mirror)):
            if label == "mirror" and not link:
                continue
            try:
                served, size = digest(link)
            except (OSError, SystemExit) as error:
                problems.append(f"{label} {link} failed: {type(error).__name__}")
                continue
            if served != release.get("sha256"):
                problems.append(f"{label} {link} serves sha256 {served}, not the catalog's")
            if str(size) != str(release.get("size_bytes")):
                problems.append(f"{label} {link} serves {size} bytes, not {release.get('size_bytes')}")
        platforms.add(release.get("platform"))
        entries.append({"platform": release.get("platform"), "version": release.get("version"), "url": url,
                        "mirror_url": mirror, "ok": not problems, "problems": problems})
    expected = {CATALOG_PLATFORMS[target][0] for target in preset.platforms if target in CATALOG_PLATFORMS}
    missing = sorted(expected - platforms)
    return {"market": market, "catalog": source, "ready": bool(entries) and not missing and all(
        entry["ok"] for entry in entries), "missing_platforms": missing, "entries": entries}


def command_verify_downloads(args) -> int:
    preset = market_presets.market(args.market)
    if not preset.portal and not args.catalog:
        raise SystemExit(f"the {args.market} market has no portal; pass --catalog <releases.json>")
    source = args.catalog or preset.portal + "/api/releases"
    report = downloads_report(args.market, load_catalog(source), source)
    print(json.dumps(report, indent=2, ensure_ascii=False))
    return 0 if report["ready"] else 1


def command_notes(args) -> int:
    version = workspace_version()
    print(release_notes(version, (ROOT / "CHANGELOG.md").read_text()))
    return 0


def release_tag(contour: str, tag: str | None, version: str, market: str = "global") -> str:
    """Dev packages only ever go to an explicit v<version>-dev.N tag, never a production one;
    a market's packages to its own v<version>-<market> tag."""
    if market != "global":
        label = market_presets.market(market).label
        expected = tag_for(version) + label
        if contour == "dev" or (tag or expected) != expected:
            raise SystemExit(f"{market} market releases go to {expected} (production contour only)")
        return expected
    if contour == "dev":
        if not tag or not DEV_TAG.fullmatch(tag):
            raise SystemExit(f"dev-contour releases need an explicit --tag/--ref like {tag_for(version)}-dev.1, "
                             f"got {tag!r}")
        return tag
    tag = tag or tag_for(version)
    if "-dev" in tag:
        raise SystemExit(f"{tag} is a dev tag; pass --contour dev")
    return tag


def command_draft(args) -> int:
    version = workspace_version()
    contour, market = args.contour, getattr(args, "market", "global")
    tag = release_tag(contour, args.tag, version, market)
    target = args.target or ("dev" if contour == "dev" else "main")
    title = f"OMOBA {version} DEV (test contour)" if contour == "dev" else f"OMOBA {version} (playtest)"
    if market != "global":
        title = f"OMOBA {version} ({market.upper()} market)"
    if args.dry_run:
        out = dist_path(version, args.out, contour, market)
        files = sorted(str(p) for p in out.iterdir() if p.is_file()) if out.is_dir() else []
        print(json.dumps({"dry_run": True, "contour": contour, "market": market, "tag": tag,
                          "target": target, "title": title, "dist": str(out), "files": files}, indent=2))
        return 0
    out = dist_dir(version, args.out, contour, market)
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
            "--title", title, "--notes-file", notes, "--target", target)
    print(f"Draft release {tag} is ready. Review it on GitHub; publish with: "
          f"gh release edit {tag} --draft=false --repo {REPO}")
    return 0


def ci_command(args, version: str) -> list[str]:
    """The `gh workflow run` invocation for release.yml; dev runs build the dev branch by default."""
    if getattr(args, "market", "global") != "global":
        raise SystemExit("release.yml builds the global market only; build market packages on the Mac "
                         f"(python3 scripts/release.py build all --market {args.market})")
    flavor = resolve_flavor(args.contour, args.server, args.registry)
    tag = release_tag(flavor.contour, args.ref, version)
    branch = args.branch or ("dev" if flavor.dev else "main")
    fields = ["-f", f"tag={tag}"]
    if flavor.dev:
        fields += ["-f", "contour=dev", "-f", f"server={flavor.server}", "-f", f"registry={flavor.registry}"]
    else:
        if flavor.server:
            fields += ["-f", f"server={flavor.server}"]
        if flavor.registry:
            fields += ["-f", f"registry={flavor.registry}"]
    return ["gh", "workflow", "run", "release.yml", "--repo", REPO, "--ref", branch, *fields]


def command_ci(args) -> int:
    command = ci_command(args, workspace_version())
    if args.dry_run:
        print(json.dumps({"dry_run": True, "command": command}, indent=2))
        return 0
    run(*command)
    print("Started. Watch with: gh run watch --repo " + REPO)
    return 0


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    contour_help = "prod (default) or dev: dev lobby + dev registry, own settings folder, -dev package names"
    market_help = ("global (default), ru or cn: the market's features, servers, settings folder and "
                   "-<market> package names (scripts/markets.py)")
    sub.add_parser("check", help="what this host can build")
    sub.add_parser("markets", help="print every market's features, servers and platforms")
    preflight = sub.add_parser("preflight", help="check that a market's servers answer (DNS, HTTPS health, "
                                                 "portal to Account API)")
    preflight.add_argument("--market", choices=tuple(MARKETS), default="ru")
    catalog = sub.add_parser("catalog", help="write the Account API download catalog from publish receipts")
    catalog.add_argument("--market", choices=tuple(MARKETS), default="ru")
    catalog.add_argument("--out", type=Path)
    catalog.add_argument("--min-release", help="oldest release that may still play online: older games "
                                               "show Update required (needs a player notice, RELEASING.md)")
    verify = sub.add_parser("verify-downloads", help="download every package a market's catalog lists (and its "
                                                     "mirror) and check host, region, status, SHA-256 and size")
    verify.add_argument("--market", choices=tuple(MARKETS), default="ru")
    verify.add_argument("--catalog", help="releases.json or an https:// URL (default: <portal>/api/releases)")
    build = sub.add_parser("build", help="build platform packages into dist/v<version>/")
    build.add_argument("platform", choices=PLATFORMS + ("all",))
    build.add_argument("--server", help="initial game server host:port compiled into clients")
    build.add_argument("--registry", help="initial Ekza registry origin compiled into clients")
    build.add_argument("--passport", help="Ekza passport URL (…/api/passport) compiled into clients "
                                          f"(dev default: {DEV_PASSPORT}, closed)")
    build.add_argument("--contour", choices=CONTOURS, default="prod", help=contour_help)
    build.add_argument("--market", choices=tuple(MARKETS), default="global", help=market_help)
    build.add_argument("--feature", action="append", metavar="NAME=on|off",
                       help="override one market feature (ekza, supporter, vk_sign_in); repeatable")
    build.add_argument("--account-api", help="Account API origin compiled into clients (default: the market's)")
    build.add_argument("--portal", help="player portal origin compiled into clients (with --account-api)")
    build.add_argument("--out", type=Path, help="dist base directory (default: dist/)")
    build.add_argument("--ios-build", help="iOS build number (default: local xcconfig + 1)")
    build.add_argument("--upload", action="store_true", help="iOS: upload to App Store Connect / TestFlight")
    build.add_argument("--dry-run", action="store_true", help="print what would be built and baked in; build nothing")
    sub.add_parser("notes", help="print release notes for the current version")
    publish = sub.add_parser("publish", help="upload dist/ packages to the market's download host (rsync), pin "
                                             "them to IPFS (Pinata) and verify every served copy")
    publish.add_argument("--out", type=Path)
    publish.add_argument("--gateway", default="https://ekza.mypinata.cloud")
    publish.add_argument("--contour", choices=CONTOURS, default="prod", help=contour_help)
    publish.add_argument("--market", choices=tuple(MARKETS), default="global", help=market_help)
    publish.add_argument("--rsync-target", help="[user@]host:/folder of the market's download host "
                                                f"(default: ${DOWNLOADS_TARGET_ENV}); markets with one only")
    publish.add_argument("--dry-run", action="store_true", help="list the uploads and pins; contact nothing")
    draft = sub.add_parser("draft", help="create or update a DRAFT GitHub release with dist/ files")
    draft.add_argument("--tag", help="release tag (dev contour: required, v<version>-dev.N)")
    draft.add_argument("--target", help="commit/branch the new tag points at (default: main; dev contour: dev)")
    draft.add_argument("--out", type=Path)
    draft.add_argument("--contour", choices=CONTOURS, default="prod", help=contour_help)
    draft.add_argument("--market", choices=tuple(MARKETS), default="global", help=market_help)
    draft.add_argument("--dry-run", action="store_true", help="print the tag, target and files; change nothing")
    ci = sub.add_parser("ci", help="run the GitHub release workflow (all platforms)")
    ci.add_argument("--ref", help="release tag to create/update (default: v<version>; dev contour: required)")
    ci.add_argument("--branch", help="branch whose workflow and code are built (default: main; dev contour: dev)")
    ci.add_argument("--server")
    ci.add_argument("--registry")
    ci.add_argument("--contour", choices=CONTOURS, default="prod", help=contour_help)
    ci.add_argument("--market", choices=tuple(MARKETS), default="global", help=market_help)
    ci.add_argument("--dry-run", action="store_true", help="print the gh command; start nothing")
    args = parser.parse_args(argv)
    if args.command == "check":
        print(json.dumps(check_report(), indent=2))
        return 0
    return {"build": command_build, "notes": command_notes, "draft": command_draft, "ci": command_ci,
            "publish": command_publish, "markets": command_markets,
            "preflight": command_preflight, "catalog": command_catalog,
            "verify-downloads": command_verify_downloads}[args.command](args)


if __name__ == "__main__":
    sys.exit(main())
