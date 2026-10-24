#!/usr/bin/env python3
"""OMOBA's committed game contract, its header rule, the protocol reader checks and the fixtures."""
import contextlib
import copy
import io
import json
from pathlib import Path
import shutil
import tempfile
import unittest

import build_game_contract_fixtures as fixtures
from export_game_contract import (CONTRACT, INPUTS, PACKAGE_SCHEMA_FILE, PROFILE, PROFILE_SCHEMA_FILE, PROTOCOL, ROOT,
                                  SCHEMA_FILE, ContractError, export, finish, read_contract, render, revision_of,
                                  schema_errors)

COMMIT = "0" * 40
CATALOG = Path(CONTRACT).parent


class GameContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = (ROOT / CONTRACT).read_bytes()
        cls.contract = read_contract(cls.data)

    def test_committed_contract_is_finished_and_valid(self):
        document = export(ROOT, ROOT / CATALOG, check=True)
        self.assertEqual(document["revision"], revision_of(document))
        self.assertEqual((document["schema"], document["schemaVersion"], document["minReaderVersion"]),
                         ("ekza.game-contract.v1", "1.0", "1.0"))
        self.assertEqual(document["profiles"], {"used": [PROFILE], "required": []})
        slots = document["slots"]
        self.assertEqual(len(slots), 107)
        self.assertEqual(sum(slot["customizable"] for slot in slots.values()), 101)
        self.assertEqual({slot["kind"] for slot in slots.values()}, {"animation"})
        self.assertEqual([target["id"] for target in document["targets"]][:2], ["any", "warrior"])
        self.assertEqual(len(document["targets"]), 18)

    def test_unchanged_body_keeps_its_header(self):
        again = finish(ROOT, json.loads(self.data), commit=COMMIT)
        self.assertEqual(render(again), self.data)

    def game_copy(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        for path in set(INPUTS) | {CONTRACT, SCHEMA_FILE, PROFILE_SCHEMA_FILE, PROTOCOL}:
            (root / path).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / path, root / path)
        return root

    def changed_body(self, root):
        body = json.loads(self.data)
        body["slots"]["base.run"]["animation"]["window"]["maxSecs"] = 1.1
        (root / CONTRACT).write_bytes(render(body))

    def test_changed_body_gets_a_new_revision_that_names_the_old_one(self):
        root = self.game_copy()
        self.changed_body(root)
        document = export(root, root / CATALOG, commit=COMMIT)
        self.assertEqual(document["previousRevision"], self.contract["revision"])
        self.assertNotEqual(document["revision"], self.contract["revision"])
        self.assertEqual(document["source"]["commit"], COMMIT)
        self.assertIn("presentationSha256", document["ext"][PROFILE])
        export(root, root / CATALOG, check=True)

    def test_check_refuses_a_body_without_its_header(self):
        root = self.game_copy()
        self.changed_body(root)
        with self.assertRaisesRegex(Exception, "Stale export file"):
            export(root, root / CATALOG, check=True, commit=COMMIT)

    def verdict(self, mutate, recompute=True):
        document = copy.deepcopy(self.contract)
        mutate(document)
        if recompute:
            document["revision"] = revision_of(document)
        try:
            with contextlib.redirect_stderr(io.StringIO()):
                read_contract(render(document))
        except ContractError as error:
            return error.code
        return "ok"

    def test_reader_checks(self):
        self.assertEqual(self.verdict(lambda d: d.update(minReaderVersion="1.1", schemaVersion="1.1")),
                         "reader-version")
        self.assertEqual(self.verdict(lambda d: d["rigs"]["humanoid"].update(gripBones=[])), "schema")
        self.assertEqual(self.verdict(lambda d: d.update(schemaVersion="1.3") or
                                      d["rigs"]["humanoid"].update(gripBones=[])), "ok")
        self.assertEqual(self.verdict(lambda d: d.update(license="MIT"), recompute=False), "revision")
        self.assertEqual(self.verdict(lambda d: d["targets"][1]["slots"].reverse()), "reference")
        self.assertEqual(self.verdict(lambda d: d["slots"]["base.walk"].update(
            customizable=True, customizableFrom="1")), "reference")
        self.assertEqual(self.verdict(lambda d: d["rules"].append(
            {"id": "clip.foot-slide", "severity": "error", "scope": "animation", "params": {},
             "checkedBy": ["tool"]})), "unknown-rule")
        self.assertEqual(self.verdict(lambda d: d["rules"].append(
            {"id": "omoba.clip.foot-slide", "severity": "error", "scope": "animation", "params": {},
             "checkedBy": ["tool"]})), "reference")
        self.assertEqual(self.verdict(lambda d: d["slots"]["base.idle"]["ext"][PROFILE].update(role="run")),
                         "profile")

    def test_package_manifest_schema_accepts_the_spec_example(self):
        example = {
            "schema": "ekza.package.v1", "schemaVersion": "1.0", "kind": "character",
            "contract": {"schema": "ekza.game-contract.v1", "game": "omoba", "revision": self.contract["revision"]},
            "target": {"id": "wildspark", "requirementId": "character.wildspark"},
            "slots": {"base.idle": {"mode": "default"},
                      "base.run": {"mode": "custom", "animation": 0, "markers": {}},
                      "basic.wildspark.0": {"mode": "custom", "animation": 1, "markers": {"contact": 0.067}}},
            "requires": [],
            "provenance": {"tool": "ekza-studio", "toolVersion": "0.7.0"},
        }
        self.assertEqual(schema_errors(ROOT, example, PACKAGE_SCHEMA_FILE), [])
        example["slots"]["base.run"]["animation"] = -1
        self.assertTrue(schema_errors(ROOT, example, PACKAGE_SCHEMA_FILE))

    def test_fixtures_are_current(self):
        with contextlib.redirect_stderr(io.StringIO()):
            files = fixtures.build(ROOT)
            fixtures.verify(ROOT, files)
        fixtures.write(ROOT, files, check=True)
        manifest = json.loads(files["manifest.json"])
        self.assertEqual(len(manifest["packageCases"]), 45)
        self.assertEqual({row["document"] for row in manifest["contractCases"]},
                         {"ok", "refused: schema", "refused: revision", "refused: reference", "refused: profile",
                          "refused: unknown-rule", "refused: reader-version"})


if __name__ == "__main__":
    unittest.main()
