#!/usr/bin/env python3
"""The Verdant Crown tokens and UI assets in the client match what the handoff shipped."""
import copy
import json
import unittest

import sync_ui_assets
import sync_ui_tokens


class UiTokensTest(unittest.TestCase):
    def test_repo_copy_is_valid_unedited_and_matches_the_handoff_when_present(self):
        # With omoba-ui checked out next to this repository the copy must equal
        # handoff/tokens.json byte for byte; in CI only the lock hash is checked.
        self.assertEqual(sync_ui_tokens.check(sync_ui_tokens.find_handoff()), [])

    def test_validation_rejects_malformed_tokens(self):
        tokens = json.loads(sync_ui_tokens.COPY.read_text())
        self.assertEqual(sync_ui_tokens.validate(tokens), [])
        broken = copy.deepcopy(tokens)
        broken["color.surface.0"] = "#061513"
        broken["type.title.case"] = "title"
        broken["type.title.family"] = "font.family.missing"
        broken["size.button.height.desktop"] = "46"
        broken["widget.thing"] = 1
        del broken["type.body.line_height"]
        problems = "\n".join(sync_ui_tokens.validate(broken))
        for fragment in ("color.surface.0", "type.title.case", "type.title.family",
                         "size.button.height.desktop", "unknown group", "type.body: missing line_height"):
            self.assertIn(fragment, problems)

    def test_fonts_resolve_to_installed_files(self):
        tokens = json.loads(sync_ui_tokens.COPY.read_text())
        fonts = {key: value for key, value in tokens.items() if key.startswith("font.family.")}
        self.assertEqual(len(fonts), 8)
        for key, path in fonts.items():
            with self.subTest(key=key):
                self.assertTrue(sync_ui_tokens.font_asset(path).is_file())


class UiAssetsTest(unittest.TestCase):
    def test_installed_assets_match_the_manifest_and_the_budget(self):
        self.assertEqual(sync_ui_assets.check(sync_ui_tokens.find_handoff()), [])

    def test_manifest_keeps_insets_atlases_and_the_credit_line(self):
        manifest = json.loads(sync_ui_assets.MANIFEST.read_text())
        self.assertIn("game-icons.net", manifest["credits_required"])
        by_path = {entry["path"]: entry for entry in manifest["assets"]}
        slab = by_path["frames/button-primary@1x.png"]
        self.assertEqual((slab["nine_slice"]["left"], slab["nine_slice"]["top"]), (16, 14))
        self.assertEqual(by_path["sprites/cooldown-sweep-atlas@1x.png"]["atlas"]["at_1x"]["frames"], 60)
        self.assertFalse(any(path.endswith(".svg") for path in by_path))
        total = sum(entry["bytes"] for entry in manifest["assets"])
        self.assertLessEqual(total, sync_ui_assets.PACKAGE_BUDGET)


if __name__ == "__main__":
    unittest.main()
