#!/usr/bin/env python3
"""External probe of a market's public servers (LR-17).

Runs every minute on a probe host OUTSIDE the market's own host (ops/probe/README.md),
so it also notices when that whole host or its network is gone. Each round checks:

- the lobby's UDP compatibility check (the same pre-game exchange a game client sends
  first, shared/src/compatibility.rs): a valid, correlated report must come back;
- the Account API `GET <account_api>/health/ready` (200);
- the portal `GET <portal>/` (200..399).

A check that fails twice in a row (OMOBA_PROBE_FAILURES) alerts the owner on Telegram
(OMOBA_TELEGRAM_BOT_TOKEN, OMOBA_TELEGRAM_CHAT_ID), reminds every
OMOBA_PROBE_REPEAT_MINUTES while it stays down and says once when it is back. Without
the bot, alerts go to the journal (stderr) only.

    market_probe.py config --market ru [--out FILE]   # the probe host's settings, from scripts/markets.py
    market_probe.py check --market ru [--json]        # one round from a checkout; no state, no alerts
    market_probe.py check --env-file FILE [--json]    # one round with an installed settings file
    market_probe.py run [--env-file FILE] [--state DIR]   # the timer: one round, state and alerts

The public names come only from scripts/markets.py: `config` writes them into the probe
host's settings file, and `release.py preflight` reuses the UDP check
(`udp_compatibility`, `lobby_check`). The installed copy is this one file and needs
nothing but Python 3.9+ from the standard library. Nothing it sends or logs carries a
player's data; the bot token is never printed.
"""
from __future__ import annotations

import argparse
import datetime as dt
import ipaddress
import json
import os
import re
import secrets
import socket
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

TOOL = "scripts/market_probe.py"
PLACEHOLDER = "CHANGE_ME"

# shared/src/compatibility.rs: the stable pre-game JSON exchange.
HANDSHAKE_VERSION = 1
MAX_PROBE_BYTES = 2048
PROBE_PADDING = 512
CONTRACT_NUMBERS = ("handshake", "protocol")
CONTRACT_LABELS = ("release", "catalog", "geometry", "gameplay")
CONTRACT_KEYS = ("handshake", "release", "protocol", "catalog", "geometry", "gameplay")
LABEL = re.compile(r"[A-Za-z0-9._+-]{1,64}")
NONCE = re.compile(r"[0-9a-f]{32}")
# The server answers any valid contract with its own (`response_with_update`); the probe
# only needs an answer. With an expected contract (`release.py preflight --contract`)
# that contract is sent and compared instead.
PROBE_CLIENT = {"handshake": HANDSHAKE_VERSION, "release": "0.0.0", "protocol": 0,
                "catalog": "probe", "geometry": "probe", "gameplay": "probe"}
UDP_TIMEOUT = 4.0       # like `shared --example compatibility check`
UDP_INTERVAL = 0.5
HTTP_TIMEOUT = 8.0

# Settings of an installed probe (systemd EnvironmentFile, or --env-file).
TARGET_KEYS = ("OMOBA_PROBE_LOBBY", "OMOBA_PROBE_ACCOUNT_API", "OMOBA_PROBE_PORTAL")
DEFAULTS = {"OMOBA_PROBE_FAILURES": "2", "OMOBA_PROBE_REPEAT_MINUTES": "60"}
KNOWN_KEYS = frozenset(("OMOBA_PROBE_MARKET", *TARGET_KEYS, *DEFAULTS,
                        "OMOBA_TELEGRAM_BOT_TOKEN", "OMOBA_TELEGRAM_CHAT_ID"))
CHECK_NAMES = {"lobby_udp": "OMOBA_PROBE_LOBBY", "account_api": "OMOBA_PROBE_ACCOUNT_API",
               "portal": "OMOBA_PROBE_PORTAL"}
DNS_NAME = re.compile(r"(?=.{1,253}\Z)(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z](?:[a-z0-9-]{0,61}[a-z0-9])?")
MARKET_KEY = re.compile(r"[a-z][a-z0-9-]{0,15}")


class ProbeError(Exception):
    """Unusable settings; the message is safe to print."""


# ---------------------------------------------------------------- the UDP compatibility check

def valid_contract(value) -> bool:
    """shared::compatibility::ReleaseContract::valid, plus the u16 numbers serde enforces."""
    if not isinstance(value, dict):
        return False
    for key in CONTRACT_NUMBERS:
        number = value.get(key)
        if not isinstance(number, int) or isinstance(number, bool) or not 0 <= number <= 0xFFFF:
            return False
    return all(isinstance(value.get(key), str) and LABEL.fullmatch(value[key]) for key in CONTRACT_LABELS)


def contract_issue(client: dict, server: dict) -> str | None:
    """ReleaseContract::compare: the first mismatch (release labels may differ), or None."""
    if client.get("handshake") != HANDSHAKE_VERSION or server.get("handshake") != HANDSHAKE_VERSION:
        return "handshake"
    for key in ("protocol", "catalog", "geometry", "gameplay"):
        if client.get(key) != server.get(key):
            return key
    return None


def probe_request(nonce: str, client: dict) -> bytes:
    return json.dumps({"type": "compatibility_probe", "nonce": nonce, "client": client,
                       "padding": "0" * PROBE_PADDING}, separators=(",", ":")).encode()


def parse_report(data: bytes, nonce: str) -> dict | None:
    """CompatibilityProbe::accept_report: a report for this nonce with a valid contract."""
    if len(data) > MAX_PROBE_BYTES:
        return None
    try:
        message = json.loads(data)
    except ValueError:
        return None
    if not isinstance(message, dict) or message.get("type") != "compatibility_report" \
            or message.get("nonce") != nonce or not valid_contract(message.get("server")):
        return None
    update = message.get("update")
    report = {"server": {key: message["server"][key] for key in CONTRACT_KEYS}}
    if isinstance(update, dict) and isinstance(update.get("min_release"), str) \
            and LABEL.fullmatch(update["min_release"]):
        report["min_release"] = update["min_release"]
    return report


def udp_compatibility(address: str, port: int, client: dict | None = None, timeout: float = UDP_TIMEOUT,
                      interval: float = UDP_INTERVAL) -> dict:
    """Send the compatibility probe to address:port (resent every `interval`) until a valid
    report for its nonce arrives or `timeout` passes. A connected socket only receives from
    that endpoint. {"ok": True, "server": contract, "rtt_ms": n} or {"ok": False, "error": ...}."""
    client = client or PROBE_CLIENT
    nonce = secrets.token_hex(16)
    request = probe_request(nonce, client)
    family = socket.AF_INET6 if ":" in address else socket.AF_INET
    started = time.monotonic()
    deadline = started + timeout
    try:
        with socket.socket(family, socket.SOCK_DGRAM) as sock:
            sock.connect((address, port))
            next_send = started
            while True:
                now = time.monotonic()
                if now >= deadline:
                    return {"ok": False, "error": f"no compatibility reply in {timeout:g} s"}
                if now >= next_send:
                    sock.send(request)
                    next_send = now + interval
                sock.settimeout(max(0.01, min(next_send, deadline) - time.monotonic()))
                try:
                    data = sock.recv(MAX_PROBE_BYTES + 1)
                except socket.timeout:
                    continue
                report = parse_report(data, nonce)
                if report is not None:
                    return {"ok": True, "rtt_ms": round((time.monotonic() - started) * 1000), **report}
    except ConnectionRefusedError:
        return {"ok": False, "error": "refused: nothing listens on the UDP port (ICMP port unreachable)"}
    except OSError as error:
        return {"ok": False, "error": f"UDP: {type(error).__name__}: {error.strerror or error}"}


def split_endpoint(endpoint: str) -> tuple[str, int]:
    """`host:port` → (host, port); the host lower-cased without a trailing dot."""
    host, sep, port = (endpoint or "").rpartition(":")
    host = host.strip("[]").lower().rstrip(".")
    if not sep or not host or not port.isdigit() or not 0 < int(port) <= 0xFFFF:
        raise ProbeError(f"{endpoint!r} is not host:port")
    return host, int(port)


def resolve_addresses(host: str) -> list[str]:
    """IPv4 first: the lobby publishes no AAAA record (ops/vps/ru/README.md)."""
    found = {info[4][0] for info in socket.getaddrinfo(host, None, type=socket.SOCK_DGRAM)}
    return sorted(found, key=lambda address: (":" in address, address))


def lobby_check(endpoint: str, addresses=None, resolve=None, udp=None, expected: dict | None = None,
                timeout: float = UDP_TIMEOUT) -> dict:
    """The UDP compatibility check of one lobby. `addresses` skips DNS (preflight resolved it);
    `expected` is the contract of the release about to ship: it is sent as the client and
    the server's answer must be compatible with it."""
    check = {"endpoint": endpoint}
    try:
        host, port = split_endpoint(endpoint)
    except ProbeError as error:
        return {**check, "ok": False, "error": str(error)}
    if addresses is None:
        try:
            addresses = (resolve or resolve_addresses)(host)
        except OSError as error:
            return {**check, "ok": False, "error": f"DNS: {type(error).__name__}"}
    if not addresses:
        return {**check, "ok": False, "error": f"DNS: {host} has no address"}
    addresses = sorted(addresses, key=lambda address: ":" in address)    # IPv4 first, order kept
    check["address"] = addresses[0]
    result = (udp or udp_compatibility)(addresses[0], port, expected, timeout)
    check.update(result)
    if result.get("ok") and expected is not None:
        issue = contract_issue(expected, result["server"])
        check.update(compatible=issue is None, issue=issue, ok=issue is None)
        if issue:
            check["error"] = f"the server's {issue} differs from the expected contract"
    return check


# ---------------------------------------------------------------- HTTP checks

def http_status(url: str, timeout: float = HTTP_TIMEOUT) -> int:
    """The status of GET url (redirects followed); an HTTP error status is a status, not an error."""
    request = urllib.request.Request(url, method="GET", headers={"User-Agent": "omoba-market-probe"})
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            return response.status
    except urllib.error.HTTPError as error:
        return error.code


def http_check(url: str, good, fetch=None) -> dict:
    try:
        status = (fetch or http_status)(url)
    except (OSError, ValueError) as error:
        reason = getattr(error, "reason", None)
        detail = type(reason).__name__ if isinstance(reason, BaseException) else type(error).__name__
        return {"url": url, "ok": False, "error": detail}
    check = {"url": url, "ok": status in good, "status": status}
    if not check["ok"]:
        check["error"] = f"HTTP {status}"
    return check


def run_checks(settings: dict, resolve=None, fetch=None, udp=None) -> dict:
    """One round over the configured targets: {check name: result}."""
    checks = {}
    if settings.get("OMOBA_PROBE_LOBBY"):
        checks["lobby_udp"] = lobby_check(settings["OMOBA_PROBE_LOBBY"], resolve=resolve, udp=udp)
    if settings.get("OMOBA_PROBE_ACCOUNT_API"):
        checks["account_api"] = http_check(settings["OMOBA_PROBE_ACCOUNT_API"] + "/health/ready", (200,), fetch)
    if settings.get("OMOBA_PROBE_PORTAL"):
        checks["portal"] = http_check(settings["OMOBA_PROBE_PORTAL"] + "/", range(200, 400), fetch)
    return checks


# ---------------------------------------------------------------- settings

def parse_env_text(text: str) -> tuple[dict, list]:
    """(values, problems) of a KEY=VALUE file; unknown keys are problems."""
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


def origin_problem(key: str, value: str) -> str | None:
    parts = urllib.parse.urlsplit(value)
    if parts.scheme != "https" or not parts.hostname or parts.path not in ("", "/") or parts.query \
            or parts.fragment or parts.username or parts.password:
        return f"{key} must be an https:// origin, got {value!r}"
    return None


def load_settings(values: dict) -> dict:
    """Validated settings from env values (systemd EnvironmentFile or --env-file)."""
    settings = {key: values[key] for key in KNOWN_KEYS if values.get(key) not in (None, "")}
    for key, default in DEFAULTS.items():
        settings.setdefault(key, default)
    problems = []
    market = settings.get("OMOBA_PROBE_MARKET", "")
    if not MARKET_KEY.fullmatch(market):
        problems.append("OMOBA_PROBE_MARKET must name the market (ru, global, ...)")
    targets = [key for key in TARGET_KEYS if settings.get(key) and PLACEHOLDER not in settings[key]]
    if not targets:
        problems.append("no target: set OMOBA_PROBE_LOBBY, OMOBA_PROBE_ACCOUNT_API and/or OMOBA_PROBE_PORTAL "
                        f"(`{TOOL} config --market <market>`)")
    if settings.get("OMOBA_PROBE_LOBBY"):
        try:
            host, _ = split_endpoint(settings["OMOBA_PROBE_LOBBY"])
            if not DNS_NAME.fullmatch(host):
                ipaddress.ip_address(host)
        except (ProbeError, ValueError):
            problems.append(f"OMOBA_PROBE_LOBBY must be <dns name or address>:<port>, got "
                            f"{settings['OMOBA_PROBE_LOBBY']!r}")
    for key in ("OMOBA_PROBE_ACCOUNT_API", "OMOBA_PROBE_PORTAL"):
        if settings.get(key):
            settings[key] = settings[key].rstrip("/")
            problem = origin_problem(key, settings[key])
            if problem:
                problems.append(problem)
    for key, low in (("OMOBA_PROBE_FAILURES", 1), ("OMOBA_PROBE_REPEAT_MINUTES", 1)):
        if not re.fullmatch(r"\d{1,4}", settings[key]) or int(settings[key]) < low:
            problems.append(f"{key} must be a whole number >= {low}")
    token, chat = (settings.get(key, "") for key in ("OMOBA_TELEGRAM_BOT_TOKEN", "OMOBA_TELEGRAM_CHAT_ID"))
    if bool(token and PLACEHOLDER not in token) != bool(chat and PLACEHOLDER not in chat):
        problems.append("set both OMOBA_TELEGRAM_BOT_TOKEN and OMOBA_TELEGRAM_CHAT_ID, or neither")
    if problems:
        raise ProbeError("; ".join(problems))
    return settings


def read_env_file(path: str) -> dict:
    try:
        text = Path(path).read_text()
    except OSError as error:
        raise ProbeError(f"cannot read {path}: {error.strerror or error}") from None
    values, problems = parse_env_text(text)
    if problems:
        raise ProbeError(f"{path}: " + "; ".join(problems))
    return values


def market_targets(key: str) -> dict:
    """The market's public names from scripts/markets.py (a checkout only; the probe host has
    the settings file `config` wrote)."""
    scripts = str(Path(__file__).resolve().parent)
    if scripts not in sys.path:
        sys.path.insert(0, scripts)
    import markets
    preset = markets.market(key)
    if preset.blocked:
        raise ProbeError(f"the {key} market is blocked: {preset.blocked}")
    targets = {"OMOBA_PROBE_MARKET": key, "OMOBA_PROBE_LOBBY": preset.lobby,
               "OMOBA_PROBE_ACCOUNT_API": preset.account_api, "OMOBA_PROBE_PORTAL": preset.portal}
    if not all(targets.values()):
        raise ProbeError(f"scripts/markets.py names no lobby, Account API and portal of its own for {key}")
    return targets


CONFIG_TEXT = """# OMOBA external probe of the {market} market (LR-17): ops/probe/README.md.
# Written by `python3 {tool} config --market {market}` from scripts/markets.py: change the
# names there and write this file again; never edit them here. Root-only (chmod 600):
# it holds the Telegram bot token. Never commit real values.
OMOBA_PROBE_MARKET={market}
OMOBA_PROBE_LOBBY={lobby}
OMOBA_PROBE_ACCOUNT_API={account_api}
OMOBA_PROBE_PORTAL={portal}
# Alert after this many failed rounds in a row (one round a minute); remind while down.
OMOBA_PROBE_FAILURES=2
OMOBA_PROBE_REPEAT_MINUTES=60
# The owner's Telegram bot (launch-readiness defaults); without it alerts stay in the journal.
OMOBA_TELEGRAM_BOT_TOKEN={placeholder}
OMOBA_TELEGRAM_CHAT_ID={placeholder}
"""


def config_text(key: str) -> str:
    targets = market_targets(key)
    return CONFIG_TEXT.format(market=key, tool=TOOL, lobby=targets["OMOBA_PROBE_LOBBY"],
                              account_api=targets["OMOBA_PROBE_ACCOUNT_API"], portal=targets["OMOBA_PROBE_PORTAL"],
                              placeholder=PLACEHOLDER)


# ---------------------------------------------------------------- state and alerts

def redact(text: str, token: str = "") -> str:
    if token and len(token) >= 4:
        text = text.replace(token, "<redacted>")
    return re.sub(r"bot\d+:[A-Za-z0-9_-]{20,}", "bot<redacted>", text)


def clock(moment: float) -> str:
    return dt.datetime.fromtimestamp(moment, dt.timezone.utc).strftime("%Y-%m-%d %H:%M UTC")


def describe(name: str, check: dict) -> str:
    target = check.get("endpoint") or check.get("url") or name
    label = {"lobby_udp": "lobby UDP", "account_api": "Account API", "portal": "portal"}.get(name, name)
    return f"{label} {target}"


def telegram(settings: dict, text: str, opener=None) -> bool:
    token, chat = settings.get("OMOBA_TELEGRAM_BOT_TOKEN", ""), settings.get("OMOBA_TELEGRAM_CHAT_ID", "")
    if not token or not chat or PLACEHOLDER in token or PLACEHOLDER in chat:
        return False
    data = urllib.parse.urlencode({"chat_id": chat, "text": text[:3500], "disable_web_page_preview": "true"}).encode()
    request = urllib.request.Request(f"https://api.telegram.org/bot{token}/sendMessage", data=data, method="POST")
    try:
        with (opener or urllib.request.urlopen)(request, timeout=20):
            return True
    except (urllib.error.URLError, OSError, ValueError) as error:
        print(f"<4>the Telegram alert was not delivered: {redact(str(error), token)}", file=sys.stderr, flush=True)
        return False


def advance(state: dict, checks: dict, settings: dict, now: float) -> tuple[list, list]:
    """Update the per-check state with one round; (lines to alert, lines of recoveries)."""
    threshold = int(settings["OMOBA_PROBE_FAILURES"])
    repeat = int(settings["OMOBA_PROBE_REPEAT_MINUTES"]) * 60
    entries = state.setdefault("checks", {})
    alerts, recoveries = [], []
    for name in list(entries):
        if name not in checks:
            del entries[name]            # a target that is no longer configured
    for name, check in checks.items():
        entry = entries.setdefault(name, {"failures": 0})
        if check["ok"]:
            if entry.get("alerted_at") is not None:
                minutes = max(1, round((now - entry.get("since", now)) / 60))
                recoveries.append(f"- {describe(name, check)} answers again (down about {minutes} min)")
            entries[name] = {"failures": 0}
            continue
        if entry["failures"] == 0:
            entry["since"] = now
        entry["failures"] += 1
        entry["error"] = check.get("error", "failed")
        sent = entry.get("alerted_at")
        if entry["failures"] >= threshold and (sent is None or now - sent >= repeat):
            entry["alerted_at"] = now
            alerts.append(f"- {describe(name, check)}: {entry['error']} ({entry['failures']} checks in a row, "
                          f"since {clock(entry['since'])})")
    return alerts, recoveries


def load_state(path: Path) -> dict:
    try:
        state = json.loads(path.read_text())
        return state if isinstance(state, dict) else {}
    except (FileNotFoundError, ValueError):
        return {}


def save_state(path: Path, state: dict) -> None:
    temporary = path.with_name(path.name + ".next")
    temporary.write_text(json.dumps(state, indent=2, sort_keys=True))
    os.replace(temporary, path)


def run_round(settings: dict, state_dir: str, now=time.time, resolve=None, fetch=None, udp=None,
              opener=None, host: str | None = None) -> dict:
    """The timer's job: one round, the state and its alerts."""
    path = Path(state_dir) / "state.json"
    state = load_state(path)
    checks = run_checks(settings, resolve=resolve, fetch=fetch, udp=udp)
    moment = now()
    alerts, recoveries = advance(state, checks, settings, moment)
    market = settings["OMOBA_PROBE_MARKET"]
    where = host or socket.gethostname()
    token = settings.get("OMOBA_TELEGRAM_BOT_TOKEN", "")
    delivered = {}
    for kind, lines, priority in (("down", alerts, 3), ("up", recoveries, 5)):
        if not lines:
            continue
        title = f"OMOBA {market}: " + ("down" if kind == "down" else "back up") + f" (external probe on {where})"
        text = redact(title + "\n" + "\n".join(lines), token)
        print(f"<{priority}>{text}", file=sys.stderr, flush=True)
        delivered[kind] = telegram(settings, text, opener)
    state["last_round"] = {"at": clock(moment), "ok": all(check["ok"] for check in checks.values())}
    save_state(path, state)
    return {"market": market, "ok": state["last_round"]["ok"], "checks": checks,
            "alerted": alerts, "recovered": recoveries, "telegram": delivered}


# ---------------------------------------------------------------- command line

def settings_from(args) -> dict:
    if getattr(args, "market", None):
        return load_settings(market_targets(args.market))
    if args.env_file:
        return load_settings(read_env_file(args.env_file))
    return load_settings({key: value for key, value in os.environ.items() if key in KNOWN_KEYS})


def main(argv=None, stdout=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    config = sub.add_parser("config", help="write the probe host's settings for a market from scripts/markets.py")
    config.add_argument("--market", required=True)
    config.add_argument("--out", help="write here (0600, never replaced) instead of stdout")
    check = sub.add_parser("check", help="one round, no state and no alerts; exit 1 when a check fails")
    source = check.add_mutually_exclusive_group(required=True)
    source.add_argument("--market", help="the market's names from scripts/markets.py (a checkout)")
    source.add_argument("--env-file", help="an installed settings file")
    check.add_argument("--json", action="store_true")
    run = sub.add_parser("run", help="one round with state and alerts (the probe host's timer)")
    run.add_argument("--env-file", help="settings file (default: the environment, as systemd passes it)")
    run.add_argument("--state", help="state folder (default: $STATE_DIRECTORY from systemd)")
    args = parser.parse_args(argv)
    stdout = stdout or sys.stdout
    try:
        if args.command == "config":
            text = config_text(args.market)
            if not args.out:
                stdout.write(text)
                return 0
            descriptor = os.open(args.out, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(descriptor, "w") as handle:
                handle.write(text)
            print(f"wrote {args.out}: add the Telegram bot, then install it (ops/probe/README.md)", file=sys.stderr)
            return 0
        settings = settings_from(args)
        if args.command == "check":
            checks = run_checks(settings)
            ok = all(check["ok"] for check in checks.values())
            if args.json:
                print(json.dumps({"market": settings["OMOBA_PROBE_MARKET"], "ok": ok, "checks": checks},
                                 indent=2, sort_keys=True), file=stdout)
            else:
                print("ok" if ok else "DOWN", file=stdout)
                for name, check in checks.items():
                    print(f"  {'ok  ' if check['ok'] else 'FAIL'} {describe(name, check)}"
                          + ("" if check["ok"] else f": {check.get('error')}"), file=stdout)
            return 0 if ok else 1
        state = args.state or os.environ.get("STATE_DIRECTORY", "").split(":")[0]
        if not state or not os.path.isdir(state):
            raise ProbeError("run needs a state folder: --state DIR, or StateDirectory= in the unit")
        print(json.dumps(run_round(settings, state), sort_keys=True), file=stdout)
        return 0
    except FileExistsError:
        print(f"ERROR: {args.out} exists; it is never replaced", file=sys.stderr)
        return 2
    except ProbeError as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
