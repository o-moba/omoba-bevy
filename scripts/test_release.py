import tempfile
import unittest
import zipfile
from pathlib import Path

import release


class ReleaseHelpers(unittest.TestCase):
    def test_workspace_version_reads_the_canonical_field(self):
        with tempfile.TemporaryDirectory() as root:
            Path(root, "Cargo.toml").write_text(
                '[workspace]\nmembers = []\n\n[workspace.package]\nedition = "2024"\nversion = "1.2.3-rc.4"\n')
            self.assertEqual(release.workspace_version(Path(root)), "1.2.3-rc.4")

    def test_android_version_codes_increase_and_finals_follow_their_prereleases(self):
        codes = [release.android_version_code(v) for v in
                 ("0.23.0-rc.6", "0.24.0-rc.1", "0.24.0", "0.24.1", "0.25.0", "1.0.0")]
        self.assertEqual(codes, sorted(codes))
        self.assertEqual(len(set(codes)), len(codes))
        self.assertLess(release.android_version_code("0.24.0-rc.1"), release.android_version_code("0.24.0"))
        self.assertLess(release.android_version_code("99.999.999"), 2_100_000_000)

    def test_release_notes_pick_the_version_section_or_unreleased(self):
        changelog = "# Changelog\n\n## [Unreleased]\n\n- next\n\n## [0.24.0] - 2026-09-25\n\n### Party\n- together\n\n## [0.23.0] - x\n\n- old\n"
        self.assertIn("together", release.release_notes("0.24.0", changelog))
        self.assertNotIn("old", release.release_notes("0.24.0", changelog))
        self.assertIn("next", release.release_notes("9.9.9", changelog))

    def test_artifact_names_are_versioned_per_platform(self):
        names = {p: release.artifact_name("0.24.0", p) for p in release.PLATFORMS}
        self.assertEqual(len(set(names.values())), len(names))
        self.assertTrue(all("0.24.0" in n for n in names.values()))

    def test_server_argument_must_be_host_port(self):
        self.assertEqual(release.validate_server("play.example.com:4000"), "play.example.com:4000")
        self.assertIsNone(release.validate_server(None))
        for bad in ("play.example.com", "host:0", "host:70000", "http://x:1", "a b:1"):
            with self.assertRaises(SystemExit):
                release.validate_server(bad)

    def test_player_readme_explains_joining_and_hosting(self):
        text = release.player_readme("0.24.0", "windows", release.resolve_flavor(server="10.0.0.2:4000"))
        self.assertIn("10.0.0.2:4000", text)
        self.assertIn("Host Practice Server.bat", text)
        self.assertIn("PLAY VS BOTS", text)

    def test_zip_keeps_executable_bits_and_a_root_folder(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary, "stage")
            source.mkdir()
            script = source / "run.sh"
            script.write_text("#!/bin/sh\n")
            script.chmod(0o755)
            archive = Path(temporary, "out.zip")
            release.zip_tree(source, archive, "Omoba")
            with zipfile.ZipFile(archive) as bundle:
                info = bundle.getinfo("Omoba/run.sh")
                self.assertTrue((info.external_attr >> 16) & 0o111)

    def test_checksums_cover_every_artifact(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory / "a.zip").write_bytes(b"a")
            (directory / "b.apk").write_bytes(b"b")
            lines = release.write_checksums(directory).read_text().splitlines()
            self.assertEqual(sorted(line.split()[1] for line in lines), ["a.zip", "b.apk"])


    def test_windows_stage_rejects_non_x64_executables(self):
        with tempfile.TemporaryDirectory() as temporary:
            fake = Path(temporary, "client.exe")
            fake.write_bytes(b"MZ" + b"\x00" * 200)
            with self.assertRaises(SystemExit):
                release.stage_windows(Path(temporary, "stage"), "0.24.0", release.PROD, fake, fake)

    def test_pinata_token_stays_out_of_the_command_line(self):
        from unittest import mock
        calls = []

        def fake_run(args, **kwargs):
            calls.append((args, kwargs))
            return mock.Mock(returncode=0, stdout='{"IpfsHash": "bafytest"}', stderr="")

        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(release.subprocess, "run", fake_run):
            package = Path(temporary, "Omoba-0.24.0-windows-x64.zip")
            package.write_bytes(b"zip")
            self.assertEqual(release.pinata_pin(package, "secret-token"), "bafytest")
        args, kwargs = calls[0]
        self.assertNotIn("secret-token", " ".join(map(str, args)))
        self.assertIn("secret-token", kwargs["input"])

    def test_publish_skips_packages_already_pinned_with_the_same_hash(self):
        from unittest import mock
        with tempfile.TemporaryDirectory() as temporary:
            out = Path(temporary)
            version = "0.24.0"
            package = out / release.artifact_name(version, "linux")
            package.write_bytes(b"linux")
            (out / "ipfs.json").write_text(
                '{"linux": {"sha256": "%s", "url": "https://x/ipfs/c/f"}}' % release.sha256(package))
            with mock.patch.dict(release.os.environ, {"PINATA_JWT": "t"}), \
                    mock.patch.object(release, "workspace_version", return_value=version), \
                    mock.patch.object(release, "dist_dir", return_value=out), \
                    mock.patch.object(release, "pinata_pin", side_effect=AssertionError("re-pinned")):
                self.assertEqual(release.command_publish(
                    mock.Mock(out=None, gateway="https://g", contour="prod", dry_run=False)), 0)


    def test_android_package_name_follows_the_universal_switch(self):
        from unittest import mock
        with mock.patch.dict(release.os.environ, {"OMOBA_ANDROID_UNIVERSAL": "1"}):
            self.assertEqual(release.artifact_name("0.24.0", "android"), "Omoba-0.24.0-android-universal.apk")
        with mock.patch.dict(release.os.environ, {}, clear=True):
            self.assertEqual(release.artifact_name("0.24.0", "android"), "Omoba-0.24.0-android-arm64.apk")


class DevContour(unittest.TestCase):
    """`--contour dev` tester builds never collide with, or point at, production."""

    def setUp(self):
        from unittest import mock
        patcher = mock.patch.dict(release.os.environ)
        patcher.start()
        self.addCleanup(patcher.stop)
        release.os.environ.pop(release.DEV_IP_ENV, None)

    def test_production_defaults_are_unchanged(self):
        flavor = release.resolve_flavor()
        self.assertEqual(flavor, release.PROD)
        self.assertEqual(flavor.client_defaults(), {})
        self.assertEqual(flavor.config_dir, "omoba-bevy")
        self.assertEqual(flavor.app_name, "Omoba")
        self.assertEqual(flavor.bundle_id, release.MAC_BUNDLE_ID)
        self.assertEqual(release.artifact_name("0.24.0", "macos"), "Omoba-0.24.0-macos-arm64.zip")
        self.assertEqual(release.artifact_name("0.24.0", "linux"), "omoba-0.24.0-linux-x64.tar.gz")
        self.assertEqual(release.dist_path("0.24.0", Path("/d")), Path("/d/v0.24.0"))

    def test_dev_defaults_bake_the_dev_lobby_registry_and_settings_folder(self):
        flavor = release.resolve_flavor("dev")
        self.assertEqual(flavor.client_defaults(), {
            "OMOBA_DEFAULT_GAME_SERVER_ADDR": "lobby.dev.omoba.io:4100",
            "OMOBA_DEFAULT_REGISTRY_URL": "https://registry.dev.ekza.io",
            "OMOBA_DEFAULT_CONFIG_DIR_SUFFIX": "dev",
        })
        self.assertEqual(flavor.config_dir, "omoba-bevy-dev")
        self.assertEqual(flavor.app_name, "Omoba Dev")
        self.assertEqual(flavor.bundle_id, "space.ekza.omoba.desktop.dev")

    def test_dev_package_names_and_dist_directory_never_overlap_production(self):
        prod = {release.artifact_name("0.24.0", p) for p in release.PLATFORMS}
        dev = {release.artifact_name("0.24.0", p, "dev") for p in release.PLATFORMS}
        self.assertFalse(prod & dev)
        self.assertTrue(all("0.24.0-dev-" in name for name in dev))
        self.assertEqual(release.dist_path("0.24.0", Path("/d"), "dev"), Path("/d/v0.24.0-dev"))
        for bad in ("staging", "", "PROD"):
            with self.assertRaises(SystemExit):
                release.artifact_name("0.24.0", "macos", bad)

    def test_a_dev_build_refuses_production_endpoints(self):
        # The dev lobby is on its own VPS: no port on the production VPS is a dev endpoint.
        for server in ("77.246.105.57:4000", "77.246.105.57:04000", "77.246.105.57:4100",
                       "77.246.105.57:41000", "77.246.105.57:41100", "omoba.io:4000", "lobby.omoba.io:4100",
                       "LOBBY.OMOBA.IO.:4100", "play.ekza.io:4000"):
            with self.assertRaises(SystemExit, msg=server):
                release.resolve_flavor("dev", server=server)
        for registry in ("https://registry.ekza.io", "https://REGISTRY.ekza.io/", "https://studio.ekza.io",
                         "https://registry.ekza.io./v1", "https://wss.ekza.io/mirror", "https://ekza.io"):
            with self.assertRaises(SystemExit, msg=registry):
                release.resolve_flavor("dev", registry=registry)

    def test_a_production_build_refuses_dev_endpoints(self):
        for server in ("lobby.dev.omoba.io:4100", "Lobby.Dev.Omoba.IO.:4100", "dev.omoba.io:4000",
                       "x.dev.ekza.io:4000"):
            with self.assertRaises(SystemExit, msg=server):
                release.resolve_flavor(server=server)
        for registry in ("https://registry.dev.ekza.io", "https://Registry.Dev.Ekza.io/", "https://studio.dev.ekza.io"):
            with self.assertRaises(SystemExit, msg=registry):
                release.resolve_flavor(registry=registry)
        with self.assertRaises(SystemExit):
            release.resolve_flavor("staging")

    def test_a_production_build_refuses_the_dev_vps_address_once_it_is_known(self):
        self.assertEqual(release.resolve_flavor(server="203.0.113.10:4100").contour, "prod")
        release.os.environ[release.DEV_IP_ENV] = "203.0.113.10"
        with self.assertRaises(SystemExit):
            release.resolve_flavor(server="203.0.113.10:4000")
        with self.assertRaises(SystemExit):
            release.resolve_flavor(registry="https://203.0.113.10")
        self.assertEqual(release.resolve_flavor("dev", server="203.0.113.10:4100").server, "203.0.113.10:4100")
        self.assertEqual(release.resolve_flavor(server="77.246.105.57:4000").server, "77.246.105.57:4000")

    def test_zone_matching_is_by_label(self):
        self.assertTrue(release.is_prod_host("omoba.io"))
        self.assertTrue(release.is_prod_host("app.omoba.io"))
        self.assertFalse(release.is_prod_host("notomoba.io"))
        self.assertFalse(release.is_prod_host("lobby.dev.omoba.io"))
        self.assertFalse(release.is_prod_host("dev.ekza.io"))
        self.assertTrue(release.is_dev_host("dev.omoba.io"))
        self.assertFalse(release.is_dev_host("devomoba.io"))
        self.assertFalse(release.is_dev_host("lobby.dev.omoba.io.evil.example"))
        self.assertEqual(release.endpoint_host("Lobby.Dev.Omoba.IO.:4100"), "lobby.dev.omoba.io")
        self.assertEqual(release.registry_host("https://Registry.Dev.Ekza.io./v1"), "registry.dev.ekza.io")

    def test_custom_endpoints_stay_possible_within_a_contour(self):
        lan = release.resolve_flavor(server="192.168.1.20:4000", registry="http://127.0.0.1:8018/")
        self.assertEqual(lan.registry, "http://127.0.0.1:8018")
        self.assertIsNone(lan.config_suffix)
        rehearsal = release.resolve_flavor("dev", server="127.0.0.1:4100", registry="http://localhost:8018")
        self.assertEqual((rehearsal.server, rehearsal.config_suffix), ("127.0.0.1:4100", "dev"))
        by_ip = release.resolve_flavor("dev", server="203.0.113.10:4100")
        self.assertEqual((by_ip.server, by_ip.registry), ("203.0.113.10:4100", "https://registry.dev.ekza.io"))

    def test_registry_follows_the_sdk_url_rule(self):
        self.assertEqual(release.validate_registry("https://registry.dev.ekza.io/"), "https://registry.dev.ekza.io")
        self.assertEqual(release.validate_registry("https://wss.ekza.io/mirror"), "https://wss.ekza.io/mirror")
        self.assertIsNone(release.validate_registry(None))
        for bad in ("http://registry.dev.ekza.io", "https://u:p@registry.dev.ekza.io", "https://x.io?a=1",
                    "https://x.io#f", "registry.dev.ekza.io", "https://x.io:99999", "ftp://x.io",
                    "https://x.io/a b", 'https://x.io/"'):
            with self.assertRaises(SystemExit, msg=bad):
                release.validate_registry(bad)

    def test_builds_never_inherit_compiled_defaults_from_the_shell(self):
        from unittest import mock
        exported = {"OMOBA_DEFAULT_GAME_SERVER_ADDR": "lobby.dev.omoba.io:4100",
                    "OMOBA_DEFAULT_REGISTRY_URL": "https://registry.dev.ekza.io",
                    "OMOBA_DEFAULT_CONFIG_DIR_SUFFIX": "dev", "PATH": "/bin"}
        with mock.patch.dict(release.os.environ, exported, clear=True):
            prod = release.client_env(release.PROD)
            dev = release.client_env(release.resolve_flavor("dev", registry="http://127.0.0.1:8018"))
        for key in release.CLIENT_DEFAULT_ENVS:
            self.assertNotIn(key, prod)
        self.assertEqual(prod["PATH"], "/bin")
        self.assertEqual(dev["OMOBA_DEFAULT_REGISTRY_URL"], "http://127.0.0.1:8018")
        self.assertEqual(release.linux_docker_env_args(release.PROD), [])
        self.assertEqual(release.linux_docker_env_args(release.resolve_flavor("dev")), [
            "-e", "OMOBA_DEFAULT_GAME_SERVER_ADDR=lobby.dev.omoba.io:4100",
            "-e", "OMOBA_DEFAULT_REGISTRY_URL=https://registry.dev.ekza.io",
            "-e", "OMOBA_DEFAULT_CONFIG_DIR_SUFFIX=dev"])

    def test_dev_builds_are_desktop_only(self):
        dev = release.resolve_flavor("dev")
        every = dict.fromkeys(release.PLATFORMS, True)
        self.assertEqual(release.build_targets("all", dev, every), ["macos", "windows", "linux"])
        self.assertEqual(release.build_targets("all", release.PROD, every), list(release.PLATFORMS))
        self.assertEqual(release.build_targets("linux", dev, every), ["linux"])
        for mobile in ("android", "ios"):
            with self.assertRaises(SystemExit):
                release.build_targets(mobile, dev, every)
        with self.assertRaises(SystemExit):
            release.build_android("0.24.0", Path("/nonexistent"), dev)
        with self.assertRaises(SystemExit):
            release.build_ios("0.24.0", Path("/nonexistent"), dev, None, False)

    def test_dev_readme_says_what_it_points_at_and_where_it_keeps_state(self):
        dev = release.player_readme("0.24.0", "macos", release.resolve_flavor("dev"))
        for expected in ("DEV TEST BUILD", "lobby.dev.omoba.io:4100", "https://registry.dev.ekza.io",
                         '"omoba-bevy-dev"', 'xattr -dr com.apple.quarantine "/path/to/Omoba Dev.app"'):
            self.assertIn(expected, dev)
        prod = release.player_readme("0.24.0", "macos", release.resolve_flavor(server="77.246.105.57:4000"))
        self.assertNotIn("DEV", prod)
        self.assertIn("xattr -dr com.apple.quarantine /path/to/Omoba.app\n", prod)
        self.assertIn("77.246.105.57:4000 by default.\n\nPLAY WITH A FRIEND", prod)

    def test_mac_dev_app_has_its_own_identity_and_launcher_paths(self):
        prod = release.mac_info_plist("0.24.0", release.PROD, "Omoba.icns")
        dev = release.mac_info_plist("0.24.0", release.resolve_flavor("dev"), None)
        self.assertEqual((prod["CFBundleIdentifier"], prod["CFBundleName"]), (release.MAC_BUNDLE_ID, "Omoba"))
        self.assertEqual((dev["CFBundleIdentifier"], dev["CFBundleName"]), ("space.ekza.omoba.desktop.dev", "Omoba Dev"))
        self.assertEqual(release.mac_practice_command(release.PROD), """#!/bin/sh
# Practice server with bots for you and your friends (UDP 4000).
DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
export SERVER_ADDR="${SERVER_ADDR:-0.0.0.0:4000}"
export OMOBA_MATCH_MODE=practice
export OMOBA_ASSET_DIR="$DIR/Omoba.app/Contents/Resources/assets"
echo "OMOBA practice server on $SERVER_ADDR. Friends connect to this computer's IP, port 4000."
echo "Close this window to stop the server."
exec "$DIR/Omoba.app/Contents/MacOS/server"
""")
        script = release.mac_practice_command(release.resolve_flavor("dev"))
        self.assertIn('exec "$DIR/Omoba Dev.app/Contents/MacOS/server"', script)
        import subprocess
        self.assertEqual(subprocess.run(["sh", "-n"], input=script, text=True).returncode, 0)

    def test_dev_releases_only_use_dev_tags(self):
        self.assertEqual(release.release_tag("prod", None, "0.24.0"), "v0.24.0")
        self.assertEqual(release.release_tag("dev", "v0.45.0-dev.3", "0.45.0"), "v0.45.0-dev.3")
        for bad in (None, "v0.45.0", "v0.45.0-rc.1", "v0.45.0-dev", "0.45.0-dev.1"):
            with self.assertRaises(SystemExit, msg=bad):
                release.release_tag("dev", bad, "0.45.0")
        with self.assertRaises(SystemExit):
            release.release_tag("prod", "v0.45.0-dev.1", "0.45.0")

    def test_ci_dispatch_matches_the_workflow_inputs(self):
        from unittest import mock

        def args(**values):
            base = dict(contour="prod", server=None, registry=None, ref=None, branch=None)
            return mock.Mock(**{**base, **values})

        self.assertEqual(release.ci_command(args(), "0.24.0"),
                         ["gh", "workflow", "run", "release.yml", "--repo", release.REPO, "--ref", "main",
                          "-f", "tag=v0.24.0"])
        dev = release.ci_command(args(contour="dev", ref="v0.45.0-dev.1"), "0.45.0")
        self.assertEqual(dev[dev.index("--ref") + 1], "dev")
        for field in ("tag=v0.45.0-dev.1", "contour=dev", "server=lobby.dev.omoba.io:4100",
                      "registry=https://registry.dev.ekza.io"):
            self.assertIn(field, dev)
        with self.assertRaises(SystemExit):
            release.ci_command(args(contour="dev"), "0.45.0")
        workflow = (release.ROOT / ".github/workflows/release.yml").read_text()
        for needle in ("      registry:\n", "      contour:\n", '--contour "$RELEASE_CONTOUR"',
                       '--registry "$RELEASE_REGISTRY"', "if: ${{ github.event.inputs.contour != 'dev' }}",
                       "github.event.inputs.contour != 'dev' && vars.OMOBA_RELEASE_SERVER",
                       'title="OMOBA ${RELEASE_TAG#v} DEV (test contour)"'):
            self.assertIn(needle, workflow)

    def test_build_dry_run_plans_without_building_or_creating_dist(self):
        import contextlib
        import io
        import json
        from unittest import mock
        report = {"platforms": dict.fromkeys(release.PLATFORMS, True), "free_gib": 100.0}
        with tempfile.TemporaryDirectory() as temporary, \
                mock.patch.object(release, "check_report", return_value=report), \
                mock.patch.object(release, "workspace_version", return_value="0.24.0"), \
                mock.patch.object(release.subprocess, "check_call", side_effect=AssertionError("ran")), \
                mock.patch.object(release.subprocess, "Popen", side_effect=AssertionError("ran")), \
                contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(release.main(["build", "all", "--contour", "dev", "--dry-run",
                                           "--out", temporary]), 0)
            plan = json.loads(output.getvalue())
            self.assertFalse(Path(temporary, "v0.24.0-dev").exists())
        self.assertEqual(plan["config_dir"], "omoba-bevy-dev")
        self.assertEqual([Path(p).name for p in plan["packages"]], [
            "Omoba-0.24.0-dev-macos-arm64.zip", "Omoba-0.24.0-dev-windows-x64.zip",
            "omoba-0.24.0-dev-linux-x64.tar.gz"])

    def test_compiled_default_names_match_the_rust_sources(self):
        sources = {
            "OMOBA_DEFAULT_GAME_SERVER_ADDR": "client/src/session_config.rs",
            "OMOBA_DEFAULT_REGISTRY_URL": "passport/src/community.rs",
            "OMOBA_DEFAULT_CONFIG_DIR_SUFFIX": "client/src/platform.rs",
        }
        self.assertEqual(set(sources), set(release.CLIENT_DEFAULT_ENVS))
        for key, path in sources.items():
            self.assertIn(f'option_env!("{key}")', (release.ROOT / path).read_text(), path)
        self.assertIn('format!("omoba-bevy-{suffix}")', (release.ROOT / "client/src/platform.rs").read_text())


if __name__ == "__main__":
    unittest.main()
