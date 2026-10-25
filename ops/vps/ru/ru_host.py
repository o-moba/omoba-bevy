#!/usr/bin/env python3
"""Bootstrap, ship, check and reboot-test the OMOBA RU host (LR-01).

One VPS in Russia runs everything Russian players reach: PostgreSQL, the RU lobby
with its match workers, the RU Account API and the RU player portal behind Caddy
(ops/vps/ru/README.md). This tool sets that host up from a fresh Ubuntu 24.04+ and
ships releases to it. Run it on the Mac from a clean checkout. The target is the RU
VPS only: ssh alias from --host or $RU_HOST (default vds-ru, logging in as root with
a key), its public IPv4 from --ip or $RU_IP. Without --check or --apply nothing is
contacted; nothing on the host changes without --apply:

    export RU_HOST=vds-ru RU_IP=<RU VPS IPv4>
    # 1. host: packages, PostgreSQL 16+ with und-x-icu, Caddy, firewall, key-only SSH,
    #    users, folders, env files (secrets made on the host), the database and the units
    python3 ops/vps/ru/ru_host.py bootstrap --vkid-client-id <VK ID app id> \\
        --node-tarball node-v22.<x>.<y>-linux-x64.tar.xz --node-sha256 <SHASUMS256.txt line>
    python3 ops/vps/ru/ru_host.py bootstrap ... --check   # read-only preflight on the host
    python3 ops/vps/ru/ru_host.py bootstrap ... --apply
    # 2. lobby + Account API from the RU Linux package (pg_dump, migrations, grants)
    python3 scripts/release.py build linux --market ru
    python3 ops/vps/ru/ru_host.py ship --package dist/v<v>-ru/omoba-<v>-ru-linux-x64.tar.gz --apply
    # 3. portal: omoba-web built with the RU env (README.md, "Portal bundle")
    python3 ops/vps/ru/ru_host.py portal-bundle --web ../omoba-web
    python3 ops/vps/ru/ru_host.py ship-portal --bundle dist/portal-ru/omoba-portal-ru-<rev>.tar.gz --apply
    # 4. backups (LR-03): the owner fills /opt/omoba-ru/backup.env and the keys, then the
    #    secrets go to the escrow holder, encrypted to the holder's public key
    python3 ops/vps/ru/ru_host.py escrow --recipient-file holder-public.asc --out omoba-ru-secrets.asc
    # 5. everything at once, read-only; then a reboot that must bring it all back
    python3 ops/vps/ru/ru_host.py status
    python3 ops/vps/ru/ru_host.py reboot-check --apply
    python3 ops/vps/ru/ru_host.py rollback --kind server|portal --release <name> --apply
    # the RU public names (scripts/markets.py) and the DNS A records the owner sets; local only
    python3 ops/vps/ru/ru_host.py names

Guards. On the Mac, before any ssh: the alias and its `ssh -G` HostName must not be
the production or dev VPS, HostName must equal $RU_IP (an IPv4 literal in
~/.ssh/config) and the login user root; bootstrap also asks `whois` whether $RU_IP is
in Russia (--country-confirmed overrides it when the provider says so). On the host
(the same file, piped to `ssh <alias> python3 - remote <mode> <payload>`): it refuses
to run where a production or dev marker exists (units, users, folders, addresses),
on anything but Ubuntu 24.04+ x86-64, or where $RU_IP is not a local address
(--behind-nat accepts provider NAT). Bootstrap writes /opt/omoba-ru/host.json; every
later run needs it to name market ru and $RU_IP. It writes only below /opt/omoba-ru,
/var/lib/omoba-ru, /var/backups/omoba-ru, its own unit files, /etc/caddy/Caddyfile
and /etc/ssh/sshd_config.d/00-omoba-ru.conf, never prints or passes a secret in argv,
never overwrites a generated secret, never interrupts a running match, never migrates
without the encrypted off-host backup of ru_backup.py (LR-03) and rolls a failed ship
back to the previous release.
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
import secrets
import shutil
import socket
import subprocess
import sys
import tarfile
import tempfile
import time
import zlib
from pathlib import Path
from urllib.parse import unquote, urlsplit, urlunsplit

# ---------------------------------------------------------------- the RU host

DEFAULT_HOST = "vds-ru"
HOST_ENV, IP_ENV, DEV_IP_ENV = "RU_HOST", "RU_IP", "DEV_IP"
MARKET = "ru"
TOOL = "ops/vps/ru/ru_host.py"
USER = "omoba-ru"                 # lobby, match workers and Account API
PORTAL_USER = "omoba-portal-ru"   # the portal's Node.js process
OPT = "/opt/omoba-ru"
RELEASES = OPT + "/releases"
CURRENT = OPT + "/current"
PORTAL = OPT + "/portal"
PORTAL_RELEASES = PORTAL + "/releases"
PORTAL_CURRENT = PORTAL + "/current"
NODE = OPT + "/node"
NODE_CURRENT = NODE + "/current"
NODE_SYSTEM = NODE + "/system"    # bin/node -> /usr/bin/node when Ubuntu's Node.js is new enough
INCOMING = OPT + "/incoming"
MARKER = OPT + "/host.json"
HISTORY = OPT + "/deployments.jsonl"
LOCK = "/run/lock/omoba-ru-host.lock"
STATE = "/var/lib/omoba-ru"
STATE_DIRS = (STATE, STATE + "/matches", STATE + "/lobby-outbox")
PORTAL_CACHE = "/var/cache/omoba-portal-ru"   # the portal unit's CacheDirectory
BACKUPS = "/var/backups/omoba-ru"
# Backups (LR-03): ru_backup.py with its units and timers, its settings and keys.
BACKUP_TOOL_DIR = OPT + "/backup"
BACKUP_SCRIPT = BACKUP_TOOL_DIR + "/ru_backup.py"
BACKUP_KEYS = OPT + "/backup-keys"
BACKUP_ENV = OPT + "/backup.env"
BACKUP_ENV_EXAMPLE = "backup.env.example"
BACKUP_UNITS = ("omoba-backup-ru.service", "omoba-backup-ru.timer", "omoba-restore-drill-ru.service",
                "omoba-restore-drill-ru.timer", "omoba-backup-check-ru.service", "omoba-backup-check-ru.timer")
BACKUP_TIMERS = tuple(name for name in BACKUP_UNITS if name.endswith(".timer"))
BACKUP_COMMANDS = {"omoba-backup-ru.service": "backup --reason nightly", "omoba-restore-drill-ru.service": "drill",
                   "omoba-backup-check-ru.service": "check --alert"}
PYTHON = "/usr/bin/python3"
UNIT_DIR = "/etc/systemd/system"
CADDYFILE = "/etc/caddy/Caddyfile"
SSHD_DROPIN = "/etc/ssh/sshd_config.d/00-omoba-ru.conf"
AUTHORIZED_KEYS = "/root/.ssh/authorized_keys"
SAFE_PATH = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"

KINDS = ("lobby", "account-api", "portal")
SERVER_KINDS = ("lobby", "account-api")   # shipped together from the RU Linux package
UNIT_FILES = {"lobby": "omoba-lobby-ru.service", "account-api": "omoba-account-api-ru.service",
              "portal": "omoba-portal-ru.service"}
ENV_EXAMPLES = {"lobby": "lobby.env.example", "account-api": "account-api.env.example",
                "portal": "portal.env.example"}
ENV_PATHS = {kind: f"{OPT}/{name[:-len('.example')]}" for kind, name in ENV_EXAMPLES.items()}
# (path, owner, mode) of every folder bootstrap owns.
DIRS = ((OPT, "root", "0755"), (RELEASES, "root", "0755"), (PORTAL, "root", "0755"),
        (PORTAL_RELEASES, "root", "0755"), (NODE, "root", "0755"), (INCOMING, "root", "0700"),
        *((path, USER, "0700") for path in STATE_DIRS),
        (BACKUP_TOOL_DIR, "root", "0755"), (BACKUP_KEYS, "root", "0700"), (BACKUPS, "root", "0700"))
USERS = {USER: STATE, PORTAL_USER: "/nonexistent"}

LOBBY_PORT = 4000
HTTPS_PORT = 443
ACCOUNT_API_ADDR = ("127.0.0.1", 40550)
PORTAL_ADDR = ("127.0.0.1", 3010)
DB_HOST, DB_PORT = "127.0.0.1", 5432
DB_NAME = "omoba_ru"
DB_OWNER, DB_GAME, DB_PORTAL = "omoba_owner", "omoba_game", "omoba_portal"
DB_ROLES = (DB_OWNER, DB_GAME, DB_PORTAL)
DB_PASSWORD = re.compile(r"[A-Za-z0-9_-]{24,128}")
ICU_COLLATION = "und-x-icu"
MIN_POSTGRES = 16
MIN_UBUNTU = (24, 4)
MIN_NODE = (20, 9)        # Next.js 16
PACKAGES = ("postgresql", "caddy", "ufw", "openssh-server", "gnupg")
# The RU public names and DNS zone are written only in scripts/markets.py (MARKETS["ru"]):
# the Mac half reads them there (ru_names) and hands them to the host half in every
# payload, so the DNS, Caddyfile and unit checks follow a rename of that one file.
HOST_KEYS = ("lobby", "account_api", "portal")
NAME_KEYS = (*HOST_KEYS, "zone")
DNS_NAME = re.compile(r"(?=.{1,253}\Z)(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z](?:[a-z0-9-]{0,61}[a-z0-9])?")
GRANTS = "account-api/ops/grants.sql"
MIGRATION_LOGIN_SECONDS = 900
MIN_FREE_MIB = 2048
PLACEHOLDER = "CHANGE_ME"
LOOPBACK = frozenset({"127.0.0.1", "localhost", "::1"})

SERVER_BINARIES = ("omoba-server", "omoba-account-api")
RU_PACKAGE = re.compile(r"omoba-(\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?)-ru-linux-x64\.tar\.gz")
SERVER_RELEASE = re.compile(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?-ru-[0-9a-f]{7,40}")
PORTAL_BUNDLE = re.compile(r"omoba-portal-ru-([0-9a-f]{7,40})\.tar\.gz")
PORTAL_RELEASE = re.compile(r"[0-9a-f]{7}-[0-9a-f]{8}")
NODE_TARBALL = re.compile(r"node-v(\d+)\.(\d+)\.(\d+)-linux-x64\.tar\.(?:xz|gz)")
VKID = re.compile(r"[0-9]{1,16}")
HEX64 = re.compile(r"[0-9a-f]{64}")
PAYLOAD = re.compile(r"[A-Za-z0-9+/=]*")
SSH_ALIAS = re.compile(r"(?:[a-z_][a-z0-9_-]*@)?[A-Za-z0-9][A-Za-z0-9._-]*")

# ---------------------------------------------------------------- other contours (never a target)

PROD_SSH_ALIASES = frozenset({"vds-eternal"})
DEV_SSH_ALIASES = frozenset({"vds-dev"})
PROD_IPS = frozenset({"77.246.105.57"})
PUBLIC_ZONES = ("omoba.io", "ekza.io")
FOREIGN_PATHS = ("/opt/omoba", "/var/lib/omoba", "/opt/ekza-studio", "/opt/ekza-mirror",
                 "/root/docker/nginx-proxy-manager", "/etc/systemd/system/omoba-lobby.service",
                 "/opt/omoba-dev", "/var/lib/omoba-dev", "/opt/ekza-dev",
                 "/etc/systemd/system/omoba-lobby-dev.service",
                 "/etc/systemd/system/omoba-account-api-dev.service")
FOREIGN_UNITS = ("omoba-lobby", "omoba-lobby-dev", "omoba-account-api-dev")
FOREIGN_USERS = ("omoba", "omoba-dev")
# Hosted Supabase and the production project must never appear in a unit or an env file.
FORBIDDEN_TOKENS = ("snrjxwutqxokeujuiepn", "supabase.co", "supabase.com")

# ---------------------------------------------------------------- what the units must say

COMMON_UNIT_RULES = {"Unit.StartLimitIntervalSec": "0", "Service.Restart": "always",
                     "Service.NoNewPrivileges": "true", "Service.ProtectSystem": "strict",
                     "Install.WantedBy": "multi-user.target"}
UNIT_RULES = {
    "lobby": {"Service.User": USER, "Service.Group": USER, "Service.WorkingDirectory": STATE,
              "Service.EnvironmentFile": ENV_PATHS["lobby"], "Service.ExecStart": CURRENT + "/omoba-server",
              "Service.ReadWritePaths": STATE},
    "account-api": {"Service.User": USER, "Service.Group": USER,
                    "Service.EnvironmentFile": ENV_PATHS["account-api"],
                    "Service.ExecStart": CURRENT + "/omoba-account-api"},
    "portal": {"Service.User": PORTAL_USER, "Service.Group": PORTAL_USER,
               "Service.WorkingDirectory": PORTAL_CURRENT, "Service.EnvironmentFile": ENV_PATHS["portal"],
               "Service.ExecStart": f"{NODE_CURRENT}/bin/node {PORTAL_CURRENT}/server.js",
               "Service.CacheDirectory": posixpath.basename(PORTAL_CACHE)},
}
UNIT_ENV_RULES = {
    "lobby": {"SERVER_ADDR": f"0.0.0.0:{LOBBY_PORT}", "OMOBA_SERVER_ROLE": "lobby",
              "OMOBA_MATCH_ROOT": STATE + "/matches", "OMOBA_CAREER_OUTBOX": STATE + "/lobby-outbox",
              "OMOBA_MATCH_EXECUTABLE": CURRENT + "/omoba-server"},
    "account-api": {},
    "portal": {"HOSTNAME": PORTAL_ADDR[0], "NODE_ENV": "production", "NEXT_TELEMETRY_DISABLED": "1"},
}
# Secrets and the public address live only in the env files; RU has no Ekza registry or passport.
UNIT_FORBIDDEN_ENV = ("OMOBA_DATABASE_URL", "OMOBA_MATCH_PUBLIC_HOST", "OMOBA_PORTAL_SECRET",
                      "OMOBA_SOCIAL_SECRET", "OMOBA_REGISTRY_URL", "OMOBA_PASSPORT_URL", "EKZA_PASSPORT_URL")
NEEDS_POSTGRES = ("lobby", "account-api")
MULTI_VALUED = ("Service.Environment", "Unit.After", "Unit.Wants")

# Key-only SSH. sshd keeps the first value it reads; Ubuntu includes sshd_config.d/*.conf
# first, and this name sorts before cloud-init's 50-cloud-init.conf.
SSHD_TEXT = """# OMOBA RU host (ops/vps/ru/ru_host.py, LR-01): key-only SSH. sshd keeps the first
# value it reads, and this file sorts before cloud-init's 50-cloud-init.conf.
PubkeyAuthentication yes
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitEmptyPasswords no
PermitRootLogin prohibit-password
AuthenticationMethods publickey
"""
SSHD_REQUIRED = {"pubkeyauthentication": ("yes",), "passwordauthentication": ("no",),
                 "kbdinteractiveauthentication": ("no",), "permitemptypasswords": ("no",),
                 "permitrootlogin": ("prohibit-password", "without-password"),
                 "authenticationmethods": ("publickey",)}


class Refused(SystemExit):
    """A guard said no. Nothing after the guard ran."""

    def __init__(self, message: str):
        super().__init__(f"REFUSED: {message}")
        self.message = message


def within(path: str, base: str) -> bool:
    """Component-wise: /opt/omoba-ru is NOT within /opt/omoba; /opt/omoba/x/../y is."""
    path, base = posixpath.normpath(path), posixpath.normpath(base)
    return path == base or path.startswith(base.rstrip("/") + "/")


def touches_foreign(path: str) -> bool:
    return any(within(path, other) for other in FOREIGN_PATHS)


def in_zone(name: str, zones) -> bool:
    return any(name == zone or name.endswith("." + zone) for zone in zones)


def is_foreign_name(value: str, own_zones=()) -> bool:
    """The production or dev VPS (alias or address) or an omoba.io / ekza.io name outside
    `own_zones`, the RU zone from scripts/markets.py (case and trailing dot ignored)."""
    name = (value or "").strip().lower().rstrip(".")
    if "@" in name:
        name = name.rsplit("@", 1)[1]
    return name in PROD_SSH_ALIASES or name in DEV_SSH_ALIASES or name in PROD_IPS or \
        (in_zone(name, PUBLIC_ZONES) and not in_zone(name, own_zones))


ALLOWED_WRITES = (OPT, STATE, BACKUPS, CADDYFILE, CADDYFILE + ".orig", CADDYFILE + ".omoba-next",
                  SSHD_DROPIN, *(f"{UNIT_DIR}/{name}" for name in (*UNIT_FILES.values(), *BACKUP_UNITS)))


def assert_ru_constants() -> None:
    """The RU names above must stay disjoint from production and dev even after an edit."""
    for path in (*ALLOWED_WRITES, PORTAL_CACHE, LOCK):
        if touches_foreign(path):
            raise Refused(f"RU path {path} is inside a production or dev path")
    if USER in FOREIGN_USERS or PORTAL_USER in FOREIGN_USERS or \
            any(name.rsplit(".", 1)[0] in FOREIGN_UNITS for name in (*UNIT_FILES.values(), *BACKUP_UNITS)):
        raise Refused("an RU user or unit equals a production or dev one")
    if is_foreign_name(DEFAULT_HOST):
        raise Refused("RU host defaults name another contour")


def names_problems(names) -> list[str]:
    """The RU public names (scripts/markets.py, carried in the payload) must be DNS names in
    the RU zone (scripts/markets.py), the zone must neither cover nor sit inside a production or dev
    zone (omoba.io / ekza.io), and the Account API and the portal need their own Caddy sites."""
    if not isinstance(names, dict) or set(names) != set(NAME_KEYS):
        return ["the payload lacks the RU names (scripts/markets.py)"]
    problems = [f"the RU {key} name {printable(names[key])} is not a lowercase DNS name" for key in NAME_KEYS
                if not isinstance(names[key], str) or not DNS_NAME.fullmatch(names[key])]
    if problems:
        return problems
    zone = names["zone"]
    if in_zone(zone, PUBLIC_ZONES) or any(in_zone(public, (zone,)) for public in PUBLIC_ZONES):
        problems.append(f"the RU zone {zone} overlaps the production or dev names "
                        f"({' / '.join(PUBLIC_ZONES)}); the RU market has its own zone")
    problems += [f"the RU {key} name {names[key]} is outside the RU zone {zone}"
                 for key in HOST_KEYS if not in_zone(names[key], (zone,))]
    if names["account_api"] == names["portal"]:
        problems.append(f"the RU Account API and portal share the name {names['portal']}; Caddy serves them apart")
    return problems


def ipv4_literal(value: str) -> str | None:
    try:
        return str(ipaddress.IPv4Address((value or "").strip()))
    except (ipaddress.AddressValueError, ValueError):
        return None


PRIVATE_V4 = tuple(ipaddress.IPv4Network(net) for net in
                   ("10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "100.64.0.0/10"))


def validate_ru_ip(value: str | None, dev_ip: str | None = None) -> str | None:
    """$RU_IP: the RU VPS's public IPv4; never the production or dev VPS."""
    if value is None or value.strip() == "":
        return None
    address = ipv4_literal(value)
    if address is None:
        raise Refused(f"{IP_ENV} must be an IPv4 address, got {value!r}")
    parsed = ipaddress.IPv4Address(address)
    if address in PROD_IPS:
        raise Refused(f"{address} is the production VPS; the RU tool only targets the RU VPS")
    if dev_ip and address == dev_ip:
        raise Refused(f"{address} is the dev VPS ({DEV_IP_ENV}); the RU tool only targets the RU VPS")
    if parsed.is_loopback or parsed.is_unspecified or parsed.is_multicast or parsed.is_link_local \
            or parsed.is_reserved or any(parsed in net for net in PRIVATE_V4):
        raise Refused(f"{IP_ENV} {address} is not a public host address")
    return address


def version_tuple(text: str) -> tuple:
    try:
        return tuple(int(part) for part in (text or "").strip().strip('"').split("."))
    except ValueError:
        return ()


def printable(value) -> str:
    """Only short, plain identifiers are echoed; anything else could be a pasted secret."""
    value = str(value) if value is not None else ""
    return value if re.fullmatch(r"[A-Za-z0-9._:\[\]-]{1,64}", value) else "(not shown)"


# ---------------------------------------------------------------- units

def parse_unit(text: str) -> tuple[dict, dict]:
    """(directives, environment) as lists, so duplicates are visible."""
    directives, environment = {}, {}
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


def unit_name(kind: str) -> str:
    return UNIT_FILES[kind][:-len(".service")]


def unit_path(kind: str) -> str:
    return f"{UNIT_DIR}/{UNIT_FILES[kind]}"


def arena_ports(lobby_unit: str) -> tuple[int, int]:
    """First and last UDP port of the match workers, from the lobby unit."""
    environment = parse_unit(lobby_unit)[1]
    try:
        first = int(environment["OMOBA_MATCH_FIRST_PORT"][0])
        capacity = int(environment["OMOBA_MATCH_CAPACITY"][0])
    except (KeyError, IndexError, ValueError):
        raise Refused("the lobby unit must set OMOBA_MATCH_FIRST_PORT and OMOBA_MATCH_CAPACITY") from None
    last = first + capacity - 1
    if not 1 <= capacity <= 64 or first < 1024 or last > 65535:
        raise Refused(f"match ports {first}..{last} (capacity {capacity}) are out of range")
    if first <= LOBBY_PORT <= last or first <= HTTPS_PORT <= last:
        raise Refused(f"match ports {first}..{last} overlap the lobby or HTTPS port")
    return first, last


def unit_problems(kind: str, text: str, own_zones=()) -> list[str]:
    """Every reason this is not the RU unit of `kind`; empty means it is. `own_zones`: the
    RU zone, whose names are not another contour's."""
    name = UNIT_FILES[kind]
    problems = []
    directives, environment = parse_unit(text or "")
    for key, expected in {**COMMON_UNIT_RULES, **UNIT_RULES[kind]}.items():
        values = directives.get(key, [])
        if values != [expected]:
            section, option = key.split(".", 1)
            problems.append(f"{name}: [{section}] {option} must be exactly {expected!r}, found {values}")
    if not directives.get("Service.MemoryMax"):
        problems.append(f"{name}: [Service] MemoryMax is missing")
    if kind in NEEDS_POSTGRES:
        for key in ("Unit.After", "Unit.Wants"):
            if "postgresql.service" not in " ".join(directives.get(key, [])).split():
                problems.append(f"{name}: [Unit] {key.split('.')[1]}= must include postgresql.service")
    for key, expected in UNIT_ENV_RULES[kind].items():
        if environment.get(key, []) != [expected]:
            problems.append(f"{name}: Environment {key} must be exactly {expected!r}, found {environment.get(key, [])}")
    for key in UNIT_FORBIDDEN_ENV:
        if key in environment:
            problems.append(f"{name}: Environment {key} belongs in {ENV_PATHS[kind]}, not in the unit")
    for key, values in directives.items():
        if len(values) > 1 and key not in MULTI_VALUED:
            problems.append(f"{name}: {key} appears {len(values)} times")
        for value in values:
            for token in re.findall(r"/[^\s\"']*", value):
                if touches_foreign(token):
                    problems.append(f"{name}: {key} references production/dev path {token}")
            for token in re.findall(r"[A-Za-z0-9][A-Za-z0-9.-]*", value):
                if is_foreign_name(token, own_zones):
                    problems.append(f"{name}: {key} references another contour's host {token}")
    if any(token in (text or "").lower() for token in FORBIDDEN_TOKENS):
        problems.append(f"{name}: mentions hosted Supabase or the production project")
    if kind == "lobby":
        try:
            arena_ports(text)
        except Refused as refusal:
            problems.append(f"{name}: {refusal.message}")
    return problems


def backup_unit_problems(name: str, text: str, own_zones=()) -> list[str]:
    """Every reason this is not the RU backup unit or timer `name` (LR-03)."""
    directives, _ = parse_unit(text or "")
    problems = []
    if name.endswith(".service"):
        rules = {"Service.Type": "oneshot", "Service.ExecStart": f"{PYTHON} {BACKUP_SCRIPT} {BACKUP_COMMANDS[name]}",
                 "Service.NoNewPrivileges": "true", "Service.ProtectSystem": "strict",
                 "Service.ProtectHome": "true", "Service.PrivateTmp": "true", "Service.ReadWritePaths": BACKUPS,
                 "Service.UMask": "0077"}
        if not directives.get("Service.TimeoutStartSec"):
            problems.append(f"{name}: [Service] TimeoutStartSec is missing")
        if any(key.startswith("Install.") for key in directives):
            problems.append(f"{name}: a timer starts it; it has no [Install] section")
        if name != "omoba-backup-check-ru.service":
            for key in ("Unit.After", "Unit.Wants"):
                if "postgresql.service" not in " ".join(directives.get(key, [])).split():
                    problems.append(f"{name}: [Unit] {key.split('.')[1]}= must include postgresql.service")
    else:
        rules = {"Install.WantedBy": "timers.target"}
        if len(directives.get("Timer.OnCalendar", [])) != 1:
            problems.append(f"{name}: [Timer] needs exactly one OnCalendar=")
        if name != "omoba-backup-check-ru.timer" and directives.get("Timer.Persistent") != ["true"]:
            problems.append(f"{name}: [Timer] Persistent must be true (a missed run happens at the next boot)")
    for key, expected in rules.items():
        if directives.get(key, []) != [expected]:
            section, option = key.split(".", 1)
            problems.append(f"{name}: [{section}] {option} must be exactly {expected!r}, found {directives.get(key, [])}")
    for key, values in directives.items():
        for value in values:
            for token in re.findall(r"/[^\s\"']*", value):
                if touches_foreign(token):
                    problems.append(f"{name}: {key} references production/dev path {token}")
            for token in re.findall(r"[A-Za-z0-9][A-Za-z0-9.-]*", value):
                if is_foreign_name(token, own_zones):
                    problems.append(f"{name}: {key} references another contour's host {token}")
    return problems


def backup_kit_problems(backup, own_zones=()) -> list[str]:
    if not isinstance(backup, dict) or not isinstance(backup.get("units"), dict) or \
            set(backup["units"]) != set(BACKUP_UNITS):
        return ["the backup kit lacks its units"]
    problems = [problem for name, text in backup["units"].items() for problem in backup_unit_problems(name, text, own_zones)]
    script = backup.get("script")
    if not isinstance(script, str) or not script.startswith("#!/usr/bin/env python3\n") or "LR-03" not in script:
        problems.append("the backup kit lacks ru_backup.py")
    example = backup.get("env_example")
    if not isinstance(example, str) or "\nOMOBA_BACKUP_TARGET=" + PLACEHOLDER not in example:
        problems.append(f"the backup kit lacks {BACKUP_ENV_EXAMPLE} with OMOBA_BACKUP_TARGET={PLACEHOLDER}")
    return problems


def encode_backup_kit(backup: dict) -> str:
    return base64.b64encode(zlib.compress(json.dumps(backup, sort_keys=True).encode(), 9)).decode()


def decode_backup_kit(raw) -> dict:
    if not isinstance(raw, str) or not raw or not PAYLOAD.fullmatch(raw):
        raise Refused("payload lacks the backup kit (ops/vps/ru/ru_backup.py and its units)")
    inflater = zlib.decompressobj()
    data = inflater.decompress(base64.b64decode(raw), 1 << 21)
    if inflater.unconsumed_tail:
        raise Refused("the backup kit in the payload is too large")
    return json.loads(data)


# ---------------------------------------------------------------- env files

def parse_env(text: str) -> tuple[dict, list]:
    """(values, problems) of a systemd EnvironmentFile; duplicates are a problem."""
    values, problems = {}, []
    for number, raw in enumerate((text or "").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith(("#", ";")):
            continue
        key, sep, value = line.partition("=")
        key = key.strip()
        if not sep or not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", key):
            problems.append(f"line {number} is not KEY=value")
            continue
        if key in values:
            problems.append(f"{key} appears more than once")
            continue
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in "'\"":
            value = value[1:-1]
        values[key] = value
    return values, problems


def fill_env(text: str, fill: dict) -> str:
    """`text` with each placeholder value named in `fill` replaced; comments and order kept."""
    lines = []
    for raw in text.splitlines():
        stripped = raw.strip()
        key, sep, value = stripped.partition("=")
        if sep and not stripped.startswith(("#", ";")) and key.strip() in fill and PLACEHOLDER in value:
            raw = f"{key.strip()}={fill[key.strip()]}"
        lines.append(raw)
    return "\n".join(lines) + "\n"


def with_password(url: str, password: str) -> str:
    parts = urlsplit(url)
    netloc = f"{parts.username}:{password}@{parts.hostname}" + (f":{parts.port}" if parts.port else "")
    return urlunsplit((parts.scheme, netloc, parts.path, parts.query, parts.fragment))


def database_url(user: str, password: str) -> str:
    return f"postgres://{user}:{password}@{DB_HOST}:{DB_PORT}/{DB_NAME}"


def database_url_problems(url: str, role: str) -> list[str]:
    """Why `url` is not the loopback role@omoba_ru URL with a generated password; never echoes it."""
    lowered = (url or "").lower()
    problems = []
    if any(token in lowered for token in FORBIDDEN_TOKENS) or any(ip in lowered for ip in PROD_IPS):
        problems.append("names hosted Supabase, the production project or the production VPS")
    try:
        parts = urlsplit(url or "")
        port = parts.port
    except ValueError:
        return problems + ["is not a postgres URL with a role, a password, a port and a database"]
    if parts.scheme not in ("postgres", "postgresql"):
        problems.append("is not a postgres:// URL")
    if (parts.hostname or "").lower() not in LOOPBACK:
        problems.append(f"host {printable(parts.hostname)} is not loopback; PostgreSQL runs on this host")
    if port != DB_PORT:
        problems.append(f"port {port} is not {DB_PORT}")
    if unquote(parts.username or "") != role:
        problems.append(f"user {printable(unquote(parts.username or ''))} is not {role}")
    if not DB_PASSWORD.fullmatch(unquote(parts.password or "")):
        problems.append("password must be 24-128 characters of [A-Za-z0-9_-]")
    if unquote(parts.path.lstrip("/")) != DB_NAME:
        problems.append(f"database {printable(unquote(parts.path.lstrip('/')))} is not {DB_NAME}")
    if parts.query or parts.fragment or any(ch.isspace() for ch in url or ""):
        problems.append("must not carry query parameters, a fragment or spaces")
    return problems


def placeholder_problems(key: str, value: str, example: str, expected_ip: str | None) -> list[str]:
    """A filled placeholder of the example must hold a valid generated or supplied value."""
    if key == "OMOBA_DATABASE_URL":
        return database_url_problems(value, unquote(urlsplit(example).username or ""))
    if key.endswith("_SECRET"):
        return [] if HEX64.fullmatch(value) else ["must be 64 lowercase hex characters (openssl rand -hex 32)"]
    if key == "OMOBA_VKID_CLIENT_ID":
        return [] if VKID.fullmatch(value) else ["must be the numeric VK ID app id"]
    if key == "OMOBA_MATCH_PUBLIC_HOST":
        if ipv4_literal(value) is None:
            return ["must be this host's public IPv4"]
        return [f"is {value}, not {IP_ENV} {expected_ip}"] if expected_ip and value != expected_ip else []
    return [] if value else ["is empty"]


def env_problems(kind: str, text: str, example: str, expected_ip: str | None = None,
                 unit: str | None = None) -> tuple[list, list]:
    """(problems, warnings) of an RU env file against its example: every fixed value of the
    example unchanged, every placeholder filled with a valid value, nothing the unit sets."""
    values, problems = parse_env(text)
    wanted, _ = parse_env(example)
    warnings = []
    for key, value in wanted.items():
        actual = values.get(key)
        if actual is None:
            problems.append(f"{key} is missing")
        elif PLACEHOLDER in actual:
            problems.append(f"{key} is still a placeholder"
                            + (" (needs the owner's VK ID app, LR-19)" if key == "OMOBA_VKID_CLIENT_ID" else ""))
        elif PLACEHOLDER not in value:
            if actual != value:
                problems.append(f"{key} must be {value!r} as in {ENV_EXAMPLES[kind]}")
        else:
            problems += [f"{key} {problem}" for problem in placeholder_problems(key, actual, value, expected_ip)]
    if unit is not None:
        unit_env = parse_unit(unit)[1]
        problems += [f"{key} is set by {UNIT_FILES[kind]}; an EnvironmentFile would override it"
                     for key in values if key in unit_env]
    if any(token in (text or "").lower() for token in FORBIDDEN_TOKENS):
        problems.append("mentions hosted Supabase or the production project")
    warnings += [f"{key} is not in {ENV_EXAMPLES[kind]}" for key in values if key not in wanted]
    return problems, warnings


def placeholder_fill(values: dict, expected_ip: str | None, vkid: str | None) -> dict:
    """Values for the placeholders bootstrap can fill: generated passwords and secrets,
    the VK ID app id when given, this host's address. Never for a value already set."""
    fill = {}
    for key, value in values.items():
        if PLACEHOLDER not in value:
            continue
        if key == "OMOBA_DATABASE_URL":
            fill[key] = with_password(value, secrets.token_hex(32))
        elif key.endswith("_SECRET"):
            fill[key] = secrets.token_hex(32)
        elif key == "OMOBA_VKID_CLIENT_ID" and vkid:
            fill[key] = vkid
        elif key == "OMOBA_MATCH_PUBLIC_HOST" and expected_ip:
            fill[key] = expected_ip
    return fill


def example_problems(kind: str, text: str) -> list[str]:
    """The repository examples are the contract the host's env files are checked against."""
    values, problems = parse_env(text)
    role = {"lobby": DB_GAME, "account-api": DB_PORTAL}.get(kind)
    if role:
        url = values.get("OMOBA_DATABASE_URL", "")
        filled = with_password(url, "x" * 32) if url else ""
        problems += [f"OMOBA_DATABASE_URL {problem}" for problem in database_url_problems(filled, role)]
    if kind == "lobby" and PLACEHOLDER not in values.get("OMOBA_MATCH_PUBLIC_HOST", ""):
        problems.append("OMOBA_MATCH_PUBLIC_HOST must be a placeholder (bootstrap fills in $RU_IP)")
    if kind == "portal" and values.get("PORT") != str(PORTAL_ADDR[1]):
        problems.append(f"PORT must be {PORTAL_ADDR[1]}")
    for key, value in values.items():
        if (key.endswith("_SECRET") or key == "OMOBA_VKID_CLIENT_ID") and PLACEHOLDER not in value:
            problems.append(f"{key} must stay a placeholder in the example")
    return [f"{ENV_EXAMPLES[kind]}: {problem}" for problem in problems]


# ---------------------------------------------------------------- firewall and SSH

def firewall_rules(lobby_unit: str, ssh_ports) -> list[tuple[str, str, str]]:
    """(ufw action, port, comment) of every inbound rule: SSH, HTTPS, the lobby, the arenas.
    SSH is `allow`, not `limit`: this tool opens several connections in a few seconds."""
    first, last = arena_ports(lobby_unit)
    arenas = f"{first}/udp" if first == last else f"{first}:{last}/udp"
    rules = [("allow", f"{port}/tcp", "omoba-ru ssh (key-only)") for port in sorted(ssh_ports)]
    return rules + [("allow", f"{HTTPS_PORT}/tcp", "omoba-ru https (Caddy)"),
                    ("allow", f"{LOBBY_PORT}/udp", "omoba-ru lobby"),
                    ("allow", arenas, "omoba-ru match workers")]


UFW_RULE = re.compile(r"^(?P<to>\S+(?: \(v6\))?(?: on \S+)?)\s+(?P<action>ALLOW|DENY|REJECT|LIMIT)"
                      r"(?: (?P<direction>IN|OUT|FWD))?\s+(?P<source>\S+(?: \(v6\))?(?: on \S+)?)"
                      r"(?:\s+#\s*(?P<comment>.*?))?\s*$")


def parse_ufw(text: str) -> dict:
    """`ufw status verbose` as {active, incoming, rules: [{to, action, direction, source, v6, comment}]}."""
    status = {"active": False, "incoming": None, "rules": []}
    for line in (text or "").splitlines():
        if line.startswith("Status:"):
            status["active"] = line.split(":", 1)[1].strip() == "active"
        elif line.startswith("Default:"):
            found = re.search(r"(\w+) \(incoming\)", line)
            status["incoming"] = found.group(1) if found else None
        else:
            found = UFW_RULE.match(line.strip())
            if found and found.group("to") not in ("To", "--"):
                to = found.group("to")
                status["rules"].append({"to": to.replace(" (v6)", ""), "v6": "(v6)" in to,
                                        "action": found.group("action"),
                                        "direction": found.group("direction") or "IN",
                                        "source": found.group("source").replace(" (v6)", ""),
                                        "comment": found.group("comment") or ""})
    return status


def firewall_problems(status: dict, rules) -> list[str]:
    """Inbound open for exactly `rules`, from anywhere, everything else denied."""
    problems = []
    if not status["active"]:
        return ["ufw is not active: the host accepts every inbound port"]
    if status["incoming"] != "deny":
        problems.append(f"ufw default incoming policy is {status['incoming']!r}, not deny")
    expected = {spec for _, spec, _ in rules}
    open_rules = [rule for rule in status["rules"] if rule["action"] in ("ALLOW", "LIMIT")
                  and rule["direction"] == "IN"]
    for _, spec, _ in rules:
        if not any(rule["to"] == spec and not rule["v6"] and rule["source"] == "Anywhere"
                   for rule in open_rules):
            problems.append(f"ufw does not allow {spec} from anywhere")
    for to in sorted({rule["to"] for rule in open_rules if rule["to"] not in expected}):
        problems.append(f"ufw opens {to}, which the RU host does not use: ufw delete allow {to}")
    return problems


def sshd_problems(effective: str) -> list[str]:
    """`sshd -T` must show key-only authentication."""
    values = {}
    for line in (effective or "").splitlines():
        key, _, value = line.strip().partition(" ")
        values.setdefault(key.lower(), value.strip().lower())
    return [f"sshd {key} is {values.get(key, 'unset')!r}, must be {' or '.join(allowed)}"
            for key, allowed in SSHD_REQUIRED.items() if values.get(key) not in allowed]


def ssh_ports(effective: str) -> set[int]:
    ports = {int(value) for key, _, value in (line.strip().partition(" ") for line in (effective or "").splitlines())
             if key.lower() == "port" and value.strip().isdigit()}
    return ports or {22}


def dns_problems(resolve, expected_ip: str, names: dict) -> tuple[list, list]:
    """The RU names (scripts/markets.py, from the payload) must resolve to this host only (A);
    the lobby name must have no AAAA, since the lobby binds IPv4 and clients would try IPv6
    first."""
    problems, warnings = [], []
    for name in dict.fromkeys(names[key] for key in HOST_KEYS):
        v4 = resolve(name, socket.AF_INET)
        v6 = {address for address in resolve(name, socket.AF_INET6) if not address.lower().startswith("::ffff:")}
        if not v4:
            problems.append(f"{name} has no A record; point it at {expected_ip}")
        elif v4 != {expected_ip}:
            problems.append(f"{name} resolves to {sorted(v4)}, not {expected_ip}")
        if v6 and name == names["lobby"]:
            problems.append(f"{name} has AAAA records {sorted(v6)}; the lobby binds IPv4 only: remove them")
        elif v6:
            warnings.append(f"{name} has AAAA records {sorted(v6)}; they must reach this host's Caddy")
    return problems, warnings


# ---------------------------------------------------------------- PostgreSQL

STATE_SQL = f"""-- ru_host:state
SELECT json_build_object(
  'version', current_setting('server_version_num')::int,
  'icu', EXISTS (SELECT 1 FROM pg_collation WHERE collname = '{ICU_COLLATION}'),
  'listen', current_setting('listen_addresses'),
  'port', current_setting('port')::int,
  'roles', (SELECT coalesce(json_object_agg(rolname, rolcanlogin), '{{}}'::json) FROM pg_roles
            WHERE rolname IN ('{DB_OWNER}', '{DB_GAME}', '{DB_PORTAL}')),
  'database_owner', (SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname = '{DB_NAME}'));
"""
SCHEMA_SQL = """-- ru_host:schema
SELECT to_regclass('public.career_schema_version') IS NOT NULL
   AND to_regclass('portal.schema_version') IS NOT NULL;
"""
OWNER_NOLOGIN_SQL = f"""-- ru_host:owner-nologin
ALTER ROLE {DB_OWNER} NOLOGIN PASSWORD NULL VALID UNTIL 'infinity';
"""


def sql_secret(value: str) -> str:
    if not DB_PASSWORD.fullmatch(value or ""):
        raise Refused("internal: a database password with unexpected characters")
    return f"'{value}'"


def bootstrap_sql(game_password: str, portal_password: str) -> str:
    """Roles and database when missing (idempotent); runtime passwords from the env files.
    Collation C keeps indexes stable across glibc upgrades; handles use und-x-icu explicitly."""
    roles = ", ".join(f"'{role}'" for role in DB_ROLES)
    return f"""-- ru_host:bootstrap
SELECT format('CREATE ROLE %I NOLOGIN', r) FROM unnest(ARRAY[{roles}]) AS r
 WHERE NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = r) \\gexec
SELECT 'CREATE DATABASE {DB_NAME} OWNER {DB_OWNER} TEMPLATE template0 ENCODING ''UTF8'' LOCALE ''C.UTF-8'''
 WHERE NOT EXISTS (SELECT 1 FROM pg_database WHERE datname = '{DB_NAME}') \\gexec
REVOKE ALL ON DATABASE {DB_NAME} FROM PUBLIC;
GRANT CONNECT ON DATABASE {DB_NAME} TO {DB_GAME}, {DB_PORTAL};
ALTER ROLE {DB_OWNER} NOLOGIN;
ALTER ROLE {DB_GAME} LOGIN PASSWORD {sql_secret(game_password)};
ALTER ROLE {DB_PORTAL} LOGIN PASSWORD {sql_secret(portal_password)};
"""


def owner_login_sql(password: str, until: str) -> str:
    if not re.fullmatch(r"\d{4}-\d\d-\d\d \d\d:\d\d:\d\d\+00", until):
        raise Refused("internal: unexpected VALID UNTIL timestamp")
    return f"""-- ru_host:owner-login
ALTER ROLE {DB_OWNER} LOGIN PASSWORD {sql_secret(password)} VALID UNTIL '{until}';
"""


def grants_sql(grants: str) -> str:
    """account-api/ops/grants.sql run as the schema owner, as on every contour."""
    return (f"-- ru_host:grants\nSET ROLE {DB_OWNER};\n\\set portal_role {DB_PORTAL}\n"
            f"\\set game_role {DB_GAME}\n{grants}\nRESET ROLE;\n")


def validate_grants(grants) -> str:
    if not isinstance(grants, str) or not 0 < len(grants) < 65536 or ':"game_role"' not in grants:
        raise Refused(f"payload lacks {GRANTS}")
    return grants


def postgres_problems(state: dict | None) -> list[str]:
    if state is None:
        return ["PostgreSQL does not answer the local superuser (runuser -u postgres -- psql)"]
    problems = []
    if int(state.get("version") or 0) < MIN_POSTGRES * 10000:
        problems.append(f"PostgreSQL {state.get('version')} is older than {MIN_POSTGRES}")
    if not state.get("icu"):
        problems.append(f'PostgreSQL has no ICU collation "{ICU_COLLATION}" (docs/player-handles.md)')
    listen = [address.strip() for address in str(state.get("listen") or "").split(",") if address.strip()]
    if not listen or any(address not in LOOPBACK for address in listen):
        problems.append(f"PostgreSQL listen_addresses is {printable(state.get('listen'))}; it must be loopback only")
    if state.get("port") != DB_PORT:
        problems.append(f"PostgreSQL listens on port {state.get('port')}, not {DB_PORT}")
    owner = state.get("database_owner")
    if owner not in (None, DB_OWNER):
        problems.append(f"database {DB_NAME} exists and is owned by {printable(owner)}, not {DB_OWNER}")
    return problems


def redact(text: str, hidden=()) -> str:
    for value in hidden:
        if value:
            text = text.replace(value, "<redacted>")
    text = re.sub(r"(?i)(password\s+)'[^']*'", r"\1'<redacted>'", text)
    return re.sub(r"://([^:/@\s]+):[^@\s]*@", r"://\1:<redacted>@", text)


# ---------------------------------------------------------------- releases, bundles, archives

def is_linux_x86_64_elf(head: bytes) -> bool:
    # ELF magic, 64-bit, little-endian, e_machine 0x3e (x86-64).
    return len(head) >= 20 and head[:4] == b"\x7fELF" and head[4] == 2 and head[5] == 1 \
        and head[18:20] == b"\x3e\x00"


def sha256_file(path: str) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def validate_server_release(name: str) -> str:
    if not SERVER_RELEASE.fullmatch(name or ""):
        raise Refused(f"release name must look like <version>-ru-<sha7>, got {name!r}")
    return name


def validate_portal_release(name: str) -> str:
    if not PORTAL_RELEASE.fullmatch(name or ""):
        raise Refused(f"portal release name must look like <rev7>-<sha8>, got {name!r}")
    return name


def server_files(payload: dict) -> dict:
    files = {"omoba-server": payload.get("sha256"), "omoba-account-api": payload.get("account_api_sha256")}
    for name, digest in files.items():
        if not (isinstance(digest, str) and HEX64.fullmatch(digest)):
            raise Refused(f"payload lacks the sha256 of {name}")
    return files


def portal_problems(paths: dict, read) -> tuple[list, list]:
    """Is this the RU portal's standalone build, ready to run? `paths` maps each relative
    path to 'file', 'dir' or 'link'; read(path, size) returns its first bytes. Used on
    the Mac (archive members) and on the host (the unpacked release)."""
    problems, warnings = [], []
    if paths.get("server.js") != "file":
        return ["server.js is missing: not a Next.js standalone build"], warnings
    text = read("server.js", 16 << 20).decode("utf-8", "replace")
    market = re.search(r'"OMOBA_BUILT_MARKET":"([^"]*)"', text)
    contour = re.search(r'"OMOBA_BUILT_CONTOUR":"([^"]*)"', text)
    if not market or not re.fullmatch(r"ru(?: \(.*\))?", market.group(1)):
        problems.append("not built with OMOBA_MARKET=ru (server.js OMOBA_BUILT_MARKET is "
                        f"{printable(market.group(1)) if market else 'missing'})")
    elif "ekza on" in market.group(1):
        problems.append("built with the ekza feature on; the RU portal never shows Ekza")
    if not contour or contour.group(1) != "production":
        problems.append("not a production-contour build (OMOBA_CONTOUR must be unset or production)")
    for required, kind in ((".next/BUILD_ID", "file"), (".next/static", "dir"), ("node_modules", "dir")):
        if paths.get(required) != kind:
            problems.append(f"{required} is missing (copy .next/static next to server.js; see README.md)")
    for name in sorted(path for path in paths if "/" not in path and path.startswith(".env")):
        problems.append(f"{name} would ship build-time env (possibly secrets) to the host")
    for path, kind in sorted(paths.items()):
        if kind == "file" and path.endswith(".node"):
            head = read(path, 64)
            if not is_linux_x86_64_elf(head):
                warnings.append(f"{path} is a native addon for another platform; build the portal "
                                "on linux/amd64 if that package is needed")
    return problems, warnings


def tree_listing(root: str) -> dict:
    found = {}
    for directory, dirs, files in os.walk(root):
        for name in dirs + files:
            path = os.path.join(directory, name)
            relative = os.path.relpath(path, root)
            found[relative] = "link" if os.path.islink(path) else ("dir" if os.path.isdir(path) else "file")
    return found


def read_head(root: str):
    def read(relative: str, size: int) -> bytes:
        with open(os.path.join(root, relative), "rb") as handle:
            return handle.read(size)
    return read


def _renamed(member: tarfile.TarInfo, **changes) -> tarfile.TarInfo:
    """A renamed copy where TarInfo.replace exists (Python 3.12+, the host); in place on an
    older Mac Python, which only inspects archives."""
    if hasattr(member, "replace"):
        return member.replace(deep=False, **changes)
    for key, value in changes.items():
        setattr(member, key, value)
    return member


def archive_members(archive: tarfile.TarFile, top: str) -> list:
    """Members below `top/`, renamed relative to it; anything else is refused."""
    members = []
    for member in archive.getmembers():
        name = member.name[2:] if member.name.startswith("./") else member.name
        name = name.rstrip("/")
        if name == top:
            continue
        if not name.startswith(top + "/") or ".." in name.split("/"):
            raise Refused(f"archive member {member.name!r} is outside {top}/")
        if member.isdev() or member.isfifo():
            raise Refused(f"archive member {member.name!r} is a device or fifo")
        changes = {"name": name[len(top) + 1:]}
        if member.islnk():
            if not member.linkname.startswith(top + "/"):
                raise Refused(f"hard link {member.name!r} points outside {top}/")
            changes["linkname"] = member.linkname[len(top) + 1:]
        members.append(_renamed(member, **changes))
    return members


def make_readable(root: str) -> None:
    """Folders 0755 and files world-readable, so the service users can read a release that
    root unpacked (the data filter already drops group/other write and setuid bits)."""
    for directory, dirs, files in os.walk(root):
        os.chmod(directory, 0o755)
        for name in files:
            path = os.path.join(directory, name)
            if not os.path.islink(path):
                mode = os.stat(path).st_mode
                os.chmod(path, (mode & 0o755) | 0o444 | (0o111 if mode & 0o100 else 0))


def tar_normalized(info: tarfile.TarInfo) -> tarfile.TarInfo:
    """Bundle members owned by root, readable by everyone, never setuid or group-writable."""
    info.uid = info.gid = 0
    info.uname = info.gname = "root"
    executable = info.isdir() or info.mode & 0o111
    info.mode = 0o755 if executable else 0o644
    return info


def safe_extract(archive_path: str, destination: str, top: str) -> None:
    """Unpack `top/` of a tar archive into the new folder `destination` with the tarfile data
    filter: no absolute paths, no '..', no links out, no devices, no setuid bits, no owners."""
    if not hasattr(tarfile, "data_filter"):
        raise Refused("this Python has no tarfile data filter; the RU host runs Ubuntu 24.04+")
    os.makedirs(destination)
    try:
        with tarfile.open(archive_path, "r:*") as archive:
            archive.extractall(destination, members=archive_members(archive, top), filter="data")
        make_readable(destination)
    except (tarfile.TarError, OSError) as error:
        shutil.rmtree(destination, ignore_errors=True)
        raise Refused(f"cannot unpack {posixpath.basename(archive_path)}: {error}") from None
    except Refused:
        shutil.rmtree(destination, ignore_errors=True)
        raise


# ---------------------------------------------------------------- host half

def _resolve(host: str, family: int) -> set:
    try:
        return {info[4][0] for info in socket.getaddrinfo(host, None, family, socket.SOCK_DGRAM)}
    except (socket.gaierror, UnicodeError):
        return set()


def _fetch(url: str, timeout: float = 5) -> int:
    """HTTP status of a GET to a loopback service (0: no answer); never through a proxy."""
    import urllib.error
    import urllib.request
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    try:
        with opener.open(urllib.request.Request(url, method="GET"), timeout=timeout) as response:
            return response.status
    except urllib.error.HTTPError as error:
        return error.code
    except (OSError, ValueError):
        return 0


class Host:
    """The host side. `root`, `run`, `resolve` and `fetch` exist so tests can use a fake
    filesystem, fake commands, fake DNS and fake HTTP; the remote entry point always uses
    root "/", real commands, the host's resolver and real loopback HTTP."""

    def __init__(self, root: str = "/", run=None, sleep=time.sleep, resolve=_resolve, fetch=_fetch,
                 root_uid: int = 0, now=time.time):
        self.root = root
        self.run = run or self._run
        self.sleep = sleep
        self.resolve = resolve
        self.fetch = fetch
        self.root_uid = root_uid  # owner the env files must have; tests use their own uid
        self.now = now
        self.mutating = False
        assert_ru_constants()

    @staticmethod
    def _run(args, check=True, input=None, env=None, timeout=120):
        try:
            result = subprocess.run(args, capture_output=True, text=True, timeout=timeout, input=input, env=env)
        except FileNotFoundError:
            result = subprocess.CompletedProcess(args, 127, "", f"{args[0]}: not found")
        if check and result.returncode:
            raise SystemExit(f"{' '.join(args)} failed: {result.stderr.strip() or result.stdout.strip()}")
        return result

    def p(self, path: str) -> str:
        return os.path.join(self.root, path.lstrip("/"))

    def read(self, path: str) -> str | None:
        local = self.p(path)
        return Path(local).read_text() if os.path.isfile(local) and not os.path.islink(local) else None

    def write_guard(self, path: str) -> None:
        if not self.mutating:
            raise Refused(f"internal: write to {path} in a read-only mode")
        if touches_foreign(path) or not any(within(path, allowed) for allowed in ALLOWED_WRITES):
            raise Refused(f"internal: write to {path} outside the RU host's own paths")

    def out(self, *args) -> str:
        return self.run(list(args), check=False).stdout.strip()

    def command(self, *args, timeout=120, env=None) -> None:
        if not self.mutating:
            raise Refused(f"internal: {args[0]} in a read-only mode")
        self.run(list(args), timeout=timeout, env=env)

    # ---- read-only facts

    def show(self, unit: str, prop: str) -> str:
        return self.out("systemctl", "show", unit, "-p", prop, "--value")

    def active(self, unit: str) -> bool:
        return self.run(["systemctl", "is-active", "--quiet", unit], check=False).returncode == 0

    def enabled(self, unit: str) -> bool:
        return self.out("systemctl", "is-enabled", unit) == "enabled"

    def main_pid(self, unit: str) -> int:
        try:
            return int(self.show(unit, "MainPID") or 0)
        except ValueError:
            return 0

    def exe(self, pid: int) -> str:
        return os.path.realpath(self.p(f"/proc/{pid}/exe"))

    def passwd(self, user: str) -> list | None:
        result = self.run(["getent", "passwd", user], check=False)
        return result.stdout.strip().split(":") if result.returncode == 0 else None

    def uid(self, user: str) -> int | None:
        result = self.run(["id", "-u", user], check=False)
        return int(result.stdout.strip()) if result.returncode == 0 else None

    def installed(self, package: str) -> bool:
        result = self.run(["dpkg-query", "-W", "--showformat=${Status}", package], check=False)
        return result.returncode == 0 and result.stdout.strip().endswith("install ok installed")

    def missing_packages(self) -> list:
        return [package for package in PACKAGES if not self.installed(package)]

    def local_ipv4(self) -> set:
        addresses = set()
        for line in self.out("ip", "-o", "-4", "addr", "show").splitlines():
            found = re.search(r"\binet (\d+\.\d+\.\d+\.\d+)/", line)
            if found:
                addresses.add(found.group(1))
        return addresses

    def listeners(self, protocol: str) -> set:
        """{(address, port)} of listening sockets, protocol 'udp' or 'tcp'."""
        found = set()
        for line in self.out("ss", "-Hl" + protocol[0] + "n").splitlines():
            fields = line.split()
            if len(fields) >= 4 and ":" in fields[3]:
                address, _, port = fields[3].rpartition(":")
                if port.isdigit():
                    found.add((address.strip("[]").split("%")[0], int(port)))
        return found

    def os_release(self) -> dict:
        values = {}
        for line in (self.read("/etc/os-release") or "").splitlines():
            key, sep, value = line.partition("=")
            if sep:
                values[key.strip()] = value.strip().strip('"')
        return values

    def platform_problems(self) -> list:
        system = self.os_release()
        problems = []
        if system.get("ID") != "ubuntu" or version_tuple(system.get("VERSION_ID", "")) < MIN_UBUNTU:
            problems.append(f"the RU host must run Ubuntu {MIN_UBUNTU[0]}.{MIN_UBUNTU[1]:02d} or newer, found "
                            f"{printable(system.get('ID'))} {printable(system.get('VERSION_ID'))}")
        arch = self.out("uname", "-m")
        if arch != "x86_64":
            problems.append(f"the RU packages are x86-64; this host is {printable(arch)}")
        return problems

    def foreign_markers(self, dev_ip: str | None = None) -> list:
        """Anything that says this machine is the production or the dev VPS. Read-only."""
        markers = [path for path in FOREIGN_PATHS if os.path.lexists(self.p(path))]
        markers += [f"systemd unit {unit}" for unit in FOREIGN_UNITS if self.show(unit, "LoadState") == "loaded"]
        markers += [f"user {user}" for user in FOREIGN_USERS if self.passwd(user) is not None]
        foreign_ips = PROD_IPS | ({dev_ip} if dev_ip else set())
        return markers + [f"address {ip}" for ip in sorted(self.local_ipv4() & foreign_ips)]

    def marker(self) -> dict | None:
        text = self.read(MARKER)
        if text is None:
            return None
        try:
            marker = json.loads(text)
        except ValueError:
            raise Refused(f"{MARKER} is not JSON") from None
        if not isinstance(marker, dict):
            raise Refused(f"{MARKER} is not a JSON object")
        return marker

    def identity_problems(self, payload: dict, require_marker: bool = True) -> list:
        problems = []
        markers = self.foreign_markers(payload.get("dev_ip"))
        if markers:
            problems.append("this host belongs to another contour (" + ", ".join(markers)
                            + "); the RU tool only runs on the RU VPS")
        problems += self.platform_problems()
        expected = payload.get("expected_ip")
        if not expected:
            problems.append("internal: the payload names no RU address")
        else:
            local = self.local_ipv4()
            if expected not in local and not payload.get("behind_nat"):
                problems.append(f"{expected} is not on a local interface ({sorted(local)}); check {IP_ENV}, "
                                "or pass --behind-nat if the provider uses NAT")
        try:
            marker = self.marker()
        except Refused as refusal:
            return problems + [refusal.message]
        if marker is None:
            if require_marker:
                problems.append(f"{MARKER} is missing: run `ru_host.py bootstrap --apply` first")
        elif marker.get("market") != MARKET or marker.get("ip") != expected:
            problems.append(f"{MARKER} names market {printable(marker.get('market'))} at "
                            f"{printable(marker.get('ip'))}, not {MARKET} at {expected}")
        return problems

    def guard_identity(self, payload: dict, require_marker: bool = True) -> None:
        problems = self.identity_problems(payload, require_marker)
        if problems:
            raise Refused("; ".join(problems))

    def payload_names(self, payload: dict) -> dict:
        """The RU public names the Mac half read from scripts/markets.py."""
        problems = names_problems(payload.get("names"))
        if problems:
            raise Refused("; ".join(problems))
        return payload["names"]

    def payload_units(self, payload: dict) -> dict:
        units = payload.get("units") or {}
        own_zones = (self.payload_names(payload)["zone"],)
        for kind, text in units.items():
            if kind not in KINDS:
                raise Refused(f"unknown unit kind {kind!r}")
            problems = unit_problems(kind, text, own_zones)
            if problems:
                raise Refused("unit is not the RU unit: " + "; ".join(problems))
        return units

    def payload_backup_kit(self, payload: dict) -> dict:
        """ru_backup.py, its units and backup.env.example, as the Mac half read them (LR-03)."""
        backup = decode_backup_kit(payload.get("backup_kit_z"))
        problems = backup_kit_problems(backup, (self.payload_names(payload)["zone"],))
        if problems:
            raise Refused("the backup kit is not the RU one: " + "; ".join(problems))
        return backup

    def payload_examples(self, payload: dict) -> dict:
        examples = payload.get("examples") or {}
        if set(examples) != set(KINDS):
            raise Refused("payload lacks the env examples")
        problems = [problem for kind, text in examples.items() for problem in example_problems(kind, text)]
        if problems:
            raise Refused("; ".join(problems))
        return examples

    def ssh_effective(self) -> str:
        return self.out("sshd", "-T")

    def authorized_keys(self) -> int:
        text = self.read(AUTHORIZED_KEYS) or ""
        return sum(1 for line in text.splitlines() if line.strip() and not line.lstrip().startswith("#"))

    def node_version(self, binary: str = NODE_CURRENT + "/bin/node") -> tuple | None:
        local = self.p(binary)
        if not os.path.exists(local):
            return None
        result = self.run([local, "--version"], check=False)
        found = re.fullmatch(r"v(\d+)\.(\d+)\.(\d+)", result.stdout.strip()) if result.returncode == 0 else None
        return tuple(int(part) for part in found.groups()) if found else None

    def distro_node(self) -> tuple | None:
        found = re.search(r"Candidate:\s*(?:\d+:)?(\d+)\.(\d+)", self.out("apt-cache", "policy", "nodejs"))
        return (int(found.group(1)), int(found.group(2))) if found else None

    def current(self, link: str, base: str) -> str | None:
        local = self.p(link)
        if not os.path.lexists(local):
            return None
        if not os.path.islink(local):
            raise Refused(f"{link} exists but is not a symlink")
        target = os.path.realpath(local)
        if os.path.dirname(target) != os.path.realpath(self.p(base)):
            raise Refused(f"{link} points outside {base}: {os.readlink(local)}")
        return os.path.basename(target)

    def release_present(self, kind: str) -> bool:
        if kind in SERVER_KINDS:
            return os.path.isfile(self.p(f"{CURRENT}/{'omoba-server' if kind == 'lobby' else 'omoba-account-api'}"))
        return os.path.isfile(self.p(PORTAL_CURRENT + "/server.js")) and \
            os.path.exists(self.p(NODE_CURRENT + "/bin/node"))

    def env_text(self, kind: str) -> str | None:
        return self.read(ENV_PATHS[kind])

    def env_file_report(self, kind: str, payload: dict) -> tuple[list, list]:
        path = ENV_PATHS[kind]
        local = self.p(path)
        if os.path.islink(local):
            return [f"{path} is a symlink"], []
        if not os.path.lexists(local):
            return [f"{path} is missing (bootstrap writes it)"], []
        info = os.stat(local)
        problems = []
        if info.st_uid != self.root_uid:
            problems.append(f"{path} is not owned by root")
        if info.st_mode & 0o077:
            problems.append(f"{path} has mode {oct(info.st_mode & 0o777)}; it must be 0600")
        found, warnings = env_problems(kind, self.env_text(kind) or "", payload["examples"][kind],
                                       payload.get("expected_ip"), (payload.get("units") or {}).get(kind))
        return problems + [f"{path}: {problem}" for problem in found], [f"{path}: {w}" for w in warnings]

    def env_report(self, payload: dict) -> tuple[list, list]:
        problems, warnings = [], []
        for kind in KINDS:
            more, notes = self.env_file_report(kind, payload)
            problems += more
            warnings += notes
        ids = {parse_env(self.env_text(kind) or "")[0].get("OMOBA_VKID_CLIENT_ID") for kind in ("account-api", "portal")}
        if len(ids) > 1:
            problems.append("OMOBA_VKID_CLIENT_ID differs between account-api.env and portal.env")
        return problems, warnings

    def runtime_passwords(self) -> dict:
        passwords = {}
        for kind, role in (("lobby", DB_GAME), ("account-api", DB_PORTAL)):
            url = parse_env(self.env_text(kind) or "")[0].get("OMOBA_DATABASE_URL", "")
            if database_url_problems(url, role):
                raise Refused(f"{ENV_PATHS[kind]}: OMOBA_DATABASE_URL is not a loopback {role}@{DB_NAME} URL "
                              "with a generated password")
            passwords[role] = unquote(urlsplit(url).password or "")
        return passwords

    def psql(self, database: str, sql: str, hidden=(), check: bool = True):
        """SQL on stdin (never argv) as the local superuser over the Unix socket (peer)."""
        if database not in ("postgres", DB_NAME):
            raise Refused(f"internal: psql on database {database}")
        result = self.run(["runuser", "-u", "postgres", "--", "psql", "-X", "-q", "-At", "-v", "ON_ERROR_STOP=1",
                           "-d", database], check=False, input=sql)
        if check and result.returncode:
            raise SystemExit(f"psql on {database} failed: {redact(result.stderr.strip() or result.stdout.strip(), hidden)}")
        return result

    def pg_state(self) -> dict | None:
        result = self.psql("postgres", STATE_SQL, check=False)
        try:
            state = json.loads(result.stdout.strip().splitlines()[-1]) if result.returncode == 0 else None
        except (IndexError, ValueError):
            state = None
        if not isinstance(state, dict) or not isinstance(state.get("roles"), dict):
            return None
        state["schema"] = False
        if state.get("database_owner"):
            schema = self.psql(DB_NAME, SCHEMA_SQL, check=False)
            state["schema"] = schema.returncode == 0 and schema.stdout.strip().endswith("t")
        return state

    def schema_ready(self) -> bool:
        state = self.pg_state()
        return bool(state and state.get("schema") and state.get("database_owner") == DB_OWNER)

    def running_matches(self) -> list:
        """Match worker PIDs: every omoba-server of the RU user except the lobby itself."""
        lobby = self.main_pid(unit_name("lobby")) if self.active(unit_name("lobby")) else 0
        result = self.run(["pgrep", "-u", USER, "-x", "omoba-server"], check=False)
        return sorted(int(pid) for pid in result.stdout.split() if pid.isdigit() and int(pid) != lobby)

    def guard_no_matches(self) -> None:
        workers = self.running_matches()
        if workers:
            raise Refused(f"match workers are running ({workers}); retry when no RU match is in progress")

    def available_mib(self) -> int | None:
        for line in (self.read("/proc/meminfo") or "").splitlines():
            if line.startswith("MemAvailable:"):
                return int(line.split()[1]) // 1024
        return None

    def free_mib(self) -> int:
        return shutil.disk_usage(self.p("/")).free // (1 << 20)

    # ---- the full check (read-only)

    def status(self, payload: dict) -> dict:
        """Everything the RU host needs, checked read-only. `ok` only when nothing is missing."""
        problems, warnings, facts = [], [], {}
        expected_ip = payload.get("expected_ip")
        names = self.payload_names(payload)
        units = self.payload_units(payload)
        self.payload_examples(payload)
        problems += self.identity_problems(payload)
        facts["os"] = " ".join(filter(None, (self.os_release().get("ID"), self.os_release().get("VERSION_ID"))))
        missing = self.missing_packages()
        if missing:
            problems.append(f"packages missing: {', '.join(missing)}")
        # PostgreSQL
        state = self.pg_state() if "postgresql" not in missing else None
        facts["postgres"] = None if state is None else {key: state.get(key) for key in
                                                        ("version", "icu", "listen", "port", "database_owner", "schema")}
        problems += postgres_problems(state)
        if state is not None:
            if state.get("database_owner") is None:
                problems.append(f"database {DB_NAME} does not exist (bootstrap creates it)")
            elif not state.get("schema"):
                problems.append(f"database {DB_NAME} has no career/portal schema (ship migrates it)")
            if not state["roles"].get(DB_GAME) or not state["roles"].get(DB_PORTAL):
                problems.append(f"roles {DB_GAME} and {DB_PORTAL} must exist and log in (bootstrap)")
            if state["roles"].get(DB_OWNER):
                problems.append(f"{DB_OWNER} can log in; only a migration may enable it, briefly")
        exposed = sorted(f"{address}:{port}" for address, port in self.listeners("tcp")
                         if port == DB_PORT and address not in LOOPBACK)
        if exposed:
            problems.append(f"PostgreSQL listens on {exposed}; it must be loopback only")
        for service in ("postgresql", "caddy"):
            if not self.enabled(service) or not self.active(service):
                problems.append(f"{service} must be enabled and running")
        # Caddy, firewall, SSH
        facts["caddy"] = self.out("caddy", "version").split(" ")[0] if "caddy" not in missing else None
        if self.read(CADDYFILE) != payload.get("caddyfile"):
            problems.append(f"{CADDYFILE} is not ops/vps/ru/Caddyfile.ru (bootstrap installs it)")
        if not any(port == HTTPS_PORT for _, port in self.listeners("tcp")):
            problems.append(f"nothing listens on TCP {HTTPS_PORT} (Caddy)")
        effective = self.ssh_effective()
        try:
            rules = firewall_rules(units.get("lobby", ""), ssh_ports(effective))
            problems += firewall_problems(parse_ufw(self.out("ufw", "status", "verbose")), rules)
        except Refused as refusal:
            problems.append(refusal.message)
        if self.read(SSHD_DROPIN) != SSHD_TEXT:
            problems.append(f"{SSHD_DROPIN} is missing or changed (bootstrap writes it)")
        problems += sshd_problems(effective)
        # users, folders, env files
        for user, home in USERS.items():
            entry = self.passwd(user)
            if entry is None or len(entry) < 7 or entry[5] != home or not entry[6].endswith("nologin"):
                problems.append(f"system user {user} (home {home}, nologin) is missing or different")
        for path, owner, mode in DIRS:
            local = self.p(path)
            if os.path.islink(local) or not os.path.isdir(local):
                problems.append(f"{path} is not a folder")
                continue
            info = os.stat(local)
            if oct(info.st_mode & 0o777)[2:].zfill(4) != mode:
                problems.append(f"{path} has mode {oct(info.st_mode & 0o777)}, not {mode}")
            if info.st_uid != (self.root_uid if owner == "root" else self.uid(owner)):
                problems.append(f"{path} is not owned by {owner}")
        more, notes = self.env_report(payload)
        problems += more
        warnings += notes
        # Node.js and the releases
        node = self.node_version()
        facts["node"] = ".".join(map(str, node)) if node else None
        if node is None or node[:2] < MIN_NODE:
            problems.append(f"Node.js >= {MIN_NODE[0]}.{MIN_NODE[1]} is missing at {NODE_CURRENT}/bin/node "
                            "(bootstrap --node-tarball)")
        try:
            facts["server_release"] = self.current(CURRENT, RELEASES)
            facts["portal_release"] = self.current(PORTAL_CURRENT, PORTAL_RELEASES)
        except Refused as refusal:
            problems.append(refusal.message)
        if not facts.get("server_release"):
            problems.append("no lobby/Account API release yet (ship)")
        if not facts.get("portal_release"):
            problems.append("no portal release yet (ship-portal)")
        # units
        facts["units"] = {}
        for kind in KINDS:
            name = unit_name(kind)
            installed = self.read(unit_path(kind))
            facts["units"][name] = {"enabled": self.enabled(name), "active": self.active(name),
                                    "restarts": self.show(name, "NRestarts")}
            if installed is None:
                problems.append(f"{unit_path(kind)} is not installed")
            elif kind in units and installed != units[kind]:
                problems.append(f"{unit_path(kind)} differs from ops/vps/ru/{UNIT_FILES[kind]}")
            if not facts["units"][name]["enabled"]:
                problems.append(f"{name} is not enabled: it would not start after a reboot")
            if not facts["units"][name]["active"]:
                problems.append(f"{name} is not running")
        # backups (LR-03)
        more, notes, facts["backups"] = self.backup_report(payload)
        problems += more
        warnings += notes
        # what answers
        if ("0.0.0.0", LOBBY_PORT) not in self.listeners("udp"):
            problems.append(f"the lobby does not listen on UDP {LOBBY_PORT}")
        tcp = self.listeners("tcp")
        for label, address, url, good in (
                ("Account API", ACCOUNT_API_ADDR, "http://%s:%d/health/ready" % ACCOUNT_API_ADDR, (200,)),
                ("portal", PORTAL_ADDR, "http://%s:%d/" % PORTAL_ADDR, tuple(range(200, 400)))):
            if address not in tcp:
                problems.append(f"the {label} does not listen on {address[0]}:{address[1]}")
            status = self.fetch(url)
            facts[f"{label.lower().replace(' ', '_')}_http"] = status
            if status not in good:
                problems.append(f"GET {url} answered {status or 'nothing'}")
        facts["names"] = {key: names[key] for key in HOST_KEYS}
        if expected_ip:
            more, notes = dns_problems(self.resolve, expected_ip, names)
            problems += more
            warnings += notes
        facts["available_mib"] = self.available_mib()
        facts["free_mib"] = self.free_mib()
        if facts["free_mib"] < MIN_FREE_MIB:
            problems.append(f"only {facts['free_mib']} MiB free on /")
        return {"mode": "status", "ok": not problems, "problems": problems, "warnings": warnings, "facts": facts}

    # ---- bootstrap

    def bootstrap_preflight(self, payload: dict) -> dict:
        """What bootstrap would do, and what blocks it. Read-only."""
        self.guard_identity(payload, require_marker=False)
        names = self.payload_names(payload)
        units = self.payload_units(payload)
        examples = self.payload_examples(payload)
        problems, warnings, would = [], [], []
        missing = self.missing_packages()
        if missing:
            would.append(f"apt-get install {' '.join(missing)}")
        node = payload.get("node")
        version = self.node_version()
        if node:
            would.append(f"install Node.js {node['version']} into {NODE}/v{node['version']} from the uploaded tarball")
        elif version is None or version[:2] < MIN_NODE:
            candidate = self.distro_node()
            if candidate and candidate >= MIN_NODE:
                would.append(f"apt-get install nodejs ({candidate[0]}.{candidate[1]}) and link {NODE_SYSTEM}")
            else:
                warnings.append(f"Node.js >= {MIN_NODE[0]}.{MIN_NODE[1]} is not available "
                                f"(Ubuntu offers {candidate}); pass --node-tarball, or the portal cannot run")
        for user, home in USERS.items():
            if self.passwd(user) is None:
                would.append(f"useradd --system {user} (home {home}, nologin)")
        would += [f"install -d -m {mode} {path} ({owner})" for path, owner, mode in DIRS
                  if not os.path.isdir(self.p(path))]
        if "postgresql" not in missing:
            state = self.pg_state()
            problems += postgres_problems(state)
            if state and state.get("database_owner") is None:
                would.append(f"create roles {', '.join(DB_ROLES)} and database {DB_NAME} (C.UTF-8, owner {DB_OWNER})")
        else:
            would.append(f"check PostgreSQL >= {MIN_POSTGRES}, {ICU_COLLATION}, loopback only; "
                         f"create roles {', '.join(DB_ROLES)} and database {DB_NAME}")
        for kind in KINDS:
            text = self.env_text(kind)
            values = parse_env(text if text is not None else examples[kind])[0]
            fill = placeholder_fill(values, payload["expected_ip"], payload.get("vkid"))
            if text is None:
                would.append(f"write {ENV_PATHS[kind]} (root 0600) from {ENV_EXAMPLES[kind]}, generating "
                             + (", ".join(sorted(fill)) or "nothing"))
            elif fill:
                would.append(f"fill {', '.join(sorted(fill))} in {ENV_PATHS[kind]}")
            if any(PLACEHOLDER in value for key, value in values.items() if key not in fill):
                warnings.append(f"{ENV_PATHS[kind]} keeps a placeholder: pass --vkid-client-id (VK ID app, LR-19)")
        if self.read(CADDYFILE) != payload.get("caddyfile"):
            would.append(f"install Caddyfile.ru as {CADDYFILE} (caddy validate first), enable + reload caddy")
        if self.read(SSHD_DROPIN) != SSHD_TEXT:
            if self.authorized_keys() == 0:
                problems.append(f"{AUTHORIZED_KEYS} has no key: key-only SSH would lock everyone out")
            would.append(f"write {SSHD_DROPIN} (key-only SSH), sshd -t, reload ssh")
        rules = firewall_rules(units["lobby"], ssh_ports(self.ssh_effective()))
        would.append("ufw: deny incoming; allow " + ", ".join(spec for _, spec, _ in rules) + "; enable")
        for kind in KINDS:
            if self.read(unit_path(kind)) != units.get(kind):
                would.append(f"install {unit_path(kind)}")
        backup = self.payload_backup_kit(payload)
        if self.read(BACKUP_SCRIPT) != backup["script"]:
            would.append(f"install {BACKUP_SCRIPT}")
        would += [f"install {UNIT_DIR}/{name}" for name in BACKUP_UNITS
                  if self.read(f"{UNIT_DIR}/{name}") != backup["units"][name]]
        if not os.path.lexists(self.p(BACKUP_ENV)):
            would.append(f"write {BACKUP_ENV} (root 0600) from {BACKUP_ENV_EXAMPLE}; the owner names the storage")
        would.append("systemctl enable postgresql caddy (+ each RU unit whose release is installed) and start "
                     + " ".join(BACKUP_TIMERS))
        more, notes = dns_problems(self.resolve, payload["expected_ip"], names)
        warnings += [f"DNS: {note}" for note in more + notes]
        return {"mode": "preflight", "kind": "bootstrap", "ok": not problems, "problems": problems,
                "warnings": warnings, "would": would}

    def bootstrap(self, payload: dict) -> dict:
        """Set the host up; every step is idempotent and secrets are generated here."""
        self.guard_identity(payload, require_marker=False)
        units = self.payload_units(payload)
        examples = self.payload_examples(payload)
        backup = self.payload_backup_kit(payload)
        if set(units) != set(KINDS) or "caddyfile" not in payload:
            raise Refused("payload lacks the units or Caddyfile.ru")
        self.mutating = True
        done = self.write_marker(payload["expected_ip"])
        done += self.install_packages()
        done += self.ensure_users_and_dirs()
        done += self.ensure_node(payload.get("node"))
        state = self.pg_state()
        problems = postgres_problems(state)
        if problems:
            raise Refused("PostgreSQL: " + "; ".join(problems))
        done += self.ensure_env_files(examples, payload["expected_ip"], payload.get("vkid"))
        done += self.ensure_database(state)
        done += self.ensure_caddy(payload["caddyfile"])
        done += self.ensure_ssh()
        done += self.ensure_firewall(units["lobby"])
        done += self.ensure_units(units)
        done += self.ensure_backup_kit(backup)
        self._record({"mode": "bootstrap", "done": done})
        # Releases, DNS and the VK ID app may legitimately still be missing: report them.
        report = self.status(payload)
        return {"mode": "bootstrap", "status": "BOOTSTRAPPED", "done": done, "ready": report["ok"],
                "remaining": report["problems"], "warnings": report["warnings"], "facts": report["facts"]}

    def write_marker(self, expected_ip: str) -> list:
        marker = self.marker()
        if marker is not None:
            return []
        self.write_guard(MARKER)
        if not os.path.isdir(self.p(OPT)):
            self.command("install", "-d", "-o", "root", "-g", "root", "-m", "0755", self.p(OPT))
        self._write(MARKER, json.dumps({"market": MARKET, "ip": expected_ip, "tool": TOOL,
                                        "bootstrapped_at": self._stamp()}, indent=2) + "\n", 0o644)
        return [f"write {MARKER}"]

    def install_packages(self, extra=()) -> list:
        missing = self.missing_packages() + [package for package in extra if not self.installed(package)]
        if not missing:
            return []
        env = {"PATH": SAFE_PATH, "DEBIAN_FRONTEND": "noninteractive", "LANG": "C.UTF-8"}
        self.command("apt-get", "update", "-q", env=env, timeout=900)
        self.command("apt-get", "install", "-y", "-q", "--no-install-recommends", *missing, env=env, timeout=1800)
        still = [package for package in missing if not self.installed(package)]
        if still:
            raise SystemExit(f"apt-get did not install {still}")
        return [f"apt-get install {' '.join(missing)}"]

    def ensure_users_and_dirs(self) -> list:
        done = []
        for user, home in USERS.items():
            entry = self.passwd(user)
            if entry is None:
                self.command("useradd", "--system", "--user-group", "--no-create-home", "--home-dir", home,
                             "--shell", "/usr/sbin/nologin", user)
                done.append(f"useradd {user}")
                entry = self.passwd(user)
            uid = self.uid(user)
            if entry is None or not uid or len(entry) < 7 or entry[5] != home or not entry[6].endswith("nologin"):
                raise Refused(f"user {user} exists with uid {uid} and an unexpected home or shell")
        for path, owner, mode in DIRS:
            local = self.p(path)
            self.write_guard(path)
            if os.path.islink(local) or (os.path.lexists(local) and not os.path.isdir(local)):
                raise Refused(f"{path} exists and is not a folder")
            if not os.path.isdir(local):
                self.command("install", "-d", "-o", owner, "-g", owner, "-m", mode, local)
                done.append(f"install -d {path}")
                continue
            info = os.stat(local)
            if oct(info.st_mode & 0o777)[2:].zfill(4) != mode or \
                    info.st_uid != (self.root_uid if owner == "root" else self.uid(owner)):
                self.command("chown", f"{owner}:{owner}", local)
                self.command("chmod", mode, local)
                done.append(f"chown {owner} + chmod {mode} {path}")
        return done

    def ensure_node(self, node: dict | None) -> list:
        """Node.js for the portal: an uploaded official tarball, else Ubuntu's nodejs when it
        is new enough (Ubuntu 24.04's 18.x is not), else nothing (status reports it)."""
        if node:
            match = NODE_TARBALL.fullmatch(node.get("file") or "")
            version = node.get("version")
            if not match or ".".join(match.groups()) != version or not HEX64.fullmatch(node.get("sha256") or ""):
                raise Refused("payload names an invalid Node.js tarball")
            directory = f"{NODE}/v{version}"
            done = []
            if not os.path.isdir(self.p(directory)):
                part = self.p(f"{INCOMING}/{node['file']}.part")
                self.write_guard(directory)
                if not os.path.isfile(part):
                    raise Refused(f"the Node.js tarball was not uploaded to {INCOMING}")
                if sha256_file(part) != node["sha256"]:
                    os.unlink(part)
                    raise Refused("the uploaded Node.js tarball does not match --node-sha256; removed it")
                partial = self.p(f"{NODE}/.v{version}.partial")
                shutil.rmtree(partial, ignore_errors=True)
                safe_extract(part, partial, f"node-v{version}-linux-x64")
                os.replace(partial, self.p(directory))
                os.unlink(part)
                done.append(f"install Node.js {version} into {directory}")
            binary = self.p(directory + "/bin/node")
            if os.path.islink(binary) or not os.path.isfile(binary):
                raise Refused(f"{directory}/bin/node is missing")
            with open(binary, "rb") as handle:
                if not is_linux_x86_64_elf(handle.read(64)):
                    raise Refused(f"{directory}/bin/node is not a Linux x86-64 executable")
            if self.node_version(directory + "/bin/node") != tuple(int(part) for part in version.split(".")):
                raise Refused(f"{directory}/bin/node does not report v{version}")
            if os.path.realpath(self.p(NODE_CURRENT)) != os.path.realpath(self.p(directory)):
                self._point(NODE_CURRENT, f"v{version}")
                done.append(f"{NODE_CURRENT} -> v{version}")
            return done
        current = self.node_version()
        if current and current[:2] >= MIN_NODE:
            return []
        candidate = self.distro_node()
        if not candidate or candidate < MIN_NODE:
            return []
        done = self.install_packages(extra=("nodejs",))
        self.write_guard(NODE_SYSTEM)
        os.makedirs(self.p(NODE_SYSTEM + "/bin"), exist_ok=True)
        link = self.p(NODE_SYSTEM + "/bin/node")
        if not os.path.islink(link):
            os.symlink("/usr/bin/node", link)
        self._point(NODE_CURRENT, "system")
        return done + [f"{NODE_CURRENT} -> system (/usr/bin/node)"]

    def ensure_env_files(self, examples: dict, expected_ip: str, vkid: str | None) -> list:
        """Write each env file from its example when missing; afterwards only fill placeholders.
        A generated secret is never replaced (OMOBA_SOCIAL_SECRET keys every VK identity)."""
        done = []
        for kind in KINDS:
            current = self.env_text(kind)
            if current is None and os.path.lexists(self.p(ENV_PATHS[kind])):
                raise Refused(f"{ENV_PATHS[kind]} exists and is not a regular file")
            base = current if current is not None else examples[kind]
            fill = placeholder_fill(parse_env(base)[0], expected_ip, vkid)
            if current is None or fill:
                self._write_secret(ENV_PATHS[kind], fill_env(base, fill))
                done.append(("write " if current is None else "fill ") + f"{ENV_PATHS[kind]}"
                            + (f" ({', '.join(sorted(fill))})" if fill else ""))
        return done

    def ensure_database(self, state: dict) -> list:
        passwords = self.runtime_passwords()
        self.psql("postgres", bootstrap_sql(passwords[DB_GAME], passwords[DB_PORTAL]),
                  hidden=tuple(passwords.values()))
        done = [f"role {role}" for role in DB_ROLES if role not in state["roles"]]
        if state.get("database_owner") is None:
            done.append(f"database {DB_NAME}")
        return [f"create {', '.join(done)}"] if done else []

    def ensure_caddy(self, caddyfile: str) -> list:
        done = []
        current = self.read(CADDYFILE)
        if current != caddyfile:
            self.write_guard(CADDYFILE)
            if current is not None and not os.path.lexists(self.p(CADDYFILE + ".orig")):
                self._write(CADDYFILE + ".orig", current, 0o644)
            candidate = CADDYFILE + ".omoba-next"
            self._write(candidate, caddyfile, 0o644)
            result = self.run(["caddy", "validate", "--adapter", "caddyfile", "--config", self.p(candidate)],
                              check=False)
            if result.returncode:
                os.unlink(self.p(candidate))
                raise SystemExit(f"caddy validate refused Caddyfile.ru: {(result.stderr or result.stdout).strip()[-400:]}")
            os.replace(self.p(candidate), self.p(CADDYFILE))
            done.append(f"install {CADDYFILE} (Caddyfile.ru)")
        if not self.enabled("caddy"):
            self.command("systemctl", "enable", "caddy")
            done.append("systemctl enable caddy")
        if not self.active("caddy"):
            self.command("systemctl", "start", "caddy")
            done.append("systemctl start caddy")
        elif done and done[0].startswith("install"):
            self.command("systemctl", "reload", "caddy")
            done.append("systemctl reload caddy")
        return done

    def ensure_ssh(self) -> list:
        done = []
        previous = self.read(SSHD_DROPIN)
        if previous != SSHD_TEXT:
            if self.authorized_keys() == 0:
                raise Refused(f"{AUTHORIZED_KEYS} has no key: key-only SSH would lock everyone out")
            self.write_guard(SSHD_DROPIN)
            self._write(SSHD_DROPIN, SSHD_TEXT, 0o644)
            test = self.run(["sshd", "-t"], check=False)
            if test.returncode:
                if previous is None:
                    os.unlink(self.p(SSHD_DROPIN))
                else:
                    self._write(SSHD_DROPIN, previous, 0o644)
                raise SystemExit(f"sshd -t refused the key-only drop-in: {(test.stderr or test.stdout).strip()}")
            self.command("systemctl", "try-reload-or-restart", "ssh.service")
            done.append(f"write {SSHD_DROPIN} + reload ssh (key-only)")
        problems = sshd_problems(self.ssh_effective())
        if problems:
            raise SystemExit("sshd still allows more than keys: " + "; ".join(problems)
                             + " (an earlier sshd_config line wins; fix it by hand)")
        return done

    def ensure_firewall(self, lobby_unit: str) -> list:
        rules = firewall_rules(lobby_unit, ssh_ports(self.ssh_effective()))
        status = parse_ufw(self.out("ufw", "status", "verbose"))
        expected = {spec for _, spec, _ in rules}
        commands = []
        for action, to in sorted({(rule["action"].lower(), rule["to"]) for rule in status["rules"]
                                  if rule["comment"].startswith("omoba-ru") and rule["to"] not in expected}):
            commands.append(["ufw", "--force", "delete", action, to])
        if status["incoming"] != "deny":
            commands.append(["ufw", "default", "deny", "incoming"])
            commands.append(["ufw", "default", "allow", "outgoing"])
        for action, spec, comment in rules:
            if not any(rule["to"] == spec and rule["action"] == action.upper() and not rule["v6"]
                       for rule in status["rules"]):
                commands.append(["ufw", action, spec, "comment", comment])
        if not status["active"]:
            commands.append(["ufw", "--force", "enable"])
        for command in commands:
            self.command(*command)
        problems = firewall_problems(parse_ufw(self.out("ufw", "status", "verbose")), rules)
        if problems:
            raise SystemExit("firewall: " + "; ".join(problems))
        return [" ".join(command) for command in commands]

    def ensure_units(self, units: dict) -> list:
        """Install the unit files; enable postgresql, caddy and each RU unit whose release is
        installed (ship enables the rest), so a boot never starts a missing binary."""
        done = []
        for kind in KINDS:
            if self.read(unit_path(kind)) != units[kind]:
                self._write(unit_path(kind), units[kind], 0o644)
                done.append(f"install {unit_path(kind)}")
        if done:
            self.command("systemctl", "daemon-reload")
        for service in ("postgresql", "caddy") + tuple(unit_name(kind) for kind in KINDS
                                                        if self.release_present(kind)):
            if not self.enabled(service):
                self.command("systemctl", "enable", service)
                done.append(f"systemctl enable {service}")
        return done

    def ensure_backup_kit(self, backup: dict) -> list:
        """ru_backup.py, its units and timers, and backup.env with placeholders (never replaced)."""
        done = []
        if self.read(BACKUP_SCRIPT) != backup["script"]:
            self._write(BACKUP_SCRIPT, backup["script"], 0o755)
            done.append(f"install {BACKUP_SCRIPT}")
        changed = False
        for name in BACKUP_UNITS:
            path = f"{UNIT_DIR}/{name}"
            if self.read(path) != backup["units"][name]:
                self._write(path, backup["units"][name], 0o644)
                done.append(f"install {path}")
                changed = True
        if changed:
            self.command("systemctl", "daemon-reload")
        if self.read(BACKUP_ENV) is None:
            if os.path.lexists(self.p(BACKUP_ENV)):
                raise Refused(f"{BACKUP_ENV} exists and is not a regular file")
            self._write_secret(BACKUP_ENV, backup["env_example"])
            done.append(f"write {BACKUP_ENV} from {BACKUP_ENV_EXAMPLE} (placeholders: the owner names the storage)")
        for timer in BACKUP_TIMERS:
            if not self.enabled(timer):
                self.command("systemctl", "enable", timer)
                done.append(f"systemctl enable {timer}")
            if not self.active(timer):
                self.command("systemctl", "start", timer)
                done.append(f"systemctl start {timer}")
        return done

    def backup_tool(self, *args, timeout: int = 180):
        """The installed ru_backup.py; (exit code, its JSON answer or None, stderr)."""
        result = self.run([PYTHON, self.p(BACKUP_SCRIPT), *args], check=False, timeout=timeout)
        try:
            answer = json.loads(result.stdout.strip().splitlines()[-1] if args[0] == "backup" else result.stdout)
        except (IndexError, ValueError):
            answer = None
        return result.returncode, answer, redact(result.stderr.strip())

    def backup_report(self, payload: dict) -> tuple[list, list, dict]:
        """The backup kit as installed, its timers and `ru_backup.py check --remote` (LR-03)."""
        backup = self.payload_backup_kit(payload)
        problems, warnings = [], []
        script_ok = self.read(BACKUP_SCRIPT) == backup["script"]
        if not script_ok:
            problems.append(f"{BACKUP_SCRIPT} is not ops/vps/ru/ru_backup.py (bootstrap installs it)")
        for name in BACKUP_UNITS:
            if self.read(f"{UNIT_DIR}/{name}") != backup["units"][name]:
                problems.append(f"{UNIT_DIR}/{name} differs from ops/vps/ru/{name} (bootstrap installs it)")
        local = self.p(BACKUP_ENV)
        if not os.path.isfile(local) or os.path.islink(local):
            problems.append(f"{BACKUP_ENV} is missing (bootstrap writes it)")
        else:
            info = os.stat(local)
            if info.st_mode & 0o777 != 0o600 or info.st_uid != self.root_uid:
                problems.append(f"{BACKUP_ENV} must be root-owned with mode 0600")
        for timer in BACKUP_TIMERS:
            if not self.enabled(timer):
                problems.append(f"{timer} is not enabled: it would not run after a reboot")
            elif not self.active(timer):
                problems.append(f"{timer} is not active")
        facts = {}
        if script_ok:
            _, report, errors = self.backup_tool("check", "--json", "--remote")
            if not isinstance(report, dict) or not isinstance(report.get("problems"), list):
                problems.append(f"backups: ru_backup.py check did not answer ({errors[-300:] or 'no output'})")
            else:
                problems += [f"backups: {problem}" for problem in report["problems"]]
                warnings += [f"backups: {warning}" for warning in report.get("warnings") or []]
                facts = report.get("facts") or {}
        return problems, warnings, facts

    def backup_blockers(self, payload: dict) -> list:
        """What stops the pre-migration backup of a ship (a schema exists)."""
        backup = self.payload_backup_kit(payload)
        if self.read(BACKUP_SCRIPT) != backup["script"]:
            return [f"{BACKUP_SCRIPT} is missing or older than ops/vps/ru/ru_backup.py (run bootstrap again)"]
        if payload.get("local_backup_only"):
            return []
        code, report, errors = self.backup_tool("config", "--json")
        if code == 0 and isinstance(report, dict) and report.get("ok"):
            return []
        reasons = report.get("problems") if isinstance(report, dict) else [errors[-300:] or "no answer"]
        return [f"backups are not configured ({'; '.join(map(str, reasons))}): fill {BACKUP_ENV} and the keys "
                "(README.md, \"Backups, escrow and restore drill\") or, before launch only, ship with "
                "--local-backup-only"]

    def pre_migration_backup(self, release: str, local_only: bool) -> dict:
        """`ru_backup.py backup --reason pre-migration`: the encrypted set goes off-host and a
        plain rollback dump stays in /var/backups/omoba-ru (README.md, "Rollback")."""
        if not self.mutating:
            raise Refused("internal: a backup in a read-only mode")
        args = ["backup", "--reason", "pre-migration", "--release", release] + (["--local-only"] if local_only else [])
        code, answer, errors = self.backup_tool(*args, timeout=6 * 3600)
        if code or not isinstance(answer, dict):
            raise SystemExit(f"the pre-migration backup failed, nothing was migrated: {errors[-600:] or 'no output'}")
        return {key: answer.get(key) for key in ("name", "rollback_dump", "uploaded", "target", "tables", "rows",
                                                 "schema_versions", "mode")}

    # ---- shipping

    def server_blockers(self, payload: dict) -> list:
        problems = []
        for kind in SERVER_KINDS:
            problems += self.env_file_report(kind, payload)[0]
        problems += postgres_problems(self.pg_state())
        for kind in SERVER_KINDS:
            if self.read(unit_path(kind)) is None and kind not in (payload.get("units") or {}):
                problems.append(f"{unit_path(kind)} is not installed (bootstrap)")
        if self.free_mib() < MIN_FREE_MIB:
            problems.append(f"only {self.free_mib()} MiB free on /")
        return problems

    def portal_blockers(self, payload: dict) -> list:
        problems = list(self.env_file_report("portal", payload)[0])
        node = self.node_version()
        if node is None or node[:2] < MIN_NODE:
            problems.append(f"Node.js >= {MIN_NODE[0]}.{MIN_NODE[1]} is missing at {NODE_CURRENT} (bootstrap)")
        if self.read(unit_path("portal")) is None and "portal" not in (payload.get("units") or {}):
            problems.append(f"{unit_path('portal')} is not installed (bootstrap)")
        return problems

    def ship_preflight(self, payload: dict) -> dict:
        kind = payload.get("kind")
        self.guard_identity(payload)
        self.payload_units(payload)
        self.payload_examples(payload)
        if kind == "server":
            release = validate_server_release(payload.get("release"))
            problems = self.server_blockers(payload)
            directory = f"{RELEASES}/{release}"
            current = self.current(CURRENT, RELEASES)
            would = [f"{'verify staged' if os.path.isdir(self.p(directory)) else 'stage'} {directory}"]
            if payload.get("grants"):
                if self.schema_ready():
                    problems += self.backup_blockers(payload)
                    would.append("ru_backup.py backup --reason pre-migration: "
                                 + (f"plain rollback dump in {BACKUPS} only (--local-backup-only)"
                                    if payload.get("local_backup_only") else
                                    f"encrypted set off-host + rollback dump in {BACKUPS}"))
                would.append(f"omoba-account-api migrate (temporary {DB_OWNER} login) + {GRANTS}")
            would += [f"{CURRENT}: {current} -> {release}",
                      f"systemctl enable + restart {unit_name('account-api')} {unit_name('lobby')}",
                      f"verify UDP {LOBBY_PORT}, the lobby's exe, Account API /health/ready; roll back on failure"]
            workers = self.running_matches()
            if workers:
                problems.append(f"match workers are running ({workers}); ship when no RU match is in progress")
        elif kind == "portal":
            release = validate_portal_release(payload.get("release"))
            problems = self.portal_blockers(payload)
            current = self.current(PORTAL_CURRENT, PORTAL_RELEASES)
            would = [f"unpack the bundle into {PORTAL_RELEASES}/{release}, check it is the RU production build",
                     f"{PORTAL_CURRENT}: {current} -> {release}", f"systemctl enable + restart {unit_name('portal')}",
                     f"verify GET http://{PORTAL_ADDR[0]}:{PORTAL_ADDR[1]}/; roll back on failure"]
        else:
            raise Refused(f"unknown kind {kind!r}")
        return {"mode": "preflight", "kind": kind, "ok": not problems, "problems": problems, "would": would}

    def stage(self, payload: dict) -> dict:
        """Folders and the upload targets (`<name>.part`) the Mac must scp; nothing else."""
        kind = payload.get("kind")
        self.guard_identity(payload, require_marker=kind != "node")
        self.mutating = True
        if kind == "node":
            node = payload.get("node") or {}
            if not NODE_TARBALL.fullmatch(node.get("file") or ""):
                raise Refused("payload names no Node.js tarball")
            for path, owner, mode in ((OPT, "root", "0755"), (NODE, "root", "0755"), (INCOMING, "root", "0700")):
                self._install_dir(path, owner, mode)
            if os.path.isdir(self.p(f"{NODE}/v{node.get('version')}")):
                return {"mode": "stage", "kind": kind, "upload_to": []}
            return {"mode": "stage", "kind": kind, "upload_to": [f"{INCOMING}/{node['file']}.part"]}
        if kind == "server":
            release = validate_server_release(payload.get("release"))
            files = server_files(payload)
            directory = f"{RELEASES}/{release}"
            missing = []
            for name, digest in files.items():
                path = self.p(f"{directory}/{name}")
                if os.path.lexists(path):
                    if not os.path.isfile(path) or os.path.islink(path) or sha256_file(path) != digest:
                        raise Refused(f"{directory} already holds a different {name}; releases are immutable")
                else:
                    missing.append(name)
            if missing:
                self._install_dir(directory, "root", "0755")
            return {"mode": "stage", "kind": kind, "release": release,
                    "upload_to": [f"{directory}/{name}.part" for name in missing]}
        if kind == "portal":
            release = validate_portal_release(payload.get("release"))
            if not PORTAL_BUNDLE.fullmatch(payload.get("file") or ""):
                raise Refused("payload names no portal bundle")
            if os.path.isdir(self.p(f"{PORTAL_RELEASES}/{release}")):
                return {"mode": "stage", "kind": kind, "release": release, "upload_to": []}
            self._install_dir(INCOMING, "root", "0700")
            return {"mode": "stage", "kind": kind, "release": release,
                    "upload_to": [f"{INCOMING}/{payload['file']}.part"]}
        raise Refused(f"unknown kind {kind!r}")

    def _finalize_server_upload(self, directory: str, files: dict, build: dict | None) -> None:
        for name, digest in files.items():
            part, final = self.p(f"{directory}/{name}.part"), self.p(f"{directory}/{name}")
            if os.path.isfile(part) and not os.path.lexists(final):
                self.write_guard(f"{directory}/{name}")
                if sha256_file(part) != digest:
                    os.unlink(part)
                    raise Refused(f"uploaded {name}.part does not match the local sha256; removed it")
                os.chmod(part, 0o755)
                os.replace(part, final)
        if build and not os.path.isfile(self.p(directory + "/BUILD.json")):
            self._write(directory + "/BUILD.json", json.dumps(build, indent=2) + "\n", 0o644)

    def verify_server_release(self, directory: str, files: dict) -> None:
        for name, expected in files.items():
            path = self.p(f"{directory}/{name}")
            if os.path.islink(path) or not os.path.isfile(path):
                raise Refused(f"{directory}/{name} is missing or a symlink")
            with open(path, "rb") as handle:
                if not is_linux_x86_64_elf(handle.read(64)):
                    raise Refused(f"{directory}/{name} is not a Linux x86-64 executable")
            if sha256_file(path) != expected:
                raise Refused(f"{directory}/{name} does not match its sha256")

    def migrate(self, directory: str, grants: str) -> None:
        """`omoba-account-api migrate` as the owner through a temporary, expiring login."""
        temporary = secrets.token_hex(24)
        until = time.strftime("%Y-%m-%d %H:%M:%S+00", time.gmtime(self.now() + MIGRATION_LOGIN_SECONDS))
        try:
            self.psql("postgres", owner_login_sql(temporary, until), hidden=(temporary,))
            result = self.run([self.p(f"{directory}/omoba-account-api"), "migrate"], check=False, timeout=600,
                              env={"PATH": SAFE_PATH, "OMOBA_DATABASE_URL": database_url(DB_OWNER, temporary)})
            if result.returncode:
                raise SystemExit("omoba-account-api migrate failed: "
                                 + redact(result.stderr.strip() or result.stdout.strip(), (temporary,)))
            self.psql(DB_NAME, grants_sql(grants), hidden=(temporary,))
        finally:
            self.psql("postgres", OWNER_NOLOGIN_SQL, check=False)

    def verify_unit(self, kind: str) -> None:
        result = self.run(["systemd-analyze", "verify", self.p(unit_path(kind))], check=False)
        if result.returncode:
            raise SystemExit(f"systemd-analyze verify {UNIT_FILES[kind]}: {(result.stderr or result.stdout).strip()}")

    def server_answers(self, directory: str) -> bool:
        lobby = unit_name("lobby")
        pid = self.main_pid(lobby)
        return self.active(lobby) and self.active(unit_name("account-api")) and pid > 0 and \
            self.exe(pid) == os.path.realpath(self.p(directory + "/omoba-server")) and \
            ("0.0.0.0", LOBBY_PORT) in self.listeners("udp") and \
            self.fetch("http://%s:%d/health/ready" % ACCOUNT_API_ADDR) == 200

    def portal_answers(self) -> bool:
        return self.active(unit_name("portal")) and \
            200 <= self.fetch("http://%s:%d/" % PORTAL_ADDR) < 400

    def _wait(self, check, seconds: int) -> bool:
        for _ in range(seconds):
            if check():
                return True
            self.sleep(1)
        return check()

    def activate_server(self, payload: dict, mode: str = "ship") -> dict:
        """Switch the lobby and Account API to payload["release"]; roll back on any failure."""
        self.guard_identity(payload)
        release = validate_server_release(payload.get("release"))
        directory = f"{RELEASES}/{release}"
        files = server_files(payload)
        units = {kind: text for kind, text in self.payload_units(payload).items() if kind in SERVER_KINDS}
        self.payload_examples(payload)
        grants = validate_grants(payload.get("grants")) if mode != "rollback" else None
        blockers = self.server_blockers(payload)
        if blockers:
            raise Refused("; ".join(blockers))
        self.mutating = True
        self._finalize_server_upload(directory, files, payload.get("build"))
        self.verify_server_release(directory, files)
        previous = self.current(CURRENT, RELEASES)
        old_units = {kind: self.read(unit_path(kind)) for kind in SERVER_KINDS}
        changed = [kind for kind, text in units.items() if text != old_units[kind]]
        if previous == release and not changed and self.server_answers(directory) and \
                (grants is None or self.schema_ready()):
            return {"status": "UNCHANGED", "mode": mode, "release": release}
        self.guard_no_matches()
        database = None
        if grants is not None:
            backup = None
            if self.schema_ready():
                blockers = self.backup_blockers(payload)
                if blockers:
                    raise Refused("; ".join(blockers))
                backup = self.pre_migration_backup(release, bool(payload.get("local_backup_only")))
            self.migrate(directory, grants)
            database = {"migrated": True, "backup": backup}
        was_active = {kind: self.active(unit_name(kind)) for kind in SERVER_KINDS}
        for kind in SERVER_KINDS:
            if was_active[kind]:
                self.command("systemctl", "stop", unit_name(kind))
        try:
            self._point(CURRENT, f"releases/{release}")
            for kind in changed:
                self._write(unit_path(kind), units[kind], 0o644)
            if changed:
                self.command("systemctl", "daemon-reload")
            for kind in ("account-api", "lobby"):
                self.verify_unit(kind)
                self.command("systemctl", "enable", unit_name(kind))
                self.command("systemctl", "restart", unit_name(kind))
            if not self._wait(lambda: self.server_answers(directory), 20):
                raise SystemExit(f"the lobby (UDP {LOBBY_PORT}, {directory}/omoba-server) or the Account API "
                                 "(/health/ready) did not come up")
        except BaseException as error:
            rolled = self._rollback_server(previous, old_units, changed, was_active)
            raise SystemExit(f"{mode} of {release} failed ({error}); {rolled}") from error
        record = {"mode": mode, "kind": "server", "release": release, "previous": previous,
                  "sha256": files["omoba-server"], "account_api_sha256": files["omoba-account-api"],
                  "units_changed": changed, "database": database}
        self._record(record)
        return {"status": "DEPLOYED", **record}

    def _rollback_server(self, previous, old_units: dict, changed: list, was_active: dict) -> str:
        """Put the lobby and Account API back as they were. A failed first install leaves
        them stopped and disabled for `journalctl -u`."""
        for kind in SERVER_KINDS:
            self.run(["systemctl", "stop", unit_name(kind)], check=False)
        if previous is None:
            for kind in SERVER_KINDS:
                self.run(["systemctl", "disable", unit_name(kind)], check=False)
            return "first install: lobby and Account API left stopped and disabled"
        for kind in changed:
            if old_units[kind] is not None:
                self._write(unit_path(kind), old_units[kind], 0o644)
        if changed:
            self.run(["systemctl", "daemon-reload"], check=False)
        self._point(CURRENT, f"releases/{previous}")
        for kind in SERVER_KINDS:
            if was_active[kind]:
                self.run(["systemctl", "start", unit_name(kind)], check=False)
        return f"ROLLED_BACK to {previous}"

    def _install_portal_release(self, payload: dict, release: str) -> None:
        directory = f"{PORTAL_RELEASES}/{release}"
        if os.path.isdir(self.p(directory)):
            info = json.loads(self.read(directory + "/BUILD.json") or "{}")
            if info.get("bundle_sha256") != payload.get("sha256"):
                raise Refused(f"{directory} already holds a different bundle; releases are immutable")
            return
        bundle = PORTAL_BUNDLE.fullmatch(payload.get("file") or "")
        if not bundle or not HEX64.fullmatch(payload.get("sha256") or ""):
            raise Refused("payload names no portal bundle")
        part = self.p(f"{INCOMING}/{payload['file']}.part")
        if not os.path.isfile(part):
            raise Refused(f"the portal bundle was not uploaded to {INCOMING}")
        if sha256_file(part) != payload["sha256"]:
            os.unlink(part)
            raise Refused("the uploaded portal bundle does not match the local sha256; removed it")
        self.write_guard(directory)
        partial = self.p(f"{PORTAL_RELEASES}/.{release}.partial")
        shutil.rmtree(partial, ignore_errors=True)
        safe_extract(part, partial, payload["file"][:-len(".tar.gz")])
        problems, _ = portal_problems(tree_listing(partial), read_head(partial))
        if os.path.lexists(os.path.join(partial, ".next/cache")):
            problems.append(".next/cache is a build cache; bundle without it")
        if problems:
            shutil.rmtree(partial, ignore_errors=True)
            raise Refused("not the RU portal: " + "; ".join(problems))
        os.symlink(PORTAL_CACHE, os.path.join(partial, ".next/cache"))
        Path(partial, "BUILD.json").write_text(json.dumps(
            {"kind": "portal", "market": MARKET, "release": release, "bundle": payload["file"],
             "bundle_sha256": payload["sha256"], "revision": bundle.group(1), "tool": TOOL}, indent=2) + "\n")
        os.replace(partial, self.p(directory))
        os.unlink(part)

    def activate_portal(self, payload: dict, mode: str = "ship-portal") -> dict:
        self.guard_identity(payload)
        release = validate_portal_release(payload.get("release"))
        directory = f"{PORTAL_RELEASES}/{release}"
        unit = self.payload_units(payload).get("portal")
        self.payload_examples(payload)
        blockers = self.portal_blockers(payload)
        if blockers:
            raise Refused("; ".join(blockers))
        self.mutating = True
        if mode == "rollback":
            info = json.loads(self.read(directory + "/BUILD.json") or "{}")
            if info.get("kind") != "portal" or info.get("market") != MARKET or info.get("release") != release:
                raise Refused(f"{directory}/BUILD.json is not an RU portal release record for {release}")
        else:
            self._install_portal_release(payload, release)
        problems, _ = portal_problems(tree_listing(self.p(directory)), read_head(self.p(directory)))
        cache = self.p(directory + "/.next/cache")
        if not os.path.islink(cache) or os.readlink(cache) != PORTAL_CACHE:
            problems.append(f".next/cache must link to {PORTAL_CACHE}")
        if problems:
            raise Refused(f"{directory}: " + "; ".join(problems))
        previous = self.current(PORTAL_CURRENT, PORTAL_RELEASES)
        old_unit = self.read(unit_path("portal"))
        changed = unit is not None and unit != old_unit
        if previous == release and not changed and self.portal_answers():
            return {"status": "UNCHANGED", "mode": mode, "release": release}
        was_active = self.active(unit_name("portal"))
        try:
            self._point(PORTAL_CURRENT, f"releases/{release}")
            if changed:
                self._write(unit_path("portal"), unit, 0o644)
                self.command("systemctl", "daemon-reload")
            self.verify_unit("portal")
            self.command("systemctl", "enable", unit_name("portal"))
            self.command("systemctl", "restart", unit_name("portal"))
            if not self._wait(self.portal_answers, 30):
                raise SystemExit(f"the portal did not answer on {PORTAL_ADDR[0]}:{PORTAL_ADDR[1]}")
        except BaseException as error:
            self.run(["systemctl", "stop", unit_name("portal")], check=False)
            if previous is None:
                self.run(["systemctl", "disable", unit_name("portal")], check=False)
                rolled = "first install: portal left stopped and disabled"
            else:
                if changed and old_unit is not None:
                    self._write(unit_path("portal"), old_unit, 0o644)
                    self.run(["systemctl", "daemon-reload"], check=False)
                self._point(PORTAL_CURRENT, f"releases/{previous}")
                if was_active:
                    self.run(["systemctl", "start", unit_name("portal")], check=False)
                rolled = f"ROLLED_BACK to {previous}"
            raise SystemExit(f"{mode} of portal {release} failed ({error}); {rolled}") from error
        record = {"mode": mode, "kind": "portal", "release": release, "previous": previous,
                  "bundle_sha256": payload.get("sha256"), "unit_changed": changed}
        self._record(record)
        return {"status": "DEPLOYED", **record}

    def rollback(self, payload: dict) -> dict:
        kind, release = payload.get("kind"), payload.get("release")
        if kind == "portal":
            return self.activate_portal(payload, mode="rollback")
        if kind != "server":
            raise Refused(f"unknown kind {kind!r}")
        directory = f"{RELEASES}/{validate_server_release(release)}"
        info = json.loads(self.read(directory + "/BUILD.json") or "{}")
        if info.get("market") != MARKET or info.get("release") != release:
            raise Refused(f"{directory}/BUILD.json is not an RU release record for {release}")
        return self.activate_server({**payload, "sha256": info.get("sha256"),
                                     "account_api_sha256": info.get("account_api_sha256"), "build": None},
                                    mode="rollback")

    def boot_id(self) -> dict:
        return {"boot_id": (self.read("/proc/sys/kernel/random/boot_id") or "").strip()}

    # ---- writing

    def _stamp(self) -> str:
        return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(self.now()))

    def _install_dir(self, path: str, owner: str, mode: str) -> None:
        self.write_guard(path)
        if not os.path.isdir(self.p(path)):
            self.command("install", "-d", "-o", owner, "-g", owner, "-m", mode, self.p(path))

    def _write(self, path: str, text: str, mode: int) -> None:
        self.write_guard(path)
        temporary = self.p(path + ".tmp")
        with open(temporary, "w") as handle:
            handle.write(text)
        os.chmod(temporary, mode)
        os.replace(temporary, self.p(path))

    def _write_secret(self, path: str, text: str) -> None:
        """root:root 0600 from the first byte; secrets never pass through argv or output."""
        self.write_guard(path)
        temporary = self.p(path + ".next")
        if os.path.lexists(temporary):
            os.unlink(temporary)
        descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        try:
            os.write(descriptor, text.encode())
            if os.geteuid() == 0:
                os.fchown(descriptor, 0, 0)
            os.fchmod(descriptor, 0o600)
        finally:
            os.close(descriptor)
        os.replace(temporary, self.p(path))

    def _point(self, link: str, target: str) -> None:
        self.write_guard(link)
        temporary = self.p(link + ".next")
        if os.path.lexists(temporary):
            os.unlink(temporary)
        os.symlink(target, temporary)
        os.replace(temporary, self.p(link))

    def _record(self, record: dict) -> None:
        self.write_guard(HISTORY)
        with open(self.p(HISTORY), "a") as handle:
            handle.write(json.dumps({**record, "at": self._stamp()}) + "\n")


def encode_payload(payload: dict) -> str:
    return base64.b64encode(json.dumps(payload, sort_keys=True).encode()).decode()


def decode_payload(raw: str) -> dict:
    if not PAYLOAD.fullmatch(raw):
        raise Refused("payload is not base64")
    return json.loads(base64.b64decode(raw))


REMOTE_MODES = ("status", "preflight", "bootstrap", "stage", "activate", "rollback", "boot-id")


def remote_main(argv: list) -> int:
    """Host entry point: `python3 - remote <mode> [payload]`; always root "/"."""
    if len(argv) not in (1, 2) or argv[0] not in REMOTE_MODES:
        raise Refused("usage: remote " + "|".join(REMOTE_MODES) + " [payload]")
    if os.geteuid() != 0:
        raise Refused("the host half runs as root")
    host = Host()
    mode = argv[0]
    payload = decode_payload(argv[1]) if len(argv) == 2 else {}
    if mode == "boot-id":
        result = host.boot_id()
    elif mode == "status":
        result = host.status(payload)
    elif mode == "preflight":
        result = host.bootstrap_preflight(payload) if payload.get("kind") == "bootstrap" else host.ship_preflight(payload)
    else:
        import fcntl
        with open(host.p(LOCK), "a") as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except OSError:
                raise Refused("another ru_host.py run holds the lock") from None
            if mode == "bootstrap":
                result = host.bootstrap(payload)
            elif mode == "stage":
                result = host.stage(payload)
            elif mode == "rollback":
                result = host.rollback(payload)
            elif payload.get("kind") == "portal":
                result = host.activate_portal(payload)
            else:
                result = host.activate_server(payload)
    print(json.dumps(result, indent=2))
    return 0


# ---------------------------------------------------------------- Mac half

RU_DIR = Path(__file__).resolve().parent


def repo_root() -> Path:
    return Path(__file__).resolve().parents[3]


def markets_module():
    """scripts/markets.py of this checkout. Mac only: the host half has no checkout and
    gets the RU names in the payload."""
    scripts = str(repo_root() / "scripts")
    if scripts not in sys.path:
        sys.path.insert(0, scripts)
    import markets
    return markets


def ru_names() -> dict:
    """The RU public names from the RU profile in scripts/markets.py, the one place they are
    written: the lobby host (without its port), the Account API and portal hosts and the
    RU DNS zone (zones[0])."""
    markets = markets_module()
    profile = markets.market(MARKET)
    lobby, _, port = (profile.lobby or "").rpartition(":")
    names = {"lobby": lobby.lower().rstrip("."),
             "account_api": (urlsplit(profile.account_api or "").hostname or "").rstrip("."),
             "portal": (urlsplit(profile.portal or "").hostname or "").rstrip("."),
             "zone": (profile.zones[0] if profile.zones else "").lower().rstrip(".")}
    problems = names_problems(names)
    if port != str(LOBBY_PORT):
        problems.append(f"the lobby {profile.lobby!r} must use port {LOBBY_PORT}, the lobby unit's SERVER_ADDR")
    for key in ("account_api", "portal"):
        if getattr(profile, key) != f"https://{names[key]}":
            problems.append(f"{key} {getattr(profile, key)!r} must be the bare origin https://{names[key]} "
                            "(Caddy serves it on 443)")
    for key, other in markets.MARKETS.items():
        overlap = [zone for zone in other.zones
                   if in_zone(zone, (names["zone"],)) or in_zone(names["zone"], (zone,))]
        if key != MARKET and overlap:
            problems.append(f"the RU zone {names['zone']} overlaps the {key} market's {', '.join(overlap)}")
    if problems:
        raise Refused("the RU profile in scripts/markets.py: " + "; ".join(problems))
    return names


def kit(names: dict | None = None) -> dict:
    """The repository's RU host kit: units, env examples and Caddyfile.ru, checked against
    each other and against the RU names (default: ru_names(), from scripts/markets.py)."""
    names = names or ru_names()
    own_zones = (names["zone"],)
    units = {kind: (RU_DIR / name).read_text() for kind, name in UNIT_FILES.items()}
    examples = {kind: (RU_DIR / name).read_text() for kind, name in ENV_EXAMPLES.items()}
    caddyfile = (RU_DIR / "Caddyfile.ru").read_text()
    problems = [problem for kind, text in units.items() for problem in unit_problems(kind, text, own_zones)]
    problems += [problem for kind, text in examples.items() for problem in example_problems(kind, text)]
    for key in ("account_api", "portal"):
        if not re.search(rf"(?m)^{re.escape(names[key])}\s*\{{", caddyfile):
            problems.append(f"Caddyfile.ru has no {names[key]} site (the RU {key} in scripts/markets.py)")
    for kind in ("account-api", "portal"):
        origin = parse_env(examples[kind])[0].get("OMOBA_PORTAL_ORIGIN")
        if origin != f"https://{names['portal']}":
            problems.append(f"{ENV_EXAMPLES[kind]}: OMOBA_PORTAL_ORIGIN is {origin!r}, not the RU portal "
                            f"https://{names['portal']} (scripts/markets.py)")
    for address in (ACCOUNT_API_ADDR, PORTAL_ADDR):
        if f"reverse_proxy {address[0]}:{address[1]}" not in caddyfile:
            problems.append(f"Caddyfile.ru does not proxy to {address[0]}:{address[1]}")
    problems += [f"Caddyfile.ru names another contour's host {token}"
                 for token in re.findall(r"[A-Za-z0-9][A-Za-z0-9.-]*\.[a-z]{2,}", caddyfile)
                 if is_foreign_name(token, own_zones)]
    backup = {"script": (RU_DIR / "ru_backup.py").read_text(),
              "units": {name: (RU_DIR / name).read_text() for name in BACKUP_UNITS},
              "env_example": (RU_DIR / BACKUP_ENV_EXAMPLE).read_text()}
    problems += backup_kit_problems(backup, own_zones)
    problems += backup_example_problems(backup["env_example"])
    if problems:
        raise Refused("the RU host kit is inconsistent: " + "; ".join(problems))
    return {"units": units, "examples": examples, "caddyfile": caddyfile, "names": names,
            "backup_kit_z": encode_backup_kit(backup)}


def backup_example_problems(text: str) -> list[str]:
    """backup.env.example: only settings ru_backup.py knows, the shipped layout, no real secret."""
    if str(RU_DIR) not in sys.path:
        sys.path.insert(0, str(RU_DIR))
    import ru_backup
    values, problems = ru_backup.parse_env_text(text)
    problems = [f"{BACKUP_ENV_EXAMPLE}: {problem}" for problem in problems]
    for key in ("OMOBA_BACKUP_GPG_RECIPIENT_FILE", "OMOBA_BACKUP_GPG_IDENTITY_FILE"):
        if not within(values.get(key, ""), BACKUP_KEYS):
            problems.append(f"{BACKUP_ENV_EXAMPLE}: {key} must be inside {BACKUP_KEYS}")
    for key in ("OMOBA_BACKUP_TARGET", *ru_backup.SECRET_KEYS):
        if key in values and ru_backup.PLACEHOLDER not in values[key]:
            problems.append(f"{BACKUP_ENV_EXAMPLE}: {key} must stay a {PLACEHOLDER} placeholder")
    if ru_backup.ENV_FILE != BACKUP_ENV or ru_backup.DEFAULTS["OMOBA_BACKUP_DIR"] != BACKUPS or \
            ru_backup.DEFAULTS["OMOBA_BACKUP_DATABASE"] != DB_NAME or \
            ru_backup.DEFAULTS["OMOBA_BACKUP_STATE_DIRS"].split(":") != list(STATE_DIRS[1:]) or \
            not ru_backup.DEFAULTS["OMOBA_BACKUP_GPG_RECIPIENT_FILE"].startswith(BACKUP_KEYS + "/"):
        problems.append("ru_backup.py's host layout differs from ru_host.py's")
    return problems


def ssh_config(alias: str) -> dict | None:
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


def whois_countries(address: str) -> set:
    """Country codes `whois` gives for an address (RIPE answers for Russian networks)."""
    try:
        result = subprocess.run(["whois", address], capture_output=True, text=True, timeout=30)
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return set()
    return {code.upper() for code in re.findall(r"(?im)^country:\s*([a-z]{2})\b", result.stdout)}


def resolve_target(args, need_ip: bool, country: bool = False, config_reader=None, whois=None) -> dict:
    """The RU VPS this run talks to; refuses production and dev before any ssh."""
    alias = args.host or os.environ.get(HOST_ENV) or DEFAULT_HOST
    if not SSH_ALIAS.fullmatch(alias):
        raise Refused(f"ssh alias {alias!r} is not a plain host alias")
    if is_foreign_name(alias):
        raise Refused(f"{alias} is the production or dev VPS; the RU tool only targets the RU VPS")
    dev_ip = ipv4_literal(os.environ.get(DEV_IP_ENV) or "")
    expected_ip = validate_ru_ip(args.ip if args.ip is not None else os.environ.get(IP_ENV), dev_ip)
    config = (config_reader or ssh_config)(alias)
    hostname = (config or {}).get("hostname")
    if hostname and (is_foreign_name(hostname) or hostname == dev_ip):
        raise Refused(f"ssh alias {alias} resolves to the production or dev VPS ({hostname})")
    if need_ip:
        if expected_ip is None:
            raise Refused(f"set {IP_ENV} (or --ip) to the RU VPS IPv4 before --check/--apply")
        if config is None:
            raise Refused(f"`ssh -G {alias}` failed; is {alias} in ~/.ssh/config?")
        if hostname != expected_ip:
            raise Refused(f"ssh alias {alias} connects to {hostname!r}, not {IP_ENV}={expected_ip}; "
                          f"set `HostName {expected_ip}` for Host {alias} in ~/.ssh/config")
        if config.get("user") != "root":
            raise Refused(f"ssh alias {alias} logs in as {config.get('user')!r}; the RU tool logs in as root "
                          f"with a key: set `User root` for Host {alias}")
        if country and not getattr(args, "country_confirmed", False):
            countries = (whois or whois_countries)(expected_ip)
            if MARKET.upper() not in countries:
                found = ", ".join(sorted(countries)) or "no country"
                raise Refused(f"whois gives {found} for {expected_ip}; the RU host must be in Russia (152-FZ). "
                              "Pass --country-confirmed only if the provider confirms the server is in Russia")
    return {"alias": alias, "hostname": hostname, "expected_ip": expected_ip, "dev_ip": dev_ip,
            "behind_nat": bool(getattr(args, "behind_nat", False))}


def base_payload(target: dict, files: dict) -> dict:
    return {"expected_ip": target["expected_ip"], "dev_ip": target["dev_ip"], "behind_nat": target["behind_nat"],
            "units": files["units"], "examples": files["examples"], "caddyfile": files["caddyfile"],
            "names": files["names"], "backup_kit_z": files["backup_kit_z"]}


def ssh_remote(alias: str, mode: str, payload: dict | None = None, check: bool = True):
    command = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", alias, "python3", "-", "remote", mode]
    if payload is not None:
        command.append(encode_payload(payload))
    result = subprocess.run(command, input=Path(__file__).read_bytes(), capture_output=True)
    sys.stderr.write(result.stderr.decode(errors="replace"))
    if check and result.returncode:
        raise SystemExit(f"remote {mode} failed (exit {result.returncode})")
    return result.stdout.decode() if check else (result.returncode, result.stdout.decode())


def upload(alias: str, targets, allowed: dict) -> None:
    """scp each local file to the `.part` path the host asked for; nothing else."""
    for target in targets:
        source = allowed.get(target)
        if source is None:
            raise Refused(f"the host asked for an unexpected upload {target!r}")
        subprocess.run(["scp", "-q", "-o", "BatchMode=yes", str(source), f"{alias}:{target}"], check=True)


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


def read_package(package: Path) -> tuple:
    """(version, {binary: bytes}) from an RU Linux package only."""
    match = RU_PACKAGE.fullmatch(package.name)
    if not match:
        raise Refused(f"{package.name} is not an RU Linux package (omoba-<version>-ru-linux-x64.tar.gz); "
                      "build it with: python3 scripts/release.py build linux --market ru")
    version = match.group(1)
    binaries = {}
    with tarfile.open(package, "r:gz") as archive:
        for name in SERVER_BINARIES:
            member_name = f"omoba-{version}-ru/{name}"
            members = [member for member in archive.getmembers() if member.name == member_name]
            if len(members) != 1 or not members[0].isfile():
                raise Refused(f"{package.name} has no regular file {member_name}")
            data = archive.extractfile(members[0]).read()
            if not is_linux_x86_64_elf(data[:64]):
                raise Refused(f"{member_name} is not a Linux x86-64 executable")
            binaries[name] = data
    return version, binaries


def inspect_bundle(bundle: Path) -> tuple:
    """(problems, warnings) of a portal bundle, from its members (no extraction)."""
    top = bundle.name[:-len(".tar.gz")]
    with tarfile.open(bundle, "r:gz") as archive:
        try:
            members = archive_members(archive, top)
        except Refused as refusal:
            return [refusal.message], []
        paths = {member.name: "link" if member.issym() or member.islnk() else
                 ("dir" if member.isdir() else "file") for member in members}
        for name in list(paths):   # parents of files listed without their own entries
            parts = name.split("/")
            for index in range(1, len(parts)):
                paths.setdefault("/".join(parts[:index]), "dir")
        by_name = {member.name: member for member in members}

        def read(relative: str, size: int) -> bytes:
            return archive.extractfile(by_name[relative]).read(size)
        problems, warnings = portal_problems(paths, read)
    if ".next/cache" in paths:
        problems.append(".next/cache is a build cache; bundle without it")
    return problems, warnings


def plan_or_run(args, target: dict, preflight_payload: dict, plan: dict, apply) -> int:
    if not (args.check or args.apply):
        print(json.dumps({"dry_run": True, "target": target, **plan}, indent=2))
        print("Dry run: nothing was contacted. --check runs the read-only host preflight; --apply changes the host.",
              file=sys.stderr)
        return 0
    preflight = json.loads(ssh_remote(target["alias"], "preflight", preflight_payload))
    print(json.dumps(preflight, indent=2))
    if not args.apply:
        return 0 if preflight.get("ok") else 1
    if not preflight.get("ok"):
        raise Refused("host preflight reported problems; fix them first")
    return apply()


def command_bootstrap(args) -> int:
    files = kit()
    if args.vkid_client_id is not None and not VKID.fullmatch(args.vkid_client_id):
        raise Refused("--vkid-client-id must be the numeric VK ID app id")
    node, tarball = None, None
    if args.node_tarball:
        tarball = Path(args.node_tarball)
        match = NODE_TARBALL.fullmatch(tarball.name)
        if not match:
            raise Refused(f"{tarball.name} is not an official node-v<x.y.z>-linux-x64.tar.xz")
        if tuple(int(part) for part in match.groups()[:2]) < MIN_NODE:
            raise Refused(f"Node.js {'.'.join(match.groups())} is older than {MIN_NODE[0]}.{MIN_NODE[1]} (Next.js 16)")
        digest = sha256_file(str(tarball))
        if not args.node_sha256 or digest != args.node_sha256.strip().lower():
            raise Refused(f"{tarball.name} sha256 {digest} does not match --node-sha256 "
                          "(copy it from nodejs.org/dist/v<x.y.z>/SHASUMS256.txt)")
        node = {"file": tarball.name, "version": ".".join(match.groups()), "sha256": digest}
    target = resolve_target(args, need_ip=args.check or args.apply, country=args.check or args.apply)
    payload = {**base_payload(target, files), "kind": "bootstrap", "vkid": args.vkid_client_id, "node": node}
    plan = {"steps": [
        f"preflight (read-only): not production/dev, Ubuntu {MIN_UBUNTU[0]}.{MIN_UBUNTU[1]:02d}+ x86-64, "
        f"{target['expected_ip']} local, {MARKER} absent or naming it",
        f"apt-get install {' '.join(PACKAGES)}; PostgreSQL >= {MIN_POSTGRES} with {ICU_COLLATION}, loopback only",
        f"Node.js >= {MIN_NODE[0]}.{MIN_NODE[1]} in {NODE}" + (f" from {node['file']}" if node else
                                                                 " (Ubuntu's nodejs when new enough)"),
        f"users {USER}, {PORTAL_USER}; folders under {OPT} and {STATE}",
        f"env files {', '.join(ENV_PATHS.values())} (root 0600; secrets generated on the host, never replaced)",
        f"roles {', '.join(DB_ROLES)}, database {DB_NAME}",
        f"Caddyfile.ru -> {CADDYFILE}; {SSHD_DROPIN} (key-only SSH); ufw: SSH, {HTTPS_PORT}/tcp, "
        f"{LOBBY_PORT}/udp, the arena range",
        "units installed; postgresql, caddy and units with a release enabled; full status at the end"]}

    def apply() -> int:
        if node:
            stage = json.loads(ssh_remote(target["alias"], "stage", {**payload, "kind": "node"}))
            upload(target["alias"], stage.get("upload_to") or [], {f"{INCOMING}/{node['file']}.part": tarball})
        print(ssh_remote(target["alias"], "bootstrap", payload))
        return 0
    return plan_or_run(args, target, payload, plan, apply)


def command_ship(args) -> int:
    files = kit()
    version, binaries = read_package(Path(args.package))
    revision = source_revision(args.revision, version)
    release = validate_server_release(f"{version}-ru-{revision[:7]}")
    grants = validate_grants((repo_root() / GRANTS).read_text())
    digests = {name: hashlib.sha256(data).hexdigest() for name, data in binaries.items()}
    target = resolve_target(args, need_ip=args.check or args.apply)
    build = {"market": MARKET, "release": release, "version": version, "revision": revision,
             "sha256": digests["omoba-server"], "account_api_sha256": digests["omoba-account-api"],
             "grants_sha256": hashlib.sha256(grants.encode()).hexdigest(), "package": Path(args.package).name,
             "package_sha256": sha256_file(args.package), "tool": TOOL}
    payload = {**base_payload(target, files), "kind": "server", "release": release, "grants": grants,
               "sha256": digests["omoba-server"], "account_api_sha256": digests["omoba-account-api"], "build": build,
               "local_backup_only": bool(args.local_backup_only)}
    backup = ("plain rollback dump only (--local-backup-only, before launch)" if args.local_backup_only else
              "encrypted off-host set + rollback dump (ru_backup.py)")
    plan = {"release": release, "build": build, "steps": [
        "preflight (read-only): marker, env files, PostgreSQL, no running match, backups configured",
        f"stage {RELEASES}/{release}/; scp the missing binaries as .part",
        f"activate: sha256; pre-migration backup: {backup}; migrate + grants; {CURRENT} -> {release}; "
        "units if changed; restart; verify; roll back on failure (no-op when already running this release)"]}

    def apply() -> int:
        stage = json.loads(ssh_remote(target["alias"], "stage", payload))
        with tempfile.TemporaryDirectory(prefix="omoba-ru-") as temporary:
            allowed = {}
            for name, data in binaries.items():
                local = Path(temporary) / name
                local.write_bytes(data)
                allowed[f"{RELEASES}/{release}/{name}.part"] = local
            upload(target["alias"], stage.get("upload_to") or [], allowed)
        print(ssh_remote(target["alias"], "activate", payload))
        return 0
    return plan_or_run(args, target, payload, plan, apply)


def command_portal_bundle(args) -> int:
    """Pack an omoba-web standalone build (already built with the RU env) for ship-portal."""
    web = Path(args.web).resolve()
    standalone = web / ".next/standalone"
    if not (standalone / "server.js").is_file() or not (web / ".next/static").is_dir():
        raise Refused(f"{web} has no standalone build; build it with the RU env first (README.md, 'Portal bundle')")
    if subprocess.run(["git", "status", "--porcelain"], cwd=web, capture_output=True, text=True).stdout.strip():
        raise Refused(f"{web} has uncommitted changes; bundle a clean, built checkout")
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=web, text=True).strip()
    name = f"omoba-portal-ru-{revision[:7]}"
    out = Path(args.out) if args.out else repo_root() / "dist/portal-ru"
    out.mkdir(parents=True, exist_ok=True)

    def ignore(directory, names):
        here = Path(directory)
        if here == standalone:
            return [n for n in names if n.startswith(".env") or n == "public"]
        if here == standalone / ".next":
            return [n for n in names if n in ("cache", "static")]
        return []
    with tempfile.TemporaryDirectory(prefix="omoba-portal-ru-") as temporary:
        stage = Path(temporary) / name
        shutil.copytree(standalone, stage, symlinks=True, ignore=ignore)
        shutil.copytree(web / ".next/static", stage / ".next/static", symlinks=True)
        if (web / "public").is_dir():
            shutil.copytree(web / "public", stage / "public", symlinks=True)
        problems, warnings = portal_problems(tree_listing(str(stage)), read_head(str(stage)))
        if problems:
            raise Refused("not the RU portal build: " + "; ".join(problems))
        archive = out / f"{name}.tar.gz"
        with tarfile.open(archive, "w:gz") as bundle:
            bundle.add(str(stage), arcname=name, filter=tar_normalized)
    print(json.dumps({"bundle": str(archive), "sha256": sha256_file(str(archive)), "revision": revision,
                      "warnings": warnings}, indent=2))
    return 0


def command_ship_portal(args) -> int:
    files = kit()
    bundle = Path(args.bundle)
    match = PORTAL_BUNDLE.fullmatch(bundle.name)
    if not match:
        raise Refused(f"{bundle.name} is not a portal bundle (omoba-portal-ru-<rev>.tar.gz from portal-bundle)")
    problems, warnings = inspect_bundle(bundle)
    if problems:
        raise Refused("not the RU portal build: " + "; ".join(problems))
    digest = sha256_file(str(bundle))
    release = validate_portal_release(f"{match.group(1)[:7]}-{digest[:8]}")
    target = resolve_target(args, need_ip=args.check or args.apply)
    payload = {**base_payload(target, files), "kind": "portal", "release": release, "file": bundle.name,
               "sha256": digest}
    plan = {"release": release, "bundle": bundle.name, "sha256": digest, "warnings": warnings, "steps": [
        "preflight (read-only): marker, Node.js, portal.env",
        f"scp the bundle to {INCOMING}/{bundle.name}.part",
        f"activate: sha256; unpack into {PORTAL_RELEASES}/{release}; RU production build check; "
        f"{PORTAL_CURRENT} -> {release}; restart; verify; roll back on failure"]}

    def apply() -> int:
        stage = json.loads(ssh_remote(target["alias"], "stage", payload))
        upload(target["alias"], stage.get("upload_to") or [], {f"{INCOMING}/{bundle.name}.part": bundle})
        print(ssh_remote(target["alias"], "activate", payload))
        return 0
    return plan_or_run(args, target, payload, plan, apply)


def command_rollback(args) -> int:
    files = kit()
    if args.kind == "server":
        validate_server_release(args.release)
    else:
        validate_portal_release(args.release)
    target = resolve_target(args, need_ip=args.check or args.apply)
    payload = {**base_payload(target, files), "kind": args.kind, "release": args.release}
    if args.kind == "server":
        payload["units"] = {kind: text for kind, text in payload["units"].items() if kind in SERVER_KINDS}
    plan = {"steps": [f"point the {args.kind} at the staged release {args.release} (no migration), "
                      "restart, verify, roll back on failure"]}

    def apply() -> int:
        print(ssh_remote(target["alias"], "rollback", payload))
        return 0
    return plan_or_run(args, target, payload, plan, apply)


def command_escrow(args, run=subprocess.run) -> int:
    """The host's secrets, encrypted on the host to the escrow holder's public key, saved here
    (LR-03). Plaintext never leaves the host; the holder keeps the file offline."""
    recipient = Path(args.recipient_file).read_text()
    if "BEGIN PGP PUBLIC KEY BLOCK" not in recipient or "PRIVATE KEY" in recipient:
        raise Refused(f"{args.recipient_file} must be the escrow holder's armored PUBLIC key")
    out = Path(args.out)
    if out.exists():
        raise Refused(f"{out} exists; escrow files are never overwritten")
    target = resolve_target(args, need_ip=True)
    result = run(["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", target["alias"], PYTHON, BACKUP_SCRIPT,
                  "escrow", "--recipient-file", "-"], input=recipient.encode(), capture_output=True)
    sys.stderr.write(result.stderr.decode(errors="replace"))
    if result.returncode or not result.stdout.startswith(b"-----BEGIN PGP MESSAGE-----"):
        raise SystemExit(f"escrow on {target['alias']} failed (exit {result.returncode})")
    descriptor = os.open(out, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "wb") as handle:
        handle.write(result.stdout)
    print(json.dumps({"escrow": str(out), "bytes": len(result.stdout),
                      "sha256": hashlib.sha256(result.stdout).hexdigest(),
                      "next": "give it to the escrow holder for offline storage; the holder checks it with "
                              "`gpg --decrypt <file> | tar -tzv`; delete this copy afterwards"}, indent=2))
    return 0


def command_status(args) -> int:
    target = resolve_target(args, need_ip=True)
    report = json.loads(ssh_remote(target["alias"], "status", base_payload(target, kit())))
    print(json.dumps(report, indent=2))
    return 0 if report.get("ok") else 1


def command_names(args) -> int:
    """The RU public names (scripts/markets.py) and the DNS records the owner sets. Local only."""
    names = ru_names()
    print(json.dumps({"market": MARKET, "source": "scripts/markets.py",
                      "lobby": f"{names['lobby']}:{LOBBY_PORT}", "account_api": f"https://{names['account_api']}",
                      "portal": f"https://{names['portal']}", "zone": names["zone"],
                      "dns": {"A": list(dict.fromkeys(names[key] for key in HOST_KEYS)),
                              "points_to": f"the RU VPS IPv4 (${IP_ENV})", "no_AAAA": [names["lobby"]]}}, indent=2))
    return 0


def command_reboot_check(args, sleep=time.sleep, clock=time.monotonic) -> int:
    """Reboot the RU host and prove every service comes back with no manual step."""
    target = resolve_target(args, need_ip=True)
    payload = base_payload(target, kit())
    alias = target["alias"]
    before = json.loads(ssh_remote(alias, "status", payload))
    if not args.apply:
        print(json.dumps({"dry_run": True, "status_ok": before.get("ok"), "problems": before.get("problems"),
                          "steps": ["require a clean status", "systemctl reboot", "wait for a new boot id",
                                    f"status until ok (up to {args.ready_timeout} s)"]}, indent=2))
        return 0 if before.get("ok") else 1
    if not before.get("ok"):
        raise Refused("status reports problems; fix them before the reboot test: " + "; ".join(before["problems"]))
    old_boot = json.loads(ssh_remote(alias, "boot-id"))["boot_id"]
    try:   # the connection drops while the host goes down
        subprocess.run(["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", alias, "systemctl", "reboot"],
                       capture_output=True, timeout=60)
    except subprocess.TimeoutExpired:
        pass
    started = clock()
    new_boot = None
    while clock() - started < args.boot_timeout:
        sleep(10)
        code, out = ssh_remote(alias, "boot-id", check=False)
        try:
            boot = json.loads(out).get("boot_id") if code == 0 else None
        except ValueError:
            boot = None
        if boot and boot != old_boot:
            new_boot = boot
            break
    if new_boot is None:
        raise SystemExit(f"{alias} did not come back with a new boot id within {args.boot_timeout} s")
    back = clock() - started
    report = None
    while clock() - started < back + args.ready_timeout:
        code, out = ssh_remote(alias, "status", payload, check=False)
        try:
            report = json.loads(out) if code == 0 else None
        except ValueError:
            report = None
        if report and report.get("ok"):
            break
        sleep(10)
    result = {"rebooted": True, "seconds_to_ssh": round(back), "seconds_to_ready": round(clock() - started),
              "ok": bool(report and report.get("ok")), "status": report}
    print(json.dumps(result, indent=2))
    return 0 if result["ok"] else 1


def main(argv=None) -> int:
    argv = sys.argv[1:] if argv is None else argv
    if argv[:1] == ["remote"]:
        return remote_main(argv[1:])
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    bootstrap = sub.add_parser("bootstrap", help="set up a fresh Ubuntu host (dry run unless --check/--apply)")
    bootstrap.add_argument("--vkid-client-id", help="numeric VK ID app id (LR-19); fills both env files")
    bootstrap.add_argument("--node-tarball", help="official node-v<x.y.z>-linux-x64.tar.xz (>= 20.9)")
    bootstrap.add_argument("--node-sha256", help="its sha256 from nodejs.org/dist/v<x.y.z>/SHASUMS256.txt")
    bootstrap.add_argument("--country-confirmed", action="store_true",
                           help="the provider confirms the server is in Russia although whois does not say RU")
    ship = sub.add_parser("ship", help="install or update the RU lobby and Account API from the RU Linux package")
    ship.add_argument("--package", required=True, help="omoba-<v>-ru-linux-x64.tar.gz from release.py --market ru")
    ship.add_argument("--revision", help="source commit (default: HEAD of this clean checkout)")
    ship.add_argument("--local-backup-only", action="store_true",
                      help="before launch, while backup.env names no storage: only a plain rollback dump on the "
                           "host before the migration (LR-03)")
    bundle = sub.add_parser("portal-bundle", help="pack an omoba-web RU standalone build (local only)")
    bundle.add_argument("--web", required=True, help="the omoba-web checkout, built with the RU env")
    bundle.add_argument("--out", help="output folder (default: dist/portal-ru)")
    portal = sub.add_parser("ship-portal", help="install or update the RU portal from a portal bundle")
    portal.add_argument("--bundle", required=True, help="omoba-portal-ru-<rev>.tar.gz from portal-bundle")
    rollback = sub.add_parser("rollback", help="point the lobby/Account API or the portal at an earlier release")
    rollback.add_argument("--kind", required=True, choices=("server", "portal"))
    rollback.add_argument("--release", required=True)
    status = sub.add_parser("status", help="read-only check of the whole RU host")
    reboot = sub.add_parser("reboot-check", help="reboot the RU host and verify everything comes back")
    reboot.add_argument("--apply", action="store_true", help="really reboot (default: status and plan only)")
    reboot.add_argument("--boot-timeout", type=int, default=600)
    reboot.add_argument("--ready-timeout", type=int, default=300)
    sub.add_parser("names", help="print the RU public names from scripts/markets.py and the DNS records "
                                 "to set (local only)")
    escrow = sub.add_parser("escrow", help="encrypt the host's secrets to the escrow holder's public key and "
                                           "save them here (LR-03)")
    escrow.add_argument("--recipient-file", required=True, help="the escrow holder's armored public key")
    escrow.add_argument("--out", required=True, help="where to save the encrypted file (must not exist)")
    for command in (bootstrap, ship, portal, rollback, status, reboot, escrow):
        command.add_argument("--host", help=f"ssh alias of the RU VPS (default: ${HOST_ENV} or {DEFAULT_HOST})")
        command.add_argument("--ip", help=f"RU VPS IPv4 the alias must connect to (default: ${IP_ENV})")
        command.add_argument("--behind-nat", action="store_true",
                             help="the RU address is the provider's NAT, not on a local interface")
    for command in (bootstrap, ship, portal, rollback):
        mode = command.add_mutually_exclusive_group()
        mode.add_argument("--check", action="store_true", help="run the read-only host preflight only")
        mode.add_argument("--apply", action="store_true", help="change the host")
    args = parser.parse_args(argv)
    return {"bootstrap": command_bootstrap, "ship": command_ship, "portal-bundle": command_portal_bundle,
            "ship-portal": command_ship_portal, "rollback": command_rollback, "status": command_status,
            "reboot-check": command_reboot_check, "names": command_names,
            "escrow": command_escrow}[args.command](args)


if __name__ == "__main__":
    sys.exit(main())
