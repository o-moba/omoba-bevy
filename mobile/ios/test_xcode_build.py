import os
from pathlib import Path
import plistlib
import tempfile
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ET
import xcode_build as xb


class XcodeBridgeTests(unittest.TestCase):
    def env(self, root):
        return dict(PLATFORM_NAME='iphoneos', ARCHS='arm64', CURRENT_PROJECT_VERSION='12',
                    TARGET_BUILD_DIR=str(root/'products'), FULL_PRODUCT_NAME='OmobaBeta.app',
                    EXECUTABLE_PATH='OmobaBeta.app/client', DERIVED_FILE_DIR=str(root/'derived'),
                    DWARF_DSYM_FOLDER_PATH=str(root/'products'), DWARF_DSYM_FILE_NAME='OmobaBeta.app.dSYM',
                    OMOBA_CARGO_TARGET_DIR=str(root/'cache'), PRODUCT_BUNDLE_IDENTIFIER='space.ekza.omoba.beta',
                    SDKROOT='/selected/iphoneos.sdk')

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
            info={'CFBundleShortVersionString':'0.22.0','CFBundleExecutable':'client'}
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


if __name__=='__main__': unittest.main()
