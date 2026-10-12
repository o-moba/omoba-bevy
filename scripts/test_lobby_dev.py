"""ops/vps/dev/lobby_dev.py: the DEV lobby tool only ever targets the dedicated dev VPS.

The host half runs against a temporary filesystem root, fake systemctl / pgrep /
ss / ip / id / useradd / install / ufw / docker (the dev Supabase Postgres) /
omoba-account-api and a fake resolver, so nothing here needs ssh, root, DNS,
Docker or systemd. The Mac half is exercised with ssh/scp mocked.

Two opt-in smoke tests use real binaries and are skipped otherwise:

- RealServerSmoke runs a built server with the dev unit's environment
  (OMOBA_LOBBY_SMOKE_SERVER).
- RealDatabaseSmoke runs the host half's database preparation, with a built
  omoba-account-api, against a disposable container of the Postgres image the dev
  Supabase stack runs, then signs the real lobby in with the file it wrote:

    cargo build -p server -p omoba-account-api
    docker run -d --rm --name omoba-lobby-smoke-db -e POSTGRES_PASSWORD="$(openssl rand -hex 16)" \\
        -p 127.0.0.1:59871:5432 public.ecr.aws/supabase/postgres:17.6.1.165
    OMOBA_LOBBY_SMOKE_SERVER=$CARGO_TARGET_DIR/debug/server \\
    OMOBA_LOBBY_SMOKE_ACCOUNT_API=$CARGO_TARGET_DIR/debug/omoba-account-api \\
    OMOBA_LOBBY_SMOKE_DB_CONTAINER=omoba-lobby-smoke-db OMOBA_LOBBY_SMOKE_DB_PORT=59871 \\
        python3 -m unittest discover -s scripts -p test_lobby_dev.py
    docker rm -f omoba-lobby-smoke-db
"""
from __future__ import annotations

import contextlib
import hashlib
import io
import json
import os
import re
import socket
import stat
import subprocess
import sys
import tarfile
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "ops/vps/dev"))

import lobby_dev  # noqa: E402

DEV_UNIT = (ROOT / "ops/vps/omoba-lobby-dev.service").read_text()
PROD_UNIT = (ROOT / "ops/vps/omoba-lobby.service").read_text()
GRANTS = (ROOT / lobby_dev.GRANTS).read_text()
DEV_IP = "203.0.113.10"
PROD_IP = "77.246.105.57"
GOOD_PASSWORD = "a" * 64
SHORT_PASSWORD = "s" * 5
GOOD_URL = f"postgres://omoba_dev_game:{GOOD_PASSWORD}@127.0.0.1:54322/omoba_dev"


def elf(machine: bytes = b"\x3e\x00", body: bytes = b"server") -> bytes:
    return b"\x7fELF" + bytes([2, 1, 1, 0]) + bytes(8) + b"\x02\x00" + machine + bytes(44) + body


TOOL = elf(body=b"omoba-account-api")
PASSWORD_IN_SQL = re.compile(r"PASSWORD '([^']+)'")


class FakeSystem:
    """Just enough of systemd, procps, iproute2, shadow-utils and ufw for the host half."""

    def __init__(self, root: str):
        self.root = root
        self.calls = []
        self.users = {}
        self.units = {
            "omoba-lobby-dev": {"active": False, "MainPID": "0", "NRestarts": "0", "LoadState": "not-found"},
        }
        self.enabled = set()
        self.extra_dev_pids = []
        self.udp = set()
        self.foreign_udp = set()
        self.ipv4 = {"127.0.0.1", DEV_IP}
        self.ufw = None  # None: not installed; otherwise the `ufw status` text
        self.dns = {"lobby.dev.omoba.io": {socket.AF_INET: {DEV_IP}, socket.AF_INET6: set()}}
        self.next_pid = 5000
        self.fail_start = False
        self.wrong_exe = False
        self.prod_marker_on_start = False
        self.reloads = 0
        self.restarts = 0
        # The dev Supabase Postgres container and what the tool did to it.
        self.db_running = True
        self.db_ports = ["127.0.0.1:54322"]
        self.roles = {}       # role -> can login
        self.passwords = {}   # role -> password
        self.database_owner = None
        self.schema = False
        self.sql = []         # (database, tag) of every psql batch
        self.migrations = []  # the URL each migration ran with
        self.grants = 0
        self.fail_migrate = False
        self.fail_grants = False

    def p(self, path):
        return os.path.join(self.root, path.lstrip("/"))

    def resolve(self, host, family):
        self.calls.append(["resolve", host, str(family)])
        return set(self.dns.get(host, {}).get(family, set()))

    def __call__(self, args, check=True, input=None, env=None, timeout=None):
        self.calls.append(list(args))
        code, out, *err = self.dispatch(list(args), input, env)
        if check and code:
            raise SystemExit(f"{' '.join(args)} failed")
        return subprocess.CompletedProcess(args, code, out, err[0] if err else "")

    def docker(self, args, sql):
        container = "supabase_db_ekza-dev"
        if args[0] == "inspect":
            assert args[-1] == container, args
            return (0, "true\n") if self.db_running else (1, "", "Error: No such object")
        if args[0] == "port":
            assert args[1:] == [container, "5432/tcp"], args
            return 0, "".join(f"{line}\n" for line in self.db_ports)
        if args[0] != "exec" or args[1:3] != ["-i", container]:
            raise AssertionError(f"the dev tool ran docker {args}")
        if not self.db_running:
            return 1, "", "container is not running"
        assert args[args.index("-U") + 1] == "supabase_admin" and "ON_ERROR_STOP=1" in args, args
        database = args[args.index("-d") + 1]
        assert database in ("postgres", "omoba_dev"), args
        tag = sql.splitlines()[0]
        self.sql.append((database, tag))
        if tag == "-- lobby_dev:state":
            return 0, json.dumps({"roles": self.roles, "database_owner": self.database_owner,
                                  "superuser": True}) + "\n"
        if tag == "-- lobby_dev:schema":
            return 0, "t\n" if self.schema else "f\n"
        if tag == "-- lobby_dev:bootstrap":
            for role in ("omoba_dev_owner", "omoba_dev_game", "omoba_dev_portal"):
                self.roles.setdefault(role, False)
            self.database_owner = self.database_owner or "omoba_dev_owner"
            self.roles["omoba_dev_owner"] = self.roles["omoba_dev_portal"] = False
            self.roles["omoba_dev_game"] = True
            self.passwords["omoba_dev_game"] = PASSWORD_IN_SQL.search(sql).group(1)
            return 0, ""
        if tag == "-- lobby_dev:owner-login":
            assert "VALID UNTIL" in sql
            self.roles["omoba_dev_owner"] = True
            self.passwords["omoba_dev_owner"] = PASSWORD_IN_SQL.search(sql).group(1)
            return 0, ""
        if tag == "-- lobby_dev:grants":
            assert database == "omoba_dev" and "SET ROLE omoba_dev_owner;" in sql and ':"game_role"' in sql
            assert "\\set game_role omoba_dev_game" in sql and "\\set portal_role omoba_dev_portal" in sql
            if self.fail_grants:
                return 3, "", "ERROR: permission denied"
            self.grants += 1
            return 0, ""
        if tag == "-- lobby_dev:owner-nologin":
            self.roles["omoba_dev_owner"] = False
            self.passwords.pop("omoba_dev_owner", None)
            return 0, ""
        raise AssertionError(f"unexpected SQL batch {tag}")

    def migrate(self, args, env):
        assert args[1:] == ["migrate"], args
        url = env["OMOBA_DATABASE_URL"]
        parts = urlsplit(url)
        assert (parts.username, parts.hostname, parts.port, parts.path) == \
            ("omoba_dev_owner", "127.0.0.1", 54322, "/omoba_dev"), url
        assert self.roles.get("omoba_dev_owner") and parts.password == self.passwords["omoba_dev_owner"]
        self.migrations.append(url)
        if self.fail_migrate:
            return 1, "", f"Career migration failed: cannot connect to {url}"
        self.schema = True
        return 0, "Career and Account API migrations ready.\n"

    def dispatch(self, args, sql=None, env=None):
        name = args[0]
        if name == "docker":
            return self.docker(args[1:], sql)
        if name.endswith("/omoba-account-api"):
            return self.migrate(args, env)
        if name == "systemctl":
            return self.systemctl(args[1:])
        if name == "pgrep":
            user = args[2]
            if user == "omoba-dev" and self.units["omoba-lobby-dev"]["active"]:
                pids = [self.units["omoba-lobby-dev"]["MainPID"], *map(str, self.extra_dev_pids)]
                return 0, "\n".join(pids) + "\n"
            return 1, ""
        if name == "ss":
            ports = sorted(self.udp | self.foreign_udp)
            return 0, "".join(f"UNCONN 0 0 0.0.0.0:{port} 0.0.0.0:*\n" for port in ports)
        if name == "ip":
            return 0, "".join(f"2: eth0    inet {ip}/24 brd x scope global eth0\\       valid_lft forever\n"
                              for ip in sorted(self.ipv4))
        if name == "ufw":
            if args[1:] != ["status"]:
                raise AssertionError(f"the dev tool changed the firewall: {args}")
            return (127, "") if self.ufw is None else (0, self.ufw)
        if name == "id":
            user = self.users.get(args[2])
            return (0, f"{user['uid']}\n") if user else (1, "")
        if name == "getent":
            user = self.users.get(args[2])
            if not user:
                return 2, ""
            return 0, f"{args[2]}:x:{user['uid']}:{user['uid']}::{user['home']}:{user['shell']}\n"
        if name == "useradd":
            self.users[args[-1]] = {"uid": 998, "home": args[args.index("--home-dir") + 1],
                                    "shell": args[args.index("--shell") + 1]}
            return 0, ""
        if name == "install":
            os.makedirs(args[-1], exist_ok=True)
            return 0, ""
        if name == "systemd-analyze":
            return 0, ""
        raise AssertionError(f"unexpected command {args}")

    def systemctl(self, args):
        verb = args[0]
        if verb == "show":
            unit = self.units.get(args[1], {"LoadState": "not-found"})
            return 0, str(unit.get(args[3], "")) + "\n"
        if verb == "is-active":
            return (0 if self.units.get(args[2], {}).get("active") else 3), ""
        if verb == "daemon-reload":
            self.reloads += 1
            return 0, ""
        if args[1] != "omoba-lobby-dev":
            raise AssertionError(f"the dev tool ran systemctl {verb} on {args[1]}")
        unit = self.units[args[1]]
        if verb == "stop":
            unit.update(active=False, MainPID="0")
            self.udp.discard(4100)
        elif verb in ("start", "restart"):
            self.restarts += 1
            if self.fail_start:
                unit.update(active=False, MainPID="0")
                return 1, ""
            pid = self.next_pid
            self.next_pid += 1
            unit.update(active=True, MainPID=str(pid))
            proc = self.p(f"/proc/{pid}")
            os.makedirs(proc, exist_ok=True)
            target = self.p("/usr/bin/other") if self.wrong_exe else \
                os.path.realpath(self.p("/opt/omoba-dev/current/server"))
            os.symlink(target, os.path.join(proc, "exe"))
            self.udp.add(4100)
            if self.prod_marker_on_start:
                os.makedirs(self.p("/opt/omoba"), exist_ok=True)
        elif verb == "enable":
            self.enabled.add(args[1])
        elif verb == "disable":
            self.enabled.discard(args[1])
        else:
            raise AssertionError(f"unexpected systemctl {args}")
        return 0, ""


def tree(root: str) -> dict:
    """Every path under root with its content or link target, for read-only checks."""
    found = {}
    for directory, dirs, files in os.walk(root):
        for name in dirs + files:
            path = os.path.join(directory, name)
            if os.path.islink(path):
                found[path] = "-> " + os.readlink(path)
            elif os.path.isfile(path):
                found[path] = Path(path).read_bytes()
            else:
                found[path] = "dir"
    return found


class HostCase(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = self.temporary.name
        self.fake = FakeSystem(self.root)
        os.makedirs(self.fake.p("/proc"))
        Path(self.fake.p("/proc/meminfo")).write_text("MemTotal: 4014000 kB\nMemAvailable: 1500000 kB\n")
        os.makedirs(self.fake.p("/etc/systemd/system"))
        self.host = self.fresh()

    def tearDown(self):
        for call in self.fake.calls:
            if call[0] in ("useradd", "install"):
                # Mutating commands name only the dev user and dev paths.
                for arg in call[1:]:
                    local = arg[len(self.root):] if arg.startswith(self.root) else arg
                    self.assertFalse(lobby_dev.touches_production(local), call)
                if call[0] == "useradd":
                    self.assertEqual(call[-1], "omoba-dev", call)
        self.temporary.cleanup()

    def fresh(self):
        return lobby_dev.Host(self.root, run=self.fake, sleep=lambda _: None, resolve=self.fake.resolve,
                              root_uid=os.getuid())

    def payload(self, release="0.44.0-dev-abc1234", body=b"dev-1", unit=DEV_UNIT):
        data = elf(body=body)
        digest = hashlib.sha256(data).hexdigest()
        tool = hashlib.sha256(TOOL).hexdigest()
        return {"release": release, "sha256": digest, "account_api_sha256": tool, "unit": unit,
                "grants": GRANTS, "expected_ip": DEV_IP,
                "build": {"contour": "dev", "release": release, "sha256": digest,
                          "account_api_sha256": tool}}, data

    def upload(self, payload, data, tool=TOOL):
        directory = f"/opt/omoba-dev/releases/{payload['release']}"
        Path(self.fake.p(f"{directory}/server.part")).write_bytes(data)
        if not os.path.exists(self.fake.p(f"{directory}/omoba-account-api")):
            Path(self.fake.p(f"{directory}/omoba-account-api.part")).write_bytes(tool)

    def ship(self, release="0.44.0-dev-abc1234", body=b"dev-1", unit=DEV_UNIT):
        payload, data = self.payload(release, body, unit)
        self.fresh().stage(payload)
        self.upload(payload, data)
        return self.fresh().activate(payload), payload

    def env_file(self):
        return Path(self.fake.p(lobby_dev.DATABASE_ENV))

    def current(self):
        return os.readlink(self.fake.p(lobby_dev.CURRENT))

    def assert_refused_without_change(self, action, *, before=None, contains=None):
        before = tree(self.root) if before is None else before
        with self.assertRaises(lobby_dev.Refused) as raised:
            action()
        if contains:
            self.assertIn(contains, raised.exception.message)
        self.assertEqual(tree(self.root), before)


class FirstInstallAndUpdate(HostCase):
    def test_preflight_is_read_only_and_lists_the_plan(self):
        before = tree(self.root)
        payload, _ = self.payload()
        result = self.host.preflight(payload)
        self.assertEqual(tree(self.root), before)
        self.assertTrue(result["ok"])
        self.assertEqual((result["problems"], result["public_host"]), ([], "lobby.dev.omoba.io"))
        self.assertTrue(any(step.startswith("useradd --system omoba-dev") for step in result["would"]))
        self.assertIn("/opt/omoba-dev/current -> releases/0.44.0-dev-abc1234", result["would"])
        mutating = [c for c in self.fake.calls if c[0] in ("useradd", "install")
                    or (c[0] == "systemctl" and c[1] not in ("show", "is-active"))]
        self.assertEqual(mutating, [])

    def test_first_install_creates_only_dev_state_and_starts_the_dev_unit(self):
        result, _ = self.ship()
        self.assertEqual(result["status"], "DEPLOYED")
        self.assertIsNone(result["previous"])
        self.assertTrue(result["unit_changed"])
        self.assertEqual(result["public_host"], "lobby.dev.omoba.io")
        self.assertEqual(self.current(), "releases/0.44.0-dev-abc1234")
        self.assertEqual(Path(self.fake.p(lobby_dev.UNIT_PATH)).read_text(), DEV_UNIT)
        self.assertEqual(self.fake.users["omoba-dev"]["home"], "/var/lib/omoba-dev")
        for path in lobby_dev.STATE_DIRS:
            self.assertTrue(os.path.isdir(self.fake.p(path)), path)
        self.assertEqual(self.fake.enabled, {"omoba-lobby-dev"})
        self.assertEqual(self.fake.reloads, 1)
        build = json.loads(Path(self.fake.p(
            "/opt/omoba-dev/releases/0.44.0-dev-abc1234/BUILD.json")).read_text())
        self.assertEqual(build["contour"], "dev")
        history = Path(self.fake.p(lobby_dev.HISTORY)).read_text().splitlines()
        self.assertEqual(json.loads(history[0])["release"], "0.44.0-dev-abc1234")
        for production in lobby_dev.PROD_PATHS:
            self.assertFalse(os.path.lexists(self.fake.p(production)), production)

    def test_update_switches_release_without_reloading_an_unchanged_unit(self):
        self.ship()
        result, _ = self.ship("0.44.0-dev-def5678", b"dev-2")
        self.assertEqual(result["previous"], "0.44.0-dev-abc1234")
        self.assertFalse(result["unit_changed"])
        self.assertEqual(self.current(), "releases/0.44.0-dev-def5678")
        self.assertEqual(self.fake.reloads, 1)

    def test_reshipping_the_running_release_is_a_no_op(self):
        self.ship()
        restarts, history = self.fake.restarts, Path(self.fake.p(lobby_dev.HISTORY)).read_text()
        payload, _ = self.payload()
        self.assertIn("nothing: omoba-lobby-dev already runs 0.44.0-dev-abc1234",
                      self.fresh().preflight(payload)["would"])
        self.assertEqual(self.fresh().stage(payload)["staged"], "already")
        result = self.fresh().activate(payload)
        self.assertEqual(result["status"], "UNCHANGED")
        self.assertEqual(self.fake.restarts, restarts)
        self.assertEqual(Path(self.fake.p(lobby_dev.HISTORY)).read_text(), history)

    def test_a_changed_unit_on_the_same_release_is_applied(self):
        self.ship()
        changed = lobby_dev.render_unit(DEV_UNIT, DEV_IP)
        payload, _ = self.payload(unit=changed)
        result = self.fresh().activate(payload)
        self.assertEqual((result["status"], result["unit_changed"]), ("DEPLOYED", True))
        self.assertIn(f"OMOBA_MATCH_PUBLIC_HOST={DEV_IP}", Path(self.fake.p(lobby_dev.UNIT_PATH)).read_text())

    def test_rollback_returns_to_a_staged_dev_release(self):
        self.ship()
        self.ship("0.44.0-dev-def5678", b"dev-2")
        result = self.fresh().rollback({"release": "0.44.0-dev-abc1234", "expected_ip": DEV_IP})
        self.assertEqual((result["mode"], self.current()), ("rollback", "releases/0.44.0-dev-abc1234"))

    def test_staged_releases_are_immutable(self):
        self.ship()
        payload, _ = self.payload(body=b"different")
        with self.assertRaises(lobby_dev.Refused):
            self.fresh().stage(payload)
        payload, _ = self.payload()
        payload["account_api_sha256"] = "0" * 64
        with self.assertRaises(lobby_dev.Refused):
            self.fresh().stage(payload)

    def test_stage_asks_only_for_missing_files(self):
        payload, data = self.payload()
        self.assertEqual(self.fresh().stage(payload)["upload_to"], [
            "/opt/omoba-dev/releases/0.44.0-dev-abc1234/server.part",
            "/opt/omoba-dev/releases/0.44.0-dev-abc1234/omoba-account-api.part"])
        self.upload(payload, data)
        self.fresh().activate(payload)
        self.assertEqual(self.fresh().stage(payload)["upload_to"], [])

    def test_status_is_read_only(self):
        self.ship()
        before = tree(self.root)
        status = self.fresh().status()
        self.assertEqual(tree(self.root), before)
        self.assertEqual((status["current"], status["public_host"]), ("0.44.0-dev-abc1234", "lobby.dev.omoba.io"))
        self.assertEqual(status["production_markers"], [])
        self.assertIn(DEV_IP, status["local_ipv4"])


class ProductionHostRefusals(HostCase):
    """Wherever production shows up, every mode refuses before any write."""

    MARKERS = {
        "prod unit file": lambda fake: Path(fake.p(lobby_dev.PROD_UNIT_PATH)).write_text(PROD_UNIT),
        "/opt/omoba": lambda fake: os.makedirs(fake.p("/opt/omoba/releases")),
        "/var/lib/omoba": lambda fake: os.makedirs(fake.p("/var/lib/omoba/matches")),
        "NPM edge": lambda fake: os.makedirs(fake.p("/root/docker/nginx-proxy-manager")),
        "/opt/ekza-studio": lambda fake: os.makedirs(fake.p("/opt/ekza-studio/config")),
        "loaded prod unit": lambda fake: fake.units.update({"omoba-lobby": {"LoadState": "loaded"}}),
        "prod user": lambda fake: fake.users.update(
            {"omoba": {"uid": 999, "home": "/var/lib/omoba", "shell": "/usr/sbin/nologin"}}),
        "prod address": lambda fake: fake.ipv4.add(PROD_IP),
    }

    def test_every_production_marker_blocks_every_mode(self):
        for name, plant in self.MARKERS.items():
            with self.subTest(marker=name):
                self.tearDown()
                self.setUp()
                self.ship()
                payload, data = self.payload("0.44.0-dev-def5678", b"dev-2")
                self.fresh().stage(payload)
                self.upload(payload, data)
                plant(self.fake)
                before = tree(self.root)
                for action in (lambda: self.fresh().preflight(payload), lambda: self.fresh().stage(payload),
                               lambda: self.fresh().activate(payload),
                               lambda: self.fresh().rollback({"release": "0.44.0-dev-abc1234"})):
                    self.assert_refused_without_change(action, before=before, contains="production VPS")
                self.assertEqual(self.current(), "releases/0.44.0-dev-abc1234")
        # Without a marker the same switch goes through.
        self.tearDown()
        self.setUp()
        self.ship()
        self.assertEqual(self.ship("0.44.0-dev-def5678", b"dev-2")[0]["release"], "0.44.0-dev-def5678")

    def test_a_production_marker_appearing_during_the_switch_is_reported(self):
        self.fake.prod_marker_on_start = True
        with self.assertRaises(SystemExit) as raised:
            self.ship()
        self.assertIn("PRODUCTION MARKERS APPEARED", str(raised.exception))


class Refusals(HostCase):
    def test_active_dev_matches_are_never_interrupted(self):
        self.ship()
        self.fake.extra_dev_pids = [6001]
        payload, data = self.payload("0.44.0-dev-def5678", b"dev-2")
        self.fresh().stage(payload)
        self.upload(payload, data)
        calls = len(self.fake.calls)
        with self.assertRaises(lobby_dev.Refused):
            self.fresh().activate(payload)
        self.assertNotIn(["systemctl", "stop", "omoba-lobby-dev"], self.fake.calls[calls:])
        self.assertEqual(self.current(), "releases/0.44.0-dev-abc1234")

    def test_dev_directories_must_not_be_symlinks(self):
        os.makedirs(self.fake.p("/srv/elsewhere"))
        os.makedirs(self.fake.p("/opt"))
        os.symlink(self.fake.p("/srv/elsewhere"), self.fake.p("/opt/omoba-dev"))
        payload, _ = self.payload()
        self.assert_refused_without_change(lambda: self.fresh().stage(payload))

    def test_current_must_point_into_dev_releases(self):
        os.makedirs(self.fake.p("/opt/omoba-dev/releases"))
        os.makedirs(self.fake.p("/srv/other"))
        os.symlink(self.fake.p("/srv/other"), self.fake.p("/opt/omoba-dev/current"))
        payload, _ = self.payload()
        self.assert_refused_without_change(lambda: self.fresh().preflight(payload))

    def test_a_dev_user_with_root_uid_or_a_shell_is_refused(self):
        payload, _ = self.payload()
        for entry in ({"uid": 0, "home": "/var/lib/omoba-dev", "shell": "/usr/sbin/nologin"},
                      {"uid": 998, "home": "/var/lib/omoba-dev", "shell": "/bin/bash"},
                      {"uid": 998, "home": "/home/omoba-dev", "shell": "/usr/sbin/nologin"}):
            self.fake.users["omoba-dev"] = entry
            self.assert_refused_without_change(lambda: self.fresh().stage(payload))

    def test_dev_ports_held_by_someone_else_are_refused(self):
        self.fake.foreign_udp = {41100}
        payload, _ = self.payload()
        self.assert_refused_without_change(lambda: self.fresh().preflight(payload))

    def test_low_memory_is_refused(self):
        Path(self.fake.p("/proc/meminfo")).write_text("MemAvailable: 400000 kB\n")
        payload, _ = self.payload()
        self.assert_refused_without_change(lambda: self.fresh().preflight(payload))

    def test_a_production_unit_is_refused_on_the_host_too(self):
        payload, _ = self.payload(unit=PROD_UNIT)
        self.assert_refused_without_change(lambda: self.fresh().preflight(payload))
        self.fresh().stage(payload)
        self.upload(payload, self.payload()[1])
        self.assert_refused_without_change(lambda: self.fresh().activate(payload))

    def test_tampered_upload_is_removed_and_nothing_switches(self):
        self.ship()
        payload, _ = self.payload("0.44.0-dev-def5678", b"dev-2")
        self.fresh().stage(payload)
        self.upload(payload, elf(body=b"tampered"))
        with self.assertRaises(lobby_dev.Refused):
            self.fresh().activate(payload)
        release = self.fake.p("/opt/omoba-dev/releases/0.44.0-dev-def5678")
        self.assertFalse(os.path.exists(release + "/server.part"))
        self.assertFalse(os.path.exists(release + "/server"), "a tampered upload became the release binary")
        self.assertEqual(self.current(), "releases/0.44.0-dev-abc1234")

    def test_a_tampered_migration_tool_never_runs(self):
        self.ship()
        payload, data = self.payload("0.44.0-dev-def5678", b"dev-2")
        self.fresh().stage(payload)
        self.upload(payload, data, tool=elf(body=b"tampered tool"))
        migrations = len(self.fake.migrations)
        with self.assertRaises(lobby_dev.Refused):
            self.fresh().activate(payload)
        self.assertEqual(len(self.fake.migrations), migrations)
        self.assertFalse(os.path.exists(self.fake.p(
            "/opt/omoba-dev/releases/0.44.0-dev-def5678/omoba-account-api")))

    def test_rollback_needs_a_dev_build_record(self):
        self.ship()
        os.makedirs(self.fake.p("/opt/omoba-dev/releases/0.44.0-dev-0000000"))
        self.assert_refused_without_change(lambda: self.fresh().rollback({"release": "0.44.0-dev-0000000"}))
        build = self.fake.p("/opt/omoba-dev/releases/0.44.0-dev-abc1234/BUILD.json")
        Path(build).write_text(json.dumps({"contour": "prod", "release": "0.44.0-dev-abc1234", "sha256": "0"}))
        self.assert_refused_without_change(lambda: self.fresh().rollback({"release": "0.44.0-dev-abc1234"}))

    def test_release_names_cannot_escape_the_dev_release_directory(self):
        for name in ("../../omoba/releases/x", "0.44.0-beta-1952af2", "0.44.0-dev-XYZ", ""):
            with self.assertRaises(lobby_dev.Refused, msg=name):
                self.host.release_dir(name)

    def test_read_only_modes_cannot_write(self):
        with self.assertRaises(lobby_dev.Refused):
            self.host._point_current("0.44.0-dev-abc1234")
        self.host.mutating = True
        with self.assertRaises(lobby_dev.Refused):
            self.host.write_guard("/opt/omoba/current")


class PublicHostAndFirewall(HostCase):
    def staged(self, unit=DEV_UNIT):
        payload, data = self.payload(unit=unit)
        self.fresh().stage(payload)
        self.upload(payload, data)
        return payload

    def test_a_missing_or_wrong_a_record_blocks_activation(self):
        payload = self.staged()
        for records, expected in (({}, "has no A record"),
                                  ({socket.AF_INET: {"198.51.100.7"}}, "not the dev VPS"),
                                  ({socket.AF_INET: {PROD_IP}}, "production VPS"),
                                  ({socket.AF_INET: {DEV_IP}, socket.AF_INET6: {"2001:db8::1"}}, "AAAA")):
            self.fake.dns["lobby.dev.omoba.io"] = records
            preflight = self.fresh().preflight(payload)
            self.assertFalse(preflight["ok"])
            self.assertTrue(any(expected in problem for problem in preflight["problems"]), preflight)
            self.assert_refused_without_change(lambda: self.fresh().activate(payload), contains=expected)

    def test_v4_mapped_answers_are_not_aaaa_records(self):
        self.fake.dns["lobby.dev.omoba.io"][socket.AF_INET6] = {f"::ffff:{DEV_IP}"}
        self.assertTrue(self.fresh().preflight(self.payload()[0])["ok"])

    def test_an_ip_public_host_needs_no_dns_but_must_be_the_dev_vps(self):
        self.fake.dns = {}
        payload = self.staged(lobby_dev.render_unit(DEV_UNIT, DEV_IP))
        self.assertTrue(self.fresh().preflight(payload)["ok"])
        self.assertEqual(self.fresh().activate(payload)["public_host"], DEV_IP)
        other = dict(payload, unit=lobby_dev.render_unit(DEV_UNIT, "198.51.100.7"))
        self.assertFalse(self.fresh().preflight(other)["ok"])

    def test_firewall_is_reported_never_changed(self):
        payload, _ = self.payload()
        self.fake.ufw = "Status: inactive\n"
        self.assertEqual(self.fresh().preflight(payload)["warnings"], [])
        self.fake.ufw = "Status: active\n\nTo Action From\n22/tcp ALLOW Anywhere\n4100/udp ALLOW Anywhere\n"
        warnings = self.fresh().preflight(payload)["warnings"]
        self.assertEqual(warnings, ["ufw is active without 41100/udp: run ufw allow 41100/udp comment omoba-lobby-dev"])
        self.fake.ufw += "41100/udp ALLOW Anywhere\n"
        self.assertEqual(self.fresh().preflight(payload)["warnings"], [])

    def test_an_address_missing_from_the_interfaces_is_only_a_warning(self):
        self.fake.ipv4 = {"127.0.0.1", "10.0.0.5"}
        result = self.fresh().preflight(self.payload()[0])
        self.assertTrue(result["ok"])
        self.assertTrue(any("not on a local interface" in warning for warning in result["warnings"]))


class FailuresRollBack(HostCase):
    def test_failed_start_restores_the_previous_release_and_unit(self):
        self.ship()
        changed = DEV_UNIT.replace("RestartSec=5", "RestartSec=7")
        payload, data = self.payload("0.44.0-dev-def5678", b"dev-2", unit=changed)
        self.fresh().stage(payload)
        self.upload(payload, data)
        self.fake.fail_start = True
        with self.assertRaises(SystemExit) as raised:
            self.fresh().activate(payload)
        self.assertIn("ROLLED_BACK to 0.44.0-dev-abc1234", str(raised.exception))
        self.assertEqual(self.current(), "releases/0.44.0-dev-abc1234")
        self.assertEqual(Path(self.fake.p(lobby_dev.UNIT_PATH)).read_text(), DEV_UNIT)

    def test_wrong_executable_rolls_back(self):
        self.ship()
        payload, data = self.payload("0.44.0-dev-def5678", b"dev-2")
        self.fresh().stage(payload)
        self.upload(payload, data)
        self.fake.wrong_exe = True
        with self.assertRaises(SystemExit) as raised:
            self.fresh().activate(payload)
        self.assertIn("ROLLED_BACK", str(raised.exception))
        self.assertEqual(self.current(), "releases/0.44.0-dev-abc1234")

    def test_failed_first_install_leaves_the_dev_unit_stopped_and_disabled(self):
        self.fake.fail_start = True
        with self.assertRaises(SystemExit) as raised:
            self.ship()
        self.assertIn("left stopped and disabled", str(raised.exception))
        self.assertNotIn("omoba-lobby-dev", self.fake.enabled)
        self.assertFalse(self.fake.units["omoba-lobby-dev"]["active"])


class CareerDatabase(HostCase):
    """The dev lobby's database: dev Supabase Postgres on this host, generated password, no secret out."""

    def assert_no_secret(self, *texts):
        hidden = [value for value in self.fake.passwords.values()] + \
                 [urlsplit(url).password for url in self.fake.migrations]
        for text in texts:
            for value in hidden:
                self.assertNotIn(value, text)

    def test_first_install_creates_the_database_and_a_root_only_env_file(self):
        result, _ = self.ship()
        self.assertEqual(result["database"], {"database": "omoba_dev", "env_written": True,
                                              "created_roles": ["omoba_dev_owner", "omoba_dev_game",
                                                                "omoba_dev_portal"],
                                              "created_database": True})
        mode = stat.S_IMODE(os.stat(self.env_file()).st_mode)
        self.assertEqual(mode, 0o600)
        url, problems = lobby_dev.parse_database_env(self.env_file().read_text())
        self.assertEqual(problems, [])
        self.assertEqual(urlsplit(url).password, self.fake.passwords["omoba_dev_game"])
        self.assertEqual(self.fake.database_owner, "omoba_dev_owner")
        self.assertTrue(self.fake.schema)
        self.assertEqual((len(self.fake.migrations), self.fake.grants), (1, 1))
        # The owner logged in only for the migration, with a different, expiring password.
        self.assertFalse(self.fake.roles["omoba_dev_owner"])
        self.assertNotIn("omoba_dev_owner", self.fake.passwords)
        self.assertNotEqual(urlsplit(self.fake.migrations[0]).password, self.fake.passwords["omoba_dev_game"])
        tags = [tag for _, tag in self.fake.sql]
        self.assertLess(tags.index("-- lobby_dev:bootstrap"), tags.index("-- lobby_dev:owner-login"))
        self.assertLess(tags.index("-- lobby_dev:grants"), tags.index("-- lobby_dev:owner-nologin"))
        # The database is ready before the lobby (and its match workers) start.
        first_start = next(i for i, call in enumerate(self.fake.calls) if call[:2] == ["systemctl", "restart"])
        migrate = next(i for i, call in enumerate(self.fake.calls) if call[0].endswith("/omoba-account-api"))
        self.assertLess(migrate, first_start)
        # No secret in argv, the result, the history or BUILD.json.
        self.assert_no_secret(json.dumps(self.fake.calls), json.dumps(result),
                              Path(self.fake.p(lobby_dev.HISTORY)).read_text(),
                              json.dumps(self.fresh().status()))

    def test_preflight_lists_the_database_steps_and_only_reads(self):
        payload, _ = self.payload()
        result = self.fresh().preflight(payload)
        self.assertTrue(result["ok"], result)
        self.assertTrue(any("create database omoba_dev" in step for step in result["would"]))
        self.assertTrue(any("into /opt/omoba-dev/database.env (root, 0600)" in step for step in result["would"]))
        self.assertEqual({tag for _, tag in self.fake.sql}, {"-- lobby_dev:state"})
        self.assertFalse(self.env_file().exists())

    def test_updates_keep_the_password_and_migrate_each_new_release(self):
        self.ship()
        password, text = self.fake.passwords["omoba_dev_game"], self.env_file().read_text()
        result, _ = self.ship("0.44.0-dev-def5678", b"dev-2")
        self.assertEqual(result["database"]["created_roles"], [])
        self.assertFalse(result["database"]["env_written"])
        self.assertEqual((self.fake.passwords["omoba_dev_game"], self.env_file().read_text()), (password, text))
        self.assertEqual((len(self.fake.migrations), self.fake.grants), (2, 2))
        payload, _ = self.payload("0.44.0-dev-def5678", b"dev-2")
        self.assertEqual(self.fresh().activate(payload)["status"], "UNCHANGED")
        self.assertEqual(len(self.fake.migrations), 2)

    def test_reshipping_after_a_supabase_reset_rebuilds_the_database_without_a_restart(self):
        self.ship()
        password, restarts = self.fake.passwords["omoba_dev_game"], self.fake.restarts
        self.fake.roles, self.fake.passwords, self.fake.database_owner, self.fake.schema = {}, {}, None, False
        payload, _ = self.payload()
        would = self.fresh().preflight(payload)["would"]
        self.assertIn("keep omoba-lobby-dev running 0.44.0-dev-abc1234 (no restart)", would)
        result = self.fresh().activate(payload)
        self.assertEqual(result["status"], "DATABASE_PREPARED")
        self.assertEqual((self.fake.restarts, self.fake.passwords["omoba_dev_game"]), (restarts, password))
        self.assertTrue(self.fake.schema)
        self.assertEqual(self.fresh().activate(payload)["status"], "UNCHANGED")

    def test_a_recreated_supabase_volume_gets_its_roles_back_with_the_same_password(self):
        self.ship()
        password = self.fake.passwords["omoba_dev_game"]
        self.fake.roles, self.fake.passwords, self.fake.database_owner, self.fake.schema = {}, {}, None, False
        result, _ = self.ship("0.44.0-dev-def5678", b"dev-2")
        self.assertTrue(result["database"]["created_database"])
        self.assertEqual(self.fake.passwords["omoba_dev_game"], password)

    BAD_ENV = {
        "hosted Supabase": "OMOBA_DATABASE_URL=postgres://omoba_dev_game:{p}@db.snrjxwutqxokeujuiepn.supabase.co:54322/omoba_dev\n",
        "Supabase pooler": "OMOBA_DATABASE_URL=postgres://omoba_dev_game:{p}@aws-0-eu.pooler.supabase.com:54322/omoba_dev\n",
        "production VPS": "OMOBA_DATABASE_URL=postgres://omoba_dev_game:{p}@77.246.105.57:54322/omoba_dev\n",
        "other host": "OMOBA_DATABASE_URL=postgres://omoba_dev_game:{p}@db.example.com:54322/omoba_dev\n",
        "production port": "OMOBA_DATABASE_URL=postgres://omoba_dev_game:{p}@127.0.0.1:2003/omoba_dev\n",
        "production role": "OMOBA_DATABASE_URL=postgres://omoba_game:{p}@127.0.0.1:54322/omoba_dev\n",
        "production database": "OMOBA_DATABASE_URL=postgres://omoba_dev_game:{p}@127.0.0.1:54322/omoba\n",
        "host override": "OMOBA_DATABASE_URL=postgres://omoba_dev_game:{p}@127.0.0.1:54322/omoba_dev?host=db.example.com\n",
        "short password": "OMOBA_DATABASE_URL=postgres://omoba_dev_game:{s}@127.0.0.1:54322/omoba_dev\n",
        "extra key": "OMOBA_DATABASE_URL=postgres://omoba_dev_game:{p}@127.0.0.1:54322/omoba_dev\n"
                     "OMOBA_PASSPORT_URL=https://avatar.ekza.io/api/passport\n",
        "twice": "OMOBA_DATABASE_URL=postgres://omoba_dev_game:{p}@127.0.0.1:54322/omoba_dev\n" * 2,
        "empty": "# nothing\n",
    }

    def test_a_production_or_foreign_env_file_is_refused_before_anything_switches(self):
        self.ship()
        payload, data = self.payload("0.44.0-dev-def5678", b"dev-2")
        self.fresh().stage(payload)
        self.upload(payload, data)
        good = self.env_file().read_text()
        for name, content in self.BAD_ENV.items():
            with self.subTest(env=name):
                self.env_file().write_text(content.format(p=GOOD_PASSWORD, s=SHORT_PASSWORD))
                os.chmod(self.env_file(), 0o600)
                self.assertFalse(self.fresh().preflight(payload)["ok"])
                calls = len(self.fake.calls)
                with self.assertRaises(lobby_dev.Refused) as raised:
                    self.fresh().activate(payload)
                self.assertNotIn(GOOD_PASSWORD, raised.exception.message)
                self.assertNotIn(["systemctl", "stop", "omoba-lobby-dev"], self.fake.calls[calls:])
                self.assertFalse(any(call[0] == "docker" and call[1] == "exec" for call in self.fake.calls[calls:]))
                self.assertEqual(self.current(), "releases/0.44.0-dev-abc1234")
                self.assertTrue(self.fake.units["omoba-lobby-dev"]["active"])
        self.env_file().write_text(good)
        os.chmod(self.env_file(), 0o644)
        with self.assertRaises(lobby_dev.Refused) as raised:
            self.fresh().activate(payload)
        self.assertIn("must be 0600", raised.exception.message)
        os.chmod(self.env_file(), 0o600)
        self.assertEqual(self.fresh().activate(payload)["status"], "DEPLOYED")

    def test_the_database_container_must_run_and_listen_on_loopback_only(self):
        payload, data = self.payload()
        for running, ports, expected in ((False, [], "is not running"),
                                         (True, ["0.0.0.0:54322"], "loopback only"),
                                         (True, ["127.0.0.1:54322", "[::]:54322"], "loopback only"),
                                         (True, ["127.0.0.1:5432"], "does not publish")):
            with self.subTest(running=running, ports=ports):
                self.fake.db_running, self.fake.db_ports = running, ports
                preflight = self.fresh().preflight(payload)
                self.assertFalse(preflight["ok"])
                self.assertTrue(any(expected in problem for problem in preflight["problems"]), preflight)
                self.fresh().stage(payload)
                self.upload(payload, data)
                with self.assertRaises(lobby_dev.Refused):
                    self.fresh().activate(payload)
                self.assertFalse(self.env_file().exists())
                self.assertFalse(self.fake.units["omoba-lobby-dev"]["active"])

    def test_a_database_owned_by_another_role_is_refused(self):
        self.fake.database_owner = "postgres"
        payload, data = self.payload()
        self.assertFalse(self.fresh().preflight(payload)["ok"])
        self.fresh().stage(payload)
        self.upload(payload, data)
        with self.assertRaises(lobby_dev.Refused):
            self.fresh().activate(payload)
        self.assertEqual(self.fake.migrations, [])

    def test_a_failed_migration_keeps_the_running_lobby_and_hides_the_password(self):
        self.ship()
        self.fake.fail_migrate = True
        payload, data = self.payload("0.44.0-dev-def5678", b"dev-2")
        self.fresh().stage(payload)
        self.upload(payload, data)
        calls = len(self.fake.calls)
        with self.assertRaises(SystemExit) as raised:
            self.fresh().activate(payload)
        self.assertIn("migrate failed", str(raised.exception))
        self.assertIn("<redacted>", str(raised.exception))
        self.assert_no_secret(str(raised.exception))
        self.assertNotIn(["systemctl", "stop", "omoba-lobby-dev"], self.fake.calls[calls:])
        self.assertEqual(self.current(), "releases/0.44.0-dev-abc1234")
        self.assertFalse(self.fake.roles["omoba_dev_owner"], "the temporary owner login stayed on")
        self.fake.fail_migrate, self.fake.fail_grants = False, True
        with self.assertRaises(SystemExit):
            self.fresh().activate(payload)
        self.assertFalse(self.fake.roles["omoba_dev_owner"])
        self.assertEqual(self.current(), "releases/0.44.0-dev-abc1234")

    def test_rollback_never_migrates_and_needs_the_env_file(self):
        self.ship()
        self.ship("0.44.0-dev-def5678", b"dev-2")
        migrations = len(self.fake.migrations)
        result = self.fresh().rollback({"release": "0.44.0-dev-abc1234", "expected_ip": DEV_IP})
        self.assertEqual((result["status"], len(self.fake.migrations)), ("DEPLOYED", migrations))
        self.env_file().unlink()
        with self.assertRaises(lobby_dev.Refused):
            self.fresh().rollback({"release": "0.44.0-dev-def5678", "expected_ip": DEV_IP})
        self.assertEqual(self.current(), "releases/0.44.0-dev-abc1234")

    def test_activation_needs_the_grants_and_the_tool(self):
        payload, data = self.payload()
        self.fresh().stage(payload)
        self.upload(payload, data)
        for change in ({"grants": None}, {"grants": "GRANT ALL ON ALL TABLES TO PUBLIC;"},
                       {"account_api_sha256": None}):
            with self.assertRaises(lobby_dev.Refused, msg=change):
                self.fresh().activate({**payload, **change})

    def test_status_reports_the_database_without_secrets(self):
        self.ship()
        database = self.fresh().status()["database"]
        self.assertEqual((database["env_file"], database["database_owner"], database["schema"], database["problems"]),
                         ("ok", "omoba_dev_owner", True, []))
        self.assert_no_secret(json.dumps(database))


class DatabaseUrl(unittest.TestCase):
    def test_only_the_loopback_dev_game_url_passes(self):
        self.assertEqual(lobby_dev.database_url_problems(GOOD_URL), [])
        self.assertEqual(lobby_dev.database_url_problems(GOOD_URL.replace("127.0.0.1", "localhost")), [])
        self.assertEqual(lobby_dev.database_url(lobby_dev.DB_GAME, GOOD_PASSWORD), GOOD_URL)
        for content in CareerDatabase.BAD_ENV.values():
            url = content.splitlines()[0].partition("=")[2] if "=" in content else ""
            problems = lobby_dev.parse_database_env(content.format(p=GOOD_PASSWORD, s=SHORT_PASSWORD))[1]
            self.assertTrue(problems, url)
            self.assertNotIn(GOOD_PASSWORD, json.dumps(problems))
        for bad in ("", "not a url", "mysql://omoba_dev_game:" + GOOD_PASSWORD + "@127.0.0.1:54322/omoba_dev",
                    GOOD_URL + "#x", GOOD_URL.replace(":54322", ":99999"),
                    GOOD_URL.replace(GOOD_PASSWORD, "a'b" * 10)):
            self.assertTrue(lobby_dev.database_url_problems(bad), bad)

    def test_the_written_file_parses_back(self):
        text = lobby_dev.database_env_text(GOOD_URL)
        self.assertEqual(lobby_dev.parse_database_env(text), (GOOD_URL, []))
        self.assertEqual(lobby_dev.parse_database_env(f'OMOBA_DATABASE_URL="{GOOD_URL}"\n'), (GOOD_URL, []))

    def test_redaction(self):
        text = f"ERROR at LINE 1: ALTER ROLE x LOGIN PASSWORD '{GOOD_PASSWORD}' via {GOOD_URL}"
        redacted = lobby_dev.redact(text)
        self.assertNotIn(GOOD_PASSWORD, redacted)
        self.assertIn("omoba_dev_game:<redacted>@127.0.0.1", redacted)
        self.assertEqual(lobby_dev.redact("token xyz", ("xyz",)), "token <redacted>")
        with self.assertRaises(lobby_dev.Refused):
            lobby_dev.sql_secret("x'; DROP ROLE omoba_dev_game; --" + "a" * 24)


def free_udp_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


@unittest.skipUnless(os.environ.get("OMOBA_LOBBY_SMOKE_SERVER"),
                     "set OMOBA_LOBBY_SMOKE_SERVER to a built server binary (cargo build -p server)")
class RealServerSmoke(unittest.TestCase):
    """The real server with the dev unit's environment and the env file this tool writes.

    Only what would leave this machine is redirected to loopback: bind and match
    addresses, state directories and the registry. The unit's role, match mode, team
    size, capacity and passport are used as they are.
    """

    def environment(self, directory: str, database_url: str | None) -> dict:
        unit_env = {key: values[-1] for key, values in lobby_dev.parse_unit(DEV_UNIT)[1].items()}
        first = free_udp_port()
        unit_env.update({
            "SERVER_ADDR": f"127.0.0.1:{free_udp_port()}", "OMOBA_MATCH_BIND_IP": "127.0.0.1",
            "OMOBA_MATCH_PUBLIC_HOST": "127.0.0.1", "OMOBA_MATCH_FIRST_PORT": str(first),
            "OMOBA_MATCH_ROOT": os.path.join(directory, "matches"),
            "OMOBA_CAREER_OUTBOX": os.path.join(directory, "lobby-outbox"),
            "OMOBA_MATCH_EXECUTABLE": os.environ["OMOBA_LOBBY_SMOKE_SERVER"],
            "OMOBA_REGISTRY_URL": "http://127.0.0.1:9",
        })
        for key in ("OMOBA_MATCH_ROOT", "OMOBA_CAREER_OUTBOX"):
            os.makedirs(unit_env[key])
        env = {"PATH": "/usr/bin:/bin", "HOME": directory, **unit_env}
        if database_url is not None:
            url, problems = lobby_dev.parse_database_env(lobby_dev.database_env_text(database_url))
            self.assertEqual(problems, [])
            env[lobby_dev.DATABASE_URL_KEY] = url
        return env

    def run_server(self, env, seconds, during=None):
        """(exit code or None if still running after `seconds`, output); `during` runs meanwhile."""
        with subprocess.Popen([os.environ["OMOBA_LOBBY_SMOKE_SERVER"]], env=env, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True) as process:
            seen = None
            try:
                if during:
                    time.sleep(seconds / 2)
                    seen = during()
                output, _ = process.communicate(timeout=seconds / 2 if during else seconds)
                code = process.returncode
            except subprocess.TimeoutExpired:
                process.terminate()
                output, _ = process.communicate(timeout=10)
                code = None
        return (code, output, seen) if during else (code, output)

    def test_the_dev_unit_environment_starts_the_lobby(self):
        url = os.environ.get("OMOBA_LOBBY_SMOKE_DATABASE_URL") or lobby_dev.database_url(
            lobby_dev.DB_GAME, "s" * 32)  # no database behind it: the lobby must still come up
        with tempfile.TemporaryDirectory() as directory:
            code, output = self.run_server(self.environment(directory, url), 8)
        self.assertIsNone(code, f"the dev lobby exited ({code}):\n{output}")
        self.assertIn("UDP game server is listening on 127.0.0.1:", output)
        self.assertIn("Match mode: release", output)

    def test_without_the_env_file_a_public_lobby_refuses_to_start(self):
        # Why the unit carries EnvironmentFile=/opt/omoba-dev/database.env.
        with tempfile.TemporaryDirectory() as directory:
            code, output = self.run_server(self.environment(directory, None), 20)
        self.assertNotIn(code, (None, 0), output)
        self.assertIn("Public roles require OMOBA_DATABASE_URL", output)


@unittest.skipUnless(os.environ.get("OMOBA_LOBBY_SMOKE_DB_CONTAINER") and os.environ.get("OMOBA_LOBBY_SMOKE_ACCOUNT_API"),
                     "set OMOBA_LOBBY_SMOKE_DB_CONTAINER / _DB_PORT / _ACCOUNT_API (see the module docstring)")
class RealDatabaseSmoke(unittest.TestCase):
    """prepare_database for real: Supabase Postgres image, account-api migrations, grants, env file."""

    RELEASE = "/opt/omoba-dev/releases/0.0.0-dev-0000000"

    def setUp(self):
        container = os.environ["OMOBA_LOBBY_SMOKE_DB_CONTAINER"]
        if not container.startswith("omoba-lobby-smoke-"):
            self.skipTest("the disposable container must be named omoba-lobby-smoke-* (this test writes to it)")
        for name, value in (("DB_CONTAINER", container),
                            ("DB_PORT", int(os.environ.get("OMOBA_LOBBY_SMOKE_DB_PORT", "59871")))):
            patcher = mock.patch.object(lobby_dev, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = temporary.name
        tool = Path(self.root, self.RELEASE.lstrip("/"), "omoba-account-api")
        tool.parent.mkdir(parents=True)
        tool.write_bytes(Path(os.environ["OMOBA_LOBBY_SMOKE_ACCOUNT_API"]).read_bytes())
        tool.chmod(0o755)

    def host(self):
        host = lobby_dev.Host(self.root, root_uid=os.getuid())
        host.mutating = True
        return host

    def test_prepare_twice_then_the_real_lobby_signs_in_as_the_game_role(self):
        self.assertEqual(self.host().db_container_problems(), [])
        first = self.host().prepare_database(self.RELEASE, GRANTS)
        self.assertTrue(first["env_written"])
        again = self.host().prepare_database(self.RELEASE, GRANTS)
        self.assertEqual((again["created_roles"], again["created_database"], again["env_written"]),
                         ([], False, False))
        state = self.host().db_state()
        self.assertEqual((state["roles"], state["database_owner"], state["schema"]),
                         ({"omoba_dev_owner": False, "omoba_dev_game": True, "omoba_dev_portal": False},
                          "omoba_dev_owner", True))
        env = self.host().database_env()
        self.assertEqual((env["exists"], env["problems"]), (True, []))
        self.assertEqual(stat.S_IMODE(os.stat(Path(self.root, "opt/omoba-dev/database.env")).st_mode), 0o600)
        if not os.environ.get("OMOBA_LOBBY_SMOKE_SERVER"):
            return
        smoke = RealServerSmoke()
        with tempfile.TemporaryDirectory() as directory:
            environment = smoke.environment(directory, None)
            environment[lobby_dev.DATABASE_URL_KEY] = env["url"]

            def signed_in():
                return self.host().psql("postgres", "-- lobby_dev:smoke\nSELECT count(*) FROM pg_stat_activity "
                                        "WHERE usename = 'omoba_dev_game' AND datname = 'omoba_dev';\n").stdout

            code, output, sessions = smoke.run_server(environment, 10, during=signed_in)
        self.assertIsNone(code, output)
        self.assertGreater(int(sessions.strip() or 0), 0, output)


class UnitChecks(unittest.TestCase):
    def test_the_repository_dev_unit_is_the_dev_lobby(self):
        self.assertEqual(lobby_dev.check_unit(DEV_UNIT), [])
        self.assertEqual(lobby_dev.unit_public_host(DEV_UNIT), "lobby.dev.omoba.io")

    def test_the_production_unit_is_refused(self):
        errors = " ".join(lobby_dev.check_unit(PROD_UNIT))
        for expected in ("User must be exactly 'omoba-dev'", "SERVER_ADDR", "EnvironmentFile",
                         "production path /opt/omoba/current/server", "belong to the production lobby",
                         f"production host {PROD_IP}"):
            self.assertIn(expected, errors)

    def test_every_drift_toward_production_is_refused(self):
        replacements = [
            ("SERVER_ADDR=0.0.0.0:4100", "SERVER_ADDR=0.0.0.0:4000"),
            ("OMOBA_MATCH_FIRST_PORT=41100", "OMOBA_MATCH_FIRST_PORT=41000"),
            ("OMOBA_MATCH_CAPACITY=1", "OMOBA_MATCH_CAPACITY=2"),
            ("User=omoba-dev", "User=omoba"),
            ("ReadWritePaths=/var/lib/omoba-dev", "ReadWritePaths=/var/lib/omoba"),
            ("ExecStart=/opt/omoba-dev/current/server", "ExecStart=/opt/omoba/current/server"),
            ("OMOBA_MATCH_ROOT=/var/lib/omoba-dev/matches", "OMOBA_MATCH_ROOT=/var/lib/omoba/matches"),
            ("OMOBA_REGISTRY_URL=https://registry.dev.ekza.io", "OMOBA_REGISTRY_URL=https://registry.ekza.io"),
            ("OMOBA_MATCH_PUBLIC_HOST=lobby.dev.omoba.io", f"OMOBA_MATCH_PUBLIC_HOST={PROD_IP}"),
            ("OMOBA_MATCH_PUBLIC_HOST=lobby.dev.omoba.io", "OMOBA_MATCH_PUBLIC_HOST=app.omoba.io"),
            ("OMOBA_MATCH_PUBLIC_HOST=lobby.dev.omoba.io", "OMOBA_MATCH_PUBLIC_HOST=lobby.omoba.io"),
            ("MemoryMax=512M", "MemoryMax=700M"),
            ("CPUQuota=100%", "CPUQuota=150%"),
            ("OOMScoreAdjust=300", "OOMScoreAdjust=0"),
            ("TasksMax=96", "TasksMax=128"),
            ("ProtectSystem=strict", "ProtectSystem=full"),
            ("NoNewPrivileges=true", "NoNewPrivileges=false"),
            ("[Service]\n", "[Service]\nEnvironmentFile=/opt/omoba/database.env\n"),
            ("EnvironmentFile=/opt/omoba-dev/database.env\n", "EnvironmentFile=/opt/omoba/database.env\n"),
            ("EnvironmentFile=/opt/omoba-dev/database.env\n", "EnvironmentFile=-/opt/omoba-dev/database.env\n"),
            ("EnvironmentFile=/opt/omoba-dev/database.env\n", "EnvironmentFile=/var/lib/omoba-dev/db.env\n"),
            ("EnvironmentFile=/opt/omoba-dev/database.env\n", ""),
            ("[Service]\n", "[Service]\nEnvironment=OMOBA_DATABASE_URL=postgres://x@127.0.0.1:2003/omoba\n"),
            ("[Service]\n", f"[Service]\nEnvironment=OMOBA_DATABASE_URL={GOOD_URL}\n"),
            ("OMOBA_PASSPORT_URL=http://127.0.0.1:9/api/passport",
             "OMOBA_PASSPORT_URL=https://avatar.ekza.io/api/passport"),
            ("Environment=OMOBA_PASSPORT_URL=http://127.0.0.1:9/api/passport\n", ""),
            ("[Service]\n", "[Service]\nEnvironment=EKZA_PASSPORT_URL=https://avatar.ekza.io/api/passport\n"),
            ("[Service]\n", "[Service]\nUser=omoba\n"),
            ("[Service]\n", f"[Service]\nEnvironment=EKZA_REF={lobby_dev.FORBIDDEN_TOKENS[0]}\n"),
            ("[Service]\n", "[Service]\nEnvironment=EKZA_SUPABASE_URL=https://x.supabase.co\n"),
        ]
        for old, new in replacements:
            self.assertIn(old, DEV_UNIT)
            self.assertNotEqual(lobby_dev.check_unit(DEV_UNIT.replace(old, new, 1)), [], new)

    def test_render_replaces_only_the_public_host(self):
        rendered = lobby_dev.render_unit(DEV_UNIT, DEV_IP)
        self.assertEqual(lobby_dev.check_unit(rendered), [])
        self.assertEqual(lobby_dev.unit_public_host(rendered), DEV_IP)
        self.assertEqual(rendered.replace(f"PUBLIC_HOST={DEV_IP}", "PUBLIC_HOST=lobby.dev.omoba.io"), DEV_UNIT)
        self.assertEqual(lobby_dev.render_unit(DEV_UNIT, None), DEV_UNIT)
        self.assertEqual(lobby_dev.unit_public_host(lobby_dev.render_unit(DEV_UNIT, "Lobby.Dev.Ekza.io.")),
                         "lobby.dev.ekza.io")
        for bad in (PROD_IP, "app.omoba.io", "omoba.io", "lobby.omoba.io", "dev.omoba.io", "evil.dev.omoba.io.x",
                    "127.0.0.1", "0.0.0.0", "a b.dev.omoba.io", "lobby.dev.omoba.io\nUser=root", ""):
            with self.assertRaises(lobby_dev.Refused, msg=bad):
                lobby_dev.render_unit(DEV_UNIT, bad)

    def test_production_names_are_every_public_name_outside_the_dev_zones(self):
        for name in ("avatar.ekza.io", "Avatar.Ekza.IO.", "registry.ekza.io", "omoba.io", "x.y.omoba.io",
                     "root@app.omoba.io", "vds-eternal", PROD_IP):
            self.assertTrue(lobby_dev.is_production_name(name), name)
        for name in ("vds-dev", "lobby.dev.omoba.io", "registry.dev.ekza.io", "dev.ekza.io", DEV_IP,
                     "notomoba.io", "127.0.0.1", "omoba.io.example"):
            self.assertFalse(lobby_dev.is_production_name(name), name)

    def test_paths_compare_by_component(self):
        self.assertFalse(lobby_dev.within("/opt/omoba-dev", "/opt/omoba"))
        self.assertTrue(lobby_dev.within("/opt/omoba/current", "/opt/omoba"))
        self.assertTrue(lobby_dev.within("/opt/omoba-dev/../omoba/x", "/opt/omoba"))
        self.assertFalse(lobby_dev.touches_production("/var/lib/omoba-dev/matches"))
        self.assertTrue(lobby_dev.touches_production("/var/lib/omoba/matches"))
        self.assertTrue(lobby_dev.touches_production("/etc/systemd/system/omoba-lobby.service"))
        self.assertFalse(lobby_dev.touches_production(lobby_dev.UNIT_PATH))
        lobby_dev.assert_dev_constants()

    def test_editing_a_dev_constant_into_production_is_caught(self):
        for name, value in (("OPT", "/opt/omoba"), ("STATE_DIRS", ("/var/lib/omoba",)), ("USER", "omoba"),
                            ("UNIT", "omoba-lobby"), ("LOBBY_PORT", 4000), ("MATCH_PORTS", frozenset({41001})),
                            ("DEFAULT_HOST", "vds-eternal"), ("DEFAULT_PUBLIC_HOST", PROD_IP),
                            ("DATABASE_ENV", "/opt/omoba/database.env"), ("DB_NAME", "omoba"),
                            ("DB_ROLES", ("omoba_game",)), ("DB_PORT", 2003),
                            ("PASSPORT", "https://avatar.ekza.io/api/passport")):
            with mock.patch.object(lobby_dev, name, value), self.assertRaises(lobby_dev.Refused, msg=name):
                lobby_dev.assert_dev_constants()

    def test_payloads_are_shell_safe(self):
        payload = {"release": "0.44.0-dev-abc1234", "unit": DEV_UNIT, "sha256": "0" * 64}
        encoded = lobby_dev.encode_payload(payload)
        self.assertRegex(encoded, r"^[A-Za-z0-9+/=]+$")
        self.assertEqual(lobby_dev.decode_payload(encoded), payload)
        with self.assertRaises(lobby_dev.Refused):
            lobby_dev.decode_payload("e30=; rm -rf /")


def write_package(directory, name, member, data, symlink=False, extra=None):
    """A tarball with `member` plus, by default, the dev migration tool next to it."""
    package = Path(directory, name)
    root = member.split("/", 1)[0]
    members = {member: (data, symlink)}
    for path, content in ({f"{root}/omoba-account-api": TOOL} if extra is None else extra).items():
        members[path] = (content, False)
    with tarfile.open(package, "w:gz") as archive:
        for path, (content, link) in members.items():
            info = tarfile.TarInfo(path)
            if link:
                info.type, info.linkname = tarfile.SYMTYPE, "/opt/omoba/current/server"
                archive.addfile(info)
            else:
                info.size = len(content)
                archive.addfile(info, io.BytesIO(content))
    return package


def ssh_g(hostname, user="root"):
    """A fake `ssh -G` reader returning what ~/.ssh/config would say."""
    return lambda alias: {"hostname": hostname, "user": user, "port": "22"}


def target_args(host=None, ip=None):
    return mock.Mock(host=host, ip=ip)


class TargetGuard(unittest.TestCase):
    def setUp(self):
        patcher = mock.patch.dict(os.environ, {}, clear=False)
        patcher.start()
        self.addCleanup(patcher.stop)
        for key in (lobby_dev.HOST_ENV, lobby_dev.IP_ENV, lobby_dev.PUBLIC_HOST_ENV):
            os.environ.pop(key, None)

    def test_defaults_to_vds_dev_and_reads_the_environment(self):
        target = lobby_dev.resolve_target(target_args(), need_ip=False, config_reader=ssh_g("vds-dev"))
        self.assertEqual((target["alias"], target["expected_ip"]), ("vds-dev", None))
        os.environ.update({lobby_dev.HOST_ENV: "dev-box", lobby_dev.IP_ENV: DEV_IP})
        target = lobby_dev.resolve_target(target_args(), need_ip=True, config_reader=ssh_g(DEV_IP))
        self.assertEqual((target["alias"], target["expected_ip"], target["hostname"]), ("dev-box", DEV_IP, DEV_IP))

    def test_production_aliases_and_addresses_are_refused(self):
        for alias in ("vds-eternal", "VDS-Eternal", "root@vds-eternal", PROD_IP, f"root@{PROD_IP}",
                      "registry.ekza.io", "omoba.io"):
            with self.assertRaises(lobby_dev.Refused, msg=alias):
                lobby_dev.resolve_target(target_args(alias), need_ip=False, config_reader=ssh_g(DEV_IP))

    def test_option_like_or_odd_aliases_are_refused(self):
        for alias in ("-oProxyCommand=sh", "vds dev", "vds-dev;id", "", "a/b"):
            with self.assertRaises(lobby_dev.Refused, msg=alias):
                lobby_dev.resolve_target(target_args(alias or "\x00"), need_ip=False, config_reader=ssh_g(DEV_IP))

    def test_an_alias_that_connects_to_production_is_refused(self):
        for hostname in (PROD_IP, "vds-eternal"):
            with self.assertRaises(lobby_dev.Refused, msg=hostname):
                lobby_dev.resolve_target(target_args("vds-dev", DEV_IP), need_ip=False,
                                         config_reader=ssh_g(hostname))

    def test_check_and_apply_need_the_dev_ip_and_a_matching_hostname(self):
        with self.assertRaises(lobby_dev.Refused):
            lobby_dev.resolve_target(target_args("vds-dev"), need_ip=True, config_reader=ssh_g(DEV_IP))
        for hostname in ("dev.example.org", "198.51.100.7", None):
            with self.assertRaises(lobby_dev.Refused, msg=hostname):
                lobby_dev.resolve_target(target_args("vds-dev", DEV_IP), need_ip=True,
                                         config_reader=ssh_g(hostname))
        with self.assertRaises(lobby_dev.Refused):
            lobby_dev.resolve_target(target_args("vds-dev", DEV_IP), need_ip=True, config_reader=lambda alias: None)

    def test_the_dev_ip_itself_is_validated(self):
        for bad in (PROD_IP, "127.0.0.1", "0.0.0.0", "not-an-ip", "2001:db8::1", "224.0.0.1"):
            with self.assertRaises(lobby_dev.Refused, msg=bad):
                lobby_dev.validate_dev_ip(bad)
        self.assertEqual(lobby_dev.validate_dev_ip(f" {DEV_IP} "), DEV_IP)
        self.assertIsNone(lobby_dev.validate_dev_ip(""))


class MacHalf(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.package = write_package(self.directory.name, "omoba-0.44.0-dev-linux-x64.tar.gz",
                                     "omoba-0.44.0-dev/omoba-server", elf())
        patcher = mock.patch.dict(os.environ, {}, clear=False)
        patcher.start()
        self.addCleanup(patcher.stop)
        for key in (lobby_dev.HOST_ENV, lobby_dev.IP_ENV, lobby_dev.PUBLIC_HOST_ENV):
            os.environ.pop(key, None)

    def test_only_a_dev_linux_package_with_an_x86_64_server_is_accepted(self):
        version, binaries, _ = lobby_dev.read_package(self.package)
        self.assertEqual((version, binaries), ("0.44.0", {"server": elf(), "omoba-account-api": TOOL}))
        for extra in ({}, {"omoba-0.44.0-dev/omoba-account-api": b"#!/bin/sh\n"}):
            sub = Path(self.directory.name, f"tool-{len(extra)}")
            sub.mkdir()
            package = write_package(sub, "omoba-0.44.0-dev-linux-x64.tar.gz", "omoba-0.44.0-dev/omoba-server",
                                    elf(), extra=extra)
            with self.assertRaises(lobby_dev.Refused, msg=extra):
                lobby_dev.read_package(package)
        cases = [
            ("omoba-0.44.0-linux-x64.tar.gz", "omoba-0.44.0/omoba-server", elf(), False),
            ("omoba-0.44.0-dev-linux-x64.tar.gz", "omoba-0.44.0-dev/omoba-server", elf(b"\xb7\x00"), False),
            ("omoba-0.44.0-dev-linux-x64.tar.gz", "omoba-0.44.0-dev/omoba-server", b"#!/bin/sh\n", False),
            ("omoba-0.44.0-dev-linux-x64.tar.gz", "omoba-0.44.0-dev/omoba-server", b"", True),
            ("omoba-0.44.0-dev-linux-x64.tar.gz", "omoba-0.44.0-dev/omoba", elf(), False),
        ]
        for index, (name, member, payload, symlink) in enumerate(cases):
            sub = Path(self.directory.name, str(index))
            sub.mkdir()
            package = write_package(sub, name, member, payload, symlink)
            with self.assertRaises(lobby_dev.Refused, msg=(name, member)):
                lobby_dev.read_package(package)

    def test_dry_runs_contact_nothing(self):
        def local_only(args, *rest, **kwargs):
            if args[:2] == ["ssh", "-G"]:
                return subprocess.CompletedProcess(args, 0, f"hostname {DEV_IP}\nuser root\n", "")
            if args and args[0] in ("ssh", "scp"):
                raise AssertionError(f"network in a dry run: {args}")
            return subprocess.CompletedProcess(args, 0, "", "")

        with mock.patch.object(lobby_dev.subprocess, "run", local_only), \
                contextlib.redirect_stdout(io.StringIO()) as output, \
                contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(lobby_dev.main(["ship", "--package", str(self.package), "--revision", "abc1234def"]), 0)
            self.assertEqual(lobby_dev.main(["rollback", "--release", "0.44.0-dev-abc1234"]), 0)
        plan = json.loads(output.getvalue().split("\n}\n")[0] + "\n}")
        self.assertTrue(plan["dry_run"])
        self.assertEqual((plan["release"], plan["public_host"]), ("0.44.0-dev-abc1234", "lobby.dev.omoba.io"))
        self.assertEqual((plan["target"]["alias"], plan["target"]["hostname"]), ("vds-dev", DEV_IP))
        self.assertEqual(plan["build"]["contour"], "dev")

    def test_public_host_comes_from_the_flag_or_the_environment(self):
        args = mock.Mock(unit=None, public_host=None, package=str(self.package), binary=None,
                         revision="abc1234", release=None)
        os.environ[lobby_dev.PUBLIC_HOST_ENV] = DEV_IP
        self.assertEqual(lobby_dev.prepare(args)["public_host"], DEV_IP)
        args.public_host = "lobby.dev.ekza.io"
        self.assertEqual(lobby_dev.prepare(args)["public_host"], "lobby.dev.ekza.io")
        args.public_host = PROD_IP
        with self.assertRaises(lobby_dev.Refused):
            lobby_dev.prepare(args)

    def test_refusals_happen_before_any_network(self):
        cases = [
            (["--host", "vds-eternal", "--ip", DEV_IP, "--apply"], {}),
            (["--ip", PROD_IP, "--apply"], {}),
            (["--check"], {}),                                    # no DEV_IP
            (["--apply"], {lobby_dev.IP_ENV: DEV_IP, lobby_dev.HOST_ENV: "vds-eternal"}),
            (["--ip", DEV_IP, "--apply", "--unit", str(ROOT / "ops/vps/omoba-lobby.service")], {}),
            (["--ip", DEV_IP, "--apply", "--public-host", PROD_IP], {}),
        ]
        for extra, env in cases:
            with self.subTest(extra=extra, env=env), mock.patch.dict(os.environ, env), \
                    mock.patch.object(lobby_dev, "ssh_config", return_value={"hostname": DEV_IP}), \
                    mock.patch.object(lobby_dev, "ssh_remote", side_effect=AssertionError("ssh")), \
                    mock.patch.object(lobby_dev.subprocess, "run", side_effect=AssertionError("ran")):
                with self.assertRaises(lobby_dev.Refused):
                    lobby_dev.main(["ship", "--package", str(self.package), "--revision", "abc1234", *extra])

    def apply_with(self, uploads):
        calls = []
        directory = "/opt/omoba-dev/releases/0.44.0-dev-abc1234"

        def remote(alias, mode, payload=None):
            calls.append((alias, mode, payload))
            if mode == "stage":
                return json.dumps({"mode": mode, "upload_to": [f"{directory}/{name}" for name in uploads]})
            return json.dumps({"ok": True, "mode": mode})

        def run(args, *rest, **kwargs):
            if args[:2] == ["ssh", "-G"]:
                return subprocess.CompletedProcess(args, 0, f"hostname {DEV_IP}\n", "")
            calls.append(("scp", args[-1], Path(args[-2]).read_bytes()))
            return subprocess.CompletedProcess(args, 0, "", "")

        with mock.patch.object(lobby_dev, "ssh_remote", remote), mock.patch.object(lobby_dev.subprocess, "run", run), \
                contextlib.redirect_stdout(io.StringIO()):
            lobby_dev.main(["ship", "--package", str(self.package), "--revision", "abc1234", "--ip", DEV_IP, "--apply"])
        return calls

    def test_apply_runs_preflight_stage_upload_activate_in_order(self):
        calls = self.apply_with(["server.part", "omoba-account-api.part"])
        directory = "vds-dev:/opt/omoba-dev/releases/0.44.0-dev-abc1234"
        self.assertEqual([call[1] for call in calls], ["preflight", "stage", f"{directory}/server.part",
                                                       f"{directory}/omoba-account-api.part", "activate"])
        self.assertEqual((calls[2][2], calls[3][2]), (elf(), TOOL))
        self.assertTrue(all(call[0] in ("vds-dev", "scp") for call in calls))
        activate = calls[4][2]
        self.assertEqual(activate["expected_ip"], DEV_IP)
        self.assertEqual(lobby_dev.check_unit(activate["unit"]), [])
        self.assertEqual(activate["grants"], GRANTS)
        self.assertEqual(activate["account_api_sha256"], hashlib.sha256(TOOL).hexdigest())
        self.assertEqual(activate["build"]["account_api_sha256"], hashlib.sha256(TOOL).hexdigest())
        # Already staged files are not uploaded again.
        self.assertEqual([call[1] for call in self.apply_with([])], ["preflight", "stage", "activate"])

    def test_apply_refuses_an_upload_outside_the_release(self):
        with self.assertRaises(lobby_dev.Refused):
            self.apply_with(["../../omoba/current/server.part"])
        with self.assertRaises(lobby_dev.Refused):
            self.apply_with(["client.part"])

    def test_apply_stops_when_the_host_preflight_reports_problems(self):
        modes = []

        def remote(alias, mode, payload=None):
            modes.append(mode)
            return json.dumps({"ok": False, "problems": ["lobby.dev.omoba.io has no A record"]})

        with mock.patch.object(lobby_dev, "ssh_remote", remote), \
                mock.patch.object(lobby_dev, "ssh_config", return_value={"hostname": DEV_IP}), \
                mock.patch.object(lobby_dev.subprocess, "run", side_effect=AssertionError("scp")), \
                contextlib.redirect_stdout(io.StringIO()):
            with self.assertRaises(lobby_dev.Refused):
                lobby_dev.main(["ship", "--package", str(self.package), "--revision", "abc1234",
                                "--ip", DEV_IP, "--apply"])
        self.assertEqual(modes, ["preflight"])


if __name__ == "__main__":
    unittest.main()
