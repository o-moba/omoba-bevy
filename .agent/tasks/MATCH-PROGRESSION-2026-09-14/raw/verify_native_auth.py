"""Real native client/device key -> signed UDP -> isolated PostgreSQL smoke.

Reads only the public key field into evidence; never records the private seed.
The database URL is supplied by the caller, and no production service is used.
"""
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[4]
RAW = Path(__file__).resolve().parent
BIN = RAW.parent / "target/debug"
PSQL = "/Applications/Postgres.app/Contents/Versions/18/bin/psql"
DATABASE = os.environ["OMOBA_TEST_DATABASE_URL"]
CONFIG = Path(tempfile.mkdtemp(prefix="native-auth-config-", dir=RAW))
PROOF = RAW / "native-auth-proof.json"
ADDR = "127.0.0.1:55440"

def clean_env():
    return {k: v for k, v in os.environ.items()
            if not k.startswith("OMOBA_") and k not in ("SERVER_ADDR", "GAME_SERVER_ADDR")}

def stop(process):
    if process and process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)

server = None
client = None
proof = {"status": "failed", "method": "real native client device identity and server UDP handshake, then PostgreSQL lookup", "fixture": False, "private_key_recorded": False}
try:
    with (RAW / "native-auth-server.log").open("w") as slog, (RAW / "native-auth-client.log").open("w") as clog:
        env = clean_env()
        env.update(SERVER_ADDR=ADDR, OMOBA_DATABASE_URL=DATABASE, OMOBA_MATCH_MODE="dev",
                   OMOBA_CAREER_OUTBOX=str(RAW / "native-auth-outbox"))
        server = subprocess.Popen([str(BIN / "server")], cwd=ROOT, env=env, stdout=slog, stderr=subprocess.STDOUT)
        time.sleep(2)
        assert server.poll() is None, "server exited before client started"
        env = clean_env()
        env.update(GAME_SERVER_ADDR=ADDR, OMOBA_CLIENT_CONFIG_DIR=str(CONFIG), OMOBA_TOUCH_CONTROLS="0")
        client = subprocess.Popen([str(BIN / "client")], cwd=ROOT, env=env, stdout=clog, stderr=subprocess.STDOUT)
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            assert server.poll() is None and client.poll() is None, "a native process exited"
            key_file = CONFIG / "career_identity.json"
            if key_file.exists():
                public = json.loads(key_file.read_text())["public_key"]
                assert re.fullmatch("[0-9a-f]{64}", public)
                query = "SELECT row_to_json(p) FROM career_profiles p JOIN career_keys k USING(profile_id) WHERE k.public_key='" + public + "'"
                result = subprocess.run([PSQL, DATABASE, "-X", "-t", "-A", "-c", query], capture_output=True, text=True)
                if result.returncode == 0 and result.stdout.strip():
                    profile = json.loads(result.stdout)
                    assert profile["rating"] == 1000 and profile["matches_played"] == 0
                    proof.update(status="passed", profile=profile, client_alive=True, server_alive=True,
                                 key_permissions=oct(key_file.stat().st_mode & 0o777),
                                 authenticated_key_is_public_only=True)
                    break
            time.sleep(0.5)
        assert proof["status"] == "passed", "native identity did not reach PostgreSQL within 60 seconds"
finally:
    stop(client)
    stop(server)
    PROOF.write_text(json.dumps(proof, indent=2) + "\n")
    print(json.dumps(proof, indent=2))
