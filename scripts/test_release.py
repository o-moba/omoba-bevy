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
        # Production has no in-game server picker; the dev app's is its Connection panel.
        self.assertNotIn('"Change" under SERVER', text)
        self.assertIn("GAME_SERVER_ADDR=192.168.1.20:4000", text)
        dev = release.player_readme("0.24.0", "windows", release.resolve_flavor("dev"))
        self.assertIn("Home -> Connection -> Custom", dev)
        self.assertNotIn("GAME_SERVER_ADDR", dev)

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
                    mock.Mock(out=None, gateway="https://g", contour="prod", market="global",
                              dry_run=False)), 0)


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
            "OMOBA_DEFAULT_PASSPORT_URL": "http://127.0.0.1:9/api/passport",
            "OMOBA_DEFAULT_CONFIG_DIR_SUFFIX": "dev",
            "OMOBA_ACCOUNT_API_URL": "https://account.dev.omoba.io",
            "OMOBA_PORTAL_ORIGIN": "https://app.dev.omoba.io",
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
        for registry in ("https://registry.ekza.io", "https://REGISTRY.ekza.io/", "https://studio.ekza.io", "https://hub.ekza.io",
                         "https://registry.ekza.io./v1", "https://wss.ekza.io/mirror", "https://ekza.io"):
            with self.assertRaises(SystemExit, msg=registry):
                release.resolve_flavor("dev", registry=registry)

    def test_a_production_build_refuses_dev_endpoints(self):
        for server in ("lobby.dev.omoba.io:4100", "Lobby.Dev.Omoba.IO.:4100", "dev.omoba.io:4000",
                       "x.dev.ekza.io:4000"):
            with self.assertRaises(SystemExit, msg=server):
                release.resolve_flavor(server=server)
        for registry in ("https://registry.dev.ekza.io", "https://Registry.Dev.Ekza.io/", "https://studio.dev.ekza.io",
                         "https://hub.dev.ekza.io"):
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
        # The RU market's own zone is production too: a dev build never bakes it.
        self.assertTrue(release.is_prod_host("omoba.ru"))
        self.assertTrue(release.is_prod_host("lobby.omoba.ru"))
        self.assertFalse(release.is_prod_host("notomoba.ru"))
        with self.assertRaises(SystemExit):
            release.resolve_flavor("dev", server="lobby.omoba.ru:4000")
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

    # Values a developer may have left exported from dev or LAN work.
    EXPORTED = {"OMOBA_DEFAULT_GAME_SERVER_ADDR": "lobby.dev.omoba.io:4100",
                "OMOBA_DEFAULT_REGISTRY_URL": "https://registry.dev.ekza.io",
                "OMOBA_DEFAULT_PASSPORT_URL": "https://passport.dev.ekza.io/api/passport",
                "OMOBA_DEFAULT_CONFIG_DIR_SUFFIX": "dev",
                "OMOBA_ACCOUNT_API_URL": "https://account.omoba.io",
                "OMOBA_PORTAL_ORIGIN": "https://app.omoba.io",
                "EKZA_DEV_HTTP_HOST": "192.168.1.71",
                "OMOBA_DEFAULT_SOMETHING_NEW": "x",
                "PATH": "/bin", "OMOBA_PASSPORT_URL": "runtime-only"}

    def test_builds_never_inherit_compiled_defaults_from_the_shell(self):
        from unittest import mock
        with mock.patch.dict(release.os.environ, self.EXPORTED, clear=True):
            prod = release.client_env(release.PROD)
            dev = release.client_env(release.resolve_flavor("dev", registry="http://127.0.0.1:8018"))
            xwin = release.apply_client_defaults(dict(release.os.environ), release.PROD)
        for env in (prod, xwin):
            self.assertEqual(env, {"PATH": "/bin", "OMOBA_PASSPORT_URL": "runtime-only"})
        self.assertEqual({k: v for k, v in dev.items() if release.is_build_time_env(k)},
                         release.resolve_flavor("dev", registry="http://127.0.0.1:8018").client_defaults())
        self.assertEqual(dev["OMOBA_DEFAULT_REGISTRY_URL"], "http://127.0.0.1:8018")
        self.assertEqual(dev["OMOBA_DEFAULT_PASSPORT_URL"], release.DEV_PASSPORT)
        self.assertEqual(release.linux_docker_env_args(release.PROD), [])
        self.assertEqual(release.linux_docker_env_args(release.resolve_flavor("dev")), [
            "-e", "OMOBA_DEFAULT_GAME_SERVER_ADDR=lobby.dev.omoba.io:4100",
            "-e", "OMOBA_DEFAULT_REGISTRY_URL=https://registry.dev.ekza.io",
            "-e", "OMOBA_DEFAULT_PASSPORT_URL=http://127.0.0.1:9/api/passport",
            "-e", "OMOBA_DEFAULT_CONFIG_DIR_SUFFIX=dev",
            "-e", "OMOBA_ACCOUNT_API_URL=https://account.dev.omoba.io",
            "-e", "OMOBA_PORTAL_ORIGIN=https://app.dev.omoba.io"])

    def test_the_ios_archive_never_inherits_compiled_defaults_from_the_shell(self):
        from unittest import mock
        seen = []

        def run(*args, env=None, **kwargs):
            seen.append(env)
            raise SystemExit("stop after the archive command")

        with tempfile.TemporaryDirectory() as temporary, \
                mock.patch.dict(release.os.environ, self.EXPORTED, clear=True), \
                mock.patch.object(release, "ROOT", Path(temporary)), \
                mock.patch.object(release, "require_production_source", return_value={'revision': 'test'}), \
                mock.patch.object(release, "host_platform", return_value="macos"), \
                mock.patch.object(release, "local_xcconfig",
                                  return_value={"DEVELOPMENT_TEAM": "TEAM", "CURRENT_PROJECT_VERSION": "1"}), \
                mock.patch.object(release, "run", run):
            with self.assertRaises(SystemExit):
                release.build_ios("0.24.0", Path(temporary), release.resolve_flavor(), None, False)
        self.assertEqual(seen[0], {"PATH": "/bin", "OMOBA_PASSPORT_URL": "runtime-only"})
        for flavor in (release.resolve_flavor(registry="https://registry.ekza.io"),
                       release.resolve_flavor(passport="https://avatar.ekza.io/api/passport")):
            with self.assertRaises(SystemExit):
                release.build_ios("0.24.0", Path("/nonexistent"), flavor, None, False)

    def test_a_dev_build_never_uses_the_production_passport(self):
        dev = release.resolve_flavor("dev")
        self.assertEqual(dev.passport, release.DEV_PASSPORT)
        self.assertEqual(release.registry_host(dev.passport), "127.0.0.1")
        for passport in ("https://avatar.ekza.io/api/passport", "https://AVATAR.ekza.io./api/passport",
                         "https://ekza.io/api/passport", "https://passport.example.com/api/passport",
                         "https://77.246.105.57/api/passport"):
            with self.assertRaises(SystemExit, msg=passport):
                release.resolve_flavor("dev", passport=passport)
        self.assertEqual(release.resolve_flavor("dev", passport="https://passport.dev.ekza.io/api/passport/").passport,
                         "https://passport.dev.ekza.io/api/passport")
        self.assertEqual(release.resolve_flavor("dev", passport="http://localhost:3000/api/passport").passport,
                         "http://localhost:3000/api/passport")
        # Production keeps the SDK default unless asked, and never takes a dev passport.
        self.assertIsNone(release.resolve_flavor().passport)
        self.assertNotIn("OMOBA_DEFAULT_PASSPORT_URL", release.PROD.client_defaults())
        with self.assertRaises(SystemExit):
            release.resolve_flavor(passport="https://passport.dev.ekza.io/api/passport")

    def test_passport_follows_the_sdk_url_rule(self):
        self.assertIsNone(release.validate_passport(None))
        for bad in ("https://avatar.ekza.io", "https://avatar.ekza.io/api", "http://avatar.ekza.io/api/passport",
                    "https://u:p@x.dev.ekza.io/api/passport", "https://x.dev.ekza.io/api/passport?a=1",
                    "https://x.dev.ekza.io/api/passport#f", "ftp://x/api/passport", "x.dev.ekza.io/api/passport",
                    "https://x.dev.ekza.io/api/pass port", "https://x.io:99999/api/passport"):
            with self.assertRaises(SystemExit, msg=bad):
                release.validate_passport(bad)

    def test_the_dev_linux_package_carries_the_lobby_migration_tool(self):
        from unittest import mock
        self.assertEqual(release.linux_binaries(release.PROD), ("client", "server"))
        self.assertEqual(release.linux_binaries(release.resolve_flavor("dev")),
                         ("client", "server", "omoba-account-api"))
        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(release, "stage_common"):
            root = Path(temporary)
            for name in ("client", "server", "account"):
                (root / name).write_bytes(name.encode())
            dev = release.resolve_flavor("dev")
            release.stage_linux(root / "dev", "0.24.0", dev, root / "client", root / "server",
                                {"omoba-account-api": root / "account"})
            self.assertEqual((root / "dev/omoba-account-api").read_bytes(), b"account")
            release.stage_linux(root / "prod", "0.24.0", release.PROD, root / "client", root / "server")
            self.assertFalse((root / "prod/omoba-account-api").exists())
            with self.assertRaises(SystemExit):
                release.stage_linux(root / "dev2", "0.24.0", dev, root / "client", root / "server")
            with self.assertRaises(SystemExit):
                release.stage_linux(root / "prod2", "0.24.0", release.PROD, root / "client", root / "server",
                                    {"omoba-account-api": root / "account"})
        readme = release.player_readme("0.24.0", "linux", release.resolve_flavor("dev"))
        self.assertIn("omoba-account-api is the dev lobby's database migration tool", readme)
        self.assertIn("wallet pairing and purchased avatars are off", readme)
        self.assertNotIn("omoba-account-api", release.player_readme("0.24.0", "macos", release.resolve_flavor("dev")))

    def test_dev_builds_cover_every_platform_with_their_own_app(self):
        dev = release.resolve_flavor("dev")
        every = dict.fromkeys(release.PLATFORMS, True)
        self.assertEqual(release.build_targets("all", dev, every), list(release.PLATFORMS))
        self.assertEqual(release.build_targets("all", release.PROD, every), list(release.PLATFORMS))
        for target in release.PLATFORMS:
            self.assertEqual(release.build_targets(target, dev, every), [target])
        # Phones compile the dev app's own registry and passport; only the lobby changes.
        for flavor in (release.resolve_flavor("dev", registry="https://registry.lan.example"),
                       release.resolve_flavor("dev", passport="https://passport.dev.ekza.io/api/passport")):
            with self.assertRaises(SystemExit):
                release.build_android("0.24.0", Path("/nonexistent"), flavor)
            with self.assertRaises(SystemExit):
                release.build_ios("0.24.0", Path("/nonexistent"), flavor, None, False)

    def test_dev_phone_builds_hand_the_contour_to_the_packagers(self):
        from unittest import mock
        seen = []

        def run(*args, env=None, **kwargs):
            seen.append(([str(a) for a in args], env))
            raise SystemExit("stop after the packager command")

        with tempfile.TemporaryDirectory() as temporary, \
                mock.patch.dict(release.os.environ, {"PATH": "/bin"}, clear=True), \
                mock.patch.object(release, "shared_target", return_value=Path(temporary)), \
                mock.patch.object(release, "ensure_android_keystore",
                                  return_value=Path(temporary, "key")), \
                mock.patch.object(release, "android_ndk", return_value=None), \
                mock.patch.object(release, "run", run):
            Path(temporary, "key").write_bytes(b"k")
            for flavor in (release.PROD, release.resolve_flavor("dev", server="203.0.113.10:4100")):
                with self.assertRaises(SystemExit):
                    release.build_android("0.24.0", Path(temporary), flavor)
        (prod_args, _), (dev_args, dev_env) = seen
        self.assertEqual(prod_args[prod_args.index("--contour") + 1], "prod")
        self.assertEqual(dev_args[dev_args.index("--contour") + 1], "dev")
        self.assertEqual(dev_args[dev_args.index("--server") + 1], "203.0.113.10:4100")
        self.assertEqual(dev_env["OMOBA_DEFAULT_CONFIG_DIR_SUFFIX"], "dev")
        dev_ios = release.ios_settings(release.resolve_flavor("dev"), "TEAM", "7")
        self.assertIn("OMOBA_CONTOUR=dev", dev_ios)
        self.assertIn("OMOBA_GAME_SERVER=lobby.dev.omoba.io:4100", dev_ios)
        self.assertNotIn("OMOBA_CONTOUR=dev", release.ios_settings(
            release.resolve_flavor(server="192.0.2.1:4000"), "TEAM", "7"))

    def test_ios_archive_command_selects_the_matching_shared_scheme(self):
        from unittest import mock
        for contour, scheme, configuration in [('prod', 'Omoba', 'Release'),
                                                ('dev', 'Omoba Dev', 'Release Dev')]:
            with self.subTest(contour=contour), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                with mock.patch.object(release, 'ROOT', root), \
                     mock.patch.object(release, 'require_production_source', return_value={'revision': 'test'}) as guard, \
                     mock.patch.object(release, 'host_platform', return_value='macos'), \
                     mock.patch.object(release, 'local_xcconfig', return_value={'DEVELOPMENT_TEAM': 'TEAM'}), \
                     mock.patch.object(release, 'run', side_effect=RuntimeError('stop before signing')) as run:
                    with self.assertRaisesRegex(RuntimeError, 'stop before signing'):
                        release.build_ios('0.44.0', root/'dist', release.resolve_flavor(contour), '42', False)
                args = run.call_args.args
                self.assertEqual(args[args.index('-scheme') + 1], scheme)
                self.assertEqual(args[args.index('-configuration') + 1], configuration)
                self.assertIn(f'OMOBA_CONTOUR={contour}', args)
                self.assertEqual(guard.call_count, 0 if contour == 'dev' else 1)

    def test_a_production_ios_archive_pins_its_contour_and_server_over_the_local_xcconfig(self):
        # Omoba.local.xcconfig may hold `OMOBA_CONTOUR = dev` or a LAN OMOBA_GAME_SERVER
        # for Xcode runs; command-line settings win, so a release always names both.
        self.assertEqual(release.ios_settings(release.PROD, "TEAM", "7"),
                         ["DEVELOPMENT_TEAM=TEAM", "CURRENT_PROJECT_VERSION=7",
                          "OMOBA_CARGO_PROFILE=release", "CODE_SIGN_STYLE=Automatic",
                          "OMOBA_CONTOUR=prod", "OMOBA_GAME_SERVER="])
        self.assertEqual(release.ios_settings(release.resolve_flavor(server="192.0.2.1:4000"), "TEAM", "7")[-2:],
                         ["OMOBA_CONTOUR=prod", "OMOBA_GAME_SERVER=192.0.2.1:4000"])
        # Each contour's setting is one Omoba.xcconfig and xcode_build.py both understand.
        from contours import contour_of_xcode_setting
        xcconfig = (release.ROOT / "mobile/ios/Omoba.xcconfig").read_text()
        for contour in release.CONTOURS:
            flavor = release.resolve_flavor(contour)
            setting = next(item for item in release.ios_settings(flavor, "TEAM", "7")
                           if item.startswith("OMOBA_CONTOUR="))
            value = setting.split("=", 1)[1]
            self.assertEqual(contour_of_xcode_setting(value), contour)
            self.assertIn(f"OMOBA_BUNDLE_ID_{value} = {release.MOBILE[contour].bundle_id}", xcconfig)

    def test_an_ios_archive_of_the_wrong_app_is_refused(self):
        import plistlib

        def archive_of(directory: Path, bundle_id: str | None, summary: bool) -> Path:
            archive = directory / "Omoba.xcarchive"
            app = archive / "Products/Applications/OmobaBeta.app"
            app.mkdir(parents=True)
            if bundle_id:
                (app / "Info.plist").write_bytes(plistlib.dumps({"CFBundleIdentifier": bundle_id}))
            if summary:
                (archive / "Info.plist").write_bytes(plistlib.dumps(
                    {"ApplicationProperties": {"CFBundleIdentifier": bundle_id}}))
            return archive

        prod, dev = release.PROD, release.resolve_flavor("dev")
        cases = [(prod, "space.ekza.omoba.beta", True), (dev, "space.ekza.omoba.dev", False)]
        for flavor, bundle_id, summary in cases:
            with self.subTest(flavor.contour), tempfile.TemporaryDirectory() as temporary:
                archive = archive_of(Path(temporary), bundle_id, summary)
                self.assertEqual(release.archive_bundle_id(archive), bundle_id)
                release.check_archive_identity(archive, flavor)
        refused = [(prod, "space.ekza.omoba.dev"), (dev, "space.ekza.omoba.beta"), (prod, None)]
        for flavor, bundle_id in refused:
            with self.subTest(f"{flavor.contour} as {bundle_id}"), \
                    tempfile.TemporaryDirectory() as temporary:
                archive = archive_of(Path(temporary), bundle_id, False)
                with self.assertRaises(SystemExit):
                    release.check_archive_identity(archive, flavor)

    def test_the_phone_apps_never_share_an_identity(self):
        import plistlib
        prod, dev = release.MOBILE["prod"], release.MOBILE["dev"]
        self.assertEqual((prod.bundle_id, dev.bundle_id), ("space.ekza.omoba.beta", "space.ekza.omoba.dev"))
        self.assertEqual(dev.display_name, "OMOBA Dev")
        self.assertNotEqual(prod.ios_app_icon, dev.ios_app_icon)
        self.assertNotEqual(prod.android_res, dev.android_res)
        # The production identity is still what the checked-in app files say.
        info = plistlib.loads((release.ROOT / "mobile/ios/Info.plist").read_bytes())
        self.assertEqual((info["CFBundleIdentifier"], info["CFBundleDisplayName"]),
                         (prod.bundle_id, prod.display_name))
        manifest = (release.ROOT / "mobile/android/AndroidManifest.xml").read_text()
        self.assertIn(f'package="{prod.bundle_id}"', manifest)
        self.assertIn(f'android:label="{prod.display_name}"', manifest)
        for identity in (prod, dev):
            self.assertTrue((release.ROOT / "mobile/ios/Assets.xcassets"
                             / f"{identity.ios_app_icon}.appiconset/AppIcon.png").is_file())
            self.assertTrue((release.ROOT / "mobile/android" / identity.android_res
                             / "mipmap-xxxhdpi/ic_launcher.png").is_file())
        self.assertTrue(release.mac_icon_source(release.resolve_flavor("dev")).parent.name
                        .startswith("AppIconDev"))
        self.assertEqual(release.mac_icon_source(release.PROD).parent.name, "AppIcon.appiconset")

    def test_the_dev_endpoints_match_the_clients_dev_profile(self):
        source = (release.ROOT / "client/src/connection_profile.rs").read_text()
        self.assertIn(f'CLOSED_PASSPORT: &str = "{release.DEV_PASSPORT}"', source)
        self.assertIn('"https://registry.{contour}.ekza.io"', source)
        self.assertEqual(release.DEV_REGISTRY, "https://registry.dev.ekza.io")
        self.assertIn('"lobby.{contour}.omoba.io:4100"', source)
        self.assertEqual(release.DEV_LOBBY, "lobby.dev.omoba.io:4100")

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
            base = dict(contour="prod", market="global", server=None, registry=None, ref=None, branch=None)
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
                       '--registry "$RELEASE_REGISTRY"',
                       'args=(build android --out dist --contour "$RELEASE_CONTOUR")',
                       "github.event.inputs.contour != 'dev' && vars.OMOBA_RELEASE_SERVER",
                       'title="OMOBA ${RELEASE_TAG#v} DEV (test contour)"'):
            self.assertIn(needle, workflow)

    def test_ci_runs_the_client_in_the_dev_contour_and_pull_requests_build_the_dev_apk(self):
        # BUILD_CONTOUR = Some("dev") code only compiles with the dev defaults, so CI
        # runs the client's clippy and tests with exactly the OMOBA Dev app's values,
        # in a scratch settings folder.
        ci = (release.ROOT / ".github/workflows/ci.yml").read_text()
        job = ci[ci.index("  dev-contour:\n"):ci.index("  harness:\n")]
        for key, value in release.resolve_flavor("dev").client_defaults().items():
            self.assertIn(f"      {key}: {value}\n", job)
        for needle in ("OMOBA_CLIENT_CONFIG_DIR=$RUNNER_TEMP/",
                       "cargo clippy -p client --all-targets --no-deps --locked -- -D warnings",
                       "cargo clippy -p client --lib --no-deps --no-default-features --locked -- -D warnings",
                       "cargo test -p client --locked"):
            self.assertIn(needle, job)
        workflow = (release.ROOT / ".github/workflows/release.yml").read_text()
        self.assertIn("if: github.event_name == 'pull_request'\n", workflow)
        self.assertIn("run: python3 scripts/release.py build android --out dist --contour dev\n", workflow)

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
            "omoba-0.24.0-dev-linux-x64.tar.gz", release.artifact_name("0.24.0", "android", "dev"),
            "Omoba-0.24.0-dev-ios.ipa"])

    def test_compiled_default_names_match_the_rust_sources(self):
        sources = {
            "OMOBA_DEFAULT_GAME_SERVER_ADDR": "client/src/session_config.rs",
            "OMOBA_DEFAULT_REGISTRY_URL": "passport/src/community.rs",
            "OMOBA_DEFAULT_PASSPORT_URL": "passport/src/lib.rs",
            "OMOBA_DEFAULT_CONFIG_DIR_SUFFIX": "client/src/platform.rs",
        }
        self.assertEqual(set(sources), set(release.CLIENT_DEFAULT_ENVS))
        for key, path in sources.items():
            self.assertIn(f'option_env!("{key}")', (release.ROOT / path).read_text(), path)
        # scripts/markets.py config_dir mirrors this rule: omoba-bevy[-<market>][-<contour>].
        platform = (release.ROOT / "client/src/platform.rs").read_text()
        self.assertIn("market.storage_suffix()", platform)
        self.assertIn('"omoba-bevy".to_owned()', platform)

    def test_every_compile_time_setting_in_the_sources_is_scrubbed(self):
        import re
        import subprocess
        listing = subprocess.run(["git", "grep", "-h", "-o", "-E", r'option_env!\("[A-Z0-9_]+"\)', "--", "*.rs"],
                                 cwd=release.ROOT, capture_output=True, text=True, check=True).stdout
        keys = set(re.findall(r'option_env!\("([A-Z0-9_]+)"\)', listing))
        self.assertIn("OMOBA_DEFAULT_PASSPORT_URL", keys)
        self.assertTrue(keys, "git grep found no option_env! reads")
        self.assertEqual({key for key in keys if not release.is_build_time_env(key)}, set())
        # The pinned Ekza SDK reads EKZA_DEV_HTTP_HOST at compile time (src/registry.rs).
        self.assertTrue(release.is_build_time_env("EKZA_DEV_HTTP_HOST"))
        self.assertIn("ekza-bevy-sdk", (release.ROOT / "Cargo.toml").read_text())


class Markets(unittest.TestCase):
    def test_presets_mirror_the_rust_market_module(self):
        import re
        import markets
        source = (release.ROOT / "shared/src/market.rs").read_text()
        features = source[source.index("pub const fn default_features"):source.index("pub const fn default_locale")]
        locales = source[source.index("pub const fn default_locale"):source.index("pub const fn storage_suffix")]
        suffixes = source[source.index("pub const fn storage_suffix"):source.index("impl Features")]
        names = {"global": "Global", "ru": "Ru", "cn": "Cn"}
        self.assertEqual(set(names), set(markets.MARKETS))
        for key, variant in names.items():
            preset = markets.MARKETS[key]
            arm = re.search(rf"Self::{variant} => Features \{{(.*?)\}}", features, re.S).group(1)
            rust = {name: value == "true" for name, value in re.findall(r"(\w+): (true|false)", arm)}
            self.assertEqual(rust, preset.features, key)
            self.assertIn(f'Self::{variant} => "{preset.locale}"', locales)
            expected = f'Some("{preset.storage_suffix}")' if preset.storage_suffix else "None"
            self.assertIn(f"Self::{variant} => {expected}", suffixes)
        for name, env in markets.FEATURE_ENVS.items():
            self.assertIn(f'option_env!("{env}")', source, name)
        self.assertIn('option_env!("OMOBA_MARKET")', source)

    def test_ru_build_bakes_its_market_servers_folder_and_names(self):
        ru = release.resolve_flavor(market="ru")
        self.assertEqual((ru.server, ru.account_api, ru.portal),
                         ("lobby.omoba.ru:4000", "https://account.omoba.ru", "https://omoba.ru"))
        self.assertEqual((ru.registry, ru.passport), (None, None))
        self.assertEqual(ru.config_dir, "omoba-bevy-ru")
        self.assertEqual(ru.label, "-ru")
        self.assertEqual(ru.bundle_id, "space.ekza.omoba.desktop.ru")
        defaults = ru.client_defaults()
        self.assertEqual(defaults["OMOBA_MARKET"], "ru")
        self.assertEqual(defaults["OMOBA_DEFAULT_GAME_SERVER_ADDR"], "lobby.omoba.ru:4000")
        self.assertNotIn("OMOBA_DEFAULT_REGISTRY_URL", defaults)
        self.assertFalse(any(key.startswith("OMOBA_FEATURE_") for key in defaults))
        self.assertEqual(release.artifact_name("0.45.0", "windows", "prod", "ru"), "Omoba-0.45.0-ru-windows-x64.zip")
        self.assertEqual(release.dist_path("0.45.0", Path("/d"), "prod", "ru"), Path("/d/v0.45.0-ru"))
        self.assertEqual(release.linux_binaries(ru), ("client", "server", "omoba-account-api"))
        self.assertEqual(release.build_targets("all", ru, {p: True for p in release.PLATFORMS}),
                         ["macos", "windows", "linux", "android"])
        with self.assertRaises(SystemExit):
            release.build_targets("ios", ru, {p: True for p in release.PLATFORMS})
        # The global build is unchanged.
        self.assertEqual(release.PROD.client_defaults(), {})
        self.assertEqual(release.PROD.config_dir, "omoba-bevy")

    def test_markets_never_cross_servers_and_overrides_are_explicit(self):
        for bad in (dict(market="ru", registry="https://registry.ekza.io"),
                    dict(market="ru", server="77.246.105.57:4000"),
                    dict(server="lobby.omoba.ru:4000"),
                    dict(market="ru", account_api="https://account.omoba.ru"),
                    dict(market="cn"),
                    dict(market="ru", contour="dev", account_api="https://a.example", portal="https://p.example"),
                    dict(market="ru", portal="http://evil.example")):
            with self.subTest(bad), self.assertRaises(SystemExit):
                release.resolve_flavor(**bad)
        on = release.resolve_flavor(market="ru", features={"ekza": True}, registry="https://registry.ekza.io")
        self.assertEqual(on.client_defaults()["OMOBA_FEATURE_EKZA"], "on")
        self.assertTrue(on.resolved_features["ekza"])
        free_global = release.resolve_flavor(features={"supporter": False})
        self.assertEqual(free_global.client_defaults(), {"OMOBA_FEATURE_SUPPORTER": "off"})
        local = release.resolve_flavor(market="ru", server="127.0.0.1:4000",
                                       account_api="http://127.0.0.1:40550", portal="http://127.0.0.1:3010")
        self.assertEqual(local.client_defaults()["OMOBA_PORTAL_ORIGIN"], "http://127.0.0.1:3010")
        import markets
        self.assertEqual(markets.parse_feature_overrides(["ekza=on", "vk_sign_in=off"]),
                         {"ekza": True, "vk_sign_in": False})
        for bad in (["ekza"], ["ekza=yes"], ["wallet=on"], ["ekza=on", "ekza=off"]):
            with self.subTest(bad), self.assertRaises(SystemExit):
                markets.parse_feature_overrides(bad)

    def test_ru_dev_is_the_dev_app_with_ru_features(self):
        flavor = release.resolve_flavor("dev", market="ru")
        self.assertEqual(flavor.config_dir, "omoba-bevy-ru-dev")
        self.assertEqual(flavor.label, "-ru-dev")
        defaults = flavor.client_defaults()
        self.assertEqual(defaults["OMOBA_MARKET"], "ru")
        self.assertEqual(defaults["OMOBA_DEFAULT_GAME_SERVER_ADDR"], release.DEV_LOBBY)
        self.assertEqual(defaults["OMOBA_ACCOUNT_API_URL"], release.DEV_ACCOUNT_API)
        self.assertNotIn("OMOBA_DEFAULT_REGISTRY_URL", defaults)

    def test_market_releases_use_their_own_tag_and_stay_off_the_workflow(self):
        from unittest import mock
        self.assertEqual(release.release_tag("prod", None, "0.45.0", "ru"), "v0.45.0-ru")
        for contour, tag in (("prod", "v0.45.0"), ("dev", None)):
            with self.subTest(contour), self.assertRaises(SystemExit):
                release.release_tag(contour, tag, "0.45.0", "ru")
        with self.assertRaises(SystemExit):
            release.ci_command(mock.Mock(contour="prod", market="ru", server=None, registry=None,
                                         ref=None, branch=None), "0.45.0")

    def test_preflight_reports_each_server(self):
        down = release.preflight_report("ru", resolve=lambda host: (_ for _ in ()).throw(OSError()),
                                        fetch=lambda url: 503)
        self.assertFalse(down["ready"])
        self.assertEqual(down["checks"]["account_api"]["url"], "https://account.omoba.ru/health/ready")
        up = release.preflight_report("ru", resolve=lambda host: ["203.0.113.7"], fetch=self.ru_host())
        self.assertTrue(up["ready"], up)
        self.assertFalse(release.preflight_report("cn")["ready"])

    @staticmethod
    def ru_host(**overrides):
        """A deployed RU host as the preflight sees it (ops/vps/ru/README.md#topology)."""
        answers = {"https://omoba.ru/api/me": 401, "https://account.omoba.ru/v1/me": 403, **overrides}
        return lambda url: answers.get(url, 200)

    def test_preflight_fails_on_a_403_from_a_key_gated_route(self):
        dns = lambda host: ["203.0.113.7"]
        for url in ("https://omoba.ru/api/me", "https://omoba.ru/api/releases"):
            with self.subTest(url):
                report = release.preflight_report("ru", resolve=dns, fetch=self.ru_host(**{url: 403}))
                self.assertFalse(report["ready"])
                failed = [check for check in report["checks"].values() if not check["ok"]]
                self.assertEqual([check["url"] for check in failed], [url])
                self.assertIn("LR-02", failed[0]["problem"])
        # A portal that cannot use the Account API at all is not ready either.
        for status in (400, 500, 502, 503):
            report = release.preflight_report("ru", resolve=dns, fetch=self.ru_host(**{"https://omoba.ru/api/me": status}))
            self.assertFalse(report["checks"]["portal_account"]["ok"], status)
            self.assertNotIn("problem", report["checks"]["portal_account"])
        # The portal and health answer, but the gated route is unreachable over the network.
        def unreachable(url):
            if url.endswith("/api/releases"):
                raise OSError("reset")
            return self.ru_host()(url)
        self.assertEqual(release.preflight_report("ru", resolve=dns, fetch=unreachable)["checks"]["portal_releases"]["error"], "OSError")

    def test_preflight_fails_when_the_public_account_api_passes_portal_routes(self):
        report = release.preflight_report("ru", resolve=lambda host: ["203.0.113.7"],
                                          fetch=self.ru_host(**{"https://account.omoba.ru/v1/me": 401}))
        self.assertFalse(report["ready"])
        closed = report["checks"]["account_api_portal_routes_closed"]
        self.assertEqual((closed["status"], closed["expected"]), (401, 403))
        self.assertIn("Caddyfile.ru", closed["problem"])

    def test_catalog_lists_only_verified_downloads_in_the_api_schema(self):
        receipt = {"windows": {"file": "Omoba-0.45.0-ru-windows-x64.zip", "bytes": 123, "sha256": "a" * 64,
                               "cid": "c", "url": "https://ekza.mypinata.cloud/ipfs/c/Omoba-0.45.0-ru-windows-x64.zip"}}
        catalog = release.release_catalog(receipt, "0.45.0", "ru", "2026-10-23T00:00:00Z")
        self.assertEqual(len(catalog["releases"]), 1)
        entry = catalog["releases"][0]
        self.assertEqual(set(entry), {"platform", "architecture", "version", "url", "sha256", "size_bytes",
                                      "published_at", "instructions", "test_status"})
        self.assertEqual((entry["platform"], entry["size_bytes"]), ("windows", "123"))
        self.assertIn("Omoba.exe", entry["instructions"])
        with self.assertRaises(SystemExit):
            release.release_catalog({"linux": {**receipt["windows"], "url": "http://x/y"}}, "0.45.0", "ru", "t")

    def test_the_client_reads_the_catalog_platform_names(self):
        """LR-14: the game finds its own package in releases.json by these names."""
        import re
        source = (release.ROOT / "client/src/update.rs").read_text()
        table = source[source.index("const CATALOG_PLATFORMS"):source.index("const MISMATCH_REFRESH")]
        client = dict(re.findall(r'\("([a-z]+)", "([a-z0-9_]+)"\)', table))
        self.assertEqual(set(client.values()), {platform for platform, _ in release.CATALOG_PLATFORMS.values()})
        for _, architecture in release.CATALOG_PLATFORMS.values():
            # The client maps Rust's aarch64 to arm64; a universal package matches any.
            self.assertIn(architecture, {"arm64", "x86_64", "universal"})
        self.assertIn('pub const UNIVERSAL: &str = "universal";',
                      (release.ROOT / "passport/src/release_catalog.rs").read_text())

    def test_android_market_app_has_its_own_identity_and_endpoints(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location("android_build", release.ROOT / "mobile/android/build.py")
        build = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(build)
        self.assertEqual(build.identity_for("prod", "ru"), ("space.ekza.omoba.ru", "OMOBA"))
        self.assertEqual(build.identity_for("dev", "ru"), ("space.ekza.omoba.dev", "OMOBA Dev"))
        self.assertEqual(build.identity_for(), ("space.ekza.omoba.beta", "Omoba Beta"))
        self.assertEqual(build.apk_name("0.45.0", "arm64", "prod", "ru"), "omoba-0.45.0-ru-android-arm64-debug.apk")
        env = build.contour_env({"OMOBA_DEFAULT_CONFIG_DIR_SUFFIX": "x"}, "prod", None, "ru")
        self.assertEqual((env["OMOBA_MARKET"], env["OMOBA_DEFAULT_GAME_SERVER_ADDR"], env["OMOBA_PORTAL_ORIGIN"]),
                         ("ru", "lobby.omoba.ru:4000", "https://omoba.ru"))
        self.assertNotIn("OMOBA_DEFAULT_CONFIG_DIR_SUFFIX", env)
        with self.assertRaises(RuntimeError):
            build.contour_env({"OMOBA_MARKET": "cn"}, "prod", None, "ru")
        manifest = build.manifest_for(
            (release.ROOT / "mobile/android/AndroidManifest.xml").read_text(), "0.45.0", 7, "prod", "ru")
        self.assertIn('package="space.ekza.omoba.ru"', manifest)
        self.assertIn('android:label="OMOBA"', manifest)


if __name__ == "__main__":
    unittest.main()
