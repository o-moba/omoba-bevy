import copy
import io
import unittest

import public_bots_rehearsal as rehearsal


class PublicBotsRehearsalTests(unittest.TestCase):
    def test_only_disposable_loopback_database_without_options_is_allowed(self):
        for host in ("127.0.0.1", "localhost", "[::1]"):
            self.assertTrue(rehearsal.isolated_database(f"postgres://tester:pass@{host}:5432/omoba_test"))
        for url in (
            "postgres://tester@127.0.0.1/omoba", "postgres://tester@db.example/omoba_test",
            "postgres://tester@127.0.0.1/omoba_test?host=db.example", "postgres://localhost/omoba_test#other",
            "postgres://[broken/omoba_test", "https://localhost/omoba_test", "",
        ):
            with self.subTest(url=url):
                self.assertFalse(rehearsal.isolated_database(url))

    def test_fog_of_war_does_not_require_ten_visible_enemies_but_every_peer_is_live(self):
        sample = {"phase": "running", "players": [{"id": 1}], "peers": [
            {"id": i, "snapshot_age_ms": 20, "public": {"phase": "running", "profile_id": str(i)}}
            for i in range(1, 11)
        ]}
        self.assertTrue(rehearsal.running_roster(sample, 10))
        for mutation in ("stale", "duplicate_identity", "still_loading"):
            changed = copy.deepcopy(sample)
            if mutation == "stale":
                changed["peers"][-1]["snapshot_age_ms"] = 4000
            elif mutation == "duplicate_identity":
                changed["peers"][-1]["public"]["profile_id"] = "1"
            else:
                changed["peers"][-1]["public"]["phase"] = "draft"
            self.assertFalse(rehearsal.running_roster(changed, 10))

    def test_partially_flushed_result_line_is_retried_without_loss_or_duplicates(self):
        stream = io.StringIO('BOT_RESULT [{"saved":true}')
        self.assertEqual(list(rehearsal.read_events(stream)), [])
        offset = stream.tell()
        stream.seek(0, 2)
        stream.write(']\nBOT_SAMPLE {"phase":"victory"}\n')
        stream.seek(offset)
        self.assertEqual(list(rehearsal.read_events(stream)), [
            ("BOT_RESULT", [{"saved": True}]), ("BOT_SAMPLE", {"phase": "victory"})])
        self.assertEqual(list(rehearsal.read_events(stream)), [])

    @staticmethod
    def completed_ranked(rated_matches=0):
        delta = 24 if rated_matches < 20 else 16
        participants = [{"profile_id": str(i), "team": "green" if i < 5 else "blue",
                         "participation": {"connected_ms": 600000, "disconnected_ms": 0,
                                           "afk_ms": 0, "abandoned": False},
                         "rating": {"before": 1000, "after": 1000 + (delta if i < 5 else -delta),
                                    "delta": delta if i < 5 else -delta},
                         "progression_xp_gained": 150 if i < 5 else 100} for i in range(10)]
        result = {"result_id": "result", "saved": True, "outcome": "completed", "rated": True,
                  "winner": "green", "ruleset": "verdant-default-v1", "participants": participants}
        return [{"result": copy.deepcopy(result),
                 "initial_profile": {"rating": 1000, "rated_matches": rated_matches, "matches_played": rated_matches, "completed_public_matches": 10,
                                     "wins": 0, "losses": rated_matches, "progression_xp": 0},
                 "profile": {"profile_id": str(i), "matches_played": rated_matches + 1, "rated_matches": rated_matches + 1,
                             "completed_public_matches": 11,
                             "rating": participants[i]["rating"]["after"],
                             "progression_xp": participants[i]["progression_xp_gained"],
                             "wins": int(i < 5), "losses": rated_matches + int(i >= 5)},
                 "history": [{"result_id": "result", "rating": participants[i]["rating"]}]} for i in range(10)]

    def test_a_complete_rated_result_checks_every_profile_and_history(self):
        self.assertEqual(rehearsal.verify_results(self.completed_ranked(), 10, True), "result")

    def test_ordinary_ranked_proof_cannot_accept_afk_or_missing_participation(self):
        for mutation in ("afk", "disconnected", "abandoned", "missing"):
            statuses = self.completed_ranked()
            for status in statuses:
                participant = status["result"]["participants"][5]
                if mutation == "missing":
                    participant.pop("participation")
                else:
                    key = {"afk": "afk_ms", "disconnected": "disconnected_ms",
                           "abandoned": "abandoned"}[mutation]
                    participant["participation"][key] = True if mutation == "abandoned" else 1
            with self.subTest(mutation=mutation), self.assertRaisesRegex(AssertionError, "active connected"):
                rehearsal.verify_results(statuses, 10, True)

    def test_twentieth_match_uses_placement_k_and_the_next_uses_regular_k(self):
        for count, delta in [(19, 24), (20, 16)]:
            statuses = self.completed_ranked(count)
            self.assertEqual(statuses[0]["result"]["participants"][0]["rating"]["delta"], delta)
            self.assertEqual(rehearsal.verify_results(statuses, 10, True), "result")

    def test_consistent_but_wrong_placement_award_is_rejected(self):
        statuses = self.completed_ranked()
        for status in statuses:
            for p in status["result"]["participants"]:
                p["rating"].update(after=1016 if p["team"] == "green" else 984,
                                   delta=16 if p["team"] == "green" else -16)
            me = status["result"]["participants"][int(status["profile"]["profile_id"])]
            status["profile"]["rating"] = me["rating"]["after"]
            status["history"][0]["rating"] = me["rating"].copy()
        with self.assertRaisesRegex(AssertionError, "wrong placement/team Elo"):
            rehearsal.verify_results(statuses, 10, True)

    def test_rated_proof_rejects_partial_or_uncredited_or_duplicated_settlement(self):
        for mutation in ("unrated", "not_saved", "wrong_xp", "duplicate", "missing_client", "unplayed", "duplicate_profile", "wrong_history_rating"):
            statuses = self.completed_ranked()
            if mutation == "unrated":
                statuses[-1]["result"]["rated"] = False
            elif mutation == "not_saved":
                statuses[-1]["result"]["saved"] = False
            elif mutation == "wrong_xp":
                statuses[-1]["profile"]["progression_xp"] = 0
            elif mutation == "duplicate":
                statuses[-1]["history"] *= 2
            elif mutation == "unplayed":
                statuses[-1]["result"]["gameplay_started"] = False
            elif mutation == "duplicate_profile":
                statuses[-1] = copy.deepcopy(statuses[0])
            elif mutation == "wrong_history_rating":
                statuses[-1]["history"][0]["rating"] = None
            else:
                statuses.pop()
            with self.subTest(mutation=mutation), self.assertRaises(AssertionError):
                rehearsal.verify_results(statuses, 10, True)

    def test_custom_map_victory_must_remain_unrated_without_progression(self):
        statuses = self.completed_ranked()
        for status in statuses:
            status["result"]["ruleset"] = "custom-unrated-v1"
            status["result"]["rated"] = False
            for participant in status["result"]["participants"]:
                participant["rating"] = None
                participant["progression_xp_gained"] = 0
            status["profile"].update(rating=1000, rated_matches=0, progression_xp=0, completed_public_matches=10)
            status["history"][0]["rating"] = None
        self.assertEqual(rehearsal.verify_results(statuses, 10, False, True), "result")
        statuses[-1]["profile"]["rated_matches"] = 1
        with self.assertRaises(AssertionError):
            rehearsal.verify_results(statuses, 10, False, True)


if __name__ == "__main__":
    unittest.main()
