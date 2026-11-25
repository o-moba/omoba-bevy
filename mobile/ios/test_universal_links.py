import plistlib
from pathlib import Path
import unittest
import shutil
import sys
import tempfile
from unittest.mock import patch
from build_device import ROOT, compile_delegate_nib, require_link_delegate
from universal_links import DOMAIN_KEY, associated_domains, require_associated_domains


class UniversalLinkTests(unittest.TestCase):
    def test_production_and_dev_have_different_domains(self):
        self.assertEqual(associated_domains('space.ekza.omoba.beta'), ['applinks:ios.omoba.ru'])
        self.assertEqual(associated_domains('space.ekza.omoba.dev'), ['applinks:ios.dev.omoba.io'])
        self.assertEqual(associated_domains('unknown.app'), [])

    def test_old_profile_is_rejected_instead_of_silently_losing_the_capability(self):
        for value in (None, [], 'applinks:ios.omoba.ru', ['applinks:ios.dev.omoba.io']):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, 'Associated Domains'):
                require_associated_domains({'Entitlements': {DOMAIN_KEY: value}}, ['applinks:ios.omoba.ru'])

    def test_authorized_profile_grants_only_requested_domains(self):
        requested = ['applinks:ios.omoba.ru']
        for allowed in (['*'], requested, [*requested, 'applinks:other.example']):
            self.assertEqual(require_associated_domains({'Entitlements': {DOMAIN_KEY: allowed}}, requested), {DOMAIN_KEY: requested})
        self.assertEqual(require_associated_domains({}, []), {})

    def test_old_prebuilt_executable_cannot_be_packaged_with_a_missing_nib_class(self):
        for symbols in (b'', b'_omoba_universal_links_prepare\n', b'_OBJC_CLASS_$_OmobaLinkDelegate\n'):
            with patch('build_device.output', return_value=symbols), self.assertRaisesRegex(ValueError, 'Rebuild'):
                require_link_delegate(Path('synthetic-old-client'))
        with patch('build_device.output', return_value=b'_omoba_universal_links_prepare\n_OBJC_CLASS_$_OmobaLinkDelegate\n'):
            require_link_delegate(Path('synthetic-current-client'))

    def test_xcode_and_info_load_the_custom_delegate_from_the_main_nib(self):
        here = Path(__file__).parent
        self.assertEqual(plistlib.loads((here/'Info.plist').read_bytes())['NSMainNibFile'], 'OmobaAppDelegate')
        config = (here/'Omoba.xcconfig').read_text()
        self.assertIn('CODE_SIGN_ENTITLEMENTS = $(OMOBA_ENTITLEMENTS_$(OMOBA_CONTOUR))', config)

    @unittest.skipUnless(sys.platform == 'darwin' and shutil.which('xcrun'), 'requires Xcode')
    def test_real_interface_builder_compiles_the_delegate_nib(self):
        with tempfile.TemporaryDirectory() as directory:
            bundle = Path(directory)
            compile_delegate_nib(ROOT, bundle)
            self.assertGreater((bundle/'OmobaAppDelegate.nib').stat().st_size, 0)


if __name__ == '__main__':
    unittest.main()
