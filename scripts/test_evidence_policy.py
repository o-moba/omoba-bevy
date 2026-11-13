#!/usr/bin/env python3

from pathlib import Path
import unittest

import evidence_policy


class EvidencePolicyTests(unittest.TestCase):
    def test_allows_concise_documents_verifiers_and_summaries(self):
        allowed = {
            "docs/evidence/README.md": 100_000,
            "docs/evidence/packet/README.md": 100_000,
            "docs/evidence/packet/verify.py": 128 * 1024,
            "docs/evidence/packet/verify_payload.py": 1024,
            "docs/evidence/packet/manifest.json": 64 * 1024,
            "docs/evidence/packet/summary.json": 16 * 1024,
            "docs/evidence/ARCHIVE.json": 4096,
            "docs/evidence/restore.py": 8192,
        }
        for name, size in allowed.items():
            with self.subTest(name=name):
                self.assertIsNone(evidence_policy.violation(name, size))

    def test_rejects_payloads_and_oversized_metadata(self):
        rejected = {
            "docs/evidence/packet/report.json": 16 * 1024 + 1,
            "docs/evidence/packet/manifest.json": 64 * 1024 + 1,
            "docs/evidence/packet/verify.py": 128 * 1024 + 1,
            "docs/evidence/packet/sources/server.rs": 100,
            "docs/evidence/packet/Cargo.lock": 100,
            "docs/evidence/packet/rehearse.py": 100,
            "docs/evidence/packet/run.log": 100,
            "docs/evidence/packet/capture.png": 100,
            "docs/evidence/packet/payload.gz": 100,
        }
        for name, size in rejected.items():
            with self.subTest(name=name):
                self.assertIsNotNone(evidence_policy.violation(name, size))

    def test_ignores_files_outside_evidence(self):
        self.assertIsNone(evidence_policy.violation("server/src/main.rs", 10_000_000))

    def test_current_repository_policy(self):
        repo = Path(__file__).resolve().parents[1]
        errors = []
        for name in evidence_policy.tracked_files(repo):
            error = evidence_policy.violation(name, (repo / name).stat().st_size)
            if error:
                errors.append(error)
        self.assertEqual([], errors)


if __name__ == "__main__":
    unittest.main()
