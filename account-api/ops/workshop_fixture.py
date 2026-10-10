#!/usr/bin/env python3
"""Disposable workshop fixture. Never reads ambient database URLs or removes data."""
import argparse
import os
from pathlib import Path
import secrets
import subprocess
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
TASK = ROOT / ".agent/tasks/CLASS-WORKSHOP-20261010"
DATA = TASK / "postgres"
PORT = 55581


def run(command, **kwargs):
    return subprocess.run([str(x) for x in command], check=True, cwd=ROOT, **kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["init", "serve", "status"])
    parser.add_argument("--postgres-bin", type=Path,
                        default=Path("/Applications/Postgres.app/Contents/Versions/18/bin"))
    parser.add_argument("--target-dir", type=Path,
                        default=Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")))
    args = parser.parse_args()
    pg = args.postgres_bin
    api = args.target_dir / "debug/omoba-account-api"
    seed = args.target_dir / "debug/examples/seed-local-portal"
    if args.action == "status":
        run([pg / "pg_ctl", "-D", DATA, "status"])
        with urllib.request.urlopen("http://127.0.0.1:40560/health/ready", timeout=3) as response:
            print("Account API ready:", response.status)
        return
    if not api.is_file():
        parser.error("Build omoba-account-api first; --target-dir selects its existing cache")
    if args.action == "init":
        if not seed.is_file():
            parser.error("Build --example seed-local-portal first")
        TASK.mkdir(parents=True, exist_ok=True)
        if not DATA.exists():
            run([pg / "initdb", "-D", DATA, "-U", "workshop_owner", "--auth=trust",
                 "--encoding=UTF8", "--no-locale"])
        elif not (DATA / "PG_VERSION").is_file():
            parser.error("Refusing an existing directory that is not this PostgreSQL fixture")
        running = subprocess.run([str(pg / "pg_ctl"), "-D", str(DATA), "status"],
                                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0
        if not running:
            run([pg / "pg_ctl", "-D", DATA, "-l", TASK / "postgres.log", "-o",
                 f"-h 127.0.0.1 -p {PORT} -k /private/tmp", "-w", "start"])
        owner = f"postgres://workshop_owner@127.0.0.1:{PORT}/postgres"
        verify_cluster(pg, parser)
        for role in ["workshop_portal", "workshop_game"]:
            run([pg / "psql", owner, "-v", "ON_ERROR_STOP=1", "-c",
                 f"DO $$ BEGIN IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname='{role}') "
                 f"THEN CREATE ROLE {role} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT; "
                 "END IF; END $$;"])
        for database in ["workshop_test", "workshop_demo"]:
            exists = run([pg / "psql", owner, "-At", "-c",
                          f"SELECT 1 FROM pg_database WHERE datname='{database}'"],
                         capture_output=True, text=True).stdout.strip()
            if exists != "1":
                run([pg / "createdb", "-h", "127.0.0.1", "-p", str(PORT),
                     "-U", "workshop_owner", database])
            url = f"postgres://workshop_owner@127.0.0.1:{PORT}/{database}"
            env = clean_env()
            env["OMOBA_DATABASE_URL"] = url
            run([api, "migrate"], env=env)
            run([pg / "psql", url, "-v", "ON_ERROR_STOP=1", "-v",
                 "portal_role=workshop_portal", "-v", "game_role=workshop_game", "-f",
                 ROOT / "account-api/ops/grants.sql"])
        env = clean_env()
        env["OMOBA_PORTAL_TEST_DATABASE_URL"] = f"postgres://workshop_owner@127.0.0.1:{PORT}/workshop_demo"
        run([seed], env=env)
        print("Local workshop fixture initialized; no hosted accounts or services were changed.")
        return
    if not (DATA / "PG_VERSION").is_file():
        parser.error("Run init before serve")
    verify_cluster(pg, parser)
    secret = TASK / "portal-secret"
    if not secret.exists():
        fd = os.open(secret, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w") as output:
            output.write(secrets.token_hex(32))
    env = clean_env()
    env.update(OMOBA_DATABASE_URL=f"postgres://workshop_portal@127.0.0.1:{PORT}/workshop_demo",
               OMOBA_PORTAL_ORIGIN="http://127.0.0.1:3010", OMOBA_ALLOW_INSECURE_LOCAL="1",
               OMOBA_ACCOUNT_BIND="127.0.0.1:40560", OMOBA_PORTAL_SECRET=secret.read_text().strip(),
               OMOBA_EKZA_LOCAL_FIXTURE_ORIGIN="http://127.0.0.1:40561")
    os.execve(api, [str(api)], env)


def verify_cluster(pg, parser):
    # Both init and serve must verify the explicit endpoint before trusting it.
    owner = f"postgres://workshop_owner@127.0.0.1:{PORT}/postgres"
    current = run([pg / "psql", owner, "-At", "-c", "SHOW data_directory"],
                  capture_output=True, text=True).stdout.strip()
    if Path(current).resolve() != DATA.resolve():
        parser.error("Port 55581 belongs to a different cluster; refusing to use it")


def clean_env():
    # Avoid inherited production billing/provider credentials or configuration.
    return {key: value for key, value in os.environ.items() if not key.startswith("OMOBA_")}


if __name__ == "__main__":
    main()
