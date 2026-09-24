#!/usr/bin/env python3
"""Run the PostgreSQL-backed Rust tests against a disposable database.

These tests are `#[ignore]`d in the normal gate because they need a real
PostgreSQL server. This script prepares one database the way production is
prepared (career and portal migrations, the restricted runtime roles from
`account-api/ops/grants.sql`) and then runs every test of `omoba-career-store`,
`omoba-account-api` and `server`, ignored ones included.

Required: OMOBA_TEST_DATABASE_URL, an owner URL of a database that may be
freely written and dropped (never production). Optional:
OMOBA_PORTAL_TEST_DATABASE_URL (defaults to the same database) and
OMOBA_PORTAL_ROLE_TEST_URL (defaults to the same database logged in as the
restricted portal role this script creates). Needs `psql` on PATH.

Used by `make test-postgres` and the `postgres` CI job.
"""
import os
import subprocess
import sys
from pathlib import Path
from urllib.parse import quote, urlsplit, urlunsplit

ROOT = Path(__file__).resolve().parents[1]
PORTAL_ROLE = "omoba_portal_test"
GAME_ROLE = "omoba_game_test"
# Local disposable role only; the database itself must never be production.
PORTAL_ROLE_PASSWORD = "omoba_portal_test"
CRATES = ("omoba-career-store", "omoba-account-api", "server")


def with_credentials(url, user, password):
    """Return `url` with its user and password replaced (host, port, path kept)."""
    parts = urlsplit(url)
    if parts.scheme not in ("postgres", "postgresql") or not parts.hostname:
        raise ValueError("expected a postgres://user@host/database URL")
    host = parts.hostname
    if ":" in host:
        host = f"[{host}]"
    if parts.port:
        host = f"{host}:{parts.port}"
    netloc = f"{quote(user, safe='')}:{quote(password, safe='')}@{host}"
    return urlunsplit((parts.scheme, netloc, parts.path, parts.query, parts.fragment))


def run(command, env):
    print("+", " ".join(command), flush=True)
    subprocess.run(command, cwd=ROOT, env=env, check=True)


def psql(url, env, *arguments):
    run(["psql", url, "--no-psqlrc", "-v", "ON_ERROR_STOP=1", "-q", *arguments], env)


def main(argv=None):
    extra = list(sys.argv[1:] if argv is None else argv)
    url = os.environ.get("OMOBA_TEST_DATABASE_URL", "")
    if not url:
        sys.exit("OMOBA_TEST_DATABASE_URL is not set: point it at a disposable PostgreSQL database")
    env = dict(os.environ)
    env.setdefault("OMOBA_PORTAL_TEST_DATABASE_URL", url)
    if not env.get("OMOBA_PORTAL_ROLE_TEST_URL"):
        try:
            env["OMOBA_PORTAL_ROLE_TEST_URL"] = with_credentials(
                url, PORTAL_ROLE, PORTAL_ROLE_PASSWORD
            )
        except ValueError as error:
            sys.exit(f"{error}; or set OMOBA_PORTAL_ROLE_TEST_URL for the {PORTAL_ROLE} role")
    # Tests that would otherwise skip without a URL must fail instead.
    env["OMOBA_REQUIRE_TEST_DATABASE"] = "1"

    # Fail loudly and early when the database is unreachable.
    psql(url, env, "-c", "SELECT 1")
    migrate_env = dict(env, OMOBA_DATABASE_URL=url)
    run(["cargo", "run", "--locked", "-q", "-p", "omoba-account-api", "--", "migrate"], migrate_env)
    psql(
        url,
        env,
        "-c",
        f"""DO $$ BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{PORTAL_ROLE}') THEN
    CREATE ROLE {PORTAL_ROLE} LOGIN PASSWORD '{PORTAL_ROLE_PASSWORD}';
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{GAME_ROLE}') THEN
    CREATE ROLE {GAME_ROLE} NOLOGIN;
  END IF;
END $$;""",
    )
    psql(
        url,
        env,
        "-v",
        f"portal_role={PORTAL_ROLE}",
        "-v",
        f"game_role={GAME_ROLE}",
        "-f",
        "account-api/ops/grants.sql",
    )
    command = ["cargo", "test", "--locked"]
    for crate in CRATES:
        command += ["-p", crate]
    run(command + ["--", "--include-ignored", "--test-threads=1", *extra], env)


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        sys.exit(error.returncode)
