"""Execute the live probe's real consent policy without sockets or a database."""
from pathlib import Path
import shutil
import subprocess
import unittest


class PublicQueuePolicyTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("node"), "public protocol probe requires Node")
    def test_ready_consent_is_correlated_and_cannot_starve_search_refresh(self):
        script = Path(__file__).with_name("test_public_mvp.mjs")
        result = subprocess.run(["node", str(script), "--self-test-queue-policy"],
                                capture_output=True, text=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("queue_policy: PASS (5", result.stdout)

    @unittest.skipUnless(shutil.which("node"), "public protocol probe requires Node")
    def test_draft_follows_human_locks_without_reserving_bot_classes(self):
        script = Path(__file__).with_name("test_public_mvp.mjs")
        result = subprocess.run(["node", str(script), "--self-test-draft-policy"],
                                capture_output=True, text=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("draft_policy: PASS (6", result.stdout)

    @unittest.skipUnless(shutil.which("node"), "public protocol probe requires Node")
    def test_queue_only_requires_an_explicit_isolated_fixture_database(self):
        script = Path(__file__).with_name("test_public_mvp.mjs")
        result = subprocess.run(["node", str(script), "--self-test-queue-only-policy"],
                                capture_output=True, text=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("queue_only_policy: PASS (7", result.stdout)

    @unittest.skipUnless(shutil.which("node"), "transport capacity probe requires Node")
    def test_transport_boundary_requires_local_explicit_bounds_and_preserves_u64_epochs(self):
        script = Path(__file__).with_name("test_public_transport_capacity.mjs")
        result = subprocess.run(["node", str(script), "--self-test-policy"],
                                capture_output=True, text=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("transport_capacity_policy: PASS", result.stdout)
