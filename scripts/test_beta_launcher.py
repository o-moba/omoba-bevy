#!/usr/bin/env python3
"""Exercise packaged launcher validation, environment and real child cleanup."""
import contextlib
import io
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time
import unittest
from unittest import mock

from beta_launcher import arguments, run


STUB = '''#!/usr/bin/env python3
import json, os, pathlib, sys, time
name=pathlib.Path(sys.argv[0]).name
pathlib.Path(name+'.invocation.json').write_text(json.dumps(dict(pid=os.getpid(), args=sys.argv[1:], env=dict(os.environ))))
if name == 'server': print('Server is listening', flush=True)
if name == 'client':
    time.sleep(0.5)
else:
    while True: time.sleep(0.1)
'''


class LauncherTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="omoba-beta-launch-test-")
        self.package = Path(self.temp.name).resolve()
        for binary in ("server", "client", "bots"):
            path = self.package / binary
            path.write_text(STUB)
            path.chmod(0o755)

    def tearDown(self):
        self.temp.cleanup()

    def free_address(self):
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.bind(("127.0.0.1", 0))
            return "127.0.0.1:" + str(sock.getsockname()[1])

    def invocation(self, kind):
        return json.loads((self.package / f"{kind}.invocation.json").read_text())

    def assert_stopped(self, kind):
        pid = self.invocation(kind)["pid"]
        with self.assertRaises(ProcessLookupError):
            os.kill(pid, 0)

    def test_invalid_parameters_fail(self):
        for argv in (("host", "--humans", "0"), ("host", "--humans", "11"),
                     ("join", "127.0.0.1:0"), ("join", "localhost:65536"),
                     ("join", "missing-port"), ("join", "127.0.0.1:4000", "--profile", "../outside")):
            with self.subTest(argv=argv), contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
                arguments(argv)
            self.assertEqual(error.exception.code, 2)

    def test_practice_uses_server_bots_without_harness_and_cleans_up_children(self):
        (self.package / "bots").unlink()
        self.assertEqual(run(arguments(["practice", "--bind", self.free_address()]), self.package), 0)
        for kind in ("client", "server"):
            invocation = self.invocation(kind)
            self.assertEqual(invocation["env"]["OMOBA_MATCH_MODE"], "practice")
            self.assertEqual(invocation["env"]["OMOBA_TEAM_SIZE"], "5")
            self.assert_stopped(kind)
        self.assertFalse((self.package / "bots.invocation.json").exists())
        first_profile = self.invocation("client")["env"]["OMOBA_CLIENT_CONFIG_DIR"]
        self.assertEqual(run(arguments(["practice", "--bind", self.free_address()]), self.package), 0)
        self.assertNotEqual(first_profile, self.invocation("client")["env"]["OMOBA_CLIENT_CONFIG_DIR"])

    def test_legacy_local_release_still_launches_nine_harness_players(self):
        self.assertEqual(run(arguments(["local-release", "--bind", self.free_address()]), self.package), 0)
        for kind in ("client", "server", "bots"):
            self.assertEqual(self.invocation(kind)["env"]["OMOBA_MATCH_MODE"], "release")
            self.assert_stopped(kind)
        self.assertEqual(self.invocation("bots")["args"][:2], ["--count", "9"])

    def test_practice_isolates_qa_environment_and_only_stops_owned_children(self):
        unrelated = subprocess.Popen(["python3", "-c", "import time; time.sleep(30)"])
        try:
            with mock.patch.dict(os.environ, {"OMOBA_MATCH_MODE": "dev", "OMOBA_SOCIAL_QA_DIR": "/tmp/unused-qa", "OMOBA_SOCIAL_QA_OUTPUT": "/tmp/unused-social-qa", "OMOBA_CAREER_QA_OUTPUT": "/tmp/unused-career-qa", "OMOBA_TARGETING_QA": "1", "OMOBA_AUTOJOIN": "mage:-:green"}):
                self.assertEqual(run(arguments(["practice", "--bind", self.free_address()]), self.package), 0)
            self.assertIsNone(unrelated.poll())
            env = self.invocation("client")["env"]
            self.assertEqual(env["OMOBA_MATCH_MODE"], "practice")
            for key in ("OMOBA_SOCIAL_QA_DIR", "OMOBA_SOCIAL_QA_OUTPUT", "OMOBA_CAREER_QA_OUTPUT", "OMOBA_TARGETING_QA", "OMOBA_AUTOJOIN"):
                self.assertNotIn(key, env)
        finally:
            unrelated.terminate()
            unrelated.wait(timeout=5)

    def test_source_play_defaults_to_legacy_and_explicit_practice_selects_native_bots(self):
        import play_local
        for argv, expected in (([], "local-release"), (["--mode", "practice"], "practice")):
            with mock.patch.object(play_local, "build_executables", return_value={}), mock.patch.object(play_local.beta_launcher, "run", return_value=0) as launch, mock.patch.object(play_local.signal, "signal"):
                self.assertEqual(play_local.main(argv), 0)
                self.assertEqual(launch.call_args.args[0].action, expected)

    def test_source_binaries_and_assets_work_outside_session_directory(self):
        binaries_dir = self.package / "configured-target"
        binaries_dir.mkdir()
        executables = {}
        for kind in ("client", "server", "bots"):
            destination = binaries_dir / kind
            (self.package / kind).rename(destination)
            executables[kind] = destination
        assets = self.package / "checkout-assets"
        assets.mkdir()
        self.assertEqual(run(arguments(["local-release", "--bind", self.free_address()]),
                             self.package, executables=executables, assets=assets), 0)
        for kind in ("client", "server", "bots"):
            self.assertEqual(self.invocation(kind)["env"]["OMOBA_ASSET_DIR"], str(assets))
            self.assert_stopped(kind)

    def test_join_resolves_and_uses_named_persistent_profile(self):
        args = arguments(["join", "localhost:4000", "--profile", "tester-2"])
        self.assertEqual(run(args, self.package), 0)
        env = self.invocation("client")["env"]
        self.assertEqual(env["GAME_SERVER_ADDR"], "127.0.0.1:4000")
        self.assertEqual(env["OMOBA_CLIENT_CONFIG_DIR"], str(self.package / "user-data/tester-2"))

    def test_unspecified_join_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, "real address"):
            run(arguments(["join", "0.0.0.0:4000"]), self.package)

    def test_busy_host_port_fails_before_starting_children(self):
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.bind(("127.0.0.1", 0))
            with self.assertRaises(OSError):
                run(arguments(["practice", "--bind", f"127.0.0.1:{sock.getsockname()[1]}"]), self.package)
        self.assertFalse((self.package / "server.invocation.json").exists())

    def test_host_signal_stops_owned_server_and_eight_bots(self):
        shutil.copy2(Path(__file__).with_name("beta_launcher.py"), self.package / "beta.py")
        with (self.package / "launcher.log").open("w") as log:
            process = subprocess.Popen(["python3", str(self.package / "beta.py"), "host", "--humans", "2", "--bind", self.free_address()], stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + 10
            while not (self.package / "bots.invocation.json").exists():
                if process.poll() is not None or time.monotonic() >= deadline:
                    self.fail("host failed to start")
                time.sleep(0.05)
            self.assertEqual(self.invocation("bots")["args"][:2], ["--count", "8"])
            process.terminate()
            self.assertEqual(process.wait(timeout=12), 130)
            self.assert_stopped("server")
            self.assert_stopped("bots")
            self.assertFalse((self.package / "client.invocation.json").exists())
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=12)

    def test_native_practice_server_needs_no_client_or_harness_and_cleans_up_on_signal(self):
        (self.package / "client").unlink()
        (self.package / "bots").unlink()
        shutil.copy2(Path(__file__).with_name("beta_launcher.py"), self.package / "beta.py")
        with (self.package / "launcher.log").open("w") as log:
            process = subprocess.Popen(["python3", str(self.package / "beta.py"), "practice-server", "--bind", self.free_address()], stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + 10
            while not (self.package / "server.invocation.json").exists():
                if process.poll() is not None or time.monotonic() >= deadline:
                    self.fail("native practice host failed to start")
                time.sleep(0.05)
            self.assertEqual(self.invocation("server")["env"]["OMOBA_MATCH_MODE"], "practice")
            process.terminate()
            self.assertEqual(process.wait(timeout=12), 130)
            self.assert_stopped("server")
            self.assertFalse((self.package / "bots.invocation.json").exists())
            self.assertFalse((self.package / "client.invocation.json").exists())
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=12)


if __name__ == "__main__":
    unittest.main()
