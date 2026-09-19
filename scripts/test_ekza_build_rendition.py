#!/usr/bin/env python3
"""Contract tests for the Ekza rendition builder; standard library only.

The registry runs scripts/ekza_build_rendition.py as an external command, so these
tests pin what it may rely on: one JSON document on stdout, exit 0 / 2, a
content-addressed output file, and an output that passes Omoba's own profile check.
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ekza_publish

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/ekza_build_rendition.py"
# A shipped avatar keeps its VRM humanoid metadata, so it doubles as a real source.
SOURCE = ROOT / "client/assets/avatars/anna.glb"


def glb(document: dict) -> bytes:
    body = json.dumps(document).encode()
    body += b" " * (-len(body) % 4)
    return struct.pack("<4sII", b"glTF", 2, 20 + len(body)) + struct.pack("<II", len(body), 0x4E4F534A) + body


def run(source: Path, output: Path) -> tuple[int, dict, str]:
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "--source", str(source), "--output-dir", str(output)],
        capture_output=True,
        text=True,
        timeout=120,
    )
    return result.returncode, json.loads(result.stdout), result.stdout


class BuildRenditionContract(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory(prefix="ekza-build-test-")
        self.output = Path(self.directory.name)

    def tearDown(self) -> None:
        self.directory.cleanup()

    def test_real_avatar_builds_a_content_addressed_playable_rendition(self) -> None:
        code, result, stdout = run(SOURCE, self.output)
        self.assertEqual(code, 0, stdout)
        self.assertEqual(stdout.count("\n"), 1, "stdout must be exactly one JSON line")
        self.assertTrue(result["ok"])
        self.assertEqual(result["profile"], "desktop/humanoid-glb-v1")
        self.assertEqual(result["format"], "glb")
        data = Path(result["assetPath"]).read_bytes()
        self.assertEqual(Path(result["assetPath"]).name, result["sha256"] + ".glb")
        self.assertEqual(hashlib.sha256(data).hexdigest(), result["sha256"])
        self.assertEqual(len(data), result["sizeBytes"])
        self.assertEqual(result["report"]["clips"], ["idle", "walk", "attack", "cast", "death"])
        self.assertEqual(result["report"]["sourceSha256"], hashlib.sha256(SOURCE.read_bytes()).hexdigest())
        ekza_publish.validate_omoba_profile(data)  # the same rule the game enforces
        self.assertEqual([p.name for p in self.output.iterdir()], [result["sha256"] + ".glb"])

    def test_repeated_build_is_idempotent(self) -> None:
        first = run(SOURCE, self.output)[1]
        second = run(SOURCE, self.output)[1]
        self.assertEqual(first["sha256"], second["sha256"])
        self.assertEqual(len(list(self.output.iterdir())), 1)

    def test_unusable_sources_are_reported_not_crashed(self) -> None:
        cases = {
            "source_container": b"this is not a model, it is a sentence of plain text",
            "source_not_vrm": glb({"asset": {"version": "2.0"}, "nodes": [{}]}),
            "build_failed": glb(
                {"asset": {"version": "2.0"}, "nodes": [{}], "extensions": {"VRM": {"humanoid": {"humanBones": []}}}}
            ),
        }
        for expected, payload in cases.items():
            with self.subTest(expected):
                source = self.output / f"{expected}.vrm"
                source.write_bytes(payload)
                code, result, stdout = run(source, self.output / "out")
                self.assertEqual(code, 2, stdout)
                self.assertFalse(result["ok"])
                self.assertEqual(result["issues"][0]["code"], expected, stdout)
                self.assertTrue(result["issues"][0]["message"])
                self.assertFalse((self.output / "out").exists() and any((self.output / "out").iterdir()))

    def test_limits_match_the_published_profile(self) -> None:
        self.assertEqual(ekza_publish.MAX_MODEL_BYTES, 50 * 1024 * 1024)
        self.assertEqual(ekza_publish.REQUIRED_CLIPS, ("idle", "walk", "attack", "cast", "death"))
        self.assertEqual(ekza_publish.PROJECT_SELECTORS["omoba"], ("desktop", "humanoid-glb-v1"))


if __name__ == "__main__":
    unittest.main()
