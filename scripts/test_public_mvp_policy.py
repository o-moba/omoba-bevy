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
