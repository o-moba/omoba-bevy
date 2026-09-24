import unittest
from pathlib import Path
import combat_test


class CombatTestLauncherTests(unittest.TestCase):
    def test_modes_are_exclusive_and_connection_address_is_validated(self):
        with self.assertRaises(SystemExit):
            combat_test.arguments(['--connect', '127.0.0.1:4040', '--server-only'])
        with self.assertRaises(SystemExit):
            combat_test.arguments(['--bind', '127.0.0.1:0'])
        args = combat_test.arguments(['--hero', 'mage', '--preset', 'dps'])
        self.assertEqual(args.hero, 'mage')
        self.assertEqual(args.bind, '127.0.0.1:4040')

    def test_hero_choices_are_the_catalog_classes(self):
        self.assertEqual(combat_test.arguments(['--hero', 'warden']).hero, 'warden')
        with self.assertRaises(SystemExit):
            combat_test.arguments(['--hero', 'necromancer'])

    def test_live_service_and_qa_settings_cannot_leak_into_sandbox(self):
        parent = {'PATH': '/bin', 'HOME': '/test', 'OMOBA_DATABASE_URL': 'private',
                  'OMOBA_SERVER_ROLE': 'worker', 'OMOBA_AUTOJOIN': 'old',
                  'OMOBA_VISUAL_QA_DIR': 'old', 'EKZA_REGISTRY_URL': 'old',
                  'GAME_SERVER_ADDR': 'public:4000'}
        env = combat_test.session_environment(parent, Path('/tmp/session'), '127.0.0.1:4040', '127.0.0.1:4040')
        self.assertEqual(env['OMOBA_MATCH_MODE'], 'dev')
        self.assertEqual(env['OMOBA_COMBAT_SANDBOX'], '1')
        self.assertEqual(env['GAME_SERVER_ADDR'], '127.0.0.1:4040')
        self.assertEqual(env['HOME'], '/test')
        for key in ('OMOBA_DATABASE_URL', 'OMOBA_SERVER_ROLE', 'OMOBA_AUTOJOIN', 'OMOBA_VISUAL_QA_DIR', 'EKZA_REGISTRY_URL'):
            self.assertNotIn(key, env)
        self.assertEqual(parent['OMOBA_DATABASE_URL'], 'private')


if __name__ == '__main__':
    unittest.main()
