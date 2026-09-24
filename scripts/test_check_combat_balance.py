import unittest

import check_combat_balance as balance


def capture(classes):
    matrix = [dict(level=level, attacker=a, defender=b, ttk=8.0)
              for level in (1, 5, 10) for a in classes for b in classes]
    supplements = [dict(level=level, attacker=c, policy=policy)
                   for level in (1, 10) for c in classes for policy in ('basic_only', 'q_only')]
    return dict(matrix=matrix, supplements=supplements)


class ShapeTests(unittest.TestCase):
    def test_classes_follow_the_capture_so_a_new_hero_is_not_a_shape_failure(self):
        four = ('warrior', 'mage', 'ranger', 'cleric')
        self.assertEqual(balance.check_shape(capture(four)), four)
        five = four + ('warden',)
        self.assertEqual(balance.check_shape(capture(five)), five)

    def test_a_missing_encounter_is_still_rejected(self):
        data = capture(('warrior', 'mage'))
        data['matrix'].pop()
        with self.assertRaises(AssertionError):
            balance.check_shape(data)


if __name__ == '__main__':
    unittest.main()
