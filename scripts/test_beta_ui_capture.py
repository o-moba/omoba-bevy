#!/usr/bin/env python3
"""Check capture profile evidence without starting binaries or a renderer."""
import unittest

from capture_verdant import BETA_IMAGES, verify_beta_ui_profile


def evidence(mobile, images=BETA_IMAGES):
    return {"captures": [dict(stage=stage, file=filename, mobile_controls=mobile)
                         for stage, filename in enumerate(images)]}


class BetaUiProfileEvidenceTest(unittest.TestCase):
    def test_matching_mobile_and_desktop_require_all_seven_stage_records(self):
        for mobile, profile in ((True, "mobile"), (False, "desktop")):
            for height in (390, 720):
                with self.subTest(profile=profile, height=height):
                    images = tuple(name.replace("720p", f"{height}p") for name in BETA_IMAGES)
                    result = verify_beta_ui_profile(evidence(mobile, images), mobile, images)
                    self.assertTrue(result["pass"], result["errors"])
                    self.assertEqual(result["requested_profile"], profile)
                    self.assertEqual(result["actual_profile"], profile)
                    self.assertEqual(len(result["stages"]), 7)
                    self.assertTrue(all(stage["mobile_controls"] is mobile for stage in result["stages"]))

    def test_wrong_profile_or_a_single_stage_switch_fails_with_build_guidance(self):
        for requested in (True, False):
            for mixed in (False, True):
                with self.subTest(requested=requested, mixed=mixed):
                    summary = evidence(requested if mixed else not requested)
                    summary["captures"][-1]["mobile_controls"] = not requested
                    result = verify_beta_ui_profile(summary, requested, BETA_IMAGES)
                    self.assertFalse(result["pass"])
                    self.assertEqual(result["actual_profile"], "mixed" if mixed else (
                        "desktop" if requested else "mobile"))
                    self.assertIn("Stage 6 reports", " ".join(result["errors"]))
                    self.assertIn("development/debug client build", " ".join(result["errors"]))

    def test_missing_malformed_and_nonboolean_profile_evidence_is_rejected(self):
        malformed = [None, [], {}, {"captures": None}, {"captures": {}}, {"captures": []},
                     {"captures": [None]}]
        for value in (None, 0, 1, "true", "false", [], {}):
            summary = evidence(True)
            summary["captures"][3]["mobile_controls"] = value
            malformed.append(summary)
        summary = evidence(True)
        del summary["captures"][3]["mobile_controls"]
        malformed.append(summary)
        for summary in malformed:
            with self.subTest(summary=summary):
                result = verify_beta_ui_profile(summary, True, BETA_IMAGES)
                self.assertFalse(result["pass"])
                self.assertTrue(result["errors"])

    def test_stage_coverage_cannot_be_replaced_by_duplicate_or_mislabeled_captures(self):
        malformed = []
        summary = evidence(False)
        summary["captures"].pop()
        malformed.append(summary)
        summary = evidence(False)
        summary["captures"].append(dict(summary["captures"][0]))
        malformed.append(summary)
        summary = evidence(False)
        summary["captures"][0]["file"] = BETA_IMAGES[1]
        malformed.append(summary)
        for stage in (True, "0", -1, 7, None):
            summary = evidence(False)
            summary["captures"][0]["stage"] = stage
            malformed.append(summary)
        for summary in malformed:
            with self.subTest(summary=summary):
                self.assertFalse(verify_beta_ui_profile(summary, False, BETA_IMAGES)["pass"])


if __name__ == "__main__":
    unittest.main()
