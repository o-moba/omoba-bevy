from datetime import datetime, timedelta, timezone
from pathlib import Path
import subprocess
import unittest
from unittest.mock import Mock

from install_device import choose_device, enrolled_devices, execute


def phone(identifier='device-1', udid='phone-1', state='connected'):
    return {'identifier':identifier, 'hardwareProperties':{'platform':'iOS','reality':'physical','deviceType':'iPhone','udid':udid},
            'connectionProperties':{'tunnelState':state}, 'deviceProperties':{'developerModeStatus':'enabled'}}


class InstallerTests(unittest.TestCase):
    def test_profile_expiration_and_development_type(self):
        now = datetime.now(timezone.utc)
        profile = {'ExpirationDate':now+timedelta(days=1),'ProvisionedDevices':['PHONE-1'],'Entitlements':{'get-task-allow':True}}
        self.assertEqual(enrolled_devices(profile,now),{'phone-1'})
        profile['ExpirationDate'] = now
        with self.assertRaises(ValueError): enrolled_devices(profile,now)
        profile['ExpirationDate'] = now+timedelta(days=1)
        profile['Entitlements'] = {}
        with self.assertRaises(ValueError): enrolled_devices(profile,now)

    def test_only_enrolled_physical_iphone_is_selected(self):
        wrong = phone('other','other')
        simulator = phone('sim'); simulator['hardwareProperties']['reality']='simulated'
        self.assertEqual(choose_device([wrong,simulator,phone()],{'phone-1'}),'device-1')

    def test_unavailable_and_developer_mode_disabled_fail(self):
        with self.assertRaisesRegex(ValueError,'USB'): choose_device([phone(state='unavailable')],{'phone-1'})
        disabled = phone(); disabled['deviceProperties']['developerModeStatus']='disabled'
        with self.assertRaisesRegex(ValueError,'Developer Mode'): choose_device([disabled],{'phone-1'})

    def test_multiple_requires_explicit_choice(self):
        phones = [phone(),phone('device-2','phone-2')]
        with self.assertRaisesRegex(ValueError,'Several'): choose_device(phones,{'phone-1','phone-2'})
        self.assertEqual(choose_device(phones,{'phone-1','phone-2'},'PHONE-2'),'device-2')

    def test_check_performs_no_install_or_launch(self):
        run = Mock(); execute(Path('OmobaBeta.app'),'device','space.ekza.omoba.beta',check=True,run=run)
        run.assert_not_called()

    def test_install_failure_never_launches_or_uninstalls(self):
        run = Mock(side_effect=subprocess.CalledProcessError(1,['install']))
        with self.assertRaises(subprocess.CalledProcessError): execute(Path('OmobaBeta.app'),'device','space.ekza.omoba.beta',run=run)
        self.assertEqual(run.call_count,1)
        self.assertIn('install',run.call_args.args[0])

    def test_success_installs_then_launches_without_uninstall(self):
        run = Mock(); execute(Path('OmobaBeta.app'),'device','space.ekza.omoba.beta',run=run)
        self.assertEqual(run.call_count,2)
        self.assertIn('install',run.call_args_list[0].args[0])
        self.assertIn('launch',run.call_args_list[1].args[0])


if __name__ == '__main__': unittest.main()
