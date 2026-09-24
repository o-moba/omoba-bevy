#!/usr/bin/env python3
"""Hero and item catalogs, read from the same JSON the game embeds.

`shared/assets/catalog/heroes.json` and `items.json` are the source of truth
for class ids, ability kits, projectile styles and item costs (`shared::catalog`
in Rust). Scripts import this module instead of copying those values.
"""
from __future__ import annotations

from functools import lru_cache
import json
from pathlib import Path

CATALOG_DIR = Path(__file__).resolve().parents[1] / 'shared' / 'assets' / 'catalog'
SCHEMA_VERSION = 1


@lru_cache(maxsize=None)
def _load(directory: Path, name: str, key: str) -> tuple:
    path = Path(directory) / f'{name}.json'
    with open(path, encoding='utf-8') as stream:
        data = json.load(stream)
    if data.get('schema_version') != SCHEMA_VERSION:
        raise ValueError(f'{path}: schema_version {data.get("schema_version")!r}, expected {SCHEMA_VERSION}')
    entries = data.get(key)
    if not isinstance(entries, list) or not entries:
        raise ValueError(f'{path}: "{key}" must be a non-empty list')
    return tuple(entries)


def heroes(directory: Path = CATALOG_DIR) -> tuple:
    """Hero classes in `HeroClass::ALL` order, as the JSON objects."""
    return _load(Path(directory), 'heroes', 'classes')


def items(directory: Path = CATALOG_DIR) -> tuple:
    """Shop items in `ItemId::ALL` order, as the JSON objects."""
    return _load(Path(directory), 'items', 'items')


def hero_ids(directory: Path = CATALOG_DIR) -> tuple[str, ...]:
    return tuple(hero['id'] for hero in heroes(directory))


def item_costs(directory: Path = CATALOG_DIR) -> dict[str, int]:
    return {item['id']: item['cost'] for item in items(directory)}


def projectile_styles(directory: Path = CATALOG_DIR) -> dict[str, str]:
    """Class id -> the `ProjectileStyle` wire name of its basic attack and Q."""
    return {hero['id']: hero['projectile_style'] for hero in heroes(directory)}


def offensive_slots(directory: Path = CATALOG_DIR) -> dict[str, frozenset[int]]:
    """Class id -> the Q/W/E/R slot indices whose ability deals projectile damage."""
    return {hero['id']: frozenset(slot for slot, ability in enumerate(hero['abilities'])
                                  if 'projectile_damage' in ability)
            for hero in heroes(directory)}
