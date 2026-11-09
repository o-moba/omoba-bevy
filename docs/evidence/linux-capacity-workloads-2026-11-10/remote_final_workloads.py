"""Read-only final host facts for the workload evidence packet."""
import datetime
import json
import os
from pathlib import Path
import subprocess


def call(command, input=None):
    return subprocess.run(command, input=input, text=True, capture_output=True, check=True).stdout.strip()


mem = {}
for line in Path("/proc/meminfo").read_text().splitlines():
    key, value = line.split(":", 1)
    if key in ("MemTotal", "MemAvailable"):
        mem[key] = int(value.strip().split()[0])
processes = call(["ps", "-axo", "pid=,args="])
owned = [line.strip() for line in processes.splitlines()
         if "/root/omoba-capacity-73651bc6/" in line
         and any(name in line for name in ("omoba-server", "/bots", "server-native-scope"))]
sql = "SELECT (SELECT count(*) FROM pg_database WHERE datname='omoba_test'), (SELECT count(*) FROM pg_roles WHERE rolname LIKE 'omoba_cap_%');"
db_counts = call(["docker", "exec", "supabase_db_ekza-dev", "psql", "-X", "-U", "postgres",
                  "-d", "postgres", "-At", "-c", sql])
services = {}
for service in ("omoba-lobby-dev", "omoba-account-api-dev"):
    services[service] = call(["systemctl", "is-active", service])
print(json.dumps({
    "observed_after_runs_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "logical_cpus": os.cpu_count(),
    "mem_total_kib": mem["MemTotal"],
    "mem_available_kib_at_final_check": mem["MemAvailable"],
    "owned_capacity_processes": owned,
    "disposable_omoba_test_database_count": int(db_counts.split("|")[0]),
    "disposable_capacity_role_count": int(db_counts.split("|")[1]),
    "dev_services": services,
    "scope": "final read-only checks on the shared dev VPS; not a production/RU host or uplink measurement",
}, indent=2))
