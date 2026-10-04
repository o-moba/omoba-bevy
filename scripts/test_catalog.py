#!/usr/bin/env python3
"""The Python view of the shared hero and item catalogs."""
import json
from pathlib import Path
import tempfile
import unittest

import catalog


class CatalogTests(unittest.TestCase):
    def test_shipped_catalog_lists_every_class_in_enum_order(self):
        self.assertEqual(catalog.hero_ids(), ('warrior', 'mage', 'ranger', 'cleric', 'warden', 'dawnweaver', 'wildspark',
            'cinderforge', 'edgeweaver', 'stormfist', 'veilstalker', 'emberveil',
            'orbitwright', 'riftshot', 'chainkeeper', 'frostguard', 'adventurer'))
        for hero in catalog.heroes():
            self.assertEqual(len(hero['abilities']), 4, hero['id'])
            self.assertEqual(sorted(hero['recommended_items']), sorted(catalog.item_costs()), hero['id'])

    def test_items_have_positive_costs_and_known_ids(self):
        costs = catalog.item_costs()
        self.assertEqual(list(costs), ['ember_blade', 'swift_grip', 'trail_boots',
                                       'vitality_gem', 'focus_charm', 'guardian_crest',
                                       'crit_shard', 'siphon_stone', 'windrunner_boots',
                                       'duelist_edge', 'vampiric_fang', 'arcane_focus',
                                       'bulwark', 'tempest_blade', 'bloodreaver', 'aether_crown'])
        self.assertTrue(all(isinstance(cost, int) and cost > 0 for cost in costs.values()))
        self.assertEqual(costs['focus_charm'], 100)

    def test_projectile_styles_and_offensive_slots_follow_the_kits(self):
        self.assertEqual(catalog.projectile_styles(), {'warrior': 'crescent', 'mage': 'arcane', 'ranger': 'arrow',
                                                       'cleric': 'holy', 'warden': 'claw', 'dawnweaver': 'holy', 'wildspark': 'arrow',
            'cinderforge': 'crescent', 'edgeweaver': 'crescent', 'stormfist': 'crescent',
            'veilstalker': 'claw', 'emberveil': 'arcane', 'orbitwright': 'holy',
            'riftshot': 'arrow', 'chainkeeper': 'holy', 'frostguard': 'crescent',
            'adventurer': 'crescent'})
        slots = catalog.offensive_slots()
        self.assertEqual(slots['cleric'], {0})
        for class_id in ('warrior', 'mage', 'ranger', 'warden'):
            self.assertEqual(slots[class_id], {0, 2, 3}, class_id)
        self.assertEqual(slots['dawnweaver'], {0, 2, 3})
        self.assertEqual(slots['wildspark'], {1, 2, 3})
        for hero, expected in {
            'cinderforge': {0, 1, 2, 3}, 'edgeweaver': {0, 1},
            'stormfist': {0, 2, 3}, 'veilstalker': {0, 2, 3},
            'emberveil': {0, 1, 2, 3}, 'orbitwright': {0, 1, 2, 3},
            'riftshot': {0, 2, 3}, 'chainkeeper': {0, 2, 3}, 'frostguard': {0, 3},
            'adventurer': {0, 2, 3},
        }.items():
            self.assertEqual(slots[hero], expected, hero)
        # Reusable shield/toggle effects never become fake offensive casts.
        styles = {hero['id']: hero for hero in catalog.heroes()}
        self.assertEqual(styles['dawnweaver']['abilities'][1]['effect']['kind'], 'returning_shield')
        self.assertEqual(styles['wildspark']['abilities'][0]['effect']['kind'], 'weapon_toggle')

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
