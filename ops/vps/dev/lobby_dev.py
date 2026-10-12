#!/usr/bin/env python3
"""Install, update, inspect and roll back the OMOBA DEV lobby on the dedicated dev VPS.

The target is the dev VPS only: ssh alias from --host or $DEV_HOST (default
vds-dev), its public IPv4 from --ip or $DEV_IP. Run on the Mac from a clean
checkout of the code being shipped. Nothing on the host changes without --apply:

    export DEV_HOST=vds-dev DEV_IP=<dev VPS IPv4>
    # 1. dev Linux package (its omoba-server is the lobby binary)
    python3 scripts/release.py build linux --contour dev
    P=dist/v0.44.0-dev/omoba-0.44.0-dev-linux-x64.tar.gz
    # 2. local checks and the plan; no network (default)
    python3 ops/vps/dev/lobby_dev.py ship --package $P
    # 3. read-only preflight on the host (ssh, changes nothing)
    python3 ops/vps/dev/lobby_dev.py ship --package $P --check
    # 4. stage, switch, verify; automatic rollback on failure
    python3 ops/vps/dev/lobby_dev.py ship --package $P --apply
    python3 ops/vps/dev/lobby_dev.py status                       # read-only
    python3 ops/vps/dev/lobby_dev.py rollback --release <name> --apply

Guards. On the Mac, before any ssh: the alias must not be a production alias or
address, and `ssh -G <alias>` must resolve to HostName == $DEV_IP (an IPv4
literal in ~/.ssh/config, so a proxy's fake-IP DNS cannot redirect it); the
unit must be exactly the dev lobby. On the host (the same file, piped to
`ssh <alias> python3 - remote <mode> <payload>`): it refuses to run where any
production marker exists (unit omoba-lobby, user omoba, /opt/omoba,
/var/lib/omoba, /opt/ekza-*, the NPM edge, address 77.246.105.57), only writes
under /opt/omoba-dev, /var/lib/omoba-dev and its own unit file, never
interrupts a running dev match, and rolls back to the previous release on any
failed start. Re-shipping the active release with the same unit is a no-op.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import ipaddress
import json
import os
import posixpath
import re
import socket
import subprocess
import sys
import tarfile
import tempfile
import time
from pathlib import Path

# ---------------------------------------------------------------- the dev contour

DEFAULT_HOST = "vds-dev"
HOST_ENV = "DEV_HOST"
IP_ENV = "DEV_IP"
PUBLIC_HOST_ENV = "DEV_LOBBY_PUBLIC_HOST"
UNIT = "omoba-lobby-dev"
USER = "omoba-dev"
OPT = "/opt/omoba-dev"
RELEASES = OPT + "/releases"
CURRENT = OPT + "/current"
LOCK = "/run/lock/omoba-lobby-dev.lock"  # tmpfs; taken before any dev directory exists
HISTORY = OPT + "/deployments.jsonl"
STATE = "/var/lib/omoba-dev"
STATE_DIRS = (STATE, STATE + "/matches", STATE + "/lobby-outbox")
UNIT_PATH = "/etc/systemd/system/omoba-lobby-dev.service"
LOBBY_PORT = 4100
MATCH_PORTS = frozenset({41100})
REGISTRY = "https://registry.dev.ekza.io"
DEFAULT_PUBLIC_HOST = "lobby.dev.omoba.io"
PUBLIC_HOST_KEY = "OMOBA_MATCH_PUBLIC_HOST"
MIN_AVAILABLE_MIB = 600  # MemoryMax=512M plus headroom for the rest of the dev host
DEV_HOSTNAME = re.compile(r"(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+dev\.(?:omoba|ekza)\.io")

# ---------------------------------------------------------------- production (never a target)

PROD_SSH_ALIASES = frozenset({"vds-eternal"})
PROD_IPS = frozenset({"77.246.105.57"})
PROD_HOSTNAMES = frozenset({"registry.ekza.io", "studio.ekza.io", "app.omoba.io", "omoba.io",
                            "ekza.io", "wss.ekza.io", "www.omoba.io", "www.ekza.io"})
PROD_UNIT = "omoba-lobby"
PROD_USER = "omoba"
PROD_UNIT_PATH = "/etc/systemd/system/omoba-lobby.service"
PROD_PATHS = ("/opt/omoba", "/var/lib/omoba", "/opt/ekza-studio", "/opt/ekza-mirror",
              "/root/docker/nginx-proxy-manager", PROD_UNIT_PATH)
PROD_PORTS = frozenset({4000, 41000, 41001})
# Forbidden-value guard only: the production Supabase project must never appear here.
FORBIDDEN_TOKENS = ("snrjxwutqxokeujuiepn",)

RELEASE_NAME = re.compile(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?-dev-[0-9a-f]{7,40}")
DEV_PACKAGE = re.compile(r"omoba-(\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?)-dev-linux-x64\.tar\.gz")
PAYLOAD = re.compile(r"[A-Za-z0-9+/=]*")
SSH_ALIAS = re.compile(r"(?:[a-z_][a-z0-9_-]*@)?[A-Za-z0-9][A-Za-z0-9._-]*")

# Exact values the dev unit must carry; OMOBA_MATCH_PUBLIC_HOST is checked separately.
UNIT_SERVICE = {
    "User": USER, "Group": USER, "WorkingDirectory": STATE,
    "ExecStart": CURRENT + "/server", "ReadWritePaths": STATE,
    "MemoryHigh": "448M", "MemoryMax": "512M", "CPUQuota": "100%", "OOMScoreAdjust": "300",
    "TasksMax": "96", "UMask": "0077",
    "NoNewPrivileges": "true", "PrivateTmp": "true", "ProtectSystem": "strict", "ProtectHome": "true",
}
UNIT_ENV = {
    "SERVER_ADDR": f"0.0.0.0:{LOBBY_PORT}", "OMOBA_SERVER_ROLE": "lobby",
    "OMOBA_MATCH_ROOT": STATE + "/matches", "OMOBA_CAREER_OUTBOX": STATE + "/lobby-outbox",
    "OMOBA_MATCH_FIRST_PORT": str(min(MATCH_PORTS)), "OMOBA_MATCH_CAPACITY": str(len(MATCH_PORTS)),
    "OMOBA_MATCH_EXECUTABLE": CURRENT + "/server", "OMOBA_REGISTRY_URL": REGISTRY,
}
UNIT_FORBIDDEN_KEYS = ("EnvironmentFile",)
UNIT_FORBIDDEN_ENV = ("OMOBA_DATABASE_URL",)


class Refused(SystemExit):
    """A guard said no. Nothing after the guard ran."""

    def __init__(self, message: str):
        super().__init__(f"REFUSED: {message}")
        self.message = message


def within(path: str, base: str) -> bool:
    """Component-wise: /opt/omoba-dev is NOT within /opt/omoba; /opt/omoba/x/../y is."""
    path, base = posixpath.normpath(path), posixpath.normpath(base)
    return path == base or path.startswith(base.rstrip("/") + "/")


def touches_production(path: str) -> bool:
    return any(within(path, prod) for prod in PROD_PATHS)


def is_production_name(value: str) -> bool:
    """A production ssh alias, address or public hostname (case and trailing dot ignored)."""
    name = (value or "").strip().lower().rstrip(".")
    if "@" in name:
        name = name.rsplit("@", 1)[1]
    return name in PROD_SSH_ALIASES or name in PROD_IPS or name in PROD_HOSTNAMES


def assert_dev_constants() -> None:
    """The dev names above must stay disjoint from production even after an edit."""
    for path in (OPT, RELEASES, CURRENT, LOCK, HISTORY, UNIT_PATH, *STATE_DIRS):
        if touches_production(path):
            raise Refused(f"dev path {path} is inside a production path")
    if UNIT == PROD_UNIT or USER == PROD_USER or UNIT_PATH == PROD_UNIT_PATH:
        raise Refused("dev unit/user equals the production one")
    if ({LOBBY_PORT} | MATCH_PORTS) & PROD_PORTS:
        raise Refused("dev ports overlap production ports")
    if is_production_name(DEFAULT_HOST) or is_production_name(DEFAULT_PUBLIC_HOST):
        raise Refused("dev host defaults name production")


def ipv4_literal(value: str) -> str | None:
    try:
        return str(ipaddress.IPv4Address(value))
    except (ipaddress.AddressValueError, ValueError):
        return None


def validate_dev_ip(value: str | None) -> str | None:
    """$DEV_IP: a public IPv4 that is not production."""
    if value is None or value == "":
        return None
    address = ipv4_literal(value.strip())
    if address is None:
        raise Refused(f"{IP_ENV} must be an IPv4 address, got {value!r}")
    parsed = ipaddress.IPv4Address(address)
    if address in PROD_IPS:
        raise Refused(f"{address} is the production VPS; the dev lobby runs on the dedicated dev VPS")
    if parsed.is_loopback or parsed.is_unspecified or parsed.is_multicast or parsed.is_link_local:
        raise Refused(f"{IP_ENV} {address} is not a host address")
    return address


def validate_public_host(value: str) -> str:
    """Where clients reach the match worker: a *.dev.omoba.io / *.dev.ekza.io name or the dev IPv4."""
    host = (value or "").strip().lower().rstrip(".")
    if is_production_name(host):
        raise Refused(f"public host {value!r} is production")
    if ipv4_literal(host):
        return validate_dev_ip(host)
    if not DEV_HOSTNAME.fullmatch(host) or len(host) > 200:
        raise Refused(f"public host must be a *.dev.omoba.io / *.dev.ekza.io name or the dev "
                      f"VPS IPv4, got {value!r}")
    return host


# ---------------------------------------------------------------- unit file

def parse_unit(text: str) -> tuple[dict[str, list[str]], dict[str, list[str]]]:
    """(directives, environment) as lists, so duplicates are visible."""
    directives: dict[str, list[str]] = {}
    environment: dict[str, list[str]] = {}
    section = None
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith(("#", ";")):
            continue
        if line.startswith("[") and line.endswith("]"):
            section = line[1:-1]
            continue
        key, sep, value = line.partition("=")
        if not sep:
            continue
        key, value = key.strip(), value.strip()
        directives.setdefault(f"{section}.{key}", []).append(value)
        if section == "Service" and key == "Environment":
            for assignment in value.split():
                name, _, setting = assignment.strip('"').partition("=")
                environment.setdefault(name, []).append(setting)
    return directives, environment


def unit_public_host(text: str) -> str | None:
    values = parse_unit(text)[1].get(PUBLIC_HOST_KEY, [])
    return values[0] if len(values) == 1 else None


def render_unit(text: str, public_host: str | None) -> str:
    """The repository unit with only OMOBA_MATCH_PUBLIC_HOST replaced."""
    if public_host is None:
        return text
    host = validate_public_host(public_host)
    pattern = re.compile(rf"^Environment={PUBLIC_HOST_KEY}=\S*$", re.M)
    if len(pattern.findall(text)) != 1:
        raise Refused(f"the unit must set {PUBLIC_HOST_KEY} on exactly one Environment= line")
    return pattern.sub(f"Environment={PUBLIC_HOST_KEY}={host}", text)


def check_unit(text: str) -> list[str]:
    """Every reason this unit is not exactly the dev lobby; empty means it is."""
    errors = []
    directives, environment = parse_unit(text)
    for key, expected in UNIT_SERVICE.items():
        values = directives.get(f"Service.{key}", [])
        if values != [expected]:
            errors.append(f"[Service] {key} must be exactly {expected!r}, found {values}")
    for key, expected in UNIT_ENV.items():
        values = environment.get(key, [])
        if values != [expected]:
            errors.append(f"Environment {key} must be exactly {expected!r}, found {values}")
    public = environment.get(PUBLIC_HOST_KEY, [])
    if len(public) != 1:
        errors.append(f"Environment {PUBLIC_HOST_KEY} must appear once, found {public}")
    else:
        try:
            validate_public_host(public[0])
        except Refused as refusal:
            errors.append(refusal.message)
    for key in UNIT_FORBIDDEN_KEYS:
        if f"Service.{key}" in directives:
            errors.append(f"[Service] {key} is not allowed in the guest-only dev lobby")
    for key in UNIT_FORBIDDEN_ENV:
        if key in environment:
            errors.append(f"Environment {key} is not allowed in the guest-only dev lobby")
    for key, values in directives.items():
        if len(values) > 1 and not key.endswith(".Environment"):
            errors.append(f"{key} appears {len(values)} times")
        for value in values:
            for token in re.findall(r"/[^\s\"']*", value):
                if touches_production(token):
                    errors.append(f"{key} references production path {token}")
            for token in re.findall(r"[A-Za-z0-9][A-Za-z0-9.-]*", value):
                if is_production_name(token):
                    errors.append(f"{key} references production host {token}")
    ports = set()
    try:
        ports.add(int(environment["SERVER_ADDR"][0].rsplit(":", 1)[1]))
        first = int(environment["OMOBA_MATCH_FIRST_PORT"][0])
        ports.update(range(first, first + int(environment["OMOBA_MATCH_CAPACITY"][0])))
    except (KeyError, IndexError, ValueError):
        errors.append("lobby/match ports are not readable")
    if ports & PROD_PORTS:
        errors.append(f"ports {sorted(ports & PROD_PORTS)} belong to the production lobby")
    if any(token in text.lower() for token in FORBIDDEN_TOKENS):
        errors.append("the unit mentions the production Supabase project")
    if UNIT_PATH.rsplit("/", 1)[1] != UNIT + ".service":
        errors.append("unexpected unit file name")
    return errors


def assert_dev_unit(text: str) -> None:
    errors = check_unit(text)
    if errors:
        raise Refused("unit is not the dev lobby: " + "; ".join(errors))


def is_linux_x86_64_elf(head: bytes) -> bool:
    # ELF magic, 64-bit, little-endian, e_machine 0x3e (x86-64).
    return len(head) >= 20 and head[:4] == b"\x7fELF" and head[4] == 2 and head[5] == 1 \
        and head[18:20] == b"\x3e\x00"


def validate_release_name(name: str) -> str:
    if not RELEASE_NAME.fullmatch(name or ""):
        raise Refused(f"release name must look like <version>-dev-<sha7>, got {name!r}")
    return name


# ---------------------------------------------------------------- host half

def _resolve(host: str, family: int) -> set[str]:
    try:
        return {info[4][0] for info in socket.getaddrinfo(host, None, family, socket.SOCK_DGRAM)}
    except (socket.gaierror, UnicodeError):
        return set()


class Host:
    """The host side. `root`, `run` and `resolve` exist so tests can use a fake
    filesystem, fake commands and fake DNS; the remote entry point always uses
    root "/", real commands and the host's resolver."""

    def __init__(self, root: str = "/", run=None, sleep=time.sleep, resolve=_resolve):
        self.root = root
        self.run = run or self._run
        self.sleep = sleep
        self.resolve = resolve
        self.mutating = False
        assert_dev_constants()

    @staticmethod
    def _run(args, check=True):
        try:
            result = subprocess.run(args, capture_output=True, text=True, timeout=120)
        except FileNotFoundError:
            result = subprocess.CompletedProcess(args, 127, "", f"{args[0]}: not found")
        if check and result.returncode:
            raise SystemExit(f"{' '.join(args)} failed: {result.stderr.strip() or result.stdout.strip()}")
        return result

    def p(self, path: str) -> str:
        return os.path.join(self.root, path.lstrip("/"))

    def write_guard(self, path: str) -> None:
        if not self.mutating:
            raise Refused(f"internal: write to {path} in a read-only mode")
        if touches_production(path):
            raise Refused(f"internal: write to production path {path}")

    def out(self, *args, check=True) -> str:
        return self.run(list(args), check=check).stdout.strip()

    # ---- read-only facts

    def show(self, unit: str, prop: str) -> str:
        return self.out("systemctl", "show", unit, "-p", prop, "--value", check=False)

    def active(self, unit: str) -> bool:
        return self.run(["systemctl", "is-active", "--quiet", unit], check=False).returncode == 0

    def uid(self, user: str) -> int | None:
        result = self.run(["id", "-u", user], check=False)
        return int(result.stdout.strip()) if result.returncode == 0 else None

    def passwd(self, user: str) -> list[str] | None:
        result = self.run(["getent", "passwd", user], check=False)
        return result.stdout.strip().split(":") if result.returncode == 0 else None

    def local_ipv4(self) -> set[str]:
        addresses = set()
        for line in self.out("ip", "-o", "-4", "addr", "show", check=False).splitlines():
            found = re.search(r"\binet (\d+\.\d+\.\d+\.\d+)/", line)
            if found:
                addresses.add(found.group(1))
        return addresses

    def udp_ports(self) -> set[int]:
        ports = set()
        for line in self.out("ss", "-Hlun", check=False).splitlines():
            fields = line.split()
            if len(fields) >= 4 and ":" in fields[3]:
                try:
                    ports.add(int(fields[3].rsplit(":", 1)[1]))
                except ValueError:
                    pass
        return ports

    def available_mib(self) -> int:
        with open(self.p("/proc/meminfo")) as handle:
            for line in handle:
                if line.startswith("MemAvailable:"):
                    return int(line.split()[1]) // 1024
        raise Refused("MemAvailable missing from /proc/meminfo")

    def current_release(self) -> str | None:
        link = self.p(CURRENT)
        if not os.path.lexists(link):
            return None
        if not os.path.islink(link):
            raise Refused(f"{CURRENT} exists but is not a symlink")
        target = os.path.realpath(link)
        releases = os.path.realpath(self.p(RELEASES))
        if os.path.dirname(target) != releases:
            raise Refused(f"{CURRENT} points outside {RELEASES}: {os.readlink(link)}")
        return os.path.basename(target)

    def production_markers(self) -> list[str]:
        """Anything that says this machine is the production VPS. Read-only."""
        markers = [path for path in PROD_PATHS if os.path.lexists(self.p(path))]
        if self.show(PROD_UNIT, "LoadState") == "loaded":
            markers.append(f"systemd unit {PROD_UNIT}")
        if self.passwd(PROD_USER) is not None:
            markers.append(f"user {PROD_USER}")
        markers += [f"address {ip}" for ip in sorted(self.local_ipv4() & PROD_IPS)]
        return markers

    def dev_main_pid(self) -> int:
        try:
            return int(self.show(UNIT, "MainPID") or 0)
        except ValueError:
            return 0

    def exe(self, pid: int) -> str:
        return os.path.realpath(self.p(f"/proc/{pid}/exe"))

    def installed_unit(self) -> str | None:
        path = self.p(UNIT_PATH)
        return Path(path).read_text() if os.path.isfile(path) else None

    def firewall_notes(self) -> list[str]:
        """ufw is reported, never changed: opening ports is a host bootstrap decision."""
        result = self.run(["ufw", "status"], check=False)
        if result.returncode != 0 or "Status: active" not in result.stdout:
            return []
        missing = [f"{port}/udp" for port in sorted({LOBBY_PORT} | MATCH_PORTS)
                   if not re.search(rf"^{port}/udp\s+ALLOW", result.stdout, re.M)]
        return [f"ufw is active without {', '.join(missing)}: run "
                + " && ".join(f"ufw allow {rule} comment {UNIT}" for rule in missing)] if missing else []

    def public_host_problems(self, public_host: str, expected_ip: str | None) -> list[str]:
        """Clients resolve OMOBA_MATCH_PUBLIC_HOST themselves; check it from the host's DNS."""
        if ipv4_literal(public_host):
            if expected_ip and public_host != expected_ip:
                return [f"public host {public_host} is not the dev VPS address {expected_ip}"]
            return []
        v4 = self.resolve(public_host, socket.AF_INET)
        v6 = {address for address in self.resolve(public_host, socket.AF_INET6)
              if not address.lower().startswith("::ffff:")}
        problems = []
        if not v4:
            problems.append(f"{public_host} has no A record; add it or ship with --public-host <dev IPv4>")
        elif expected_ip and v4 != {expected_ip}:
            problems.append(f"{public_host} resolves to {sorted(v4)}, not the dev VPS {expected_ip}")
        if v4 & PROD_IPS:
            problems.append(f"{public_host} resolves to the production VPS")
        if v6:
            problems.append(f"{public_host} has AAAA records {sorted(v6)}; the lobby binds IPv4 only "
                            "and clients would try IPv6 first: remove them")
        return problems

    # ---- guards (no writes)

    def guard_not_production(self) -> None:
        markers = self.production_markers()
        if markers:
            raise Refused("this host looks like the production VPS (" + ", ".join(markers)
                          + "); the dev lobby only runs on the dedicated dev VPS")

    def guard_layout(self) -> None:
        for path in (OPT, RELEASES, *STATE_DIRS):
            local = self.p(path)
            if os.path.islink(local):
                raise Refused(f"{path} is a symlink; the dev contour owns real directories only")
            if os.path.lexists(local) and not os.path.isdir(local):
                raise Refused(f"{path} exists and is not a directory")
        self.current_release()

    def guard_user(self) -> bool:
        """True when omoba-dev already exists and is the dev account."""
        entry = self.passwd(USER)
        if entry is None:
            return False
        uid = self.uid(USER)
        if uid is None or uid == 0:
            raise Refused(f"user {USER} has uid {uid}")
        if len(entry) < 7 or entry[5] != STATE or not entry[6].endswith("nologin"):
            raise Refused(f"user {USER} exists with an unexpected home/shell: {entry[5:7]}")
        return True

    def guard_no_matches(self) -> None:
        if not self.active(UNIT):
            return
        main = self.dev_main_pid()
        result = self.run(["pgrep", "-u", USER, "-x", "server"], check=False)
        pids = sorted(int(pid) for pid in result.stdout.split())
        if pids != [main]:
            raise Refused(f"dev match workers are running ({pids}, lobby {main}); retry when the dev lobby is idle")

    def guard_ports(self) -> None:
        if self.active(UNIT):
            return
        held = ({LOBBY_PORT} | MATCH_PORTS) & self.udp_ports()
        if held:
            raise Refused(f"UDP {sorted(held)} already bound while {UNIT} is not running")

    def guard_memory(self) -> None:
        available = self.available_mib()
        if available < MIN_AVAILABLE_MIB:
            raise Refused(f"only {available} MiB available (< {MIN_AVAILABLE_MIB}); free memory on the dev host first")

    def guard_unit_payload(self, payload: dict) -> str | None:
        unit = payload.get("unit")
        if unit is None:
            return None
        assert_dev_unit(unit)
        return unit

    def release_dir(self, release: str) -> str:
        validate_release_name(release)
        path = posixpath.join(RELEASES, release)
        if posixpath.dirname(path) != RELEASES or touches_production(path):
            raise Refused(f"release path {path} escapes {RELEASES}")
        return path

    def verify_release(self, release: str, sha256: str) -> str:
        directory = self.release_dir(release)
        server = self.p(directory + "/server")
        if os.path.islink(server) or not os.path.isfile(server):
            raise Refused(f"{directory}/server is missing or a symlink")
        with open(server, "rb") as handle:
            if not is_linux_x86_64_elf(handle.read(64)):
                raise Refused(f"{directory}/server is not a Linux x86-64 executable")
        actual = sha256_file(server)
        if actual != sha256:
            raise Refused(f"{directory}/server sha256 {actual} != expected {sha256}")
        return directory

    def running_release_ok(self, directory: str) -> bool:
        pid = self.dev_main_pid()
        return self.active(UNIT) and pid > 0 and \
            self.exe(pid) == os.path.realpath(self.p(directory + "/server"))

    # ---- modes

    def status(self) -> dict:
        releases = sorted(os.listdir(self.p(RELEASES))) if os.path.isdir(self.p(RELEASES)) else []
        installed = self.installed_unit()
        return {
            "unit": UNIT, "active": self.active(UNIT),
            "dev": {key: self.show(UNIT, key) for key in ("ActiveState", "MainPID", "NRestarts", "MemoryCurrent")},
            "current": self.current_release() if os.path.isdir(self.p(OPT)) else None,
            "releases": releases, "user": self.passwd(USER) is not None,
            "public_host": unit_public_host(installed) if installed else None,
            "udp": sorted(self.udp_ports() & ({LOBBY_PORT} | MATCH_PORTS | PROD_PORTS)),
            "local_ipv4": sorted(self.local_ipv4()),
            "production_markers": self.production_markers(),
            "firewall": self.firewall_notes(),
            "available_mib": self.available_mib(),
        }

    def preflight(self, payload: dict) -> dict:
        """Every guard ship/rollback would run, with the actions it would take. Read-only."""
        self.guard_not_production()
        self.guard_layout()
        user = self.guard_user()
        self.guard_no_matches()
        self.guard_ports()
        self.guard_memory()
        release = payload.get("release")
        directory = self.release_dir(release) if release else None
        unit = self.guard_unit_payload(payload)
        expected_ip = payload.get("expected_ip")
        warnings = []
        local = self.local_ipv4()
        if expected_ip and expected_ip not in local:
            warnings.append(f"{expected_ip} is not on a local interface ({sorted(local)}); "
                            "fine behind provider NAT, otherwise check DEV_IP")
        public_host = unit_public_host(unit or self.installed_unit() or "")
        problems = self.public_host_problems(public_host, expected_ip) if public_host else []
        warnings += self.firewall_notes()
        would = []
        if not user:
            would.append(f"useradd --system {USER} (home {STATE}, nologin)")
        would += [f"install -d {path}" for path in (OPT, RELEASES, *STATE_DIRS) if not os.path.isdir(self.p(path))]
        if directory:
            staged = os.path.isfile(self.p(directory + "/server"))
            would.append(f"{'verify staged' if staged else 'stage'} {directory}/server")
        if unit is not None and unit != self.installed_unit():
            would.append(f"install {UNIT_PATH} + systemctl daemon-reload")
        if directory:
            if self.current_release() == release and unit in (None, self.installed_unit()) \
                    and self.running_release_ok(directory):
                would.append(f"nothing: {UNIT} already runs {release}")
            else:
                would += [f"{CURRENT} -> releases/{release}", f"systemctl enable + restart {UNIT}",
                          f"verify /proc/<MainPID>/exe == {directory}/server and UDP {LOBBY_PORT}"]
        return {"mode": "preflight", "ok": not problems, "current": self.current_release(),
                "public_host": public_host, "problems": problems, "warnings": warnings, "would": would}

    def _install_dir(self, path: str, owner: str = "root", mode: str = "0755") -> None:
        self.write_guard(path)
        if not os.path.isdir(self.p(path)):
            self.run(["install", "-d", "-o", owner, "-g", owner, "-m", mode, self.p(path)])

    def stage(self, payload: dict) -> dict:
        self.guard_not_production()
        self.guard_layout()
        user = self.guard_user()
        directory = self.release_dir(payload["release"])
        self.mutating = True
        if not user:
            self.write_guard(STATE)
            self.run(["useradd", "--system", "--user-group", "--home-dir", STATE,
                      "--shell", "/usr/sbin/nologin", USER])
            self.guard_user()
        self._install_dir(OPT)
        self._install_dir(RELEASES)
        for path in STATE_DIRS:
            self._install_dir(path, USER, "0750")
        server = self.p(directory + "/server")
        if os.path.isfile(server):
            if sha256_file(server) != payload["sha256"]:
                raise Refused(f"{directory} already holds a different server; releases are immutable")
            return {"mode": "stage", "release": payload["release"], "staged": "already"}
        self._install_dir(directory)
        return {"mode": "stage", "release": payload["release"], "upload_to": directory + "/server.part"}

    def _point_current(self, release: str) -> None:
        self.write_guard(CURRENT)
        temporary = self.p(CURRENT + ".next")
        if os.path.lexists(temporary):
            os.unlink(temporary)
        os.symlink(f"releases/{release}", temporary)
        os.replace(temporary, self.p(CURRENT))

    def _write_unit(self, text: str) -> None:
        self.write_guard(UNIT_PATH)
        temporary = self.p(UNIT_PATH + ".next")
        with open(temporary, "w") as handle:
            handle.write(text)
        os.chmod(temporary, 0o644)
        os.replace(temporary, self.p(UNIT_PATH))

    def _verify_unit(self, text: str) -> None:
        with tempfile.TemporaryDirectory(prefix="omoba-lobby-dev-") as directory:
            candidate = os.path.join(directory, UNIT + ".service")
            with open(candidate, "w") as handle:
                handle.write(text)
            self.run(["systemd-analyze", "verify", candidate])

    def _finalize_upload(self, directory: str, payload: dict) -> None:
        part, server = self.p(directory + "/server.part"), self.p(directory + "/server")
        if os.path.isfile(part) and not os.path.lexists(server):
            self.write_guard(directory + "/server.part")
            if sha256_file(part) != payload["sha256"]:
                os.unlink(part)
                raise Refused("uploaded server.part does not match the local sha256; removed it")
            os.chmod(part, 0o755)
            os.replace(part, server)
        build = self.p(directory + "/BUILD.json")
        if payload.get("build") and not os.path.isfile(build):
            self.write_guard(directory + "/BUILD.json")
            with open(build, "w") as handle:
                json.dump(payload["build"], handle, indent=2)
                handle.write("\n")
            os.chmod(build, 0o644)

    def _start_and_verify(self, directory: str) -> int:
        self.run(["systemctl", "enable", UNIT])
        self.run(["systemctl", "restart", UNIT])
        for _ in range(10):
            self.sleep(1)
            if self.active(UNIT) and LOBBY_PORT in self.udp_ports():
                break
        if not self.active(UNIT):
            raise SystemExit(f"{UNIT} is not active after start")
        pid = self.dev_main_pid()
        if pid <= 0 or self.exe(pid) != os.path.realpath(self.p(directory + "/server")):
            raise SystemExit(f"{UNIT} MainPID {pid} does not run {directory}/server")
        if LOBBY_PORT not in self.udp_ports():
            raise SystemExit(f"{UNIT} is running but UDP {LOBBY_PORT} is not bound")
        return pid

    def _record(self, record: dict) -> None:
        self.write_guard(HISTORY)
        with open(self.p(HISTORY), "a") as handle:
            handle.write(json.dumps(record) + "\n")

    def activate(self, payload: dict, mode: str = "activate") -> dict:
        """Switch the dev lobby to payload["release"]; roll back on any failure."""
        self.guard_not_production()
        self.guard_layout()
        if not self.guard_user():
            raise Refused(f"user {USER} is missing; run stage first")
        release = payload["release"]
        directory = self.release_dir(release)
        unit = self.guard_unit_payload(payload)
        old_unit = self.installed_unit()
        public_host = unit_public_host(unit or old_unit or "")
        problems = self.public_host_problems(public_host, payload.get("expected_ip")) if public_host else []
        if problems:
            raise Refused("match workers would be unreachable: " + "; ".join(problems))
        unit_changed = unit is not None and unit != old_unit
        if not unit_changed and old_unit is None:
            raise Refused(f"{UNIT_PATH} is not installed and no unit was supplied")
        previous = self.current_release()
        self.mutating = True
        self._finalize_upload(directory, payload)
        self.verify_release(release, payload["sha256"])
        if previous == release and not unit_changed and self.running_release_ok(directory):
            return {"status": "UNCHANGED", "mode": mode, "release": release, "pid": self.dev_main_pid()}
        self.guard_no_matches()
        self.guard_ports()
        self.guard_memory()
        was_active = self.active(UNIT)
        if was_active:
            self.run(["systemctl", "stop", UNIT])
        try:
            self._point_current(release)
            if unit_changed:
                self._verify_unit(unit)
                self._write_unit(unit)
                self.run(["systemctl", "daemon-reload"])
            pid = self._start_and_verify(directory)
        except BaseException as error:
            rolled = self._rollback(previous, old_unit, unit_changed, was_active)
            raise SystemExit(f"{mode} of {release} failed ({error}); {rolled}") from error
        markers = self.production_markers()
        if markers:
            raise SystemExit(f"PRODUCTION MARKERS APPEARED during the dev switch: {markers}")
        record = {"mode": mode, "release": release, "previous": previous, "pid": pid,
                  "sha256": payload["sha256"], "unit_changed": unit_changed, "public_host": public_host,
                  "at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}
        self._record(record)
        return {"status": "DEPLOYED", **record}

    def _rollback(self, previous: str | None, old_unit: str | None, unit_changed: bool,
                  was_active: bool) -> str:
        """Put the dev lobby back as it was: release, unit file, running or not. A failed
        first install keeps the new unit, stopped and disabled, for `journalctl -u`."""
        self.run(["systemctl", "stop", UNIT], check=False)
        if previous is None:
            self.run(["systemctl", "disable", UNIT], check=False)
            return "first install: dev lobby left stopped and disabled"
        if unit_changed:
            if old_unit is not None:
                self._write_unit(old_unit)
            self.run(["systemctl", "daemon-reload"], check=False)
        self._point_current(previous)
        if was_active:
            self.run(["systemctl", "start", UNIT], check=False)
        return f"ROLLED_BACK to {previous}"

    def rollback(self, payload: dict) -> dict:
        release = payload["release"]
        directory = self.release_dir(release)
        build = self.p(directory + "/BUILD.json")
        if not os.path.isfile(build):
            raise Refused(f"{directory}/BUILD.json is missing; not a release this tool staged")
        with open(build) as handle:
            info = json.load(handle)
        if info.get("contour") != "dev" or info.get("release") != release:
            raise Refused(f"{directory}/BUILD.json is not a dev release record for {release}")
        return self.activate({"release": release, "sha256": info["sha256"], "unit": None,
                              "expected_ip": payload.get("expected_ip")}, mode="rollback")


def sha256_file(path: str) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def encode_payload(payload: dict) -> str:
    return base64.b64encode(json.dumps(payload, sort_keys=True).encode()).decode()


def decode_payload(raw: str) -> dict:
    if not PAYLOAD.fullmatch(raw):
        raise Refused("payload is not base64")
    return json.loads(base64.b64decode(raw))


def remote_main(argv: list[str]) -> int:
    """Host entry point: `python3 - remote <mode> <payload>`; always root "/"."""
    if len(argv) not in (1, 2) or argv[0] not in ("status", "preflight", "stage", "activate", "rollback"):
        raise Refused("usage: remote status|preflight|stage|activate|rollback [payload]")
    if os.geteuid() != 0 and argv[0] != "status":
        raise Refused("the host half runs as root")
    host = Host()
    payload = decode_payload(argv[1]) if len(argv) == 2 else {}
    if argv[0] in ("status", "preflight"):
        result = host.status() if argv[0] == "status" else host.preflight(payload)
    else:
        import fcntl
        with open(host.p(LOCK), "a") as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except OSError:
                raise Refused("another lobby_dev.py run holds the lock") from None
            result = {"stage": host.stage, "activate": host.activate, "rollback": host.rollback}[argv[0]](payload)
    print(json.dumps(result, indent=2))
    return 0


# ---------------------------------------------------------------- Mac half

def repo_root() -> Path:
    return Path(__file__).resolve().parents[3]


def ssh_config(alias: str) -> dict[str, str] | None:
    """`ssh -G`: what ssh would connect to, from local config only (no network)."""
    try:
        result = subprocess.run(["ssh", "-G", alias], capture_output=True, text=True, timeout=10)
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return None
    if result.returncode:
        return None
    config = {}
    for line in result.stdout.splitlines():
        key, _, value = line.partition(" ")
        config.setdefault(key.lower(), value.strip())
    return config


def resolve_target(args, need_ip: bool, config_reader=None) -> dict:
    """The dev VPS this run talks to; refuses production before any ssh."""
    alias = args.host or os.environ.get(HOST_ENV) or DEFAULT_HOST
    if not SSH_ALIAS.fullmatch(alias):
        raise Refused(f"ssh alias {alias!r} is not a plain host alias")
    if is_production_name(alias):
        raise Refused(f"{alias} is the production VPS; the dev lobby runs on the dedicated dev VPS ({DEFAULT_HOST})")
    expected_ip = validate_dev_ip(args.ip if args.ip is not None else os.environ.get(IP_ENV))
    config = (config_reader or ssh_config)(alias)
    hostname = (config or {}).get("hostname")
    if hostname and is_production_name(hostname):
        raise Refused(f"ssh alias {alias} resolves to the production VPS ({hostname})")
    if need_ip:
        if expected_ip is None:
            raise Refused(f"set {IP_ENV} (or --ip) to the dev VPS IPv4 before --check/--apply")
        if config is None:
            raise Refused(f"`ssh -G {alias}` failed; is {alias} in ~/.ssh/config?")
        if hostname != expected_ip:
            raise Refused(f"ssh alias {alias} connects to {hostname!r}, not {IP_ENV}={expected_ip}; "
                          f"set `HostName {expected_ip}` for Host {alias} in ~/.ssh/config")
    return {"alias": alias, "hostname": hostname, "user": (config or {}).get("user"),
            "expected_ip": expected_ip}


def read_package(package: Path) -> tuple[str, bytes, str]:
    """(version, omoba-server bytes, package sha256) from a dev Linux package only."""
    match = DEV_PACKAGE.fullmatch(package.name)
    if not match:
        raise Refused(f"{package.name} is not a dev Linux package (omoba-<version>-dev-linux-x64.tar.gz); "
                      "build it with: python3 scripts/release.py build linux --contour dev")
    version = match.group(1)
    member_name = f"omoba-{version}-dev/omoba-server"
    with tarfile.open(package, "r:gz") as archive:
        members = [m for m in archive.getmembers() if m.name == member_name]
        if len(members) != 1 or not members[0].isfile():
            raise Refused(f"{package.name} has no regular file {member_name}")
        data = archive.extractfile(members[0]).read()
    if not is_linux_x86_64_elf(data[:64]):
        raise Refused(f"{member_name} is not a Linux x86-64 executable")
    return version, data, sha256_file(str(package))


def source_revision(explicit: str | None, version: str) -> str:
    if explicit:
        if not re.fullmatch(r"[0-9a-f]{7,40}", explicit):
            raise Refused(f"--revision must be a hex commit id, got {explicit!r}")
        return explicit
    root = repo_root()
    if subprocess.run(["git", "status", "--porcelain"], cwd=root, capture_output=True, text=True).stdout.strip():
        raise Refused(f"{root} has uncommitted changes; ship from a clean checkout or pass --revision")
    cargo = (root / "Cargo.toml").read_text()
    found = re.search(r'(?ms)^\[workspace\.package\]\s*\n(?:(?!^\[).)*?^version\s*=\s*"([^"\n]+)"', cargo)
    if not found or found.group(1) != version:
        raise Refused(f"package version {version} does not match this checkout "
                      f"({found.group(1) if found else 'unknown'}); pass --revision")
    return subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()


def prepare(args) -> dict:
    """Everything that can be checked on the Mac, before any network."""
    unit_path = Path(args.unit) if args.unit else repo_root() / "ops/vps/omoba-lobby-dev.service"
    public_host = args.public_host or os.environ.get(PUBLIC_HOST_ENV) or None
    unit = render_unit(unit_path.read_text(), public_host)
    assert_dev_unit(unit)
    if args.package:
        version, data, package_sha = read_package(Path(args.package))
        package_name = Path(args.package).name
    else:
        data = Path(args.binary).read_bytes()
        if not is_linux_x86_64_elf(data[:64]):
            raise Refused(f"{args.binary} is not a Linux x86-64 executable")
        if not args.version:
            raise Refused("--binary needs --version")
        version, package_sha, package_name = args.version, None, None
    revision = source_revision(args.revision, version)
    release = validate_release_name(args.release or f"{version}-dev-{revision[:7]}")
    sha = hashlib.sha256(data).hexdigest()
    build = {"contour": "dev", "release": release, "version": version, "revision": revision,
             "sha256": sha, "package": package_name, "package_sha256": package_sha,
             "unit_sha256": hashlib.sha256(unit.encode()).hexdigest(), "tool": "ops/vps/dev/lobby_dev.py"}
    return {"release": release, "sha256": sha, "data": data, "unit": unit, "build": build,
            "public_host": unit_public_host(unit)}


def ssh_remote(alias: str, mode: str, payload: dict | None = None) -> str:
    command = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", alias, "python3", "-", "remote", mode]
    if payload is not None:
        command.append(encode_payload(payload))
    source = Path(__file__).read_bytes()
    result = subprocess.run(command, input=source, capture_output=True)
    sys.stderr.write(result.stderr.decode(errors="replace"))
    if result.returncode:
        raise SystemExit(f"remote {mode} failed (exit {result.returncode})")
    return result.stdout.decode()


def command_ship(args) -> int:
    staged = prepare(args)
    target = resolve_target(args, need_ip=args.check or args.apply)
    alias = target["alias"]
    payload = {key: staged[key] for key in ("release", "sha256", "unit", "build")}
    payload["expected_ip"] = target["expected_ip"]
    directory = f"{RELEASES}/{staged['release']}"
    plan = {
        "target": target, "release": staged["release"], "sha256": staged["sha256"],
        "public_host": staged["public_host"], "build": staged["build"],
        "steps": [
            f"remote preflight (read-only): no production markers, {USER} layout, no dev matches, "
            f"UDP {LOBBY_PORT}/{sorted(MATCH_PORTS)} free, >= {MIN_AVAILABLE_MIB} MiB available, "
            f"{staged['public_host']} resolves to the dev VPS (A only)",
            f"remote stage: user {USER}, {OPT}, {STATE}, {directory}/",
            f"scp omoba-server -> {alias}:{directory}/server.part",
            f"remote activate: sha256 check, {CURRENT} -> releases/{staged['release']}, unit if changed, "
            f"restart {UNIT}, verify, roll back on failure (no-op when already running this release)",
        ],
    }
    if not (args.check or args.apply):
        print(json.dumps({"dry_run": True, **plan}, indent=2))
        print("Dry run: nothing was contacted. --check runs the read-only host preflight; --apply ships.",
              file=sys.stderr)
        return 0
    preflight = ssh_remote(alias, "preflight", payload)
    print(preflight)
    if not args.apply:
        return 0
    if not json.loads(preflight).get("ok"):
        raise Refused("host preflight reported problems; fix them first")
    print(ssh_remote(alias, "stage", {"release": staged["release"], "sha256": staged["sha256"]}))
    with tempfile.NamedTemporaryFile(prefix="omoba-server-dev-", delete=False) as handle:
        handle.write(staged["data"])
    try:
        subprocess.run(["scp", "-q", "-o", "BatchMode=yes", handle.name,
                        f"{alias}:{directory}/server.part"], check=True)
    finally:
        os.unlink(handle.name)
    print(ssh_remote(alias, "activate", payload))
    return 0


def command_rollback(args) -> int:
    release = validate_release_name(args.release)
    target = resolve_target(args, need_ip=args.check or args.apply)
    payload = {"release": release, "unit": None, "expected_ip": target["expected_ip"]}
    if not (args.check or args.apply):
        print(json.dumps({"dry_run": True, "target": target, "steps": [
            f"remote preflight (read-only) for {RELEASES}/{release}",
            f"remote rollback: {CURRENT} -> releases/{release} (BUILD.json contour dev, sha256 checked), "
            f"restart {UNIT}, verify"]}, indent=2))
        return 0
    print(ssh_remote(target["alias"], "preflight", payload))
    if args.apply:
        print(ssh_remote(target["alias"], "rollback", payload))
    return 0


def command_status(args) -> int:
    target = resolve_target(args, need_ip=True)
    print(ssh_remote(target["alias"], "status"))
    return 0


def main(argv=None) -> int:
    argv = sys.argv[1:] if argv is None else argv
    if argv[:1] == ["remote"]:
        return remote_main(argv[1:])
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    ship = sub.add_parser("ship", help="install or update the dev lobby (dry-run unless --check/--apply)")
    source = ship.add_mutually_exclusive_group(required=True)
    source.add_argument("--package", help="dev Linux package from `release.py build linux --contour dev`")
    source.add_argument("--binary", help="a Linux x86-64 server binary (needs --version)")
    ship.add_argument("--version", help="version for --binary")
    ship.add_argument("--revision", help="source commit (default: HEAD of this clean checkout)")
    ship.add_argument("--release", help="release directory name (default: <version>-dev-<sha7>)")
    ship.add_argument("--unit", help="unit file (default: ops/vps/omoba-lobby-dev.service)")
    ship.add_argument("--public-host", help=f"match worker host clients reach (default: the unit's "
                                            f"{DEFAULT_PUBLIC_HOST}; ${PUBLIC_HOST_ENV})")
    rollback = sub.add_parser("rollback", help="point the dev lobby at an earlier staged dev release")
    rollback.add_argument("--release", required=True)
    status = sub.add_parser("status", help="read-only dev lobby state on the dev VPS")
    for command in (ship, rollback, status):
        command.add_argument("--host", help=f"ssh alias of the dev VPS (default: ${HOST_ENV} or {DEFAULT_HOST})")
        command.add_argument("--ip", help=f"dev VPS IPv4 the alias must connect to (default: ${IP_ENV})")
    for command in (ship, rollback):
        mode = command.add_mutually_exclusive_group()
        mode.add_argument("--check", action="store_true", help="run the read-only host preflight only")
        mode.add_argument("--apply", action="store_true", help="change the host")
    args = parser.parse_args(argv)
    return {"ship": command_ship, "rollback": command_rollback, "status": command_status}[args.command](args)


if __name__ == "__main__":
    sys.exit(main())
