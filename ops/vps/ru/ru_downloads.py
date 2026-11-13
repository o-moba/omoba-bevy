#!/usr/bin/env python3
"""Count RU downloads per platform from Caddy's access log (LR-23).

Caddy serves the RU packages from /opt/omoba-ru/downloads under the portal's download
path (Caddyfile.ru) and logs only those requests: JSON, the client address shortened to
/24 (IPv4) or /48 (IPv6), no request headers, rolled (gzip) files kept 90 days. This
script reads the log and its rolled copies and prints totals; it never prints an
address. Standard library only, so it runs on the host as it is:

    # from the Mac, through the guarded ssh of ru_host.py (the usual way)
    python3 ops/vps/ru/ru_host.py downloads [--since 2026-11-01] [--until 2026-11-30]
    # on the host, as root
    python3 - count < ru_downloads.py
    # against a copied log (it holds shortened addresses: keep it in Russia)
    python3 ops/vps/ru/ru_downloads.py count --log <folder>/omoba-ru-downloads.log

What counts:
- `downloads`: GET requests for a package file answered 200 or 206, counted once per
  shortened address, file and UTC day. A resumed or segmented download counts once.
  Players who share an address or a /24 (a carrier NAT, an office) on the same day
  for the same file also count once, so this is a floor.
- `completed`: 200 answers that sent the whole file in one response.
- `requests`: every such GET (resumes and segments included).
HEAD requests, 304 answers, errors and other paths are not counted. Package names are
release.py's `artifact_name` for the market (`Omoba-<v>-ru-windows-x64.zip`,
`omoba-<v>-ru-linux-x64.tar.gz`, `Omoba-<v>-ru-android-universal.apk`, …) under
`<prefix>/v<version>/`.
"""
from __future__ import annotations

import argparse
import datetime
import glob
import gzip
import json
import os
import re
import sys
from urllib.parse import unquote, urlsplit

DEFAULT_LOG = "/var/log/caddy/omoba-ru-downloads.log"   # ru_host.py DOWNLOAD_LOG
DEFAULT_PREFIX = "/downloads"   # scripts/markets.py MARKETS["ru"].downloads_path
DEFAULT_LABEL = "-ru"           # scripts/markets.py MARKETS["ru"].label
PLATFORMS = ("macos", "windows", "linux", "android")
VERSION = r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?"
DAY = re.compile(r"\d{4}-\d{2}-\d{2}")
PREFIX = re.compile(r"(?:/[a-z0-9][a-z0-9_-]*)+")
LABEL = re.compile(r"-[a-z]{2,8}")


def package_pattern(label: str) -> re.Pattern:
    """Production package names of one market (`label`, e.g. `-ru`); never dev packages."""
    return re.compile(rf"[Oo]moba-(?P<version>{VERSION}){re.escape(label)}-(?P<platform>{'|'.join(PLATFORMS)})"
                      r"-[a-z0-9_]+\.(?:zip|tar\.gz|apk)")


def log_files(log: str) -> list[str]:
    """The log and its rolled copies (`<stem>-<time>.log[.gz]`), oldest name first."""
    folder, name = os.path.split(log)
    stem = name[:-len(".log")] if name.endswith(".log") else name
    found = set(glob.glob(os.path.join(glob.escape(folder), glob.escape(stem) + "-*.log*")))
    if os.path.isfile(log):
        found.add(log)
    return sorted(path for path in found if os.path.isfile(path))


def read_lines(paths):
    for path in paths:
        opener = gzip.open if path.endswith(".gz") else open
        with opener(path, "rt", encoding="utf-8", errors="replace") as handle:
            yield from handle


def day_of(value) -> datetime.date:
    """Caddy's `ts`: Unix seconds (the JSON default) or an ISO 8601 string."""
    if isinstance(value, (int, float)):
        return datetime.datetime.fromtimestamp(value, datetime.timezone.utc).date()
    return datetime.datetime.fromisoformat(str(value).replace("Z", "+00:00")).astimezone(
        datetime.timezone.utc).date()


def whole_file(record: dict) -> bool:
    """A 200 whose body is as long as its Content-Length: the whole file in one response."""
    lengths = (record.get("resp_headers") or {}).get("Content-Length") or []
    try:
        length = int(lengths[0] if isinstance(lengths, list) else lengths)
        return length > 0 and int(record.get("size", -1)) == length
    except (ValueError, TypeError, IndexError):
        return False


def count(lines, prefix: str = DEFAULT_PREFIX, label: str = DEFAULT_LABEL,
          since: datetime.date | None = None, until: datetime.date | None = None) -> dict:
    """Totals per platform, per version and per UTC day; no address leaves this function."""
    pattern = package_pattern(label)
    platforms = {platform: {"downloads": 0, "completed": 0, "requests": 0} for platform in PLATFORMS}
    versions, days, seen = {}, {}, set()
    lines_read = unreadable = 0
    first = last = None
    for line in lines:
        if not line.strip():
            continue
        lines_read += 1
        try:
            record = json.loads(line)
            request = record["request"]
            method, uri, status = request["method"], request["uri"], int(record["status"])
            day = day_of(record["ts"])
        except (ValueError, KeyError, TypeError, OverflowError, OSError):
            unreadable += 1
            continue
        if (since and day < since) or (until and day > until):
            continue
        first, last = min(first or day, day), max(last or day, day)
        if method != "GET" or status not in (200, 206):
            continue
        path = unquote(urlsplit(str(uri)).path)
        if not path.startswith(prefix + "/"):
            continue
        parts = path[len(prefix) + 1:].split("/")
        match = pattern.fullmatch(parts[-1]) if len(parts) == 2 else None
        if match is None or parts[0] != "v" + match["version"]:
            continue
        platform, version = match["platform"], match["version"]
        totals = platforms[platform]
        totals["requests"] += 1
        if status == 200 and whole_file(record):
            totals["completed"] += 1
        key = (request.get("client_ip") or request.get("remote_ip") or "", parts[-1], day)
        if key in seen:
            continue
        seen.add(key)
        totals["downloads"] += 1
        versions.setdefault(version, dict.fromkeys(PLATFORMS, 0))[platform] += 1
        days.setdefault(day.isoformat(), dict.fromkeys(PLATFORMS, 0))[platform] += 1
    return {"first_day": first and first.isoformat(), "last_day": last and last.isoformat(),
            "lines": lines_read, "unreadable": unreadable,
            "total_downloads": sum(totals["downloads"] for totals in platforms.values()),
            "platforms": platforms, "versions": dict(sorted(versions.items())), "days": dict(sorted(days.items()))}


def parse_day(value: str | None, flag: str) -> datetime.date | None:
    if value is None:
        return None
    if not DAY.fullmatch(value):
        raise SystemExit(f"--{flag} must be YYYY-MM-DD, got {value!r}")
    return datetime.date.fromisoformat(value)


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    counter = sub.add_parser("count", help="print downloads per platform, version and UTC day as JSON")
    counter.add_argument("--log", default=DEFAULT_LOG, help=f"the current log file (default {DEFAULT_LOG})")
    counter.add_argument("--prefix", default=DEFAULT_PREFIX, help="the download path on the portal")
    counter.add_argument("--label", default=DEFAULT_LABEL, help="the market's package label")
    counter.add_argument("--since", help="first UTC day counted, YYYY-MM-DD")
    counter.add_argument("--until", help="last UTC day counted, YYYY-MM-DD")
    args = parser.parse_args(argv)
    if not PREFIX.fullmatch(args.prefix) or not LABEL.fullmatch(args.label):
        raise SystemExit("--prefix must look like /downloads and --label like -ru")
    files = log_files(args.log)
    if not files:
        raise SystemExit(f"no download log at {args.log} (Caddyfile.ru logs the download path there)")
    report = count(read_lines(files), args.prefix, args.label, parse_day(args.since, "since"),
                   parse_day(args.until, "until"))
    print(json.dumps({"log_files": len(files), **report}, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
