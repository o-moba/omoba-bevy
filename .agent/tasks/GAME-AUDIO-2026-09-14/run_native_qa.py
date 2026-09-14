"""Isolated real-UDP native audio/settings captures; stops only its own subprocesses."""
import json
import os
from pathlib import Path
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[3]
TASK = Path(__file__).resolve().parent
BIN = ROOT.parent / "omoba-bevy-match-progression/.agent/tasks/MATCH-PROGRESSION-2026-09-14/target/debug"
BASE = dict(os.environ)
for name in list(BASE):
    if (name.startswith("OMOBA_") and "QA" in name) or name in (
        "OMOBA_DATABASE_URL", "OMOBA_TEST_DATABASE_URL", "OMOBA_AUTOJOIN",
        "OMOBA_AVATAR_MANIFEST", "OMOBA_TOUCH_CONTROLS",
    ):
        BASE.pop(name)
BASE.update(OMOBA_ASSET_DIR=str(ROOT / "client/assets"), OMOBA_DEBUG_UI="0")

def stop(process):
    if process and process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)

for index, (label, width, height, mode, touch) in enumerate([
    ("desktop-3d", 1280, 720, "models3d", False),
    ("phone-3d", 844, 390, "models3d", True),
    ("phone-2d", 844, 390, "sprite2d", True),
]):
    output = TASK / "raw" / label
    output.mkdir(parents=True, exist_ok=True)
    if any(output.iterdir()):
        raise RuntimeError(f"Archive the previous {label} capture before rerunning QA")
    address = f"127.0.0.1:{40527 + index}"
    config = TASK / "runtime" / uuid.uuid4().hex / label / "client"
    server = client = None
    with (output / "server.log").open("w") as slog, (output / "client.log").open("w") as clog:
        try:
            server = subprocess.Popen([BIN / "server"], cwd=ROOT,
                env=dict(BASE, SERVER_ADDR=address, OMOBA_MATCH_MODE="practice", OMOBA_TEAM_SIZE="5",
                    OMOBA_CAREER_OUTBOX=str(TASK / "runtime" / label / "outbox")), stdout=slog, stderr=subprocess.STDOUT)
            deadline = time.monotonic() + 10
            while "is listening" not in (output / "server.log").read_text():
                if server.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError(f"{label}: server did not start")
                time.sleep(0.1)
            env = dict(BASE, GAME_SERVER_ADDR=address, OMOBA_PLAYER_VISUAL_MODE=mode, OMOBA_AUTOJOIN="mage:agnes:green",
                OMOBA_AUDIO_QA_OUTPUT=str(output), OMOBA_QA_WIDTH=str(width), OMOBA_QA_HEIGHT=str(height),
                OMOBA_CLIENT_CONFIG_DIR=str(config))
            if touch:
                env["OMOBA_TOUCH_CONTROLS"] = "1"
            print(f"Running {label} on {address}", flush=True)
            client = subprocess.Popen([BIN / "client"], cwd=ROOT, env=env, stdout=clog, stderr=subprocess.STDOUT)
            deadline = time.monotonic() + 100
            while True:
                receipt = output / "qa-summary.json"
                if receipt.exists():
                    try:
                        summary = json.loads(receipt.read_text())
                        break
                    except json.JSONDecodeError:
                        pass  # Wait for the complete filesystem write.
                failure = output / "qa-failure.json"
                if failure.exists():
                    raise RuntimeError(f"{label}: native QA failed; see {failure}")
                if client.poll() is not None:
                    raise RuntimeError(f"{label}: client exited without a successful capture receipt")
                if time.monotonic() >= deadline:
                    raise RuntimeError(f"{label}: native capture timed out")
                time.sleep(0.1)
            # Completed GPU readbacks and the written receipt define success.
            # Owned-process cleanup below also handles a delayed Bevy shutdown.
            assert summary["status"] == "passed" and summary["real_practice_server"]
            assert summary["diagnostics"]["music_sink"] and summary["diagnostics"]["observed_effect_sinks"] > 0
            assert summary["final_settings"]["muted"] is False
            for capture in summary["captures"]:
                assert (output / capture["file"]).is_file()
            saved = json.loads((config / "client_preferences.json").read_text())
            assert abs(saved["audio"]["music"] - 0.2) < 0.001 and saved["audio"]["muted"] is False
            print(f"PASS {label}: {len(summary['captures'])} captures, real audio sinks and saved settings", flush=True)
        finally:
            stop(client)
            stop(server)
