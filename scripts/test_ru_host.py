"""ops/vps/ru/ru_host.py: the RU host tool only ever targets the RU VPS (LR-01).

The host half runs against a temporary filesystem root with fake apt / dpkg-query /
systemctl / ufw / sshd / caddy / runuser psql / pg_dump / useradd / install / ss / ip /
pgrep / node / omoba-account-api, fake DNS and fake loopback HTTP, so nothing here needs
ssh, root, PostgreSQL, Caddy or systemd. The Mac half is exercised with ssh, scp and
whois mocked. A bootstrap → ship → ship-portal → status → reboot round runs end to end
against the fakes.
"""
from __future__ import annotations

import contextlib
import dataclasses
import hashlib
import io
import json
import os
import re
import socket
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "ops/vps/ru"))
sys.path.insert(0, str(ROOT / "scripts"))

import markets  # noqa: E402
import ru_host  # noqa: E402

RU_IP = "95.163.10.20"
DEV_IP = "203.0.113.10"
PROD_IP = "77.246.105.57"
VK_ID = "51234567"
GRANTS = (ROOT / ru_host.GRANTS).read_text()
KIT = ru_host.kit()
NAMES = KIT["names"]   # the RU profile of scripts/markets.py
RU_PORTAL = markets.MARKETS["ru"].portal
PASSWORD_IN_SQL = re.compile(r"ALTER ROLE (\w+) LOGIN PASSWORD '([^']+)'")
HAS_DATA_FILTER = hasattr(tarfile, "data_filter")
HEX64 = re.compile(r"[0-9a-f]{64}")


def elf(body: bytes = b"server", machine: bytes = b"\x3e\x00") -> bytes:
    return b"\x7fELF" + bytes([2, 1, 1, 0]) + bytes(8) + b"\x02\x00" + machine + bytes(44) + body


SERVER = elf(b"omoba-server ru 1")
ACCOUNT_API = elf(b"omoba-account-api ru 1")
SERVER_SERVICES = ("omoba-lobby-ru", "omoba-account-api-ru")


def ufw_line(to: str, action: str, source: str, comment: str) -> str:
    return f"{to:<27}{action:<12}{source:<27}" + (f"# {comment}" if comment else "")


class FakeSystem:
    """Just enough of Ubuntu (apt, systemd, ufw, OpenSSH, Caddy, PostgreSQL) for the host half."""

    def __init__(self, root: str):
        self.root = root
        self.calls = []
        self.users = {}
        self.packages = {"openssh-server"}
        self.node_candidate = "18.19.1+dfsg-6ubuntu5"
        self.arch = "x86_64"
        self.ipv4 = {"127.0.0.1", RU_IP}
        self.units = {}
        self.enabled = set()
        self.udp, self.tcp = set(), set()
        self.extra_workers = []
        self.next_pid = 7000
        self.fail_start = set()
        self.fail_sshd_test = False
        self.fail_caddy_validate = False
        self.fail_migrate = False
        self.ufw = {"active": False, "incoming": "deny", "rules": []}
        self.dns = {NAMES[key]: {socket.AF_INET: {RU_IP}, socket.AF_INET6: set()} for key in ru_host.HOST_KEYS}
        # PostgreSQL after `apt-get install postgresql` (Ubuntu 24.04: 16 with ICU, localhost).
        self.pg = {"version": 160004, "icu": True, "listen": "localhost", "port": 5432}
        self.roles, self.passwords = {}, {}
        self.database_owner = None
        self.schema = False
        self.sql, self.migrations, self.dumps = [], [], []
        self.grants = 0
        self.write("/etc/os-release", 'ID=ubuntu\nVERSION_ID="24.04"\n')
        self.write("/proc/meminfo", "MemTotal: 4014000 kB\nMemAvailable: 2500000 kB\n")
        self.write("/proc/sys/kernel/random/boot_id", "boot-1\n")
        self.write("/root/.ssh/authorized_keys", "ssh-ed25519 AAAA owner@mac\n")
        os.makedirs(self.p("/etc/systemd/system"))
        os.makedirs(self.p("/etc/ssh/sshd_config.d"))

    def p(self, path):
        return os.path.join(self.root, path.lstrip("/"))

    def write(self, path, text):
        os.makedirs(os.path.dirname(self.p(path)), exist_ok=True)
        Path(self.p(path)).write_text(text)

    def unit(self, name):
        return self.units.setdefault(name, {"active": False, "MainPID": "0", "NRestarts": "0"})

    def resolve(self, host, family):
        return set(self.dns.get(host, {}).get(family, set()))

    def fetch(self, url):
        if url == "http://127.0.0.1:40550/health/ready":
            return (200 if self.schema else 503) if self.unit("omoba-account-api-ru")["active"] else 0
        if url == "http://127.0.0.1:3010/":
            return 200 if self.unit("omoba-portal-ru")["active"] else 0
        raise AssertionError(f"unexpected fetch {url}")

    def __call__(self, args, check=True, input=None, env=None, timeout=None):
        self.calls.append(list(args))
        code, out, *err = self.dispatch(list(args), input, env)
        if check and code:
            raise SystemExit(f"{' '.join(args)} failed")
        return subprocess.CompletedProcess(args, code, out, err[0] if err else "")

    # ---- commands

    def dispatch(self, args, stdin, env):
        name = args[0]
        if name == "systemctl":
            return self.systemctl(args[1:])
        if name == "dpkg-query":
            return (0, "install ok installed") if args[-1] in self.packages else (1, "", "not installed")
        if name == "apt-get":
            if args[1] == "install":
                assert env and env.get("DEBIAN_FRONTEND") == "noninteractive", env
                for package in args[args.index("--no-install-recommends") + 1:]:
                    self.install_package(package)
            return 0, ""
        if name == "apt-cache":
            return 0, f"nodejs:\n  Installed: (none)\n  Candidate: {self.node_candidate}\n"
        if name == "uname":
            return 0, self.arch + "\n"
        if name == "ip":
            return 0, "".join(f"2: eth0    inet {ip}/24 brd x scope global eth0\n" for ip in sorted(self.ipv4))
        if name == "ss":
            sockets = self.udp if args[1] == "-Hlun" else self.tcp
            state = "UNCONN" if args[1] == "-Hlun" else "LISTEN"
            return 0, "".join(f"{state} 0 0 {address}:{port} 0.0.0.0:*\n" for address, port in sorted(sockets))
        if name == "getent":
            user = self.users.get(args[2])
            return (0, f"{args[2]}:x:{user['uid']}:{user['uid']}::{user['home']}:{user['shell']}\n") if user else (2, "")
        if name == "id":
            return (0, f"{os.getuid()}\n") if args[2] in self.users else (1, "")
        if name == "useradd":
            self.users[args[-1]] = {"uid": 990 + len(self.users), "home": args[args.index("--home-dir") + 1],
                                    "shell": args[args.index("--shell") + 1]}
            return 0, ""
        if name == "install":
            assert args[1] == "-d", args
            os.makedirs(args[-1], exist_ok=True)
            os.chmod(args[-1], int(args[args.index("-m") + 1], 8))
            return 0, ""
        if name == "chmod":
            os.chmod(args[2], int(args[1], 8))
            return 0, ""
        if name == "chown":
            return 0, ""
        if name == "ufw":
            return self.ufw_command(args[1:])
        if name == "sshd":
            return self.sshd(args[1:])
        if name == "caddy":
            if args[1] == "version":
                return 0, "v2.6.2 h1:abc=\n"
            assert args[1:4] == ["validate", "--adapter", "caddyfile"], args
            return (1, "", "Error: adapting config") if self.fail_caddy_validate else (0, "Valid configuration\n")
        if name == "runuser":
            assert args[1:4] == ["-u", "postgres", "--"], args
            if args[4] == "psql":
                return self.psql(args[4:], stdin)
            if args[4] == "pg_dump":
                assert "omoba_ru" in args and "-Fc" in args, args
                Path(args[args.index("-f") + 1]).write_bytes(b"PGDMP")
                self.dumps.append(args[args.index("-f") + 1])
                return 0, ""
        if name == "pgrep":
            assert args[1:] == ["-u", "omoba-ru", "-x", "omoba-server"], args
            lobby = self.unit("omoba-lobby-ru")
            pids = ([lobby["MainPID"]] if lobby["active"] else []) + [str(pid) for pid in self.extra_workers]
            return (0, "\n".join(pids) + "\n") if pids else (1, "")
        if name == "systemd-analyze":
            return 0, ""
        if name.endswith("/bin/node"):
            found = re.search(rb"v\d+\.\d+\.\d+", Path(name).read_bytes())
            return 0, found.group().decode() + "\n"
        if name.endswith("/omoba-account-api"):
            return self.migrate(args, env)
        raise AssertionError(f"unexpected command {args}")

    def install_package(self, package):
        self.packages.add(package)
        if package == "postgresql":
            self.enabled.add("postgresql")
            self.unit("postgresql")["active"] = True
            self.tcp.add(("127.0.0.1", 5432))
        if package == "caddy":
            self.enabled.add("caddy")
            self.unit("caddy")["active"] = True
            self.tcp.add(("*", 443))
            self.write("/etc/caddy/Caddyfile", ":80 {\n\troot * /usr/share/caddy\n\tfile_server\n}\n")

    def systemctl(self, args):
        verb = args[0]
        if verb == "show":
            unit = args[1]
            if args[3] == "LoadState":
                return 0, ("loaded" if unit in self.units else "not-found") + "\n"
            return 0, str(self.unit(unit).get(args[3], "")) + "\n"
        if verb == "is-active":
            return (0 if self.units.get(args[2], {}).get("active") else 3), ""
        if verb == "is-enabled":
            return (0, "enabled\n") if args[1] in self.enabled else (1, "disabled\n")
        if verb == "daemon-reload":
            return 0, ""
        unit = args[1]
        assert unit in ("caddy", "postgresql", "ssh.service", *SERVER_SERVICES, "omoba-portal-ru"), args
        if verb == "enable":
            self.enabled.add(unit)
        elif verb == "disable":
            self.enabled.discard(unit)
        elif verb == "stop":
            self.stop(unit)
        elif verb in ("start", "restart"):
            self.stop(unit)
            return self.start(unit)
        elif verb not in ("reload", "try-reload-or-restart"):
            raise AssertionError(f"unexpected systemctl {args}")
        return 0, ""

    def stop(self, unit):
        state = self.unit(unit)
        state.update(active=False, MainPID="0")
        if unit == "omoba-lobby-ru":
            self.udp.discard(("0.0.0.0", 4000))
        self.tcp.discard({"omoba-account-api-ru": ("127.0.0.1", 40550),
                          "omoba-portal-ru": ("127.0.0.1", 3010)}.get(unit))

    def start(self, unit):
        state = self.unit(unit)
        if unit in self.fail_start:
            return 1, "", "start failed"
        if unit == "omoba-lobby-ru":
            binary = self.p("/opt/omoba-ru/current/omoba-server")
            if not os.path.isfile(binary):
                return 1, "", "exec failed"
            pid = self.next_pid
            self.next_pid += 1
            os.makedirs(self.p(f"/proc/{pid}"), exist_ok=True)
            os.symlink(os.path.realpath(binary), self.p(f"/proc/{pid}/exe"))
            state["MainPID"] = str(pid)
            self.udp.add(("0.0.0.0", 4000))
        elif unit == "omoba-account-api-ru":
            if not os.path.isfile(self.p("/opt/omoba-ru/current/omoba-account-api")):
                return 1, "", "exec failed"
            self.tcp.add(("127.0.0.1", 40550))
        elif unit == "omoba-portal-ru":
            if not os.path.isfile(self.p("/opt/omoba-ru/portal/current/server.js")):
                return 1, "", "exec failed"
            self.tcp.add(("127.0.0.1", 3010))
        state["active"] = True
        state["NRestarts"] = "0"
        return 0, ""

    def boot(self):
        """A reboot: everything stops, then systemd starts what is enabled, in dependency order."""
        for unit in list(self.units):
            self.stop(unit)
        Path(self.p("/proc/sys/kernel/random/boot_id")).write_text("boot-2\n")
        for unit in ("postgresql", "caddy", "omoba-lobby-ru", "omoba-account-api-ru", "omoba-portal-ru"):
            if unit in self.enabled:
                self.start(unit)

    def ufw_command(self, args):
        if args == ["status", "verbose"]:
            if not self.ufw["active"]:
                return 0, "Status: inactive\n"
            lines = ["Status: active", "Logging: on (low)",
                     f"Default: {self.ufw['incoming']} (incoming), allow (outgoing), disabled (routed)",
                     "New profiles: skip", "", ufw_line("To", "Action", "From", ""), ufw_line("--", "------", "----", "")]
            lines += [ufw_line(to, f"{action.upper()} IN", "Anywhere", comment) for action, to, comment in self.ufw["rules"]]
            lines += [ufw_line(f"{to} (v6)", f"{action.upper()} IN", "Anywhere (v6)", comment)
                      for action, to, comment in self.ufw["rules"]]
            return 0, "\n".join(lines) + "\n"
        if args[0] == "default":
            if args[2] == "incoming":
                self.ufw["incoming"] = args[1]
            return 0, ""
        if args[0] in ("allow", "limit", "deny"):
            assert args[2] == "comment", args
            if not any(rule[1] == args[1] for rule in self.ufw["rules"]):
                self.ufw["rules"].append((args[0], args[1], args[3]))
            return 0, ""
        if args[:2] == ["--force", "delete"]:
            self.ufw["rules"] = [rule for rule in self.ufw["rules"] if rule[1] != args[3]]
            return 0, ""
        if args == ["--force", "enable"]:
            self.ufw["active"] = True
            return 0, ""
        raise AssertionError(f"unexpected ufw {args}")

    def sshd(self, args):
        if args == ["-t"]:
            return (255, "", "line 3: Bad configuration option") if self.fail_sshd_test else (0, "")
        assert args == ["-T"], args
        keys_only = os.path.isfile(self.p("/etc/ssh/sshd_config.d/00-omoba-ru.conf"))
        values = {"port": "22", "pubkeyauthentication": "yes",
                  "passwordauthentication": "no" if keys_only else "yes",
                  "kbdinteractiveauthentication": "no",
                  "permitemptypasswords": "no",
                  "permitrootlogin": "without-password" if keys_only else "yes",
                  "authenticationmethods": "publickey" if keys_only else "any"}
        return 0, "".join(f"{key} {value}\n" for key, value in values.items())

    def psql(self, args, sql):
        database = args[args.index("-d") + 1]
        assert "ON_ERROR_STOP=1" in args and database in ("postgres", "omoba_ru"), args
        tag = sql.splitlines()[0]
        self.sql.append((database, tag))
        if "postgresql" not in self.packages:
            return 2, "", "psql: not found"
        if tag == "-- ru_host:state":
            return 0, json.dumps({**self.pg, "roles": self.roles, "database_owner": self.database_owner}) + "\n"
        if tag == "-- ru_host:schema":
            return 0, "t\n" if self.schema else "f\n"
        if tag == "-- ru_host:bootstrap":
            assert "LOCALE ''C.UTF-8''" in sql and "TEMPLATE template0" in sql
            for role in ru_host.DB_ROLES:
                self.roles.setdefault(role, False)
            self.database_owner = self.database_owner or "omoba_owner"
            for role, password in PASSWORD_IN_SQL.findall(sql):
                self.roles[role] = True
                self.passwords[role] = password
            return 0, ""
        if tag == "-- ru_host:owner-login":
            assert "VALID UNTIL" in sql
            role, password = PASSWORD_IN_SQL.search(sql).groups()
            self.roles[role] = True
            self.passwords[role] = password
            return 0, ""
        if tag == "-- ru_host:grants":
            assert database == "omoba_ru" and "SET ROLE omoba_owner;" in sql and ':"game_role"' in sql
            assert "\\set game_role omoba_game" in sql and "\\set portal_role omoba_portal" in sql
            self.grants += 1
            return 0, ""
        if tag == "-- ru_host:owner-nologin":
            self.roles["omoba_owner"] = False
            self.passwords.pop("omoba_owner", None)
            return 0, ""
        raise AssertionError(f"unexpected SQL batch {tag}")

    def migrate(self, args, env):
        assert args[1:] == ["migrate"], args
        url = urlsplit(env["OMOBA_DATABASE_URL"])
        assert (url.username, url.hostname, url.port, url.path) == ("omoba_owner", "127.0.0.1", 5432, "/omoba_ru")
        assert self.roles.get("omoba_owner") and url.password == self.passwords["omoba_owner"]
        self.migrations.append(env["OMOBA_DATABASE_URL"])
        if self.fail_migrate:
            return 1, "", f"migration failed for {env['OMOBA_DATABASE_URL']}"
        self.schema = True
        return 0, "Career and Account API migrations ready.\n"


def tree(root: str) -> dict:
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


def node_tarball(directory: str, version: str = "22.11.0") -> Path:
    path = Path(directory) / f"node-v{version}-linux-x64.tar.xz"
    top = f"node-v{version}-linux-x64"
    with tarfile.open(path, "w:xz") as archive:
        def add(name, data=None, link=None, mode=0o644):
            info = tarfile.TarInfo(f"{top}/{name}")
            if link:
                info.type, info.linkname = tarfile.SYMTYPE, link
                archive.addfile(info)
            else:
                info.size, info.mode = len(data), mode
                archive.addfile(info, io.BytesIO(data))
        add("bin/node", elf(f"node v{version}".encode()), mode=0o755)
        add("lib/node_modules/npm/bin/npm-cli.js", b"// npm\n")
        add("bin/npm", link="../lib/node_modules/npm/bin/npm-cli.js")
    return path


def portal_tree(directory: Path, market: str = "ru (ekza off, supporter off, vk_sign_in on)",
                contour: str = "production", extra=None) -> Path:
    env = json.dumps({"OMOBA_BUILT_CONTOUR": contour, "OMOBA_BUILT_MARKET": market}, separators=(",", ":"))
    files = {"server.js": f"const nextConfig = {{\"env\":{env}}};\nrequire('next')\n",
             "package.json": "{}\n", ".next/BUILD_ID": "build-1\n", ".next/static/chunks/app.js": "x\n",
             "node_modules/next/package.json": "{}\n", "public/brand/logo.svg": "<svg/>\n", **(extra or {})}
    for name, text in files.items():
        (directory / name).parent.mkdir(parents=True, exist_ok=True)
        (directory / name).write_bytes(text if isinstance(text, bytes) else text.encode())
    return directory


def portal_bundle(directory: str, revision: str = "abc1234", **kwargs) -> Path:
    name = f"omoba-portal-ru-{revision}"
    stage = Path(directory) / "stage" / name
    stage.mkdir(parents=True)
    portal_tree(stage, **kwargs)
    (stage / "server.js").chmod(0o600)   # as from a Mac with umask 077
    bundle = Path(directory) / f"{name}.tar.gz"
    with tarfile.open(bundle, "w:gz") as archive:
        archive.add(str(stage), arcname=name)
    return bundle


class HostCase(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = self.temporary.name
        self.fake = FakeSystem(self.root)
        self.host = self.fresh()

    def tearDown(self):
        for path in ru_host.FOREIGN_PATHS:
            self.assertFalse(os.path.lexists(self.fake.p(path)) and path not in getattr(self, "planted", ()),
                             f"the RU tool created {path}")
        for call in self.fake.calls:
            if call[0] in ("useradd", "install", "chmod", "chown"):
                for arg in call[1:]:
                    local = arg[len(self.root):] if arg.startswith(self.root) else arg
                    self.assertFalse(ru_host.touches_foreign(local), call)
            if call[0] == "useradd":
                self.assertIn(call[-1], ("omoba-ru", "omoba-portal-ru"))
        self.temporary.cleanup()

    def fresh(self):
        return ru_host.Host(self.root, run=self.fake, sleep=lambda _: None, resolve=self.fake.resolve,
                            fetch=self.fake.fetch, root_uid=os.getuid(), now=lambda: 1_790_000_000)

    def payload(self, **extra):
        return {"expected_ip": RU_IP, "dev_ip": None, "behind_nat": False, "units": dict(KIT["units"]),
                "examples": dict(KIT["examples"]), "caddyfile": KIT["caddyfile"], "names": dict(NAMES), **extra}

    def bootstrap(self, **extra):
        return self.fresh().bootstrap(self.payload(kind="bootstrap", vkid=VK_ID, **extra))

    def env(self, kind):
        return ru_host.parse_env(Path(self.fake.p(ru_host.ENV_PATHS[kind])).read_text())[0]

    def server_payload(self, version="0.45.0", revision="abc1234", server=SERVER, account_api=ACCOUNT_API):
        release = f"{version}-ru-{revision}"
        sha, api = hashlib.sha256(server).hexdigest(), hashlib.sha256(account_api).hexdigest()
        return self.payload(kind="server", release=release, grants=GRANTS, sha256=sha, account_api_sha256=api,
                            build={"market": "ru", "release": release, "sha256": sha, "account_api_sha256": api})

    def ship(self, payload=None, server=SERVER, account_api=ACCOUNT_API):
        payload = payload or self.server_payload(server=server, account_api=account_api)
        stage = self.fresh().stage(payload)
        for target in stage["upload_to"]:
            name = os.path.basename(target)[:-len(".part")]
            Path(self.fake.p(target)).write_bytes({"omoba-server": server, "omoba-account-api": account_api}[name])
        return self.fresh().activate_server(payload), payload

    def portal_payload(self, bundle: Path):
        digest = ru_host.sha256_file(str(bundle))
        revision = ru_host.PORTAL_BUNDLE.fullmatch(bundle.name).group(1)
        return self.payload(kind="portal", release=f"{revision[:7]}-{digest[:8]}", file=bundle.name, sha256=digest)

    def ship_portal(self, bundle: Path):
        payload = self.portal_payload(bundle)
        stage = self.fresh().stage(payload)
        for target in stage["upload_to"]:
            Path(self.fake.p(target)).write_bytes(bundle.read_bytes())
        return self.fresh().activate_portal(payload), payload

    def install_node(self):
        with tempfile.TemporaryDirectory() as directory:
            tarball = node_tarball(directory)
            node = {"file": tarball.name, "version": "22.11.0", "sha256": ru_host.sha256_file(str(tarball))}
            stage = self.fresh().stage(self.payload(kind="node", node=node))
            for target in stage["upload_to"]:
                Path(self.fake.p(target)).write_bytes(tarball.read_bytes())
            return node


class RuKitTest(unittest.TestCase):
    """The repository's units, env examples and Caddyfile are what the tool installs."""

    def test_the_repository_units_pass_every_rule(self):
        for kind, text in KIT["units"].items():
            self.assertEqual(ru_host.unit_problems(kind, text, (NAMES["zone"],)), [], kind)

    def test_lobby_and_account_api_wait_for_postgres_and_never_give_up(self):
        for kind in ("lobby", "account-api"):
            directives, _ = ru_host.parse_unit(KIT["units"][kind])
            for key in ("Unit.After", "Unit.Wants"):
                self.assertIn("postgresql.service", " ".join(directives[key]).split(), (kind, key))
            self.assertEqual(directives["Unit.StartLimitIntervalSec"], ["0"])
            self.assertEqual(directives["Service.Restart"], ["always"])
            self.assertNotIn("Unit.StartLimitBurst", directives)

    def test_the_portal_unit_restarts_always_with_caps_and_its_own_user(self):
        directives, environment = ru_host.parse_unit(KIT["units"]["portal"])
        self.assertEqual(directives["Service.Restart"], ["always"])
        self.assertEqual(directives["Service.User"], ["omoba-portal-ru"])
        self.assertNotEqual(directives["Service.User"], ru_host.parse_unit(KIT["units"]["lobby"])[0]["Service.User"])
        self.assertTrue(directives["Service.MemoryMax"] and directives["Service.MemoryHigh"])
        self.assertTrue(directives["Service.CPUQuota"] and directives["Service.TasksMax"])
        self.assertEqual(environment["HOSTNAME"], ["127.0.0.1"], "Next.js binds loopback only")
        self.assertEqual(directives["Service.ExecStart"],
                         ["/opt/omoba-ru/node/current/bin/node /opt/omoba-ru/portal/current/server.js"])

    def test_a_weakened_unit_is_refused(self):
        lobby = KIT["units"]["lobby"]
        cases = {
            "Restart": lobby.replace("Restart=always", "Restart=on-failure"),
            "StartLimitIntervalSec": lobby.replace("StartLimitIntervalSec=0", "StartLimitIntervalSec=120"),
            "postgresql.service": lobby.replace("After=network-online.target postgresql.service",
                                                "After=network-online.target"),
            "OMOBA_DATABASE_URL": lobby.replace("[Service]\n", "[Service]\nEnvironment=OMOBA_DATABASE_URL=x\n"),
            "production/dev path": lobby.replace("ReadWritePaths=/var/lib/omoba-ru",
                                                 "ReadWritePaths=/var/lib/omoba-ru /var/lib/omoba"),
            "another contour": lobby.replace("[Service]\n", "[Service]\nEnvironment=OMOBA_REGISTRY_URL="
                                                           "https://registry.ekza.io\n"),
            "overlap": lobby.replace("OMOBA_MATCH_FIRST_PORT=41000", "OMOBA_MATCH_FIRST_PORT=3999"),
        }
        for needle, text in cases.items():
            problems = ru_host.unit_problems("lobby", text)
            self.assertTrue(any(needle in problem for problem in problems), (needle, problems))
        portal = KIT["units"]["portal"].replace("MemoryMax=512M\n", "")
        self.assertTrue(any("MemoryMax" in p for p in ru_host.unit_problems("portal", portal)))

    def test_the_arena_range_comes_from_the_lobby_unit(self):
        self.assertEqual(ru_host.arena_ports(KIT["units"]["lobby"]), (41000, 41001))
        rules = ru_host.firewall_rules(KIT["units"]["lobby"], {22})
        self.assertEqual([spec for _, spec, _ in rules], ["22/tcp", "443/tcp", "4000/udp", "41000:41001/udp"])
        wider = KIT["units"]["lobby"].replace("OMOBA_MATCH_CAPACITY=2", "OMOBA_MATCH_CAPACITY=6")
        self.assertEqual(ru_host.firewall_rules(wider, {2222})[-1][1], "41000:41005/udp")
        self.assertEqual(ru_host.firewall_rules(wider, {2222})[0][1], "2222/tcp")

    def test_env_examples_are_complete_placeholders(self):
        for kind, text in KIT["examples"].items():
            self.assertEqual(ru_host.example_problems(kind, text), [], kind)
        lobby = ru_host.parse_env(KIT["examples"]["lobby"])[0]
        self.assertEqual(set(lobby), {"OMOBA_DATABASE_URL", "OMOBA_MATCH_PUBLIC_HOST"})
        # The env files must not override what the units set.
        for kind in ru_host.KINDS:
            unit_env = ru_host.parse_unit(KIT["units"][kind])[1]
            self.assertFalse(set(ru_host.parse_env(KIT["examples"][kind])[0]) & set(unit_env), kind)

    def test_kit_refuses_an_inconsistent_caddyfile(self):
        with tempfile.TemporaryDirectory() as directory:
            for path in (ROOT / "ops/vps/ru").iterdir():
                if path.is_file():
                    (Path(directory) / path.name).write_bytes(path.read_bytes())
            caddyfile = Path(directory) / "Caddyfile.ru"
            caddyfile.write_text(caddyfile.read_text().replace("127.0.0.1:3010", "127.0.0.1:3000"))
            with mock.patch.object(ru_host, "RU_DIR", Path(directory)), self.assertRaises(ru_host.Refused) as refused:
                ru_host.kit()
        self.assertIn("does not proxy to 127.0.0.1:3010", refused.exception.message)

    def test_the_names_are_the_ru_profile_of_markets_py(self):
        ru = markets.MARKETS["ru"]
        self.assertEqual(NAMES, {"lobby": ru.lobby.rpartition(":")[0], "account_api": urlsplit(ru.account_api).hostname,
                                 "portal": urlsplit(ru.portal).hostname, "zone": ru.zones[0]})
        self.assertEqual(ru.lobby.rpartition(":")[2], str(ru_host.LOBBY_PORT))
        # Written once: the tool itself holds none of them, so a rename edits markets.py only.
        source = Path(ru_host.__file__).read_text()
        for name in NAMES.values():
            self.assertNotIn(name, source)

    def test_a_renamed_profile_moves_every_check(self):
        # Any rename (the omoba.ru sweep, here a made-up zone that is never the real one)
        # changes what the kit, `names` and the DNS check expect, with no edit to the tool.
        moved = dataclasses.replace(markets.MARKETS["ru"], lobby="lobby.play.example:4000",
                                    account_api="https://account.play.example", portal="https://play.example",
                                    zones=("play.example",))
        with mock.patch.dict(markets.MARKETS, {"ru": moved}):
            names = ru_host.ru_names()
            self.assertEqual(names, {"lobby": "lobby.play.example", "account_api": "account.play.example",
                                     "portal": "play.example", "zone": "play.example"})
            # Caddyfile.ru and the env examples still serve the current names: the kit says so.
            with self.assertRaises(ru_host.Refused) as refused:
                ru_host.kit()
            for needle in ("Caddyfile.ru has no account.play.example site", "Caddyfile.ru has no play.example site",
                           "account-api.env.example: OMOBA_PORTAL_ORIGIN is", "portal.env.example: OMOBA_PORTAL_ORIGIN is"):
                self.assertIn(needle, refused.exception.message)
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                self.assertEqual(ru_host.main(["names"]), 0)
            printed = json.loads(out.getvalue())
        self.assertEqual(printed["dns"]["A"], ["lobby.play.example", "account.play.example", "play.example"])
        self.assertEqual((printed["lobby"], printed["portal"]), ("lobby.play.example:4000", "https://play.example"))
        resolve = lambda name, family: {RU_IP} if family == socket.AF_INET else set()  # noqa: E731
        self.assertEqual(ru_host.dns_problems(resolve, RU_IP, names), ([], []))
        problems, _ = ru_host.dns_problems(lambda name, family: set(), RU_IP, names)
        self.assertEqual(problems, [f"{name} has no A record; point it at {RU_IP}"
                                    for name in ("lobby.play.example", "account.play.example", "play.example")])

    def test_a_profile_the_host_cannot_serve_is_refused(self):
        ru = markets.MARKETS["ru"]
        cases = {
            "must use port 4000": dataclasses.replace(ru, lobby=ru.lobby.rpartition(":")[0] + ":4100"),
            "must be the bare origin": dataclasses.replace(ru, portal=ru.portal + "/play"),
            "overlaps the production or dev names": dataclasses.replace(ru, zones=("omoba.io",)),
            "is outside the RU zone": dataclasses.replace(ru, account_api="https://account.example.ru"),
            "overlaps the cn market's": dataclasses.replace(ru, zones=markets.MARKETS["cn"].zones),
            "is not a lowercase DNS name": dataclasses.replace(ru, zones=()),
        }
        for needle, profile in cases.items():
            with mock.patch.dict(markets.MARKETS, {"ru": profile}), self.assertRaises(ru_host.Refused, msg=needle) as refused:
                ru_host.ru_names()
            self.assertIn(needle, refused.exception.message)


class ParsersTest(unittest.TestCase):
    UFW = """Status: active
Logging: on (low)
Default: deny (incoming), allow (outgoing), disabled (routed)
New profiles: skip

To                         Action      From
--                         ------      ----
22/tcp                     ALLOW IN    Anywhere                   # omoba-ru ssh (key-only)
443/tcp                    ALLOW IN    Anywhere                   # omoba-ru https (Caddy)
4000/udp                   ALLOW IN    Anywhere                   # omoba-ru lobby
41000:41001/udp            ALLOW IN    Anywhere                   # omoba-ru match workers
80/tcp                     ALLOW IN    Anywhere
22/tcp (v6)                ALLOW IN    Anywhere (v6)              # omoba-ru ssh (key-only)
80/tcp (v6)                ALLOW IN    Anywhere (v6)
"""

    def test_ufw_status_is_parsed_and_extra_ports_are_problems(self):
        status = ru_host.parse_ufw(self.UFW)
        self.assertTrue(status["active"])
        self.assertEqual(status["incoming"], "deny")
        self.assertEqual(status["rules"][3]["to"], "41000:41001/udp")
        self.assertEqual(status["rules"][3]["comment"], "omoba-ru match workers")
        self.assertTrue(status["rules"][5]["v6"])
        rules = ru_host.firewall_rules(KIT["units"]["lobby"], {22})
        problems = ru_host.firewall_problems(status, rules)
        self.assertEqual(problems, ["ufw opens 80/tcp, which the RU host does not use: ufw delete allow 80/tcp"])
        self.assertEqual(ru_host.firewall_problems(ru_host.parse_ufw("Status: inactive\n"), rules),
                         ["ufw is not active: the host accepts every inbound port"])
        allow_all = self.UFW.replace("deny (incoming)", "allow (incoming)").replace("4000/udp  ", "4001/udp  ")
        problems = ru_host.firewall_problems(ru_host.parse_ufw(allow_all), rules)
        self.assertIn("ufw default incoming policy is 'allow', not deny", problems)
        self.assertIn("ufw does not allow 4000/udp from anywhere", problems)

    def test_sshd_must_be_key_only(self):
        good = "port 22\npubkeyauthentication yes\npasswordauthentication no\nkbdinteractiveauthentication no\n" \
               "permitemptypasswords no\npermitrootlogin without-password\nauthenticationmethods publickey\n"
        self.assertEqual(ru_host.sshd_problems(good), [])
        self.assertEqual(ru_host.ssh_ports(good + "port 2222\n"), {22, 2222})
        problems = ru_host.sshd_problems(good.replace("passwordauthentication no", "passwordauthentication yes"))
        self.assertEqual(problems, ["sshd passwordauthentication is 'yes', must be no"])
        self.assertTrue(ru_host.SSHD_TEXT.splitlines()[0].startswith("#"))
        self.assertTrue(ru_host.SSHD_DROPIN.endswith("/00-omoba-ru.conf"), "sorts before 50-cloud-init.conf")

    def test_env_files_are_checked_against_their_examples(self):
        example = KIT["examples"]["account-api"]
        values = ru_host.parse_env(example)[0]
        good = ru_host.fill_env(example, ru_host.placeholder_fill(values, RU_IP, VK_ID))
        self.assertEqual(ru_host.env_problems("account-api", good, example, RU_IP), ([], []))
        self.assertIn("# openssl rand -hex 32", good, "comments are kept")
        wrong = good.replace(f"OMOBA_PORTAL_ORIGIN={RU_PORTAL}", "OMOBA_PORTAL_ORIGIN=https://evil.example")
        self.assertIn(f"OMOBA_PORTAL_ORIGIN must be '{RU_PORTAL}' as in account-api.env.example",
                      ru_host.env_problems("account-api", wrong, example)[0])
        no_vk = ru_host.fill_env(example, ru_host.placeholder_fill(values, RU_IP, None))
        self.assertEqual(ru_host.env_problems("account-api", no_vk, example)[0],
                         ["OMOBA_VKID_CLIENT_ID is still a placeholder (needs the owner's VK ID app, LR-19)"])
        bad_secret = re.sub(r"OMOBA_SOCIAL_SECRET=\w+", "OMOBA_SOCIAL_SECRET=short", good)
        self.assertIn("OMOBA_SOCIAL_SECRET must be 64 lowercase hex characters (openssl rand -hex 32)",
                      ru_host.env_problems("account-api", bad_secret, example)[0])
        remote_db = re.sub(r"@127\.0\.0\.1:", "@db.example.com:", good)
        self.assertTrue(any("not loopback" in p for p in ru_host.env_problems("account-api", remote_db, example)[0]))
        portal = ru_host.fill_env(KIT["examples"]["portal"], {"OMOBA_VKID_CLIENT_ID": VK_ID}) + "HOSTNAME=0.0.0.0\n"
        problems, warnings = ru_host.env_problems("portal", portal, KIT["examples"]["portal"], RU_IP, KIT["units"]["portal"])
        self.assertEqual(problems, ["HOSTNAME is set by omoba-portal-ru.service; an EnvironmentFile would override it"])
        self.assertEqual(warnings, ["HOSTNAME is not in portal.env.example"])
        lobby = ru_host.fill_env(KIT["examples"]["lobby"], ru_host.placeholder_fill(
            ru_host.parse_env(KIT["examples"]["lobby"])[0], "95.163.10.21", None))
        self.assertIn("OMOBA_MATCH_PUBLIC_HOST is 95.163.10.21, not RU_IP 95.163.10.20",
                      ru_host.env_problems("lobby", lobby, KIT["examples"]["lobby"], RU_IP)[0])

    def test_postgres_must_be_16_with_icu_on_loopback(self):
        good = {"version": 160004, "icu": True, "listen": "localhost", "port": 5432, "database_owner": None}
        self.assertEqual(ru_host.postgres_problems(good), [])
        self.assertEqual(ru_host.postgres_problems({**good, "listen": "127.0.0.1,::1"}), [])
        problems = ru_host.postgres_problems({**good, "version": 140011, "icu": False, "listen": "*",
                                              "database_owner": "postgres"})
        self.assertEqual(len(problems), 4, problems)
        self.assertIn("older than 16", problems[0])
        self.assertIn('"und-x-icu"', problems[1])
        self.assertIn("loopback only", problems[2])

    def test_dns_must_point_at_the_ru_host_without_aaaa_for_the_lobby(self):
        dns = {NAMES[key]: {socket.AF_INET: {RU_IP}, socket.AF_INET6: set()} for key in ru_host.HOST_KEYS}
        resolve = lambda name, family: dns[name][family]  # noqa: E731
        self.assertEqual(ru_host.dns_problems(resolve, RU_IP, NAMES), ([], []))
        dns[NAMES["lobby"]][socket.AF_INET6] = {"2a00::1"}
        dns[NAMES["portal"]][socket.AF_INET] = {"1.2.3.4"}
        problems, _ = ru_host.dns_problems(resolve, RU_IP, NAMES)
        self.assertEqual(len(problems), 2, problems)

    def test_names_of_other_contours_are_foreign(self):
        own = (NAMES["zone"],)
        for name in ("vds-eternal", "vds-dev", PROD_IP, "omoba.io", "app.omoba.io", "lobby.dev.omoba.io",
                     "registry.ekza.io", "root@vds-eternal", "APP.OMOBA.IO."):
            self.assertTrue(ru_host.is_foreign_name(name, own), name)
        for name in ("vds-ru", NAMES["lobby"], NAMES["account_api"], NAMES["portal"], RU_IP, "postgresql.service"):
            self.assertFalse(ru_host.is_foreign_name(name, own), name)

    def test_the_host_half_checks_the_names_it_is_given(self):
        self.assertEqual(ru_host.names_problems(NAMES), [])
        self.assertEqual(ru_host.names_problems(None), ["the payload lacks the RU names (scripts/markets.py)"])
        for zone in ("omoba.io", "dev.omoba.io", "x.dev.omoba.io", "app.omoba.io", "ekza.io", "dev.ekza.io"):
            names = {key: f"{key.replace('_', '-')}.{zone}" for key in ru_host.HOST_KEYS}
            problems = ru_host.names_problems({**names, "zone": zone})
            self.assertTrue(problems and problems[0].startswith(f"the RU zone {zone} overlaps the production or dev"),
                            (zone, problems))
        self.assertIn("the RU portal name app.omoba.io is outside the RU zone " + NAMES["zone"],
                      ru_host.names_problems({**NAMES, "portal": "app.omoba.io"}))
        self.assertTrue(ru_host.names_problems({**NAMES, "portal": NAMES["account_api"]}))
        self.assertIn("the RU lobby name (not shown) is not a lowercase DNS name",
                      ru_host.names_problems({**NAMES, "lobby": "lobby x; rm -rf /"}))
        self.assertFalse(ru_host.touches_foreign("/opt/omoba-ru/current"))
        self.assertTrue(ru_host.touches_foreign("/opt/omoba/current"))
        self.assertTrue(ru_host.touches_foreign("/opt/omoba-ru/../omoba-dev/x"))

    def test_the_ru_address_must_be_public_and_not_another_vps(self):
        self.assertEqual(ru_host.validate_ru_ip(f" {RU_IP} "), RU_IP)
        for value in (PROD_IP, "127.0.0.1", "10.1.2.3", "192.168.1.5", "100.64.0.1", "0.0.0.0", NAMES["lobby"]):
            with self.assertRaises(ru_host.Refused, msg=value):
                ru_host.validate_ru_ip(value)
        with self.assertRaises(ru_host.Refused):
            ru_host.validate_ru_ip(DEV_IP, dev_ip=DEV_IP)


class BootstrapTest(HostCase):
    def test_preflight_on_a_fresh_host_changes_nothing_and_lists_the_work(self):
        before = tree(self.root)
        report = self.fresh().bootstrap_preflight(self.payload(kind="bootstrap", vkid=None))
        self.assertEqual(tree(self.root), before)
        self.assertTrue(report["ok"], report)
        self.assertIn("apt-get install postgresql caddy ufw", report["would"][0])
        self.assertTrue(any("omoba-portal-ru" in step for step in report["would"]))
        self.assertTrue(any("placeholder" in warning for warning in report["warnings"]))
        self.assertTrue(any("Node.js" in warning for warning in report["warnings"]))
        changes = [call for call in self.fake.calls
                   if call[0] in ("apt-get", "useradd", "install", "chmod", "chown")
                   or (call[0] == "ufw" and call[1] != "status")
                   or (call[0] == "systemctl" and call[1] not in ("show", "is-active", "is-enabled"))]
        self.assertEqual(changes, [])

    def test_the_host_half_uses_the_names_of_the_payload(self):
        # It has no checkout: the names come only from the Mac half (scripts/markets.py).
        for names in (None, {**NAMES, "zone": "omoba.io"}):
            for check in (self.fresh().status, self.fresh().bootstrap_preflight):
                with self.assertRaises(ru_host.Refused, msg=(names, check)):
                    check(self.payload(kind="bootstrap", names=names))
        self.fake.dns.clear()
        report = self.fresh().bootstrap_preflight(self.payload(kind="bootstrap", vkid=VK_ID))
        self.assertIn(f"DNS: {NAMES['portal']} has no A record; point it at {RU_IP}", report["warnings"])
        self.assertEqual(self.fresh().status(self.payload())["facts"]["names"],
                         {key: NAMES[key] for key in ru_host.HOST_KEYS})

    def test_bootstrap_refuses_other_contours_and_platforms(self):
        cases = {
            "/opt/omoba": lambda: os.makedirs(self.fake.p("/opt/omoba")),
            "/opt/omoba-dev": lambda: os.makedirs(self.fake.p("/opt/omoba-dev")),
            "user omoba-dev": lambda: self.fake.users.update({"omoba-dev": {"uid": 900, "home": "/x", "shell": "/bin/sh"}}),
            "systemd unit omoba-lobby": lambda: self.fake.unit("omoba-lobby"),
            f"address {PROD_IP}": lambda: self.fake.ipv4.add(PROD_IP),
            "Ubuntu 24.04 or newer": lambda: self.fake.write("/etc/os-release", 'ID=ubuntu\nVERSION_ID="22.04"\n'),
            "found debian": lambda: self.fake.write("/etc/os-release", 'ID=debian\nVERSION_ID="12"\n'),
            "x86-64": lambda: setattr(self.fake, "arch", "aarch64"),
            "not on a local interface": lambda: self.fake.ipv4.discard(RU_IP),
            "names market ru at 95.163.10.99": lambda: self.fake.write(
                ru_host.MARKER, json.dumps({"market": "ru", "ip": "95.163.10.99"})),
        }
        for needle, plant in cases.items():
            with self.subTest(needle):
                self.tearDown()
                self.setUp()
                plant()
                self.planted = [path for path in ru_host.FOREIGN_PATHS if os.path.lexists(self.fake.p(path))]
                before = tree(self.root)
                with self.assertRaises(ru_host.Refused) as refused:
                    self.bootstrap()
                self.assertIn(needle, refused.exception.message)
                self.assertEqual(tree(self.root), before, "nothing was written")
                self.assertFalse([call for call in self.fake.calls if call[0] in ("apt-get", "useradd")])
        self.tearDown()
        self.setUp()
        self.fake.ipv4.discard(RU_IP)
        self.assertEqual(self.bootstrap(behind_nat=True)["status"], "BOOTSTRAPPED")

    def test_a_fresh_host_is_set_up_completely(self):
        result = self.bootstrap()
        self.assertEqual(result["status"], "BOOTSTRAPPED")
        fake = self.fake
        self.assertTrue({"postgresql", "caddy", "ufw"} <= fake.packages)
        self.assertEqual(json.loads(Path(fake.p(ru_host.MARKER)).read_text())["ip"], RU_IP)
        self.assertEqual(set(fake.users), {"omoba-ru", "omoba-portal-ru"})
        self.assertEqual(fake.users["omoba-portal-ru"]["shell"], "/usr/sbin/nologin")
        for path, _, mode in ru_host.DIRS:
            self.assertEqual(oct(os.stat(fake.p(path)).st_mode & 0o777), oct(int(mode, 8)), path)
        # Env files: root-only, secrets generated here, the address and the VK ID app filled in.
        for kind, path in ru_host.ENV_PATHS.items():
            self.assertEqual(os.stat(fake.p(path)).st_mode & 0o777, 0o600, path)
            problems, _ = ru_host.env_problems(kind, Path(fake.p(path)).read_text(), KIT["examples"][kind], RU_IP,
                                               KIT["units"][kind])
            self.assertEqual(problems, [], kind)
        api = self.env("account-api")
        self.assertRegex(api["OMOBA_SOCIAL_SECRET"], HEX64)
        self.assertNotEqual(api["OMOBA_SOCIAL_SECRET"], api["OMOBA_PORTAL_SECRET"])
        self.assertEqual(self.env("portal")["OMOBA_VKID_CLIENT_ID"], VK_ID)
        self.assertEqual(self.env("lobby")["OMOBA_MATCH_PUBLIC_HOST"], RU_IP)
        # Database: roles log in with exactly the env files' passwords; the owner does not.
        self.assertEqual(fake.database_owner, "omoba_owner")
        self.assertEqual(fake.roles, {"omoba_owner": False, "omoba_game": True, "omoba_portal": True})
        self.assertEqual(fake.passwords["omoba_game"], urlsplit(self.env("lobby")["OMOBA_DATABASE_URL"]).password)
        self.assertEqual(fake.passwords["omoba_portal"], urlsplit(api["OMOBA_DATABASE_URL"]).password)
        # Caddy, SSH, firewall, units.
        self.assertEqual(Path(fake.p("/etc/caddy/Caddyfile")).read_text(), KIT["caddyfile"])
        self.assertIn("file_server", Path(fake.p("/etc/caddy/Caddyfile.orig")).read_text())
        self.assertEqual(Path(fake.p(ru_host.SSHD_DROPIN)).read_text(), ru_host.SSHD_TEXT)
        self.assertTrue(fake.ufw["active"])
        self.assertEqual(fake.ufw["incoming"], "deny")
        self.assertEqual([rule[1] for rule in fake.ufw["rules"]], ["22/tcp", "443/tcp", "4000/udp", "41000:41001/udp"])
        for kind in ru_host.KINDS:
            self.assertEqual(Path(fake.p(ru_host.unit_path(kind))).read_text(), KIT["units"][kind])
        self.assertIn(["systemctl", "daemon-reload"], fake.calls)
        self.assertEqual(fake.enabled, {"postgresql", "caddy"}, "no RU unit is enabled before its release exists")
        # What is left is shipping, the portal runtime and nothing about the host itself.
        remaining = "\n".join(result["remaining"])
        self.assertIn("no lobby/Account API release yet (ship)", remaining)
        self.assertIn("Node.js >= 20.9 is missing", remaining)
        self.assertNotIn("ufw", remaining)
        self.assertNotIn("sshd", remaining)
        self.assertNotIn("PostgreSQL", remaining)
        # No secret ever reached argv or the result.
        secrets = [api["OMOBA_SOCIAL_SECRET"], api["OMOBA_PORTAL_SECRET"], fake.passwords["omoba_game"]]
        blob = json.dumps(result) + json.dumps(fake.calls)
        for secret in secrets:
            self.assertNotIn(secret, blob)

    def test_bootstrap_is_idempotent_and_never_replaces_a_secret(self):
        self.fresh().bootstrap(self.payload(kind="bootstrap", vkid=None))
        api = self.env("account-api")
        self.assertIn("CHANGE_ME", api["OMOBA_VKID_CLIENT_ID"])
        second = self.bootstrap()   # now with the VK ID app id
        self.assertEqual(second["done"], ["fill /opt/omoba-ru/account-api.env (OMOBA_VKID_CLIENT_ID)",
                                          "fill /opt/omoba-ru/portal.env (OMOBA_VKID_CLIENT_ID)"])
        after = self.env("account-api")
        self.assertEqual(after["OMOBA_VKID_CLIENT_ID"], VK_ID)
        for key in ("OMOBA_SOCIAL_SECRET", "OMOBA_PORTAL_SECRET", "OMOBA_DATABASE_URL"):
            self.assertEqual(after[key], api[key], key)
        third = self.bootstrap()
        self.assertEqual(third["done"], [])

    def test_postgres_without_icu_or_on_a_public_address_stops_before_secrets(self):
        self.fake.pg.update(icu=False, listen="*")
        with self.assertRaises(ru_host.Refused) as refused:
            self.bootstrap()
        self.assertIn("und-x-icu", refused.exception.message)
        self.assertIn("loopback only", refused.exception.message)
        self.assertFalse(os.path.exists(self.fake.p(ru_host.ENV_PATHS["account-api"])))

    def test_key_only_ssh_needs_an_authorized_key_and_a_valid_config(self):
        os.unlink(self.fake.p("/root/.ssh/authorized_keys"))
        report = self.fresh().bootstrap_preflight(self.payload(kind="bootstrap"))
        self.assertFalse(report["ok"])
        with self.assertRaises(ru_host.Refused) as refused:
            self.bootstrap()
        self.assertIn("lock everyone out", refused.exception.message)
        self.assertFalse(os.path.exists(self.fake.p(ru_host.SSHD_DROPIN)))
        self.fake.write("/root/.ssh/authorized_keys", "ssh-ed25519 AAAA owner@mac\n")
        self.fake.fail_sshd_test = True
        with self.assertRaises(SystemExit) as failed:
            self.bootstrap()
        self.assertIn("sshd -t refused", str(failed.exception))
        self.assertFalse(os.path.exists(self.fake.p(ru_host.SSHD_DROPIN)), "the bad drop-in was removed")

    def test_caddy_validate_failure_keeps_the_old_caddyfile(self):
        self.fake.fail_caddy_validate = True
        with self.assertRaises(SystemExit):
            self.bootstrap()
        self.assertIn("file_server", Path(self.fake.p("/etc/caddy/Caddyfile")).read_text())
        self.assertFalse(os.path.exists(self.fake.p("/etc/caddy/Caddyfile.omoba-next")))

    def test_firewall_drops_its_own_stale_arena_rule(self):
        self.bootstrap()
        wider = KIT["units"]["lobby"].replace("OMOBA_MATCH_CAPACITY=2", "OMOBA_MATCH_CAPACITY=4")
        payload = self.payload(kind="bootstrap", vkid=VK_ID)
        payload["units"]["lobby"] = wider
        self.fresh().bootstrap(payload)
        self.assertEqual([rule[1] for rule in self.fake.ufw["rules"]],
                         ["22/tcp", "443/tcp", "4000/udp", "41000:41003/udp"])
        self.assertIn(["ufw", "--force", "delete", "allow", "41000:41001/udp"], self.fake.calls)

    @unittest.skipUnless(HAS_DATA_FILTER, "needs Python with the tarfile data filter (3.12+)")
    def test_node_from_an_official_tarball(self):
        node = self.install_node()
        result = self.bootstrap(node=node)
        self.assertIn("install Node.js 22.11.0 into /opt/omoba-ru/node/v22.11.0", result["done"])
        self.assertEqual(os.readlink(self.fake.p(ru_host.NODE_CURRENT)), "v22.11.0")
        self.assertEqual(os.readlink(self.fake.p("/opt/omoba-ru/node/v22.11.0/bin/npm")),
                         "../lib/node_modules/npm/bin/npm-cli.js")
        self.assertEqual(result["facts"]["node"], "22.11.0")
        self.assertFalse(os.listdir(self.fake.p(ru_host.INCOMING)), "the upload was consumed")
        self.assertEqual(self.fresh().stage(self.payload(kind="node", node=node))["upload_to"], [], "already installed")

    def test_a_tampered_node_tarball_is_refused(self):
        node = self.install_node()
        with self.assertRaises(ru_host.Refused) as refused:
            self.bootstrap(node={**node, "sha256": "0" * 64})
        self.assertIn("does not match --node-sha256", refused.exception.message)
        self.assertFalse(os.path.exists(self.fake.p(f"{ru_host.INCOMING}/{node['file']}.part")))

    def test_ubuntus_nodejs_is_used_only_when_new_enough(self):
        self.bootstrap()
        self.assertNotIn("nodejs", self.fake.packages, "Ubuntu 24.04's 18.x is too old for Next.js 16")
        self.fake.node_candidate = "22.14.0+dfsg-1ubuntu1"
        result = self.bootstrap()
        self.assertIn("nodejs", self.fake.packages)
        self.assertEqual(os.readlink(self.fake.p(ru_host.NODE_CURRENT)), "system")
        self.assertEqual(os.readlink(self.fake.p("/opt/omoba-ru/node/system/bin/node")), "/usr/bin/node")
        self.assertIn("apt-get install nodejs", result["done"])


class ShipTest(HostCase):
    def setUp(self):
        super().setUp()
        self.bootstrap()

    def test_first_ship_migrates_switches_and_enables(self):
        result, payload = self.ship()
        self.assertEqual(result["status"], "DEPLOYED")
        fake = self.fake
        self.assertEqual(os.readlink(fake.p(ru_host.CURRENT)), f"releases/{payload['release']}")
        self.assertEqual(len(fake.migrations), 1)
        self.assertEqual(fake.grants, 1)
        self.assertFalse(fake.roles["omoba_owner"], "the owner login is disabled again")
        self.assertEqual(fake.dumps, [], "nothing to back up before the first migration")
        self.assertTrue({"omoba-lobby-ru", "omoba-account-api-ru"} <= fake.enabled)
        self.assertTrue(fake.unit("omoba-lobby-ru")["active"] and fake.unit("omoba-account-api-ru")["active"])
        build = json.loads(Path(fake.p(f"/opt/omoba-ru/releases/{payload['release']}/BUILD.json")).read_text())
        self.assertEqual(build["market"], "ru")
        history = Path(fake.p(ru_host.HISTORY)).read_text().splitlines()
        self.assertEqual(json.loads(history[-1])["release"], payload["release"])
        self.assertEqual(self.fresh().activate_server(payload)["status"], "UNCHANGED")

    def test_an_update_backs_the_database_up_first(self):
        self.ship()
        newer = self.server_payload(version="0.45.1", revision="def5678", server=elf(b"omoba-server ru 2"))
        result, _ = self.ship(newer, server=elf(b"omoba-server ru 2"))
        self.assertEqual(result["previous"], "0.45.0-ru-abc1234")
        self.assertEqual(len(self.fake.dumps), 1)
        self.assertIn("/var/backups/omoba-ru/omoba_ru-before-0.45.1-ru-def5678-", self.fake.dumps[0])

    def test_ship_waits_for_the_vk_id_app_and_for_running_matches(self):
        self.tearDown()
        self.setUp_without_vk()
        with self.assertRaises(ru_host.Refused) as refused:
            self.ship()
        self.assertIn("OMOBA_VKID_CLIENT_ID is still a placeholder", refused.exception.message)
        self.bootstrap()
        self.ship()
        self.fake.extra_workers = [8100]
        newer = self.server_payload(version="0.45.1", revision="def5678")
        with self.assertRaises(ru_host.Refused) as refused:
            self.ship(newer)
        self.assertIn("match workers are running", refused.exception.message)
        self.assertEqual(os.readlink(self.fake.p(ru_host.CURRENT)), "releases/0.45.0-ru-abc1234")

    def setUp_without_vk(self):
        HostCase.setUp(self)
        self.fresh().bootstrap(self.payload(kind="bootstrap", vkid=None))

    def test_a_failed_start_rolls_back(self):
        self.ship()
        self.fake.fail_start.add("omoba-lobby-ru")
        newer = self.server_payload(version="0.45.1", revision="def5678", server=elf(b"omoba-server ru 2"))
        with self.assertRaises(SystemExit) as failed:
            self.ship(newer, server=elf(b"omoba-server ru 2"))
        self.assertIn("ROLLED_BACK to 0.45.0-ru-abc1234", str(failed.exception))
        self.assertEqual(os.readlink(self.fake.p(ru_host.CURRENT)), "releases/0.45.0-ru-abc1234")

    def test_a_failed_first_install_is_left_stopped_and_disabled(self):
        self.fake.fail_start.add("omoba-account-api-ru")
        with self.assertRaises(SystemExit) as failed:
            self.ship()
        self.assertIn("first install", str(failed.exception))
        self.assertFalse({"omoba-lobby-ru", "omoba-account-api-ru"} & self.fake.enabled)

    def test_releases_are_immutable_and_uploads_checked(self):
        self.ship()
        other = self.server_payload(server=elf(b"different"))
        with self.assertRaises(ru_host.Refused) as refused:
            self.fresh().stage(other)
        self.assertIn("releases are immutable", refused.exception.message)
        newer = self.server_payload(version="0.45.1", revision="def5678")
        stage = self.fresh().stage(newer)
        for target in stage["upload_to"]:
            Path(self.fake.p(target)).write_bytes(b"tampered")
        with self.assertRaises(ru_host.Refused) as refused:
            self.fresh().activate_server(newer)
        self.assertIn("does not match the local sha256", refused.exception.message)

    def test_rollback_points_at_a_staged_release_without_migrating(self):
        self.ship()
        self.ship(self.server_payload(version="0.45.1", revision="def5678", server=elf(b"v2")), server=elf(b"v2"))
        migrations = len(self.fake.migrations)
        result = self.fresh().rollback(self.payload(kind="server", release="0.45.0-ru-abc1234"))
        self.assertEqual(result["status"], "DEPLOYED")
        self.assertEqual(result["mode"], "rollback")
        self.assertEqual(len(self.fake.migrations), migrations)
        self.assertEqual(os.readlink(self.fake.p(ru_host.CURRENT)), "releases/0.45.0-ru-abc1234")

    def test_ship_needs_a_bootstrapped_host(self):
        os.unlink(self.fake.p(ru_host.MARKER))
        with self.assertRaises(ru_host.Refused) as refused:
            self.fresh().stage(self.server_payload())
        self.assertIn("bootstrap", refused.exception.message)


@unittest.skipUnless(HAS_DATA_FILTER, "needs Python with the tarfile data filter (3.12+)")
class PortalAndRebootTest(HostCase):
    def setUp(self):
        super().setUp()
        self.node = self.install_node()
        self.bootstrap(node=self.node)
        self.work = tempfile.TemporaryDirectory()

    def tearDown(self):
        self.work.cleanup()
        super().tearDown()

    def test_ship_portal_unpacks_links_the_cache_and_starts(self):
        result, payload = self.ship_portal(portal_bundle(self.work.name))
        self.assertEqual(result["status"], "DEPLOYED")
        directory = self.fake.p(f"/opt/omoba-ru/portal/releases/{payload['release']}")
        self.assertEqual(os.readlink(os.path.join(directory, ".next/cache")), "/var/cache/omoba-portal-ru")
        self.assertEqual(json.loads(Path(directory, "BUILD.json").read_text())["bundle_sha256"], payload["sha256"])
        self.assertTrue(os.path.isfile(os.path.join(directory, ".next/static/chunks/app.js")))
        self.assertEqual(os.stat(os.path.join(directory, "server.js")).st_mode & 0o777, 0o644,
                         "readable by omoba-portal-ru")
        self.assertIn("omoba-portal-ru", self.fake.enabled)
        self.assertEqual(self.fresh().activate_portal(payload)["status"], "UNCHANGED")

    def test_a_bundle_for_another_market_or_with_env_files_is_refused(self):
        cases = {"not built with OMOBA_MARKET=ru": {"market": "global (ekza on, supporter on, vk_sign_in off)"},
                 "not a production-contour build": {"contour": "dev"},
                 ".env.local would ship build-time env": {"extra": {".env.local": "SECRET=1\n"}}}
        for index, (needle, kwargs) in enumerate(cases.items()):
            directory = os.path.join(self.work.name, str(index))
            os.makedirs(directory)
            bundle = portal_bundle(directory, revision=f"abc123{index}", **kwargs)
            problems, _ = ru_host.inspect_bundle(bundle)
            self.assertTrue(any(needle in p for p in problems), (needle, problems))
            with self.assertRaises(ru_host.Refused) as refused:
                self.ship_portal(bundle)
            self.assertIn(needle, refused.exception.message)
            self.assertFalse(os.path.lexists(self.fake.p(ru_host.PORTAL_CURRENT)))

    def test_a_bundle_escaping_its_folder_is_refused(self):
        bundle = Path(self.work.name) / "omoba-portal-ru-abc1234.tar.gz"
        with tarfile.open(bundle, "w:gz") as archive:
            info = tarfile.TarInfo("omoba-portal-ru-abc1234/../../etc/cron.d/x")
            info.size = 2
            archive.addfile(info, io.BytesIO(b"x\n"))
        self.assertIn("outside", ru_host.inspect_bundle(bundle)[0][0])
        with self.assertRaises(ru_host.Refused):
            self.ship_portal(bundle)
        self.assertFalse(os.path.exists(self.fake.p("/etc/cron.d/x")))

    def test_a_portal_that_does_not_answer_rolls_back(self):
        _, first = self.ship_portal(portal_bundle(self.work.name))
        os.makedirs(os.path.join(self.work.name, "next"))
        self.fake.fail_start.add("omoba-portal-ru")
        with self.assertRaises(SystemExit) as failed:
            self.ship_portal(portal_bundle(os.path.join(self.work.name, "next"), revision="def5678"))
        self.assertIn(f"ROLLED_BACK to {first['release']}", str(failed.exception))
        self.assertEqual(os.readlink(self.fake.p(ru_host.PORTAL_CURRENT)), f"releases/{first['release']}")

    def test_everything_is_up_after_a_reboot_with_no_manual_step(self):
        self.ship()
        self.ship_portal(portal_bundle(self.work.name))
        report = self.fresh().status(self.payload())
        self.assertEqual(report["problems"], [])
        self.assertTrue(report["ok"])
        self.assertEqual(report["facts"]["account_api_http"], 200)
        self.fake.boot()
        after = self.fresh().status(self.payload())
        self.assertTrue(after["ok"], after["problems"])
        self.assertEqual(self.fresh().boot_id(), {"boot_id": "boot-2"})
        # A unit that is not enabled would not come back: status says so before the reboot test.
        self.fake.enabled.discard("omoba-portal-ru")
        self.fake.boot()
        problems = self.fresh().status(self.payload())["problems"]
        self.assertIn("omoba-portal-ru is not enabled: it would not start after a reboot", problems)
        self.assertIn("omoba-portal-ru is not running", problems)

    def test_status_reports_drift(self):
        self.ship()
        self.ship_portal(portal_bundle(self.work.name))
        self.fake.ufw["rules"].append(("allow", "5432/tcp", ""))
        self.fake.tcp.add(("0.0.0.0", 5432))
        os.unlink(self.fake.p(ru_host.SSHD_DROPIN))
        self.fake.dns[NAMES["lobby"]][socket.AF_INET] = set()
        os.chmod(self.fake.p(ru_host.ENV_PATHS["portal"]), 0o644)
        problems = "\n".join(self.fresh().status(self.payload())["problems"])
        for needle in ("ufw opens 5432/tcp", "PostgreSQL listens on ['0.0.0.0:5432']", "sshd passwordauthentication",
                       f"{NAMES['lobby']} has no A record", "portal.env has mode 0o644"):
            self.assertIn(needle, problems)


class RemoteEntryTest(unittest.TestCase):
    def test_the_host_half_runs_as_root_only(self):
        with mock.patch("os.geteuid", return_value=1000):
            with self.assertRaises(ru_host.Refused):
                ru_host.remote_main(["status", ru_host.encode_payload({})])
        with self.assertRaises(ru_host.Refused):
            ru_host.remote_main(["deploy-everything"])
        with self.assertRaises(ru_host.Refused):
            ru_host.decode_payload("not base64!")


def args(**values):
    defaults = {"host": None, "ip": None, "behind_nat": False, "country_confirmed": False, "check": False,
                "apply": False}
    return SimpleNamespace(**{**defaults, **values})


class MacTargetTest(unittest.TestCase):
    def setUp(self):
        patcher = mock.patch.dict(os.environ, {}, clear=False)
        patcher.start()
        self.addCleanup(patcher.stop)
        for key in ("RU_HOST", "RU_IP", "DEV_IP"):
            os.environ.pop(key, None)

    def resolve(self, alias="vds-ru", ip=RU_IP, hostname=RU_IP, user="root", countries=("RU",), **extra):
        return ru_host.resolve_target(args(host=alias, ip=ip, **extra), need_ip=True, country=True,
                                      config_reader=lambda _: {"hostname": hostname, "user": user},
                                      whois=lambda _: set(countries))

    def test_the_ru_vps_is_accepted(self):
        target = self.resolve()
        self.assertEqual((target["alias"], target["expected_ip"]), ("vds-ru", RU_IP))

    def test_other_hosts_are_refused_before_any_ssh(self):
        cases = {"production or dev VPS": dict(alias="vds-eternal"),
                 "is the production VPS": dict(ip=PROD_IP, hostname=PROD_IP),
                 "connects to": dict(hostname="95.163.10.21"),
                 "logs in as": dict(user="ubuntu"),
                 "the RU host must be in Russia": dict(countries=("NL",)),
                 "gives no country": dict(countries=())}
        for needle, kwargs in cases.items():
            with self.assertRaises(ru_host.Refused, msg=needle) as refused:
                self.resolve(**kwargs)
            self.assertIn(needle, refused.exception.message)
        os.environ["DEV_IP"] = DEV_IP
        with self.assertRaises(ru_host.Refused) as refused:
            self.resolve(ip=DEV_IP, hostname=DEV_IP)
        self.assertIn("dev VPS", refused.exception.message)

    def test_the_owner_can_confirm_the_country_when_whois_is_silent(self):
        self.assertEqual(self.resolve(countries=(), country_confirmed=True)["expected_ip"], RU_IP)

    def test_reboot_check_waits_for_a_new_boot_and_a_clean_status(self):
        boots = iter(["boot-1", "boot-1", "boot-2"])
        statuses = iter([{"ok": True, "problems": []}, {"ok": False, "problems": ["omoba-portal-ru is not running"]},
                         {"ok": True, "problems": []}])
        reboots = []

        def remote(alias, mode, payload=None, check=True):
            text = json.dumps({"boot_id": next(boots)} if mode == "boot-id" else next(statuses))
            return text if check else (0, text)
        clock = iter(range(0, 10_000, 10))
        with mock.patch.object(ru_host, "ssh_remote", side_effect=remote), \
                mock.patch.object(ru_host, "ssh_config", return_value={"hostname": RU_IP, "user": "root"}), \
                mock.patch.object(ru_host.subprocess, "run", side_effect=lambda cmd, **kw: reboots.append(cmd)):
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                code = ru_host.command_reboot_check(args(host="vds-ru", ip=RU_IP, apply=True, boot_timeout=600,
                                                         ready_timeout=300),
                                                    sleep=lambda _: None, clock=lambda: next(clock))
        self.assertEqual(code, 0)
        self.assertEqual(reboots, [["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "vds-ru",
                                    "systemctl", "reboot"]])
        result = json.loads(out.getvalue())
        self.assertTrue(result["ok"] and result["rebooted"])

    def test_reboot_check_refuses_an_unhealthy_host(self):
        with mock.patch.object(ru_host, "ssh_remote", return_value=json.dumps(
                {"ok": False, "problems": ["no portal release yet (ship-portal)"]})), \
                mock.patch.object(ru_host, "ssh_config", return_value={"hostname": RU_IP, "user": "root"}), \
                mock.patch.object(ru_host.subprocess, "run") as run:
            with self.assertRaises(ru_host.Refused):
                ru_host.command_reboot_check(args(host="vds-ru", ip=RU_IP, apply=True, boot_timeout=1,
                                                  ready_timeout=1))
            run.assert_not_called()

    def test_a_dry_run_contacts_nothing(self):
        with mock.patch.object(ru_host, "ssh_remote") as remote, mock.patch.object(ru_host, "ssh_config",
                                                                                   return_value=None):
            out = io.StringIO()
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(ru_host.main(["bootstrap", "--host", "vds-ru", "--ip", RU_IP]), 0)
            remote.assert_not_called()
        self.assertTrue(json.loads(out.getvalue())["dry_run"])


class MacPackageTest(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory()
        self.addCleanup(self.work.cleanup)

    def package(self, name="omoba-0.45.0-ru-linux-x64.tar.gz", top="omoba-0.45.0-ru", server=SERVER):
        path = Path(self.work.name) / name
        with tarfile.open(path, "w:gz") as archive:
            for member, data in (("omoba-server", server), ("omoba-account-api", ACCOUNT_API), ("omoba", b"client")):
                info = tarfile.TarInfo(f"{top}/{member}")
                info.size, info.mode = len(data), 0o755
                archive.addfile(info, io.BytesIO(data))
        return path

    def test_only_an_ru_linux_package_ships(self):
        version, binaries = ru_host.read_package(self.package())
        self.assertEqual(version, "0.45.0")
        self.assertEqual(binaries["omoba-server"], SERVER)
        for name, top in (("omoba-0.45.0-linux-x64.tar.gz", "omoba-0.45.0"),
                          ("omoba-0.45.0-dev-linux-x64.tar.gz", "omoba-0.45.0-dev"),
                          ("omoba-0.45.0-ru-dev-linux-x64.tar.gz", "omoba-0.45.0-ru-dev")):
            with self.assertRaises(ru_host.Refused, msg=name):
                ru_host.read_package(self.package(name, top))
        with self.assertRaises(ru_host.Refused):
            ru_host.read_package(self.package(server=elf(machine=b"\xb7\x00")))

    def test_ship_apply_uploads_only_what_the_host_asks_for(self):
        package = self.package()
        answers = {"preflight": json.dumps({"ok": True}),
                   "stage": json.dumps({"upload_to": ["/opt/omoba-ru/releases/0.45.0-ru-abc1234/omoba-server.part"]}),
                   "activate": json.dumps({"status": "DEPLOYED"})}
        sent = []

        def remote(alias, mode, payload=None, check=True):
            sent.append((mode, payload))
            return answers[mode]
        uploads = []
        with mock.patch.object(ru_host, "ssh_remote", side_effect=remote), \
                mock.patch.object(ru_host, "ssh_config", return_value={"hostname": RU_IP, "user": "root"}), \
                mock.patch.object(ru_host.subprocess, "run", side_effect=lambda cmd, **kw: uploads.append(
                    (cmd, Path(cmd[-2]).read_bytes()))), \
                contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            code = ru_host.main(["ship", "--package", str(package), "--revision", "abc1234", "--host", "vds-ru",
                                 "--ip", RU_IP, "--apply"])
        self.assertEqual(code, 0)
        self.assertEqual([mode for mode, _ in sent], ["preflight", "stage", "activate"])
        self.assertEqual(sent[-1][1]["release"], "0.45.0-ru-abc1234")
        self.assertEqual(sent[-1][1]["grants"], GRANTS)
        self.assertEqual(len(uploads), 1)
        self.assertEqual(uploads[0][0][-1], "vds-ru:/opt/omoba-ru/releases/0.45.0-ru-abc1234/omoba-server.part")
        self.assertEqual(uploads[0][1], SERVER)
        answers["stage"] = json.dumps({"upload_to": ["/etc/cron.d/evil.part"]})
        sent.clear()
        with mock.patch.object(ru_host, "ssh_remote", side_effect=remote), \
                mock.patch.object(ru_host, "ssh_config", return_value={"hostname": RU_IP, "user": "root"}), \
                contextlib.redirect_stdout(io.StringIO()), self.assertRaises(ru_host.Refused):
            ru_host.main(["ship", "--package", str(package), "--revision", "abc1234", "--host", "vds-ru",
                          "--ip", RU_IP, "--apply"])

    def test_portal_bundle_packs_a_built_ru_checkout(self):
        web = Path(self.work.name) / "omoba-web"
        standalone = web / ".next/standalone"
        standalone.mkdir(parents=True)
        portal_tree(standalone)
        (standalone / ".env").write_text("SECRET=1\n")   # Next copies .env files; the bundle must not
        (standalone / ".next/cache").mkdir()
        (web / ".next/static/chunks").mkdir(parents=True)
        (web / ".next/static/chunks/app.js").write_text("x\n")
        (web / "public").mkdir()
        (web / "public/robots.txt").write_text("User-agent: *\n")
        (web / ".gitignore").write_text(".next/\n")
        for command in (["git", "init", "-q"], ["git", "add", "."],
                        ["git", "-c", "user.email=t@example.com", "-c", "user.name=t", "commit", "-qm", "x"]):
            subprocess.run(command, cwd=web, check=True)
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            ru_host.main(["portal-bundle", "--web", str(web), "--out", self.work.name])
        result = json.loads(out.getvalue())
        bundle = Path(result["bundle"])
        self.assertRegex(bundle.name, ru_host.PORTAL_BUNDLE)
        self.assertEqual(ru_host.inspect_bundle(bundle), ([], []))
        with tarfile.open(bundle) as archive:
            names = archive.getnames()
            self.assertEqual({(m.uid, m.uname, m.mode) for m in archive.getmembers() if m.isfile()},
                             {(0, "root", 0o644)}, "readable by the portal user whatever the Mac's umask")
        top = bundle.name[:-len(".tar.gz")]
        self.assertIn(f"{top}/public/robots.txt", names)
        self.assertIn(f"{top}/.next/static/chunks/app.js", names)
        self.assertNotIn(f"{top}/.env", names)
        self.assertNotIn(f"{top}/.next/cache", names)


if __name__ == "__main__":
    unittest.main()
