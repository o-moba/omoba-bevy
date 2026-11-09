"""Run one exact-release disposable Linux workload and always clean its fixture."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import secrets
import signal
import socket
import subprocess
import sys
import time

parser = argparse.ArgumentParser()
parser.add_argument("--workload", choices=("casual", "ranked", "bots", "mixed"), required=True)
parser.add_argument("--clients", type=int, required=True)
parser.add_argument("--seconds", type=int, default=30)
parser.add_argument("--label", required=True)
parser.add_argument("--port", type=int, required=True)
parser.add_argument("--worker-port", type=int, required=True)
args = parser.parse_args()
config = json.loads(Path(__file__).with_name("fixture.json").read_text())
root = Path(config["remote_root"])
server = root / "omoba-server"
bots = root / "bots"
helper = root / "examples/qualify-ranked-fixture"
server_scope = root / "server-native-scope"
output = root / args.label
container = "supabase_db_ekza-dev"
database = "omoba_test"
role_stem = "omoba_cap_" + args.label.replace("-", "_")
game_role = role_stem + "_game"
portal_role = role_stem + "_portal"
password = secrets.token_hex(24)
original_run = subprocess.run
summary = {
    "status": "FAIL", "label": args.label, "workload": args.workload,
    "clients": args.clients, "seconds": args.seconds,
    "generator": "exact native Rust release bots; workload-aware independent ten-client shards",
    "source_parent": config["source_parent"], "release_run": config["release_run"],
    "server_sha256": hashlib.sha256(server.read_bytes()).hexdigest(),
    "bots_sha256": hashlib.sha256(bots.read_bytes()).hexdigest(),
    "helper_sha256": hashlib.sha256(helper.read_bytes()).hexdigest() if helper.exists() else None,
    "server_load_metrics": True, "database": database, "scope": config["scope"],
}
created = False
expected_binaries = json.loads(Path(__file__).with_name("artifact-hashes.json").read_text())


def call(command, **kwargs):
    return original_run(command, capture_output=True, text=True, **kwargs)


def sql(text, db="postgres"):
    result = call(["docker", "exec", "-i", container, "psql", "-X", "-U", "postgres",
                   "-d", db, "-At", "-v", "ON_ERROR_STOP=1"], input=text, timeout=60)
    if result.returncode:
        raise RuntimeError(result.stderr.replace(password, "[fixture-secret]")[-2000:])
    return result.stdout.strip()


def owned_processes():
    result = call(["ps", "-axo", "pid=,args="], check=True)
    rows = []
    for line in result.stdout.splitlines():
        stripped = line.strip()
        if not stripped:
            continue
        pid, command = stripped.split(None, 1)
        if any(str(path) in command for path in (server, bots, server_scope)):
            rows.append(int(pid))
    return rows


try:
    assert root.is_dir() and server.is_file() and bots.is_file() and server_scope.is_file()
    assert summary["server_sha256"] == expected_binaries["server_sha256"]
    assert summary["bots_sha256"] == expected_binaries["bots_sha256"]
    assert summary["helper_sha256"] == expected_binaries["helper_sha256"]
    assert not owned_processes(), "owned capacity processes already exist"
    assert not output.exists(), "output already exists"
    shards = args.clients // 10
    if args.workload == "bots":
        arenas = args.clients
    elif args.workload == "mixed":
        assert shards >= 3 and shards % 3 == 0
        arenas = (shards // 3) * 12
    else:
        arenas = shards
    summary["arenas"] = arenas
    if args.workload in ("ranked", "mixed"):
        assert helper.is_file() and os.access(helper, os.X_OK)
    for port in [args.port, *range(args.worker_port, args.worker_port + arenas)]:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.bind(("127.0.0.1", port))
    assert sql(f"SELECT count(*) FROM pg_database WHERE datname='{database}'") == "0", "refusing to reuse omoba_test"
    sql(f"CREATE ROLE {game_role} LOGIN PASSWORD '{password}'; CREATE ROLE {portal_role} NOLOGIN;")
    sql(f"CREATE DATABASE {database};")
    created = True
    sql("CREATE TABLE career_schema_version(version integer primary key)", database)
    migrations = {}
    for migration in sorted((root / "career-store/migrations/postgres").glob("[0-9]*.sql")):
        version = int(migration.name.split("_", 1)[0])
        text = migration.read_text()
        migrations[str(migration.relative_to(root))] = hashlib.sha256(migration.read_bytes()).hexdigest()
        if "INSERT INTO career_schema_version" not in text:
            text += f"\nINSERT INTO career_schema_version VALUES({version});"
        sql("BEGIN;" + text + "\nCOMMIT;", database)
    for migration in sorted((root / "account-api/migrations").glob("[0-9]*.sql")):
        migrations[str(migration.relative_to(root))] = hashlib.sha256(migration.read_bytes()).hexdigest()
        sql("BEGIN;" + migration.read_text() + "\nCOMMIT;", database)
    sql("REVOKE CREATE ON SCHEMA public FROM PUBLIC;", database)
    grants = root / "account-api/ops/grants.sql"
    migrations[str(grants.relative_to(root))] = hashlib.sha256(grants.read_bytes()).hexdigest()
    sql(f"\\set game_role {game_role}\n\\set portal_role {portal_role}\n" + grants.read_text(), database)
    summary["migration_sha256"] = migrations
    summary["career_schema"] = int(sql("SELECT max(version) FROM career_schema_version", database))
    summary["portal_schema"] = int(sql("SELECT max(version) FROM portal.schema_version", database))
    assert summary["career_schema"] == 13 and summary["portal_schema"] == 11

    env = {key: value for key, value in os.environ.items() if not key.startswith("OMOBA_")}
    env.update(OMOBA_TEST_DATABASE_URL=f"postgres://{game_role}:{password}@127.0.0.1:54322/{database}",
               PATH=f"{root}/bin:{env.get('PATH', '')}")
    os.environ.update(env)
    sys.path.insert(0, str(root / "scripts"))
    import public_playing_capacity as capacity

    def source_aware_run(command, **kwargs):
        if command[:3] == ["git", "rev-parse", "HEAD"]:
            return subprocess.CompletedProcess(command, 0, stdout=config["source_parent"] + "\n", stderr="")
        if command[:2] == ["git", "status"]:
            return subprocess.CompletedProcess(command, 0, stdout=" M remote-fixture-not-a-git-checkout\n", stderr="")
        return original_run(command, **kwargs)

    capacity.subprocess.run = source_aware_run
    sys.argv = [str(root / "scripts/public_playing_capacity.py"),
                "--server-bin", str(server_scope), "--bots-bin", str(bots),
                "--output", str(output), "--workload", args.workload,
                "--clients", str(args.clients), "--seconds", str(args.seconds),
                "--input-hz", "20", "--timeout", str(max(240, args.seconds + 120)),
                "--port", str(args.port), "--worker-port", str(args.worker_port),
                "--server-load-metrics"]
    if args.workload in ("ranked", "mixed"):
        sys.argv.append("--qualify-ranked-fixtures")
    started = time.monotonic()
    capacity.main()
    summary["elapsed_seconds"] = time.monotonic() - started
    report = json.loads((output / "report.json").read_text())
    summary["probe_status"] = report["status"]
    for key in ("input", "wire", "producer_timing", "server_load", "resources", "ranked_qualification"):
        summary[key] = report.get(key)
    assert report["status"] == "PASS"
    summary["status"] = "PASS"
except Exception as error:
    summary["error"] = type(error).__name__ + ": " + str(error).replace(password, "[fixture-secret]")
    report_path = output / "report.json"
    if report_path.exists():
        report = json.loads(report_path.read_text())
        summary["probe_status"] = report.get("status")
        for key in ("input", "wire", "producer_timing", "server_load", "resources", "ranked_qualification"):
            if key in report:
                summary[key] = report[key]
    raise
finally:
    remaining = owned_processes()
    summary["remaining_before_forced_cleanup"] = remaining
    for pid in remaining:
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    deadline = time.monotonic() + 10
    while owned_processes() and time.monotonic() < deadline:
        time.sleep(.2)
    summary["owned_processes_remain"] = owned_processes()
    if created:
        try:
            sql(f"DROP DATABASE {database};")
        except Exception as error:
            summary["database_cleanup_error"] = str(error).replace(password, "[fixture-secret]")
    try:
        sql(f"DROP ROLE IF EXISTS {portal_role}; DROP ROLE IF EXISTS {game_role};")
    except Exception as error:
        summary["role_cleanup_error"] = str(error).replace(password, "[fixture-secret]")
    summary["database_absent"] = sql(f"SELECT count(*) FROM pg_database WHERE datname='{database}'") == "0"
    summary["roles_absent"] = sql(
        f"SELECT count(*) FROM pg_roles WHERE rolname IN ('{game_role}', '{portal_role}')") == "0"
    if summary["owned_processes_remain"] or not summary["database_absent"] or not summary["roles_absent"]:
        summary["status"] = "FAIL"
    if isinstance(summary.get("server_load"), dict):
        summary["server_load"] = {key: value for key, value in summary["server_load"].items() if key != "workers"}
    (root / f"{args.label}-summary.json").write_text(json.dumps(summary, indent=2) + "\n")

print(json.dumps({key: value for key, value in summary.items()
                  if key not in ("resources", "server_load", "input", "wire", "producer_timing")}, indent=2))
assert summary["status"] == "PASS"
