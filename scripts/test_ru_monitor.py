"""ops/vps/ru/ru_monitor.py: host monitoring and owner alerts of the RU host (LR-17).

Runs against a temporary filesystem root with a fake `systemctl show`, a fake statvfs,
a fixed clock and a fake Telegram, so nothing here needs root or systemd. The units
that call it are checked in test_ru_host.py (RuKitTest).
"""
from __future__ import annotations

import contextlib
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
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


class FakeHost:
    """systemd units as `systemctl show` reports them, a disk and a clock."""

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
        self.free = 0.5
        self.clock = T0
        self.uptime = 3600.0
        self.calls = []

    def run(self, args):
        self.calls.append(args)
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
        self.host = FakeHost()
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
        return ru_monitor.Monitor(ru_monitor.Config(self.env), self.state_dir, root=self.root, run=self.host.run,
                                  now=lambda: self.host.clock, monotonic=lambda: 10 + self.host.uptime,
                                  statvfs=self.host.statvfs, opener=self.telegram, stderr=self.journal,
                                  hostname="ru-1")

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

    def test_units(self):
        units = self.host.units
        units["omoba-portal-ru.service"].update(ActiveState="activating", SubState="auto-restart", NRestarts="14")
        units["omoba-account-api-ru.service"].update(ActiveState="inactive", UnitFileState="disabled")
        units["postgresql.service"].update(ActiveState="failed", SubState="failed")
        units["omoba-backup-ru.service"].update(ActiveState="failed", Result="exit-code")
        units["omoba-backup-check-ru.timer"].update(UnitFileState="disabled", ActiveState="inactive")
        del units["omoba-restore-drill-ru.timer"]
        report = self.check()
        self.assertEqual(report["codes"], ["unit:omoba-portal-ru.service", "unit:postgresql.service",
                                           "unit:omoba-backup-ru.service", "unit:omoba-restore-drill-ru.timer",
                                           "unit:omoba-backup-check-ru.timer"],
                         "a disabled RU service has no release yet and is not a problem")
        problems = "\n".join(report["problems"])
        for needle in ("omoba-portal-ru.service is not running (activating/auto-restart, 14 restarts",
                       "omoba-backup-ru.service: the last run failed (exit-code",
                       "omoba-restore-drill-ru.timer is not installed: no monthly restore drill runs",
                       "omoba-backup-check-ru.timer is disabled, inactive: a missing or failed backup is no longer "
                       "reported (systemctl enable --now omoba-backup-check-ru.timer)"):
            self.assertIn(needle, problems)

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
