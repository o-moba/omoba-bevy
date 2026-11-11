#!/usr/bin/env python3
"""One isolated RU fair-play framebuffer capture; no OS window or unlock."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import struct
import subprocess
import sys
import time

SCENES = ("taken", "wide-draft", "wide-lobby", "compatibility",
          "party-restart", "search-restart", "remake-menu", "remake-prompt",
          "surrender-prompt", "remade-result", "surrender-result")
OBSERVED_SCENES = {
    "wide-draft": ("Draft", ("PrematchStageText-",)),
    "wide-lobby": ("Lobby", ("LobbyMemberName-", "LobbyMemberStatus-",
                              "LobbyRosterName-", "LobbyRosterTag-")),
    "compatibility": ("Home", ("HomeCompatibilityDetail", "HomeOfflineNotice")),
}
SOURCE_FILES = ("client/src/lib.rs", "client/src/qa/mod.rs",
                "client/src/qa/offscreen.rs", "client/src/qa/fair_play_qa.rs",
                "client/src/qa/result_qa.rs", "client/src/qa/creator_loop_qa.rs",
                "client/src/net/session.rs", "client/src/pause_menu/match_votes.rs",
                "client/src/world.rs", "client/src/platform.rs",
                "client/src/platform/ui_profile.rs",
                "client/src/frontend/mod.rs", "client/src/frontend/home.rs",
                "client/src/frontend/widgets.rs",
                "client/src/frontend/lobby.rs", "client/src/frontend/draft.rs",
                "client/src/frontend/draft/stage.rs", "client/src/party.rs",
                "client/src/ui/widgets/surfaces.rs")


def sha(path):
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def png(path, viewport, relative_to):
    with path.open("rb") as file:
        header = file.read(24)
    if (path.stat().st_size <= 1000 or header[:8] != b"\x89PNG\r\n\x1a\n"
            or len(header) != 24 or struct.unpack(">II", header[16:24]) != viewport):
        raise ValueError("missing, incomplete or wrong-size capture: " + str(path))
    return {"file": str(path.relative_to(relative_to)), "sha256": sha(path),
            "bytes": path.stat().st_size}


def screen_locked():
    if sys.platform != "darwin":
        return None
    try:
        result = subprocess.run(
            ["/usr/bin/python3", "-c", 'import Quartz;print(int(bool((Quartz.CGSessionCopyCurrentDictionary() or {}).get("CGSSessionScreenIsLocked",False))))'],
            text=True, capture_output=True, timeout=3, check=True)
        return {"0": False, "1": True}.get(result.stdout.strip())
    except (OSError, subprocess.SubprocessError):
        return None  # Metadata only; rendering never needs Quartz or an unlock.


def stop(child):
    if child.poll() is None:
        try:
            os.killpg(child.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            child.wait(timeout=10)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.wait()


def interrupted(signum, _frame):
    raise KeyboardInterrupt("capture interrupted by signal " + str(signum))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path, help="matching desktop qa-enabled client")
    parser.add_argument("--scene", required=True, choices=SCENES)
    parser.add_argument("--output", required=True, type=Path, help="new directory under this repo's .agent/")
    parser.add_argument("--language", choices=("en", "ru", "zh-Hans"), default="en")
    parser.add_argument("--legal-version", required=True, help="built legal version, synthetic fixture only")
    parser.add_argument("--width", type=int, default=852)
    parser.add_argument("--height", type=int, default=393)
    parser.add_argument("--guide", action="store_true", help="capture and dismiss the real first-match guide before a vote prompt")
    parser.add_argument("--nickname", help="synthetic Home nickname for bounded-name checks; no real account change")
    parser.add_argument("--build-report", type=Path, help="optional verified build JSON with binary_sha256")
    parser.add_argument("--timeout", type=int, default=180)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[4]
    binary = args.binary.expanduser().resolve(strict=True)
    output = args.output.expanduser()
    output = (root / output).resolve() if not output.is_absolute() else output.resolve()
    if (not output.is_relative_to(root / ".agent") or output == root / ".agent"
            or not 200 <= args.width <= 3840 or not 200 <= args.height <= 3840
            or not 30 <= args.timeout <= 240):
        parser.error("use a fresh .agent/ subdirectory, viewport200..3840 and timeout30..240")
    if args.guide and args.scene not in ("remake-prompt", "surrender-prompt"):
        parser.error("--guide applies to one vote-prompt scene")
    if args.nickname is not None:
        name, separator, tag = args.nickname.partition("#")
        if (args.scene != "party-restart" or not 1 <= len(name) <= 20
                or name != name.strip() or len(name.encode("utf-8")) > 80
                or not all(c.isalnum() or c in " _-." for c in name)
                or (separator and (len(tag) != 4 or not tag.isascii() or not tag.isdigit()))):
            parser.error("--nickname requires party-restart and a valid1..20-character name with optional #1234 tag")
    assets = root / "client/assets"
    if not (assets / "avatars/manifest.json").is_file():
        parser.error("this checkout needs its normal client assets; the helper downloads nothing")
    identity = sha(binary)
    build = None
    if args.build_report:
        build = json.loads(args.build_report.read_text())
        if build.get("binary_sha256") != identity:
            parser.error("selected binary does not match the supplied build report")
    output.mkdir(parents=True, exist_ok=False)
    state = output / "state"
    state.mkdir()
    (state / "account_region.json").write_text(json.dumps({"schema": 1, "selected": "ru", "pending": None, "account": None}))
    (state / "legal_acceptance.json").write_text(json.dumps({"region": "ru", "doc_version": args.legal_version, "accepted_at": int(time.time())}))
    (state / "client_preferences.json").write_text(json.dumps({"schema_version": 8, "audio": {"muted": True}, "beginner_tips": {"enabled": args.guide}}))
    if args.nickname is not None:
        (state / "career_nickname.json").write_text(json.dumps(args.nickname))
    env = {k: v for k, v in os.environ.items() if not k.startswith(
        ("OMOBA_", "EKZA_", "SOLANA_", "YOOKASSA_", "APPLE_", "VERCEL_"))
        and k not in ("SERVER_ADDR", "GAME_SERVER_ADDR")}
    env.update(RUST_BACKTRACE="1", BEVY_BACKTRACE="full", OMOBA_QA_OFFSCREEN="1",
               GAME_SERVER_ADDR="127.0.0.1:9", OMOBA_ACCOUNT_API_URL="http://127.0.0.1:9",
               OMOBA_PORTAL_ORIGIN="http://127.0.0.1:9", OMOBA_ALLOW_INSECURE_LOCAL="1",
               OMOBA_ASSET_DIR=str(assets), OMOBA_CLIENT_CONFIG_DIR=str(state),
               OMOBA_LANGUAGE=args.language, OMOBA_QA_WIDTH=str(args.width),
               OMOBA_QA_HEIGHT=str(args.height), OMOBA_TOUCH_CONTROLS="1",
               OMOBA_FAIR_PLAY_QA_OUTPUT=str(output), OMOBA_FAIR_PLAY_QA_SCENE=args.scene,
               NO_PROXY="127.0.0.1,localhost,::1", no_proxy="127.0.0.1,localhost,::1")
    loop = output / "loop"
    if args.guide or args.scene in OBSERVED_SCENES:
        loop.mkdir()
        env["OMOBA_CREATOR_LOOP_QA"] = str(loop)
    git = lambda *command: subprocess.check_output(["git", *command], cwd=root)
    report = {"status": "running", "scene": args.scene, "language": args.language,
              "nickname_fixture": args.nickname,
              "region": "ru", "viewport": [args.width, args.height],
              "binary_sha256": identity, "head_at_capture": git("rev-parse", "HEAD").decode().strip(),
              "tracked_diff_sha256": hashlib.sha256(git("diff", "HEAD")).hexdigest(),
              "source_hashes": {name: sha(root / name) for name in SOURCE_FILES},
              "runner_sha256": sha(Path(__file__)), "screen_locked_at_launch": screen_locked(),
              "scope": "production offscreen renderer and real UI-kit actions; synthetic RU session/views/settings; no live match/account, desktop/window, physical phone or keep-awake/haptic proof",
              "build_source_parent": build.get("source_parent") if build else None,
              "source_binary_link": "caller-provided build; paths alone do not prove matching sources"}
    started = time.monotonic()
    signal.signal(signal.SIGTERM, interrupted)
    guide_sent = False
    child = None
    failure = None
    try:
        with (output / "client.log").open("w") as log:
            child = subprocess.Popen([str(binary)], cwd=state, env=env,
                                     stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            while child.poll() is None:
                content = (output / "client.log").read_text(errors="replace")
                if "panicked at " in content or "Encountered a panic" in content:
                    raise RuntimeError("native panic; inspect retained client.log")
                if time.monotonic() - started > args.timeout:
                    raise TimeoutError("native capture deadline")
                current = loop / "qa-state.json"
                if args.guide and not guide_sent and current.is_file():
                    snapshot = json.loads(current.read_text())
                    if ("HelpDismissButton" in snapshot.get("buttons", [])
                            and snapshot.get("input", {}).get("modal_open") is True):
                        (loop / "commands.jsonl").write_text(
                            json.dumps({"op": "shot", "file": "guide-before-dismiss.png"}) + "\n"
                            + json.dumps({"op": "press", "id": "HelpDismissButton"}) + "\n")
                        guide_sent = True
                time.sleep(.2)
        if child.returncode != 0:
            raise RuntimeError("native client failed: " + str(child.returncode))
        summary = json.loads((output / "qa-summary.json").read_text())
        report["summary"] = summary
        if (summary.get("status") != "passed" or summary.get("color_pixels") is not True
                or summary.get("render_surface") != "offscreen image with virtual window metrics"
                or summary.get("locale") != args.language):
            raise RuntimeError("native state/readback/offscreen/locale gate did not pass")
        if args.scene in OBSERVED_SCENES:
            expected_screen, prefixes = OBSERVED_SCENES[args.scene]
            observed = json.loads((loop / "qa-state.json").read_text())
            labels = observed.get("labels", {})
            geometry = observed.get("label_geometry", {})
            report["observed"] = {"screen": observed.get("screen"),
                                  "labels": labels, "label_geometry": geometry}
            if (summary.get("ui_profile") != "Mobile"
                    or summary.get("debug_assertions") is not True):
                raise RuntimeError("UI-2/UI-3 scenes require an exact-source debug Mobile preview")
            if observed.get("screen") != expected_screen:
                raise RuntimeError("retained Creator Loop state does not match the captured screen")
            for prefix in prefixes:
                matches = [name for name in labels if name.startswith(prefix)]
                required = 5 if prefix.startswith(
                    ("PrematchStageText-", "LobbyMember", "LobbyRoster")) else 1
                if len(matches) < required or any(name not in geometry for name in matches):
                    raise RuntimeError("missing observed labels or geometry for " + prefix)
                for name in matches:
                    if labels[name].strip() == "…":
                        raise RuntimeError("label collapsed to an ellipsis: " + name)
                    shaped = geometry[name].get("layout_size")
                    available = geometry[name].get("node_size")
                    if (not shaped or not available
                            or any(shaped[axis] > available[axis] + 1.0 for axis in (0, 1))):
                        raise RuntimeError("shaped label exceeds its node: " + name)
            if args.scene == "wide-lobby":
                tags = {name: text for name, text in labels.items()
                        if name.startswith("LobbyRosterTag-")}
                expected = {f"#{tag:04}" for tag in range(1000, 1005)}
                if len(tags) != 5 or set(tags.values()) != expected:
                    raise RuntimeError("wide lobby must retain five distinct public handle tags")
        report["screenshot"] = png(output / "fair-play.png", (args.width, args.height), output)
        if args.guide:
            if not guide_sent:
                raise RuntimeError("guide was not actually observed and dismissed")
            report["guide_screenshot"] = png(loop / "guide-before-dismiss.png", (args.width, args.height), output)
        report["status"] = "captured_pending_visual_inspection"
    except (Exception, KeyboardInterrupt) as error:
        failure = str(error) or type(error).__name__
        report.update(status="failed", error=failure)
    finally:
        if child is not None:
            stop(child)
        report.update(seconds=round(time.monotonic() - started, 2), guide_actions_submitted=guide_sent)
        (output / "capture.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"status": report["status"], "output": str(output), "error": failure}))
    return 1 if failure else 0


if __name__ == "__main__":
    sys.exit(main())
