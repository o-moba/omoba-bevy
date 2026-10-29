"""Classic identity authoring policy checks; these tests do not grant game access."""
import copy
import json
import tempfile
import unittest
from pathlib import Path

import hero_designs as hd
from export_game_contract import CONTRACT, ROOT, export, render, schema_errors, PROFILE_SCHEMA_FILE


class HeroDesignTests(unittest.TestCase):
    def setUp(self):
        self.contract = json.loads((ROOT / CONTRACT).read_bytes())
        self.known = hd.class_ids(ROOT)
        self.source = hd.read_source((ROOT / hd.SOURCE).read_bytes(), self.known)
        self.policy = hd.build_export(ROOT, self.contract["revision"], "0" * 40)

    def configured(self):
        policy = copy.deepcopy(self.policy)
        row = policy["classes"][self.known.index("wildspark")]
        row.update(status="configured", heroDesignId="fixture.wildspark", baseRevision="sha256-" + "a" * 64,
                   baseSha256="b" * 64, designContractRevision="sha256-" + "c" * 64)
        policy["revision"] = hd.revision_of(policy)
        binding = {"gameId": "omoba", "modeScope": "classic", "classId": "wildspark",
                   "gameContractRevision": self.contract["revision"], "heroDesignsRevision": policy["revision"],
                   **{key: row[key] for key in ("heroDesignId", "baseRevision", "baseSha256", "designContractRevision")}}
        return policy, binding

    def validate(self, binding, policy, target="wildspark"):
        return hd.validate_classic_binding(binding, policy=policy, target=target,
                                          game_contract_revision=self.contract["revision"])

    def test_committed_policy_explicitly_blocks_every_canonical_class(self):
        self.assertEqual(len(self.known), 17)
        for row in self.policy["classes"]:
            self.assertEqual(row["status"], "unconfigured")
            self.assertIsNone(row["heroDesignId"])
            binding = {key: None for key in hd.BINDING_FIELDS}
            with self.assertRaisesRegex(hd.HeroDesignError, "unconfigured"):
                self.validate(binding, self.policy, row["classId"])

    def test_only_exact_configured_class_binding_passes(self):
        policy, binding = self.configured()
        self.assertEqual(self.validate(binding, policy), binding)
        self.assertEqual(schema_errors(ROOT, binding, PROFILE_SCHEMA_FILE, "classicBinding"), [])
        self.assertEqual(schema_errors(ROOT, policy, PROFILE_SCHEMA_FILE, "heroDesignPolicy"), [])

    def test_any_unknown_and_wrong_class_cannot_claim_classic_binding(self):
        policy, binding = self.configured()
        for target in ("any", "unknown", "warrior"):
            with self.subTest(target=target), self.assertRaises(hd.HeroDesignError):
                self.validate(binding, policy, target)
        for key in hd.BINDING_FIELDS:
            changed = dict(binding, **{key: "any"})
            with self.subTest(field=key), self.assertRaises(hd.HeroDesignError):
                self.validate(changed, policy)
        with self.assertRaises(hd.HeroDesignError):
            self.validate(dict(binding, arbitraryUploadUrl="https://untrusted.invalid/a.vrm"), policy)

    def test_malformed_incomplete_or_rehashed_partial_policy_fails_closed(self):
        policy, binding = self.configured()
        for change in (
            lambda d: d.update(gameContractRevision=12),
            lambda d: d["classes"][0].update(status="configured"),
            lambda d: d["classes"][0].update(classId="unknown"),
            lambda d: d["classes"][0].update(classId="wildspark"),
            lambda d: d.update(uploadApproval=True),
            lambda d: d.pop("source"),
        ):
            changed = copy.deepcopy(policy)
            change(changed)
            changed["revision"] = hd.revision_of(changed)
            with self.assertRaises(hd.HeroDesignError):
                self.validate(binding, changed)
        with self.assertRaises(hd.HeroDesignError):
            self.validate(None, policy)

    def test_source_rejects_unknown_duplicate_missing_and_partially_assigned_rows(self):
        for change in (
            lambda d: d["classes"].pop(),
            lambda d: d["classes"][0].update(classId="any"),
            lambda d: d["classes"][0].update(classId="mage"),
            lambda d: d["classes"][0].update(heroDesignId="guessed-base"),
            lambda d: d.update(modeScope="workshop"),
            lambda d: d.update(approval=True),
        ):
            changed = copy.deepcopy(self.source)
            change(changed)
            with self.assertRaises(hd.HeroDesignError):
                hd.read_source(render(changed), self.known)
        with self.assertRaises(hd.HeroDesignError):
            hd.read_source(b"x" * (hd.MAX_BYTES + 1), self.known)
        with self.assertRaises(hd.HeroDesignError):
            hd.read_source(b'{"schema":1,"schema":2}', self.known)

    def test_policy_checksum_and_contract_revision_are_bound(self):
        policy, _ = self.configured()
        policy["classes"][0]["status"] = "configured"
        with self.assertRaises(hd.HeroDesignError):
            hd.read_export(render(policy), self.known, self.contract["revision"])
        with self.assertRaisesRegex(hd.HeroDesignError, "Stale"):
            hd.read_export(render(self.policy), self.known, "sha256-" + "f" * 64)

    def test_export_is_deterministic_keeps_core_bytes_and_checks_companion(self):
        before = (ROOT / CONTRACT).read_bytes()
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            export(ROOT, directory, commit="0" * 40)
            core = (directory / "game-contract.json").read_bytes()
            policy = (directory / hd.EXPORT).read_bytes()
            export(ROOT, directory, commit="0" * 40)
            self.assertEqual((directory / hd.EXPORT).read_bytes(), policy)
            self.assertEqual(core, before)
            export(ROOT, directory, check=True, commit="0" * 40)
            (directory / hd.EXPORT).write_bytes(policy + b" ")
            with self.assertRaisesRegex(Exception, "Stale export"):
                export(ROOT, directory, check=True, commit="0" * 40)

    def test_unchanged_policy_preserves_source_provenance_across_unrelated_commit(self):
        again = hd.build_export(ROOT, self.contract["revision"], "1" * 40, render(self.policy))
        self.assertEqual(again, self.policy)


if __name__ == "__main__":
    unittest.main()
