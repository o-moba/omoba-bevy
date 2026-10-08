"""The skill capture launcher refuses unusable evidence and wrong pictures; the sheets refuse black tiles."""
import contextlib
import copy
import io
import json
import math
from pathlib import Path
import re
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


# A small registry and motion library in the shape of the packaged files: a thrown bolt, a
# warned ray whose row holds a windup, a self cast that draws no accent and a ground slam
# that plays a base clip.
CLIPS = {"hurl_overhand": dict(duration=0.8), "cast": dict(duration=0.5), "spell_prepare": dict(duration=1.0),
         "rally_raise": dict(duration=1.0), "attack": dict(duration=0.6)}
ROWS = {
    "bolt": dict(release="hurl_overhand", motion=dict(rate=2.0, start=0.25), cast=dict(pattern="toss_arc"),
                 body=dict(archetype="traveller"), impact=dict(kind="spark_fork")),
    "ray": dict(release="cast", windup="spell_prepare", cast=dict(pattern="inward_gather"),
                body=dict(archetype="lane"), aux=dict(orb=dict(archetype="orbiter")),
                impact=dict(kind="pierce_through")),
    "calm": dict(release="rally_raise", cast=dict(pattern="none")),
    "slam": dict(release="attack", cast=dict(pattern="ground_ring"), body=dict(archetype="zone"),
                 impact=dict(kind="ring_burst")),
}
SKILLS = ["bolt", "ray", "calm", "slam"]


def expected():
    rows = dict(copy.deepcopy(ROWS),
                **{f"filler_{index}": dict(release="cast") for index in range(launcher.REGISTRY_ROWS - len(ROWS))})
    return dict(rows=rows, fingerprint="00000000000000ab", clips=CLIPS)


def effect(identity, skill, kind, position=(0.0, 0.0), end=(0.0, 1.0), radius=0.5, consumed=0):
    return dict(id=identity, skill=skill, kind=kind, position=list(position), end=list(end), radius=radius,
                consumed_segments=consumed)


def root(identity, body=None, parts=5, lights=0, visible=True):
    return dict(effect_id=identity, visible=visible, body=body, parts=parts + lights, mesh_parts=parts, lights=lights)


def ring(archetype, at=(0.0, 0.0), radius=0.5, engine=1):
    return dict(archetype=archetype, boundary=dict(shape="ring", center=list(at), radius=radius), engine_parts=engine)


def hard_summary():
    """A run whose stills are what `ROWS` describes."""
    captures, skills = [dict(file="0-idle.png", phase="idle", slot=None)], []
    worlds = {
        "bolt": ([effect(2, "bolt", "bolt")], [dict(id=2, skill="bolt", kind="bolt", block="body", part_budget=12)],
                 [root(2, ring("traveller"))]),
        "ray": ([effect(4, "ray", "beam", (0.0, 0.0), (3.0, 4.0), 0.8), effect(9, "ray", "orb", (1.0, 1.0), (1.0, 1.0), 0.65)],
                [dict(id=4, skill="ray", kind="beam", block="body", part_budget=18),
                 dict(id=9, skill="ray", kind="orb", block="aux", part_budget=18)],
                [root(4, dict(archetype="lane", engine_parts=4,
                              boundary=dict(shape="capsule", radius=0.8, **{"from": [0.0, 0.0], "to": [3.0, 4.0]})), 16),
                 root(9, ring("orbiter", (1.0, 1.0), 0.65), 7)]),
        "calm": ([], [], []),
        "slam": ([effect(6, "slam", "field")], [dict(id=6, skill="slam", kind="field", block="body", part_budget=12)],
                 [root(6, ring("zone"), 9)]),
    }
    for slot, skill in enumerate(SKILLS):
        key = launcher.SLOT_KEYS[slot]
        hits = skill in ("bolt", "ray", "slam")
        effects, rows, roots = worlds[skill]
        for order, phase in enumerate(("windup", "release", "impact" if hits else "settled"), start=1):
            still = dict(file=f"{slot + 1}-{key}-{order}-{phase}.png", phase=phase, slot=slot, skill=skill,
                         gate="edge", since_edge_secs=0.02 * order, latest_action=dict(sequence=7, slot=slot),
                         animation=launcher.label(ROWS[skill]["release"]), idle_file="0-idle.png",
                         changed_pixels=900, zoom=0.55, effects=copy.deepcopy(effects),
                         effect_rows=copy.deepcopy(rows), skill_vfx=copy.deepcopy(roots),
                         particles=dict(live=14), effect_visible_parts=20, effect_lights=0,
                         state_visuals=[], hero_states=[dict(hero=1, states=[])], receipt_looks=[])
            captures.append(still)
        captures[-3]["animation"] = "spell_prepare" if skill == "ray" else captures[-3]["animation"]
        third = captures[-1]
        if hits:
            third["receipt"] = dict(id=40 + slot)
            third["receipt_looks"] = [dict(receipt=40 + slot, slot=slot, particles=8,
                                           impact=(ROWS[skill].get("impact") or {}).get("kind"))]
        else:
            third["gate"] = "no_damage"
        skills.append(dict(slot=slot, skill=skill, home="mage", identity=dict(motion=ROWS[skill]["release"]),
                           staging=dict(settles=None if hits else "no_damage"),
                           peaks=dict(accent_of_cast=0 if skill == "calm" else 6, impact_of_one_receipt=8,
                                      live_particles=20, effect_visible_parts=24, effect_lights=0)))
    return dict(visual_mode="Models3d", registry=dict(origin="packaged", fnv64="00000000000000ab",
                                                      profiles=launcher.REGISTRY_ROWS),
                captures=captures, skills=skills)


def still_of(summary, file):
    return next(capture for capture in summary["captures"] if capture["file"] == file)


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

    def test_flight_look_ends_with_the_stills_its_basic_record_names(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            summary = phase_summary()
            still = lambda file, phase: dict(file=file, phase=phase, slot=255, skill="basic", mean_pixel=91.0)
            summary["captures"] += [still("5-basic-flight.png", "flight"), still("5-basic-impact.png", "impact")]
            touch_stills(directory, summary)
            # A still nobody recorded, and a record without its still, are both refused.
            both = ["5-basic-flight.png", "5-basic-impact.png"]
            self.assertEqual(launcher.phase_problems(summary, directory),
                             [f"basic attack stills {both} do not match the basic attack record"])
            summary["basic"] = dict(slot=255, stills=both)
            self.assertEqual(launcher.phase_problems(summary, directory), [])
            summary["captures"].pop()
            self.assertEqual(launcher.phase_problems(summary, directory),
                             ["basic attack stills ['5-basic-flight.png'] do not match the basic attack record"])
            # The rocket round of a repeater has a record of its own inside the first.
            rockets = ["6-rockets-flight.png", "6-rockets-impact.png"]
            summary["captures"] += [still("5-basic-impact.png", "impact"), still(rockets[0], "flight"),
                                    still(rockets[1], "impact")]
            touch_stills(directory, summary)
            self.assertEqual(launcher.phase_problems(summary, directory),
                             [f"basic attack stills {both + rockets} do not match the basic attack record"])
            summary["basic"]["rockets"] = dict(slot=255, weapon_mode="Rockets", stills=rockets)
            self.assertEqual(launcher.phase_problems(summary, directory), [])
            # A run without a flight look has neither a record nor a still.
            del summary["captures"][-4:]
            summary["basic"] = None
            self.assertEqual(launcher.phase_problems(summary, directory), [])

    def test_the_rounds_of_a_basic_attack_are_drawn_from_their_rows_and_profiles(self):
        expected = dict(
            basic_attacks=dict(wildspark=dict(impact=dict(kind="splinter"),
                                              rockets=dict(impact=dict(kind="ember_puff"))),
                               frostguard=dict(impact=dict(kind="facet_pop"))),
            projectiles=dict(wild_bullet=dict(form="dart"), wild_rocket=dict(form="tumbler", model={})))

        def shot(profile, body, style):
            return dict(action_slot=255, profile=profile, body=body, style=style)

        def run():
            flight = lambda file, shots: dict(file=file, phase="flight", slot=255, projectiles=shots)
            hit = lambda file, receipt, kind: dict(
                file=file, phase="impact", slot=255, receipt=dict(id=receipt),
                receipt_looks=[dict(receipt=receipt, impact=kind, particles=6)])
            stills = [flight("5-basic-flight.png", [shot("wild_bullet", "dart+block", "bullet")]),
                      hit("5-basic-impact.png", 4, "splinter"),
                      flight("6-rockets-flight.png", [shot("wild_rocket", "tumbler+block", "rocket")]),
                      hit("6-rockets-impact.png", 5, "ember_puff")]
            summary = {"class": "wildspark", "basic": dict(
                stills=["5-basic-flight.png", "5-basic-impact.png"],
                rockets=dict(weapon_mode="Rockets", stills=["6-rockets-flight.png", "6-rockets-impact.png"]))}
            return summary, stills

        problems = lambda summary, stills, flat=False: launcher.basic_problems(summary, expected, stills, flat)
        self.assertEqual(problems(*run()), [])
        # No basic record, no claim.
        self.assertEqual(problems({"class": "wildspark", "basic": None}, []), [])
        # The rocket round shows the form of its profile, not the model the profile carries.
        summary, stills = run()
        stills[2]["projectiles"][0]["body"] = "shape"
        self.assertEqual(problems(summary, stills),
                         ["6-rockets-flight.png: wild_rocket is drawn as shape, its profile says the form tumbler"])
        # The flat view draws its own shapes.
        self.assertEqual(problems(summary, stills, flat=True), [])
        # It is fired in rocket mode and replicated as a rocket.
        summary, stills = run()
        summary["basic"]["rockets"]["weapon_mode"] = "Repeater"
        stills[2]["projectiles"][0]["style"] = "bullet"
        self.assertEqual(problems(summary, stills),
                         ["the rocket round was captured in weapon mode Repeater",
                          "6-rockets-flight.png: the rocket round threw a projectile of style bullet"])
        # Each round is hit with the recipe of its own row.
        summary, stills = run()
        stills[3]["receipt_looks"][0]["impact"] = "splinter"
        self.assertEqual(problems(summary, stills),
                         ["6-rockets-impact.png: the hit is drawn as splinter, the row of the rockets round "
                          "says ember_puff"])
        summary, stills = run()
        stills[1]["receipt_looks"] = []
        self.assertEqual(problems(summary, stills), ["5-basic-impact.png: the receipt of the still was not drawn"])
        # A round without a still of its hit is not evidence of it.
        summary, stills = run()
        summary["basic"]["stills"].pop()
        self.assertEqual(problems(summary, stills), ["the basic round has no still of its hit"])
        # A melee core throws nothing: its round is the hit alone.
        melee = {"class": "frostguard", "basic": dict(stills=["5-basic-impact.png"])}
        hit = dict(file="5-basic-impact.png", phase="impact", slot=255, receipt=dict(id=9),
                   receipt_looks=[dict(receipt=9, impact="facet_pop", particles=6)])
        self.assertEqual(problems(melee, [hit]), [])
        # A row that names no impact keeps the burst of the wire style, which names no recipe.
        hit["receipt_looks"][0]["impact"] = None
        self.assertEqual(problems({"class": "warrior", "basic": melee["basic"]}, [hit]), [])

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
            first = launcher.merge_manifest(path, dict(phases=True), dict(mage=dict(hero="mage", **{"pass": True})))
            self.assertTrue(first["pass"])
            second = launcher.merge_manifest(path, dict(phases=True),
                                             dict(warden=dict(hero="warden", **{"pass": False})))
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
                    (["--mixed-recipe"], "need --phases"),
                    (["--phases", "--mixed-recipe", "--hero", "mage"], "names its own hero"),
                    (["--phases", "--avatar", "nobody"], "Unknown avatar"),
                    (["--phases", "--release-at", "3"], "expected `contact` or 0..2 seconds"),
                    (["--phases", "--release-at", "soon"], "invalid release_time value"),
            ):
                code, stderr = run_main(base + extra)
                self.assertEqual(code, 2, extra)
                self.assertIn(message, stderr)
            self.assertFalse((temporary / "out").exists())


class HardAssertionTests(unittest.TestCase):
    """`hard_problems` names every still that is not what the registry of the run describes."""

    def problems(self, edit=None, expectation=None):
        summary = hard_summary()
        if edit:
            edit(summary)
        return launcher.hard_problems(summary, expectation or expected())

    def assert_one(self, edit, text, expectation=None):
        found = self.problems(edit, expectation)
        self.assertEqual(len(found), 1, found)
        self.assertIn(text, found[0])

    def test_a_run_that_matches_its_registry_has_no_problem(self):
        self.assertEqual(self.problems(), [])

    def test_fingerprint_is_fnv_1a_of_the_registry_file(self):
        self.assertEqual(launcher.fnv64(b""), 0xcbf29ce484222325)
        self.assertEqual(launcher.fnv64(b"a"), 0xaf63dc4c8601ec8c)
        self.assertEqual(launcher.fnv64(b"foobar"), 0x85944171f73967e8)
        packaged = launcher.expectations(ROOT / "client/assets")
        self.assertEqual(len(packaged["rows"]), launcher.REGISTRY_ROWS)
        self.assertRegex(packaged["fingerprint"], "^[0-9a-f]{16}$")
        self.assertEqual(int(packaged["fingerprint"], 16),
                         launcher.fnv64((ROOT / "client/assets/config/skills.skillfx").read_bytes()))
        # Every clip a packaged row names is in the library the labels are read against.
        for row in packaged["rows"].values():
            for clip in filter(None, (row["release"], row.get("windup"))):
                self.assertIn(clip, packaged["clips"])
        # The compiled-in copy, another file and a short registry are all refused.
        self.assert_one(lambda s: s["registry"].update(origin="embedded"), "embedded registry")
        self.assert_one(lambda s: s["registry"].update(fnv64="00000000000000ac"), "loaded registry 00000000000000ac")
        self.assert_one(lambda s: s["registry"].update(profiles=67), "67 rows")
        short = expected()
        del short["rows"]["filler_0"]
        self.assert_one(None, "expected 68", short)

    def test_the_body_plays_the_clip_of_the_row_and_not_a_fallback(self):
        rows = expected()["rows"]
        release = dict(phase="release", slot=0, since_edge_secs=0.1, latest_action=dict(slot=0))
        self.assertEqual(launcher.expected_label(rows["bolt"], CLIPS, release), "hurl_overhand")
        self.assertEqual(launcher.expected_label(rows["slam"], CLIPS, release), "Attack")
        # The clip of `bolt` runs for 0.8 * (1 - 0.25) / 2 = 0.3 s: after it nothing is known.
        self.assertEqual(launcher.expected_label(rows["bolt"], CLIPS, dict(release, since_edge_secs=0.24)),
                         "hurl_overhand")
        self.assertIsNone(launcher.expected_label(rows["bolt"], CLIPS, dict(release, since_edge_secs=0.26)))
        # Another accepted action takes the body of a row that holds nothing.
        self.assertIsNone(launcher.expected_label(rows["bolt"], CLIPS, dict(release, latest_action=dict(slot=255))))
        # A held windup stands through it, and the release follows the telegraph however late.
        late = dict(release, phase="windup", since_edge_secs=3.0, latest_action=dict(slot=255))
        self.assertEqual(launcher.expected_label(rows["ray"], CLIPS, late), "spell_prepare")
        self.assertEqual(launcher.expected_label(rows["ray"], CLIPS, dict(late, phase="release")), "Cast")
        self.assertIsNone(launcher.expected_label(rows["bolt"], CLIPS, dict(release, phase="impact")))
        # A clip the rig does not have falls back to the generic cast: that is a failure.
        self.assert_one(lambda s: still_of(s, "1-q-2-release.png").update(animation="Cast"),
                        "plays Cast, the row of bolt says hurl_overhand")
        self.assert_one(lambda s: still_of(s, "2-w-1-windup.png").update(animation="Cast"),
                        "plays Cast, the row of ray says spell_prepare")
        self.assert_one(lambda s: still_of(s, "2-w-2-release.png").update(animation="spell_prepare"),
                        "says Cast")

    def test_every_effect_is_drawn_as_its_row_says_over_the_replicated_geometry(self):
        def body(summary, file="2-w-2-release.png", index=0):
            return still_of(summary, file)["skill_vfx"][index]

        for edit, text in (
                (lambda s: still_of(s, "1-q-1-windup.png").update(skill_vfx=[]), "bolt bolt: no visible root"),
                (lambda s: body(s, "1-q-1-windup.png").update(visible=False), "bolt bolt: no visible root"),
                (lambda s: body(s, "1-q-1-windup.png").update(body=None), "bolt bolt: its root is not the body of its row"),
                (lambda s: body(s, "1-q-1-windup.png")["body"].update(archetype="zone"),
                 "archetype zone, the row says traveller"),
                (lambda s: body(s, "1-q-1-windup.png")["body"]["boundary"].update(radius=0.6), "ring is not"),
                (lambda s: body(s, "1-q-1-windup.png")["body"]["boundary"].update(center=[0.0, 0.5]), "ring is not"),
                (lambda s: body(s, "1-q-1-windup.png")["body"].update(engine_parts=2), "2 boundary parts, expected 1"),
                (lambda s: body(s, "1-q-1-windup.png").update(mesh_parts=13), "13 mesh parts, budget 12"),
                (lambda s: body(s)["body"]["boundary"].update(to=[3.0, 4.5]), "not the replicated segment"),
                (lambda s: body(s)["body"]["boundary"].update(radius=1.0), "not as wide"),
                (lambda s: body(s)["body"].update(engine_parts=2), "2 boundary parts, expected 4"),
                (lambda s: body(s)["body"].update(boundary=dict(shape="ring", center=[0.0, 0.0], radius=0.8)),
                 "a ring boundary on a lane"),
                (lambda s: body(s, index=1)["body"].update(archetype="zone"), "ray orb: archetype zone"),
                (lambda s: still_of(s, "4-r-3-impact.png").update(skill_vfx=[]), "slam field: no visible root"),
                (lambda s: body(s, "4-r-1-windup.png").update(mesh_parts=99), "99 mesh parts, budget 12"),
        ):
            self.assert_one(edit, text)
        # The effect of a first cast is drawn by the `body` of its row: a row without one fails.
        bodiless = expected()
        del bodiless["rows"]["slam"]["body"]
        self.assertEqual(self.problems(None, bodiless),
                         [f"4-r-{order}-{phase}.png: slam field: its row has no body"
                          for order, phase in enumerate(("windup", "release", "impact"), start=1)])
        # The orb is drawn by the `aux` block of the row; a kind the row gives no body draws nothing.
        def no_aux(summary):
            for still in summary["captures"]:
                still["skill_vfx"] = [r for r in still.get("skill_vfx", []) if r["effect_id"] != 9]
        self.assertEqual(len(self.problems(no_aux)), 3)
        bare = expected()
        del bare["rows"]["ray"]["aux"]
        self.assertEqual(self.problems(no_aux, bare), [])
        # The flat view has no clip and no 3D body: a body there is the defect.
        def flat(summary):
            summary["visual_mode"] = "Sprite2d"
            for still in summary["captures"]:
                still.update(skill_vfx=[], animation="Waiting for animation binding")
        self.assertEqual(self.problems(flat), [])
        def flat_with_body(summary):
            flat(summary)
            still_of(summary, "1-q-2-release.png")["skill_vfx"] = [dict(root(2, ring("traveller")), name="SkillVfx-bolt-2")]
        self.assert_one(flat_with_body, "1-q-2-release.png: SkillVfx-bolt-2: a 3D body in the flat view")

    def test_boundaries_of_every_shape_follow_the_replicated_fields(self):
        cases = [
            (dict(archetype="sector", engine_parts=8,
                  boundary=dict(shape="sector", apex=[1.0, 1.0], axis=[0.6, 0.8], radius=5.0, half_angle=math.acos(0.6))),
             effect(1, "x", "beam_warning", (1.0, 1.0), (4.0, 5.0), 3.0)),
            (dict(archetype="wall", engine_parts=1,
                  boundary=dict(shape="segment", **{"from": [-2.0, 1.0], "to": [2.0, 1.0]})),
             effect(1, "x", "shield_wall", (0.0, 0.0), (0.0, 1.0), 2.0)),
            (dict(archetype="cage", engine_parts=6, boundary=dict(shape="pentagon", center=[0.0, 0.0], radius=5.0)),
             effect(1, "x", "cage", (0.0, 0.0), (0.0, 0.0), 5.0, consumed=0b00101)),
            (dict(archetype="traveller", engine_parts=0, boundary=dict(shape="none")), effect(1, "x", "soul")),
            (dict(archetype="lane", engine_parts=2,
                  boundary=dict(shape="lane", half_width=0.5, **{"from": [0.0, 0.0], "to": [0.0, 1.0]})),
             effect(1, "x", "beam")),
        ]
        for body, seen in cases:
            self.assertEqual(launcher.boundary_problems(body, seen), [], body["archetype"])
        sector, wall, cage = (copy.deepcopy(case) for case in cases[:3])
        sector[0]["boundary"]["half_angle"] = 0.5
        self.assertIn("half angle", launcher.boundary_problems(*sector)[0])
        sector[0]["boundary"].update(half_angle=math.acos(0.6), axis=[0.8, 0.6])
        self.assertIn("does not point", launcher.boundary_problems(*sector)[0])
        wall[0]["boundary"]["to"] = [2.0, 0.0]
        self.assertIn("wall bar", launcher.boundary_problems(*wall)[0])
        cage[1]["consumed_segments"] = 0
        self.assertEqual(launcher.boundary_problems(*cage), ["6 boundary parts, expected 10"])

    def test_particle_part_and_light_budgets_hold_in_every_still_and_between_them(self):
        release = "1-q-2-release.png"
        for edit, text in (
                (lambda s: s["skills"][0]["peaks"].update(accent_of_cast=0), "0 accent particles of the cast"),
                (lambda s: s["skills"][0]["peaks"].update(accent_of_cast=9), "9 accent particles of the cast"),
                (lambda s: s["skills"][2]["peaks"].update(accent_of_cast=1), "calm: 1 accent particles, its row draws none"),
                (lambda s: s["skills"][0]["peaks"].update(impact_of_one_receipt=13), "13 impact particles"),
                (lambda s: s["skills"][0]["peaks"].update(live_particles=257), "bolt: 257 live particles at once"),
                (lambda s: s["skills"][0]["peaks"].update(effect_visible_parts=401), "bolt: 401 visible effect parts"),
                (lambda s: s["skills"][0]["peaks"].update(effect_lights=3), "bolt: 3 effect lights at once"),
                (lambda s: still_of(s, release)["particles"].update(live=257), f"{release}: 257 live particles"),
                (lambda s: still_of(s, release).update(effect_visible_parts=401), "401 visible effect parts"),
                (lambda s: still_of(s, release).update(effect_lights=3), "3 effect lights"),
        ):
            self.assert_one(edit, text)
        # Every row has a `cast`; a run that saw no accent of one that draws is no evidence of it.
        self.assert_one(lambda s: s["skills"][3]["peaks"].update(accent_of_cast=None),
                        "slam: None accent particles of the cast")

    def test_a_hit_is_drawn_by_the_recipe_of_its_row(self):
        impact = "1-q-3-impact.png"
        self.assert_one(lambda s: still_of(s, impact)["receipt_looks"][0].update(impact="ring_burst"),
                        "drawn as ring_burst, the row of bolt says spark_fork")
        self.assert_one(lambda s: still_of(s, impact)["receipt_looks"][0].update(impact=None),
                        "drawn as None, the row of bolt says spark_fork")
        self.assert_one(lambda s: still_of(s, impact)["receipt_looks"][0].update(particles=13), "13 impact particles")
        self.assert_one(lambda s: still_of(s, impact).update(receipt_looks=[]), "was not drawn")
        self.assert_one(lambda s: still_of(s, "4-r-3-impact.png")["receipt_looks"][0].update(impact="blast"),
                        "drawn as blast, the row of slam says ring_burst")

    def test_the_third_still_is_an_impact_exactly_when_the_staged_cast_has_to_hit(self):
        def settle(summary):
            still_of(summary, "1-q-3-impact.png").update(phase="settled", gate="no_receipt_in_probe")
        self.assert_one(settle, "bolt: the staged cast has to hit, its third still is settled (no_receipt_in_probe)")
        self.assert_one(lambda s: still_of(s, "3-e-3-settled.png").update(gate="no_receipt_in_probe"),
                        "calm: the staged cast yields no receipt (no_damage)")
        self.assert_one(lambda s: s["skills"][0]["staging"].update(settles="mark_not_detonated"),
                        "bolt: the staged cast yields no receipt (mark_not_detonated), its third still is impact")

    def test_a_cast_changes_the_stage(self):
        self.assert_one(lambda s: still_of(s, "3-e-2-release.png").update(changed_pixels=0),
                        "3-e-2-release.png: the stage does not differ from 0-idle.png")
        self.assert_one(lambda s: still_of(s, "1-q-1-windup.png").pop("changed_pixels"), "does not differ")
        # Nothing may be left of a self cast when it has settled.
        self.assertEqual(self.problems(lambda s: still_of(s, "3-e-3-settled.png").update(changed_pixels=0)), [])

    def test_state_visuals_follow_the_replicated_flags(self):
        def stunned(parts=3, visible=3):
            return dict(state_visuals=[dict(hero=2, state="stunned", parts=parts, visible_parts=visible)],
                        hero_states=[dict(hero=1, states=[]), dict(hero=2, states=["stunned", "slowed"])])

        still = "1-q-3-impact.png"
        self.assertEqual(self.problems(lambda s: still_of(s, still).update(stunned())), [])
        self.assert_one(lambda s: still_of(s, still).update(stunned(), state_visuals=[]),
                        "hero 2 reports ['stunned', 'slowed'] and shows None")
        self.assert_one(lambda s: still_of(s, still).update(stunned(parts=5, visible=5)), "stunned has 5 parts")
        self.assert_one(lambda s: still_of(s, still).update(stunned(visible=2)), "draws 2 of 3 parts")
        # The visual is the highest state of the flags, and the flags come highest first.
        found = self.problems(lambda s: still_of(s, still).update(
            stunned(), hero_states=[dict(hero=2, states=["slowed", "stunned"])]))
        self.assertEqual(len(found), 2, found)
        self.assertIn("its flags report ['slowed', 'stunned']", found[0])
        self.assertIn("out of rank", found[1])
        self.assert_one(lambda s: still_of(s, still).update(
            stunned(), hero_states=[dict(hero=1, states=[]), dict(hero=2, states=["slowed"])]),
            "its flags report ['slowed']")
        # A visual without a state behind it is refused twice: it shows a state nobody reported.
        found = self.problems(lambda s: still_of(s, still).update(
            stunned(), hero_states=[dict(hero=1, states=[]), dict(hero=2, states=[])]))
        self.assertEqual(len(found), 2, found)
        # The flat view draws its states with gizmos: a mesh there is a 3D body in 2D.
        def flat(summary):
            summary["visual_mode"] = "Sprite2d"
            for capture in summary["captures"]:
                capture["skill_vfx"] = []
            still_of(summary, still).update(stunned())
        self.assert_one(flat, "a state mesh on hero 2 in the flat view")
        self.assertEqual(list(launcher.STATE_PARTS), launcher.STATE_RANK)
        status = (ROOT / "client/src/skill_presentation/status.rs").read_text()
        for state in launcher.STATE_RANK:
            self.assertIn(f'"{state}"', status)

    def test_each_slot_reports_what_it_shows_next_to_its_row(self):
        reports = launcher.slot_reports(hard_summary(), expected())
        self.assertEqual([report["skill"] for report in reports], SKILLS)
        self.assertEqual(reports[0]["shown"], dict(clip="hurl_overhand", windup_clip="hurl_overhand",
                                                   body=["traveller"], impact="spark_fork"))
        self.assertEqual(reports[0]["profile"], dict(clip="hurl_overhand", windup_clip=None, body="traveller",
                                                     impact="spark_fork"))
        self.assertEqual(reports[1]["shown"], dict(clip="Cast", windup_clip="spell_prepare", body=["lane"],
                                                   impact="pierce_through"))
        self.assertEqual(reports[1]["profile"]["windup_clip"], "spell_prepare")
        self.assertEqual([report["matches"] for report in reports[:2]], [dict(clip=True, body=True, impact=True)] * 2)
        # A self cast has no body and no hit.
        self.assertEqual(reports[2]["shown"]["body"], [])
        self.assertEqual(reports[2]["matches"], dict(clip=True, body=True, impact=None))
        self.assertEqual(reports[3]["shown"], dict(clip="Attack", windup_clip="Attack", body=["zone"],
                                                   impact="ring_burst"))
        self.assertEqual(reports[3]["matches"], dict(clip=True, body=True, impact=True))
        summary = hard_summary()
        still_of(summary, "1-q-3-impact.png")["receipt_looks"][0]["impact"] = "blast"
        for still in summary["captures"][1:4]:
            still["skill_vfx"][0]["body"]["archetype"] = "zone"
            still["animation"] = "Cast"
        self.assertEqual(launcher.slot_reports(summary, expected())[0]["matches"],
                         dict(clip=False, body=False, impact=False))

    def test_the_mixed_recipe_takes_four_skills_of_four_other_classes_off_their_buttons(self):
        catalog = {skill["id"]: skill for skill in
                   json.loads((ROOT / "shared/assets/catalog/skills.json").read_text())["skills"]}
        rows = launcher.expectations(ROOT / "client/assets")["rows"]
        recipe = launcher.MIXED_RECIPE
        self.assertEqual(len(recipe["skills"]), len(launcher.SLOT_KEYS))
        homes = [rows[skill]["home"] for skill in recipe["skills"]]
        self.assertEqual(len(set(homes)), 4)
        self.assertNotIn(recipe["hero"], homes)
        for slot, skill in enumerate(recipe["skills"]):
            self.assertNotEqual(catalog[skill]["slot"], launcher.SLOT_KEYS[slot], skill)
        # Four different bodies, all of which the staged target can be hit by.
        self.assertEqual(len({rows[skill]["body"]["archetype"] for skill in recipe["skills"]}), 4)
        self.assertTrue(all(rows[skill].get("impact") for skill in recipe["skills"]))
        self.assertNotIn(launcher.MIXED_NAME, launcher.HEROES)

    def test_a_slot_captured_from_farther_away_has_the_idle_still_of_its_view(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            summary = phase_summary()
            summary["skills"][2]["framing"] = dict(zoom=0.8, widened=True, idle_file="3-e-0-idle.png")
            touch_stills(directory, summary)
            self.assertEqual(launcher.phase_problems(summary, directory),
                             ["None: slot idle stills [] do not match its framing"])
            summary["captures"].append(dict(file="3-e-0-idle.png", phase="slot_idle", slot=2, mean_pixel=80.0))
            touch_stills(directory, summary)
            self.assertEqual(launcher.phase_problems(summary, directory), [])
            summary["skills"][2]["framing"] = dict(zoom=0.55, widened=False, idle_file="0-idle.png")
            self.assertEqual(launcher.phase_problems(summary, directory),
                             ["None: slot idle stills ['3-e-0-idle.png'] do not match its framing"])

    def test_the_harness_and_the_scripts_measure_the_same_stage(self):
        harness = (ROOT / "client/src/qa/standard_kits_qa.rs").read_text()
        crop = re.search(r"const STAGE_CROP: \[u32; 4\] = \[(\d+), (\d+), (\d+), (\d+)\];", harness)
        level = re.search(r"const CHANGED_LEVEL: u8 = (\d+);", harness)
        zoom = re.search(r"pub const CAMERA_MIN_ZOOM: f32 = ([0-9.]+);", (ROOT / "client/src/camera.rs").read_text())
        self.assertTrue(crop and level and zoom)
        if sheets:
            self.assertEqual(tuple(int(edge) for edge in crop.groups()), sheets.CROP)
            self.assertEqual(int(level.group(1)), sheets.CHANGED_LEVEL)
            self.assertEqual(float(zoom.group(1)), sheets.CLOSE_ZOOM)


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

    def test_look_alike_report_measures_distance_and_energy_and_refuses_equal_tiles(self):
        def frame(*boxes):
            """A grey stage with coloured boxes, as offsets inside the crop."""
            image = Image.new("RGB", (1280, 720), (5, 5, 200))
            image.paste((90, 90, 90), sheets.CROP)
            for (left, top, width, height), colour in boxes:
                x, y = sheets.CROP[0] + left, sheets.CROP[1] + top
                image.paste(colour, (x, y, x + width, y + height))
            return image

        def run_of(directory, looks, hero="warrior", skills="{key}_skill"):
            """One class whose release still of each slot is the idle stage plus the given boxes."""
            run = Path(directory)
            (run / hero).mkdir(parents=True)
            summary = phase_summary()
            frame().save(run / hero / "0-idle.png")
            for capture in summary["captures"][2:]:
                capture["skill"] = skills.format(key=launcher.SLOT_KEYS[capture["slot"]])
                boxes = looks[capture["slot"]] if capture["phase"] == "release" else []
                frame(*boxes).save(run / hero / capture["file"])
            stills = [capture for capture in summary["captures"] if capture.get("phase")]
            (run / hero / "capture-run.json").write_text(json.dumps({"pass": True, "stills": stills}))
            return run

        red, pale = (250, 40, 40), (110, 110, 110)
        looks = [[((100, 100, 200, 100), red)], [((500, 50, 100, 300), red)], [((300, 300, 60, 60), red)],
                 [((40, 380, 400, 40), red)]]
        with tempfile.TemporaryDirectory() as temporary:
            run = run_of(Path(temporary) / "new", looks)
            # The earlier run: the first skill looked the same, the second had a larger shape
            # and the third only a faint one.
            before = run_of(Path(temporary) / "before", [looks[0], [((450, 50, 300, 300), red)],
                                                         [((300, 300, 60, 60), pale)], []])
            index = sheets.build(run, before)
            report = json.loads((run / "sheets" / "lookalike.json").read_text())
            self.assertEqual(index["lookalike"], "lookalike.json")
            first, second, third, fourth = (report["skills"][f"warrior/{key}_skill"] for key in "qwer")
            # Energy is the changed area: the box, whatever its colour, once it clears the level.
            close = dict(view="close", baseline_view="close")
            self.assertEqual(first["energy"], dict(release=dict(new=20000, baseline=20000, **close),
                                                   impact=dict(new=0, baseline=0, **close)))
            self.assertEqual(second["energy"]["release"], dict(new=30000, baseline=90000, **close))
            self.assertEqual(third["energy"]["release"], dict(new=3600, baseline=0, **close))
            self.assertEqual(fourth["energy"]["release"], dict(new=16000, baseline=0, **close))
            self.assertEqual(report["below_baseline_energy"],
                             ["warrior/w_skill release: 30000 changed pixels, baseline 90000"])
            # Four different tiles; the one that did not change is an error, nothing else is.
            self.assertGreater(report["min_distance_in_class"]["warrior"], 0)
            self.assertEqual(report["min_distance_in_run"], report["min_distance_in_class"]["warrior"])
            self.assertEqual(first["baseline_distance"], 0)
            self.assertGreater(second["baseline_distance"], 0)
            self.assertEqual(report["zero_distance"], ["warrior/q_skill has the release tile of the baseline"])
            self.assertEqual(index["zero_distance"], report["zero_distance"])
            self.assertEqual(first["nearest_in_class"]["distance"], first["nearest_in_run"]["distance"])
            # The six pairs of the class, nearest first.
            listed = report["closest_in_class"]
            self.assertEqual(len(listed), 6)
            self.assertEqual(listed[0]["distance"], report["min_distance_in_class"]["warrior"])
            self.assertEqual([pair["distance"] for pair in listed], sorted(pair["distance"] for pair in listed))
            self.assertTrue(all(key.startswith("warrior/") for pair in listed for key in pair["skills"]))
            table = (run / "sheets" / "lookalike.md").read_text()
            self.assertIn("| warrior/w_skill |", table)
            self.assertIn("30000 (90000, x0.33)", table)
            self.assertIn("32 x 18 bits", table)
            self.assertIn(f"Nearest pairs of one class: {listed[0]['distance']} warrior/", table)
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaisesRegex(SystemExit, "release tile of the baseline"):
                sheets.main([str(run), "--baseline", str(before)])
            # Without a baseline the same run is fine.
            with contextlib.redirect_stdout(io.StringIO()):
                sheets.main([str(run)])
        with tempfile.TemporaryDirectory() as temporary:
            # Two skills of one class with the same release tile are an error by themselves.
            run = run_of(temporary, [looks[0], looks[0], looks[2], looks[3]])
            self.assertEqual(sheets.build(run)["zero_distance"],
                             ["warrior/q_skill and warrior/w_skill have the same release tile"])
        with tempfile.TemporaryDirectory() as temporary:
            # Two classes with one look on Q: equal in the run, which is an error, and no pair
            # of either class. The nearest pairs are listed class by class.
            run = run_of(temporary, looks)
            run_of(temporary, [looks[0], [((600, 20, 80, 80), red)], [((20, 200, 300, 30), red)],
                               [((650, 300, 100, 100), red)]], "mage", "mage_{key}")
            report = sheets.lookalike(run, ["warrior", "mage"])
            first = report["skills"]["warrior/q_skill"]
            self.assertEqual(first["nearest_in_run"], dict(skill="mage/mage_q", distance=0))
            self.assertTrue(first["nearest_in_class"]["skill"].startswith("warrior/"))
            self.assertGreater(first["nearest_in_class"]["distance"], 0)
            self.assertEqual(report["zero_distance"], ["warrior/q_skill and mage/mage_q have the same release tile"])
            self.assertEqual(report["min_distance_in_run"], 0)
            self.assertTrue(all(nearest > 0 for nearest in report["min_distance_in_class"].values()))
            self.assertEqual(len(report["closest_in_class"]), sheets.CLOSEST_LISTED)
            for pair in report["closest_in_class"]:
                self.assertEqual(len({key.split("/")[0] for key in pair["skills"]}), 1, pair)
            # One skill captured in two folders, as a mixed recipe does, is meant to look the same.
            run_of(temporary, looks, launcher.MIXED_NAME)
            self.assertEqual(sheets.folders(run), ["warrior", "mage", launcher.MIXED_NAME])
            report = sheets.lookalike(run, ["warrior", launcher.MIXED_NAME])
            self.assertEqual(report["zero_distance"], [])
            self.assertGreater(report["min_distance_in_run"], 0)
        with tempfile.TemporaryDirectory() as temporary:
            # A still taken from farther away is measured against the idle still of its view
            # and its energy is not compared with a closer baseline.
            run = run_of(Path(temporary) / "new", looks)
            before = run_of(Path(temporary) / "before",
                            [looks[0], [((450, 50, 300, 300), red)], looks[2], looks[3]])
            record = json.loads((run / "warrior" / "capture-run.json").read_text())
            frame(((0, 0, 50, 50), red)).save(run / "warrior" / "2-w-0-idle.png")
            for still in record["stills"]:
                if still["slot"] == 1:
                    still.update(zoom=0.8, idle_file="2-w-0-idle.png")
            (run / "warrior" / "capture-run.json").write_text(json.dumps(record))
            report = sheets.lookalike(run, ["warrior"], before)
            wide = report["skills"]["warrior/w_skill"]["energy"]["release"]
            self.assertEqual((wide["new"], wide["baseline"], wide["view"]), (30000 + 2500, 90000, "zoom 0.80"))
            self.assertEqual(report["below_baseline_energy"], [])
            self.assertIn("32500 (90000) [zoom 0.80]", sheets.lookalike_table(report))
            # So is a still of a cast that carried the hero, and the camera, off its spot.
            for still in record["stills"]:
                if still["slot"] == 1:
                    still.update(zoom=0.55, idle_file="0-idle.png", hero_from_home=6.0)
            (run / "warrior" / "capture-run.json").write_text(json.dumps(record))
            report = sheets.lookalike(run, ["warrior"], before)
            self.assertEqual(report["skills"]["warrior/w_skill"]["energy"]["release"]["view"], "moved")
            self.assertEqual(report["below_baseline_energy"], [])
            self.assertIn("30000 (90000) [moved]", sheets.lookalike_table(report))
            # Seen from the closest view on its spot, the smaller shape is listed.
            for still in record["stills"]:
                still.update(hero_from_home=0.0)
            (run / "warrior" / "capture-run.json").write_text(json.dumps(record))
            self.assertEqual(sheets.lookalike(run, ["warrior"], before)["below_baseline_energy"],
                             ["warrior/w_skill release: 30000 changed pixels, baseline 90000"])

    def test_difference_hash_and_changed_area(self):
        grey = Image.new("RGB", (800, 450), (90, 90, 90))
        marked = grey.copy()
        marked.paste((250, 250, 250), (200, 100, 400, 200))
        self.assertEqual(sheets.distance(sheets.difference_hash(grey), sheets.difference_hash(grey)), 0)
        self.assertGreater(sheets.distance(sheets.difference_hash(grey), sheets.difference_hash(marked)), 0)
        self.assertEqual(sheets.changed_pixels(marked, grey), 200 * 100)
        # One channel just over the level counts; all channels at the level do not.
        faint, at_level = grey.copy(), grey.copy()
        faint.paste((90, 90 + sheets.CHANGED_LEVEL + 1, 90), (0, 0, 10, 10))
        at_level.paste((90 + sheets.CHANGED_LEVEL,) * 3, (0, 0, 10, 10))
        self.assertEqual(sheets.changed_pixels(faint, grey), 100)
        self.assertEqual(sheets.changed_pixels(at_level, grey), 0)

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
