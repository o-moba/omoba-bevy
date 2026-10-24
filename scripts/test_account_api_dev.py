"""Isolated DEV host/credential/activation regressions; no SSH or Docker needed."""
import hashlib
import importlib.util
import json
import sys
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('account_api_dev', Path(__file__).resolve().parents[1] / 'ops/vps/dev/account_api_dev.py')
api = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(api)
REV = 'a' * 40

class HostGuard(unittest.TestCase):
    def test_guard_refuses_wrong_host_before_database_access(self):
        with patch.object(api.os, 'geteuid', return_value=0), patch.object(api, 'run', return_value='77.246.105.57'), patch.object(api, 'sql') as sql:
            with self.assertRaisesRegex(RuntimeError, 'exact dev VPS'):
                api.guard()
            sql.assert_not_called()

    def test_production_marker_is_refused(self):
        with patch.object(api.os, 'geteuid', return_value=0), patch.object(api, 'run', return_value='146.103.102.18'), patch.object(Path, 'exists', return_value=True):
            with self.assertRaisesRegex(RuntimeError, 'production marker'):
                api.guard()

    def test_database_exposure_and_role_privileges(self):
        good_ports = {'5432/tcp': [{'HostIp': '127.0.0.1', 'HostPort': '54322'}]}
        cases = [
            ({'5432/tcp': [{'HostIp': '0.0.0.0', 'HostPort': '54322'}]}, [], 'database exposure'),
            (good_ports, ['omoba_dev_portal:true:false:false:false'], 'role privileges'),
            (good_ports, ['omoba_dev_portal:false:false:false:false', '1'], 'inherited'),
            (good_ports, ['omoba_dev_portal:false:false:false:false', '0'], None),
        ]
        for ports, results, error in cases:
            with self.subTest(error=error), patch.object(api.os, 'geteuid', return_value=0), patch.object(Path, 'exists', return_value=False), patch.object(api, 'run', side_effect=['146.103.102.18', 'true', json.dumps(ports)]), patch.object(api, 'sql', side_effect=results):
                if error:
                    with self.assertRaisesRegex(RuntimeError, error):
                        api.guard()
                else:
                    api.guard()

class Activation(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / 'current').mkdir()
        (self.root / 'current/omoba-account-api').write_bytes(b'candidate')
        self.build = {'contour': 'dev', 'revision': REV, 'account_api_sha256': hashlib.sha256(b'candidate').hexdigest()}
        self.root_patch = patch.object(api, 'ROOT', self.root)
        self.root_patch.start()
        self.addCleanup(self.root_patch.stop)

    def save(self):
        (self.root / 'current/BUILD.json').write_text(json.dumps(self.build))

    def test_candidate_revision_contour_and_digest(self):
        for field, value, error in [('contour', 'prod', 'does not match'), ('revision', 'b'*40, 'does not match'), ('account_api_sha256', '0'*64, 'hash mismatch')]:
            with self.subTest(field=field), patch.object(api, 'sql') as sql:
                previous = self.build[field]
                self.build[field] = value
                self.save()
                with self.assertRaisesRegex(RuntimeError, error):
                    api.validate_activation(REV)
                sql.assert_not_called()
                self.build[field] = previous

    def test_schema_guard(self):
        self.save()
        for old in ('1\n2\n3\n4\n5', '1\n2\n3\n4\n5\n6'):
            with patch.object(api, 'sql', return_value=old):
                with self.assertRaisesRegex(RuntimeError, 'migration7'):
                    api.validate_activation(REV)
        with patch.object(api, 'sql', return_value='1\n2\n3\n4\n5\n6\n7'):
            api.validate_activation(REV)

    def test_bad_activation_cannot_write_or_enable_login(self):
        with patch.object(sys, 'argv', ['tool', '--apply', '--backup', '/opt/ekza-dev/backups/20261016T025301Z', '--activate-revision', 'bad']), patch.object(api, 'guard'), patch.object(api, 'secure', return_value='{"backup":true}'), patch.object(Path, 'is_file', return_value=True), patch.object(Path, 'stat', return_value=type('Info', (), {'st_size': 10})()), patch.object(api, 'atomic') as atomic, patch.object(api, 'sql') as sql:
            with self.assertRaisesRegex(RuntimeError, 'full revision'):
                api.main()
            atomic.assert_not_called()
            sql.assert_not_called()

class Credentials(unittest.TestCase):
    def values(self):
        return {'OMOBA_DATABASE_URL': 'postgres://omoba_dev_portal:' + 'a'*64 + '@127.0.0.1:54322/omoba_dev', 'OMOBA_PORTAL_SECRET': 'b'*64, 'OMOBA_PORTAL_ORIGIN': api.PORTAL, 'OMOBA_ACCOUNT_BIND': '127.0.0.1:40550', 'OMOBA_TRUST_LOOPBACK_PROXY': '1'}

    def test_existing_values_reused_without_rotation(self):
        values = self.values()
        with patch.object(Path, 'exists', return_value=True), patch.object(api, 'secure', return_value=''.join(k+'='+v+'\n' for k,v in values.items())), patch.object(api.secrets, 'token_hex') as random:
            self.assertEqual(api.env_values(), values)
            random.assert_not_called()

    def test_drift_refused(self):
        for field, value in [('OMOBA_DATABASE_URL', 'postgres://omoba_dev_owner:password@127.0.0.1:54322/omoba_dev'), ('OMOBA_PORTAL_ORIGIN', 'https://app.omoba.io'), ('OMOBA_ACCOUNT_BIND', '0.0.0.0:40550'), ('OMOBA_PORTAL_SECRET', 'short')]:
            values = self.values()
            values[field] = value
            with self.subTest(field=field), patch.object(Path, 'exists', return_value=True), patch.object(api, 'secure', return_value=''.join(k+'='+v+'\n' for k,v in values.items())):
                with self.assertRaisesRegex(RuntimeError, 'env drift'):
                    api.env_values()

    def test_secure_refuses_symlink_and_world_readable_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'key'
            path.write_text('private')
            path.chmod(0o644)
            with self.assertRaisesRegex(RuntimeError, 'insecure'):
                api.secure(path)
            link = Path(tmp) / 'link'
            link.symlink_to(path)
            with self.assertRaisesRegex(RuntimeError, 'insecure'):
                api.secure(link)

    def test_command_errors_do_not_disclose_private_stderr(self):
        result = type('Result', (), {'returncode': 1, 'stderr': 'private password', 'stdout': ''})()
        with patch.object(api.subprocess, 'run', return_value=result):
            with self.assertRaisesRegex(RuntimeError, '^command failed: docker$'):
                api.run(['docker', 'exec'], 'private SQL')

class Ingress(unittest.TestCase):
    def test_private_header_stripped_and_ip_replaced(self):
        config = api.fragment('a'*64)
        self.assertIn('header X-Omoba-Portal-Key ' + 'a'*64, config)
        self.assertEqual(config.count('header_up -X-Omoba-Portal-Key'), 2)
        self.assertEqual(config.count('header_up X-Real-IP {remote_host}'), 2)
        self.assertIn('respond "Forbidden" 403', config)
        self.assertIn('path /v1/*', config)
        native = config.split('@native {', 1)[1].split('}', 1)[0]
        self.assertIn('method POST', native)
        self.assertNotIn('/v1/auth/pairings/create', native)
        self.assertNotIn('/v1/supporter/status', native)
        self.assertNotIn('*', native)
        self.assertNotIn('77.246.105.57', config)
        self.assertIn('max_size 64KB', config)

if __name__ == '__main__':
    unittest.main()
