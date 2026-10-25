#!/usr/bin/env python3
"""Encrypted off-host backups, secrets escrow and the monthly restore drill of the RU host (LR-03).

Runs on the RU host as root: `ru_host.py bootstrap` installs it as
/opt/omoba-ru/backup/ru_backup.py with three timers (README.md, "Backups, escrow and
restore drill"), and `ru_host.py ship` runs `backup --reason pre-migration` before every
migration. `keygen` and `escrow --file` also run on the Mac. Settings come only from
/opt/omoba-ru/backup.env (root 0600, from backup.env.example; --env-file elsewhere):

    ru_backup.py backup [--reason nightly|pre-migration|manual] [--release R] [--local-only]
    ru_backup.py drill [--set NAME] [--from remote|local]   # restore into a scratch database, verify
    ru_backup.py check [--json] [--remote] [--alert]        # last backup < 26 h, drill, escrow, settings
    ru_backup.py config [--json]                            # are the settings complete?
    ru_backup.py list [--json]                              # sets on the host and off-host
    ru_backup.py fetch --set NAME|latest --out DIR          # download, decrypt, verify, unpack (restore)
    ru_backup.py escrow --recipient-file FILE|- [--file PATH ...] > secrets.tar.asc
    ru_backup.py keygen --out DIR                           # a new backup key pair (on the Mac)

A backup set is one file, omoba-ru-<UTC>-<reason>.tar.gpg: a tar of database.dump
(`pg_dump -Fc` of the whole database, career and portal), state.tar.gz (the match root
with its archive and every allocation outbox, and the lobby outbox) and manifest.json
(rows and an order-independent digest per table, both schema version lists, checksums),
encrypted with GnuPG to the public key in OMOBA_BACKUP_GPG_RECIPIENT_FILE and signed with
the secret key in OMOBA_BACKUP_GPG_IDENTITY_FILE; fetch and the drill refuse a set that
key did not sign, so a set planted on the storage is never restored. The target
(S3-compatible storage or an SFTP account in Russia) keeps 14 daily and 8 weekly sets;
the host keeps the newest OMOBA_BACKUP_KEEP_LOCAL. A set that could not be uploaded
stays on the host and goes up with the next run. The drill restores the newest
off-host set into <database>_restore_drill, compares tables, row counts, digests and
schema versions with the manifest and drops the scratch database. Failures alert:
always in the journal, and through Telegram when OMOBA_TELEGRAM_BOT_TOKEN and
OMOBA_TELEGRAM_CHAT_ID are set. No secret is printed, logged or put in argv.
"""
from __future__ import annotations

import argparse
import contextlib
import datetime as dt
import fcntl
import hashlib
import hmac
import io
import json
import os
import re
import shutil
import socket
import stat
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ElementTree
from pathlib import Path

TOOL = "ops/vps/ru/ru_backup.py"
MARKET = "ru"
ENV_FILE = "/opt/omoba-ru/backup.env"
PLACEHOLDER = "CHANGE_ME"
SAFE_PATH = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
REASONS = ("nightly", "pre-migration", "manual")
STAMP = "%Y%m%dT%H%M%SZ"
SET_NAME = re.compile(r"omoba-ru-(\d{8}T\d{6}Z)-(nightly|pre-migration|manual)\.tar\.gpg")
ROLLBACK_DUMP = re.compile(r"[a-z_][a-z0-9_]*-before-[0-9A-Za-z.+-]+-(\d{8}T\d{6}Z)\.dump")
RELEASE = re.compile(r"[0-9A-Za-z][0-9A-Za-z.+-]{0,99}")
DB_NAME = re.compile(r"[a-z_][a-z0-9_]{0,40}")
IDENT = r'(?:[a-z_][a-z0-9_$]*|"(?:[^"]|"")+")'
QUALIFIED = re.compile(rf"{IDENT}\.{IDENT}")
VERSION_TABLES = {"career": "public.career_schema_version", "portal": "portal.schema_version"}
SET_MEMBERS = ("database.dump", "state.tar.gz", "manifest.json")
MANIFEST_FORMAT = 1
MAX_BACKUP_AGE_HOURS = 26
MAX_DRILL_AGE_DAYS = 35
STUCK_RUN_HOURS = 6
ALERT_REPEAT_SECONDS = 6 * 3600
KEEP_ROLLBACK_DUMPS = 3
LOCK_WAIT_SECONDS = 2 * 3600
MIN_FREE_BYTES = 1 << 30
DRILL_SPACE_FACTOR = 12          # a compressed custom-format dump restores to many times its size
PG_DATA = "/var/lib/postgresql"
S3_MAX_PUT = 5 * (1 << 30)
DIGEST_MASK = (1 << 128) - 1
SAFE_REMOTE_PATH = re.compile(r"[A-Za-z0-9._/~+-]+")

# Host layout. The OMOBA_BACKUP_DATABASE ... _RUN_AS keys exist for tests and drills on
# another machine; the RU host uses these defaults.
DEFAULTS = {
    "OMOBA_BACKUP_DATABASE": "omoba_ru",
    "OMOBA_BACKUP_DIR": "/var/backups/omoba-ru",
    "OMOBA_BACKUP_STATE_DIRS": "/var/lib/omoba-ru/matches:/var/lib/omoba-ru/lobby-outbox",
    "OMOBA_BACKUP_ESCROW": "/opt/omoba-ru/account-api.env:/opt/omoba-ru/lobby.env:/opt/omoba-ru/portal.env:"
                           "/opt/omoba-ru/backup.env:/opt/omoba-ru/host.json:/opt/omoba-ru/backup-keys",
    "OMOBA_BACKUP_RUN_AS": "postgres",
    "OMOBA_BACKUP_GPG_RECIPIENT_FILE": "/opt/omoba-ru/backup-keys/backup-public.asc",
    "OMOBA_BACKUP_GPG_IDENTITY_FILE": "/opt/omoba-ru/backup-keys/backup-secret.asc",
    "OMOBA_BACKUP_SSH_KEY": "/opt/omoba-ru/backup-keys/ssh_key",
    "OMOBA_BACKUP_SSH_KNOWN_HOSTS": "/opt/omoba-ru/backup-keys/known_hosts",
    "OMOBA_BACKUP_KEEP_DAILY": "14",
    "OMOBA_BACKUP_KEEP_WEEKLY": "8",
    "OMOBA_BACKUP_KEEP_LOCAL": "2",
}
OPTIONAL_KEYS = ("OMOBA_BACKUP_TARGET", "OMOBA_BACKUP_S3_ENDPOINT", "OMOBA_BACKUP_S3_REGION",
                 "OMOBA_BACKUP_S3_ACCESS_KEY_ID", "OMOBA_BACKUP_S3_SECRET_ACCESS_KEY",
                 "OMOBA_TELEGRAM_BOT_TOKEN", "OMOBA_TELEGRAM_CHAT_ID")
KNOWN_KEYS = frozenset(DEFAULTS) | frozenset(OPTIONAL_KEYS)
SECRET_KEYS = ("OMOBA_BACKUP_S3_SECRET_ACCESS_KEY", "OMOBA_BACKUP_S3_ACCESS_KEY_ID", "OMOBA_TELEGRAM_BOT_TOKEN")


class BackupError(Exception):
    """A step failed; the message is safe to print and to send as an alert."""


# ---------------------------------------------------------------- settings

def parse_env_text(text: str) -> tuple[dict, list]:
    """(values, problems) of a systemd-style KEY=VALUE file; unknown keys are problems."""
    values, problems = {}, []
    for number, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        key, sep, value = line.partition("=")
        key, value = key.strip(), value.strip()
        if not sep or not re.fullmatch(r"[A-Z][A-Z0-9_]*", key):
            problems.append(f"line {number} is not KEY=VALUE")
            continue
        if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
            value = value[1:-1]
        if key in values:
            problems.append(f"{key} is set twice")
        if key not in KNOWN_KEYS:
            problems.append(f"unknown setting {key}")
        values[key] = value
    return values, problems


class Config:
    def __init__(self, path: str = ENV_FILE, values: dict | None = None):
        self.path = path
        self.exists = True
        self.problems = []
        if values is None:
            try:
                text = Path(path).read_text()
            except FileNotFoundError:
                text, self.exists = "", False
            except PermissionError:
                raise BackupError(f"cannot read {path}: run as root") from None
            values, self.problems = parse_env_text(text)
        self.values = values

    def get(self, key: str) -> str:
        value = self.values.get(key, DEFAULTS.get(key, ""))
        return "" if PLACEHOLDER in value else value

    def number(self, key: str) -> int:
        try:
            return int(self.get(key))
        except ValueError:
            return 0

    def paths(self, key: str) -> list:
        return [part for part in self.get(key).split(":") if part]

    @property
    def database(self) -> str:
        return self.get("OMOBA_BACKUP_DATABASE")

    @property
    def drill_database(self) -> str:
        return self.database + "_restore_drill"

    @property
    def backup_dir(self) -> str:
        return self.get("OMOBA_BACKUP_DIR")

    @property
    def sets_dir(self) -> str:
        return os.path.join(self.backup_dir, "sets")

    @property
    def work_dir(self) -> str:
        return os.path.join(self.backup_dir, "work")

    @property
    def recipient(self) -> str:
        return self.get("OMOBA_BACKUP_GPG_RECIPIENT_FILE")

    @property
    def identity(self) -> str:
        return self.get("OMOBA_BACKUP_GPG_IDENTITY_FILE")

    def secrets(self) -> list:
        return [self.values[key] for key in SECRET_KEYS if self.values.get(key) and PLACEHOLDER not in self.values[key]]


def redact(text: str, hidden=()) -> str:
    for value in hidden:
        if value and len(value) >= 4:
            text = text.replace(value, "<redacted>")
    text = re.sub(r"bot\d+:[A-Za-z0-9_-]{20,}", "bot<redacted>", text)
    return re.sub(r"://([^:/@\s]+):[^@\s]*@", r"://\1:<redacted>@", text)


def tail(data, limit: int = 400) -> str:
    if isinstance(data, bytes):
        data = data.decode(errors="replace")
    data = (data or "").strip()
    return data[-limit:] if data else "no output"


def parse_target(url: str) -> dict:
    """The off-host target from OMOBA_BACKUP_TARGET; raises BackupError when it is invalid."""
    parts = urllib.parse.urlsplit(url or "")
    if parts.password is not None or parts.query or parts.fragment:
        raise BackupError("OMOBA_BACKUP_TARGET must not carry a password, query or fragment; "
                          "credentials go into their own settings")
    if parts.scheme == "s3":
        bucket, prefix = parts.netloc, parts.path.strip("/")
        if not re.fullmatch(r"[a-z0-9][a-z0-9.-]{1,61}[a-z0-9]", bucket or ""):
            raise BackupError(f"OMOBA_BACKUP_TARGET: {bucket!r} is not an S3 bucket name")
        if prefix and (not SAFE_REMOTE_PATH.fullmatch(prefix) or ".." in prefix.split("/")):
            raise BackupError("OMOBA_BACKUP_TARGET: the S3 prefix may use only letters, digits and ._/~+-")
        return {"scheme": "s3", "bucket": bucket, "prefix": prefix}
    if parts.scheme == "sftp":
        user, host = parts.username, parts.hostname
        try:
            port = parts.port or 22
        except ValueError:
            raise BackupError("OMOBA_BACKUP_TARGET: the SFTP port is not a number") from None
        path = parts.path
        if not user or not re.fullmatch(r"[a-z_][a-z0-9_.-]{0,31}", user):
            raise BackupError("OMOBA_BACKUP_TARGET: sftp:// needs a login, sftp://<user>@<host>/<folder>")
        if not host or not re.fullmatch(r"[a-z0-9][a-z0-9.-]{0,252}", host):
            raise BackupError("OMOBA_BACKUP_TARGET: the SFTP host must be a DNS name or IPv4 address")
        if not path.startswith("/") or path == "/" or not SAFE_REMOTE_PATH.fullmatch(path) or ".." in path.split("/"):
            raise BackupError("OMOBA_BACKUP_TARGET: the SFTP folder must be an absolute path of letters, "
                              "digits and ._/~+-")
        return {"scheme": "sftp", "user": user, "host": host, "port": port, "path": path.rstrip("/")}
    if parts.scheme == "file":
        if parts.netloc or not parts.path.startswith("/") or parts.path == "/":
            raise BackupError("OMOBA_BACKUP_TARGET: file:// needs an absolute folder, file:///mnt/backup")
        return {"scheme": "file", "path": parts.path.rstrip("/")}
    raise BackupError("OMOBA_BACKUP_TARGET must be s3://<bucket>/<prefix>, sftp://<user>@<host>[:<port>]/<folder> "
                      "or file:///<folder>")


def secret_file_problem(path: str, label: str) -> str | None:
    try:
        info = os.stat(path)
    except FileNotFoundError:
        return f"{label} {path} does not exist"
    except PermissionError:
        return f"{label} {path} is not readable: run as root"
    if not stat.S_ISREG(info.st_mode):
        return f"{label} {path} is not a regular file"
    if info.st_mode & 0o077:
        return f"{label} {path} has mode {oct(info.st_mode & 0o777)}; it must be 0600"
    if os.geteuid() == 0 and info.st_uid != 0:
        return f"{label} {path} must be owned by root"
    return None


def config_problems(cfg: Config) -> list:
    """[(code, text)] of everything a backup and the restore drill need."""
    problems = [("config", f"{cfg.path}: {problem}") for problem in cfg.problems]
    if not cfg.exists:
        return [("config-missing", f"{cfg.path} does not exist (ru_host.py bootstrap writes it from "
                                   "backup.env.example)")]
    if os.geteuid() == 0:
        info = os.stat(cfg.path)
        if info.st_mode & 0o077 or info.st_uid != 0:
            problems.append(("config", f"{cfg.path} must be root-owned with mode 0600"))
    if not DB_NAME.fullmatch(cfg.database or ""):
        problems.append(("config", "OMOBA_BACKUP_DATABASE is not a plain database name"))
    for key in ("OMOBA_BACKUP_DIR", "OMOBA_BACKUP_GPG_RECIPIENT_FILE", "OMOBA_BACKUP_GPG_IDENTITY_FILE"):
        if not cfg.get(key).startswith("/"):
            problems.append(("config", f"{key} must be an absolute path"))
    for key in ("OMOBA_BACKUP_STATE_DIRS", "OMOBA_BACKUP_ESCROW"):
        if any(not path.startswith("/") for path in cfg.paths(key)):
            problems.append(("config", f"{key} must list absolute paths separated by ':'"))
    for key, low in (("OMOBA_BACKUP_KEEP_DAILY", 1), ("OMOBA_BACKUP_KEEP_WEEKLY", 0), ("OMOBA_BACKUP_KEEP_LOCAL", 1)):
        if not re.fullmatch(r"\d{1,3}", cfg.get(key)) or cfg.number(key) < low:
            problems.append(("config", f"{key} must be a whole number >= {low}"))
    target = cfg.get("OMOBA_BACKUP_TARGET")
    if not target:
        problems.append(("target-unset", "OMOBA_BACKUP_TARGET is not set: name the off-host storage in Russia "
                                         "(s3://<bucket>/<prefix> or sftp://<user>@<host>/<folder>)"))
    else:
        try:
            parsed = parse_target(target)
        except BackupError as error:
            problems.append(("target-invalid", str(error)))
            parsed = {}
        if parsed.get("scheme") == "s3":
            endpoint = cfg.get("OMOBA_BACKUP_S3_ENDPOINT")
            parts = urllib.parse.urlsplit(endpoint)
            if parts.scheme != "https" or not parts.hostname or parts.path not in ("", "/") or parts.query:
                problems.append(("target-invalid", "OMOBA_BACKUP_S3_ENDPOINT must be the storage's https:// origin "
                                                   "(e.g. https://storage.yandexcloud.net)"))
            for key in ("OMOBA_BACKUP_S3_REGION", "OMOBA_BACKUP_S3_ACCESS_KEY_ID", "OMOBA_BACKUP_S3_SECRET_ACCESS_KEY"):
                if not cfg.get(key):
                    problems.append(("target-invalid", f"{key} is not set (s3:// target)"))
        elif parsed.get("scheme") == "sftp":
            for key, label in (("OMOBA_BACKUP_SSH_KEY", "the SFTP login key"),):
                problem = secret_file_problem(cfg.get(key), label)
                if problem:
                    problems.append(("target-invalid", f"{problem} ({key})"))
            known = cfg.get("OMOBA_BACKUP_SSH_KNOWN_HOSTS")
            if not os.path.isfile(known) or not Path(known).read_text().strip():
                problems.append(("target-invalid", f"the SFTP host key file {known} is missing or empty "
                                                   "(OMOBA_BACKUP_SSH_KNOWN_HOSTS; ssh-keyscan, then compare the "
                                                   "fingerprint with the provider)"))
    try:
        recipient = key_info(cfg.recipient)
    except BackupError as error:
        problems.append(("recipient", f"encryption key: {error} (OMOBA_BACKUP_GPG_RECIPIENT_FILE)"))
        recipient = None
    if recipient is not None:
        if recipient["secret"]:
            problems.append(("recipient", f"{cfg.recipient} holds a secret key; give the host only the public key"))
        if not recipient["encrypt"]:
            problems.append(("recipient", f"{cfg.recipient} has no usable encryption key"))
    problem = secret_file_problem(cfg.identity, "the backup secret key")
    if problem:
        problems.append(("identity", f"{problem} (OMOBA_BACKUP_GPG_IDENTITY_FILE: it signs every set, and the "
                                     "monthly drill decrypts with it)"))
    elif recipient is not None:
        try:
            identity = key_info(cfg.identity)
        except BackupError as error:
            problems.append(("identity", f"backup secret key: {error}"))
        else:
            if not identity["secret"] or not set(recipient["encrypt"]) & set(identity["encrypt"]):
                problems.append(("identity", f"{cfg.identity} is not the secret key of {cfg.recipient}: "
                                             "the drill could not decrypt the sets"))
            elif not identity["sign"]:
                problems.append(("identity", f"{cfg.identity} has no usable signing key (make the backup key "
                                             "with `ru_backup.py keygen`)"))
    if cfg.values.get("OMOBA_TELEGRAM_BOT_TOKEN") and not cfg.get("OMOBA_TELEGRAM_CHAT_ID"):
        problems.append(("config", "OMOBA_TELEGRAM_BOT_TOKEN is set without OMOBA_TELEGRAM_CHAT_ID"))
    return problems


# ---------------------------------------------------------------- processes

def base_env() -> dict:
    env = {"PATH": os.environ.get("PATH") or SAFE_PATH, "LANG": "C", "PGCLIENTENCODING": "UTF8"}
    for key in ("PGHOST", "PGPORT", "PGUSER", "PGPASSWORD", "TMPDIR"):
        if os.environ.get(key):
            env[key] = os.environ[key]
    return env


def exe(name: str) -> str:
    found = shutil.which(name, path=base_env()["PATH"])
    if not found:
        raise BackupError(f"{name} is not installed")
    return found


def db_user(cfg: Config) -> dict:
    """subprocess arguments that run a database client as OMOBA_BACKUP_RUN_AS (peer login)."""
    user = cfg.get("OMOBA_BACKUP_RUN_AS")
    if not user:
        return {}
    import pwd
    try:
        entry = pwd.getpwnam(user)
    except KeyError:
        raise BackupError(f"the OS user {user} (OMOBA_BACKUP_RUN_AS) does not exist") from None
    if os.geteuid() == entry.pw_uid:
        return {}
    if os.geteuid() != 0:
        raise BackupError(f"run as root: the database is reached as the {user} OS user")
    return {"user": entry.pw_uid, "group": entry.pw_gid, "extra_groups": []}


def psql(cfg: Config, database: str, sql: str, timeout: int = 900) -> str:
    if database != "postgres" and not DB_NAME.fullmatch(database):
        raise BackupError(f"internal: psql on {database!r}")
    result = subprocess.run([exe("psql"), "-X", "-q", "-At", "-v", "ON_ERROR_STOP=1", "-w", "-d", database],
                            input=sql.encode(), capture_output=True, env=base_env(), cwd="/", timeout=timeout,
                            **db_user(cfg))
    if result.returncode:
        raise BackupError(f"psql on {database} failed: {tail(result.stderr)}")
    return result.stdout.decode()


def pg_dump(cfg: Config, database: str, path: str, timeout: int = 3 * 3600) -> None:
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "wb") as out:
        result = subprocess.run([exe("pg_dump"), "-Fc", "-w", "-d", database], stdout=out, stderr=subprocess.PIPE,
                                env=base_env(), cwd="/", timeout=timeout, **db_user(cfg))
    if result.returncode:
        raise BackupError(f"pg_dump {database} failed: {tail(result.stderr)}")
    with open(path, "rb") as handle:
        if handle.read(5) != b"PGDMP":
            raise BackupError(f"pg_dump {database} wrote no custom-format archive")


def sql_literal(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


# ---------------------------------------------------------------- what a dump holds

def copy_table(line: str) -> str | None:
    """`public.career_profiles` from `COPY public.career_profiles (a, b) FROM stdin;`."""
    if not line.startswith("COPY ") or not line.endswith(" FROM stdin;"):
        return None
    rest, index, parts = line[5:], 0, []
    while index < len(rest):
        if rest[index] == '"':
            end = index + 1
            while True:
                end = rest.index('"', end)
                if rest[end + 1:end + 2] == '"':
                    end += 2
                    continue
                break
            parts.append(rest[index:end + 1])
            index = end + 1
        else:
            match = re.match(r'[^ ."(]+', rest[index:])
            if not match:
                return None
            parts.append(match.group())
            index += match.end()
        if rest[index:index + 1] != ".":
            break
        index += 1
    name = ".".join(parts)
    return name if QUALIFIED.fullmatch(name) else None


def scan_dump(path: str, timeout: int = 3 * 3600) -> dict:
    """Rows and an order-independent digest per table, and the schema versions, read from
    the dump's own COPY data (`pg_restore --data-only -f -`, no database involved)."""
    versions_of = {table: key for key, table in VERSION_TABLES.items()}
    tables, versions = {}, {}
    with tempfile.TemporaryFile() as errors:
        process = subprocess.Popen([exe("pg_restore"), "--data-only", "-f", "-", path], stdout=subprocess.PIPE,
                                   stderr=errors, env=base_env(), cwd="/")
        current, rows, digest, found = None, 0, 0, None
        try:
            for line in process.stdout:
                if current is None:
                    if line.startswith(b"COPY ") and line.endswith(b" FROM stdin;\n"):
                        current = copy_table(line[:-1].decode("utf-8", "surrogateescape"))
                        if current is None:
                            raise BackupError(f"unexpected COPY header in the dump: {tail(line, 120)}")
                        rows, digest = 0, 0
                        found = [] if current in versions_of else None
                    continue
                if line == b"\\.\n":
                    tables[current] = {"rows": rows, "digest": f"{digest:032x}"}
                    if found is not None:
                        versions[versions_of[current]] = sorted(found)
                    current = None
                    continue
                rows += 1
                digest = (digest + int.from_bytes(hashlib.sha256(line).digest()[:16], "big")) & DIGEST_MASK
                if found is not None:
                    found.append(int(line.split(b"\t", 1)[0]))
            code = process.wait(timeout=timeout)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            process.stdout.close()
        errors.seek(0)
        if code or current is not None:
            raise BackupError(f"pg_restore could not read {os.path.basename(path)}: {tail(errors.read())}")
    return {"tables": tables, "schema_versions": versions}


def live_versions(cfg: Config, database: str) -> dict:
    present = psql(cfg, database, "SELECT " + ", ".join(
        f"to_regclass({sql_literal(table)}) IS NOT NULL" for table in VERSION_TABLES.values()) + ";")
    flags = present.strip().split("|")
    tables = {key: table for (key, table), flag in zip(VERSION_TABLES.items(), flags) if flag == "t"}
    if not tables:
        return {}
    sql = "SELECT json_build_object(" + ", ".join(
        f"{sql_literal(key)}, (SELECT coalesce(json_agg(version ORDER BY version), '[]') FROM {table})"
        for key, table in tables.items()) + ");"
    return json.loads(psql(cfg, database, sql).strip())


def archive_state(roots: list, out: str) -> dict:
    """state.tar.gz of the match root (archive and outboxes) and the lobby outbox, as
    var/lib/omoba-ru/...; a file that disappears while the tar is written is counted."""
    files = size = vanished = skipped = 0
    missing = []
    descriptor = os.open(out, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "wb") as raw, tarfile.open(fileobj=raw, mode="w:gz", compresslevel=6) as archive:
        for root in roots:
            if not os.path.isdir(root):
                missing.append(root)
                continue
            for directory, dirs, names in os.walk(root):
                dirs.sort()
                try:
                    archive.add(directory, arcname=directory.lstrip("/"), recursive=False)
                except FileNotFoundError:   # a worker folder moved to archive/ meanwhile
                    vanished += 1
                    dirs[:] = []
                    continue
                for name in sorted(names):
                    path = os.path.join(directory, name)
                    try:
                        info = os.lstat(path)
                        if stat.S_ISREG(info.st_mode):
                            with open(path, "rb") as handle:
                                member = archive.gettarinfo(arcname=path.lstrip("/"), fileobj=handle)
                                archive.addfile(member, handle)
                            files += 1
                            size += member.size
                        elif stat.S_ISLNK(info.st_mode):
                            archive.add(path, arcname=path.lstrip("/"), recursive=False)
                        else:
                            skipped += 1
                    except FileNotFoundError:
                        vanished += 1
    return {"files": files, "file_bytes": size, "roots": list(roots), "missing": missing, "vanished": vanished,
            "skipped": skipped}


def state_counts(path: str) -> dict:
    files = size = 0
    with tarfile.open(path, "r:gz") as archive:
        for member in archive:
            if member.isfile():
                files += 1
                size += member.size
    return {"files": files, "file_bytes": size}


def sha256_file(path: str) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


# ---------------------------------------------------------------- GnuPG

@contextlib.contextmanager
def gpg_home(agent: bool = False):
    """A throwaway GnuPG home (short path: the agent socket lives in it). `agent`: secret-key
    work (import, sign, decrypt, keygen) starts gpg-agent, which is stopped afterwards."""
    home = tempfile.mkdtemp(prefix="omoba-gpg-")
    try:
        yield home
    finally:
        if agent or any(name.startswith("S.") for name in os.listdir(home)):
            with contextlib.suppress(Exception):
                subprocess.run([exe("gpgconf"), "--homedir", home, "--kill", "gpg-agent"], capture_output=True,
                               env=base_env(), timeout=30)
        shutil.rmtree(home, ignore_errors=True)


def gpg(home: str, *args) -> list:
    return [exe("gpg"), "--homedir", home, "--batch", "--no-tty", "--quiet", "--no-greeting", *args]


def key_info(path_or_text: str, text: bool = False) -> dict:
    """{'fingerprint', 'encrypt': [fingerprints], 'sign': [fingerprints], 'secret'} of an armored key file."""
    if not text and not os.path.isfile(path_or_text):
        raise BackupError(f"{path_or_text} does not exist")
    with gpg_home() as home:
        source = path_or_text
        if text:
            source = os.path.join(home, "key.asc")
            Path(source).write_text(path_or_text)
        # --no-autostart: listing a secret key file needs no gpg-agent.
        result = subprocess.run(gpg(home, "--no-autostart", "--with-colons", "--show-keys", source),
                                capture_output=True, env=base_env(), timeout=60)
    if result.returncode:
        raise BackupError(f"not an OpenPGP key ({tail(result.stderr, 160)})")
    primary, encrypt, sign, secret, pending = None, [], [], False, None
    for line in result.stdout.decode(errors="replace").splitlines():
        fields = line.split(":")
        if fields[0] in ("pub", "sec", "sub", "ssb"):
            secret = secret or fields[0] in ("sec", "ssb")
            # Lowercase letters are this (sub)key's own capabilities; uppercase ones describe the whole key.
            valid = fields[1] not in ("r", "e", "i", "d")
            caps = fields[11] if len(fields) > 11 else ""
            pending = (fields[0], valid and "e" in caps, valid and "s" in caps)
        elif fields[0] == "fpr" and pending:
            if pending[0] in ("pub", "sec") and primary is None:
                primary = fields[9]
            if pending[1]:
                encrypt.append(fields[9])
            if pending[2]:
                sign.append(fields[9])
            pending = None
    if primary is None:
        raise BackupError("no OpenPGP key found")
    return {"fingerprint": primary, "encrypt": encrypt, "sign": sign, "secret": secret}


def import_identity(cfg: Config, home: str) -> str:
    """Import the backup secret key into a throwaway GnuPG home; its primary fingerprint."""
    if not os.path.isfile(cfg.identity):
        raise BackupError(f"the backup secret key {cfg.identity} is missing (OMOBA_BACKUP_GPG_IDENTITY_FILE)")
    imported = subprocess.run(gpg(home, "--import", cfg.identity), capture_output=True, env=base_env(), timeout=60)
    if imported.returncode:
        raise BackupError(f"gpg --import of the backup secret key failed: {tail(imported.stderr)}")
    return key_info(cfg.identity)["fingerprint"]


def encrypt_to(recipient: str, out_path: str, home: str, armor: bool = False, signer: str | None = None):
    """A gpg process encrypting its stdin to `recipient` into out_path (and signing it with
    `signer`, a secret key already imported into `home`)."""
    args = ["--trust-model", "always", "--compress-algo", "none", "--recipient-file", recipient, "--yes",
            "--output", out_path] + (["--armor"] if armor else []) + \
        (["--local-user", signer, "--sign"] if signer else []) + ["--encrypt"]
    errors = tempfile.TemporaryFile()
    process = subprocess.Popen(gpg(home, *args), stdin=subprocess.PIPE, stderr=errors, env=base_env(), cwd="/")
    return process, errors


def signed_by(status: str) -> set:
    """Primary fingerprints of the good signatures in a gpg --status-file."""
    found = set()
    for line in status.splitlines():
        fields = line.split()
        if fields[:2] == ["[GNUPG:]", "VALIDSIG"] and len(fields) > 2:
            found.add(fields[11] if len(fields) > 11 else fields[2])
    return found


def finish(process, errors, what: str) -> None:
    code = process.wait()
    errors.seek(0)
    message = errors.read()
    errors.close()
    if code:
        raise BackupError(f"{what} failed: {tail(message)}")


# ---------------------------------------------------------------- off-host targets

class FileTarget:
    """A folder; off-host only when it is the mount of another machine."""

    def __init__(self, path: str):
        self.path = path

    def describe(self) -> str:
        return f"file://{self.path}"

    def list(self) -> dict:
        if not os.path.isdir(self.path):
            return {}
        return {name: os.path.getsize(os.path.join(self.path, name)) for name in os.listdir(self.path)
                if SET_NAME.fullmatch(name)}

    def put(self, local: str, name: str) -> None:
        os.makedirs(self.path, mode=0o700, exist_ok=True)
        part = os.path.join(self.path, name + ".part")
        shutil.copyfile(local, part)
        os.chmod(part, 0o600)
        os.replace(part, os.path.join(self.path, name))

    def get(self, name: str, local: str) -> None:
        shutil.copyfile(os.path.join(self.path, name), local)

    def delete(self, names: list) -> None:
        for name in names:
            os.unlink(os.path.join(self.path, name))


class SftpTarget:
    """An SFTP account (OpenSSH `sftp -b`), host key pinned in its own known_hosts file."""

    LISTING = re.compile(r"^[-dlcbps][-rwxsStT]{9}\S*\s+\d+\s+\S+\s+\S+\s+(\d+)\s+\S+\s+\S+\s+\S+\s+(.+)$")

    def __init__(self, user: str, host: str, port: int, path: str, key: str, known_hosts: str):
        self.user, self.host, self.port, self.path = user, host, port, path
        self.key, self.known_hosts = key, known_hosts

    def describe(self) -> str:
        return f"sftp://{self.user}@{self.host}:{self.port}{self.path}"

    def _batch(self, commands: list, timeout: int = 3 * 3600) -> str:
        for command in commands:
            if "\n" in command or '"' in command:
                raise BackupError("internal: unsafe SFTP batch command")
        args = [exe("sftp"), "-F", "none", "-b", "-", "-P", str(self.port),
                "-o", "BatchMode=yes", "-o", "IdentitiesOnly=yes", "-o", f"IdentityFile={self.key}",
                "-o", "StrictHostKeyChecking=yes", "-o", f"UserKnownHostsFile={self.known_hosts}",
                "-o", "GlobalKnownHostsFile=/dev/null", "-o", "ConnectTimeout=20",
                "-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=4", f"{self.user}@{self.host}"]
        result = subprocess.run(args, input="\n".join(commands).encode() + b"\n", capture_output=True,
                                env=base_env(), cwd="/", timeout=timeout)
        if result.returncode:
            raise BackupError(f"sftp {self.describe()} failed: {tail(result.stderr)}")
        return result.stdout.decode(errors="replace")

    @staticmethod
    def _safe(path: str) -> str:
        if not SAFE_REMOTE_PATH.fullmatch(path):
            raise BackupError(f"internal: unsafe path for SFTP {path!r}")
        return path

    def list(self) -> dict:
        found = {}
        for line in self._batch([f"-ls -ln {self._safe(self.path)}"], timeout=300).splitlines():
            match = self.LISTING.match(line.strip())
            if match:
                name = match.group(2).rstrip("/").rsplit("/", 1)[-1]
                if SET_NAME.fullmatch(name):
                    found[name] = int(match.group(1))
        return found

    def put(self, local: str, name: str) -> None:
        remote = self._safe(f"{self.path}/{name}")
        # A copy of the wrong size under the final name goes first: SFTP v3 rename never replaces.
        self._batch([f"-mkdir {self._safe(self.path)}", f"-rm {remote}.part",
                     f"put {self._safe(local)} {remote}.part", f"-rm {remote}", f"rename {remote}.part {remote}"])

    def get(self, name: str, local: str) -> None:
        self._batch([f"get {self._safe(self.path + '/' + name)} {self._safe(local)}"])

    def delete(self, names: list) -> None:
        if names:
            self._batch([f"rm {self._safe(self.path + '/' + name)}" for name in names], timeout=600)


def sigv4(method: str, url: str, headers: dict, payload_hash: str, region: str, access: str, secret: str,
          amz_date: str, service: str = "s3") -> str:
    """AWS Signature Version 4 `Authorization` value; `headers` (with host) are all signed."""
    parts = urllib.parse.urlsplit(url)
    quote = lambda value: urllib.parse.quote(value, safe="-_.~")  # noqa: E731
    query = urllib.parse.parse_qsl(parts.query, keep_blank_values=True)
    canonical_query = "&".join(f"{quote(key)}={quote(value)}" for key, value in sorted(query))
    signed = {key.lower(): " ".join(str(value).strip().split()) for key, value in headers.items()}
    names = sorted(signed)
    canonical = "\n".join([method, parts.path or "/", canonical_query,
                           "".join(f"{name}:{signed[name]}\n" for name in names), ";".join(names), payload_hash])
    scope = f"{amz_date[:8]}/{region}/{service}/aws4_request"
    to_sign = "\n".join(["AWS4-HMAC-SHA256", amz_date, scope, hashlib.sha256(canonical.encode()).hexdigest()])
    key = ("AWS4" + secret).encode()
    for part in (amz_date[:8], region, service, "aws4_request"):
        key = hmac.new(key, part.encode(), hashlib.sha256).digest()
    signature = hmac.new(key, to_sign.encode(), hashlib.sha256).hexdigest()
    return f"AWS4-HMAC-SHA256 Credential={access}/{scope}, SignedHeaders={';'.join(names)}, Signature={signature}"


EMPTY_SHA256 = hashlib.sha256(b"").hexdigest()


class S3Target:
    """S3-compatible object storage, path-style requests signed with SigV4 (Yandex Object
    Storage, Selectel, VK Cloud ...). Single PUTs, so a set may not exceed 5 GiB."""

    def __init__(self, endpoint: str, region: str, bucket: str, prefix: str, access: str, secret: str,
                 opener=None, clock=time.time):
        self.endpoint, self.region, self.bucket, self.prefix = endpoint.rstrip("/"), region, bucket, prefix
        self.access, self.secret = access, secret
        self.opener = opener or urllib.request.urlopen
        self.clock = clock

    def describe(self) -> str:
        return f"s3://{self.bucket}/{self.prefix} at {self.endpoint}"

    def _url(self, key: str = "", query: dict | None = None) -> str:
        path = "/" + self.bucket + ("/" + urllib.parse.quote(key, safe="/-_.~") if key else "")
        encoded = "&".join(f"{urllib.parse.quote(k, safe='-_.~')}={urllib.parse.quote(v, safe='-_.~')}"
                           for k, v in sorted((query or {}).items()))
        return self.endpoint + path + ("?" + encoded if encoded else "")

    def _key(self, name: str) -> str:
        return f"{self.prefix}/{name}" if self.prefix else name

    def _request(self, method: str, url: str, body=None, length: int = 0, payload_hash: str = EMPTY_SHA256,
                 timeout: int = 600):
        amz_date = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime(self.clock()))
        headers = {"host": urllib.parse.urlsplit(url).netloc, "x-amz-content-sha256": payload_hash,
                   "x-amz-date": amz_date}
        authorization = sigv4(method, url, headers, payload_hash, self.region, self.access, self.secret, amz_date)
        request = urllib.request.Request(url, data=body, method=method)
        for key, value in headers.items():
            if key != "host":
                request.add_header(key, value)
        request.add_header("Authorization", authorization)
        if body is not None:
            request.add_header("Content-Length", str(length))
            request.add_header("Content-Type", "application/octet-stream")
        try:
            return self.opener(request, timeout=timeout)
        except urllib.error.HTTPError as error:
            detail = error.read()[:2000].decode(errors="replace")
            code = re.search(r"<Code>([^<]{1,80})</Code>", detail)
            raise BackupError(f"S3 {method} {urllib.parse.urlsplit(url).path} answered {error.code}"
                              + (f" {code.group(1)}" if code else "")) from None
        except (urllib.error.URLError, OSError) as error:
            raise BackupError(f"S3 {self.endpoint} did not answer: {getattr(error, 'reason', error)}") from None

    def list(self) -> dict:
        found, token = {}, None
        prefix = self.prefix + "/" if self.prefix else ""
        for _ in range(1000):
            query = {"list-type": "2", "prefix": prefix, "max-keys": "1000"}
            if token:
                query["continuation-token"] = token
            with self._request("GET", self._url(query=query), timeout=120) as response:
                root = ElementTree.fromstring(response.read())
            space = root.tag[:root.tag.index("}") + 1] if root.tag.startswith("{") else ""
            for item in root.findall(f"{space}Contents"):
                key = item.findtext(f"{space}Key") or ""
                name = key[len(prefix):]
                if key.startswith(prefix) and SET_NAME.fullmatch(name):
                    found[name] = int(item.findtext(f"{space}Size") or 0)
            token = root.findtext(f"{space}NextContinuationToken")
            if (root.findtext(f"{space}IsTruncated") or "").lower() != "true" or not token:
                return found
        raise BackupError("S3 listing did not end after 1000 pages")

    def put(self, local: str, name: str) -> None:
        size = os.path.getsize(local)
        if size > S3_MAX_PUT:
            raise BackupError(f"{name} is {size} bytes; single S3 uploads stop at 5 GiB")
        payload_hash = sha256_file(local)
        url = self._url(self._key(name))
        with open(local, "rb") as body:
            with self._request("PUT", url, body=body, length=size, payload_hash=payload_hash, timeout=3 * 3600):
                pass
        with self._request("HEAD", url, timeout=120) as response:
            stored = int(response.headers.get("Content-Length") or -1)
        if stored != size:
            raise BackupError(f"S3 holds {stored} bytes of {name}, not {size}")

    def get(self, name: str, local: str) -> None:
        with self._request("GET", self._url(self._key(name)), timeout=3 * 3600) as response, open(local, "wb") as out:
            shutil.copyfileobj(response, out, 1 << 20)

    def delete(self, names: list) -> None:
        for name in names:
            with self._request("DELETE", self._url(self._key(name)), timeout=120):
                pass


def open_target(cfg: Config, opener=None):
    target = parse_target(cfg.get("OMOBA_BACKUP_TARGET"))
    if target["scheme"] == "s3":
        return S3Target(cfg.get("OMOBA_BACKUP_S3_ENDPOINT"), cfg.get("OMOBA_BACKUP_S3_REGION"), target["bucket"],
                        target["prefix"], cfg.get("OMOBA_BACKUP_S3_ACCESS_KEY_ID"),
                        cfg.get("OMOBA_BACKUP_S3_SECRET_ACCESS_KEY"), opener=opener)
    if target["scheme"] == "sftp":
        return SftpTarget(target["user"], target["host"], target["port"], target["path"],
                          cfg.get("OMOBA_BACKUP_SSH_KEY"), cfg.get("OMOBA_BACKUP_SSH_KNOWN_HOSTS"))
    return FileTarget(target["path"])


# ---------------------------------------------------------------- retention

def set_time(name: str) -> dt.datetime:
    match = SET_NAME.fullmatch(name)
    return dt.datetime.strptime(match.group(1), STAMP).replace(tzinfo=dt.timezone.utc)


def retained(names, daily: int = 14, weekly: int = 8) -> set:
    """The newest set of each of the `daily` most recent days that have a set, and of each
    of the `weekly` most recent ISO weeks; the newest set always. Counting days that have
    sets (not calendar days) means a stretch of failed nights never deletes the last good sets."""
    ordered = sorted((name for name in names if SET_NAME.fullmatch(name)), key=lambda n: (set_time(n), n),
                     reverse=True)
    keep, days, weeks = set(ordered[:1]), [], []
    for name in ordered:
        moment = set_time(name)
        day, week = moment.date(), tuple(moment.isocalendar()[:2])
        if day not in days:
            days.append(day)
            if len(days) <= daily:
                keep.add(name)
        if week not in weeks:
            weeks.append(week)
            if len(weeks) <= weekly:
                keep.add(name)
    return keep


# ---------------------------------------------------------------- state and alerts

def iso(moment: float) -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(moment))


def parse_iso(text) -> float | None:
    try:
        return dt.datetime.strptime(text, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=dt.timezone.utc).timestamp()
    except (TypeError, ValueError):
        return None


class Context:
    def __init__(self, cfg: Config, now=time.time, opener=None, notify_opener=None, stderr=None):
        self.cfg = cfg
        self.now = now
        self.opener = opener
        self.notify_opener = notify_opener or urllib.request.urlopen
        self.stderr = stderr or sys.stderr
        self.state_path = os.path.join(cfg.backup_dir, "state.json")
        try:
            self.state = json.loads(Path(self.state_path).read_text())
        except (FileNotFoundError, ValueError):
            self.state = {}
        except PermissionError:
            raise BackupError(f"cannot read {self.state_path}: run as root") from None

    def save(self) -> None:
        os.makedirs(self.cfg.backup_dir, mode=0o700, exist_ok=True)
        temporary = self.state_path + ".next"
        descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(descriptor, "w") as handle:
            json.dump(self.state, handle, indent=2, sort_keys=True)
        os.replace(temporary, self.state_path)

    def target(self):
        return open_target(self.cfg, self.opener)

    def log(self, text: str, priority: int = 6) -> None:
        # systemd reads a leading <N> as the journal priority (3 = error, 4 = warning, 6 = info).
        print(f"<{priority}>{redact(text, self.cfg.secrets())}", file=self.stderr, flush=True)

    def alert(self, code: str, text: str) -> bool:
        return self.alert_many([(code, text)])

    def alert_many(self, problems: list) -> bool:
        """Journal always; one Telegram message listing every problem when any of them has not
        been sent in the last 6 h."""
        for _, text in problems:
            self.log(text, 3)
        alerts = self.state.setdefault("alerts", {})
        now = self.now()
        due = [code for code, _ in problems
               if not (isinstance(alerts.get(code), (int, float)) and now - alerts[code] < ALERT_REPEAT_SECONDS)]
        if not due:
            return False
        for code in due:
            alerts[code] = now
        self.save()
        lines = "\n".join(f"- {text}" for _, text in problems)
        return self.telegram(f"OMOBA RU backups on {socket.gethostname()}:\n{lines}")

    def resolve(self, codes) -> None:
        """Forget alerts whose problem is gone; tell Telegram once when everything passes again."""
        alerts = self.state.get("alerts") or {}
        gone = [code for code in alerts if code not in codes]
        for code in gone:
            del alerts[code]
        if gone:
            self.save()
            if not alerts:
                self.telegram(f"OMOBA RU backups on {socket.gethostname()}: all checks pass again")

    def telegram(self, text: str) -> bool:
        token, chat = self.cfg.get("OMOBA_TELEGRAM_BOT_TOKEN"), self.cfg.get("OMOBA_TELEGRAM_CHAT_ID")
        if not token or not chat:
            return False
        data = urllib.parse.urlencode({"chat_id": chat, "text": redact(text, self.cfg.secrets())[:3500],
                                       "disable_web_page_preview": "true"}).encode()
        request = urllib.request.Request(f"https://api.telegram.org/bot{token}/sendMessage", data=data, method="POST")
        try:
            with self.notify_opener(request, timeout=20):
                return True
        except (urllib.error.URLError, OSError, ValueError) as error:
            self.log(f"the Telegram alert was not delivered: {redact(str(error), [token])}", 4)
            return False


def ensure_dirs(cfg: Config) -> None:
    for path in (cfg.backup_dir, cfg.sets_dir, cfg.work_dir):
        os.makedirs(path, mode=0o700, exist_ok=True)
        os.chmod(path, 0o700)


@contextlib.contextmanager
def locked(cfg: Config, wait: int | None = None, sleep=time.sleep):
    ensure_dirs(cfg)
    descriptor = os.open(os.path.join(cfg.backup_dir, ".lock"), os.O_RDWR | os.O_CREAT, 0o600)
    deadline = time.monotonic() + (LOCK_WAIT_SECONDS if wait is None else wait)
    try:
        while True:
            try:
                fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                if time.monotonic() >= deadline:
                    raise BackupError("another backup or restore drill is still running") from None
                sleep(5)
        yield
    finally:
        os.close(descriptor)


def lock_busy(cfg: Config) -> bool:
    path = os.path.join(cfg.backup_dir, ".lock")
    if not os.path.exists(path):
        return False
    descriptor = os.open(path, os.O_RDWR)
    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return False
    except BlockingIOError:
        return True
    finally:
        os.close(descriptor)


def local_sets(cfg: Config) -> dict:
    if not os.path.isdir(cfg.sets_dir):
        return {}
    return {name: os.path.getsize(os.path.join(cfg.sets_dir, name)) for name in os.listdir(cfg.sets_dir)
            if SET_NAME.fullmatch(name)}


def newest(names):
    names = [name for name in names if SET_NAME.fullmatch(name)]
    return max(names, key=lambda n: (set_time(n), n)) if names else None


def free_bytes(path: str) -> int:
    return shutil.disk_usage(path).free


# ---------------------------------------------------------------- backup

def run_backup(ctx: Context, reason: str = "nightly", release: str | None = None, local_only: bool = False) -> dict:
    cfg = ctx.cfg
    if reason not in REASONS:
        raise BackupError(f"unknown reason {reason!r}")
    if release is not None and not RELEASE.fullmatch(release):
        raise BackupError("--release must be a release name like 0.45.1-ru-abc1234")
    problems = [] if local_only else config_problems(cfg)
    stamp = time.strftime(STAMP, time.gmtime(ctx.now()))
    name = f"omoba-ru-{stamp}-{reason}.tar.gpg"
    with locked(cfg):
        attempt = {"name": name, "reason": reason, "release": release, "local_only": local_only,
                   "started_at": iso(ctx.now())}
        ctx.state["last_attempt"] = attempt
        ctx.save()
        try:
            if problems:
                raise BackupError("backups are not configured: " + "; ".join(text for _, text in problems))
            summary = backup_steps(ctx, name, stamp, reason, release, local_only)
        except BaseException as error:
            text = redact(str(error) or type(error).__name__, cfg.secrets())
            attempt.update(finished_at=iso(ctx.now()), ok=False, error=text)
            ctx.save()
            if not local_only:
                ctx.alert("backup-failed", f"backup {name} failed: {text}")
            if isinstance(error, BackupError):
                raise BackupError(text) from None
            raise
        attempt.update(finished_at=iso(ctx.now()), ok=True)
        if local_only:
            ctx.state["last_local_only"] = {**summary, "at": iso(ctx.now())}
        else:
            ctx.state["last_success"] = {**summary, "at": iso(ctx.now())}
            ctx.state.setdefault("first_success_at", iso(ctx.now()))
            (ctx.state.get("alerts") or {}).pop("backup-failed", None)
        ctx.save()
    ctx.log(f"backup {name}: {json.dumps(summary, sort_keys=True)}")
    return summary


def backup_steps(ctx: Context, name: str, stamp: str, reason: str, release: str | None, local_only: bool) -> dict:
    cfg = ctx.cfg
    ensure_dirs(cfg)
    for leftover in os.listdir(cfg.work_dir):   # the lock is held: nothing else uses them
        shutil.rmtree(os.path.join(cfg.work_dir, leftover), ignore_errors=True)
    for leftover in os.listdir(cfg.sets_dir):
        if leftover.endswith(".part"):
            os.unlink(os.path.join(cfg.sets_dir, leftover))
    previous = max(local_sets(cfg).values() or [0])
    if free_bytes(cfg.backup_dir) < MIN_FREE_BYTES + 3 * previous:
        raise BackupError(f"only {free_bytes(cfg.backup_dir) >> 20} MiB free in {cfg.backup_dir}")
    work = tempfile.mkdtemp(prefix=stamp + "-", dir=cfg.work_dir)
    try:
        dump = os.path.join(work, "database.dump")
        pg_dump(cfg, cfg.database, dump)
        scan = scan_dump(dump)
        live = live_versions(cfg, cfg.database)
        rollback = None
        if reason == "pre-migration" and release:
            rollback = os.path.join(cfg.backup_dir, f"{cfg.database}-before-{release}-{stamp}.dump")
            try:
                os.link(dump, rollback)
            except OSError:
                shutil.copyfile(dump, rollback)
            os.chmod(rollback, 0o600)
        summary = {"name": None if local_only else name, "reason": reason, "release": release,
                   "rollback_dump": rollback, "tables": len(scan["tables"]),
                   "rows": sum(table["rows"] for table in scan["tables"].values()),
                   "schema_versions": scan["schema_versions"], "dump_bytes": os.path.getsize(dump)}
        if scan["schema_versions"] != {key: value for key, value in live.items() if key in scan["schema_versions"]}:
            raise BackupError(f"the dump's schema versions {scan['schema_versions']} differ from the live "
                              f"database {live}; a migration ran during the backup")
        if local_only:
            prune_rollback_dumps(cfg)
            return {**summary, "mode": "local-only"}
        state_archive = os.path.join(work, "state.tar.gz")
        state = archive_state(cfg.paths("OMOBA_BACKUP_STATE_DIRS"), state_archive)
        manifest = {
            "format": MANIFEST_FORMAT, "market": MARKET, "tool": TOOL, "name": name, "reason": reason,
            "release": release, "created_at": iso(ctx.now()), "host": socket.gethostname(),
            "database": {"name": cfg.database, "file": "database.dump", "bytes": os.path.getsize(dump),
                         "sha256": sha256_file(dump), "tables": scan["tables"],
                         "schema_versions": scan["schema_versions"]},
            "state": {"file": "state.tar.gz", "bytes": os.path.getsize(state_archive),
                      "sha256": sha256_file(state_archive), **state},
        }
        local = os.path.join(cfg.sets_dir, name)
        seal(cfg, work, manifest, local)
        summary.update(set_bytes=os.path.getsize(local), set_sha256=sha256_file(local),
                       state_files=state["files"], state_missing=state["missing"])
    finally:
        shutil.rmtree(work, ignore_errors=True)
    summary.update(sync_offhost(ctx, name))
    prune_rollback_dumps(cfg)
    return summary


def seal(cfg: Config, work: str, manifest: dict, out: str) -> None:
    """tar(database.dump, state.tar.gz, manifest.json) streamed into gpg, encrypted to the
    backup public key and signed with the backup secret key; renamed when complete."""
    part = out + ".part"
    data = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode()

    def normalized(info: tarfile.TarInfo) -> tarfile.TarInfo:
        info.uid = info.gid = 0
        info.uname = info.gname = "root"
        info.mode = 0o600
        return info
    with gpg_home(agent=True) as home:
        signer = import_identity(cfg, home)
        process, errors = encrypt_to(cfg.recipient, part, home, signer=signer)
        try:
            with tarfile.open(fileobj=process.stdin, mode="w|") as archive:
                for member in SET_MEMBERS[:2]:
                    archive.add(os.path.join(work, member), arcname=member, filter=normalized)
                info = normalized(tarfile.TarInfo("manifest.json"))
                info.size, info.mtime = len(data), int(time.time())
                archive.addfile(info, io.BytesIO(data))
            process.stdin.close()
        except BaseException:
            process.kill()
            process.wait()
            errors.close()
            with contextlib.suppress(FileNotFoundError):
                os.unlink(part)
            raise
        finish(process, errors, "gpg --encrypt")
    os.chmod(part, 0o600)
    os.replace(part, out)


def sync_offhost(ctx: Context, name: str) -> dict:
    """Upload the new set and any earlier one the target lacks, check sizes, then prune the
    target (only when the new set is there) and the host."""
    cfg = ctx.cfg
    target = ctx.target()
    local = local_sets(cfg)
    remote = target.list()
    wanted = retained(set(local) | set(remote), cfg.number("OMOBA_BACKUP_KEEP_DAILY"),
                      cfg.number("OMOBA_BACKUP_KEEP_WEEKLY"))
    uploaded = []
    for set_name in sorted(local, key=set_time):
        if remote.get(set_name) != local[set_name] and (set_name in wanted or set_name == name):
            target.put(os.path.join(cfg.sets_dir, set_name), set_name)
            uploaded.append(set_name)
    remote = target.list()
    for set_name in uploaded:
        if remote.get(set_name) != local[set_name]:
            raise BackupError(f"{set_name} is not on {target.describe()} after the upload "
                              f"({remote.get(set_name)} of {local[set_name]} bytes)")
    keep = retained(remote, cfg.number("OMOBA_BACKUP_KEEP_DAILY"), cfg.number("OMOBA_BACKUP_KEEP_WEEKLY"))
    pruned = sorted(set(remote) - keep, key=set_time)
    if name in remote and pruned:
        target.delete(pruned)
    confirmed = {set_name for set_name, size in remote.items() if local.get(set_name) == size}
    prune_local(cfg, confirmed)
    return {"target": target.describe(), "uploaded": uploaded, "pruned_off_host": pruned if name in remote else [],
            "off_host_sets": len(remote) - (len(pruned) if name in remote else 0)}


def prune_local(cfg: Config, confirmed: set) -> list:
    """Keep the newest OMOBA_BACKUP_KEEP_LOCAL sets; an older one stays while it is not off-host
    yet and the retention policy still wants it."""
    local = local_sets(cfg)
    recent = set(sorted(local, key=set_time, reverse=True)[:cfg.number("OMOBA_BACKUP_KEEP_LOCAL")])
    policy = retained(local, cfg.number("OMOBA_BACKUP_KEEP_DAILY"), cfg.number("OMOBA_BACKUP_KEEP_WEEKLY"))
    removed = []
    for set_name in local:
        if set_name not in recent and (set_name in confirmed or set_name not in policy):
            os.unlink(os.path.join(cfg.sets_dir, set_name))
            removed.append(set_name)
    return removed


def prune_rollback_dumps(cfg: Config) -> list:
    """Plain pre-migration dumps (for `ru_host.py rollback`): the newest three stay."""
    dumps = sorted((name for name in os.listdir(cfg.backup_dir) if ROLLBACK_DUMP.fullmatch(name)),
                   key=lambda n: ROLLBACK_DUMP.fullmatch(n).group(1), reverse=True)
    for name in dumps[KEEP_ROLLBACK_DUMPS:]:
        os.unlink(os.path.join(cfg.backup_dir, name))
    return dumps[KEEP_ROLLBACK_DUMPS:]


# ---------------------------------------------------------------- fetch and the restore drill

def obtain(ctx: Context, wanted: str, source: str, into: str) -> str:
    """Path of set `wanted` ('latest' or a name) from the target (remote) or this host (local)."""
    cfg = ctx.cfg
    if wanted != "latest" and not SET_NAME.fullmatch(wanted):
        raise BackupError(f"{wanted!r} is not a set name (omoba-ru-<UTC>-<reason>.tar.gpg) or 'latest'")
    if source == "local":
        sets = local_sets(cfg)
        name = newest(sets) if wanted == "latest" else wanted
        if name not in sets:
            raise BackupError(f"no set {wanted} on this host ({cfg.sets_dir})")
        return os.path.join(cfg.sets_dir, name)
    target = ctx.target()
    sets = target.list()
    name = newest(sets) if wanted == "latest" else wanted
    if name not in sets:
        raise BackupError(f"no set {wanted} on {target.describe()}")
    path = os.path.join(into, name)
    target.get(name, path)
    if os.path.getsize(path) != sets[name]:
        raise BackupError(f"downloaded {os.path.getsize(path)} of {sets[name]} bytes of {name}")
    return path


def unseal(cfg: Config, set_path: str, out: str) -> dict:
    """Decrypt a set with the backup secret key, unpack its three members into `out` and
    verify them; nothing stays in `out` when any check fails. A set the backup key did not
    sign is refused: anyone with the public key or write access to the storage could make
    one, and its dump would run as the database superuser in the drill."""
    seen = set()
    try:
        return unseal_into(cfg, set_path, out, seen)
    except BaseException:
        for member in seen:
            with contextlib.suppress(FileNotFoundError):
                os.unlink(os.path.join(out, member))
        raise


def unseal_into(cfg: Config, set_path: str, out: str, seen: set) -> dict:
    with gpg_home(agent=True) as home:
        signer = import_identity(cfg, home)
        status_path = os.path.join(home, "status")
        errors = tempfile.TemporaryFile()
        process = subprocess.Popen(gpg(home, "--trust-model", "always", "--status-file", status_path, "--decrypt",
                                       set_path), stdout=subprocess.PIPE, stderr=errors, env=base_env(), cwd="/")
        try:
            with tarfile.open(fileobj=process.stdout, mode="r|") as archive:
                for member in archive:
                    if member.name not in SET_MEMBERS or not member.isfile() or member.name in seen:
                        raise BackupError(f"{os.path.basename(set_path)} holds an unexpected entry {member.name!r}")
                    seen.add(member.name)
                    descriptor = os.open(os.path.join(out, member.name), os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
                    with os.fdopen(descriptor, "wb") as handle:
                        shutil.copyfileobj(archive.extractfile(member), handle, 1 << 20)
            while process.stdout.read(1 << 16):
                pass
        except tarfile.TarError as error:
            process.kill()
            process.wait()
            errors.seek(0)
            detail = tail(errors.read(), 240)
            errors.close()
            raise BackupError(f"{os.path.basename(set_path)} could not be decrypted or unpacked ({error}; "
                              f"gpg: {detail})") from None
        except BaseException:
            process.kill()
            process.wait()
            errors.close()
            raise
        finally:
            process.stdout.close()
        code = process.wait()
        errors.seek(0)
        detail = errors.read()
        errors.close()
        status = Path(status_path).read_text(errors="replace") if os.path.exists(status_path) else ""
    decrypted = "[GNUPG:] DECRYPTION_OKAY" in status
    if code or signer not in signed_by(status):
        if code and not decrypted:
            raise BackupError(f"gpg --decrypt (wrong key or a damaged set) failed: {tail(detail)}")
        raise BackupError(f"{os.path.basename(set_path)} is not signed by the backup key {signer}: refused "
                          "(this host did not make it; someone with access to the storage may have)")
    if seen != set(SET_MEMBERS):
        raise BackupError(f"{os.path.basename(set_path)} lacks {sorted(set(SET_MEMBERS) - seen)}")
    manifest = json.loads(Path(out, "manifest.json").read_text())
    if manifest.get("format") != MANIFEST_FORMAT or manifest.get("market") != MARKET:
        raise BackupError("the manifest is not an RU backup manifest of a known format")
    for section, member in (("database", "database.dump"), ("state", "state.tar.gz")):
        if sha256_file(os.path.join(out, member)) != manifest[section]["sha256"]:
            raise BackupError(f"{member} does not match the manifest's sha256")
    counts = state_counts(os.path.join(out, "state.tar.gz"))
    if counts != {key: manifest["state"][key] for key in counts}:
        raise BackupError(f"state.tar.gz holds {counts}, the manifest says "
                          f"{ {key: manifest['state'][key] for key in counts} }")
    return manifest


def database_exists(cfg: Config, database: str) -> bool:
    return psql(cfg, "postgres", f"SELECT 1 FROM pg_database WHERE datname = {sql_literal(database)};").strip() == "1"


def create_scratch_sql(source: str, scratch: str) -> str:
    """CREATE DATABASE <scratch> with the source database's encoding and locale."""
    return (f"SELECT format('CREATE DATABASE %I TEMPLATE template0 ENCODING %L LOCALE_PROVIDER %s "
            f"LC_COLLATE %L LC_CTYPE %L%s', {sql_literal(scratch)}, pg_encoding_to_char(d.encoding), "
            "CASE j->>'datlocprovider' WHEN 'i' THEN 'icu' WHEN 'b' THEN 'builtin' ELSE 'libc' END, "
            "d.datcollate, d.datctype, CASE j->>'datlocprovider' "
            "WHEN 'i' THEN format(' ICU_LOCALE %L', coalesce(j->>'datlocale', j->>'daticulocale')) "
            "WHEN 'b' THEN format(' BUILTIN_LOCALE %L', j->>'datlocale') ELSE '' END) "
            f"FROM pg_database d, LATERAL to_jsonb(d) j WHERE d.datname = {sql_literal(source)} \\gexec\n"
            f"REVOKE ALL ON DATABASE {scratch} FROM PUBLIC;\n")


def drop_scratch(cfg: Config) -> None:
    scratch = cfg.drill_database
    if scratch == cfg.database or not scratch.endswith("_restore_drill") or not DB_NAME.fullmatch(scratch):
        raise BackupError(f"internal: refusing to drop {scratch}")
    psql(cfg, "postgres", f"DROP DATABASE IF EXISTS {scratch} WITH (FORCE);")


def restored_facts(cfg: Config, database: str, manifest_tables) -> dict:
    tables = json.loads(psql(cfg, database, """SELECT coalesce(json_agg(format('%s.%s', quote_ident(n.nspname),
 quote_ident(c.relname)) ORDER BY 1), '[]') FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
 WHERE c.relkind = 'r' AND n.nspname NOT IN ('pg_catalog', 'information_schema')
   AND n.nspname NOT LIKE 'pg\\_toast%' AND n.nspname NOT LIKE 'pg\\_temp%';""").strip())
    rows = {}
    names = [name for name in manifest_tables if QUALIFIED.fullmatch(name) and name in tables]
    if names:
        rows = json.loads(psql(cfg, database, "SELECT json_object_agg(t, n) FROM (VALUES " + ", ".join(
            f"({sql_literal(name)}, (SELECT count(*) FROM {name}))" for name in names) + ") AS v(t, n);").strip())
    return {"tables": tables, "rows": rows, "schema_versions": live_versions(cfg, database)}


def run_drill(ctx: Context, wanted: str = "latest", source: str = "remote") -> dict:
    cfg = ctx.cfg
    started = ctx.now()
    result = {"started_at": iso(started), "source": source, "ok": False}
    try:
        problems = config_problems(cfg)
        if source == "local":
            problems = [problem for problem in problems if not problem[0].startswith("target")]
        if problems:
            raise BackupError("the restore drill is not configured: " + "; ".join(text for _, text in problems))
        with locked(cfg):
            ensure_dirs(cfg)
            work = tempfile.mkdtemp(prefix="drill-", dir=cfg.work_dir)
            try:
                result.update(drill_steps(ctx, wanted, source, work))
            finally:
                shutil.rmtree(work, ignore_errors=True)
    except BaseException as error:
        result["error"] = redact(str(error) or type(error).__name__, cfg.secrets())
        result["finished_at"] = iso(ctx.now())
        ctx.state["last_drill"] = result
        ctx.save()
        ctx.alert("drill-failed", f"the restore drill failed: {result['error']}")
        if isinstance(error, BackupError):
            raise BackupError(result["error"]) from None
        raise
    result["finished_at"] = iso(ctx.now())
    result["seconds"] = round(ctx.now() - started)
    ctx.state["last_drill"] = result
    if result["ok"] and not result.get("skipped"):
        ctx.state["last_drill_success"] = result
        (ctx.state.get("alerts") or {}).pop("drill-failed", None)
    ctx.save()
    if not result["ok"]:
        ctx.alert("drill-failed", "the restore drill failed: " + "; ".join(result.get("errors") or []))
    ctx.log(f"restore drill: {json.dumps(result, sort_keys=True)}")
    return result


def drill_steps(ctx: Context, wanted: str, source: str, work: str) -> dict:
    cfg = ctx.cfg
    set_path = obtain(ctx, wanted, source, work)
    need = MIN_FREE_BYTES + DRILL_SPACE_FACTOR * os.path.getsize(set_path)
    for folder in (work, PG_DATA):
        if os.path.isdir(folder) and free_bytes(folder) < need:
            raise BackupError(f"only {free_bytes(folder) >> 20} MiB free in {folder}; the drill needs about "
                              f"{need >> 20} MiB")
    unpacked = os.path.join(work, "set")
    os.mkdir(unpacked, 0o700)
    manifest = unseal(cfg, set_path, unpacked)
    facts = {"name": os.path.basename(set_path), "created_at": manifest["created_at"],
             "tables": len(manifest["database"]["tables"]),
             "rows": sum(table["rows"] for table in manifest["database"]["tables"].values()),
             "state_files": manifest["state"]["files"], "schema_versions": manifest["database"]["schema_versions"]}
    live = live_versions(cfg, cfg.database) if database_exists(cfg, cfg.database) else {}
    if not manifest["database"]["schema_versions"] and not live:
        return {**facts, "ok": True, "skipped": "the database has no career or portal schema yet; nothing to drill"}
    errors, warnings = [], []
    missing = [key for key in VERSION_TABLES if key not in manifest["database"]["schema_versions"]]
    if missing:
        errors.append(f"the backup holds no {' or '.join(missing)} schema version table")
    drop_scratch(cfg)
    try:
        source = cfg.database if database_exists(cfg, cfg.database) else "template1"
        psql(cfg, "postgres", create_scratch_sql(source, cfg.drill_database))
        if not database_exists(cfg, cfg.drill_database):
            raise BackupError(f"could not create {cfg.drill_database}")
        with open(os.path.join(unpacked, "database.dump"), "rb") as dump:
            restored = subprocess.run([exe("pg_restore"), "--exit-on-error", "-w", "-d", cfg.drill_database],
                                      stdin=dump, capture_output=True, env=base_env(), cwd="/", timeout=6 * 3600,
                                      **db_user(cfg))
        if restored.returncode:
            raise BackupError(f"pg_restore into {cfg.drill_database} failed: {tail(restored.stderr)}")
        expected = manifest["database"]["tables"]
        got = restored_facts(cfg, cfg.drill_database, expected)
        redump = os.path.join(work, "redump.dump")
        pg_dump(cfg, cfg.drill_database, redump)
        rescan = scan_dump(redump)
    finally:
        drop_scratch(cfg)
    if sorted(got["tables"]) != sorted(expected):
        errors.append(f"tables differ: missing {sorted(set(expected) - set(got['tables']))}, "
                      f"extra {sorted(set(got['tables']) - set(expected))}")
    for table, info in sorted(expected.items()):
        if table not in got["rows"]:
            continue
        if got["rows"][table] != info["rows"]:
            errors.append(f"{table}: {got['rows'][table]} rows restored, {info['rows']} in the backup")
        elif rescan["tables"].get(table, {}).get("digest") != info["digest"]:
            errors.append(f"{table}: the restored rows differ from the backup")
    if got["schema_versions"] != manifest["database"]["schema_versions"]:
        errors.append(f"schema versions {got['schema_versions']} restored, "
                      f"{manifest['database']['schema_versions']} in the backup")
    if live and live != manifest["database"]["schema_versions"]:
        warnings.append(f"the live database is at {live}: a migration ran after this backup")
    age = ctx.now() - (parse_iso(manifest["created_at"]) or 0)
    if source == "remote" and age > MAX_BACKUP_AGE_HOURS * 3600:
        warnings.append(f"the newest off-host set is {round(age / 3600)} h old")
    return {**facts, "ok": not errors, "errors": errors[:20], "warnings": warnings}


def run_fetch(ctx: Context, wanted: str, out: str, source: str) -> dict:
    if os.path.exists(out) and os.listdir(out):
        raise BackupError(f"{out} is not empty")
    os.makedirs(out, mode=0o700, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="fetch-", dir=out) as download:
        set_path = obtain(ctx, wanted, source, download)
        manifest = unseal(ctx.cfg, set_path, out)
    return {"set": os.path.basename(set_path), "out": out, "created_at": manifest["created_at"],
            "schema_versions": manifest["database"]["schema_versions"], "files": list(SET_MEMBERS),
            "tables": len(manifest["database"]["tables"]), "state_files": manifest["state"]["files"]}


# ---------------------------------------------------------------- health

def escrow_paths(cfg: Config) -> list:
    """Regular files of OMOBA_BACKUP_ESCROW, folders expanded one level."""
    found = []
    for path in cfg.paths("OMOBA_BACKUP_ESCROW"):
        if os.path.isdir(path):
            found += [os.path.join(path, name) for name in sorted(os.listdir(path))
                      if os.path.isfile(os.path.join(path, name)) and not os.path.islink(os.path.join(path, name))]
        elif os.path.isfile(path) and not os.path.islink(path):
            found.append(path)
    return found


def escrow_problems(ctx: Context) -> list:
    record = ctx.state.get("escrow")
    if not record:
        return [("escrow-missing", "the secrets were never escrowed (README.md, \"Secrets escrow\": "
                                   "ru_host.py escrow, stored offline by the escrow holder)")]
    current = {}
    for path in escrow_paths(ctx.cfg):
        try:
            current[path] = sha256_file(path)
        except PermissionError:
            return [("escrow-unreadable", f"cannot read {path}: run as root")]
    changed = sorted(path for path in set(current) | set(record.get("files") or {})
                     if current.get(path) != (record.get("files") or {}).get(path))
    if changed:
        return [("escrow-stale", f"the escrow of {record.get('at')} is stale: {', '.join(changed)} changed "
                                 "since; escrow again")]
    return []


def health(ctx: Context, remote: bool = False) -> dict:
    cfg = ctx.cfg
    now = ctx.now()
    problems = list(config_problems(cfg))
    warnings, facts = [], {"config": cfg.path}
    try:
        facts["target"] = ctx.target().describe() if cfg.get("OMOBA_BACKUP_TARGET") else None
        if facts["target"] and facts["target"].startswith("file://"):
            warnings.append("the target is a local folder: it is off-host only if it mounts another machine")
    except BackupError:
        facts["target"] = None
    success = ctx.state.get("last_success") or {}
    attempt = ctx.state.get("last_attempt") or {}
    at = parse_iso(success.get("at"))
    facts.update(last_success=success.get("at"), last_set=success.get("name"),
                 local_sets=len(local_sets(cfg)) if os.path.isdir(cfg.backup_dir) else 0)
    if at is None:
        problems.append(("backup-missing", "no backup has succeeded yet"))
    elif now - at > MAX_BACKUP_AGE_HOURS * 3600:
        problems.append(("backup-stale", f"the last successful backup is {round((now - at) / 3600)} h old "
                                         f"(more than {MAX_BACKUP_AGE_HOURS} h)"))
    if attempt and attempt.get("ok") is False and not attempt.get("local_only"):
        problems.append(("backup-failed", f"the last backup run failed: {attempt.get('error')}"))
    started = parse_iso(attempt.get("started_at"))
    if attempt and "finished_at" not in attempt and started and now - started > STUCK_RUN_HOURS * 3600 \
            and not lock_busy(cfg):
        problems.append(("backup-interrupted", f"the backup started at {attempt['started_at']} never finished"))
    drill, drill_ok = ctx.state.get("last_drill") or {}, ctx.state.get("last_drill_success") or {}
    facts.update(last_drill=drill.get("finished_at"), last_drill_ok=drill.get("ok"),
                 last_drill_success=drill_ok.get("finished_at"))
    first = parse_iso(ctx.state.get("first_success_at"))
    drill_at = parse_iso(drill_ok.get("finished_at"))
    if drill and not drill.get("ok"):
        problems.append(("drill-failed", f"the last restore drill failed: {drill.get('error') or drill.get('errors')}"))
    elif drill_at and now - drill_at > MAX_DRILL_AGE_DAYS * 86400:
        problems.append(("drill-stale", f"the last successful restore drill is {round((now - drill_at) / 86400)} "
                                        f"days old (more than {MAX_DRILL_AGE_DAYS})"))
    elif drill_at is None and first and now - first > MAX_DRILL_AGE_DAYS * 86400:
        problems.append(("drill-missing", f"no restore drill has succeeded in the {MAX_DRILL_AGE_DAYS} days "
                                          "since the first backup"))
    elif drill_at is None:
        warnings.append("no restore drill has run yet (omoba-restore-drill-ru.timer runs it monthly; "
                        "`ru_backup.py drill` runs it now)")
    problems += escrow_problems(ctx)
    facts["escrow"] = (ctx.state.get("escrow") or {}).get("at")
    if remote and facts["target"]:
        try:
            sets = ctx.target().list()
        except BackupError as error:
            problems.append(("target-unreachable", f"the off-host storage did not answer: {error}"))
        else:
            latest = newest(sets)
            days = sorted({set_time(name).date().isoformat() for name in sets})
            facts.update(off_host_sets=len(sets), off_host_newest=latest, off_host_days=len(days),
                         off_host_oldest=days[0] if days else None)
            if latest is None:
                problems.append(("off-host-empty", f"{facts['target']} holds no backup set"))
            elif now - set_time(latest).timestamp() > MAX_BACKUP_AGE_HOURS * 3600:
                problems.append(("off-host-stale", f"the newest off-host set {latest} is more than "
                                                   f"{MAX_BACKUP_AGE_HOURS} h old"))
    seen, unique = set(), []
    for code, text in problems:
        if (code, text) not in seen:
            seen.add((code, text))
            unique.append((code, redact(text, cfg.secrets())))
    return {"ok": not unique, "problems": [text for _, text in unique], "codes": [code for code, _ in unique],
            "warnings": warnings, "facts": facts}


# ---------------------------------------------------------------- escrow and keys

ESCROW_README = """OMOBA RU secrets escrow (ops/vps/ru/ru_backup.py escrow, LR-03).

Created {at} on {host}. Files below keep their path on the RU host (without the leading /).
OMOBA_SOCIAL_SECRET (account-api.env) must never change: it keys every linked VK identity.
OMOBA_PORTAL_SECRET keys sessions, recovery codes and device codes. backup-keys/ holds the
backup encryption keys and the SFTP login. Restore on a new host: README.md, "Restore onto
a new host" (put the env files back before ru_host.py bootstrap; edit OMOBA_MATCH_PUBLIC_HOST).

{files}
"""


def run_escrow(ctx: Context | None, recipient_text: str, files: list | None, out, record: bool) -> dict:
    if "PRIVATE KEY" in recipient_text:
        raise BackupError("give the escrow holder's PUBLIC key; a private key was passed")
    info = key_info(recipient_text, text=True)
    if info["secret"] or not info["encrypt"]:
        raise BackupError("the escrow recipient key has no usable encryption (sub)key")
    if files:
        paths = []
        for path in files:
            if not os.path.isfile(path):
                raise BackupError(f"{path} is not a file")
            paths.append(os.path.abspath(path))
    else:
        paths = escrow_paths(ctx.cfg)
    if not paths:
        raise BackupError("nothing to escrow")
    try:
        digests = {path: sha256_file(path) for path in paths}
    except OSError as error:
        raise BackupError(f"cannot read {error.filename}: run as root") from None
    listing = "\n".join(f"{digest}  {path}" for path, digest in digests.items())
    now = (ctx.now if ctx else time.time)()
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w:gz") as archive:
        text = ESCROW_README.format(at=iso(now), host=socket.gethostname(), files=listing).encode()
        readme = tarfile.TarInfo("ESCROW.txt")
        readme.size, readme.mtime, readme.mode = len(text), int(now), 0o600
        archive.addfile(readme, io.BytesIO(text))
        for path in paths:
            archive.add(path, arcname=path.lstrip("/"), recursive=False)
    with gpg_home() as home:
        recipient = os.path.join(home, "recipient.asc")
        Path(recipient).write_text(recipient_text)
        sealed = os.path.join(home, "escrow.asc")
        process, errors = encrypt_to(recipient, sealed, home, armor=True)
        process.stdin.write(buffer.getvalue())
        process.stdin.close()
        finish(process, errors, "gpg --encrypt")
        data = Path(sealed).read_bytes()
    out.write(data)
    out.flush()
    summary = {"files": paths, "recipient": info["fingerprint"], "sha256": hashlib.sha256(data).hexdigest(),
               "bytes": len(data), "at": iso(now)}
    if record and ctx is not None:
        ctx.state["escrow"] = {"at": iso(now), "recipient": info["fingerprint"], "files": digests}
        ctx.save()
        ctx.resolve([code for code in (ctx.state.get("alerts") or {}) if not code.startswith("escrow")])
    return summary


KEY_PARAMS = """%no-protection
Key-Type: eddsa
Key-Curve: ed25519
Key-Usage: sign
Subkey-Type: ecdh
Subkey-Curve: cv25519
Subkey-Usage: encrypt
Name-Real: OMOBA RU backups {date}
Expire-Date: 0
%commit
"""


def run_keygen(out: str) -> dict:
    folder = Path(out)
    if folder.exists() and any(folder.iterdir()):
        raise BackupError(f"{out} is not empty")
    folder.mkdir(mode=0o700, parents=True, exist_ok=True)
    os.chmod(folder, 0o700)
    with gpg_home(agent=True) as home:
        made = subprocess.run(gpg(home, "--gen-key"), input=KEY_PARAMS.format(date=time.strftime("%Y-%m-%d")).encode(),
                              capture_output=True, env=base_env(), timeout=120)
        if made.returncode:
            raise BackupError(f"gpg --gen-key failed: {tail(made.stderr)}")
        public = subprocess.run(gpg(home, "--armor", "--export"), capture_output=True, env=base_env(), timeout=60)
        secret = subprocess.run(gpg(home, "--armor", "--export-secret-keys"), capture_output=True, env=base_env(),
                                timeout=60)
    if public.returncode or secret.returncode or b"PRIVATE KEY" not in secret.stdout:
        raise BackupError("gpg --export failed")
    public_path, secret_path = folder / "backup-public.asc", folder / "backup-secret.asc"
    public_path.write_bytes(public.stdout)
    os.chmod(public_path, 0o644)
    descriptor = os.open(secret_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "wb") as handle:
        handle.write(secret.stdout)
    return {"public": str(public_path), "secret": str(secret_path), "fingerprint": key_info(str(public_path))["fingerprint"]}


# ---------------------------------------------------------------- command line

def print_report(report: dict, as_json: bool, out=None) -> None:
    out = out or sys.stdout
    if as_json:
        print(json.dumps({key: value for key, value in report.items() if key != "codes"}, indent=2, sort_keys=True),
              file=out)
        return
    print("ok" if report["ok"] else "PROBLEMS", file=out)
    for problem in report["problems"]:
        print(f"  problem: {problem}", file=out)
    for warning in report.get("warnings", []):
        print(f"  warning: {warning}", file=out)
    for key, value in sorted(report.get("facts", {}).items()):
        print(f"  {key}: {value}", file=out)


def main(argv=None, stdin=None, stdout=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--env-file", default=ENV_FILE, help=f"settings (default {ENV_FILE})")
    sub = parser.add_subparsers(dest="command", required=True)
    backup = sub.add_parser("backup", help="dump, seal, upload and prune (nightly, and before every migration)")
    backup.add_argument("--reason", choices=REASONS, default="manual")
    backup.add_argument("--release", help="the release about to be migrated to (pre-migration)")
    backup.add_argument("--local-only", action="store_true",
                        help="only the plain rollback dump on this host (before launch, without storage)")
    drill = sub.add_parser("drill", help="restore a set into a scratch database and verify it")
    drill.add_argument("--set", default="latest")
    drill.add_argument("--from", dest="source", choices=("remote", "local"), default="remote")
    check = sub.add_parser("check", help="backup age, failures, drill, escrow and settings")
    check.add_argument("--json", action="store_true")
    check.add_argument("--remote", action="store_true", help="also list the off-host sets")
    check.add_argument("--alert", action="store_true", help="alert on problems (journal, Telegram)")
    config = sub.add_parser("config", help="are the settings complete?")
    config.add_argument("--json", action="store_true")
    listing = sub.add_parser("list", help="sets on this host and off-host")
    listing.add_argument("--json", action="store_true")
    fetch = sub.add_parser("fetch", help="download, decrypt, verify and unpack a set (restore)")
    fetch.add_argument("--set", default="latest")
    fetch.add_argument("--out", required=True)
    fetch.add_argument("--from", dest="source", choices=("remote", "local"), default="remote")
    escrow = sub.add_parser("escrow", help="encrypt the host's secrets to the escrow holder's key, to stdout")
    escrow.add_argument("--recipient-file", required=True, help="the holder's armored public key, or - for stdin")
    escrow.add_argument("--file", action="append", help="escrow these files instead (e.g. the Android "
                                                        "release keystore on the Mac); not recorded")
    keygen = sub.add_parser("keygen", help="make a backup key pair into an empty folder")
    keygen.add_argument("--out", required=True)
    args = parser.parse_args(argv)
    stdout = stdout or sys.stdout
    try:
        if args.command == "keygen":
            result = run_keygen(args.out)
            print(json.dumps(result, indent=2), file=stdout)
            print("Give the escrow holder backup-secret.asc (offline). Put backup-public.asc on the host as "
                  "OMOBA_BACKUP_GPG_RECIPIENT_FILE and backup-secret.asc as OMOBA_BACKUP_GPG_IDENTITY_FILE: it "
                  "signs the sets and the monthly drill decrypts with it (README.md, \"Backups\").", file=sys.stderr)
            return 0
        if args.command == "escrow" and args.file:
            text = (stdin or sys.stdin).read() if args.recipient_file == "-" else Path(args.recipient_file).read_text()
            summary = run_escrow(None, text, args.file, getattr(stdout, "buffer", stdout), record=False)
            print(json.dumps(summary, indent=2), file=sys.stderr)
            return 0
        ctx = Context(Config(args.env_file))
        if args.command == "backup":
            print(json.dumps(run_backup(ctx, args.reason, args.release, args.local_only), sort_keys=True), file=stdout)
            return 0
        if args.command == "drill":
            result = run_drill(ctx, args.set, args.source)
            print(json.dumps(result, indent=2, sort_keys=True), file=stdout)
            return 0 if result["ok"] else 1
        if args.command == "check":
            report = health(ctx, remote=args.remote)
            if args.alert:
                if report["problems"]:
                    ctx.alert_many(list(zip(report["codes"], report["problems"])))
                ctx.resolve(report["codes"])
            print_report(report, args.json, stdout)
            return 0 if report["ok"] else 1
        if args.command == "config":
            problems = config_problems(ctx.cfg)
            print_report({"ok": not problems, "problems": [text for _, text in problems], "warnings": [],
                          "facts": {"config": ctx.cfg.path}}, args.json, stdout)
            return 0 if not problems else 1
        if args.command == "list":
            local = local_sets(ctx.cfg)
            remote = ctx.target().list() if ctx.cfg.get("OMOBA_BACKUP_TARGET") else {}
            keep = retained(remote, ctx.cfg.number("OMOBA_BACKUP_KEEP_DAILY"), ctx.cfg.number("OMOBA_BACKUP_KEEP_WEEKLY"))
            rows = [{"name": name, "host_bytes": local.get(name), "off_host_bytes": remote.get(name),
                     "kept": name in keep or name not in remote}
                    for name in sorted(set(local) | set(remote), key=set_time, reverse=True)]
            if args.json:
                print(json.dumps(rows, indent=2), file=stdout)
            else:
                for row in rows:
                    print(f"{row['name']}  host={row['host_bytes']}  off-host={row['off_host_bytes']}", file=stdout)
            return 0
        if args.command == "fetch":
            print(json.dumps(run_fetch(ctx, args.set, args.out, args.source), indent=2), file=stdout)
            return 0
        if args.command == "escrow":
            text = (stdin or sys.stdin).read() if args.recipient_file == "-" else Path(args.recipient_file).read_text()
            summary = run_escrow(ctx, text, None, getattr(stdout, "buffer", stdout), record=True)
            print(json.dumps(summary, indent=2), file=sys.stderr)
            return 0
    except BackupError as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1
    return 2


if __name__ == "__main__":
    sys.exit(main())
