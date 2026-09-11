"""Test evidence rejection, not game-implementation arithmetic."""
import copy
import unittest

from capture_targeting import IMAGES, verify_targeting


def fixture(mobile=False):
    names = ["ready", "basic_start", "basic_damage", "basic_stop", "cancellation_verified"]
    names += (["drag_preview", "drag_release", "touch_canceled", "empty_aim_released",
               "independent_movement"] if mobile else
              ["left_select", "selection_no_attack", "right_attack", "ground_cancel",
               "alt_click_blocked", "ui_click_blocked"])
    events = [dict(event=name, snapshot_tick=10, elapsed_seconds=.1) for name in names]
    for event in events:
        if event["event"] == "basic_start":
            event.update(snapshot_tick=20, elapsed_seconds=1.)
        elif event["event"] == "basic_stop":
            event.update(snapshot_tick=30, elapsed_seconds=3.)
        elif event["event"] == "cancellation_verified":
            event.update(snapshot_tick=60, elapsed_seconds=5.)
    summary = dict(scenario="targeting", pass_=True, scripted_input=True, synthetic_damage=False,
                   manual_interaction_verified=False,
                   setup_fixture="development server target placement and ambient AI disabled",
                   player_id=1, target_id=2, other_target_ids=[3], events=events,
                   skill_cooldowns=[0, 0, 0, 0],
                   captures=[dict(stage=i, file=name, mobile_controls=mobile,
                                  selected_target=dict(kind="player", id=2) if (i == 2 or (i == 1 and not mobile)) else None,
                                  nodes=[dict(name="LockedTargetIndicator", visible=True,
                                              size=[48, 48] if (i == 2 or (i == 1 and not mobile)) else [0, 0])])
                             for i, name in enumerate(IMAGES)],
                   commands=[dict(kind="basic_attack", target=dict(kind="player", id=2),
                                  elapsed_seconds=t) for t in ([1.2] if mobile else [1.2, 2.2])])
    summary["pass"] = summary.pop("pass_")
    snapshots = []
    for tick in range(10, 61):
        hits = int(tick >= 22) + (0 if mobile else int(tick >= 27))
        caster = dict(id=1, hp=100, mana=100, action_sequence=hits,
                      action_kind="attack" if hits else "idle", action_slot=255 if hits else 0,
                      basic_attack_request_id=hits, basic_attack_cooldown_secs=1.,
                      basic_attack_remaining_secs=.5 if hits else 0.)
        snapshots.append(dict(snapshot_tick=tick, players=[caster, dict(id=2, hp=100-hits*10),
                                                          dict(id=3, hp=100)]))
    return summary, snapshots


class TargetingEvidenceTests(unittest.TestCase):
    def test_desktop_and_mobile_require_real_server_readback(self):
        for mobile in (False, True):
            summary, snapshots = fixture(mobile)
            self.assertTrue(verify_targeting(summary, snapshots, mobile)["pass"])
            self.assertFalse(verify_targeting(summary, [], mobile)["pass"])

    def test_wrong_target_or_skill_substitution_is_rejected(self):
        for change in ({"target": {"kind": "player", "id": 3}}, {"kind": "cast"}):
            summary, snapshots = fixture()
            summary["commands"][0].update(change)
            self.assertFalse(verify_targeting(summary, snapshots, False)["pass"])

    def test_aim_or_cancel_attack_leak_is_rejected(self):
        for timestamp in (.5, 4.):
            summary, snapshots = fixture(True)
            summary["commands"].append(dict(kind="basic_attack", target=dict(kind="player", id=2),
                                             elapsed_seconds=timestamp))
            self.assertFalse(verify_targeting(summary, snapshots, True)["pass"])

    def test_server_damage_before_input_or_after_cancel_is_rejected(self):
        for tick in (15, 45):
            summary, snapshots = fixture(True)
            next(snapshot for snapshot in snapshots if snapshot["snapshot_tick"] == tick)["players"][1]["hp"] -= 5
            result = verify_targeting(summary, snapshots, True)
            self.assertFalse(result["pass"])
            self.assertTrue(any("damage-free" in error for error in result["errors"]))

    def test_hp_change_without_accepted_basic_action_is_rejected(self):
        summary, snapshots = fixture()
        for snapshot in snapshots:
            snapshot["players"][0]["action_slot"] = 0
        self.assertFalse(verify_targeting(summary, snapshots, False)["pass"])

    def test_one_damage_drop_cannot_prove_desktop_repetition(self):
        summary, snapshots = fixture()
        for snapshot in snapshots:
            snapshot["players"][1]["hp"] = max(snapshot["players"][1]["hp"], 90)
        self.assertFalse(verify_targeting(summary, snapshots, False)["pass"])

    def test_other_target_damage_and_mana_cost_are_rejected(self):
        for player_index, key, value in ((2, "hp", 90), (0, "mana", 90)):
            summary, snapshots = fixture()
            snapshots[15]["players"][player_index][key] = value
            self.assertFalse(verify_targeting(summary, snapshots, False)["pass"])

    def test_profile_or_duplicate_capture_cannot_hide_missing_stage(self):
        for duplicate in (False, True):
            summary, snapshots = fixture(True)
            if duplicate:
                summary["captures"][3] = copy.deepcopy(summary["captures"][2])
            else:
                summary["captures"][1]["mobile_controls"] = False
            self.assertFalse(verify_targeting(summary, snapshots, True)["pass"])

    def test_locked_target_requires_an_actually_visible_marker(self):
        for mutation in ("missing", "zero", "hidden", "stale", "invalid"):
            summary, snapshots = fixture(True)
            capture = summary["captures"][0 if mutation == "stale" else 2]
            if mutation == "missing":
                capture["nodes"] = []
            elif mutation in ("zero", "stale", "invalid"):
                capture["nodes"][0]["size"] = {"zero": [0, 48], "stale": [48, 48], "invalid": [float("nan"), 48]}[mutation]
            else:
                capture["nodes"][0]["visible"] = False
            self.assertFalse(verify_targeting(summary, snapshots, True)["pass"], mutation)

    def test_unobserved_second_target_or_short_cancel_window_is_rejected(self):
        for missing_target in (False, True):
            summary, snapshots = fixture()
            if missing_target:
                for snapshot in snapshots:
                    snapshot["players"].pop()
            else:
                next(e for e in summary["events"] if e["event"] == "cancellation_verified")["snapshot_tick"] = 31
            self.assertFalse(verify_targeting(summary, snapshots, False)["pass"])

    def test_missing_provenance_and_missing_gesture_are_rejected(self):
        for key in ("synthetic_damage", "setup_fixture"):
            summary, snapshots = fixture(True)
            del summary[key]
            self.assertFalse(verify_targeting(summary, snapshots, True)["pass"])
        summary, snapshots = fixture(True)
        summary["events"] = [e for e in summary["events"] if e["event"] != "touch_canceled"]
        self.assertFalse(verify_targeting(summary, snapshots, True)["pass"])


if __name__ == "__main__":
    unittest.main()
