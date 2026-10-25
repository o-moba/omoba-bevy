"""scripts/market_probe.py: the external probe of a market's servers (LR-17).

The UDP compatibility check runs over real loopback sockets against a fake lobby that
answers like `shared::compatibility::response_with_update` (and whose rules are read
from shared/src/compatibility.rs, so the two stay in step); the HTTP checks run against
a real loopback HTTP server. Nothing here leaves the machine or needs cargo.
"""
from __future__ import annotations

import contextlib
import http.server
import io
import json
import os
import re
import socket
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from unittest import mock
from urllib.parse import parse_qs

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import market_probe  # noqa: E402
import markets  # noqa: E402

SERVER = {"handshake": 1, "release": "0.45.0", "protocol": 11, "catalog": "catalog-1",
          "geometry": "geometry-1", "gameplay": "gameplay-1"}
COMPATIBILITY_RS = (ROOT / "shared/src/compatibility.rs").read_text()


def rust_const(name: str) -> int:
    found = re.search(rf"pub const {name}: \w+ = (\d+);", COMPATIBILITY_RS)
    assert found, name
    return int(found.group(1))


class FakeLobby:
    """A UDP endpoint that answers compatibility probes the way the lobby does."""

    def __init__(self, mode: str = "answer"):
        self.mode = mode
        self.requests = []
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.bind(("127.0.0.1", 0))
        self.sock.settimeout(0.05)
        self.port = self.sock.getsockname()[1]
        self.stopped = threading.Event()
        self.thread = threading.Thread(target=self.serve, daemon=True)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *exc):
        self.stopped.set()
        self.thread.join(2)
        self.sock.close()

    def serve(self):
        while not self.stopped.is_set():
            try:
                data, peer = self.sock.recvfrom(4096)
            except socket.timeout:
                continue
            self.requests.append(data)
            for reply in self.replies(data):
                self.sock.sendto(reply, peer)

    def replies(self, data: bytes) -> list:
        """`response_with_update`: only a valid probe gets a report, never larger than the probe."""
        if len(data) > rust_const("MAX_PROBE_BYTES"):
            return []
        try:
            message = json.loads(data)
        except ValueError:
            return []
        nonce, padding = message.get("nonce", ""), message.get("padding", "")
        if message.get("type") != "compatibility_probe" or not re.fullmatch(r"[0-9a-fA-F]{32}", nonce) \
                or not market_probe.valid_contract(message.get("client")) \
                or not rust_const("PROBE_PADDING") <= len(padding) <= 1024:
            return []
        if self.mode == "silent":
            return []
        report = {"type": "compatibility_report", "nonce": nonce, "server": SERVER,
                  "update": {"min_release": "0.45.0", "url": "https://portal.example/download"}}
        encoded = json.dumps(report).encode()
        if len(encoded) > len(data):
            del report["update"]
            encoded = json.dumps(report).encode()
        stale = json.dumps({**report, "nonce": "0" * 32}).encode()
        if self.mode == "stale":
            return [stale]
        if self.mode == "junk-first":
            return [b"{not json", stale, json.dumps({**report, "server": {**SERVER, "catalog": ""}}).encode(), encoded]
        return [encoded]


class Handler(http.server.BaseHTTPRequestHandler):
    answers = {"/health/ready": 200, "/": 200}

    def do_GET(self):  # noqa: N802
        self.send_response(self.answers.get(self.path, 404))
        self.end_headers()

    def log_message(self, *args):
        pass


@contextlib.contextmanager
def http_server(**answers):
    handler = type("H", (Handler,), {"answers": {**Handler.answers, **answers}})
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_address[1]}"
    finally:
        server.shutdown()
        server.server_close()


class WireTest(unittest.TestCase):
    def test_the_probe_follows_the_rust_contract(self):
        self.assertEqual(market_probe.HANDSHAKE_VERSION, rust_const("HANDSHAKE_VERSION"))
        self.assertEqual(market_probe.PROBE_PADDING, rust_const("PROBE_PADDING"))
        self.assertEqual(market_probe.MAX_PROBE_BYTES, rust_const("MAX_PROBE_BYTES"))
        self.assertIn('#[serde(tag = "type", rename_all = "snake_case")]\npub enum CompatibilityDatagram', COMPATIBILITY_RS)
        self.assertRegex(COMPATIBILITY_RS, r"CompatibilityProbe \{\s*nonce: String,\s*client: ReleaseContract,\s*padding: String")
        self.assertRegex(COMPATIBILITY_RS, r"CompatibilityReport \{\s*nonce: String,\s*server: ReleaseContract")
        self.assertIn("(PROBE_PADDING..=1024).contains(&padding.len())", COMPATIBILITY_RS)
        self.assertIn("decode_hex::<16>(&nonce)", COMPATIBILITY_RS)
        fields = re.search(r"pub struct ReleaseContract \{(.*?)\}", COMPATIBILITY_RS, re.S).group(1)
        self.assertEqual(tuple(re.findall(r"pub (\w+):", fields)), market_probe.CONTRACT_KEYS)
        request = market_probe.probe_request("ab" * 16, market_probe.PROBE_CLIENT)
        self.assertLessEqual(len(request), market_probe.MAX_PROBE_BYTES)
        self.assertTrue(market_probe.valid_contract(market_probe.PROBE_CLIENT))
        self.assertTrue(FakeLobby().replies(request), "the lobby's own rules accept the probe")

    def test_only_a_correlated_valid_report_counts(self):
        nonce = "cd" * 16
        good = json.dumps({"type": "compatibility_report", "nonce": nonce, "server": SERVER}).encode()
        self.assertEqual(market_probe.parse_report(good, nonce), {"server": SERVER})
        with_update = json.dumps({"type": "compatibility_report", "nonce": nonce, "server": SERVER,
                                  "update": {"min_release": "0.45.0"}}).encode()
        self.assertEqual(market_probe.parse_report(with_update, nonce)["min_release"], "0.45.0")
        for bad in ({"nonce": "ef" * 16}, {"type": "compatibility_probe"}, {"server": {**SERVER, "protocol": 70000}},
                    {"server": {**SERVER, "handshake": True}}, {"server": {**SERVER, "gameplay": "x" * 65}},
                    {"server": {**SERVER, "catalog": "a b"}}, {"server": "x"}):
            message = {"type": "compatibility_report", "nonce": nonce, "server": SERVER, **bad}
            self.assertIsNone(market_probe.parse_report(json.dumps(message).encode(), nonce), bad)
        self.assertIsNone(market_probe.parse_report(b" " * 2049 + good, nonce))
        self.assertIsNone(market_probe.parse_report(b"\xff", nonce))

    def test_contract_comparison_ignores_the_release_label(self):
        self.assertIsNone(market_probe.contract_issue({**SERVER, "release": "0.45.1"}, SERVER))
        for key, value in (("protocol", 12), ("catalog", "c2"), ("geometry", "g2"), ("gameplay", "p2"),
                           ("handshake", 2)):
            self.assertEqual(market_probe.contract_issue({**SERVER, key: value}, SERVER), key)


class UdpTest(unittest.TestCase):
    def test_a_lobby_that_answers_passes(self):
        with FakeLobby() as lobby:
            result = market_probe.udp_compatibility("127.0.0.1", lobby.port, timeout=2)
        self.assertTrue(result["ok"], result)
        self.assertEqual(result["server"], SERVER)
        self.assertEqual(json.loads(lobby.requests[0])["client"], market_probe.PROBE_CLIENT)

    def test_stale_or_invalid_reports_are_ignored(self):
        with FakeLobby("junk-first") as lobby:
            self.assertTrue(market_probe.udp_compatibility("127.0.0.1", lobby.port, timeout=2)["ok"])
        with FakeLobby("stale") as lobby:
            result = market_probe.udp_compatibility("127.0.0.1", lobby.port, timeout=0.6, interval=0.2)
        self.assertEqual(result, {"ok": False, "error": "no compatibility reply in 0.6 s"})
        self.assertGreaterEqual(len(lobby.requests), 2, "the probe is resent")

    def test_a_silent_or_closed_port_fails(self):
        with FakeLobby("silent") as lobby:
            result = market_probe.udp_compatibility("127.0.0.1", lobby.port, timeout=0.5, interval=0.2)
        self.assertFalse(result["ok"])
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as closed:
            closed.bind(("127.0.0.1", 0))
            port = closed.getsockname()[1]
        result = market_probe.udp_compatibility("127.0.0.1", port, timeout=0.5, interval=0.2)
        self.assertFalse(result["ok"])
        self.assertTrue(result["error"].startswith(("refused", "no compatibility reply")), result)

    def test_lobby_check_resolves_and_compares(self):
        with FakeLobby() as lobby:
            endpoint = f"localhost:{lobby.port}"
            seen = []
            check = market_probe.lobby_check(endpoint, resolve=lambda host: seen.append(host) or ["127.0.0.1"])
            self.assertEqual(seen, ["localhost"])
            self.assertTrue(check["ok"], check)
            self.assertEqual((check["endpoint"], check["address"]), (endpoint, "127.0.0.1"))
            other = market_probe.lobby_check(endpoint, addresses=["127.0.0.1"], expected={**SERVER, "geometry": "g9"})
        self.assertFalse(other["ok"])
        self.assertEqual((other["compatible"], other["issue"]), (False, "geometry"))
        self.assertEqual(json.loads(lobby.requests[-1])["client"]["geometry"], "g9")
        failed = market_probe.lobby_check("lobby.example:4000", resolve=lambda host: (_ for _ in ()).throw(OSError()))
        self.assertEqual(failed, {"endpoint": "lobby.example:4000", "ok": False, "error": "DNS: OSError"})
        self.assertFalse(market_probe.lobby_check("lobby.example", addresses=["127.0.0.1"])["ok"])
        self.assertEqual(market_probe.lobby_check("lobby.example:4000", addresses=[])["error"],
                         "DNS: lobby.example has no address")
        probed = []
        market_probe.lobby_check("lobby.example:4000", addresses=["2001:db8::1", "203.0.113.7"],
                                 udp=lambda address, port, client, timeout: probed.append(address) or {"ok": False})
        self.assertEqual(probed, ["203.0.113.7"], "IPv4 first: the lobby has no AAAA record")


class HttpTest(unittest.TestCase):
    def test_statuses_and_unreachable_servers(self):
        with http_server(**{"/health/ready": 503}) as origin:
            self.assertEqual(market_probe.http_check(origin + "/", range(200, 400)),
                             {"url": origin + "/", "ok": True, "status": 200})
            ready = market_probe.http_check(origin + "/health/ready", (200,))
            self.assertEqual((ready["ok"], ready["status"], ready["error"]), (False, 503, "HTTP 503"))
        gone = market_probe.http_check(origin + "/", (200,))
        self.assertFalse(gone["ok"])
        self.assertIn(gone["error"], ("ConnectionRefusedError", "URLError"))


def settings(**extra):
    values = {"OMOBA_PROBE_MARKET": "ru", "OMOBA_PROBE_LOBBY": "lobby.example:4000",
              "OMOBA_PROBE_ACCOUNT_API": "https://account.example", "OMOBA_PROBE_PORTAL": "https://portal.example",
              "OMOBA_TELEGRAM_BOT_TOKEN": "123456:" + "A" * 30, "OMOBA_TELEGRAM_CHAT_ID": "-1001"}
    values.update(extra)
    return market_probe.load_settings({key: value for key, value in values.items() if value is not None})


class SettingsTest(unittest.TestCase):
    def test_config_comes_from_markets_py(self):
        text = market_probe.config_text("ru")
        values, problems = market_probe.parse_env_text(text)
        self.assertEqual(problems, [])
        ru = markets.MARKETS["ru"]
        self.assertEqual((values["OMOBA_PROBE_LOBBY"], values["OMOBA_PROBE_ACCOUNT_API"], values["OMOBA_PROBE_PORTAL"]),
                         (ru.lobby, ru.account_api, ru.portal))
        loaded = market_probe.load_settings(values)
        self.assertEqual(loaded["OMOBA_PROBE_FAILURES"], "2", "alert after 2 failed rounds in a row")
        self.assertFalse(market_probe.telegram(loaded, "x"), "placeholders: the journal only")
        for market in ("global", "cn"):
            with self.assertRaises(market_probe.ProbeError):
                market_probe.config_text(market)

    def test_config_out_is_owner_only_and_never_replaced(self):
        with tempfile.TemporaryDirectory() as directory:
            out = os.path.join(directory, "ru.env")
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(market_probe.main(["config", "--market", "ru", "--out", out]), 0)
                self.assertEqual(os.stat(out).st_mode & 0o777, 0o600)
                Path(out).write_text("kept")
                self.assertEqual(market_probe.main(["config", "--market", "ru", "--out", out]), 2)
            self.assertEqual(Path(out).read_text(), "kept")

    def test_bad_settings_are_refused(self):
        cases = {"OMOBA_PROBE_MARKET": ("RU!", None), "OMOBA_PROBE_LOBBY": ("lobby.example", "lobby..x:4000"),
                 "OMOBA_PROBE_PORTAL": ("http://portal.example", "https://portal.example/x",
                                        "https://u:p@portal.example"),
                 "OMOBA_PROBE_FAILURES": ("0", "two"),
                 "OMOBA_TELEGRAM_CHAT_ID": (None, "CHANGE_ME")}
        for key, values in cases.items():
            for value in values:
                with self.subTest(key=key, value=value), self.assertRaises(market_probe.ProbeError) as refused:
                    settings(**{key: value})
                self.assertNotIn("A" * 30, str(refused.exception), "the token is never echoed")
        with self.assertRaises(market_probe.ProbeError):
            settings(OMOBA_PROBE_LOBBY=None, OMOBA_PROBE_ACCOUNT_API=None, OMOBA_PROBE_PORTAL=None)
        self.assertEqual(settings(OMOBA_PROBE_PORTAL="https://portal.example/")["OMOBA_PROBE_PORTAL"],
                         "https://portal.example")
        self.assertEqual(market_probe.parse_env_text("OMOBA_PROBE_PORTL=x\n")[1], ["unknown setting OMOBA_PROBE_PORTL"])


class RoundTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.state = self.temporary.name
        self.sent = []
        self.now = 1_790_000_000.0
        self.lobby_up = True

    def tearDown(self):
        self.temporary.cleanup()

    def opener(self, request, timeout):
        self.sent.append((request.full_url, parse_qs(request.data.decode())))
        return contextlib.nullcontext()

    def udp(self, address, port, client, timeout):
        return {"ok": True, "server": SERVER, "rtt_ms": 30} if self.lobby_up else \
            {"ok": False, "error": "no compatibility reply in 4 s"}

    def round(self, config=None, fetch=lambda url: 200):
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            result = market_probe.run_round(config or settings(), self.state, now=lambda: self.now,
                                            resolve=lambda host: ["203.0.113.7"], fetch=fetch, udp=self.udp,
                                            opener=self.opener, host="probe-1")
        self.now += 60
        return result, stderr.getvalue()

    def texts(self):
        return [query["text"][0] for _, query in self.sent]

    def test_two_failures_in_a_row_alert_once_then_remind_and_recover(self):
        self.assertTrue(self.round()[0]["ok"])
        self.lobby_up = False
        first, journal = self.round()
        self.assertFalse(first["ok"])
        self.assertEqual((first["alerted"], self.sent), ([], []), "one failed round is not an outage")
        second, journal = self.round()
        self.assertEqual(len(self.sent), 1)
        url, query = self.sent[0]
        self.assertTrue(url.startswith("https://api.telegram.org/bot123456:"))
        self.assertEqual(query["chat_id"], ["-1001"])
        text = query["text"][0]
        self.assertIn("OMOBA ru: down (external probe on probe-1)", text)
        self.assertIn("lobby UDP lobby.example:4000: no compatibility reply in 4 s (2 checks in a row", text)
        self.assertIn("<3>OMOBA ru: down", journal)
        for _ in range(59):
            self.round()
        self.assertEqual(len(self.sent), 1, "no repeat within the hour")
        self.round()
        self.assertEqual(len(self.sent), 2, "a reminder after 60 minutes")
        self.assertIn("(62 checks in a row", self.texts()[-1])
        self.lobby_up = True
        back, journal = self.round()
        self.assertTrue(back["ok"])
        self.assertIn("OMOBA ru: back up (external probe on probe-1)\n- lobby UDP lobby.example:4000 answers again",
                      self.texts()[-1])
        self.round()
        self.assertEqual(len(self.sent), 3, "the recovery is said once")
        state = json.loads(Path(self.state, "state.json").read_text())
        self.assertEqual(state["checks"]["lobby_udp"], {"failures": 0})

    def test_a_flap_never_alerts(self):
        for _ in range(5):
            self.lobby_up = not self.lobby_up
            self.round()
        self.assertEqual(self.sent, [])

    def test_every_target_is_checked_and_grouped_in_one_message(self):
        down = {"https://account.example/health/ready": 503}
        fetch = lambda url: down.get(url, 200)
        self.round(fetch=fetch)
        result, _ = self.round(fetch=fetch)
        self.assertEqual(sorted(result["checks"]), ["account_api", "lobby_udp", "portal"])
        self.assertEqual(self.texts(), ["OMOBA ru: down (external probe on probe-1)\n- Account API "
                                        "https://account.example/health/ready: HTTP 503 (2 checks in a row, since "
                                        "2026-09-21 14:13 UTC)"])

    def test_without_a_bot_alerts_stay_in_the_journal(self):
        quiet = settings(OMOBA_TELEGRAM_BOT_TOKEN="CHANGE_ME", OMOBA_TELEGRAM_CHAT_ID="CHANGE_ME")
        self.lobby_up = False
        self.round(quiet)
        result, journal = self.round(quiet)
        self.assertEqual((self.sent, result["telegram"]), ([], {"down": False}))
        self.assertIn("<3>OMOBA ru: down", journal)

    def test_an_undelivered_alert_never_prints_the_token(self):
        token = "123456:" + "A" * 30

        def broken(request, timeout):
            raise OSError(f"bad request to https://api.telegram.org/bot{token}/sendMessage")
        self.lobby_up = False
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            for _ in range(2):
                market_probe.run_round(settings(), self.state, now=lambda: self.now, resolve=lambda h: ["203.0.113.7"],
                                       fetch=lambda url: 200, udp=self.udp, opener=broken, host="probe-1")
        self.assertIn("not delivered", stderr.getvalue())
        self.assertNotIn(token, stderr.getvalue())

    def test_run_needs_a_state_folder_and_reads_the_environment(self):
        environment = {"OMOBA_PROBE_MARKET": "ru", "OMOBA_PROBE_PORTAL": "https://portal.example",
                       "STATE_DIRECTORY": self.state, "UNRELATED": "x"}
        out = io.StringIO()
        with mock.patch.dict(os.environ, environment, clear=True), \
                mock.patch.object(market_probe, "run_checks", return_value={"portal": {"ok": True}}) as checks:
            self.assertEqual(market_probe.main(["run"], stdout=out), 0)
        self.assertEqual(checks.call_args.args[0]["OMOBA_PROBE_PORTAL"], "https://portal.example")
        self.assertTrue(json.loads(out.getvalue())["ok"])
        with mock.patch.dict(os.environ, {**environment, "STATE_DIRECTORY": ""}, clear=True), \
                contextlib.redirect_stderr(io.StringIO()) as err:
            self.assertEqual(market_probe.main(["run"]), 2)
        self.assertIn("state folder", err.getvalue())

    def test_check_exits_1_when_anything_is_down(self):
        with tempfile.TemporaryDirectory() as directory:
            env = os.path.join(directory, "ru.env")
            Path(env).write_text("OMOBA_PROBE_MARKET=ru\nOMOBA_PROBE_PORTAL=https://portal.example\n")
            for ok, code in ((True, 0), (False, 1)):
                check = {"url": "https://portal.example/", "ok": ok, "error": "HTTP 502"}
                with mock.patch.object(market_probe, "run_checks", return_value={"portal": check}):
                    out = io.StringIO()
                    self.assertEqual(market_probe.main(["check", "--env-file", env], stdout=out), code)
                self.assertEqual(out.getvalue().splitlines()[0], "ok" if ok else "DOWN")


class UnitTest(unittest.TestCase):
    def test_the_probe_units_run_the_installed_copy_every_minute_unprivileged(self):
        service = (ROOT / "ops/probe/omoba-probe@.service").read_text()
        timer = (ROOT / "ops/probe/omoba-probe@.timer").read_text()
        for line in ("ExecStart=/usr/bin/python3 /opt/omoba-probe/market_probe.py run",
                     "EnvironmentFile=/etc/omoba-probe/%i.env", "StateDirectory=omoba-probe/%i",
                     "DynamicUser=yes", "Type=oneshot", "ProtectSystem=strict"):
            self.assertIn(f"\n{line}\n", service)
        self.assertIn("\nOnCalendar=*-*-* *:*:00\n", timer)
        self.assertIn("\nWantedBy=timers.target\n", timer)
        timeout = int(re.search(r"\nTimeoutStartSec=(\d+)s\n", service).group(1))
        self.assertLess(market_probe.UDP_TIMEOUT + 2 * market_probe.HTTP_TIMEOUT, timeout)
        self.assertLess(timeout, 60, "a round ends before the next minute")


if __name__ == "__main__":
    unittest.main()
