"""ops/vps/ru/ru_monitor.py: host monitoring and owner alerts of the RU host (LR-17).

Runs against a temporary filesystem root with a fake `systemctl show`, a fake statvfs,
a fixed clock and a fake Telegram, so nothing here needs root or systemd. The units
that call it are checked in test_ru_host.py (RuKitTest).
"""
from __future__ import annotations

import contextlib
import http.client
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
from pathlib import Path
from types import SimpleNamespace
from urllib.parse import parse_qs

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "ops/vps/ru"))

import ru_monitor  # noqa: E402

T0 = 1_790_000_000.0
TOKEN = "123456:" + "B" * 30
BOT = f"OMOBA_TELEGRAM_BOT_TOKEN={TOKEN}\nOMOBA_TELEGRAM_CHAT_ID=-1001\n"
LOBBY = ru_monitor.LOBBY
CLUSTER = "postgresql@16-main.service"
PG_ISREADY = ["pg_isready", "-h", "127.0.0.1", "-p", "5432", "-t", "10"]


class FakeHost:
    """systemd units as `systemctl show` reports them, pg_isready, a disk and a clock."""

    def __init__(self):
        healthy = {"LoadState": "loaded", "ActiveState": "active", "SubState": "running",
                   "UnitFileState": "enabled", "Result": "success", "NRestarts": "0", "Type": "simple",
                   "ActiveEnterTimestampMonotonic": str(10 * 10**6)}
        self.units = {name: dict(healthy) for name in (*ru_monitor.SERVICES, *ru_monitor.SYSTEM_SERVICES)}
        for name in ru_monitor.JOBS:
            self.units[name] = {**healthy, "ActiveState": "inactive", "SubState": "dead", "Type": "oneshot",
                                "UnitFileState": "static"}
        for name in ru_monitor.TIMERS:
            self.units[name] = {**healthy, "SubState": "waiting", "Type": ""}
        # Ubuntu 24.04: postgresql.service is postgresql-common's wrapper, a oneshot that stays
        # active/exited whatever its clusters do; the server is the cluster's own unit.
        self.units["postgresql.service"] = {**healthy, "SubState": "exited", "Type": "oneshot"}
        self.units[CLUSTER] = {**healthy, "Type": "forking"}
        self.pg_ready = 0
        self.free = 0.5
        self.clock = T0
        self.uptime = 3600.0
        self.calls = []

    def run(self, args):
        self.calls.append(args)
        if args[0] == "pg_isready":
            assert args == PG_ISREADY, args
            return subprocess.CompletedProcess(args, self.pg_ready, "", "")
        assert args[:2] == ["systemctl", "show"] and args[3].startswith("--property="), args
        unit = self.units.get(args[2], {"LoadState": "not-found", "ActiveState": "inactive"})
        return subprocess.CompletedProcess(args, 0, "".join(f"{key}={value}\n" for key, value in unit.items()), "")

    def statvfs(self, path):
        return SimpleNamespace(f_blocks=1000, f_frsize=1 << 20, f_bavail=int(1000 * self.free))


class Telegram:
    def __init__(self):
        self.sent, self.fail = [], None

    def __call__(self, request, timeout=None):
        if self.fail:
            raise self.fail
        self.sent.append(parse_qs(request.data.decode())["text"][0])
        return contextlib.nullcontext()


class MonitorCase(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = self.temporary.name
        self.state_dir = os.path.join(self.root, "var/lib/omoba-ru-monitor")
        os.makedirs(self.state_dir)
        for path in ru_monitor.DISK_PATHS:
            os.makedirs(os.path.join(self.root, path.lstrip("/")), exist_ok=True)
        os.makedirs(self.p(ru_monitor.MATCH_ROOT), exist_ok=True)
        self.cluster("16", "main")
        self.host = FakeHost()
        self.auto_storage = True
        self.health_roots = {ru_monitor.LOBBY_OUTBOX: None}
        self.telegram = Telegram()
        self.journal = io.StringIO()
        self.settings(BOT)

    def tearDown(self):
        self.temporary.cleanup()

    def p(self, path):
        return os.path.join(self.root, path.lstrip("/"))

    def settings(self, text):
        self.env = self.p(ru_monitor.ENV_FILE)
        os.makedirs(os.path.dirname(self.env), exist_ok=True)
        Path(self.env).write_text(text)

    def monitor(self):
        if self.auto_storage:
            for root, epoch in self.health_roots.items():
                try:
                    if not Path(self.p(root)).is_symlink():
                        self.storage_report(root, epoch)
                except OSError:
                    pass  # a deliberately broken outbox is observed by the monitor
        return ru_monitor.Monitor(ru_monitor.Config(self.env), self.state_dir, root=self.root, run=self.host.run,
                                  now=lambda: self.host.clock, monotonic=lambda: 10 + self.host.uptime,
                                  statvfs=self.host.statvfs, opener=self.telegram, stderr=self.journal,
                                  hostname="ru-1")

    def cluster(self, version, name, start=None):
        """A cluster as `apt-get install postgresql` creates it: /etc/postgresql/<version>/<name>/."""
        folder = self.p(f"{ru_monitor.PG_CONF_DIR}/{version}/{name}")
        os.makedirs(folder, exist_ok=True)
        Path(folder, "postgresql.conf").write_text("port = 5432\n")
        if start is not None:
            Path(folder, "start.conf").write_text(f"# Automatic startup configuration\n{start}\n")

    def check(self):
        report = self.monitor().check()
        report.pop("_problems")
        return report

    def worker(self, name, phase="running", age=0.0, **extra):
        folder = self.p(f"{ru_monitor.MATCH_ROOT}/{name}")
        os.makedirs(folder, exist_ok=True)
        status = {"allocation_id": name, "server_epoch": 1, "phase": phase,
                  "heartbeat_ms": int((self.host.clock - age) * 1000), "result_id": None, **extra}
        Path(folder, "status.json").write_text(json.dumps(status))
        self.health_roots[f"{ru_monitor.MATCH_ROOT}/{name}/outbox"] = status["server_epoch"]

    def storage_report(self, outbox=ru_monitor.LOBBY_OUTBOX, epoch=None, **changes):
        now = int(self.host.clock * 1000)
        report = {"version": 2, "updated_ms": now, "server_epoch": epoch,
                  "probe_checked_ms": now, "sql_ready": True, "journal_ready": True,
                  "staged_dodges": 0, "legacy_floors": 0, "unpersisted_results": 0,
                  "critical_replies": 0, "reply_receiver_closed": False,
                  "blocked_since_ms": None, **changes}
        path = Path(self.p(outbox)) / "health/status.json"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(report))
        return path


class StorageHealthTest(MonitorCase):
    def setUp(self):
        super().setUp()
        self.auto_storage = False
        self.path = self.storage_report()

    def storage(self):
        report = self.check()
        return report, report["facts"]["career_storage"]

    def test_required_lobby_report_and_readiness_are_checked_before_any_receipt(self):
        report, facts = self.storage()
        self.assertTrue(report["ok"])
        self.assertEqual(facts["checked"], 1)
        self.path.unlink()
        report, facts = self.storage()
        self.assertIn("career:storage", report["codes"])
        self.assertEqual(facts["missing"], 1)
        self.assertEqual(report["facts"]["settlements"]["scanned"], 0)
        self.assertEqual(report["facts"]["queue_dodges"]["pending"], 0)

    def test_every_pre_persistence_blocker_is_visible_after_two_minutes(self):
        now = int(self.host.clock * 1000)
        for changes in ({"sql_ready": False}, {"journal_ready": False}, {"staged_dodges": 1},
                        {"legacy_floors": 1}, {"unpersisted_results": 1},
                        {"critical_replies": 1}, {"reply_receiver_closed": True}):
            self.storage_report(**changes, blocked_since_ms=now - 119000)
            self.assertNotIn("career:storage", self.storage()[0]["codes"])
            self.storage_report(**changes, blocked_since_ms=now - 120000)
            report, facts = self.storage()
            self.assertIn("career:storage", report["codes"], changes)
            self.assertEqual(facts["persistent"], 1)
            self.assertEqual(facts["critical_replies"], changes.get("critical_replies", 0))
            self.assertEqual(facts["closed_reply_receivers"], int(changes.get("reply_receiver_closed", False)))

    def test_stale_report_or_probe_and_wrong_worker_epoch_are_actionable(self):
        now = int(self.host.clock * 1000)
        self.storage_report(updated_ms=now - 61000, probe_checked_ms=now - 61000)
        self.assertEqual(self.storage()[1]["stale"], 1)
        self.storage_report(probe_checked_ms=now - 31000)
        self.assertEqual(self.storage()[1]["stale"], 1)
        self.storage_report()
        name = "a" * 32
        self.worker(name)
        root = f"{ru_monitor.MATCH_ROOT}/{name}/outbox"
        self.storage_report(root, 2)
        self.assertEqual(self.storage()[1]["unreadable"], 1)
        self.storage_report(root, 1)
        self.assertTrue(self.storage()[0]["ok"])

    def test_worker_startup_grace_and_terminal_workers_do_not_false_alert(self):
        name = "b" * 32
        self.worker(name)
        manifest = Path(self.p(f"{ru_monitor.MATCH_ROOT}/{name}/manifest.json"))
        manifest.write_text(json.dumps({"join_deadline_ms": int((self.host.clock + 60) * 1000)}))
        self.assertTrue(self.storage()[0]["ok"])
        manifest.write_text(json.dumps({"join_deadline_ms": int((self.host.clock - 121) * 1000)}))
        self.assertEqual(self.storage()[1]["missing"], 1)
        self.worker(name, phase="finished")
        self.assertTrue(self.storage()[0]["ok"])
        self.path.unlink()
        self.host.uptime = 119
        self.assertNotIn("career:storage", self.storage()[0]["codes"])

    def test_corrupt_oversize_future_and_inconsistent_contracts_fail_closed(self):
        valid = json.loads(self.path.read_text())
        for changes in ({"version": True}, {"version": 1}, {"sql_ready": 1}, {"staged_dodges": True},
                        {"critical_replies": 257, "blocked_since_ms": int(self.host.clock * 1000)},
                        {"updated_ms": int((self.host.clock + 30) * 1000)},
                        {"probe_checked_ms": None}, {"blocked_since_ms": 0},
                        {"sql_ready": False}, {"private_key": "never disclose"}):
            self.path.write_text(json.dumps({**valid, **changes}))
            report, facts = self.storage()
            self.assertEqual(facts["unreadable"], 1, changes)
            self.assertNotIn("never disclose", json.dumps(report))
        for raw in ("{bad", "x" * 4097):
            self.path.write_text(raw)
            self.assertEqual(self.storage()[1]["unreadable"], 1)
        self.storage_report()
        self.worker("c" * 32, phase="unknown")
        self.assertEqual(self.storage()[1]["unreadable"], 1)

    def test_links_and_fifo_never_escape_or_block_the_health_read(self):
        outside = Path(self.root) / "outside"
        outside.write_text(self.path.read_text())
        self.path.unlink()
        self.path.symlink_to(outside)
        self.assertEqual(self.storage()[1]["unreadable"], 1)
        self.path.unlink()
        os.mkfifo(self.path)
        self.assertEqual(self.storage()[1]["unreadable"], 1)

    def test_worker_scan_limit_is_not_a_healthy_result(self):
        for letter in "ab":
            self.worker(letter * 32)
            self.storage_report(f"{ru_monitor.MATCH_ROOT}/{letter * 32}/outbox", 1)
        with mock.patch.object(ru_monitor, "MAX_WORKER_DIRS", 1):
            report, facts = self.storage()
        self.assertIn("career:storage", report["codes"])
        self.assertTrue(facts["scan_limited"])

    def test_resolution_reuses_existing_alert_state_without_identifiers(self):
        self.path.unlink()
        self.monitor().alert_check()
        self.assertEqual(len(self.telegram.sent), 1)
        self.assertNotIn("profile", self.telegram.sent[0])
        self.storage_report()
        self.monitor().alert_check()
        self.host.clock += 16 * 60
        self.storage_report()
        self.monitor().alert_check()
        self.assertEqual(len(self.telegram.sent), 2)
        self.assertIn("career storage availability: ok again", self.telegram.sent[-1])


class ConfigTest(MonitorCase):
    def test_defaults_bot_and_bad_values(self):
        cfg = ru_monitor.Config(self.env)
        self.assertEqual((cfg.problems, cfg.telegram()), ([], (TOKEN, "-1001")))
        self.assertEqual(cfg.number("OMOBA_ALERT_DISK_MIN_FREE_PERCENT"), 15)
        example = ru_monitor.Config(values=ru_monitor.parse_env_text(
            (ROOT / "ops/vps/ru/alerts.env.example").read_text())[0])
        self.assertEqual((example.problems, example.telegram()), ([], ("", "")), "placeholders: the journal only")
        self.settings("OMOBA_ALERT_DISK_MIN_FREE_PERCENT=0\nOMOBA_TELEGRAM_BOT_TOKEN=1:x\nOMOBA_ALERT_DISKS=1\n")
        cfg = ru_monitor.Config(self.env)
        self.assertEqual(cfg.number("OMOBA_ALERT_DISK_MIN_FREE_PERCENT"), 15, "a bad value falls back")
        joined = "\n".join(cfg.problems)
        for needle in ("unknown setting OMOBA_ALERT_DISKS", "OMOBA_ALERT_DISK_MIN_FREE_PERCENT must be",
                       "set both OMOBA_TELEGRAM_BOT_TOKEN and OMOBA_TELEGRAM_CHAT_ID"):
            self.assertIn(needle, joined)
        missing = ru_monitor.Config(self.env + ".gone")
        self.assertIn("does not exist", missing.problems[-1])


class CheckTest(MonitorCase):
    def test_a_healthy_host_passes(self):
        report = self.check()
        self.assertTrue(report["ok"], report["problems"])
        self.assertEqual(list(report["facts"]["disk"]), ["/"], "one entry per filesystem")
        self.assertEqual(report["warnings"], [])
        self.assertEqual((report["facts"]["postgres"]["clusters"], report["facts"]["postgres"]["accepting"]),
                         (["16-main"], True))

    def test_units(self):
        units = self.host.units
        units["omoba-portal-ru.service"].update(ActiveState="activating", SubState="auto-restart", NRestarts="14")
        units["omoba-account-api-ru.service"].update(ActiveState="inactive", UnitFileState="disabled")
        units["caddy.service"].update(ActiveState="failed", SubState="failed")
        # A crashed cluster: its own unit failed, the wrapper still active/exited.
        units[CLUSTER].update(ActiveState="failed", SubState="failed", Result="signal")
        units["omoba-backup-ru.service"].update(ActiveState="failed", Result="exit-code")
        units["omoba-backup-check-ru.timer"].update(UnitFileState="disabled", ActiveState="inactive")
        del units["omoba-restore-drill-ru.timer"]
        report = self.check()
        self.assertEqual(report["codes"], ["unit:omoba-portal-ru.service", "unit:caddy.service",
                                           "unit:omoba-backup-ru.service", "unit:omoba-restore-drill-ru.timer",
                                           "unit:omoba-backup-check-ru.timer", f"unit:{CLUSTER}"],
                         "a disabled RU service has no release yet and is not a problem")
        problems = "\n".join(report["problems"])
        for needle in ("omoba-portal-ru.service is not running (activating/auto-restart, 14 restarts",
                       f"{CLUSTER} is not running (failed/failed, 0 restarts; journalctl -u {CLUSTER})",
                       "omoba-backup-ru.service: the last run failed (exit-code",
                       "omoba-restore-drill-ru.timer is not installed: no monthly restore drill runs",
                       "omoba-backup-check-ru.timer is disabled, inactive: a missing or failed backup is no longer "
                       "reported (systemctl enable --now omoba-backup-check-ru.timer)"):
            self.assertIn(needle, problems)

    def test_postgres_watches_the_cluster_not_the_wrapper(self):
        """Ubuntu's postgresql.service stays active/exited with its cluster down; the check
        watches postgresql@<version>-<name>.service and then port 5432."""
        units = self.host.units
        units[CLUSTER].update(ActiveState="inactive", SubState="dead")      # pg_ctlcluster 16 main stop
        report = self.check()
        self.assertEqual(report["codes"], [f"unit:{CLUSTER}"])
        self.assertIn(f"{CLUSTER} is not running (inactive/dead", report["problems"][0])
        self.assertNotIn(PG_ISREADY, self.host.calls, "a stopped unit already says why")
        self.assertEqual(units["postgresql.service"]["ActiveState"], "active", "the wrapper hides it")
        # The unit runs but the server takes no connection (recovery, a hung postmaster).
        units[CLUSTER].update(ActiveState="active", SubState="running")
        for code, why in ((2, "no response"), (1, "it rejects connections: starting, stopping or recovering"),
                          (127, "pg_isready did not run")):
            self.host.pg_ready = code
            report = self.check()
            self.assertEqual(report["codes"], ["postgres:connect"])
            self.assertEqual(report["problems"][0], f"PostgreSQL does not accept connections on port 5432 ({why}; "
                                                    f"journalctl -u {CLUSTER})")
        self.host.pg_ready = 0
        # Every cluster that starts at boot is watched; manual, disabled and half-made ones are not.
        self.cluster("17", "main")
        self.cluster("16", "scratch", start="manual")
        self.cluster("16", "old", start="disabled")
        os.makedirs(self.p(f"{ru_monitor.PG_CONF_DIR}/16/empty"))
        units["postgresql@17-main.service"] = {"LoadState": "loaded", "ActiveState": "failed", "SubState": "failed"}
        report = self.check()
        self.assertEqual(report["facts"]["postgres"]["clusters"], ["16-main", "17-main"])
        self.assertEqual(report["codes"], ["unit:postgresql@17-main.service"])
        # No cluster that starts at boot: nothing to watch is itself the problem.
        shutil.rmtree(self.p(ru_monitor.PG_CONF_DIR))
        report = self.check()
        self.assertEqual(report["codes"], ["postgres:cluster"])
        self.assertIn("no PostgreSQL cluster in /etc/postgresql starts at boot", report["problems"][0])

    def test_disk_below_the_threshold(self):
        self.host.free = 0.149
        report = self.check()
        self.assertEqual(report["codes"], ["disk:/"])
        self.assertIn("only 14% free on the filesystem of / (0.1 of 1.0 GiB; alert below 15%)", report["problems"][0])
        self.settings(BOT + "OMOBA_ALERT_DISK_MIN_FREE_PERCENT=10\n")
        self.assertTrue(self.check()["ok"])

    def test_stale_workers(self):
        self.worker("a" * 32, age=2)
        self.worker("b" * 32, phase="forming", age=400)
        self.worker("c" * 32, phase="finished", age=4000)
        self.worker("archive", age=9000)
        os.makedirs(self.p(f"{ru_monitor.MATCH_ROOT}/broken"))
        Path(self.p(f"{ru_monitor.MATCH_ROOT}/broken/status.json")).write_text("{")
        report = self.check()
        self.assertEqual(report["codes"], ["worker:" + "b" * 32])
        self.assertIn("match worker bbbbbbbb (forming) has written no heartbeat for 6 min", report["problems"][0])
        self.assertEqual(report["facts"]["workers"], {"live": 2})
        # Right after a lobby restart the pool adopts or relaunches its workers first.
        self.host.uptime = 60
        self.assertTrue(self.check()["ok"])
        self.host.uptime = 3600
        self.host.units[LOBBY]["ActiveState"] = "inactive"
        self.assertEqual(self.check()["codes"], [f"unit:{LOBBY}"], "a stopped lobby is the problem, not its workers")


class CapacityTest(MonitorCase):
    """LR-09: the lobby's arena report (server/src/capacity.rs) and the capacity alert."""

    def report(self, busy_for=None, age=1.0, **extra):
        clock_ms = int(self.host.clock * 1000)
        report = {"version": 1, "updated_ms": clock_ms - int(age * 1000), "capacity": 2, "occupied": 2,
                  "searching": 5, "waiting_for_arena": 3 if busy_for is not None else 0,
                  "busy_since_ms": None if busy_for is None else clock_ms - int(busy_for * 1000),
                  "longest_wait_secs": int(busy_for or 0), "busy_periods": 1, **extra}
        Path(self.p(ru_monitor.CAPACITY_REPORT)).write_text(json.dumps(report))

    def test_a_long_wait_for_a_free_arena_alerts(self):
        self.assertEqual(self.check()["facts"]["capacity"], {"skipped": "no report (a lobby before LR-09)"})
        self.report()
        report = self.check()
        self.assertTrue(report["ok"], report["problems"])
        self.assertEqual(report["facts"]["capacity"]["busy_seconds"], 0)
        self.report(busy_for=60)
        self.assertTrue(self.check()["ok"], "a short wait is not an alert")
        self.report(busy_for=150)
        report = self.check()
        self.assertEqual(report["codes"], ["capacity:arenas"])
        self.assertEqual(report["problems"][0],
                         "players have waited for a free arena for 2 min: 2 of 2 arenas busy, 3 players waiting "
                         "(the longest 150 s); check CPU and memory, then raise OMOBA_MATCH_CAPACITY "
                         "(README.md, \"Arena capacity\")")
        self.settings(BOT + "OMOBA_ALERT_CAPACITY_WAIT_SECONDS=300\n")
        self.assertTrue(self.check()["ok"])
        self.settings(BOT)
        # A stale report (the lobby stopped writing) or a stopped lobby is no capacity alert.
        self.report(busy_for=900, age=600)
        self.assertEqual(self.check()["facts"]["capacity"], {"skipped": "the report is stale"})
        self.report(busy_for=900)
        self.host.units[LOBBY]["ActiveState"] = "inactive"
        self.assertEqual(self.check()["codes"], [f"unit:{LOBBY}"])
        self.host.units[LOBBY]["ActiveState"] = "active"
        Path(self.p(ru_monitor.CAPACITY_REPORT)).write_text("{")
        self.assertEqual(self.check()["facts"]["capacity"], {"skipped": "the report is unreadable"})

    def test_the_alert_resolves_once_arenas_free(self):
        self.report(busy_for=200)
        monitor = self.monitor()
        first = monitor.alert_check()
        self.assertEqual(len(self.telegram.sent), 1)
        self.assertIn("players have waited for a free arena", self.telegram.sent[0])
        self.assertFalse(first["ok"])
        self.report()
        self.host.clock += 16 * 60
        self.report()
        self.monitor().alert_check()
        self.assertIn("arena capacity: ok again", self.telegram.sent[-1])


class SettlementTest(MonitorCase):
    def receipt(self, root, name, kind, age):
        folder = Path(self.p(root))
        folder.mkdir(parents=True, exist_ok=True)
        path = folder / (name + ".json")
        path.write_text(json.dumps({"kind": kind, "result": {"nickname": "never expose this"}}))
        os.utime(path, (self.host.clock - age, self.host.clock - age))
        return path

    def test_terminal_receipts_age_while_live_checkpoints_do_not_alert(self):
        root = ru_monitor.MATCH_ROOT + "/" + "a" * 32 + "/outbox"
        self.receipt(root, "live", "Checkpoint", 7200)
        self.receipt(ru_monitor.LOBBY_OUTBOX, "new", "Settle", 299)
        self.assertTrue(self.check()["ok"])
        pending = self.receipt(root, "waiting", "Settle", 300)
        report = self.check()
        self.assertEqual(report["codes"], ["settlement:outbox"])
        self.assertEqual(report["facts"]["settlements"]["stale"], 1)
        self.assertNotIn("never expose this", str(report))
        pending.unlink()
        self.assertTrue(self.check()["ok"])

    def test_broken_receipts_and_scan_limits_are_actionable(self):
        path = self.receipt(ru_monitor.LOBBY_OUTBOX, "broken", "Settle", 301)
        path.write_text("{")
        self.assertEqual(self.check()["facts"]["settlements"]["unreadable"], 1)
        path.unlink()
        self.receipt(ru_monitor.LOBBY_OUTBOX, "one", "Start", 900)
        self.receipt(ru_monitor.LOBBY_OUTBOX, "two", "Start", 900)
        with mock.patch.object(ru_monitor, "MAX_OUTBOX_RECORDS", 1):
            report = self.check()
        self.assertEqual(report["codes"], ["settlement:outbox"])
        self.assertTrue(report["facts"]["settlements"]["scan_limited"])

    def test_old_terminal_time_alerts_even_when_retry_just_rewrote_the_file(self):
        path = self.receipt(ru_monitor.LOBBY_OUTBOX, "rewritten", "Settle", 0)
        path.write_text(json.dumps({"kind": "Settle", "result": {
            "ended_at_ms": int((self.host.clock - 301) * 1000),
            "nickname": "never expose this",
        }}))
        os.utime(path, (self.host.clock, self.host.clock))
        report = self.check()
        self.assertEqual(report["codes"], ["settlement:outbox"])
        self.assertEqual(report["facts"]["settlements"]["oldest_seconds"], 301)
        self.assertNotIn("never expose this", str(report))

    def test_terminal_times_reject_malformed_and_future_values(self):
        for ended in [True, -1, "old", 1.5, 2**64, int((self.host.clock + 30) * 1000)]:
            path = self.receipt(ru_monitor.LOBBY_OUTBOX, "invalid-time", "Settle", 0)
            path.write_text(json.dumps({"kind": "Settle", "result": {"ended_at_ms": ended}}))
            self.assertEqual(self.check()["facts"]["settlements"]["unreadable"], 1, ended)
            path.unlink()

    def test_receipts_symlinked_outside_the_outbox_are_not_read(self):
        path = self.receipt(ru_monitor.LOBBY_OUTBOX, "live", "Start", 900)
        outside = Path(self.root) / "outside.json"
        path.rename(outside)
        path.symlink_to(outside)
        self.assertTrue(self.check()["ok"])
        self.assertEqual(self.check()["facts"]["settlements"]["scanned"], 0)


class QueueDodgeTest(MonitorCase):
    def event(self, age=0, root=None, event_id="a" * 32):
        folder = Path(self.p(root or ru_monitor.LOBBY_OUTBOX)) / "queue-dodges"
        folder.mkdir(parents=True, exist_ok=True)
        path = folder / (event_id + ".json")
        event = {"version": 1, "event_id": event_id, "profile_id": "b" * 64,
                 "observed_at_ms": int((self.host.clock - age) * 1000)}
        path.write_text(json.dumps(event))
        os.utime(path, (self.host.clock, self.host.clock))
        return path, event

    def test_new_journal_events_and_native_lock_temp_files_are_healthy(self):
        path, _ = self.event(299)
        (path.parent / ".lock").touch()
        (path.parent / ".pending.tmp").write_text("incomplete write")
        report = self.check()
        self.assertTrue(report["ok"], report["problems"])
        self.assertEqual(report["facts"]["queue_dodges"]["pending"], 1)
        self.assertEqual(report["facts"]["queue_dodges"]["stale"], 0)
        self.assertEqual(report["facts"]["settlements"]["scanned"], 0)

    def test_old_lobby_and_worker_events_alert_without_exposing_identities(self):
        self.event(301, event_id="1" * 32)
        self.event(600, root=ru_monitor.MATCH_ROOT + "/" + "c" * 32 + "/outbox", event_id="2" * 32)
        report = self.check()
        self.assertEqual(report["codes"], ["queue-dodge:journal"])
        facts = report["facts"]["queue_dodges"]
        self.assertEqual((facts["pending"], facts["stale"], facts["oldest_seconds"]), (2, 2, 600))
        self.assertEqual(report["facts"]["settlements"]["stale"], 0)
        for identity in ["b" * 64, "1" * 32, "2" * 32]:
            self.assertNotIn(identity, str(report))

    def test_acknowledged_journal_removal_resolves_the_existing_alert(self):
        path, _ = self.event(301)
        self.monitor().alert_check()
        self.assertEqual(len(self.telegram.sent), 1)
        self.assertIn("matchmaking penalty delivery needs attention", self.telegram.sent[0])
        path.unlink()
        self.host.clock += 16 * 60
        self.monitor().alert_check()
        self.assertIn("matchmaking penalty delivery: ok again", self.telegram.sent[-1])
        self.assertNotIn("b" * 64, " ".join(self.telegram.sent))

    def test_corrupt_future_and_unknown_fields_are_actionable(self):
        path, valid = self.event()
        cases = [None, [], {**valid, "version": 2}, {**valid, "version": True},
                 {**valid, "version": 1.0}, {**valid, "profile_id": "B" * 64},
                 {**valid, "event_id": "f" * 32}, {**valid, "observed_at_ms": -1},
                 {**valid, "observed_at_ms": True}, {**valid, "observed_at_ms": 2**63},
                 {**valid, "observed_at_ms": int((self.host.clock + 30) * 1000)},
                 {**valid, "private": TOKEN}]
        for value in cases:
            path.write_text(json.dumps(value))
            report = self.check()
            self.assertEqual(report["codes"], ["queue-dodge:journal"], value)
            self.assertEqual(report["facts"]["queue_dodges"]["unreadable"], 1, value)
            self.assertNotIn(TOKEN, str(report))
        path.write_text("{")
        self.assertEqual(self.check()["facts"]["queue_dodges"]["unreadable"], 1)
        path.write_bytes(b" " * (ru_monitor.MAX_QUEUE_EVENT_BYTES + 1))
        with mock.patch.object(ru_monitor.json, "loads", side_effect=AssertionError("oversized JSON parsed")):
            _, facts = self.monitor().queue_dodge_problems()
        self.assertEqual(facts["unreadable"], 1)

    def test_leaf_and_root_symlinks_are_not_followed(self):
        path, _ = self.event()
        outside = Path(self.root) / "outside.json"
        path.rename(outside)
        path.symlink_to(outside)
        with self.assertRaises(OSError):
            ru_monitor.read_regular_json(path, ru_monitor.MAX_QUEUE_EVENT_BYTES)
        with mock.patch.object(ru_monitor, "read_regular_json", side_effect=AssertionError("link followed")):
            _, facts = self.monitor().queue_dodge_problems()
            self.assertEqual(facts["unreadable"], 1)
        path.unlink()
        folder = path.parent
        folder.rmdir()
        target = Path(self.root) / "outside-journal"
        target.mkdir()
        folder.symlink_to(target, target_is_directory=True)
        self.assertEqual(self.check()["facts"]["queue_dodges"]["unreadable"], 1)

    def test_ignored_temp_entries_still_count_toward_the_bounded_scan(self):
        path, _ = self.event()
        path.unlink()
        for name in [".one.tmp", ".two.tmp"]:
            (path.parent / name).write_text("pending")
        with mock.patch.object(ru_monitor, "MAX_QUEUE_JOURNAL_ENTRIES", 1):
            report = self.check()
        self.assertEqual(report["codes"], ["queue-dodge:journal"])
        self.assertTrue(report["facts"]["queue_dodges"]["scan_limited"])
        self.assertEqual(report["facts"]["queue_dodges"]["scanned"], 1)

    def test_native_journal_capacity_limits_alert_before_any_event_is_old(self):
        path, _ = self.event(event_id="1" * 32)
        other, _ = self.event(event_id="2" * 32)
        with mock.patch.object(ru_monitor, "MAX_QUEUE_EVENTS_PER_JOURNAL", 1):
            report = self.check()
        self.assertEqual(report["codes"], ["queue-dodge:journal"])
        self.assertTrue(report["facts"]["queue_dodges"]["capacity_exceeded"])
        path.unlink()
        other.unlink()
        for name in [".one.tmp", ".two.tmp"]:
            (path.parent / name).touch()
        with mock.patch.object(ru_monitor, "MAX_QUEUE_ENTRIES_PER_JOURNAL", 1):
            report = self.check()
        self.assertTrue(report["facts"]["queue_dodges"]["capacity_exceeded"])

    def test_unexpected_entries_and_non_regular_files_block_journal_health(self):
        path, _ = self.event()
        path.unlink()
        for name in [".lock", "note.txt", "d" * 32 + ".json"]:
            entry = path.parent / name
            entry.mkdir()
            self.assertEqual(self.check()["facts"]["queue_dodges"]["unreadable"], 1, name)
            entry.rmdir()
        os.mkfifo(path)
        with self.assertRaises(ValueError):
            ru_monitor.read_regular_json(path, ru_monitor.MAX_QUEUE_EVENT_BYTES)
        self.assertEqual(self.check()["facts"]["queue_dodges"]["unreadable"], 1)


class AlertTest(MonitorCase):
    def alert(self):
        return self.monitor().alert_check()

    def test_alert_at_once_remind_hourly_and_resolve_once(self):
        self.host.free = 0.05
        first = self.alert()
        self.assertEqual(len(self.telegram.sent), 1)
        self.assertEqual(self.telegram.sent[0], "OMOBA RU host ru-1: problems\n- only 5% free on the filesystem of / "
                                                "(0.0 of 1.0 GiB; alert below 15%)")
        self.assertEqual(first["telegram"], {"problems": True, "resolved": False})
        self.host.clock += 300
        self.alert()
        self.assertEqual(len(self.telegram.sent), 1, "no repeat within the hour")
        self.assertIn("<4>only 5% free", self.journal.getvalue())
        self.host.clock += 3300
        self.alert()
        self.assertEqual(len(self.telegram.sent), 2)
        self.assertIn("(since 2026-09-21 14:13 UTC)", self.telegram.sent[1])
        self.host.free = 0.5
        self.host.clock += 300
        self.alert()
        self.assertEqual(len(self.telegram.sent), 2, "resolved only after 15 minutes without it")
        self.host.clock += 900
        resolved = self.alert()
        self.assertEqual(resolved["resolved"], ["free disk on /: ok again (first seen 2026-09-21 14:13 UTC)"])
        self.assertEqual(self.telegram.sent[-1], "OMOBA RU host ru-1: resolved\n- free disk on /: ok again "
                                                 "(first seen 2026-09-21 14:13 UTC)")
        self.host.clock += 300
        self.alert()
        self.assertEqual(len(self.telegram.sent), 3, "said once")
        self.assertNotIn(TOKEN, self.journal.getvalue())

    def test_a_failed_timer_job_reminds_daily(self):
        self.host.units["omoba-backup-check-ru.service"].update(ActiveState="failed", Result="exit-code")
        for _ in range(24 * 12):        # a day of checks every 5 minutes
            self.alert()
            self.host.clock += 300
        self.assertEqual(len(self.telegram.sent), 1, "not again within 24 h")
        self.alert()
        self.assertEqual(len(self.telegram.sent), 2, "the daily reminder")

    def test_without_a_bot_alerts_stay_in_the_journal(self):
        self.settings("OMOBA_ALERT_REPEAT_MINUTES=60\n")
        self.host.free = 0.05
        report = self.alert()
        self.assertEqual((self.telegram.sent, report["telegram"]["problems"]), ([], False))
        self.assertIn("<3>OMOBA RU host ru-1: problems\n- only 5% free", self.journal.getvalue())
        self.assertIn("no Telegram bot in", report["warnings"][0])

    def test_an_undelivered_alert_is_logged_without_the_token(self):
        self.telegram.fail = OSError(f"https://api.telegram.org/bot{TOKEN}/sendMessage: timed out")
        self.host.free = 0.05
        self.alert()
        journal = self.journal.getvalue()
        self.assertIn("<4>the Telegram alert was not delivered", journal)
        self.assertNotIn(TOKEN, journal)

    def test_a_reply_that_is_not_http_is_an_undelivered_alert(self):
        self.telegram.fail = http.client.BadStatusLine("garbage")
        self.host.free = 0.05
        report = self.alert()
        self.assertFalse(report["telegram"]["problems"])
        self.assertIn("<4>the Telegram alert was not delivered", self.journal.getvalue())

    def test_test_alert(self):
        out = io.StringIO()
        self.assertEqual(ru_monitor.main(["test-alert"], stdout=out, monitor=self.monitor()), 0)
        self.assertEqual(self.telegram.sent, [f"OMOBA RU host ru-1: test alert from {ru_monitor.TOOL}; the bot works"])
        self.settings("")
        self.assertEqual(ru_monitor.main(["test-alert"], stdout=io.StringIO(), monitor=self.monitor()), 1)


class UnitFailedTest(MonitorCase):
    def failed(self, unit=LOBBY, **environ):
        return self.monitor().unit_failed(unit, environ={f"MONITOR_{key.upper()}": value for key, value in environ.items()})

    def test_a_crashing_service_alerts_at_once_then_hourly(self):
        self.host.units[LOBBY].update(ActiveState="activating", SubState="auto-restart", NRestarts="1")
        result = self.failed(service_result="exit-code", exit_code="exited", exit_status="101")
        self.assertTrue(result["alerted"])
        self.assertEqual(self.telegram.sent, [
            "OMOBA RU host ru-1: unit failed\n- omoba-lobby-ru.service failed (exit-code, exited 101); systemd "
            "restarts it, 1 restarts so far (journalctl -u omoba-lobby-ru.service)"])
        for _ in range(30):      # crash-looping every 5 s for a while
            self.host.clock += 5
            self.failed(service_result="signal", exit_code="killed", exit_status="SEGV")
        self.assertEqual(len(self.telegram.sent), 1)
        self.assertIn("<3>omoba-lobby-ru.service failed (signal, killed SEGV)", self.journal.getvalue())
        self.host.clock += 3600
        self.failed(service_result="exit-code")
        self.assertEqual(len(self.telegram.sent), 2)

    def test_a_service_is_resolved_once_it_stays_up(self):
        self.failed(service_result="exit-code")
        self.host.clock += 600
        self.monitor().alert_check()
        self.assertEqual(len(self.telegram.sent), 1, "up for 10 minutes is not yet settled")
        self.host.clock += 600
        self.monitor().alert_check()
        self.assertEqual(self.telegram.sent[-1], "OMOBA RU host ru-1: resolved\n- omoba-lobby-ru.service: ok again "
                                                 "(first seen 2026-09-21 14:13 UTC)")

    def test_a_job_failure_reminds_daily_and_odd_values_are_dropped(self):
        job = "omoba-backup-ru.service"
        self.host.units[job].update(ActiveState="failed", Result="exit-code")
        self.failed(job, service_result="exit-code", exit_status="1; rm -rf /")
        self.assertEqual(self.telegram.sent, ["OMOBA RU host ru-1: unit failed\n- omoba-backup-ru.service failed "
                                              "(exit-code); the timer runs it again on schedule "
                                              "(journalctl -u omoba-backup-ru.service)"])
        self.host.clock += 6 * 3600
        self.failed(job, service_result="exit-code")
        self.assertEqual(len(self.telegram.sent), 1)
        # A unit it does not watch itself is settled once it no longer fails.
        other = "omoba-nftables-ru.service"
        self.host.units[other] = {"LoadState": "loaded", "ActiveState": "failed", "Type": "oneshot"}
        self.failed(other)
        self.host.clock += 3600
        self.monitor().alert_check()
        self.assertNotIn("omoba-nftables-ru.service: ok again", "\n".join(self.telegram.sent))
        self.host.units[other]["ActiveState"] = "inactive"
        self.monitor().alert_check()
        self.assertIn("omoba-nftables-ru.service: ok again", self.telegram.sent[-1])

    def test_a_failed_oneshot_without_a_timer_stays_down(self):
        """The per-source limits (LR-16) are a oneshot that stays active; no timer reruns it."""
        limits = "omoba-limits-ru.service"
        self.host.units[limits] = {"LoadState": "loaded", "ActiveState": "failed", "SubState": "failed",
                                   "Result": "exit-code", "NRestarts": "0", "Type": "oneshot",
                                   "RemainAfterExit": "yes"}
        self.failed(limits, service_result="exit-code", exit_code="exited", exit_status="1")
        self.assertEqual(self.telegram.sent, ["OMOBA RU host ru-1: unit failed\n- omoba-limits-ru.service failed "
                                              "(exit-code, exited 1); it stays down, 0 restarts so far "
                                              "(journalctl -u omoba-limits-ru.service)"])
        self.host.units[limits].update(ActiveState="active", SubState="exited", Result="success")
        self.host.clock += 3600
        self.monitor().alert_check()
        self.assertIn("omoba-limits-ru.service: ok again", self.telegram.sent[-1])

    def test_a_crashed_cluster_is_one_alert_from_the_drop_in_and_the_check(self):
        """postgresql-alert-ru.conf gives the cluster unit OnFailure=; the host check reports it
        under the same code, so one crash is one message until the hourly reminder."""
        self.host.units[CLUSTER].update(ActiveState="failed", SubState="failed", Result="signal")
        self.failed(CLUSTER, service_result="signal", exit_code="killed", exit_status="KILL")
        self.assertEqual(self.telegram.sent, [
            f"OMOBA RU host ru-1: unit failed\n- {CLUSTER} failed (signal, killed KILL); it stays down, 0 restarts "
            f"so far (journalctl -u {CLUSTER})"])
        self.host.clock += 300
        report = self.monitor().alert_check()
        self.assertEqual(report["problems"], [f"{CLUSTER} is not running (failed/failed, 0 restarts; "
                                              f"journalctl -u {CLUSTER})"])
        self.assertEqual((report["alerted"], len(self.telegram.sent)), ([], 1))
        self.host.units[CLUSTER].update(ActiveState="active", SubState="running")
        self.host.clock += 300
        self.monitor().alert_check()
        self.assertEqual(len(self.telegram.sent), 1, "resolved only after 15 minutes without it")
        self.host.clock += 900
        self.monitor().alert_check()
        self.assertEqual(self.telegram.sent[-1], f"OMOBA RU host ru-1: resolved\n- {CLUSTER}: ok again "
                                                 "(first seen 2026-09-21 14:13 UTC)")

    def test_refusals(self):
        with self.assertRaises(ru_monitor.MonitorError):
            self.failed("../../etc/passwd")
        with contextlib.redirect_stderr(io.StringIO()) as err:
            code = ru_monitor.main(["--state", os.path.join(self.root, "nowhere"), "--env-file", self.env,
                                    "unit-failed", LOBBY])
        self.assertEqual(code, 2)
        self.assertIn("no state folder", err.getvalue())


class CommandLineTest(MonitorCase):
    def test_check_exit_codes_and_json(self):
        out = io.StringIO()
        self.assertEqual(ru_monitor.main(["check", "--json"], stdout=out, monitor=self.monitor()), 0)
        report = json.loads(out.getvalue())
        self.assertEqual((report["ok"], report["codes"]), (True, []))
        self.host.free = 0.05
        out = io.StringIO()
        self.assertEqual(ru_monitor.main(["check"], stdout=out, monitor=self.monitor()), 1)
        self.assertTrue(out.getvalue().startswith("PROBLEMS\n  problem: only 5% free"))
        out = io.StringIO()
        self.assertEqual(ru_monitor.main(["check", "--alert"], stdout=out, monitor=self.monitor()), 0,
                         "with --alert the unit fails only when the check itself breaks")
        self.assertEqual(json.loads(out.getvalue())["alerted"][0][:12], "only 5% free")
        state = json.loads(Path(self.state_dir, "state.json").read_text())
        self.assertEqual(list(state["alerts"]), ["disk:/"])
        self.assertEqual(os.stat(Path(self.state_dir, "state.json")).st_mode & 0o777, 0o600)


if __name__ == "__main__":
    unittest.main()
