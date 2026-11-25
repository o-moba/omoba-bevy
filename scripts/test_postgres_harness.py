import json
import os
from pathlib import Path
import tempfile
import unittest

import postgres_harness as gate


class PostgresHarnessTests(unittest.TestCase):
    def test_explicit_database_selection_cannot_fall_back_from_unsafe_url(self):
        safe = "postgres://postgres@127.0.0.1:5432/omoba_test"
        self.assertEqual(gate.database_url({"OMOBA_TEST_DATABASE_URL": safe}), safe)
        for unsafe in ("", "postgres://remote/omoba_test",
                       safe + "?host=remote", safe + "#ignored",
                       "postgres://localhost:99999/omoba_test",
                       "postgres://localhost:0/omoba_test",
                       "postgres://local\nhost/omoba_test",
                       "postgres://localhost/production"):
            with self.subTest(url=unsafe), self.assertRaises(ValueError):
                gate.database_url({"OMOBA_TEST_DATABASE_URL": safe,
                                   "OMOBA_HARNESS_DATABASE_URL": unsafe})
        self.assertTrue(gate.isolated_database_url("postgresql://u:p@[::1]:5432/omoba_test"))

    @staticmethod
    def accounts():
        return [{"method": method, "profile_id": f"{i:064x}", "seed": "a" * 64}
                for i, method in enumerate(sorted(gate.METHODS), 1)]

    def test_private_fixture_validation_never_returns_seeds(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fixture.json"
            path.write_text(json.dumps(self.accounts()))
            path.chmod(0o600)
            self.assertEqual(gate.validate_fixture(path), sorted(gate.METHODS))
            path.chmod(0o644)
            with self.assertRaisesRegex(ValueError, "private regular file"):
                gate.validate_fixture(path)
            path.chmod(0o600)
            link = Path(directory) / "link.json"
            link.symlink_to(path)
            with self.assertRaisesRegex(ValueError, "private regular file"):
                gate.validate_fixture(link)

    def test_fixture_requires_three_distinct_enrolled_profiles(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fixture.json"
            for mutation in ("missing", "duplicate", "invalid_seed"):
                accounts = self.accounts()
                if mutation == "missing":
                    accounts.pop()
                elif mutation == "duplicate":
                    accounts[1]["profile_id"] = accounts[0]["profile_id"]
                else:
                    accounts[0]["seed"] = "not-a-signing-key"
                path.write_text(json.dumps(accounts))
                path.chmod(0o600)
                with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                    gate.validate_fixture(path)

    def test_empty_filtered_or_ignored_rust_tests_cannot_pass_the_gate(self):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "test.log"
            good = "test result: ok. 1 passed; 0 failed; 0 ignored; 12 filtered out; finished in 0.1s\n"
            log.write_text(good)
            gate.assert_one_test(log)
            for bad in ("", good.replace("1 passed", "0 passed"),
                        good.replace("0 ignored", "1 ignored"), good + good):
                log.write_text(bad)
                with self.subTest(log=bad), self.assertRaises(AssertionError):
                    gate.assert_one_test(log)

    @unittest.skipUnless(os.name == "posix", "process identity uses POSIX ps")
    def test_cleanup_does_not_signal_a_reused_or_unowned_process(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "test.log"
            log.write_text("HARNESS_OWNED_GROUP 123 unrelated birth\n")
            with patch.object(gate, "process_birth", return_value="new process"), \
                    patch.object(gate.os, "killpg") as kill:
                gate.stop_registration_groups(log)
                kill.assert_not_called()


if __name__ == "__main__":
    unittest.main()
