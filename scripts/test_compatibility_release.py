import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import release


class CompatibilityReleaseTests(unittest.TestCase):
    def test_artifact_manifest_comes_from_shared_contract_and_shared_cache(self):
        manifest = {"release": "0.41.0", "protocol": 9}
        with tempfile.TemporaryDirectory() as directory:
            with patch.object(release, "run", side_effect=["/repo/.git", json.dumps(manifest)]) as run:
                with patch.dict(release.os.environ, {}, clear=True):
                    path = release.write_compatibility_manifest(Path(directory), "0.41.0")
            self.assertEqual(json.loads(path.read_text()), manifest)
            self.assertEqual(run.call_args.kwargs["env"]["CARGO_TARGET_DIR"], "/repo/target")
            self.assertIn("--locked", run.call_args.args)

    def test_stale_release_manifest_is_not_published_as_current(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch.object(release, "run", side_effect=["/repo/.git", '{"release":"0.1.0"}']):
                with self.assertRaises(SystemExit):
                    release.write_compatibility_manifest(Path(directory), "0.41.0")
            self.assertFalse((Path(directory) / "compatibility.json").exists())
