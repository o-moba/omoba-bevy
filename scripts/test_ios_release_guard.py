"""Exercise real Git refs/status without compiling or publishing anything."""
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from ios_release_guard import require_production_source
import release


class ProductionSourceTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.git('init', '-q', '-b', 'main')
        self.git('config', 'user.name', 'Release Guard Test')
        self.git('config', 'user.email', 'test@example.invalid')
        self.git('config', 'commit.gpgsign', 'false')
        self.git('config', 'tag.gpgsign', 'false')
        (self.root/'source.rs').write_text('const VERSION: u8 = 1;\n')
        (self.root/'.gitignore').write_text('/builds/\n/mobile/ios/Omoba.local.xcconfig\n')
        self.git('add', '.')
        self.git('commit', '-qm', 'fixture')
        self.git('update-ref', 'refs/remotes/origin/main', 'HEAD')
        self.git('tag', 'v0.45.0')

    def git(self, *args):
        return subprocess.check_output(['git', *args], cwd=self.root, text=True,
                                       stderr=subprocess.PIPE).strip()

    def reject(self, reason):
        with self.assertRaisesRegex(ValueError, reason) as raised:
            require_production_source(self.root)
        self.assertIn('Omoba Dev', str(raised.exception))

    def test_allows_clean_main_and_detached_stable_release(self):
        for detached in (False, True):
            if detached:
                self.git('checkout', '--detach', '-q', 'HEAD')
            result = require_production_source(self.root)
            self.assertEqual(result['revision'], self.git('rev-parse', 'HEAD'))
            self.assertEqual(result['branch'], None if detached else 'main')
            self.assertEqual(result['stable_tags'], ['v0.45.0'])

    def test_allows_release_ancestor_of_local_origin_main(self):
        release_sha = self.git('rev-parse', 'HEAD')
        self.git('commit', '--allow-empty', '-qm', 'later main')
        self.git('update-ref', 'refs/remotes/origin/main', 'HEAD')
        self.git('checkout', '--detach', '-q', release_sha)
        self.assertEqual(require_production_source(self.root)['revision'], release_sha)

    def test_rejects_dev_feature_and_release_branches_at_same_valid_tag(self):
        for branch in ('dev', 'feature/test', 'release/0.45'):
            with self.subTest(branch=branch):
                self.git('checkout', '-qb', branch)
                self.reject('checked out branch')

    def test_rejects_unstaged_staged_and_deleted_tracked_source(self):
        path = self.root/'source.rs'
        path.write_text('changed\n')
        self.reject('uncommitted tracked')
        self.git('add', 'source.rs')
        self.reject('uncommitted tracked')
        self.git('restore', '--staged', '--worktree', 'source.rs')
        path.unlink()
        self.reject('uncommitted tracked')

    def test_rejects_untracked_source_but_allows_ignored_local_settings_and_outputs(self):
        for relative in ('mobile/ios/Omoba.local.xcconfig', 'builds/archive.log'):
            path = self.root/relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('local fixture\n')
        require_production_source(self.root)
        (self.root/'new.rs').write_text('untracked source\n')
        self.reject('untracked files')

    def test_requires_exact_stable_tag_not_prerelease_or_ancestor(self):
        self.git('tag', '-d', 'v0.45.0')
        self.reject('no exact stable')
        for tag in ('v0.45.0-dev.1', 'v0.45.0-rc.1', 'v0.45.0+build', 'v00.45.0'):
            self.git('tag', tag)
        self.reject('no exact stable')
        self.git('tag', 'v0.45.0')
        self.git('commit', '--allow-empty', '-qm', 'after tag')
        self.git('update-ref', 'refs/remotes/origin/main', 'HEAD')
        self.reject('no exact stable')

    def test_rejects_missing_main_reference_and_non_main_reachable_commit(self):
        self.git('update-ref', '-d', 'refs/remotes/origin/main')
        self.reject('required reference')
        self.git('update-ref', 'refs/remotes/origin/main', 'HEAD')
        self.git('commit', '--allow-empty', '-qm', 'unreleased')
        self.git('tag', 'v0.46.0')
        self.reject('not reachable')

    def test_rejects_missing_git_and_parent_repository_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, 'metadata'):
                require_production_source(Path(directory))
        child = self.root/'nested'
        child.mkdir()
        with self.assertRaisesRegex(ValueError, 'checkout root'):
            require_production_source(child)

    def test_release_entry_point_rejects_before_any_build_or_signing_config_access(self):
        self.git('checkout', '-qb', 'dev')
        with mock.patch.object(release, 'ROOT', self.root), \
             mock.patch.object(release, 'local_xcconfig') as config, \
             mock.patch.object(release, 'run') as effects:
            with self.assertRaisesRegex(SystemExit, 'Production iOS archive blocked'):
                release.build_ios('0.45.0', self.root/'dist', release.PROD, '1', True)
        config.assert_not_called()
        effects.assert_not_called()
        self.assertFalse((self.root/'target').exists())

    def test_release_checks_source_again_before_export_or_upload(self):
        def archive_and_dirty(*args, **kwargs):
            (self.root/'source.rs').write_text('changed during archive\n')
        with mock.patch.object(release, 'ROOT', self.root), \
             mock.patch.object(release, 'host_platform', return_value='macos'), \
             mock.patch.object(release, 'local_xcconfig', return_value={'DEVELOPMENT_TEAM': 'TEAM'}), \
             mock.patch.object(release, 'check_archive_identity'), \
             mock.patch.object(release, 'run', side_effect=archive_and_dirty) as effects:
            with self.assertRaisesRegex(SystemExit, 'uncommitted tracked'):
                release.build_ios('0.45.0', self.root/'dist', release.PROD, '1', True)
        self.assertEqual(effects.call_count, 1)
        self.assertIn('archive', effects.call_args.args)
        self.assertFalse((self.root/'target/release-ios/ExportOptions.plist').exists())

    def test_remembered_dev_build_never_overwrites_primary_checkout_settings(self):
        primary = self.root/'primary'
        linked = self.root/'linked'
        relative = Path('mobile/ios/Omoba.local.xcconfig')
        prod_text = 'DEVELOPMENT_TEAM = PRODTEAM\nCURRENT_PROJECT_VERSION = 100\nOMOBA_CONTOUR = prod\n'
        for root, content in ((primary, prod_text), (linked, 'DEVELOPMENT_TEAM = DEVTEAM\n')):
            (root/relative).parent.mkdir(parents=True)
            (root/relative).write_text(content)
        with mock.patch.object(release, 'ROOT', linked), \
             mock.patch.object(release.subprocess, 'check_output', return_value=str(primary/'.git')):
            release.remember_ios_build('12')
            release.remember_ios_build('13')
        self.assertEqual((primary/relative).read_text(), prod_text)
        self.assertEqual((linked/relative).read_text(), 'DEVELOPMENT_TEAM = DEVTEAM\nCURRENT_PROJECT_VERSION = 13\n')


if __name__ == '__main__':
    unittest.main()
