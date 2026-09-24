#!/usr/bin/env python3
"""The Python view of the shared hero and item catalogs."""
import json
from pathlib import Path
import tempfile
import unittest

import catalog


class CatalogTests(unittest.TestCase):
    def test_shipped_catalog_lists_every_class_in_enum_order(self):
        self.assertEqual(catalog.hero_ids(), ('warrior', 'mage', 'ranger', 'cleric', 'warden'))
        for hero in catalog.heroes():
            self.assertEqual(len(hero['abilities']), 4, hero['id'])
            self.assertEqual(sorted(hero['recommended_items']), sorted(catalog.item_costs()), hero['id'])

    def test_items_have_positive_costs_and_known_ids(self):
        costs = catalog.item_costs()
        self.assertEqual(list(costs), ['ember_blade', 'swift_grip', 'trail_boots',
                                       'vitality_gem', 'focus_charm', 'guardian_crest'])
        self.assertTrue(all(isinstance(cost, int) and cost > 0 for cost in costs.values()))
        self.assertEqual(costs['focus_charm'], 100)

    def test_projectile_styles_and_offensive_slots_follow_the_kits(self):
        self.assertEqual(catalog.projectile_styles(), {'warrior': 'crescent', 'mage': 'arcane', 'ranger': 'arrow',
                                                       'cleric': 'holy', 'warden': 'claw'})
        slots = catalog.offensive_slots()
        self.assertEqual(slots['cleric'], {0})
        for class_id in ('warrior', 'mage', 'ranger', 'warden'):
            self.assertEqual(slots[class_id], {0, 2, 3}, class_id)
        # Q opens with damage for every class.
        self.assertTrue(all(0 in s for s in slots.values()))

    def test_catalog_directory_resolves_from_any_working_directory(self):
        self.assertTrue((catalog.CATALOG_DIR / 'heroes.json').is_file())
        self.assertTrue((catalog.CATALOG_DIR / 'items.json').is_file())

    def test_rejects_an_unknown_schema_version_or_empty_table(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'heroes.json').write_text(json.dumps({'schema_version': 2, 'classes': [{'id': 'x'}]}))
            (root / 'items.json').write_text(json.dumps({'schema_version': 1, 'items': []}))
            with self.assertRaisesRegex(ValueError, 'schema_version 2'):
                catalog.heroes(root)
            with self.assertRaisesRegex(ValueError, 'non-empty list'):
                catalog.items(root)


if __name__ == '__main__':
    unittest.main()
