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
/var/lib/omoba, /opt/ekza-studio, /opt/ekza-mirror, the NPM edge, address
77.246.105.57; the dev host's own /opt/ekza-dev is not a marker), only writes
under /opt/omoba-dev, /var/lib/omoba-dev and its own unit file, never
interrupts a running dev match, and rolls back to the previous release on any
failed start. Re-shipping the active release with the same unit is a no-op.

Career database. A public lobby refuses to start without OMOBA_DATABASE_URL
(server/src/runtime/mod.rs), and without storage it cannot queue anybody:
matchmaking needs a signed account profile. The dev lobby therefore gets its own
database `omoba_dev` and roles omoba_dev_{owner,game,portal} in the dev Supabase
stack's Postgres on this host (container supabase_db_ekza-dev, published on
127.0.0.1:54322 only). Activation creates them when missing, generates the game
role's password on the host into /opt/omoba-dev/database.env (root, 0600; the
unit's only EnvironmentFile), runs the release's `omoba-account-api migrate`
under a temporary, expiring owner login, applies account-api/ops/grants.sql as
the owner and disables the owner login again. The file must name a loopback
database URL for omoba_dev_game@omoba_dev and nothing else; a production
Supabase name, the production VPS or production role names are refused. No
secret is printed, passed in argv or written outside that file.
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
import socket
import subprocess
import sys
import tarfile
import tempfile
import time
from pathlib import Path
from urllib.parse import unquote, urlsplit

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
# No dev Ekza passport exists yet; the SDK default (avatar.ekza.io) is production. A closed
# loopback port makes protected-avatar tickets fail closed. Mirrors scripts/release.py DEV_PASSPORT.
PASSPORT = "http://127.0.0.1:9/api/passport"
DEFAULT_PUBLIC_HOST = "lobby.dev.omoba.io"
PUBLIC_HOST_KEY = "OMOBA_MATCH_PUBLIC_HOST"
MIN_AVAILABLE_MIB = 600  # MemoryMax=512M plus headroom for the rest of the dev host
DEV_HOSTNAME = re.compile(r"(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+dev\.(?:omoba|ekza)\.io")

# Release directory contents: the lobby binary and its schema migration tool (never a service).
ACCOUNT_API = "omoba-account-api"
RELEASE_BINARIES = ("server", ACCOUNT_API)
GRANTS = "account-api/ops/grants.sql"

# Career database in the dev Supabase stack's Postgres (Supabase CLI project ekza-dev).
DATABASE_ENV = OPT + "/database.env"
DATABASE_URL_KEY = "OMOBA_DATABASE_URL"
DB_CONTAINER = "supabase_db_ekza-dev"
DB_PORT = 54322
DB_ADMIN = "supabase_admin"  # superuser over the container's local socket (trust)
DB_NAME = "omoba_dev"
DB_OWNER, DB_GAME, DB_PORTAL = "omoba_dev_owner", "omoba_dev_game", "omoba_dev_portal"
DB_ROLES = (DB_OWNER, DB_GAME, DB_PORTAL)
DB_LOOPBACK = frozenset({"127.0.0.1", "localhost", "::1"})
DB_PASSWORD = re.compile(r"[A-Za-z0-9_-]{24,128}")
MIGRATION_LOGIN_SECONDS = 900

# ---------------------------------------------------------------- production (never a target)

PROD_SSH_ALIASES = frozenset({"vds-eternal"})
PROD_IPS = frozenset({"77.246.105.57"})
# Every omoba.io / ekza.io name outside the dev zones is production (as in scripts/release.py),
# including names not listed anywhere yet, such as avatar.ekza.io (the production passport).
PUBLIC_ZONES = ("omoba.io", "ekza.io")
DEV_ZONES = ("dev.omoba.io", "dev.ekza.io")
PROD_DB_NAMES = frozenset({"omoba"})
PROD_DB_ROLES = frozenset({"omoba_owner", "omoba_game", "omoba_portal"})
PROD_DB_PORTS = frozenset({2003})
PROD_UNIT = "omoba-lobby"
PROD_USER = "omoba"
PROD_UNIT_PATH = "/etc/systemd/system/omoba-lobby.service"
PROD_PATHS = ("/opt/omoba", "/var/lib/omoba", "/opt/ekza-studio", "/opt/ekza-mirror",
              "/root/docker/nginx-proxy-manager", PROD_UNIT_PATH)
PROD_PORTS = frozenset({4000, 41000, 41001})
# Forbidden-value guard only: the production Supabase project and hosted Supabase must never
# appear in the unit or the database URL.
FORBIDDEN_TOKENS = ("snrjxwutqxokeujuiepn", "supabase.co", "supabase.com")

RELEASE_NAME = re.compile(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?-dev-[0-9a-f]{7,40}")
DEV_PACKAGE = re.compile(r"omoba-(\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?)-dev-linux-x64\.tar\.gz")
PAYLOAD = re.compile(r"[A-Za-z0-9+/=]*")
SSH_ALIAS = re.compile(r"(?:[a-z_][a-z0-9_-]*@)?[A-Za-z0-9][A-Za-z0-9._-]*")

# Exact values the dev unit must carry; OMOBA_MATCH_PUBLIC_HOST is checked separately.
UNIT_SERVICE = {
    "User": USER, "Group": USER, "WorkingDirectory": STATE, "EnvironmentFile": DATABASE_ENV,
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
    "OMOBA_PASSPORT_URL": PASSPORT,
}
# The database URL is a secret and lives only in DATABASE_ENV; EKZA_PASSPORT_URL is the
# SDK-wide passport fallback and has no place in the dev unit.
UNIT_FORBIDDEN_ENV = (DATABASE_URL_KEY, "EKZA_PASSPORT_URL")


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


def in_zone(name: str, zones) -> bool:
    return any(name == zone or name.endswith("." + zone) for zone in zones)


def is_production_name(value: str) -> bool:
    """A production ssh alias, address, or any omoba.io / ekza.io name outside the dev zones
    (case and trailing dot ignored)."""
    name = (value or "").strip().lower().rstrip(".")
    if "@" in name:
        name = name.rsplit("@", 1)[1]
    return name in PROD_SSH_ALIASES or name in PROD_IPS or \
        (in_zone(name, PUBLIC_ZONES) and not in_zone(name, DEV_ZONES))


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
    if touches_production(DATABASE_ENV) or not within(DATABASE_ENV, OPT):
        raise Refused(f"dev database file {DATABASE_ENV} is outside {OPT}")
    if DB_NAME in PROD_DB_NAMES or set(DB_ROLES) & PROD_DB_ROLES or DB_PORT in PROD_DB_PORTS:
        raise Refused("dev database name, roles or port equal production ones")
    if urlsplit(PASSPORT).hostname not in DB_LOOPBACK:
        raise Refused(f"dev passport {PASSPORT} is not the closed loopback placeholder")


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
    for key in UNIT_FORBIDDEN_ENV:
        if key in environment:
            errors.append(f"Environment {key} is not allowed in the dev unit "
                          f"(the dev database URL lives only in {DATABASE_ENV})")
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
        errors.append("the unit mentions the production Supabase project or hosted Supabase")
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


def release_files(payload: dict, require_tool: bool = True) -> dict[str, str]:
    """{file in the release directory: expected sha256}. Old rollback records lack the tool."""
    files = {"server": payload.get("sha256")}
    if payload.get("account_api_sha256") or require_tool:
        files[ACCOUNT_API] = payload.get("account_api_sha256")
    for name, digest in files.items():
        if not (isinstance(digest, str) and re.fullmatch(r"[0-9a-f]{64}", digest)):
            raise Refused(f"payload lacks the sha256 of {name}")
    return files


# ---------------------------------------------------------------- career database

def _printable(value: str) -> str:
    """Only short, plain identifiers are echoed; anything else could be a pasted secret."""
    return value if re.fullmatch(r"[A-Za-z0-9._:\[\]-]{1,64}", value or "") else "(not shown)"


def database_url(user: str, password: str, database: str = DB_NAME) -> str:
    return f"postgres://{user}:{password}@127.0.0.1:{DB_PORT}/{database}"


def database_env_text(url: str) -> str:
    return (f"# OMOBA DEV lobby career database: written on the dev VPS by ops/vps/dev/lobby_dev.py.\n"
            f"# root:root 0600, read by systemd for {UNIT}. Never copy it off this host.\n"
            f"{DATABASE_URL_KEY}={url}\n")


def database_url_problems(url: str) -> list[str]:
    """Why `url` is not the dev lobby's loopback omoba_dev_game@omoba_dev URL; never echoes it."""
    lowered = (url or "").lower()
    problems = []
    if any(token in lowered for token in FORBIDDEN_TOKENS) or any(ip in lowered for ip in PROD_IPS):
        problems.append("names production (hosted Supabase, the production project or the production VPS)")
    try:
        parts = urlsplit(url or "")
        port = parts.port
    except ValueError:
        return problems + ["is not a loopback postgres URL with a role, a password, a port and a database"]
    if parts.scheme not in ("postgres", "postgresql"):
        problems.append("is not a postgres:// URL")
    host = (parts.hostname or "").lower()
    if host not in DB_LOOPBACK:
        problems.append(f"host {_printable(host)} is not loopback; the dev career database runs on this host")
    if port != DB_PORT:
        problems.append(f"port {port} is not the dev Supabase database port {DB_PORT}")
    user = unquote(parts.username or "")
    if user in PROD_DB_ROLES or user != DB_GAME:
        problems.append(f"user {_printable(user)} is not {DB_GAME}")
    if not DB_PASSWORD.fullmatch(unquote(parts.password or "")):
        problems.append("password must be 24-128 characters of [A-Za-z0-9_-]")
    database = unquote(parts.path.lstrip("/"))
    if database in PROD_DB_NAMES or database != DB_NAME:
        problems.append(f"database {_printable(database)} is not {DB_NAME}")
    if parts.query or parts.fragment or any(ch.isspace() for ch in url or ""):
        problems.append("must not carry query parameters, a fragment or spaces")
    return problems


def parse_database_env(text: str) -> tuple[str | None, list[str]]:
    """(URL, problems) of a database.env; only one OMOBA_DATABASE_URL line is allowed, because
    EnvironmentFile settings override the unit's Environment= (passport, registry, ports)."""
    url, problems = None, []
    for number, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith(("#", ";")):
            continue
        key, separator, value = line.partition("=")
        if not separator or key.strip() != DATABASE_URL_KEY:
            problems.append(f"line {number}: only {DATABASE_URL_KEY}=... is allowed")
            continue
        if url is not None:
            problems.append(f"{DATABASE_URL_KEY} appears more than once")
            continue
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in "'\"":
            value = value[1:-1]
        url = value
    if url is None:
        problems.append(f"{DATABASE_URL_KEY} is missing")
    else:
        problems += [f"{DATABASE_URL_KEY} {problem}" for problem in database_url_problems(url)]
    return url, problems


def password_of(url: str) -> str:
    return unquote(urlsplit(url).password or "")


def redact(text: str, hidden=()) -> str:
    for value in hidden:
        if value:
            text = text.replace(value, "<redacted>")
    text = re.sub(r"(?i)(password\s+)'[^']*'", r"\1'<redacted>'", text)
    return re.sub(r"://([^:/@\s]+):[^@\s]*@", r"://\1:<redacted>@", text)


def sql_secret(value: str) -> str:
    if not DB_PASSWORD.fullmatch(value):
        raise Refused("internal: a database password with unexpected characters")
    return f"'{value}'"


STATE_SQL = f"""-- lobby_dev:state
SELECT json_build_object(
  'roles', (SELECT coalesce(json_object_agg(rolname, rolcanlogin), '{{}}'::json) FROM pg_roles
            WHERE rolname IN ('{DB_OWNER}', '{DB_GAME}', '{DB_PORTAL}')),
  'database_owner', (SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname = '{DB_NAME}'),
  'superuser', (SELECT rolsuper FROM pg_roles WHERE rolname = current_user));
"""
SCHEMA_SQL = """-- lobby_dev:schema
SELECT to_regclass('public.career_schema_version') IS NOT NULL
   AND to_regclass('portal.schema_version') IS NOT NULL;
"""
OWNER_NOLOGIN_SQL = f"""-- lobby_dev:owner-nologin
ALTER ROLE {DB_OWNER} NOLOGIN PASSWORD NULL VALID UNTIL 'infinity';
"""


def bootstrap_sql(game_password: str) -> str:
    """Roles and database when missing (idempotent); the game role's password from database.env."""
    roles = ", ".join(f"'{role}'" for role in DB_ROLES)
    return f"""-- lobby_dev:bootstrap
SELECT format('CREATE ROLE %I NOLOGIN', r) FROM unnest(ARRAY[{roles}]) AS r
 WHERE NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = r) \\gexec
SELECT 'CREATE DATABASE {DB_NAME} OWNER {DB_OWNER}'
 WHERE NOT EXISTS (SELECT 1 FROM pg_database WHERE datname = '{DB_NAME}') \\gexec
REVOKE ALL ON DATABASE {DB_NAME} FROM PUBLIC;
GRANT CONNECT ON DATABASE {DB_NAME} TO {DB_GAME}, {DB_PORTAL};
ALTER ROLE {DB_OWNER} NOLOGIN;
ALTER ROLE {DB_GAME} LOGIN PASSWORD {sql_secret(game_password)};
"""


def owner_login_sql(password: str, until: str) -> str:
    if not re.fullmatch(r"\d{4}-\d\d-\d\d \d\d:\d\d:\d\d\+00", until):
        raise Refused("internal: unexpected VALID UNTIL timestamp")
    return f"""-- lobby_dev:owner-login
ALTER ROLE {DB_OWNER} LOGIN PASSWORD {sql_secret(password)} VALID UNTIL '{until}';
"""


def grants_sql(grants: str) -> str:
    """account-api/ops/grants.sql run as the schema owner, as in production."""
    return (f"-- lobby_dev:grants\nSET ROLE {DB_OWNER};\n\\set portal_role {DB_PORTAL}\n"
            f"\\set game_role {DB_GAME}\n{grants}\nRESET ROLE;\n")


def validate_grants(grants) -> str:
    if not isinstance(grants, str) or not 0 < len(grants) < 65536 or ':"game_role"' not in grants:
        raise Refused(f"payload lacks {GRANTS}")
    return grants


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

    def __init__(self, root: str = "/", run=None, sleep=time.sleep, resolve=_resolve, root_uid: int = 0):
        self.root = root
        self.run = run or self._run
        self.sleep = sleep
        self.resolve = resolve
        self.root_uid = root_uid  # owner database.env must have; tests use their own uid
        self.mutating = False
        assert_dev_constants()

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

    # ---- career database (dev Supabase Postgres on this host)

    def psql(self, database: str, sql: str, hidden=(), check: bool = True):
        """SQL on stdin (never argv) as the container's local superuser."""
        if database not in ("postgres", DB_NAME):
            raise Refused(f"internal: psql on database {database}")
        result = self.run(["docker", "exec", "-i", DB_CONTAINER, "psql", "-X", "-q", "-At",
                           "-v", "ON_ERROR_STOP=1", "-U", DB_ADMIN, "-d", database], check=False, input=sql)
        if check and result.returncode:
            detail = redact(result.stderr.strip() or result.stdout.strip(), hidden)
            raise SystemExit(f"psql on {DB_CONTAINER}/{database} failed: {detail}")
        return result

    def db_container_problems(self) -> list[str]:
        running = self.run(["docker", "inspect", "-f", "{{.State.Running}}", DB_CONTAINER], check=False)
        if running.returncode or running.stdout.strip() != "true":
            return [f"the dev Supabase database container {DB_CONTAINER} is not running; start the "
                    "dev Supabase stack (project ekza-dev) on this host first"]
        published = set(self.run(["docker", "port", DB_CONTAINER, "5432/tcp"], check=False).stdout.split())
        expected = f"127.0.0.1:{DB_PORT}"
        problems = []
        if expected not in published:
            problems.append(f"{DB_CONTAINER} does not publish 5432/tcp on {expected}")
        if published - {expected}:
            problems.append(f"{DB_CONTAINER} also publishes 5432/tcp on {sorted(published - {expected})}; "
                            "the dev database must be reachable on loopback only")
        return problems

    def db_state(self) -> dict | None:
        """Roles, database owner and schema presence; None when the database cannot be queried."""
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

    def database_ready(self) -> bool:
        """Roles, database and schema all exist (e.g. not lost with a reset Supabase volume)."""
        state = None if self.db_container_problems() else self.db_state()
        return bool(state and state.get("schema") and state.get("database_owner") == DB_OWNER
                    and state["roles"].get(DB_GAME))

    def database_env(self) -> dict:
        """{exists, url (only when valid), problems} for database.env; never returns a bad URL."""
        path = self.p(DATABASE_ENV)
        if os.path.islink(path):
            return {"exists": True, "url": None, "problems": ["is a symlink"]}
        if not os.path.lexists(path):
            return {"exists": False, "url": None, "problems": []}
        if not os.path.isfile(path):
            return {"exists": True, "url": None, "problems": ["is not a regular file"]}
        info = os.stat(path)
        problems = []
        if info.st_uid != self.root_uid:
            problems.append("is not owned by root")
        if info.st_mode & 0o077:
            problems.append(f"has mode {oct(info.st_mode & 0o777)}; it must be 0600")
        try:
            text = Path(path).read_text()
        except OSError:
            return {"exists": True, "url": None, "problems": problems + ["is not readable here"]}
        url, more = parse_database_env(text)
        problems += more
        return {"exists": True, "url": None if problems else url, "problems": problems}

    def database_report(self) -> dict:
        """Read-only: what activation would do to the dev career database, and what blocks it."""
        container = self.db_container_problems()
        env = self.database_env()
        problems = container + [f"{DATABASE_ENV} {problem}" for problem in env["problems"]]
        state = None if container else self.db_state()
        would = []
        if not container:
            if state is None:
                problems.append(f"cannot query {DB_CONTAINER} as {DB_ADMIN} over its local socket")
            else:
                owner = state.get("database_owner")
                if owner not in (None, DB_OWNER):
                    problems.append(f"database {DB_NAME} exists and is owned by {_printable(owner)}, "
                                    f"not {DB_OWNER}")
                missing = [role for role in DB_ROLES if role not in state["roles"]]
                if missing:
                    would.append(f"create roles {', '.join(missing)} (NOLOGIN) in {DB_CONTAINER}")
                if owner is None:
                    would.append(f"create database {DB_NAME} owned by {DB_OWNER}")
        if not env["exists"]:
            would.append(f"generate the {DB_GAME} password on this host into {DATABASE_ENV} (root, 0600)")
        would.append(f"{DB_GAME} LOGIN with that password; migrate {DB_NAME} with the release's "
                     f"{ACCOUNT_API} under a temporary owner login; apply {GRANTS}; owner NOLOGIN")
        return {"problems": problems, "would": would,
                "env_file": "missing" if not env["exists"] else ("invalid" if env["problems"] else "ok"),
                "roles": (state or {}).get("roles"), "database_owner": (state or {}).get("database_owner"),
                "schema": (state or {}).get("schema")}

    def write_database_env(self, password: str) -> None:
        self.write_guard(DATABASE_ENV)
        url = database_url(DB_GAME, password)
        if database_url_problems(url):
            raise Refused("internal: the generated database URL fails its own check")
        temporary = self.p(DATABASE_ENV + ".next")
        if os.path.lexists(temporary):
            os.unlink(temporary)
        descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        try:
            os.write(descriptor, database_env_text(url).encode())
            if os.geteuid() == 0:
                os.fchown(descriptor, 0, 0)
            os.fchmod(descriptor, 0o600)
        finally:
            os.close(descriptor)
        os.replace(temporary, self.p(DATABASE_ENV))

    def migrate_database(self, directory: str, grants: str) -> None:
        """`omoba-account-api migrate` as the owner through a temporary, expiring login."""
        temporary = secrets.token_hex(24)
        until = time.strftime("%Y-%m-%d %H:%M:%S+00", time.gmtime(time.time() + MIGRATION_LOGIN_SECONDS))
        try:
            self.psql("postgres", owner_login_sql(temporary, until), hidden=(temporary,))
            result = self.run([self.p(f"{directory}/{ACCOUNT_API}"), "migrate"], check=False, timeout=600,
                              env={"PATH": "/usr/sbin:/usr/bin:/sbin:/bin",
                                   DATABASE_URL_KEY: database_url(DB_OWNER, temporary)})
            if result.returncode:
                detail = redact(result.stderr.strip() or result.stdout.strip(), (temporary,))
                raise SystemExit(f"{ACCOUNT_API} migrate failed: {detail}")
            self.psql(DB_NAME, grants_sql(grants), hidden=(temporary,))
        finally:
            self.psql("postgres", OWNER_NOLOGIN_SQL, check=False)

    def prepare_database(self, directory: str, grants: str) -> dict:
        """Roles, database, database.env and schema for the release about to start."""
        problems = self.db_container_problems()
        if problems:
            raise Refused("dev career database: " + "; ".join(problems))
        env = self.database_env()
        if env["problems"]:
            raise Refused(f"{DATABASE_ENV} " + "; ".join(env["problems"])
                          + "; fix it, or remove it so a new password is generated")
        state = self.db_state()
        if state is None:
            raise Refused(f"cannot query {DB_CONTAINER} as {DB_ADMIN} over its local socket")
        if state.get("database_owner") not in (None, DB_OWNER):
            raise Refused(f"database {DB_NAME} exists and is owned by {_printable(state['database_owner'])}")
        self.write_guard(DATABASE_ENV)
        password = password_of(env["url"]) if env["exists"] else secrets.token_hex(32)
        if not env["exists"]:
            self.write_database_env(password)
        self.psql("postgres", bootstrap_sql(password), hidden=(password,))
        self.migrate_database(directory, grants)
        written = self.database_env()
        if written["problems"] or not written["exists"]:
            raise SystemExit(f"{DATABASE_ENV} is not valid after preparation: {written['problems']}")
        return {"database": DB_NAME, "env_written": not env["exists"],
                "created_roles": [role for role in DB_ROLES if role not in state["roles"]],
                "created_database": state.get("database_owner") is None}

    def require_database_env(self) -> dict:
        env = self.database_env()
        if env["problems"] or not env["exists"]:
            raise Refused(f"{DATABASE_ENV} " + ("; ".join(env["problems"]) or "is missing")
                          + "; ship a release (not a rollback) to prepare the dev career database")
        return {"database": DB_NAME, "env_written": False}

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

    def verify_release(self, release: str, files: dict[str, str]) -> str:
        directory = self.release_dir(release)
        for name, expected in files.items():
            path = self.p(f"{directory}/{name}")
            if os.path.islink(path) or not os.path.isfile(path):
                raise Refused(f"{directory}/{name} is missing or a symlink")
            with open(path, "rb") as handle:
                if not is_linux_x86_64_elf(handle.read(64)):
                    raise Refused(f"{directory}/{name} is not a Linux x86-64 executable")
            actual = sha256_file(path)
            if actual != expected:
                raise Refused(f"{directory}/{name} sha256 {actual} != expected {expected}")
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
            "database": self.database_report(),
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
        database = self.database_report()
        problems += database["problems"]
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
                if self.database_ready() and not database["problems"]:
                    would.append(f"nothing: {UNIT} already runs {release}")
                else:
                    would += database["would"] + [f"keep {UNIT} running {release} (no restart)"]
            else:
                would += database["would"]
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
        files = release_files(payload)
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
        missing = []
        for name, digest in files.items():
            path = self.p(f"{directory}/{name}")
            if os.path.lexists(path):
                if not os.path.isfile(path) or os.path.islink(path) or sha256_file(path) != digest:
                    raise Refused(f"{directory} already holds a different {name}; releases are immutable")
            else:
                missing.append(name)
        if not missing:
            return {"mode": "stage", "release": payload["release"], "staged": "already", "upload_to": []}
        self._install_dir(directory)
        return {"mode": "stage", "release": payload["release"],
                "upload_to": [f"{directory}/{name}.part" for name in missing]}

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

    def _finalize_upload(self, directory: str, payload: dict, files: dict[str, str]) -> None:
        for name, digest in files.items():
            part, final = self.p(f"{directory}/{name}.part"), self.p(f"{directory}/{name}")
            if os.path.isfile(part) and not os.path.lexists(final):
                self.write_guard(f"{directory}/{name}.part")
                if sha256_file(part) != digest:
                    os.unlink(part)
                    raise Refused(f"uploaded {name}.part does not match the local sha256; removed it")
                os.chmod(part, 0o755)
                os.replace(part, final)
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
        files = release_files(payload, require_tool=mode != "rollback")
        grants = validate_grants(payload.get("grants")) if mode != "rollback" else None
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
        self._finalize_upload(directory, payload, files)
        self.verify_release(release, files)
        if previous == release and not unit_changed and self.running_release_ok(directory):
            if grants is None or self.database_ready():
                return {"status": "UNCHANGED", "mode": mode, "release": release, "pid": self.dev_main_pid()}
            # The release runs but its database is gone (a reset dev Supabase volume): rebuild it;
            # the lobby's career worker reconnects by itself.
            database = self.prepare_database(directory, grants)
            record = {"mode": mode, "release": release, "previous": previous, "pid": self.dev_main_pid(),
                      "database": database, "at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}
            self._record(record)
            return {"status": "DATABASE_PREPARED", **record}
        self.guard_no_matches()
        self.guard_ports()
        self.guard_memory()
        # Before the running lobby is touched: a failure here leaves it as it was. A rollback
        # never runs an older release's migrations against a newer schema.
        database = self.require_database_env() if grants is None else self.prepare_database(directory, grants)
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
                  "sha256": payload["sha256"], "account_api_sha256": files.get(ACCOUNT_API),
                  "unit_changed": unit_changed, "public_host": public_host, "database": database,
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
        return self.activate({"release": release, "sha256": info["sha256"],
                              "account_api_sha256": info.get("account_api_sha256"), "unit": None,
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


# Package member -> release directory file.
PACKAGE_MEMBERS = {"omoba-server": "server", ACCOUNT_API: ACCOUNT_API}


def read_package(package: Path) -> tuple[str, dict[str, bytes], str]:
    """(version, {release file: bytes}, package sha256) from a dev Linux package only."""
    match = DEV_PACKAGE.fullmatch(package.name)
    if not match:
        raise Refused(f"{package.name} is not a dev Linux package (omoba-<version>-dev-linux-x64.tar.gz); "
                      "build it with: python3 scripts/release.py build linux --contour dev")
    version = match.group(1)
    binaries = {}
    with tarfile.open(package, "r:gz") as archive:
        for member, name in PACKAGE_MEMBERS.items():
            member_name = f"omoba-{version}-dev/{member}"
            members = [m for m in archive.getmembers() if m.name == member_name]
            if len(members) != 1 or not members[0].isfile():
                raise Refused(f"{package.name} has no regular file {member_name} "
                              "(rebuild it with this checkout's scripts/release.py)")
            data = archive.extractfile(members[0]).read()
            if not is_linux_x86_64_elf(data[:64]):
                raise Refused(f"{member_name} is not a Linux x86-64 executable")
            binaries[name] = data
    return version, binaries, sha256_file(str(package))


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
        version, binaries, package_sha = read_package(Path(args.package))
        package_name = Path(args.package).name
    else:
        if not args.version or not args.account_api:
            raise Refused("--binary needs --version and --account-api")
        binaries = {"server": Path(args.binary).read_bytes(), ACCOUNT_API: Path(args.account_api).read_bytes()}
        for name, data in binaries.items():
            if not is_linux_x86_64_elf(data[:64]):
                raise Refused(f"{name} ({args.binary if name == 'server' else args.account_api}) "
                              "is not a Linux x86-64 executable")
        version, package_sha, package_name = args.version, None, None
    revision = source_revision(args.revision, version)
    release = validate_release_name(args.release or f"{version}-dev-{revision[:7]}")
    grants = validate_grants((repo_root() / GRANTS).read_text())
    digests = {name: hashlib.sha256(data).hexdigest() for name, data in binaries.items()}
    build = {"contour": "dev", "release": release, "version": version, "revision": revision,
             "sha256": digests["server"], "account_api_sha256": digests[ACCOUNT_API],
             "grants_sha256": hashlib.sha256(grants.encode()).hexdigest(),
             "package": package_name, "package_sha256": package_sha,
             "unit_sha256": hashlib.sha256(unit.encode()).hexdigest(), "tool": "ops/vps/dev/lobby_dev.py"}
    return {"release": release, "sha256": digests["server"], "account_api_sha256": digests[ACCOUNT_API],
            "binaries": binaries, "unit": unit, "grants": grants, "build": build,
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
    payload = {key: staged[key] for key in ("release", "sha256", "account_api_sha256", "unit", "grants", "build")}
    payload["expected_ip"] = target["expected_ip"]
    directory = f"{RELEASES}/{staged['release']}"
    plan = {
        "target": target, "release": staged["release"], "sha256": staged["sha256"],
        "account_api_sha256": staged["account_api_sha256"],
        "public_host": staged["public_host"], "build": staged["build"],
        "steps": [
            f"remote preflight (read-only): no production markers, {USER} layout, no dev matches, "
            f"UDP {LOBBY_PORT}/{sorted(MATCH_PORTS)} free, >= {MIN_AVAILABLE_MIB} MiB available, "
            f"{staged['public_host']} resolves to the dev VPS (A only), {DB_CONTAINER} runs on "
            f"127.0.0.1:{DB_PORT} only, {DATABASE_ENV} (if present) names {DB_GAME}@{DB_NAME} on loopback",
            f"remote stage: user {USER}, {OPT}, {STATE}, {directory}/",
            f"scp omoba-server and {ACCOUNT_API} -> {alias}:{directory}/<name>.part (only missing ones)",
            f"remote activate: sha256 checks; career database: roles/database {DB_NAME} when missing, "
            f"{DATABASE_ENV} generated on the host when missing, {ACCOUNT_API} migrate + {GRANTS}; then "
            f"{CURRENT} -> releases/{staged['release']}, unit if changed, restart {UNIT}, verify, roll back "
            "on failure (no-op when already running this release)",
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
    stage = ssh_remote(alias, "stage", {key: staged[key] for key in ("release", "sha256", "account_api_sha256")})
    print(stage)
    for target_path in json.loads(stage).get("upload_to") or []:
        name = posixpath.basename(target_path)[:-len(".part")]
        if target_path != f"{directory}/{name}.part" or name not in RELEASE_BINARIES:
            raise Refused(f"the host asked for an unexpected upload {target_path!r}")
        with tempfile.NamedTemporaryFile(prefix=f"omoba-dev-{name}-", delete=False) as handle:
            handle.write(staged["binaries"][name])
        try:
            subprocess.run(["scp", "-q", "-o", "BatchMode=yes", handle.name, f"{alias}:{target_path}"],
                           check=True)
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
            f"remote rollback: {CURRENT} -> releases/{release} (BUILD.json contour dev, sha256 checked; "
            f"no migration, {DATABASE_ENV} must already be valid), restart {UNIT}, verify"]}, indent=2))
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
    source.add_argument("--binary", help="a Linux x86-64 server binary (needs --version and --account-api)")
    ship.add_argument("--account-api", help=f"the matching Linux x86-64 {ACCOUNT_API} for --binary")
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
