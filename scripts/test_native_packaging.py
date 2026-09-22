#!/usr/bin/env python3
"""Regression checks for source identity and Cargo executable selection."""
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

import package_native


class PackagingIdentityTests(unittest.TestCase):
    def test_source_change_during_build_rejects_package_before_output_creation(self):
        with tempfile.TemporaryDirectory(prefix="omoba-build-identity-") as directory:
            output = Path(directory) / "package"
            with mock.patch("sys.argv", ["package_native.py", "--output", str(output)]), \
                    mock.patch.object(package_native, "collect_legal_notices", return_value={}), \
                    mock.patch.object(package_native, "validate", return_value={"status": "PASS", "errors": []}), \
                    mock.patch.object(package_native, "source_identity", side_effect=[
                        {"source_identity_sha256": "before-build"},
                        {"source_identity_sha256": "changed-during-build"},
                    ]) as identity, \
                    mock.patch.object(package_native, "build_executables", return_value={}) as build:
                with self.assertRaisesRegex(RuntimeError, "source changed during build"):
                    package_native.main()
                build.assert_called_once_with("dev")
                self.assertEqual(identity.call_count, 2)
            self.assertFalse(output.exists())

    def test_dirty_identity_covers_untracked_sources_but_ignores_runtime_artifacts(self):
        with tempfile.TemporaryDirectory(prefix="omoba-source-proof-") as directory:
            root = Path(directory)
            def git(*args):
                return subprocess.check_output(["git", *args], cwd=root, stderr=subprocess.DEVNULL)
            git("init")
            (root / ".gitignore").write_text("runtime/\n")
            (root / "tracked.rs").write_text("initial source\n")
            git("add", ".")
            git("-c", "user.name=Packaging test", "-c", "user.email=test@example.invalid",
                "commit", "-m", "test: initialize fixture")
            with mock.patch.object(package_native, "ROOT", root):
                clean = package_native.source_identity()
                self.assertFalse(clean["source_dirty"])
                (root / "new.rs").write_text("first implementation\n")
                first = package_native.source_identity()
                (root / "new.rs").write_text("second implementation\n")
                second = package_native.source_identity()
                self.assertEqual(first["source_diff_sha256"], second["source_diff_sha256"])
                self.assertNotEqual(first["source_identity_sha256"], second["source_identity_sha256"])
                self.assertNotEqual(first["untracked_source_sha256"], second["untracked_source_sha256"])
                (root / "runtime").mkdir()
                (root / "runtime/log.txt").write_text("not source\n")
                self.assertEqual(second, package_native.source_identity())

    def test_build_uses_cargo_emitted_paths_and_rejects_incomplete_or_failed_builds(self):
        executable_paths = {name: Path("/configured-cargo-target/custom-profile") / name
                            for name in ("client", "server", "bots", "migrate-career", "omoba-account-api")}
        events = [dict(reason="compiler-artifact", target=dict(name=name), executable=str(path))
                  for name, path in executable_paths.items()]
        def process_for(messages, returncode=0):
            process = mock.Mock()
            process.stdout = io.StringIO("".join(json.dumps(event) + "\n" for event in messages))
            process.wait.return_value = returncode
            return process
        with mock.patch.object(package_native.subprocess, "Popen", return_value=process_for(events)) as build:
            self.assertEqual(package_native.build_executables("dev"), executable_paths)
            self.assertIn("dev", build.call_args.args[0])
            self.assertNotIn("--target", build.call_args.args[0])
        with mock.patch.object(package_native.subprocess, "Popen", return_value=process_for(events[:1])):
            with self.assertRaisesRegex(RuntimeError, "required executables"):
                package_native.build_executables("dev")
        with mock.patch.object(package_native.subprocess, "Popen", return_value=process_for(events, 1)):
            with self.assertRaisesRegex(RuntimeError, "build failed"):
                package_native.build_executables("dev")
        for required in ("migrate-career", "omoba-account-api"):
            with self.subTest(required=required), mock.patch.object(
                package_native.subprocess, "Popen", return_value=process_for(
                    [event for event in events if event["target"]["name"] != required]
                )
            ):
                with self.assertRaisesRegex(RuntimeError, required):
                    package_native.build_executables("dev")


@unittest.skipIf(os.name == "nt", "POSIX shell launcher checks")
class LobbyLauncherTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="omoba-lobby-package-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.package = self.root / "package with spaces"
        self.package.mkdir()
        package_native.write_lobby_launcher(self.package)
        self.launcher = self.package / "launch-lobby.sh"
        self.server = self.package / "server"
        self.server.write_text('''#!/usr/bin/env python3
import json, os, sys
print(json.dumps({"cwd": os.getcwd(), "args": sys.argv[1:], "env": {
    key: value for key, value in os.environ.items()
    if key.startswith("OMOBA_") and key != "OMOBA_DATABASE_URL" or key == "SERVER_ADDR"
}}))
''')
        self.server.chmod(0o755)
        self.environment = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "OMOBA_DATABASE_URL": "postgresql://packaging-fixture.invalid/isolated_test",
            "OMOBA_MATCH_ROOT": str(self.root / "persistent state"),
            "SERVER_ADDR": "127.0.0.1:40400",
            "OMOBA_MATCH_PUBLIC_HOST": "game.example.invalid",
            "OMOBA_MATCH_BIND_IP": "127.0.0.1",
            "OMOBA_MATCH_FIRST_PORT": "41400",
        }

    def run_launcher(self, environment=None, *arguments):
        return subprocess.run(
            [str(self.launcher), *arguments], env=environment or self.environment,
            cwd=self.root, capture_output=True, text=True, check=False,
        )

    def test_launcher_requires_explicit_service_configuration_before_creating_state(self):
        for key in self.environment.keys() - {"PATH"}:
            with self.subTest(missing=key):
                environment = dict(self.environment)
                del environment[key]
                result = self.run_launcher(environment)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(key, result.stderr)
                self.assertFalse(result.stdout)
                self.assertNotIn("packaging-fixture.invalid", result.stderr)
                self.assertFalse(Path(self.environment["OMOBA_MATCH_ROOT"]).exists())

    def test_launcher_runs_packaged_server_with_external_persistent_root_and_pinned_role(self):
        environment = dict(self.environment,
            OMOBA_MATCH_CAPACITY="3", OMOBA_SERVER_ROLE="match",
            OMOBA_MATCH_EXECUTABLE="/do/not/run/this", OMOBA_MATCH_MODE="dev",
            OMOBA_TEAM_SIZE="1", OMOBA_CAREER_OUTBOX=str(self.package / "wrong-outbox"),
            OMOBA_MATCH_ALLOCATION="stale.json", OMOBA_MATCH_RECOVERY="1",
            OMOBA_MAP_CONFIG="custom.json", OMOBA_TARGETING_QA="1")
        result = self.run_launcher(environment, "argument with spaces")
        self.assertEqual(result.returncode, 0, result.stderr)
        record = json.loads(result.stdout)
        persistent_root = Path(self.environment["OMOBA_MATCH_ROOT"]).resolve()
        self.assertEqual(Path(record["cwd"]), persistent_root)
        self.assertEqual(record["args"], ["argument with spaces"])
        self.assertEqual(record["env"]["OMOBA_SERVER_ROLE"], "lobby")
        self.assertEqual(record["env"]["OMOBA_MATCH_EXECUTABLE"], str(self.server.resolve()))
        self.assertEqual(record["env"]["OMOBA_CAREER_OUTBOX"], str(persistent_root / "lobby-outbox"))
        self.assertEqual(record["env"]["OMOBA_MATCH_MODE"], "release")
        self.assertEqual(record["env"]["OMOBA_TEAM_SIZE"], "5")
        for key in ("SERVER_ADDR", "OMOBA_MATCH_PUBLIC_HOST", "OMOBA_MATCH_BIND_IP", "OMOBA_MATCH_FIRST_PORT", "OMOBA_MATCH_CAPACITY"):
            self.assertEqual(record["env"][key], environment[key])
        for key in ("OMOBA_MATCH_ALLOCATION", "OMOBA_MATCH_RECOVERY", "OMOBA_MAP_CONFIG", "OMOBA_TARGETING_QA"):
            self.assertNotIn(key, record["env"])
        self.assertFalse((self.package / "wrong-outbox").exists())

    def test_launcher_rejects_relative_package_local_and_symlinked_state_roots(self):
        inside = self.package / "inside"
        inside.mkdir()
        alias = self.root / "alias"
        alias.symlink_to(inside, target_is_directory=True)
        for root in ("relative/state", str(self.package), str(inside), str(alias)):
            with self.subTest(root=root):
                result = self.run_launcher(dict(self.environment, OMOBA_MATCH_ROOT=root))
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("OMOBA_MATCH_ROOT", result.stderr)
                self.assertFalse(result.stdout)

    def test_launcher_rejects_invalid_or_overflowing_worker_ports_before_execution(self):
        for port, capacity in (("x", "1"), ("0", "1"), ("65535", "2"), ("41000", "101"), ("41000", "-1")):
            with self.subTest(port=port, capacity=capacity):
                result = self.run_launcher(dict(self.environment, OMOBA_MATCH_FIRST_PORT=port, OMOBA_MATCH_CAPACITY=capacity))
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(result.stdout)

    def test_generated_launcher_passes_shell_syntax(self):
        subprocess.run(["sh", "-n", str(self.launcher)], check=True)


if __name__ == "__main__":
    unittest.main()
