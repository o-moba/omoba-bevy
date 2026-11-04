#!/usr/bin/env python3
"""Host monitoring and owner alerts of the RU host (LR-17).

Runs on the RU host as root; `ru_host.py bootstrap` installs it as
/opt/omoba-ru/monitor/ru_monitor.py with its units (README.md, "Monitoring and alerts").
Settings come only from /opt/omoba-ru/alerts.env (root 0600, from alerts.env.example):
the host's one Telegram bot and the alert thresholds.

    ru_monitor.py unit-failed UNIT        # OnFailure= of every RU unit (omoba-alert-ru@.service)
    ru_monitor.py check [--json] [--alert]   # every 5 minutes (omoba-monitor-ru.timer)
    ru_monitor.py test-alert              # one test message: is the bot set up?

`check` finds:
- an RU service that is enabled but not running, or Caddy down;
- a PostgreSQL cluster down: its postgresql@<version>-<name>.service not running, or no
  connection accepted on port 5432 (Ubuntu's postgresql.service is only a wrapper that
  stays active whatever its clusters do);
- a failed backup, restore drill or backup check run, and a backup timer that is
  disabled or stopped (then no backup is made and no missing backup is reported);
- free disk below OMOBA_ALERT_DISK_MIN_FREE_PERCENT (15) on any filesystem the host
  writes to;
- a live match worker whose heartbeat (status.json) is older than
  OMOBA_ALERT_WORKER_STALE_SECONDS (300) while the lobby runs;
- players waiting for a free arena for OMOBA_ALERT_CAPACITY_WAIT_SECONDS (120) or longer:
  every arena busy while a ready roster waits, as the lobby reports it in
  <match root>/capacity.json (LR-09).

Every problem goes to the journal; with the bot, also to Telegram: at once, then a
reminder every OMOBA_ALERT_REPEAT_MINUTES (60) while it lasts (every
OMOBA_ALERT_JOB_REPEAT_HOURS, 24, for a timer job, whose tool reports details itself),
and one "resolved" message once it has not been seen for OMOBA_ALERT_RECOVERY_MINUTES
(15). Messages carry unit names, paths, numbers and allocation ids only: never a log
line, an address or a secret.
"""
from __future__ import annotations

import argparse
import contextlib
import datetime as dt
import fcntl
import http.client
import json
import os
import re
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

TOOL = "ops/vps/ru/ru_monitor.py"
MARKET = "ru"
ENV_FILE = "/opt/omoba-ru/alerts.env"
PLACEHOLDER = "CHANGE_ME"
STATE_DIR = "/var/lib/omoba-ru-monitor"            # StateDirectory= of its units
MATCH_ROOT = "/var/lib/omoba-ru/matches"           # OMOBA_MATCH_ROOT of the lobby unit
CAPACITY_REPORT = MATCH_ROOT + "/capacity.json"    # the lobby's arena report (server/src/capacity.rs)
CAPACITY_FRESH_SECONDS = 120                        # the lobby rewrites it every 5 s
SAFE_PATH = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"

# The RU host's units (ru_host.py checks these lists against its own kit).
LOBBY = "omoba-lobby-ru.service"
SERVICES = (LOBBY, "omoba-account-api-ru.service", "omoba-portal-ru.service")
SYSTEM_SERVICES = ("caddy.service",)
# PostgreSQL: Ubuntu's postgresql.service is postgresql-common's wrapper (Type=oneshot,
# ExecStart=/bin/true, RemainAfterExit=yes), active/exited even with every cluster down.
# Each cluster runs as postgresql@<version>-<name>.service; the ones that start at boot are
# /etc/postgresql/<version>/<name>/ with start.conf "auto" or none (pg_lsclusters).
PG_CONF_DIR = "/etc/postgresql"
PG_ADDRESS = ("127.0.0.1", 5432)                    # DB_HOST, DB_PORT of ru_host.py
PG_ISREADY = {1: "it rejects connections: starting, stopping or recovering", 2: "no response",
              3: "pg_isready made no attempt"}
JOBS = ("omoba-backup-ru.service", "omoba-restore-drill-ru.service", "omoba-backup-check-ru.service")
TIMERS = {"omoba-backup-ru.timer": "no nightly backup is made",
          "omoba-restore-drill-ru.timer": "no monthly restore drill runs",
          "omoba-backup-check-ru.timer": "a missing or failed backup is no longer reported"}
# Every filesystem the host writes to; one problem per filesystem.
DISK_PATHS = ("/", "/var/lib/postgresql", "/var/lib/omoba-ru", "/var/backups/omoba-ru", "/var/log", "/opt/omoba-ru")
# match_allocation.rs Phase: a worker in these phases writes a heartbeat every second or so.
LIVE_PHASES = ("ready", "forming", "running", "settling", "recovering")
MAX_WORKER_DIRS = 4096
UNIT_PROPERTIES = ("LoadState", "ActiveState", "SubState", "UnitFileState", "Result", "NRestarts", "Type",
                   "RemainAfterExit", "ActiveEnterTimestampMonotonic")
UNIT_NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9:_.@\\-]{0,200}\.(?:service|timer|socket|mount|target|path)")
MONITOR_VALUE = re.compile(r"[A-Za-z0-9_+-]{1,40}")
ALLOCATION = re.compile(r"[0-9a-f]{32}")
DEFAULTS = {
    "OMOBA_ALERT_DISK_MIN_FREE_PERCENT": "15",
    "OMOBA_ALERT_WORKER_STALE_SECONDS": "300",
    "OMOBA_ALERT_CAPACITY_WAIT_SECONDS": "120",
    "OMOBA_ALERT_REPEAT_MINUTES": "60",
    "OMOBA_ALERT_JOB_REPEAT_HOURS": "24",
    "OMOBA_ALERT_RECOVERY_MINUTES": "15",
}
LIMITS = {"OMOBA_ALERT_DISK_MIN_FREE_PERCENT": (1, 90), "OMOBA_ALERT_WORKER_STALE_SECONDS": (120, 86400),
          "OMOBA_ALERT_CAPACITY_WAIT_SECONDS": (30, 86400),
          "OMOBA_ALERT_REPEAT_MINUTES": (5, 10080), "OMOBA_ALERT_JOB_REPEAT_HOURS": (1, 720),
          "OMOBA_ALERT_RECOVERY_MINUTES": (5, 1440)}
TELEGRAM_KEYS = ("OMOBA_TELEGRAM_BOT_TOKEN", "OMOBA_TELEGRAM_CHAT_ID")
KNOWN_KEYS = frozenset(DEFAULTS) | frozenset(TELEGRAM_KEYS)
LOCK_WAIT_SECONDS = 30


class MonitorError(Exception):
    """A step failed; the message is safe to print."""


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
                raise MonitorError(f"cannot read {path}: run as root") from None
            values, self.problems = parse_env_text(text)
        self.values = values
        for key, (low, high) in LIMITS.items():
            raw = self.values.get(key, DEFAULTS[key])
            if not re.fullmatch(r"\d{1,6}", raw) or not low <= int(raw) <= high:
                self.problems.append(f"{key} must be a whole number from {low} to {high}; using {DEFAULTS[key]}")
        if not self.exists:
            self.problems.append(f"{path} does not exist (ru_host.py bootstrap writes it from alerts.env.example)")
        token, chat = (self.values.get(key, "") for key in TELEGRAM_KEYS)
        if bool(token and PLACEHOLDER not in token) != bool(chat and PLACEHOLDER not in chat):
            self.problems.append("set both OMOBA_TELEGRAM_BOT_TOKEN and OMOBA_TELEGRAM_CHAT_ID, or neither")

    def get(self, key: str) -> str:
        value = self.values.get(key, DEFAULTS.get(key, ""))
        return "" if PLACEHOLDER in value else value

    def number(self, key: str) -> int:
        raw = self.get(key)
        low, high = LIMITS[key]
        return int(raw) if re.fullmatch(r"\d{1,6}", raw) and low <= int(raw) <= high else int(DEFAULTS[key])

    def telegram(self) -> tuple[str, str]:
        token, chat = (self.get(key) for key in TELEGRAM_KEYS)
        return (token, chat) if token and chat else ("", "")

    def secrets(self) -> list:
        token = self.get("OMOBA_TELEGRAM_BOT_TOKEN")
        return [token] if token else []


def redact(text: str, hidden=()) -> str:
    for value in hidden:
        if value and len(value) >= 4:
            text = text.replace(value, "<redacted>")
    return re.sub(r"bot\d+:[A-Za-z0-9_-]{20,}", "bot<redacted>", text)


def clock(moment: float) -> str:
    return dt.datetime.fromtimestamp(moment, dt.timezone.utc).strftime("%Y-%m-%d %H:%M UTC")


def gib(size: int) -> str:
    return f"{size / (1 << 30):.1f}"


def label(code: str) -> str:
    """What a problem code is about, for the "resolved" message."""
    kind, _, subject = code.partition(":")
    return {"unit": subject, "disk": f"free disk on {subject}", "worker": f"match worker {subject[:8]}",
            "postgres": "PostgreSQL", "capacity": "arena capacity"}.get(
        kind, "alerts.env" if kind == "config" else code)


# ---------------------------------------------------------------- the monitor

class Monitor:
    """`root`, `run`, `now`, `monotonic`, `statvfs` and `opener` exist so tests can use a fake
    filesystem, fake systemctl, a fixed clock and a fake Telegram; the host uses the real ones."""

    def __init__(self, cfg: Config, state_dir: str | None = None, root: str = "/", run=None, now=time.time,
                 monotonic=time.monotonic, statvfs=os.statvfs, opener=None, stderr=None, hostname=None):
        self.cfg = cfg
        self.state_dir = state_dir
        self.root = root
        self.run = run or self._run
        self.now = now
        self.monotonic = monotonic
        self.statvfs = statvfs
        self.opener = opener or urllib.request.urlopen
        self.stderr = stderr or sys.stderr
        self.hostname = hostname or socket.gethostname()

    @staticmethod
    def _run(args):
        env = {"PATH": SAFE_PATH, "LC_ALL": "C", "SYSTEMD_PAGER": "", "SYSTEMD_COLORS": "0"}
        try:
            return subprocess.run(args, capture_output=True, text=True, timeout=30, env=env)
        except (FileNotFoundError, subprocess.TimeoutExpired) as error:
            return subprocess.CompletedProcess(args, 127, "", str(error))

    def p(self, path: str) -> str:
        return os.path.join(self.root, path.lstrip("/"))

    def log(self, text: str, priority: int = 6) -> None:
        # systemd reads a leading <N> as the journal priority (3 = error, 4 = warning, 5 = notice).
        print(f"<{priority}>{redact(text, self.cfg.secrets())}", file=self.stderr, flush=True)

    # ---- facts

    def unit(self, name: str) -> dict:
        result = self.run(["systemctl", "show", name, "--property=" + ",".join(UNIT_PROPERTIES)])
        facts = {}
        for line in (result.stdout or "").splitlines():
            key, sep, value = line.partition("=")
            if sep and key in UNIT_PROPERTIES:
                facts[key] = value.strip()
        return facts

    def active_seconds(self, facts: dict) -> float | None:
        try:
            entered = int(facts.get("ActiveEnterTimestampMonotonic") or 0)
        except ValueError:
            return None
        return self.monotonic() - entered / 1e6 if entered else None

    @staticmethod
    def not_running(name: str, unit: dict) -> str:
        return (f"{name} is not running ({unit.get('ActiveState') or 'unknown'}/{unit.get('SubState') or 'unknown'}, "
                f"{unit.get('NRestarts') or 0} restarts; journalctl -u {name})")

    def unit_problems(self) -> tuple[list, dict]:
        """[(code, text, repeat seconds)], facts."""
        repeat, job_repeat = self.cfg.number("OMOBA_ALERT_REPEAT_MINUTES") * 60, \
            self.cfg.number("OMOBA_ALERT_JOB_REPEAT_HOURS") * 3600
        problems, facts = [], {}
        for name in (*SERVICES, *SYSTEM_SERVICES):
            unit = facts[name] = self.unit(name)
            if name in SERVICES and unit.get("UnitFileState") != "enabled":
                continue     # an RU service is enabled once its release is installed (ru_host.py ship)
            if unit.get("ActiveState") != "active":
                problems.append((f"unit:{name}", self.not_running(name, unit), repeat))
        for name in JOBS:
            unit = facts[name] = self.unit(name)
            if unit.get("ActiveState") == "failed":
                problems.append((f"unit:{name}", f"{name}: the last run failed ({unit.get('Result') or 'unknown'}; "
                                                 f"journalctl -u {name})", job_repeat))
        for name, consequence in TIMERS.items():
            unit = facts[name] = self.unit(name)
            if unit.get("UnitFileState") != "enabled" or unit.get("ActiveState") != "active":
                state = "not installed" if unit.get("LoadState") != "loaded" else \
                    f"{unit.get('UnitFileState') or 'unknown'}, {unit.get('ActiveState') or 'unknown'}"
                problems.append((f"unit:{name}", f"{name} is {state}: {consequence} "
                                                 f"(systemctl enable --now {name})", repeat))
        return problems, facts

    def postgres_clusters(self) -> list:
        """The PostgreSQL clusters that start at boot, as `<version>-<name>` (pg_lsclusters)."""
        clusters, root = [], self.p(PG_CONF_DIR)
        try:
            versions = sorted(name for name in os.listdir(root) if re.fullmatch(r"\d{1,3}(?:\.\d{1,3})?", name))
        except OSError:
            return []
        for version in versions:
            try:
                names = sorted(os.listdir(os.path.join(root, version)))
            except OSError:
                continue
            for name in names:
                folder = os.path.join(root, version, name)
                if not re.fullmatch(r"[A-Za-z0-9_.-]{1,64}", name) or \
                        not os.path.isfile(os.path.join(folder, "postgresql.conf")):
                    continue
                try:
                    words = [line.split("#", 1)[0].strip() for line in
                             Path(folder, "start.conf").read_text().splitlines()]
                    start = next((word for word in words if word), "auto")
                except FileNotFoundError:
                    start = "auto"
                except (OSError, UnicodeDecodeError):
                    start = "unreadable"
                if start == "auto":
                    clusters.append(f"{version}-{name}")
        return clusters

    def postgres_problems(self) -> tuple[list, dict]:
        """Each cluster's unit running, then port 5432 accepting connections (pg_isready; it
        does not log in). A connection is tried only while every cluster unit runs: a stopped
        unit already says why."""
        repeat = self.cfg.number("OMOBA_ALERT_REPEAT_MINUTES") * 60
        clusters = self.postgres_clusters()
        facts = {"clusters": clusters}
        if not clusters:
            return [("postgres:cluster", f"no PostgreSQL cluster in {PG_CONF_DIR} starts at boot "
                                         f"(pg_lsclusters)", repeat)], facts
        problems, units = [], [f"postgresql@{cluster}.service" for cluster in clusters]
        for name in units:
            unit = facts[name] = self.unit(name)
            if unit.get("ActiveState") != "active":
                problems.append((f"unit:{name}", self.not_running(name, unit), repeat))
        if problems:
            return problems, facts
        host, port = PG_ADDRESS
        result = self.run(["pg_isready", "-h", host, "-p", str(port), "-t", "10"])
        facts["accepting"] = result.returncode == 0
        if result.returncode != 0:
            problems.append(("postgres:connect", f"PostgreSQL does not accept connections on port {port} "
                                                 f"({PG_ISREADY.get(result.returncode, 'pg_isready did not run')}; "
                                                 f"journalctl -u {' -u '.join(units)})", repeat))
        return problems, facts

    def disk_problems(self) -> tuple[list, dict]:
        threshold = self.cfg.number("OMOBA_ALERT_DISK_MIN_FREE_PERCENT")
        repeat = self.cfg.number("OMOBA_ALERT_REPEAT_MINUTES") * 60
        problems, facts, seen = [], {}, set()
        for path in DISK_PATHS:
            local = self.p(path)
            try:
                device = os.stat(local).st_dev
                info = self.statvfs(local)
            except OSError:
                continue
            if device in seen:
                continue
            seen.add(device)
            total, free = info.f_blocks * info.f_frsize, info.f_bavail * info.f_frsize
            percent = 100.0 * free / total if total else 100.0
            facts[path] = {"free_percent": round(percent, 1), "free_gib": float(gib(free)), "total_gib": float(gib(total))}
            if percent < threshold:
                problems.append((f"disk:{path}", f"only {int(percent)}% free on the filesystem of {path} ({gib(free)} of "
                                                 f"{gib(total)} GiB; alert below {threshold}%)", repeat))
        return problems, facts

    def worker_problems(self, lobby: dict) -> tuple[list, dict]:
        """Live workers whose heartbeat stopped. Only while the lobby has run longer than the
        threshold: after a lobby restart the pool adopts or relaunches its workers first."""
        stale = self.cfg.number("OMOBA_ALERT_WORKER_STALE_SECONDS")
        repeat = self.cfg.number("OMOBA_ALERT_REPEAT_MINUTES") * 60
        facts = {"live": 0}
        up = self.active_seconds(lobby)
        if lobby.get("ActiveState") != "active" or up is None or up < stale:
            facts["skipped"] = "the lobby is not running or started recently"
            return [], facts
        root = self.p(MATCH_ROOT)
        try:
            names = sorted(name for name in os.listdir(root) if name != "archive" and not name.startswith("."))
        except OSError:
            return [], facts
        problems, now_ms = [], self.now() * 1000
        for name in names[:MAX_WORKER_DIRS]:
            path = os.path.join(root, name, "status.json")
            try:
                if os.path.getsize(path) > 65536:
                    continue
                status = json.loads(Path(path).read_text())
            except (OSError, ValueError):
                continue
            if not isinstance(status, dict) or status.get("phase") not in LIVE_PHASES:
                continue
            facts["live"] += 1
            heartbeat = status.get("heartbeat_ms")
            if not isinstance(heartbeat, int) or isinstance(heartbeat, bool):
                continue
            allocation = status.get("allocation_id") if ALLOCATION.fullmatch(str(status.get("allocation_id"))) else None
            label = (allocation or name)[:8]
            age = now_ms - heartbeat
            if age > stale * 1000:
                problems.append((f"worker:{allocation or name}",
                                 f"match worker {label} ({status['phase']}) has written no heartbeat for "
                                 f"{int(age // 60000)} min (journalctl -u {LOBBY})", repeat))
        return problems, facts

    def capacity_problems(self, lobby: dict) -> tuple[list, dict]:
        """Players waiting for a free arena too long (LR-09), from the report the running lobby
        rewrites every 5 s. A missing report (an older lobby) or a stale one (the lobby is
        down or restarting; the unit check says so) is no capacity alert."""
        threshold = self.cfg.number("OMOBA_ALERT_CAPACITY_WAIT_SECONDS")
        repeat = self.cfg.number("OMOBA_ALERT_REPEAT_MINUTES") * 60
        if lobby.get("ActiveState") != "active":
            return [], {"skipped": "the lobby is not running"}
        path = self.p(CAPACITY_REPORT)
        try:
            if os.path.getsize(path) > 65536:
                return [], {"skipped": "the report is too large"}
            report = json.loads(Path(path).read_text())
        except FileNotFoundError:
            return [], {"skipped": "no report (a lobby before LR-09)"}
        except (OSError, ValueError):
            return [], {"skipped": "the report is unreadable"}
        numbers = {key: report.get(key) if isinstance(report, dict) else None
                   for key in ("updated_ms", "capacity", "occupied", "searching", "waiting_for_arena",
                               "busy_since_ms", "longest_wait_secs")}
        numbers = {key: value for key, value in numbers.items()
                   if isinstance(value, int) and not isinstance(value, bool)}
        now_ms = self.now() * 1000
        if "updated_ms" not in numbers or now_ms - numbers["updated_ms"] > CAPACITY_FRESH_SECONDS * 1000:
            return [], {"skipped": "the report is stale"}
        facts = {key: numbers.get(key) for key in ("capacity", "occupied", "searching", "waiting_for_arena",
                                                   "longest_wait_secs")}
        since = numbers.get("busy_since_ms")
        facts["busy_seconds"] = int(max(0, now_ms - since) // 1000) if since is not None else 0
        if since is None or facts["busy_seconds"] < threshold:
            return [], facts
        return [("capacity:arenas",
                 f"players have waited for a free arena for {facts['busy_seconds'] // 60} min: "
                 f"{facts['occupied']} of {facts['capacity']} arenas busy, {facts['waiting_for_arena']} "
                 f"players waiting (the longest {facts['longest_wait_secs']} s); check CPU and memory, then "
                 f"raise OMOBA_MATCH_CAPACITY (README.md, \"Arena capacity\")", repeat)], facts

    def check(self) -> dict:
        problems, warnings, facts = [], [], {}
        for text in self.cfg.problems:
            problems.append(("config", f"{self.cfg.path}: {text}", self.cfg.number("OMOBA_ALERT_REPEAT_MINUTES") * 60))
        more, facts["units"] = self.unit_problems()
        problems += more
        more, facts["postgres"] = self.postgres_problems()
        problems += more
        more, facts["disk"] = self.disk_problems()
        problems += more
        more, facts["workers"] = self.worker_problems(facts["units"].get(LOBBY, {}))
        problems += more
        more, facts["capacity"] = self.capacity_problems(facts["units"].get(LOBBY, {}))
        problems += more
        if not self.cfg.telegram()[0]:
            warnings.append(f"no Telegram bot in {self.cfg.path}: alerts reach the journal only")
        return {"ok": not problems, "problems": [text for _, text, _ in problems],
                "codes": [code for code, _, _ in problems], "warnings": warnings, "facts": facts,
                "_problems": problems}

    # ---- state and alerts

    @contextlib.contextmanager
    def state(self):
        if not self.state_dir or not os.path.isdir(self.state_dir):
            raise MonitorError(f"no state folder {self.state_dir} (StateDirectory= of the unit)")
        descriptor = os.open(os.path.join(self.state_dir, ".lock"), os.O_RDWR | os.O_CREAT, 0o600)
        try:
            deadline = time.monotonic() + LOCK_WAIT_SECONDS
            while True:
                try:
                    fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    break
                except BlockingIOError:
                    if time.monotonic() >= deadline:
                        raise MonitorError("another ru_monitor.py run holds the state lock") from None
                    time.sleep(0.2)
            path = os.path.join(self.state_dir, "state.json")
            try:
                state = json.loads(Path(path).read_text())
                state = state if isinstance(state, dict) else {}
            except (FileNotFoundError, ValueError):
                state = {}
            yield state
            temporary = path + ".next"
            handle = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
            with os.fdopen(handle, "w") as out:
                json.dump(state, out, indent=2, sort_keys=True)
            os.replace(temporary, path)
        finally:
            os.close(descriptor)

    def observe(self, state: dict, problems: list) -> dict:
        """Record [(code, text, repeat)]; return {code: line} of those due for Telegram now."""
        alerts = state.setdefault("alerts", {})
        now, due = self.now(), {}
        for code, text, repeat in problems:
            entry = alerts.setdefault(code, {"first": now})
            entry.update(text=text, last_seen=now)
            sent = entry.get("sent")
            if not isinstance(sent, (int, float)) or now - sent >= repeat:
                entry["sent"] = now
                since = f" (since {clock(entry['first'])})" if now - entry["first"] >= 60 else ""
                due[code] = text + since
        return due

    def settle(self, state: dict, current) -> list:
        """Forget problems not seen for the recovery time; the texts of those that were alerted."""
        alerts = state.setdefault("alerts", {})
        recovery = self.cfg.number("OMOBA_ALERT_RECOVERY_MINUTES") * 60
        now, resolved = self.now(), []
        for code in sorted(alerts):
            entry = alerts[code]
            if code in current or now - entry.get("last_seen", now) < recovery:
                continue
            if code.startswith("unit:") and not self.unit_healthy(code[len("unit:"):]):
                continue
            del alerts[code]
            if entry.get("sent") is not None:
                resolved.append(f"{label(code)}: ok again (first seen {clock(entry.get('first', now))})")
        return resolved

    def unit_healthy(self, name: str) -> bool:
        """A unit an alert named is settled: running, or stopped without failing (a timer job
        between runs), or gone. Failed, restarting or still running a job is not settled."""
        if not UNIT_NAME.fullmatch(name):
            return True
        facts = self.unit(name)
        return facts.get("LoadState") != "loaded" or facts.get("ActiveState") in ("active", "inactive")

    def deliver(self, title: str, lines: list, priority: int) -> bool:
        if not lines:
            return False
        text = f"OMOBA {MARKET.upper()} host {self.hostname}: {title}\n" + "\n".join(f"- {line}" for line in lines)
        self.log(text, priority)
        return self.telegram(text)

    def telegram(self, text: str) -> bool:
        token, chat = self.cfg.telegram()
        if not token:
            return False
        data = urllib.parse.urlencode({"chat_id": chat, "text": redact(text, [token])[:3500],
                                       "disable_web_page_preview": "true"}).encode()
        request = urllib.request.Request(f"https://api.telegram.org/bot{token}/sendMessage", data=data, method="POST")
        try:
            with self.opener(request, timeout=20):
                return True
        except (urllib.error.URLError, OSError, ValueError, http.client.HTTPException) as error:
            self.log(f"the Telegram alert was not delivered: {redact(str(error), [token])}", 4)
            return False

    # ---- commands

    def unit_failed(self, name: str, environ=None) -> dict:
        """OnFailure= of an RU unit: what failed, how, and what systemd does next."""
        if not UNIT_NAME.fullmatch(name or ""):
            raise MonitorError(f"{name!r} is not a unit name")
        environ = os.environ if environ is None else environ
        facts = self.unit(name)
        detail = {key: environ.get(f"MONITOR_{key.upper()}", "") for key in ("service_result", "exit_code", "exit_status")}
        detail = {key: value for key, value in detail.items() if MONITOR_VALUE.fullmatch(value)}
        how = detail.get("service_result") or facts.get("Result") or "failed"
        if detail.get("exit_status"):
            how += f", {detail.get('exit_code') or 'status'} {detail['exit_status']}"
        # A timer job; a oneshot that stays active (the per-source limits, LR-16) has no timer.
        job = facts.get("Type") == "oneshot" and facts.get("RemainAfterExit") != "yes"
        if job:
            text = f"{name} failed ({how}); the timer runs it again on schedule (journalctl -u {name})"
            repeat = self.cfg.number("OMOBA_ALERT_JOB_REPEAT_HOURS") * 3600
        else:
            restarts = facts.get("NRestarts") or "0"
            next_step = "systemd restarts it" if facts.get("SubState") == "auto-restart" or \
                facts.get("ActiveState") in ("activating", "active") else "it stays down"
            text = f"{name} failed ({how}); {next_step}, {restarts} restarts so far (journalctl -u {name})"
            repeat = self.cfg.number("OMOBA_ALERT_REPEAT_MINUTES") * 60
        with self.state() as state:
            due = self.observe(state, [(f"unit:{name}", text, repeat)])
        if not due:
            self.log(text, 3)
        delivered = self.deliver("unit failed", list(due.values()), 3)
        return {"unit": name, "text": text, "alerted": bool(due), "telegram": delivered}

    def alert_check(self) -> dict:
        report = self.check()
        problems = report.pop("_problems")
        with self.state() as state:
            due = self.observe(state, problems)
            resolved = self.settle(state, {code for code, _, _ in problems})
            state["last_check"] = {"at": clock(self.now()), "ok": report["ok"]}
        for code, text, _ in problems:
            if code not in due:
                self.log(text, 4)     # still there; its reminder is not due yet
        report["telegram"] = {"problems": self.deliver("problems", list(due.values()), 3),
                              "resolved": self.deliver("resolved", resolved, 5)}
        report.update(alerted=list(due.values()), resolved=resolved)
        return report


# ---------------------------------------------------------------- command line

def print_report(report: dict, as_json: bool, out) -> None:
    report = {key: value for key, value in report.items() if not key.startswith("_")}
    if as_json:
        print(json.dumps(report, indent=2, sort_keys=True), file=out)
        return
    print("ok" if report["ok"] else "PROBLEMS", file=out)
    for problem in report["problems"]:
        print(f"  problem: {problem}", file=out)
    for warning in report.get("warnings", []):
        print(f"  warning: {warning}", file=out)


def main(argv=None, stdout=None, monitor=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--env-file", default=ENV_FILE, help=f"settings (default {ENV_FILE})")
    parser.add_argument("--state", help=f"state folder (default $STATE_DIRECTORY, else {STATE_DIR})")
    sub = parser.add_subparsers(dest="command", required=True)
    failed = sub.add_parser("unit-failed", help="OnFailure= handler: alert that UNIT failed")
    failed.add_argument("unit")
    check = sub.add_parser("check", help="units, timers, PostgreSQL, disk, match workers and arena capacity; "
                                         "exit 1 on a problem "
                                         "(with --alert: exit 0 once the alerts went out)")
    check.add_argument("--json", action="store_true")
    check.add_argument("--alert", action="store_true", help="alert (journal, Telegram) and remember what was sent")
    sub.add_parser("test-alert", help="send one test message through the configured bot")
    args = parser.parse_args(argv)
    stdout = stdout or sys.stdout
    try:
        if monitor is None:
            state = args.state or (os.environ.get("STATE_DIRECTORY", "").split(":")[0] or STATE_DIR)
            monitor = Monitor(Config(args.env_file), state)
        if args.command == "unit-failed":
            print(json.dumps(monitor.unit_failed(args.unit), sort_keys=True), file=stdout)
            return 0
        if args.command == "test-alert":
            sent = monitor.telegram(f"OMOBA {MARKET.upper()} host {monitor.hostname}: test alert from {TOOL}; "
                                    "the bot works")
            print("sent" if sent else f"NOT sent: check the Telegram lines of {monitor.cfg.path}", file=stdout)
            return 0 if sent else 1
        if args.alert:
            report = monitor.alert_check()
            print(json.dumps({key: report[key] for key in ("ok", "problems", "alerted", "resolved", "telegram")},
                             sort_keys=True), file=stdout)
            return 0
        report = monitor.check()
        print_report(report, args.json, stdout)
        return 0 if report["ok"] else 1
    except MonitorError as error:
        print(f"<3>ERROR: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
