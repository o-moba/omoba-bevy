"""LR-23: the RU download host. Caddyfile.ru serves the packages release.py publishes and
logs only those requests; ops/vps/ru/ru_downloads.py counts them per platform from that
log without ever printing an address. The names, paths and package patterns of the
pieces (scripts/markets.py, release.py, ru_host.py, the Makefile) stay in step."""
from __future__ import annotations

import contextlib
import gzip
import io
import json
import os
import re
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "ops/vps/ru"))
sys.path.insert(0, str(ROOT / "scripts"))

import markets  # noqa: E402
import release  # noqa: E402
import ru_downloads  # noqa: E402
import ru_host  # noqa: E402

RU = markets.MARKETS["ru"]
CADDYFILE = (ROOT / "ops/vps/ru/Caddyfile.ru").read_text()
PORTAL = urlsplit(RU.portal).hostname
WINDOWS = release.artifact_name("0.45.0", "windows", "prod", "ru")
LINUX = release.artifact_name("0.45.0", "linux", "prod", "ru")


def entry(uri, status=200, method="GET", ip="203.0.113.0", ts=1_790_000_000.5, size=1000, length=1000,
          client_ip=None):
    """One line of Caddy's JSON access log after Caddyfile.ru's filter (no request headers)."""
    request = {"remote_ip": ip, "proto": "HTTP/2.0", "method": method, "host": PORTAL, "uri": uri}
    if client_ip:
        request["client_ip"] = client_ip
    return json.dumps({"level": "info", "ts": ts, "logger": "http.log.access.log0", "msg": "handled request",
                       "request": request, "duration": 0.5, "size": size, "status": status,
                       "resp_headers": {"Content-Length": [str(length)], "Server": ["Caddy"]}})


def package(name, version="0.45.0"):
    return f"{RU.downloads_path}/{release.tag_for(version)}/{name}"


class KitTest(unittest.TestCase):
    def test_the_portal_site_serves_and_logs_only_the_download_path(self):
        self.assertEqual(ru_host.download_site_problems(CADDYFILE, PORTAL, RU.downloads_path), [])
        site = ru_host.caddy_site(CADDYFILE, PORTAL)
        self.assertIn(f"root * {ru_host.DOWNLOADS}", site)
        self.assertIn(f"output file {ru_host.DOWNLOAD_LOG}", site)
        # The Account API site neither serves nor logs downloads.
        account = ru_host.caddy_site(CADDYFILE, urlsplit(RU.account_api).hostname)
        self.assertNotIn("file_server", account)
        self.assertNotIn("log {", account)
        # No listing, no in-progress rsync files, no request headers or full addresses logged.
        self.assertNotIn("browse", site)
        self.assertRegex(site, r"file_server \{\s*hide \.\*\s*\}")
        self.assertRegex(site, r"request>remote_ip ip_mask 24 48")
        self.assertRegex(site, r"roll_keep_for 2160h")

    def test_the_kit_refuses_a_portal_site_without_the_download_host(self):
        broken = {
            "does not serve /opt/omoba-ru/downloads": CADDYFILE.replace(f"handle_path {RU.downloads_path}/*",
                                                                        "handle_path /files/*"),
            "does not log the downloads": CADDYFILE.replace(ru_host.DOWNLOAD_LOG, "/tmp/x.log"),
            "does not drop request headers": CADDYFILE.replace("request>headers delete", ""),
            "logs more than": CADDYFILE.replace("skip_log @not_downloads", ""),
        }
        for needle, text in broken.items():
            with self.subTest(needle):
                problems = ru_host.download_site_problems(text, PORTAL, RU.downloads_path)
                self.assertTrue(any(needle in problem for problem in problems), problems)
        with tempfile.TemporaryDirectory() as directory:
            for path in (ROOT / "ops/vps/ru").iterdir():
                if path.is_file():
                    (Path(directory) / path.name).write_bytes(path.read_bytes())
            (Path(directory) / "Caddyfile.ru").write_text(broken["logs more than"])
            with mock.patch.object(ru_host, "RU_DIR", Path(directory)), self.assertRaises(ru_host.Refused) as refused:
                ru_host.kit()
        self.assertIn("logs more than", refused.exception.message)

    def test_a_download_path_that_is_not_a_plain_path_is_refused(self):
        import dataclasses
        for bad in (None, "downloads", "/downloads/", "/../etc", "/Downloads x"):
            with self.subTest(bad), mock.patch.dict(markets.MARKETS, {"ru": dataclasses.replace(RU, downloads_path=bad)}), \
                    self.assertRaises(ru_host.Refused):
                ru_host.ru_downloads_path()

    def test_every_piece_names_the_same_host_folder_log_and_packages(self):
        self.assertEqual(RU.downloads, RU.portal + RU.downloads_path, "on the portal origin: no fourth DNS name")
        self.assertEqual(ru_downloads.DEFAULT_LOG, ru_host.DOWNLOAD_LOG)
        self.assertEqual(ru_downloads.DEFAULT_PREFIX, RU.downloads_path)
        self.assertEqual(ru_downloads.DEFAULT_LABEL, RU.label)
        self.assertEqual(ru_downloads.PLATFORMS, tuple(target for target in release.CATALOG_PLATFORMS
                                                       if target in RU.platforms))
        makefile = (ROOT / "Makefile").read_text()
        target = re.search(r"(?m)^RU_DOWNLOADS_TARGET \?= (.+)$", makefile).group(1)
        self.assertEqual(target.rpartition(":")[2], ru_host.DOWNLOADS)
        self.assertIn('--rsync-target "$(RU_DOWNLOADS_TARGET)"', makefile)
        self.assertIn((ru_host.DOWNLOADS, "root", "0755"), ru_host.DIRS)
        # The host tool writes no RU name itself (scripts/markets.py is the one place).
        self.assertNotIn(PORTAL, (ROOT / "ops/vps/ru/ru_downloads.py").read_text())

    def test_the_package_pattern_is_release_artifact_name(self):
        pattern = ru_downloads.package_pattern(RU.label)
        for universal in ("1", ""):
            with mock.patch.dict(os.environ, {"OMOBA_ANDROID_UNIVERSAL": universal}):
                for target in ru_downloads.PLATFORMS:
                    for version in ("0.45.0", "0.46.0-rc.2"):
                        name = release.artifact_name(version, target, "prod", "ru")
                        match = pattern.fullmatch(name)
                        self.assertIsNotNone(match, name)
                        self.assertEqual((match["platform"], match["version"]), (target, version))
        for other in (release.artifact_name("0.45.0", "windows"), release.artifact_name("0.45.0", "windows", "dev", "ru"),
                      "Omoba-0.45.0-ru-ios.ipa", "SHA256SUMS.txt", "omoba-0.45.0-ru-linux-x64.tar.gz.part"):
            self.assertIsNone(pattern.fullmatch(other), other)


class CountTest(unittest.TestCase):
    def test_downloads_per_platform_version_and_day_without_addresses(self):
        day = 86_400
        lines = [
            entry(package(WINDOWS)),                                   # complete in one response
            entry(package(WINDOWS), status=206, size=400),             # the same player resumes
            entry(package(WINDOWS), status=206, size=600),
            entry(package(WINDOWS), ip="198.51.100.0", size=10),       # another player, cut off
            entry(package(WINDOWS), ts=1_790_000_000 + day),           # the first player, next day
            entry(package(LINUX), ts="2026-09-21T13:00:00Z", client_ip="192.0.2.0"),
            entry(package(LINUX, "0.44.0")),                           # tag and version disagree
            entry(package(WINDOWS), method="HEAD"),
            entry(package(WINDOWS), status=304),
            entry(package(WINDOWS), status=404),
            entry("/api/releases"),
            entry(package("SHA256SUMS.txt")),
            entry(package(release.artifact_name("0.45.0", "windows", "dev", "ru"))),
            entry(f"{RU.downloads_path}/{WINDOWS}"),                   # no tag folder
            entry(package(WINDOWS) + "?utm=1", ip="198.51.100.77", ts=1_790_000_000 + day),
            "not json",
            json.dumps({"request": {}}),
            "",
        ]
        report = ru_downloads.count(lines, RU.downloads_path, RU.label)
        self.assertEqual(report["platforms"]["windows"], {"downloads": 4, "completed": 3, "requests": 6})
        self.assertEqual(report["platforms"]["linux"], {"downloads": 1, "completed": 1, "requests": 1})
        self.assertEqual(report["platforms"]["android"], {"downloads": 0, "completed": 0, "requests": 0})
        self.assertEqual(report["total_downloads"], 5)
        self.assertEqual(report["versions"], {"0.45.0": {"macos": 0, "windows": 4, "linux": 1, "android": 0}})
        self.assertEqual(report["days"], {"2026-09-21": {"macos": 0, "windows": 2, "linux": 1, "android": 0},
                                          "2026-09-22": {"macos": 0, "windows": 2, "linux": 0, "android": 0}})
        self.assertEqual((report["unreadable"], report["lines"]), (2, 17))
        self.assertEqual((report["first_day"], report["last_day"]), ("2026-09-21", "2026-09-22"))
        printed = json.dumps(report)
        for address in ("203.0.113", "198.51.100", "192.0.2"):
            self.assertNotIn(address, printed)
        # A window of days.
        since = ru_downloads.parse_day("2026-09-22", "since")
        later = ru_downloads.count(lines, RU.downloads_path, RU.label, since=since)
        self.assertEqual(later["platforms"]["windows"]["downloads"], 2)
        self.assertEqual(later["platforms"]["linux"]["downloads"], 0)
        with self.assertRaises(SystemExit):
            ru_downloads.parse_day("22.09.2026", "since")

    def test_rolled_gzip_logs_are_read_and_the_cli_prints_totals(self):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory, "omoba-ru-downloads.log")
            log.write_text(entry(package(WINDOWS)) + "\n")
            with gzip.open(Path(directory, "omoba-ru-downloads-2026-09-20T00-00-00.000.log.gz"), "wt") as rolled:
                rolled.write(entry(package(LINUX), ts=1_790_000_000 - 86_400) + "\n")
            Path(directory, "other.log").write_text(entry(package(WINDOWS), ip="198.51.100.0") + "\n")
            self.assertEqual([Path(path).name for path in ru_downloads.log_files(str(log))],
                             ["omoba-ru-downloads-2026-09-20T00-00-00.000.log.gz", "omoba-ru-downloads.log"])
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                self.assertEqual(ru_downloads.main(["count", "--log", str(log)]), 0)
            report = json.loads(out.getvalue())
            self.assertEqual(report["log_files"], 2)
            self.assertEqual({platform: totals["downloads"] for platform, totals in report["platforms"].items()},
                             {"macos": 0, "windows": 1, "linux": 1, "android": 0})
            for bad in (["count", "--log", str(Path(directory, "missing.log"))],
                        ["count", "--log", str(log), "--prefix", "downloads; rm -rf /"],
                        ["count", "--log", str(log), "--label", "ru"]):
                with self.subTest(bad), self.assertRaises(SystemExit):
                    ru_downloads.main(bad)


if __name__ == "__main__":
    unittest.main()
