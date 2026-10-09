"""Source provenance must remain stable when a generated snapshot is committed."""
from pathlib import Path
import subprocess
import tempfile
import unittest

from export_workshop import source_commit


class WorkshopProvenanceTests(unittest.TestCase):
    def test_snapshot_and_docs_commits_do_not_change_source_revision(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)

            def git(*args):
                return subprocess.check_output(["git", *args], cwd=root, text=True).strip()

            def commit(message):
                git("add", ".")
                git("-c", "user.name=Workshop Fixture", "-c", "user.email=fixture@example.invalid",
                    "commit", "--quiet", "-m", message)
                return git("rev-parse", "HEAD")

            git("init", "--quiet")
            source = root / "source.rs"
            source.write_text("canonical rules v1\n")
            first = commit("game source")
            self.assertEqual(source_commit(root, [source]), first)
            (root / "workshop.json").write_text('{"sourceCommit":"' + first + '"}\n')
            (root / "README.md").write_text("Documentation only.\n")
            snapshot = commit("generated snapshot and documentation")
            self.assertNotEqual(snapshot, first)
            self.assertEqual(source_commit(root, [source]), first)
            source.write_text("canonical rules v2\n")
            second = commit("changed game source")
            self.assertEqual(source_commit(root, [source]), second)


if __name__ == "__main__":
    unittest.main()
