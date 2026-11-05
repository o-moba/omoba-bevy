#!/usr/bin/env python3
"""Run the opt-in PostgreSQL harness and complete public gameplay rehearsals.

Uses a migrated disposable loopback database, the repository's existing cargo
cache A or B, and private output beneath .agent. Provider replies are local API
fixtures; portal browser UI and real provider certification are separate gates.
"""
import argparse
import json
import os
from pathlib import Path
import re
import signal
import shutil
import stat
import subprocess
import sys
import time
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
METHODS = {"vkid", "ok_ru", "mail_ru"}
API_FIXTURE_TEST = "website_registration_creates_default_accounts_for_every_entry_method"
REGISTRATION_TEST = "registered_provider_accounts_play_and_reconnect_through_the_public_lobby"
ADMISSION_TEST = "three_hundred_clients_reconnecting_after_a_lobby_restart_all_get_in"


def isolated_database_url(raw):
    if any(c in raw for c in "\r\n\t"):
        return False
    try:
        url = urlsplit(raw)
        port = url.port
        return (url.scheme in ("postgres", "postgresql")
                and url.hostname in ("127.0.0.1", "localhost", "::1")
                and url.path == "/omoba_test" and not url.query and not url.fragment
                and (port is None or port > 0))
    except ValueError:
        return False


def database_url(env):
    url = env.get("OMOBA_HARNESS_DATABASE_URL", env.get("OMOBA_TEST_DATABASE_URL", ""))
    if not isolated_database_url(url):
        raise ValueError("set a loopback OMOBA_HARNESS_DATABASE_URL or OMOBA_TEST_DATABASE_URL ending in /omoba_test without URI options")
    return url


def shared_target(env):
    common = subprocess.check_output(["git", "rev-parse", "--path-format=absolute", "--git-common-dir"], cwd=ROOT, text=True).strip()
    primary = Path(common).parent
    allowed = [(primary / name).resolve() for name in ("target", "target-b")]
    selected = Path(env.get("CARGO_TARGET_DIR", primary / "target")).resolve()
    if selected not in allowed or not selected.is_dir():
        raise ValueError("select an existing primary cargo cache A or B in CARGO_TARGET_DIR")
    return selected


def validate_fixture(path):
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_size > 65536 or info.st_mode & 0o077:
        raise ValueError("registration fixture must be a bounded private regular file")
    values = json.loads(path.read_text())
    if not isinstance(values, list) or not all(isinstance(v, dict) for v in values):
        raise ValueError("registration fixture must be an account array")
    if len(values) != len(METHODS) or {v.get("method") for v in values} != METHODS:
        raise ValueError("registration fixture must contain all three provider methods")
    if len({v.get("profile_id") for v in values}) != len(METHODS):
        raise ValueError("registration profiles must be independent")
    if not all(re.fullmatch(r"[0-9a-f]{64}", v.get(field, ""))
               for v in values for field in ("seed", "profile_id")):
        raise ValueError("invalid registration fixture")
    # Never return seed-bearing objects to the report writer.
    return sorted(v["method"] for v in values)


def assert_one_test(log):
    results = re.findall(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;", log.read_text())
    if results != [("1", "0", "0")]:
        raise AssertionError("expected exactly one executed opt-in test, with no ignores")


def process_birth(pid):
    result = subprocess.run(["ps", "-o", "lstart=", "-p", str(pid)],
                            capture_output=True, text=True, check=False)
    return result.stdout.strip() if result.returncode == 0 else None


def stop_registration_groups(log):
    """Only signal a group whose leader still has the recorded birth identity."""
    if not log.exists():
        return
    for pid, birth in re.findall(r"^HARNESS_OWNED_GROUP (\d+) (.+)$", log.read_text(), re.MULTILINE):
        pid = int(pid)
        if process_birth(pid) != birth:
            continue
        try:
            os.killpg(pid, signal.SIGTERM)
        except ProcessLookupError:
            continue
        deadline = time.monotonic() + 20
        while process_birth(pid) == birth and time.monotonic() < deadline:
            time.sleep(0.05)
        if process_birth(pid) == birth:
            try:
                os.killpg(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--ranked-timeout", type=int, default=3000)
    args = parser.parse_args(argv)
    if not 300 <= args.ranked_timeout <= 3600:
        parser.error("ranked timeout must be 300..3600 seconds")
    try:
        url, target = database_url(os.environ), shared_target(os.environ)
    except ValueError as error:
        parser.error(str(error))
    if shutil.disk_usage(ROOT).free < 20 * 1024 ** 3:
        parser.error("less than 20 GiB free on the repository volume")
    for executable in ("cargo", "node"):
        if not shutil.which(executable):
            parser.error(f"missing {executable}")
    output = (args.output or ROOT / ".agent" / f"postgres-harness-{time.time_ns()}").resolve()
    if not output.is_relative_to((ROOT / ".agent").resolve()):
        parser.error("output must be a new directory beneath this checkout's .agent")
    output.mkdir(parents=True, mode=0o700, exist_ok=False)
    output.chmod(0o700)
    fixture = output / "native-private.json"
    env = {k: v for k, v in os.environ.items()
           if not k.startswith(("OMOBA_", "GAME_")) and k not in ("SERVER_ADDR", "HARNESS_SERVER_BIN")}
    env.update(OMOBA_TEST_DATABASE_URL=url, OMOBA_HARNESS_DATABASE_URL=url,
               OMOBA_PORTAL_TEST_DATABASE_URL=url, OMOBA_REQUIRE_TEST_DATABASE="1",
               CARGO_TARGET_DIR=str(target), CARGO_TERM_COLOR="never",
               CARGO_CACHE_AUTO_CLEAN_FREQUENCY="never", HARNESS_SERVER_BIN=str(target / "debug/server"),
               HARNESS_RANKED_FIXTURE_BIN=str(target / "debug/examples/qualify-ranked-fixture"))
    env.setdefault("CARGO_BUILD_JOBS", "1")
    report = {"status": "RUNNING", "stages": [], "fixture_methods": [],
              "scope": "loopback PostgreSQL; Account API provider/enrollment fixtures; real public gameplay",
              "portal_browser_ui": False, "real_provider_accounts": False}
    started = time.monotonic()

    def run(name, command, extra=None, rust_test=False, process_report=None, timeout=3600):
        print(f"RUN {name}", flush=True)
        log = output / f"{name}.log"
        phase_start = time.monotonic()
        stage = {"name": name, "status": "RUNNING", "log": log.name}
        report["stages"].append(stage)
        (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        try:
            with log.open("w") as stream:
                child = subprocess.Popen(command, cwd=ROOT, env=dict(env, **(extra or {})),
                                         stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
                try:
                    code = child.wait(timeout=timeout)
                    if code:
                        raise subprocess.CalledProcessError(code, command)
                except BaseException:
                    if child.poll() is None:
                        os.killpg(child.pid, signal.SIGTERM)
                        try:
                            child.wait(timeout=20)
                        except subprocess.TimeoutExpired:
                            os.killpg(child.pid, signal.SIGKILL)
                            child.wait()
                    raise
            if rust_test:
                assert_one_test(log)
            if process_report:
                proof = json.loads(process_report.read_text())
                if proof.get("status") != "PASS":
                    raise AssertionError("process rehearsal did not pass")
                stage["proof"] = str(process_report.relative_to(output))
            stage["status"] = "PASS"
        except BaseException:
            stage["status"] = "FAIL"
            raise
        finally:
            if name == "registration-game":
                stop_registration_groups(log)
            stage["seconds"] = round(time.monotonic() - phase_start, 2)
            (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")

    def harness(name, target_test, filter_name):
        run(name, ["cargo", "test", "--locked", "-p", "harness", "--test", target_test,
                   filter_name, "--", "--ignored", "--exact", "--test-threads=1", "--nocapture"],
            {"OMOBA_REGISTRATION_JOURNEY_FIXTURE": str(fixture)}, rust_test=True)

    def public(name, options, timeout=420):
        case = output / name
        # Each script owns its complete lobby/worker process group and stops it.
        run(name, ["node", "scripts/test_public_mvp.mjs", *options, f"--output={case}"],
            process_report=case / "report.json", timeout=timeout)

    try:
        run("build", ["cargo", "build", "--locked", "-p", "server", "-p", "harness",
                      "-p", "omoba-account-api", "--bins"])
        run("build-ranked-fixture", ["cargo", "build", "--locked", "-p", "omoba-career-store",
                                     "--example", "qualify-ranked-fixture"])
        run("migrate", [str(target / "debug/omoba-account-api"), "migrate"], {"OMOBA_DATABASE_URL": url})
        harness("admission-300", "admission", ADMISSION_TEST)
        run("registration-api", ["cargo", "test", "--locked", "-p", "omoba-account-api", "--test",
                                 "social_postgres", API_FIXTURE_TEST, "--", "--ignored", "--exact",
                                 "--test-threads=1", "--nocapture"],
            {"OMOBA_REGISTRATION_JOURNEY_FIXTURE": str(fixture)}, rust_test=True)
        report["fixture_methods"] = validate_fixture(fixture)
        harness("registration-game", "registration_journey", REGISTRATION_TEST)
        fixture.unlink()
        public("lifecycle", ["--scenario=lifecycle", "--exercise-stale-handoff", "--seconds=3"])
        public("mixed", ["--mix=humans_only:10,quick:5,bot_practice:2", "--max-wait-secs=60",
                         "--seconds=5", "--exercise-stale-handoff"])
        public("ready-check", ["--scenario=ready-check", "--seconds=1"])
        party = ["--scenario=party-draft", "--clients=10", "--preference=humans_only",
                 "--seconds=1", "--exercise-stale-handoff"]
        public("party-leave", party)
        public("party-disconnect", [*party, "--draft-exit=disconnect"])
        public("ranked-early-crash", ["--scenario=early-recovery", "--clients=10",
                                       "--preference=humans_only", "--seconds=2", "--input-hz=0"])
        public("termination", ["--scenario=termination", "--seconds=1"])
        public("remake", ["--scenario=remake", "--clients=10", "--preference=humans_only",
                          "--seconds=1"], timeout=300)
        public("surrender", ["--scenario=surrender", "--clients=10", "--preference=humans_only",
                             "--seconds=1"], timeout=780)
        eligibility = output / "ranked-eligibility"
        eligibility_timeout = min(args.ranked_timeout, 1200)
        run("ranked-eligibility", [sys.executable, "scripts/public_bots_rehearsal.py",
                                  "--server-bin", str(target / "debug/server"),
                                  "--bots-bin", str(target / "debug/bots"), "--require-victory",
                                  "--ranked-eligibility", "--clients", "1", "--preference", "bot_practice",
                                  "--timeout", str(eligibility_timeout), "--output", str(eligibility)],
            process_report=eligibility / "report.json", timeout=eligibility_timeout + 60)
        case = output / "ranked-victory"
        run("ranked-victory", [sys.executable, "scripts/public_bots_rehearsal.py",
                              "--server-bin", str(target / "debug/server"),
                              "--bots-bin", str(target / "debug/bots"), "--require-victory", "--qualify-ranked-fixtures",
                              "--timeout", str(args.ranked_timeout), "--output", str(case)],
            process_report=case / "report.json", timeout=args.ranked_timeout + 60)
        report["status"] = "PASS"
    except BaseException as error:
        report.update(status="FAIL", error=f"{type(error).__name__}: {error}")
        raise
    finally:
        fixture.unlink(missing_ok=True)
        report.update(private_fixture_removed=not fixture.exists(), seconds=round(time.monotonic() - started, 2))
        (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps(report), flush=True)
    return 0


if __name__ == "__main__":
    def interrupted(_signal, _frame):
        raise InterruptedError("gate interrupted; stopping owned phase and removing private fixture")
    signal.signal(signal.SIGTERM, interrupted)
    raise SystemExit(main())
