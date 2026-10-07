"""The skill capture launcher refuses unusable evidence; the sheets refuse black tiles."""
import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest

import capture_standard_skills as launcher

try:  # Only the contact sheets need Pillow; the launcher tests run without it.
    from PIL import Image
    import build_skill_contact_sheets as sheets
except ModuleNotFoundError:
    Image = sheets = None

ROOT = Path(__file__).resolve().parents[1]


def phase_summary(third="impact"):
    captures = [dict(file="01-selection.png", mean_pixel=80.0),
                dict(file="0-idle.png", phase="idle", slot=None, skill=None, mean_pixel=90.0)]
    for slot, key in enumerate(launcher.SLOT_KEYS):
        for order, phase in enumerate(("windup", "release", third), start=1):
            captures.append(dict(file=f"{slot + 1}-{key}-{order}-{phase}.png", phase=phase, slot=slot,
                                 skill=f"{key}_skill", animation="cast", mean_pixel=90.0 + slot))
    return dict(captures=captures, skills=[dict(slot=slot) for slot in range(4)])


def touch_stills(directory, summary):
    for capture in summary["captures"]:
        (directory / capture["file"]).write_bytes(b"\x89PNG" + bytes(64))


def stage_colour(index):
    return (40 + index, 90, 60)


def write_stills(directory, summary):
    """Frames whose stage crop is one colour per still and whose HUD area is blue."""
    for index, capture in enumerate(summary["captures"]):
        frame = Image.new("RGB", (1280, 720), (5, 5, 200))
        frame.paste(stage_colour(index), sheets.CROP)
        frame.save(directory / capture["file"])


def run_main(arguments):
    with contextlib.redirect_stderr(io.StringIO()) as stderr:
        try:
            launcher.main(arguments)
        except SystemExit as exit_:
            return exit_.code, stderr.getvalue()
    return 0, stderr.getvalue()


class LauncherTests(unittest.TestCase):
    def test_hero_list_is_the_catalog_and_includes_the_adventurer(self):
        catalog = json.loads((ROOT / "shared/assets/catalog/heroes.json").read_text())
        self.assertEqual(launcher.HEROES, [hero["id"] for hero in catalog["classes"]])
        self.assertIn("adventurer", launcher.HEROES)
        self.assertNotIn("adventurer", launcher.ROSTER_HEROES)

    def test_look_options_name_a_shipped_rig_and_a_release_time(self):
        slugs = launcher.avatar_slugs(ROOT / "client/assets")
        self.assertEqual(len(slugs), 15)
        self.assertIn("agnes", slugs)
        self.assertEqual([launcher.release_time(value) for value in ("contact", "0", "0.45", "2")],
                         ["contact", "0", "0.45", "2"])

    def test_overlay_links_the_assets_and_replaces_only_the_named_registry(self):
        with tempfile.TemporaryDirectory() as temporary:
            temporary = Path(temporary)
            assets = temporary / "source"
            (assets / "config").mkdir(parents=True)
            (assets / "avatars").mkdir()
            (assets / "avatars" / "agnes.glb").write_text("model")
            (assets / "config" / "skills.skillfx").write_text("shipped skills")
            (assets / "config" / "combat_visuals.json").write_text("shipped visuals")
            overlay = temporary / "target.skillfx"
            overlay.write_text("target skills")
            (temporary / "work").mkdir()
            root = launcher.overlay_assets(assets, temporary / "work", dict(skillfx=overlay))
            self.assertTrue((root / "avatars").is_symlink())
            self.assertEqual((root / "avatars" / "agnes.glb").read_text(), "model")
            self.assertFalse((root / "config").is_symlink())
            self.assertEqual((root / "config" / "skills.skillfx").read_text(), "target skills")
            self.assertEqual((root / "config" / "combat_visuals.json").read_text(), "shipped visuals")
            self.assertEqual((assets / "config" / "skills.skillfx").read_text(), "shipped skills")
            (temporary / "both").mkdir()
            root = launcher.overlay_assets(assets, temporary / "both",
                                           dict(skillfx=overlay, combat_visuals=overlay))
            self.assertEqual((root / "config" / "combat_visuals.json").read_text(), "target skills")
            self.assertEqual((assets / "config" / "combat_visuals.json").read_text(), "shipped visuals")

    def test_phase_run_needs_idle_and_three_real_stills_per_slot(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            for third in ("impact", "settled"):
                summary = phase_summary(third)
                touch_stills(directory, summary)
                self.assertEqual(launcher.phase_problems(summary, directory), [])
            summary = phase_summary()
            (directory / "2-w-2-release.png").unlink()
            self.assertEqual(launcher.phase_problems(summary, directory), ["2-w-2-release.png is missing"])
            touch_stills(directory, summary)
            summary["captures"][3]["mean_pixel"] = 0.0
            self.assertEqual(launcher.phase_problems(summary, directory),
                             ["1-q-2-release.png is black or was not read back"])
            summary = phase_summary()
            del summary["captures"][-1]
            self.assertTrue(any("slot r" in problem for problem in launcher.phase_problems(summary, directory)))
            summary = phase_summary()
            summary["captures"][-1]["phase"] = "release"
            self.assertTrue(any("slot r" in problem for problem in launcher.phase_problems(summary, directory)))
            summary = phase_summary()
            del summary["captures"][1]
            self.assertIn("expected exactly one idle baseline", launcher.phase_problems(summary, directory))
            summary = phase_summary()
            summary["skills"].pop()
            self.assertIn("expected four skill records", launcher.phase_problems(summary, directory))

    def test_flight_look_ends_with_the_still_its_basic_record_names(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            summary = phase_summary()
            flight = dict(file="5-basic-flight.png", phase="flight", slot=255, skill="basic", mean_pixel=91.0)
            summary["captures"].append(flight)
            touch_stills(directory, summary)
            # A still nobody recorded, and a record without its still, are both refused.
            mismatch = "flight stills ['5-basic-flight.png'] do not match the basic attack record"
            self.assertEqual(launcher.phase_problems(summary, directory), [mismatch])
            summary["basic"] = dict(slot=255, stills=["5-basic-flight.png"])
            self.assertEqual(launcher.phase_problems(summary, directory), [])
            summary["captures"].pop()
            self.assertEqual(launcher.phase_problems(summary, directory),
                             ["flight stills [] do not match the basic attack record"])
            # A melee core throws nothing: no record and no still.
            summary["basic"] = None
            self.assertEqual(launcher.phase_problems(summary, directory), [])

    def test_aim_stills_belong_to_their_skill_record_and_carry_a_preview(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)

            def with_aim():
                summary = phase_summary()
                summary["aim"] = True
                for slot, key in enumerate(launcher.SLOT_KEYS):
                    summary["skills"][slot].update(modular=True, aim_stills=[f"{slot + 1}-{key}-0-aim.png"])
                    summary["captures"].append(dict(
                        file=f"{slot + 1}-{key}-0-aim.png", phase="aim", slot=slot, skill=f"{key}_skill",
                        mean_pixel=88.0, aim=dict(held_key=key.upper(), preview=dict(shape="lane"))))
                # The first slot offered a recast after its cast.
                summary["skills"][0]["aim_stills"].append("1-q-4-recast-aim.png")
                summary["captures"].append(dict(
                    file="1-q-4-recast-aim.png", phase="recast_aim", slot=0, skill="q_skill",
                    mean_pixel=88.0, aim=dict(held_key="Q", preview=dict(shape="none"))))
                touch_stills(directory, summary)
                return summary

            # The aim stills stand beside the three stills of the cast.
            self.assertEqual(launcher.phase_problems(with_aim(), directory), [])
            summary = with_aim()
            summary["skills"][1]["aim_stills"] = []
            self.assertEqual(launcher.phase_problems(summary, directory),
                             ["slot w: aim stills ['2-w-0-aim.png'] do not match its record"])
            summary = with_aim()
            del summary["captures"][-1]["aim"]["preview"]
            self.assertEqual(launcher.phase_problems(summary, directory),
                             ["1-q-4-recast-aim.png records no aim preview"])
            summary = with_aim()
            summary["captures"][-1]["mean_pixel"] = 0.0
            self.assertEqual(launcher.phase_problems(summary, directory),
                             ["1-q-4-recast-aim.png is black or was not read back"])
            # With --aim a modular skill without its aim still is refused; a legacy
            # skill has none and needs none.
            summary = with_aim()
            summary["skills"][2]["aim_stills"] = []
            summary["captures"] = [c for c in summary["captures"] if c["file"] != "3-e-0-aim.png"]
            self.assertEqual(launcher.phase_problems(summary, directory), ["slot e has no aim still"])
            summary["skills"][2]["modular"] = False
            self.assertEqual(launcher.phase_problems(summary, directory), [])
            # A run without --aim has no aim stills and asks for none.
            summary = phase_summary()
            for record in summary["skills"]:
                record["modular"] = True
            self.assertEqual(launcher.phase_problems(summary, directory), [])

    def test_manifest_keeps_classes_of_earlier_runs_and_fails_with_any_class(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "manifest.json"
            first = launcher.merge_manifest(path, dict(phases=True), [dict(hero="mage", **{"pass": True})])
            self.assertTrue(first["pass"])
            second = launcher.merge_manifest(path, dict(phases=True), [dict(hero="warden", **{"pass": False})])
            self.assertEqual(sorted(second["classes"]), ["mage", "warden"])
            self.assertFalse(second["pass"])
            self.assertEqual(json.loads(path.read_text()), second)

    def test_arguments_are_refused_before_anything_is_launched(self):
        with tempfile.TemporaryDirectory() as temporary:
            temporary = Path(temporary)
            for name in ("client", "server"):
                (temporary / name).write_text("binary")
            (temporary / "assets" / "avatars").mkdir(parents=True)
            (temporary / "assets" / "avatars" / "manifest.json").write_text('{"avatars": [{"slug": "agnes"}]}')
            base = ["--client-bin", str(temporary / "client"), "--server-bin", str(temporary / "server"),
                    "--assets", str(temporary / "assets"), "--output", str(temporary / "out")]
            for extra, message in (
                    (["--phases", "--roster"], "--phases replaces"),
                    (["--phases", "--timeout", "120"], "at least 300"),
                    (["--hero", "adventurer"], "adventurer needs --phases"),
                    (["--phases", "--skillfx", str(temporary / "absent.skillfx")], "Overlay file not found"),
                    (["--visual-mode", "isometric"], "invalid choice"),
                    (["--avatar", "agnes"], "need --phases"),
                    (["--release-at", "contact"], "need --phases"),
                    (["--flight"], "need --phases"),
                    (["--interleave"], "need --phases"),
                    (["--aim"], "need --phases"),
                    (["--phases", "--avatar", "nobody"], "Unknown avatar"),
                    (["--phases", "--release-at", "3"], "expected `contact` or 0..2 seconds"),
                    (["--phases", "--release-at", "soon"], "invalid release_time value"),
            ):
                code, stderr = run_main(base + extra)
                self.assertEqual(code, 2, extra)
                self.assertIn(message, stderr)
            self.assertFalse((temporary / "out").exists())


@unittest.skipUnless(sheets, "the contact sheets need Pillow")
class ContactSheetTests(unittest.TestCase):
    def run_directory(self, temporary, heroes):
        run = Path(temporary)
        for hero in heroes:
            (run / hero).mkdir()
            summary = phase_summary("settled" if hero == "mage" else "impact")
            write_stills(run / hero, summary)
            stills = [capture for capture in summary["captures"] if capture.get("phase")]
            (run / hero / "capture-run.json").write_text(json.dumps({"pass": True, "stills": stills}))
        return run

    def test_one_sheet_per_class_and_a_wall_of_release_stills(self):
        with tempfile.TemporaryDirectory() as temporary:
            run = self.run_directory(temporary, ["warrior", "mage"])
            index = sheets.build(run)
            self.assertEqual(index["classes"], {"warrior": "warrior.png", "mage": "mage.png"})
            self.assertEqual(index["skills"], 8)
            with Image.open(run / "sheets" / "mage.png") as sheet:
                self.assertEqual(sheet.width, 3 * (sheets.TILE[0] + sheets.GAP) + sheets.GAP)
                self.assertEqual(sheet.height,
                                 sheets.HEADER + 4 * (sheets.TILE[1] + sheets.LABEL + sheets.GAP) + sheets.GAP)
                # The third column of the first row is the settled still of Q
                # (capture 4), cropped to the stage: no HUD pixel reaches a tile corner.
                left = sheets.GAP + 2 * (sheets.TILE[0] + sheets.GAP)
                for x, y in ((0, 0), (sheets.TILE[0] - 1, sheets.TILE[1] - 1)):
                    self.assertEqual(sheet.getpixel((left + x, sheets.HEADER + y)), stage_colour(4))
            with Image.open(run / "sheets" / "wall-release.png") as wall:
                self.assertEqual(wall.height,
                                 sheets.HEADER + 2 * (sheets.WALL_TILE[1] + sheets.LABEL + sheets.GAP) + sheets.GAP)
                # Rows follow the catalog order: warrior first, then mage, whose
                # release still of R is capture 12.
                x = 104 + sheets.GAP + 3 * (sheets.WALL_TILE[0] + sheets.GAP) + 50
                y = sheets.HEADER + (sheets.WALL_TILE[1] + sheets.LABEL + sheets.GAP) + 50
                self.assertEqual(wall.getpixel((x, y)), stage_colour(12))

    def test_black_missing_or_failed_evidence_is_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            run = self.run_directory(temporary, ["warrior"])
            Image.new("RGB", (1280, 720)).save(run / "warrior" / "3-e-2-release.png")
            with self.assertRaisesRegex(ValueError, "is black"):
                sheets.build(run)
        with tempfile.TemporaryDirectory() as temporary:
            run = self.run_directory(temporary, ["warrior"])
            (run / "warrior" / "3-e-1-windup.png").unlink()
            with self.assertRaises(OSError):
                sheets.build(run)
        with tempfile.TemporaryDirectory() as temporary:
            run = self.run_directory(temporary, ["warrior"])
            (run / "warrior" / "capture-run.json").write_text(json.dumps({"pass": False, "stills": []}))
            with self.assertRaisesRegex(ValueError, "did not pass"):
                sheets.build(run)
            with self.assertRaisesRegex(ValueError, "no phase capture"):
                sheets.build(run / "sheets")


if __name__ == "__main__":
    unittest.main()
