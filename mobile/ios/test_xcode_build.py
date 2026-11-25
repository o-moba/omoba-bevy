import os
from pathlib import Path
import plistlib
import shutil
import sys
import json
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ET
import xcode_build as xb


class XcodeBridgeTests(unittest.TestCase):
    def test_production_archive_guard_precedes_settings_and_cargo_even_unsigned(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch.object(xb, 'ROOT', Path(directory)), \
                 patch.object(xb, 'settings') as settings, patch.object(xb.subprocess, 'run', wraps=subprocess.run) as run:
                with self.assertRaisesRegex(ValueError, 'Production iOS archive blocked'):
                    xb.build({'ACTION': 'install', 'OMOBA_CONTOUR': 'prod', 'CODE_SIGNING_ALLOWED': 'NO'})
            settings.assert_not_called()
            self.assertTrue(all(call.args[0][0] == 'git' for call in run.call_args_list))

    def test_dev_archive_and_production_run_do_not_use_production_archive_guard(self):
        for env in ({'ACTION': 'install', 'OMOBA_CONTOUR': 'dev'},
                    {'ACTION': 'build', 'OMOBA_CONTOUR': 'prod'}):
            with self.subTest(env=env), patch.object(xb, 'require_production_source') as guard, \
                 patch.object(xb, 'settings', side_effect=RuntimeError('reached build settings')):
                with self.assertRaisesRegex(RuntimeError, 'reached build settings'):
                    xb.build(env)
                guard.assert_not_called()

    def env(self, root):
        return dict(PLATFORM_NAME='iphoneos', ARCHS='arm64', CURRENT_PROJECT_VERSION='12',
                    TARGET_BUILD_DIR=str(root/'products'), FULL_PRODUCT_NAME='OmobaBeta.app',
                    EXECUTABLE_PATH='OmobaBeta.app/client', DERIVED_FILE_DIR=str(root/'derived'),
                    DWARF_DSYM_FOLDER_PATH=str(root/'products'), DWARF_DSYM_FILE_NAME='OmobaBeta.app.dSYM',
                    OMOBA_CARGO_TARGET_DIR=str(root/'cache'), PRODUCT_BUNDLE_IDENTIFIER='space.ekza.omoba.beta',
                    SDKROOT='/selected/iphoneos.sdk')

    def test_worktrees_share_primary_cargo_cache_and_allow_explicit_cache(self):
        with patch.object(xb.subprocess, 'check_output', return_value='/repo/omoba/.git\n') as git:
            self.assertEqual(xb.cargo_cache({}), Path('/repo/omoba/target'))
            self.assertIn('--git-common-dir', git.call_args.args[0])
        with patch.object(xb.subprocess, 'check_output') as git:
            self.assertEqual(xb.cargo_cache({'CARGO_TARGET_DIR': '/cache/b'}), Path('/cache/b'))
            self.assertEqual(xb.cargo_cache({'CARGO_TARGET_DIR': '/cache/b',
                                           'OMOBA_CARGO_TARGET_DIR': '/cache/a'}), Path('/cache/a'))
            git.assert_not_called()

    @unittest.skipUnless(sys.platform == 'darwin' and shutil.which('xcodebuild'), 'requires Xcode')
    def test_xcode_resolves_both_schemes_despite_stale_local_settings(self):
        here = Path(xb.__file__).parent
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            shutil.copytree(here/'Omoba.xcodeproj', root/'Omoba.xcodeproj')
            shutil.copy2(here/'Omoba.xcconfig', root/'Omoba.xcconfig')
            for scheme, config, contour, bundle, product, icon in (
                ('Omoba', 'Debug', 'prod', 'space.ekza.omoba.beta', 'OmobaBeta.app', 'AppIcon'),
                ('Omoba', 'Release', 'prod', 'space.ekza.omoba.beta', 'OmobaBeta.app', 'AppIcon'),
                ('Omoba Dev', 'Debug Dev', 'dev', 'space.ekza.omoba.dev', 'OmobaDev.app', 'AppIconDev'),
                ('Omoba Dev', 'Release Dev', 'dev', 'space.ekza.omoba.dev', 'OmobaDev.app', 'AppIconDev'),
            ):
                with self.subTest(scheme=scheme, config=config):
                    stale = 'prod' if contour == 'dev' else 'dev'
                    (root/'Omoba.local.xcconfig').write_text(
                        f'OMOBA_CONTOUR = {stale}\nOMOBA_GAME_SERVER = 77.246.105.57:4000\n')
                    result = subprocess.run([
                        'xcodebuild', '-project', str(root/'Omoba.xcodeproj'),
                        '-scheme', scheme, '-configuration', config,
                        '-destination', 'generic/platform=iOS', '-showBuildSettings', '-json',
                    ], capture_output=True, text=True, timeout=60, check=True)
                    values = json.loads(result.stdout)[0]['buildSettings']
                    self.assertEqual(values['OMOBA_CONTOUR'], contour)
                    self.assertEqual(values['PRODUCT_BUNDLE_IDENTIFIER'], bundle)
                    self.assertEqual(values['FULL_PRODUCT_NAME'], product)
                    self.assertEqual(values['ASSETCATALOG_COMPILER_APPICON_NAME'], icon)
                    if contour == 'dev':
                        self.assertFalse(values.get('OMOBA_GAME_SERVER'))
                        self.assertEqual(xb.contour_build_settings(values)['OMOBA_DEFAULT_GAME_SERVER_ADDR'],
                                         'lobby.dev.omoba.io:4100')
            dev_scheme = ET.parse(root/'Omoba.xcodeproj/xcshareddata/xcschemes/Omoba Dev.xcscheme')
            self.assertEqual(dev_scheme.find('LaunchAction').get('buildConfiguration'), 'Debug Dev')
            self.assertEqual(dev_scheme.find('ArchiveAction').get('buildConfiguration'), 'Release Dev')

    def test_local_ekza_settings_apply_only_to_debug_and_private_host(self):
        import json
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root/'.ekza-lan').mkdir()
            path = root/'.ekza-lan/client.json'
            path.write_text(json.dumps({'development_host':'192.168.1.71','registry':'http://192.168.1.71:8018','game_server':'192.168.1.71:4030'}))
            self.assertEqual(xb.ekza_build_settings({'CONFIGURATION':'Debug'}, root)['EKZA_DEV_HTTP_HOST'], '192.168.1.71')
            self.assertEqual(xb.ekza_build_settings({'CONFIGURATION':'Debug'}, root)['OMOBA_DEFAULT_GAME_SERVER_ADDR'], '192.168.1.71:4030')
            self.assertNotIn('OMOBA_DEFAULT_GAME_SERVER_ADDR', xb.ekza_build_settings({'CONFIGURATION':'Debug','OMOBA_GAME_SERVER':'192.168.1.71:4010'}, root))
            self.assertEqual(xb.ekza_build_settings({'CONFIGURATION':'Release'}, root), {})
            self.assertEqual(xb.ekza_build_settings({'CONFIGURATION':'Debug', 'ACTION':'install'}, root), {})
            path.write_text(json.dumps({'development_host':'8.8.8.8','registry':'http://8.8.8.8'}))
            with self.assertRaises(ValueError): xb.ekza_build_settings({'CONFIGURATION':'Debug'}, root)

    def test_the_contour_picks_the_app_and_never_crosses_identities(self):
        self.assertEqual(xb.contour({'PRODUCT_BUNDLE_IDENTIFIER': 'space.ekza.omoba.beta'}), 'prod')
        self.assertEqual(xb.contour({'OMOBA_CONTOUR': ' ', 'PRODUCT_BUNDLE_IDENTIFIER': 'x.team.omoba'}), 'prod')
        self.assertEqual(xb.contour({'OMOBA_CONTOUR': 'dev', 'PRODUCT_BUNDLE_IDENTIFIER': 'space.ekza.omoba.dev'}), 'dev')
        for env in ({'OMOBA_CONTOUR': 'dev', 'PRODUCT_BUNDLE_IDENTIFIER': 'space.ekza.omoba.beta'},
                    {'OMOBA_CONTOUR': '', 'PRODUCT_BUNDLE_IDENTIFIER': 'space.ekza.omoba.dev'},
                    {'OMOBA_CONTOUR': 'staging', 'PRODUCT_BUNDLE_IDENTIFIER': 'space.ekza.omoba.dev'}):
            with self.subTest(env=env), self.assertRaises(ValueError):
                xb.contour(env)

    def test_the_dev_app_compiles_its_dev_profile_and_never_the_lan_rehearsal(self):
        import json
        dev = {'OMOBA_CONTOUR': 'dev', 'PRODUCT_BUNDLE_IDENTIFIER': 'space.ekza.omoba.dev', 'CONFIGURATION': 'Debug'}
        with self.assertRaisesRegex(ValueError, 'production'):
            xb.contour_build_settings({**dev, 'OMOBA_GAME_SERVER': '77.246.105.57:4000'})
        with self.assertRaisesRegex(ValueError, 'dev lobby'):
            xb.contour_build_settings({'OMOBA_CONTOUR': 'prod', 'OMOBA_GAME_SERVER': 'lobby.dev.omoba.io:4100'})
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root/'.ekza-lan').mkdir()
            (root/'.ekza-lan/client.json').write_text(json.dumps({
                'development_host': '192.168.1.71', 'registry': 'http://192.168.1.71:8018'}))
            self.assertEqual(xb.contour_build_settings(dev, root), {
                'OMOBA_DEFAULT_GAME_SERVER_ADDR': 'lobby.dev.omoba.io:4100',
                'OMOBA_DEFAULT_REGISTRY_URL': 'https://registry.dev.ekza.io',
                'OMOBA_DEFAULT_PASSPORT_URL': 'http://127.0.0.1:9/api/passport',
                'OMOBA_DEFAULT_CONFIG_DIR_SUFFIX': 'dev',
                'OMOBA_ACCOUNT_API_URL': 'https://account.dev.omoba.io',
                'OMOBA_PORTAL_ORIGIN': 'https://app.dev.omoba.io'})
            by_ip = xb.contour_build_settings({**dev, 'OMOBA_GAME_SERVER': '203.0.113.10:4100'}, root)
            self.assertEqual(by_ip['OMOBA_DEFAULT_GAME_SERVER_ADDR'], '203.0.113.10:4100')
            # The production app's Debug run still takes the LAN rehearsal.
            prod = {'PRODUCT_BUNDLE_IDENTIFIER': 'space.ekza.omoba.beta', 'CONFIGURATION': 'Debug'}
            self.assertEqual(xb.contour_build_settings(prod, root)['EKZA_DEV_HTTP_HOST'], '192.168.1.71')
            self.assertNotIn('OMOBA_DEFAULT_CONFIG_DIR_SUFFIX', xb.contour_build_settings(prod, root))

    def test_the_dev_app_is_named_omoba_dev_and_production_is_unchanged(self):
        prod = xb.app_info(xb.ROOT, {'minimum_os': '15.0.0'}, 'space.ekza.omoba.beta')
        dev = xb.app_info(xb.ROOT, {'minimum_os': '15.0.0'}, 'space.ekza.omoba.dev', 'OMOBA Dev')
        self.assertEqual((prod['CFBundleIdentifier'], prod['CFBundleDisplayName'], prod['CFBundleName']),
                         ('space.ekza.omoba.beta', 'Omoba Beta', 'Omoba Beta'))
        self.assertEqual((dev['CFBundleIdentifier'], dev['CFBundleDisplayName'], dev['CFBundleName']),
                         ('space.ekza.omoba.dev', 'OMOBA Dev', 'OMOBA Dev'))
        self.assertEqual({k: v for k, v in dev.items() if not k.startswith('CFBundle')},
                         {k: v for k, v in prod.items() if not k.startswith('CFBundle')})

    def test_the_xcconfig_maps_each_contour_to_its_bundle_id_and_icon(self):
        here = Path(xb.__file__).parent
        config = {}
        for line in (here/'Omoba.xcconfig').read_text().splitlines():
            if '=' in line and not line.strip().startswith(('//', '#')):
                key, value = line.split('=', 1)
                config[key.strip()] = value.strip()

        def expand(value, contour):
            values = {**config, 'OMOBA_CONTOUR': contour}
            while '$(' in value:
                start = value.rindex('$(')
                end = value.index(')', start)
                value = value[:start] + values.get(value[start + 2:end], '') + value[end + 1:]
            return value

        self.assertEqual(config['OMOBA_CONTOUR'], '')
        for contour, bundle, icon in (('', 'space.ekza.omoba.beta', 'AppIcon'),
                                      ('prod', 'space.ekza.omoba.beta', 'AppIcon'),
                                      ('dev', 'space.ekza.omoba.dev', 'AppIconDev')):
            self.assertEqual(expand(config['PRODUCT_BUNDLE_IDENTIFIER'], contour), bundle)
            self.assertEqual(expand(config['ASSETCATALOG_COMPILER_APPICON_NAME'], contour), icon)
            icon_set = here/'Assets.xcassets'/f'{icon}.appiconset'
            contents = __import__('json').loads((icon_set/'Contents.json').read_text())
            self.assertTrue((icon_set/contents['images'][0]['filename']).is_file())

    def test_a_dev_build_stages_the_dev_app_from_a_clean_environment(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            env = {**self.env(root), 'OMOBA_CONTOUR': 'dev', 'PRODUCT_BUNDLE_IDENTIFIER': 'space.ekza.omoba.dev',
                   'CONFIGURATION': 'Release'}
            binary = root/'cache/aarch64-apple-ios/debug/client'
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b'rust')
            dsym = Path(str(binary)+'.dSYM')
            (dsym/'Contents/Resources/DWARF').mkdir(parents=True)
            (dsym/'Contents/Resources/DWARF/client').write_bytes(b'symbols')
            shell = {'OMOBA_DEFAULT_CONFIG_DIR_SUFFIX': 'stale', 'OMOBA_DEFAULT_REGISTRY_URL': 'https://registry.ekza.io',
                     'OMOBA_DEFAULT_GAME_SERVER_ADDR': 'lobby.dev.omoba.io:4100',
                     'OMOBA_ACCOUNT_API_URL': 'https://dev.omoba.invalid',
                     'OMOBA_PORTAL_ORIGIN': 'https://app.dev.omoba.io',
                     'EKZA_DEV_HTTP_HOST': '192.168.1.71'}
            with patch.dict(os.environ, shell), \
                 patch.object(xb, 'source_identity', return_value={'revision': 'r', 'tracked_diff': ''}), \
                 patch.object(xb.subprocess, 'run') as cargo, \
                 patch.object(xb, 'inspect_macho', return_value={'minimum_os': '15.0.0', 'sdk': '26.0'}), \
                 patch.object(xb, 'validate_dsym', return_value={'uuid_match_verified': True}), \
                 patch.object(xb, 'tracked_assets', return_value=[]), \
                 patch.object(xb, 'collect_legal_notices', return_value=[]), \
                 patch.object(xb, 'copy_legal_notices', side_effect=lambda _, dest: dest.mkdir(parents=True)):
                xb.build(env)
            cargo_env = cargo.call_args.kwargs['env']
            self.assertEqual(cargo_env['OMOBA_DEFAULT_CONFIG_DIR_SUFFIX'], 'dev')
            self.assertEqual(cargo_env['OMOBA_DEFAULT_REGISTRY_URL'], 'https://registry.dev.ekza.io')
            self.assertEqual(cargo_env['OMOBA_DEFAULT_GAME_SERVER_ADDR'], 'lobby.dev.omoba.io:4100')
            self.assertEqual(cargo_env['OMOBA_ACCOUNT_API_URL'], 'https://account.dev.omoba.io')
            self.assertEqual(cargo_env['OMOBA_PORTAL_ORIGIN'], 'https://app.dev.omoba.io')
            self.assertNotIn('EKZA_DEV_HTTP_HOST', cargo_env)
            info = plistlib.loads((root/'derived/Omoba-Info.plist').read_bytes())
            self.assertEqual((info['CFBundleIdentifier'], info['CFBundleDisplayName']),
                             ('space.ekza.omoba.dev', 'OMOBA Dev'))
            report = __import__('json').loads((root/'products/OmobaBeta.app/assets/legal/XCODE-BUILD.json').read_text())
            self.assertEqual(report['contour'], 'dev')
            self.assertEqual(report['bundle_id'], 'space.ekza.omoba.dev')
            self.assertEqual(report['source_revision'], 'r')
            self.assertFalse(report['source_dirty'])
            self.assertEqual(report['compiled_endpoint_overrides']['OMOBA_DEFAULT_GAME_SERVER_ADDR'],
                             'lobby.dev.omoba.io:4100')
            # The same shell never leaks into a production archive.
            with patch.dict(os.environ, shell), \
                 patch.object(xb, 'source_identity', return_value={'revision': 'r', 'tracked_diff': ''}), \
                 patch.object(xb.subprocess, 'run') as cargo, \
                 patch.object(xb, 'inspect_macho', return_value={'minimum_os': '15.0.0', 'sdk': '26.0'}), \
                 patch.object(xb, 'validate_dsym', return_value={'uuid_match_verified': True}), \
                 patch.object(xb, 'tracked_assets', return_value=[]), \
                 patch.object(xb, 'collect_legal_notices', return_value=[]), \
                 patch.object(xb, 'copy_legal_notices', side_effect=lambda _, dest: dest.mkdir(parents=True)):
                xb.build({**self.env(root), 'CONFIGURATION': 'Release'})
            prod_env = cargo.call_args.kwargs['env']
            for key in shell:
                self.assertNotIn(key, prod_env)
            info = plistlib.loads((root/'derived/Omoba-Info.plist').read_bytes())
            self.assertEqual(info['CFBundleDisplayName'], 'Omoba Beta')

    def test_rejects_simulator_and_non_arm64(self):
        for field, value in [('PLATFORM_NAME','iphonesimulator'), ('ARCHS','arm64 x86_64')]:
            with self.subTest(field=field):
                env=self.env(Path('/tmp'));env[field]=value
                with self.assertRaises(ValueError): xb.settings(env)

    def test_rejects_invalid_build_profile_and_executable_paths(self):
        for field,value in [('CURRENT_PROJECT_VERSION','0'),('CURRENT_PROJECT_VERSION','12-beta'),
                            ('OMOBA_CARGO_PROFILE','unknown'),('EXECUTABLE_PATH','../client'),
                            ('FULL_PRODUCT_NAME','unrelated')]:
            with self.subTest(field=field,value=value):
                env=self.env(Path('/tmp'));env[field]=value
                with self.assertRaises(ValueError): xb.settings(env)

    def test_scheme_archives_real_application(self):
        scheme=ET.parse(Path(xb.__file__).parent/'Omoba.xcodeproj/xcshareddata/xcschemes/Omoba.xcscheme')
        self.assertEqual(scheme.find('ArchiveAction').get('buildConfiguration'),'Release')
        ref=scheme.find('BuildAction/BuildActionEntries/BuildActionEntry')
        self.assertEqual(ref.get('buildForArchiving'),'YES')
        self.assertEqual(ref.find('BuildableReference').get('BuildableName'),'OmobaBeta.app')

    def test_incremental_build_refreshes_assets_and_preserves_xcode_owned_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);env=self.env(root)
            binary=root/'cache/aarch64-apple-ios/debug/client';binary.parent.mkdir(parents=True);binary.write_bytes(b'current-rust')
            os.utime(binary, ns=(1_000_000_000, 1_000_000_000))
            dsym=Path(str(binary)+'.dSYM');(dsym/'Contents/Resources/DWARF').mkdir(parents=True);(dsym/'Contents/Resources/DWARF/client').write_bytes(b'symbols')
            bundle=root/'products/OmobaBeta.app';(bundle/'assets').mkdir(parents=True)
            (bundle/'assets/deleted.glb').write_bytes(b'obsolete')
            (bundle/'Info.plist').write_bytes(b'xcode-owned');(bundle/'Assets.car').write_bytes(b'compiled-icon')
            model=root/'hero.glb';model.write_bytes(b'current-model')
            source={'revision':'test-revision','tracked_diff':''}
            info=xb.app_info(xb.ROOT, {'minimum_os':'15.0.0'}, 'space.ekza.omoba.beta')
            with patch.object(xb,'source_identity',return_value=source), patch.object(xb.subprocess,'run') as cargo, \
                 patch.object(xb,'inspect_macho',return_value={'sdk':'26.0'}), \
                 patch.object(xb,'validate_dsym',return_value={'uuid_match_verified':True}) as validate, \
                 patch.object(xb,'tracked_assets',return_value=[(model,Path('hero.glb'))]), \
                 patch.object(xb,'collect_legal_notices',return_value=[]), \
                 patch.object(xb,'copy_legal_notices',side_effect=lambda _,dest:dest.mkdir(parents=True)), \
                 patch.object(xb,'app_info',return_value=info):
                for _ in range(2):
                    # Simulate the prior signed output while Cargo reuses an old binary.
                    staged = bundle/'client'
                    staged.write_bytes(b'previously-signed')
                    previous_mtime = staged.stat().st_mtime_ns
                    xb.build(env)
                    self.assertGreaterEqual(staged.stat().st_mtime_ns, previous_mtime)
                    self.assertGreater(staged.stat().st_mtime_ns, binary.stat().st_mtime_ns)
            self.assertFalse((bundle/'assets/deleted.glb').exists())
            self.assertEqual((bundle/'assets/hero.glb').read_bytes(),b'current-model')
            self.assertEqual((bundle/'client').read_bytes(),binary.read_bytes())
            self.assertEqual((bundle/'Info.plist').read_bytes(),b'xcode-owned')
            self.assertEqual((bundle/'Assets.car').read_bytes(),b'compiled-icon')
            self.assertEqual(plistlib.loads((root/'derived/Omoba-Info.plist').read_bytes())['CFBundleVersion'],'12')
            generated = plistlib.loads((root/'derived/Omoba-Info.plist').read_bytes())
            self.assertIs(generated['ITSAppUsesNonExemptEncryption'], False)
            self.assertEqual(validate.call_count,4)
            command=cargo.call_args.args[0];self.assertIn('--locked',command)
            self.assertEqual(cargo.call_args.kwargs['env']['SDKROOT'],env['SDKROOT'])
            self.assertEqual(cargo.call_args.kwargs['env']['CARGO_PROFILE_DEV_SPLIT_DEBUGINFO'],'packed')

    def test_source_change_fails_before_touching_product(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);env=self.env(root)
            with patch.object(xb,'source_identity',side_effect=[{'revision':'a'},{'revision':'b'}]), \
                 patch.object(xb.subprocess,'run'),patch.object(xb,'inspect_macho',return_value={}), \
                 patch.object(xb,'validate_dsym',return_value={}):
                with self.assertRaisesRegex(ValueError,'Source changed'): xb.build(env)
            self.assertFalse((root/'products').exists())

    def test_common_combat_edit_invalidates_build_without_a_new_commit(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root/'common/src/lib.rs'
            source.parent.mkdir(parents=True)
            source.write_text('pub const DAMAGE: u32 = 10;\n')
            subprocess.run(['git', 'init', '-q'], cwd=root, check=True)
            subprocess.run(['git', 'add', 'common'], cwd=root, check=True)
            subprocess.run(['git', '-c', 'user.name=Build Test',
                            '-c', 'user.email=build-test@example.invalid',
                            '-c', 'commit.gpgsign=false', 'commit', '-qm', 'fixture'],
                           cwd=root, check=True)
            with patch.object(xb, 'ROOT', root):
                before = xb.source_identity()
                source.write_text('pub const DAMAGE: u32 = 20;\n')
                after = xb.source_identity()
            self.assertEqual(before['revision'], after['revision'])
            self.assertNotEqual(before['tracked_diff'], after['tracked_diff'])
            with patch.object(xb, 'source_identity', side_effect=[before, after]), \
                 patch.object(xb.subprocess, 'run'), \
                 patch.object(xb, 'inspect_macho', return_value={}), \
                 patch.object(xb, 'validate_dsym', return_value={}):
                with self.assertRaisesRegex(ValueError, 'Source changed'):
                    xb.build(self.env(root))
            self.assertFalse((root/'products').exists())


if __name__=='__main__': unittest.main()
