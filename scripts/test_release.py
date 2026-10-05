import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace
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
        text = release.player_readme("0.24.0", "windows", "10.0.0.2:4000")
        self.assertIn("10.0.0.2:4000", text)
        self.assertIn("Host Practice Server.bat", text)
        self.assertIn("PLAY VS BOTS", text)

    def test_default_package_instructions_preserve_beta_and_saved_servers(self):
        text = release.player_readme("0.43.0", "android", None)
        self.assertIn("OMOBA Beta", text)
        self.assertIn("previously saved custom server", text)
        self.assertNotIn("starts on 127.0.0.1", text)

    def test_release_rejects_missing_stable_android_key_before_creating_one(self):
        with patch.dict(release.os.environ, {"OMOBA_REQUIRE_STABLE_ANDROID_KEY": "1"}, clear=True), \
                patch.object(release, "run") as run:
            with self.assertRaisesRegex(SystemExit, "refusing a new signing key"):
                release.ensure_android_keystore()
            run.assert_not_called()

    def test_draft_command_never_overwrites_a_published_release(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "CHANGELOG.md").write_text("## [0.43.0]\n- beta\n")
            out = root / "dist" / "v0.43.0"
            out.mkdir(parents=True)
            (out / "game.apk").write_bytes(b"candidate")
            args = SimpleNamespace(tag="v0.43.0", out=root / "dist", target="main")
            with patch.object(release, "ROOT", root), \
                    patch.object(release, "workspace_version", return_value="0.43.0"), \
                    patch.object(release.subprocess, "run", return_value=SimpleNamespace(
                        returncode=0, stdout='{"isDraft":false}')), \
                    patch.object(release, "run") as mutate:
                with self.assertRaisesRegex(SystemExit, "immutable"):
                    release.command_draft(args)
                mutate.assert_not_called()

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


if __name__ == "__main__":
    unittest.main()
