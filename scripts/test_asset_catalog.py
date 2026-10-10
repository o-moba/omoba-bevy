#!/usr/bin/env python3
"""Contract and corruption checks for the static creator catalogue."""
import copy
import json
from pathlib import Path
import shutil
import struct
import tempfile
import unittest
import zlib

from export_asset_catalog import CatalogError, Exporter, ROOT, canonical, digest, write_export

COMMIT = "0" * 40


def tiny_png():
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(b"\x00\xff\x00\x00")) + chunk(b"IEND", b""))


class AssetCatalogTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.exporter = Exporter()
        cls.catalog = cls.exporter.build(COMMIT)

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.temp = Path(self.temporary.name)

    def fixture(self):
        root = self.temp / "game"
        for source in self.exporter.inputs:
            target = root / source
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / source, target)
        return root

    def mutate(self, root, source, action):
        path = root / source
        value = json.loads(path.read_text())
        action(value)
        path.write_text(json.dumps(value))

    def authored_mutation_fails(self, action, message):
        root = self.fixture()
        self.mutate(root, "shared/assets/catalog/asset-requirements.json", action)
        with self.assertRaisesRegex(CatalogError, message):
            Exporter(root).build(COMMIT)

    def test_complete_graph_and_content_integrity(self):
        catalog = self.catalog
        self.assertEqual((len(catalog["classes"]), len(catalog["skills"])), (17, 68))
        ids = {r["id"] for r in catalog["requirements"]}
        skills = {s["id"] for s in catalog["skills"]}
        for row in catalog["classes"]:
            self.assertEqual([s["slot"] for s in row["skills"]], ["q", "w", "e", "r"])
            self.assertTrue({s["id"] for s in row["skills"]} <= skills)
            self.assertTrue(set(row["requirementIds"]) <= ids)
            self.assertTrue(row["basicAttack"]["requirementIds"])
        for row in catalog["skills"]:
            self.assertTrue(set(row["requirementIds"]) <= ids)
            self.assertIn("icon", row)
        for relative, data in self.exporter.files.items():
            self.assertEqual(Path(relative).stem, digest(data))
        revision_source = {key: value for key, value in catalog.items() if key != "revision"}
        self.assertEqual(catalog["revision"], "sha256-" + digest(canonical(revision_source)))

    def test_deterministic_output_and_check(self):
        exporter = Exporter()
        self.assertEqual(exporter.build(COMMIT), self.catalog)
        output = self.temp / "out"
        write_export(exporter, self.catalog, output)
        first = {p.relative_to(output): p.read_bytes() for p in output.rglob("*") if p.is_file()}
        write_export(exporter, self.catalog, output)
        self.assertEqual(first, {p.relative_to(output): p.read_bytes() for p in output.rglob("*") if p.is_file()})
        write_export(exporter, self.catalog, output, check=True)
        (output / "catalog.json").write_text("{}")
        with self.assertRaisesRegex(CatalogError, "Stale export"):
            write_export(exporter, self.catalog, output, check=True)

    def test_missing_and_corrupt_baseline_fail(self):
        root = self.fixture()
        (root / "client/assets/weapons/dagger.glb").unlink()
        with self.assertRaisesRegex(CatalogError, "Missing source"):
            Exporter(root).build(COMMIT)
        (root / "client/assets/weapons/dagger.glb").write_bytes(b"not a model")
        with self.assertRaisesRegex(CatalogError, "Invalid GLB"):
            Exporter(root).build(COMMIT)

    def test_duplicate_and_unsafe_ids_fail(self):
        for value in ("../../outside", "MixedCase", "", "x" * 129):
            with self.subTest(value=value):
                self.authored_mutation_fails(lambda d: d["requirements"][0].update(id=value), "Invalid requirement")

    def test_duplicate_requirement_fails(self):
        self.authored_mutation_fails(lambda d: d["requirements"].append(copy.deepcopy(d["requirements"][0])),
                                    "Duplicate requirement")

    def test_dangling_and_ambiguous_edges_fail(self):
        self.authored_mutation_fails(lambda d: d["requirements"][0].update(uses=[{"skillId": "missing"}]),
                                    "Unknown skill")
        self.authored_mutation_fails(lambda d: d["requirements"][0].update(
            uses=[{"skillId": "wild_switch", "basicAttackId": "wildspark"}]), "one target")
        self.authored_mutation_fails(lambda d: d["requirements"][0]["uses"][0].update(phase="bad phase"),
                                    "Invalid phase")

    def test_generated_ids_cannot_overwrite_authored(self):
        for rid in ("basic.warrior.presentation", "skill.wild_switch.presentation", "motion.reload_snap"):
            with self.subTest(rid=rid):
                self.authored_mutation_fails(lambda d: d["requirements"][0].update(id=rid), "Reserved generated")

    def test_source_paths_and_output_paths_cannot_escape(self):
        for path in ("../outside.glb", "/tmp/outside.glb", "client/../outside.glb", "client\\outside.glb"):
            self.authored_mutation_fails(lambda d: d["requirements"][0]["baseline"].update(sourcePath=path),
                                        "Unlisted handheld model|Unsafe source path")
        root = self.fixture()
        (root / "client/assets/weapons/dagger.glb").unlink()
        outside = self.temp / "outside.glb"
        outside.write_bytes((ROOT / "client/assets/weapons/dagger.glb").read_bytes())
        (root / "client/assets/weapons/dagger.glb").symlink_to(outside)
        with self.assertRaisesRegex(CatalogError, "escapes checkout"):
            Exporter(root).build(COMMIT)
        with self.assertRaisesRegex(CatalogError, "checkout or an ancestor"):
            write_export(self.exporter, self.catalog, ROOT)
        output = self.temp / "out"
        output.mkdir()
        (output / "files").symlink_to(self.temp)
        with self.assertRaisesRegex(CatalogError, "escapes destination"):
            write_export(self.exporter, self.catalog, output)

    def test_unknown_skills_motions_and_models_fail(self):
        for source, action, error in [
            ("shared/assets/catalog/heroes.json", lambda d: d["classes"][5]["skills"].__setitem__(0, "missing"), "Unknown skill"),
            ("client/assets/config/skills.skillfx", lambda d: d["skills"]["wild_switch"].update(release="missing"), "Unknown motion"),
            ("client/assets/config/skills.skillfx", lambda d: d["skills"]["wild_traps"]["body"].update(model="new_prop"), "Missing authored model"),
        ]:
            with self.subTest(error=error):
                root = self.fixture()
                self.mutate(root, source, action)
                with self.assertRaisesRegex(CatalogError, error):
                    Exporter(root).build(COMMIT)

    def test_skill_reassignment_preserves_requirement_identity(self):
        root = self.fixture()
        def swap_slots(data):
            hero = next(row for row in data["classes"] if row["id"] == "wildspark")
            hero["skills"][0], hero["skills"][1] = hero["skills"][1], hero["skills"][0]
        self.mutate(root, "shared/assets/catalog/heroes.json", swap_slots)
        moved = Exporter(root).build(COMMIT)
        original = next(s for s in self.catalog["skills"] if s["id"] == "wild_switch")
        new = next(s for s in moved["skills"] if s["id"] == "wild_switch")
        self.assertEqual(original["requirementIds"], new["requirementIds"])
        self.assertEqual(original["icon"], new["icon"])
        self.assertNotEqual(self.catalog["source"]["inputsSha256"], moved["source"]["inputsSha256"])

    def test_wildspark_boundaries_and_unresolved_prop_license(self):
        rows = {row["id"]: row for row in self.catalog["requirements"]}
        repeater = rows["wildspark.handheld.repeater"]
        self.assertEqual(repeater["classIds"], ["riftshot", "wildspark"])
        self.assertEqual(repeater["status"], "supported")
        rocket = rows["wildspark.projectile.basic-rocket"]
        self.assertEqual(rocket["baseline"]["kind"], "procedural")
        self.assertEqual(rocket["baseline"]["details"]["effectiveForm"], "tumbler")
        self.assertNotIn("url", rocket["baseline"])
        trap = rows["wildspark.prop.trap"]
        self.assertEqual(trap["status"], "planned")
        self.assertNotIn("license", trap["baseline"])
        self.assertNotIn("url", trap["baseline"])
        self.assertFalse(any("avatar" in str(row["baseline"]) for row in rows.values()))
        self.assertTrue(any(row["role"] == "animation_reference" for row in rows.values()))

    def test_rocket_precedence_cannot_silently_drift(self):
        root = self.fixture()
        self.mutate(root, "client/assets/config/combat_visuals.json",
                    lambda d: d["profiles"]["wild_rocket"].update(form="comet"))
        with self.assertRaisesRegex(CatalogError, "effective rocket form changed"):
            Exporter(root).build(COMMIT)

    def test_previews_are_optional_verified_and_change_revision(self):
        model = next(r["baseline"] for r in self.catalog["requirements"] if r["baseline"]["kind"] == "model")
        self.assertIsNone(model["preview"])
        previews = self.temp / "previews"
        previews.mkdir()
        png = tiny_png()
        (previews / f"{model['sha256']}.png").write_bytes(png)
        exported = Exporter(preview_dir=previews).build(COMMIT)
        with_preview = next(r["baseline"] for r in exported["requirements"]
                            if r["baseline"]["sha256"] == model["sha256"])
        self.assertEqual(with_preview["preview"]["sha256"], digest(png))
        self.assertNotEqual(exported["revision"], self.catalog["revision"])
        self.assertNotEqual(exported["source"]["inputsSha256"], self.catalog["source"]["inputsSha256"])
        (previews / f"{model['sha256']}.png").write_bytes(b"bad png")
        with self.assertRaisesRegex(CatalogError, "Invalid PNG"):
            Exporter(preview_dir=previews).build(COMMIT)

    def test_invalid_provenance_profile_and_text_fail(self):
        self.authored_mutation_fails(lambda d: d["requirements"][0]["baseline"].update(license="unknown"),
                                    "Missing model provenance")
        self.authored_mutation_fails(lambda d: d["requirements"][0].update(profile={"id": "projectile", "version": 1}),
                                    "Unsupported v1 capability")
        self.authored_mutation_fails(lambda d: d["requirements"][0].update(brief=" "), "Invalid brief")
        self.authored_mutation_fails(lambda d: d["requirements"][0].update(name="x" * 201), "Invalid name")

    def test_duplicate_json_keys_and_unsupported_schema_fail(self):
        root = self.fixture()
        path = root / "shared/assets/catalog/asset-requirements.json"
        path.write_text('{"schemaVersion":1,"schemaVersion":2,"requirements":[]}')
        with self.assertRaisesRegex(CatalogError, "Duplicate JSON key"):
            Exporter(root).build(COMMIT)
        path.write_text('{"schemaVersion":2,"requirements":[]}')
        with self.assertRaisesRegex(CatalogError, "Unsupported source schema"):
            Exporter(root).build(COMMIT)

    def test_runtime_mapping_drift_requires_authored_review(self):
        root = self.fixture()
        source = root / "shared/src/handheld.rs"
        source.write_text(source.read_text().replace('Some("wild-repeater")', 'Some("wild-launcher")'))
        with self.assertRaisesRegex(CatalogError, "Authored source evidence changed"):
            Exporter(root).build(COMMIT)

    def test_semantic_icon_binding_rejects_dangling_and_duplicate_ids(self):
        self.authored_mutation_fails(lambda d: d["iconAtlases"][0]["skillIds"].__setitem__(0, "missing"),
                                    "Unknown semantic icon")
        self.authored_mutation_fails(lambda d: d["iconAtlases"][0]["skillIds"].__setitem__(0, "battle_rally"),
                                    "Duplicate semantic icon")

    def test_revision_canonical_numbers_match_browser_json(self):
        self.assertEqual(canonical({"z": 1.0, "a": [0.0, -0.0, 0.000001, 0.0416667]}),
                         b'{"a":[0,0,0.000001,0.0416667],"z":1}')
        for number in (float("nan"), float("inf"), 9007199254740992, 0.0000001):
            with self.assertRaises(CatalogError):
                canonical({"n": number})


if __name__ == "__main__":
    unittest.main()
