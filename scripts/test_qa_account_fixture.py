"""Capture fixtures must restore routing without bypassing first-launch checks."""
import json
import os
from pathlib import Path
import stat
import tempfile
import unittest

from capture_showcase import base_env
from qa_account_fixture import prepare_capture_account


class CaptureAccountFixture(unittest.TestCase):
    def test_package_smoke_seeds_its_flat_scratch_config(self):
        with tempfile.TemporaryDirectory(prefix="omoba-region-fixture-") as scratch:
            self.assertTrue(prepare_capture_account(scratch, in_root=True))
            self.assertEqual(json.loads((Path(scratch) / "account_region.json").read_text())["selected"], "global")
            self.assertFalse((Path(scratch) / "config").exists())

    def test_showcase_environment_restores_a_synthetic_returning_global_player(self):
        with tempfile.TemporaryDirectory(prefix="omoba-region-fixture-") as scratch:
            env = base_env(dict(size=(852, 393), profile="phone"), "/assets", scratch, "/output")
            path = Path(env["OMOBA_CLIENT_CONFIG_DIR"]) / "account_region.json"
            self.assertEqual(json.loads(path.read_text()),
                             dict(schema=1, selected="global", pending=None, account=None))
            if os.name == "posix":
                self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
            self.assertEqual(sorted(p.name for p in path.parent.iterdir()), [path.name])

    def test_existing_region_and_pending_switch_are_never_replaced(self):
        with tempfile.TemporaryDirectory(prefix="omoba-region-fixture-") as scratch:
            folder = Path(scratch) / "config"
            folder.mkdir()
            path = folder / "account_region.json"
            record = b'{"schema":1,"selected":"ru","pending":"global","account":null}'
            path.write_bytes(record)
            self.assertFalse(prepare_capture_account(scratch))
            self.assertEqual(path.read_bytes(), record)

    def test_a_missing_scratch_root_is_not_created(self):
        with tempfile.TemporaryDirectory(prefix="omoba-region-fixture-") as scratch:
            missing = Path(scratch) / "missing"
            with self.assertRaises(FileNotFoundError):
                prepare_capture_account(missing)
            self.assertFalse(missing.exists())
