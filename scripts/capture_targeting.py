#!/usr/bin/env python3
"""Native desktop/touch targeting proof with independent server snapshots.

Uses an explicitly labeled, local development placement/ambient-world fixture.
Combat input and damage always go through production client/server systems.
No browser, image generator, additional dependency or physical-device claim.
"""
import argparse
import json
import math
import os
from pathlib import Path
import platform
import socket
import subprocess
import tempfile
import time

from capture_verdant import ScenarioPeer, SnapshotObserver, sha256, verify_beta_ui_profile


IMAGES = ("01-target-ready.png", "02-target-preview.png", "03-basic-attack.png",
          "04-target-canceled.png")


class PassiveTarget(ScenarioPeer):
    """Ordinary joined blue player. Never sends movement, casts or attacks."""
    def send(self, data):
        if data.get("type") == "transform":
            return
        if data.get("type") == "join":
            data = dict(data, team="blue", session_id=f"targeting-qa-{os.getpid()}-{self.index}")
        if data.get("type") not in ("hello", "join"):
            raise RuntimeError("passive target attempted a gameplay command")
        super().send(data)


def verify_targeting(summary, snapshots, mobile):
    """Reject missing/contradictory telemetry; damage must be independently seen."""
    errors = []
    if not isinstance(summary, dict):
        summary = {}
    profile = verify_beta_ui_profile(summary, mobile, IMAGES)
    errors.extend(profile["errors"])
    lock_indicators = []
    for capture in summary.get("captures", []):
        if not isinstance(capture, dict):
            continue
        nodes = capture.get("nodes", [])
        markers = [node for node in nodes if isinstance(node, dict)
                   and node.get("name") == "LockedTargetIndicator"] if isinstance(nodes, list) else []
        if len(markers) != 1:
            errors.append(f"Stage {capture.get('stage')} requires exactly one locked-target marker readback.")
            continue
        node = markers[0]
        size = node.get("size")
        valid_size = isinstance(size, list) and len(size) == 2 and all(
            type(value) in (int, float) and math.isfinite(value) and value >= 0 for value in size)
        if type(node.get("visible")) is not bool or not valid_size or "selected_target" not in capture:
            errors.append(f"Stage {capture.get('stage')} has invalid locked-target visibility/size/selection evidence.")
            continue
        visible = node["visible"] and min(size) > 0
        selected = capture["selected_target"] is not None
        lock_indicators.append(dict(stage=capture.get("stage"), selected_target=capture["selected_target"],
                                    indicator_visible=visible, size=size))
        if visible != selected:
            errors.append(f"Stage {capture.get('stage')} locked-target indicator does not match actual selection.")
    if summary.get("scenario") != "targeting" or summary.get("pass") is not True:
        errors.append("Missing successful native targeting summary.")
    if summary.get("scripted_input") is not True or summary.get("synthetic_damage") is not False:
        errors.append("Input/damage provenance must explicitly identify real scripted combat.")
    if summary.get("manual_interaction_verified") is not False:
        errors.append("Scripted input must not claim a manual playtest.")
    if summary.get("setup_fixture") != "development server target placement and ambient AI disabled":
        errors.append("Missing explicit deterministic server fixture provenance.")
    event_list = summary.get("events", [])
    events = {event.get("event"): event for event in event_list if isinstance(event, dict)}
    required = {"ready", "basic_start", "basic_damage", "basic_stop", "cancellation_verified"}
    required |= ({"drag_preview", "drag_release", "touch_canceled", "empty_aim_released",
                  "independent_movement"} if mobile else
                 {"left_select", "selection_no_attack", "right_attack", "ground_cancel",
                  "alt_click_blocked", "ui_click_blocked"})
    if missing := sorted(required - events.keys()):
        errors.append(f"Missing production-input events: {missing}.")
    if len(events) != len(event_list):
        errors.append("Malformed or duplicate input events.")
    commands = summary.get("commands", [])
    if not isinstance(commands, list):
        commands = []
        errors.append("Missing command readback array.")
    if any(command.get("kind") == "cast" for command in commands):
        errors.append("Targeting scenario unexpectedly emitted a Q/W/E/R cast.")
    attacks = [command for command in commands if command.get("kind") == "basic_attack"]
    target_id, player_id = summary.get("target_id"), summary.get("player_id")
    start, stop = events.get("basic_start", {}), events.get("basic_stop", {})
    start_tick, stop_tick = start.get("snapshot_tick"), stop.get("snapshot_tick")
    required_attacks = 1 if mobile else 2
    if len(attacks) < required_attacks:
        errors.append(f"Expected at least {required_attacks} actual BasicAttack requests.")
    if any(command.get("target") != {"kind": "player", "id": target_id} for command in attacks):
        errors.append("An attack did not preserve the exact intended player target.")
    start_time, stop_time = start.get("elapsed_seconds"), stop.get("elapsed_seconds")
    if not (isinstance(start_time, (int, float)) and isinstance(stop_time, (int, float))
            and start_time < stop_time):
        errors.append("Missing ordered basic-attack interval.")
    elif any(not start_time <= command.get("elapsed_seconds", -1) <= stop_time for command in attacks):
        errors.append("Basic attack escaped the permitted input interval.")
    final = events.get("cancellation_verified", {})
    if not (isinstance(stop_tick, int) and isinstance(final.get("snapshot_tick"), int)
            and final["snapshot_tick"] - stop_tick >= 15):
        errors.append("Cancellation requires at least 15 later authoritative snapshot ticks.")
    by_tick = {}
    for snapshot in snapshots:
        tick = snapshot.get("snapshot_tick")
        if isinstance(tick, int):
            by_tick[tick] = snapshot
    quiet_intervals = {}
    for label, first, last in (("preview", events.get("ready", {}).get("snapshot_tick"), start_tick),
                               ("canceled", stop_tick, final.get("snapshot_tick"))):
        quiet_hp = [player["hp"] for tick, snapshot in sorted(by_tick.items())
                    if isinstance(first, int) and isinstance(last, int) and first <= tick <= last
                    for player in snapshot.get("players", []) if player.get("id") == target_id]
        quiet_intervals[label] = dict(snapshots=len(quiet_hp),
                                      hp_decreases=sum(b < a - .01 for a, b in zip(quiet_hp, quiet_hp[1:])))
        if len(quiet_hp) < 2 or quiet_intervals[label]["hp_decreases"]:
            errors.append(f"Observer did not confirm a damage-free {label} interval.")
    relevant = [snapshot for tick, snapshot in sorted(by_tick.items())
                if isinstance(start_tick, int) and isinstance(stop_tick, int)
                and start_tick <= tick <= stop_tick]
    victim = [(snapshot["snapshot_tick"], player["hp"]) for snapshot in relevant
              for player in snapshot.get("players", []) if player.get("id") == target_id]
    caster = [player for snapshot in relevant for player in snapshot.get("players", [])
              if player.get("id") == player_id]
    hp_drops = [(b[0], a[1] - b[1]) for a, b in zip(victim, victim[1:]) if b[1] < a[1] - .01]
    if len(hp_drops) < required_attacks:
        errors.append(f"Observer saw fewer than {required_attacks} separate target HP decreases.")
    other_targets = summary.get("other_target_ids", [])
    for other_id in other_targets:
        other_hp = [player["hp"] for snapshot in relevant for player in snapshot.get("players", [])
                    if player.get("id") == other_id]
        if not other_hp or max(other_hp) - min(other_hp) > .01:
            errors.append("Unselected passive target was damaged or absent from server readback.")
    if not other_targets:
        errors.append("Exact directional selection requires a second authoritative target.")
    cooldowns = summary.get("skill_cooldowns")
    if not isinstance(cooldowns, list) or len(cooldowns) != 4 or any(
            not isinstance(value, (int, float)) or abs(value) > .001 for value in cooldowns):
        errors.append("Client skill cooldown readback must contain four unchanged zero values.")
    basic_actions = {player.get("action_sequence") for player in caster
                     if player.get("action_kind") == "attack" and player.get("action_slot") == 255
                     and player.get("basic_attack_request_id", 0) > 0
                     and player.get("basic_attack_cooldown_secs", 0) > 0
                     and player.get("basic_attack_remaining_secs", 0) > 0}
    basic_actions.discard(None)
    if len(basic_actions) < required_attacks:
        errors.append("Observer did not confirm distinct accepted basic actions and independent cooldowns.")
    mana = [player.get("mana") for player in caster]
    if not mana or any(value is None for value in mana) or any(
            b < a - .01 for a, b in zip(mana, mana[1:])):
        errors.append("Basic-only interval consumed mana or lacks mana evidence.")
    result = dict(errors=errors, ui_profile=profile,
                locked_target_indicators=lock_indicators,
                basic_requests=len(attacks), authoritative_hp_drops=hp_drops,
                accepted_basic_action_sequences=sorted(basic_actions),
                quiet_intervals=quiet_intervals,
                skill_cooldown_evidence_source="native client LocalCastCooldown; server timestamp isolation is tested separately",
                observed_target_snapshots=len(victim), observed_caster_snapshots=len(caster),
                observed_snapshot_ticks=len(by_tick), synthetic_damage=False,
                manual_interaction_verified=False)
    result["pass"] = not errors
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--client-bin", type=Path, required=True)
    parser.add_argument("--server-bin", type=Path, required=True)
    parser.add_argument("--assets", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--width", type=int, default=1280)
    parser.add_argument("--height", type=int, default=720)
    parser.add_argument("--touch-controls", action="store_true")
    parser.add_argument("--timeout", type=int, default=100)
    args = parser.parse_args()
    client, server, assets, output = (path.resolve() for path in
                                     (args.client_bin, args.server_bin, args.assets, args.output))
    if not client.is_file() or not server.is_file() or not assets.is_dir():
        parser.error("Both native binaries and the asset directory must exist.")
    if not 320 <= args.width <= 3840 or not 320 <= args.height <= 2160:
        parser.error("Unsupported viewport dimensions.")
    if output.exists() and any(output.iterdir()):
        parser.error("Output must be empty; preserve previous evidence.")
    output.mkdir(parents=True, exist_ok=True)
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as reservation:
        reservation.bind(("127.0.0.1", 0))
        address = reservation.getsockname()
    timeout = max(45, min(args.timeout, 240))
    result = dict(scenario="targeting", platform=platform.platform(), machine=platform.machine(),
                  binary_sha256=dict(client=sha256(client), server=sha256(server)),
                  client=str(client), server=str(server), assets=str(assets), pixels=[args.width, args.height],
                  requested_profile="mobile" if args.touch_controls else "desktop", scripted_peers=2,
                  setup_fixture="development server target placement and ambient AI disabled",
                  synthetic_damage=False, manual_interaction_verified=False,
                  input_method="production mouse/TouchInput; independent hello-only server observer")
    children, peers, observer = [], [], None
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="omoba-targeting-qa-") as isolated:
        env = dict(os.environ, SERVER_ADDR=f"{address[0]}:{address[1]}",
                   GAME_SERVER_ADDR=f"{address[0]}:{address[1]}", OMOBA_MATCH_MODE="dev",
                   OMOBA_TEAM_SIZE="5", OMOBA_TARGETING_QA="1", OMOBA_QA_TEAM="green",
                   OMOBA_CLIENT_CONFIG_DIR=str(Path(isolated) / "config"), OMOBA_ASSET_DIR=str(assets),
                   OMOBA_PLAYER_VISUAL_MODE="models3d", OMOBA_DEBUG_UI="0",
                   OMOBA_QA_WIDTH=str(args.width), OMOBA_QA_HEIGHT=str(args.height),
                   OMOBA_TOUCH_CONTROLS="1" if args.touch_controls else "0",
                   OMOBA_VISUAL_QA_DIR=str(output), OMOBA_VISUAL_QA_SCENARIO="targeting",
                   OMOBA_VISUAL_QA_TIMEOUT=str(timeout - 10))
        for key in ("OMOBA_AUTOJOIN", "OMOBA_MEASURE_MODELS", "OMOBA_AVATAR_MANIFEST"):
            env.pop(key, None)
        try:
            with (output / "server.log").open("w") as log:
                server_process = subprocess.Popen([str(server)], cwd=isolated, env=env,
                                                  stdout=log, stderr=subprocess.STDOUT)
            children.append(server_process)
            deadline = time.monotonic() + 15
            while "is listening" not in (output / "server.log").read_text(errors="replace"):
                if server_process.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError("Native server failed to start.")
                time.sleep(.05)
            peers = [PassiveTarget(address, index) for index in range(2)]
            observer = SnapshotObserver(address, output / "authoritative-snapshots.jsonl")
            with (output / "client.log").open("w") as log:
                client_process = subprocess.Popen([str(client)], cwd=isolated, env=env,
                                                  stdout=log, stderr=subprocess.STDOUT)
            children.append(client_process)
            while client_process.poll() is None and time.monotonic() - started < timeout:
                if server_process.poll() is not None:
                    raise RuntimeError("Native server exited during scenario.")
                now = time.monotonic()
                for peer in peers:
                    peer.update(now)
                observer.update(now)
                time.sleep(.025)
            result["client_exit_code"] = client_process.poll()
            result["timed_out"] = client_process.poll() is None
        except Exception as error:
            result["error"] = str(error)
        finally:
            for peer in peers:
                peer.close()
            if observer:
                observer.close()
            for child in reversed(children):
                if child.poll() is None:
                    child.terminate()
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
    try:
        summary = json.loads((output / "qa-summary.json").read_text())
    except (OSError, ValueError):
        summary = {}
    result["targeting"] = verify_targeting(summary, observer.samples if observer else [], args.touch_controls)
    result["images"] = {name: dict(bytes=(output / name).stat().st_size, sha256=sha256(output / name))
                        for name in IMAGES if (output / name).is_file()}
    log = (output / "client.log").read_text(errors="replace") if (output / "client.log").exists() else ""
    result["errors"] = [line for line in log.splitlines() if any(marker in line for marker in
                         ("panicked at", "does not exist", "Path not found", "Downloading model", "TARGETING_QA failed"))]
    result["elapsed_seconds"] = time.monotonic() - started
    result["capture_pass"] = bool(result.get("client_exit_code") == 0 and not result.get("error")
                                  and len(result["images"]) == len(IMAGES) and result["targeting"]["pass"]
                                  and "First snapshot received" in log and str(assets) in log
                                  and not result["errors"])
    (output / "capture-run.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    raise SystemExit(0 if result["capture_pass"] else 1)


if __name__ == "__main__":
    main()
