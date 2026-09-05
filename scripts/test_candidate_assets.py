#!/usr/bin/env python3
"""Exercise the candidate content gate against real assets and reversible mutations.

Run: python3 scripts/test_candidate_assets.py -v
Known denied bytes are read from the immutable pre-removal commit f2a3359,
never retained in the candidate. A shallow checkout may instead provide the
prior asset tree through OMOBA_DENIED_ASSET_ROOT. No downloads are performed.
"""
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import unittest
from unittest import mock

import package_native

from validate_candidate_assets import POLICY, ROOT, validate


class CandidateAssetGateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix="omoba-asset-gate-tests-")
        cls.addClassCleanup(cls.temporary.cleanup)
        cls.directory = Path(cls.temporary.name)
        cls.assets = cls.directory / "assets"
        shutil.copytree(ROOT / "client/assets", cls.assets)
        cls.reviewed_policy = json.loads(POLICY.read_text())
        cls.denied_bytes = {}
        archive = os.environ.get("OMOBA_DENIED_ASSET_ROOT")
        for relative, record in cls.reviewed_policy["denied_files"].items():
            if archive:
                data = (Path(archive) / relative).read_bytes()
            else:
                result = subprocess.run(
                    ["git", "show", f"f2a3359:client/assets/{relative}"],
                    cwd=ROOT, capture_output=True, check=False)
                if result.returncode:
                    raise RuntimeError("Denied historical fixture unavailable; provide "
                                       "OMOBA_DENIED_ASSET_ROOT pointing to prior assets")
                data = result.stdout
            if hashlib.sha256(data).hexdigest() != record["sha256"]:
                raise RuntimeError(f"Historical fixture does not match reviewed denied identity: {relative}")
            cls.denied_bytes[relative] = data

    def setUp(self):
        self.saved = {}

    def tearDown(self):
        for relative, original in self.saved.items():
            path = self.assets / relative
            if path.is_symlink() or path.exists():
                path.unlink()
            if original is not None:
                path.write_bytes(original)

    def remember(self, relative):
        if relative not in self.saved:
            path = self.assets / relative
            self.saved[relative] = path.read_bytes() if path.is_file() else None

    def write(self, relative, data):
        self.remember(relative)
        path = self.assets / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)

    def write_json(self, relative, value):
        self.write(relative, (json.dumps(value, indent=2) + "\n").encode())

    def remove(self, relative):
        self.remember(relative)
        (self.assets / relative).unlink()

    def expect_failure(self, fragment, policy=POLICY):
        result = validate(self.assets, policy)
        self.assertEqual(result["status"], "FAIL", result)
        self.assertTrue(any(fragment in error for error in result["errors"]), result)
        return result

    def replace_actor_json(self, mutate):
        relative = "avatars/agnes.glb"
        data = (self.assets / relative).read_bytes()
        old_json_length, chunk_type = struct.unpack_from("<II", data, 12)
        self.assertEqual(chunk_type, 0x4E4F534A)
        gltf = json.loads(data[20:20 + old_json_length])
        mutate(gltf)
        encoded = json.dumps(gltf, separators=(",", ":")).encode()
        encoded += b" " * (-len(encoded) % 4)
        tail = data[20 + old_json_length:]
        rewritten = (struct.pack("<4sII", b"glTF", 2, 20 + len(encoded) + len(tail))
                     + struct.pack("<II", len(encoded), chunk_type) + encoded + tail)
        self.write(relative, rewritten)
        # Deliberately simulate an erroneous review-hash update. The second,
        # independent permission check must reject the content nonetheless.
        policy = copy.deepcopy(self.reviewed_policy)
        policy["approved_actor_models"][relative]["sha256"] = hashlib.sha256(rewritten).hexdigest()
        policy_path = self.directory / "modified-review-policy.json"
        policy_path.write_text(json.dumps(policy, indent=2) + "\n")
        self.write("config/asset_policy.json", policy_path.read_bytes())
        return policy_path

    def test_reviewed_source_and_independent_package_copy_pass(self):
        for asset_root in [ROOT / "client/assets", self.assets]:
            with self.subTest(root=str(asset_root)):
                result = validate(asset_root)
                self.assertEqual(result["status"], "PASS", result)
                self.assertEqual(result["errors"], [])

    def test_every_denied_binary_and_preview_fails_when_renamed(self):
        for index, (original, data) in enumerate(self.denied_bytes.items()):
            relative = f"unexpected/relabelled-{index}.dat"
            with self.subTest(original=original):
                self.write(relative, data)
                self.expect_failure(f"denied content: {relative}")
                (self.assets / relative).unlink()

    def test_denied_stem_rejects_unrecognized_bytes_and_case_changes(self):
        self.write("unexpected/EL-BUENO.txt", b"not the historical file")
        self.expect_failure("denied content: unexpected/EL-BUENO.txt")

    def test_unknown_model_fails_even_if_it_copies_approved_geometry(self):
        self.write("unexpected/unreviewed.glb", (self.assets / "avatars/agnes.glb").read_bytes())
        self.expect_failure("unknown model: unexpected/unreviewed.glb")

    def test_model_magic_is_checked_even_with_a_non_model_extension(self):
        self.write("unexpected/disguised.dat", (self.assets / "avatars/agnes.glb").read_bytes())
        self.expect_failure("unknown model: unexpected/disguised.dat")

    def test_missing_required_actor_model_fails(self):
        self.remove("avatars/agnes.glb")
        self.expect_failure("missing approved model: avatars/agnes.glb")

    def test_missing_referenced_preview_fails(self):
        self.remove("avatars/agnes.jpg")
        self.expect_failure("missing actor preview: avatars/agnes.glb")

    def test_manifest_cc0_cannot_override_restrictive_embedded_metadata(self):
        def restrict(gltf):
            gltf["extensions"]["VRM"]["meta"]["licenseName"] = "Redistribution_Prohibited"
        policy = self.replace_actor_json(restrict)
        result = self.expect_failure("embedded licenseName must be CC0", policy)
        self.assertFalse(any("hash" in error for error in result["errors"]), result)
        manifest = json.loads((self.assets / "avatars/manifest.json").read_text())
        self.assertEqual(next(a for a in manifest["avatars"] if a["slug"] == "agnes")["license"], "CC0")

    def test_commercial_restriction_fails_even_with_an_updated_review_hash(self):
        def restrict(gltf):
            gltf["extensions"]["VRM"]["meta"]["commercialUssageName"] = "Disallow"
        policy = self.replace_actor_json(restrict)
        self.expect_failure("embedded commercialUssageName must be Allow", policy)

    def test_external_actor_dependency_fails_even_with_an_updated_review_hash(self):
        def add_remote_texture(gltf):
            gltf.setdefault("images", []).append({"uri": "https://example.invalid/texture.png"})
        policy = self.replace_actor_json(add_remote_texture)
        self.expect_failure("external images dependency", policy)

    def test_malformed_embedded_metadata_returns_a_structured_failure(self):
        def corrupt(gltf):
            gltf["extensions"]["VRM"]["meta"] = []
        policy = self.replace_actor_json(corrupt)
        self.expect_failure("invalid or incomplete asset inventory", policy)

    def test_bad_manifest_source_provenance_fails(self):
        manifest = json.loads((self.assets / "avatars/manifest.json").read_text())
        manifest["avatars"][0]["source_url"] = "https://example.invalid/unreviewed"
        self.write_json("avatars/manifest.json", manifest)
        self.expect_failure("manifest provenance mismatch")

    def test_bad_procedural_provenance_fails(self):
        manifest = json.loads((self.assets / "minions/manifest.json").read_text())
        manifest["procedural"][0]["provenance"] = "downloaded replacement"
        self.write_json("minions/manifest.json", manifest)
        self.expect_failure("missing procedural provenance")

    def test_missing_procedural_role_fails(self):
        manifest = json.loads((self.assets / "minions/manifest.json").read_text())
        manifest["procedural"].pop()
        self.write_json("minions/manifest.json", manifest)
        self.expect_failure("unreviewed minions procedural roster")

    def test_altered_environment_bytes_fail(self):
        self.write("verdant/environment.glb", (self.assets / "verdant/environment.glb").read_bytes() + b"mutation")
        self.expect_failure("unreviewed environment model hash: verdant/environment.glb")

    def test_missing_environment_model_fails(self):
        self.remove("verdant/foliage.glb")
        self.expect_failure("missing environment model: verdant/foliage.glb")

    def test_environment_manifest_cannot_silently_approve_an_extra_scene(self):
        manifest = json.loads((self.assets / "verdant/manifest.json").read_text())
        manifest["files"].append(copy.deepcopy(manifest["files"][0]))
        self.write_json("verdant/manifest.json", manifest)
        self.expect_failure("unreviewed environment manifest hash")

    def test_package_policy_cannot_approve_its_own_mutations(self):
        policy = copy.deepcopy(self.reviewed_policy)
        policy["denied_files"] = {}
        self.write_json("config/asset_policy.json", policy)
        self.expect_failure("asset policy differs from the reviewed repository policy")

    def test_actor_symlink_escape_fails(self):
        original = self.assets / "avatars/agnes.glb"
        outside = self.directory / "outside.glb"
        outside.write_bytes(original.read_bytes())
        self.remove("avatars/agnes.glb")
        original.symlink_to(outside)
        self.expect_failure("asset reference escapes root")

    def test_preview_path_traversal_fails(self):
        manifest = json.loads((self.assets / "avatars/manifest.json").read_text())
        manifest["avatars"][0]["thumbnail"] = "../../outside.png"
        self.write_json("avatars/manifest.json", manifest)
        self.expect_failure("unsafe asset reference")

    def test_cli_failure_exit_and_json_match_api(self):
        self.remove("avatars/agnes.glb")
        result = subprocess.run(["python3", str(ROOT / "scripts/validate_candidate_assets.py"),
                                 "--assets", str(self.assets)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(json.loads(result.stdout)["status"], "FAIL")
        self.assertFalse(result.stderr, result.stderr)

    def packaging_source(self, case):
        repository = self.directory / case
        (repository / "client").mkdir(parents=True)
        (repository / "client/assets").symlink_to(self.assets, target_is_directory=True)
        return repository

    def test_package_source_failure_prevents_cargo_and_output_creation(self):
        self.remove("avatars/agnes.glb")
        repository = self.packaging_source("source-rejection")
        output = repository / "candidate"
        with mock.patch.object(package_native, "ROOT", repository), \
                mock.patch("sys.argv", ["package_native.py", "--output", str(output)]), \
                mock.patch.object(package_native.subprocess, "run") as cargo, \
                mock.patch("sys.stderr"):
            with self.assertRaises(SystemExit) as failure:
                package_native.main()
        self.assertEqual(failure.exception.code, 2)
        cargo.assert_not_called()
        self.assertFalse(output.exists())

    def test_copied_content_failure_prevents_launchers_and_build_identity(self):
        repository = self.packaging_source("copied-rejection")
        output = repository / "candidate"
        target = repository / "target"
        (target / "debug").mkdir(parents=True)
        suffix = ".exe" if os.name == "nt" else ""
        for binary in ["client", "server", "bots"]:
            (target / "debug" / (binary + suffix)).write_bytes(b"test fixture; never executed")
        listed = "\n".join("client/assets/" + p.relative_to(self.assets).as_posix()
                           for p in self.assets.rglob("*") if p.is_file())
        real_copy = shutil.copy2

        def corrupted_copy(source, destination, *args, **kwargs):
            result = real_copy(source, destination, *args, **kwargs)
            if Path(destination).resolve() == (output / "assets/avatars/agnes.glb").resolve():
                with Path(destination).open("ab") as stream:
                    stream.write(b"simulated copy corruption")
            return result

        with mock.patch.object(package_native, "ROOT", repository), \
                mock.patch("sys.argv", ["package_native.py", "--output", str(output)]), \
                mock.patch.dict(os.environ, {"CARGO_TARGET_DIR": str(target)}), \
                mock.patch.object(package_native.subprocess, "run") as cargo, \
                mock.patch.object(package_native, "run", return_value=listed), \
                mock.patch.object(package_native.shutil, "copy2", side_effect=corrupted_copy):
            with self.assertRaisesRegex(RuntimeError, "packaged asset content gate failed"):
                package_native.main()
        cargo.assert_called_once()
        report = json.loads((output / "ASSET-REVIEW.json").read_text())
        self.assertEqual(report["status"], "FAIL")
        self.assertFalse((output / "BUILD.json").exists())
        self.assertFalse(list(output.glob("launch-*.sh")))


if __name__ == "__main__":
    unittest.main()
