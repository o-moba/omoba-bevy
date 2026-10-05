#!/usr/bin/env python3
"""Hero and item catalogs, read from the same JSON the game embeds.

`shared/assets/catalog/heroes.json` and `items.json` are the source of truth
for class ids, ability kits, projectile styles and item costs (`shared::catalog`
in Rust). Scripts import this module instead of copying those values.
"""
from __future__ import annotations

from functools import lru_cache
import json
import re
from pathlib import Path

CATALOG_DIR = Path(__file__).resolve().parents[1] / 'shared' / 'assets' / 'catalog'
SCHEMA_VERSION = 1


@lru_cache(maxsize=1)
def protocol_version() -> int:
    """Use the same live protocol as Rust; historical fixtures keep their version."""
    source = (CATALOG_DIR.parents[1] / 'src' / 'protocol.rs').read_text(encoding='utf-8')
    match = re.search(r'pub const PROTOCOL_VERSION: u16 = (\d+);', source)
    if match is None:
        raise ValueError('shared protocol version constant not found')
    return int(match.group(1))


@lru_cache(maxsize=1)
def map_tuning() -> dict:
    """Read the literal compact-map tuning used by both Rust authorities."""
    source_root = CATALOG_DIR.parents[1] / 'src'
    def literal(filename, name):
        source = (source_root / filename).read_text(encoding='utf-8')
        match = re.search(rf'pub const {name}: (?:f32|usize) = ([0-9]+(?:\.[0-9]+)?);', source)
        if match is None:
            raise ValueError(f'shared map tuning constant {name} not found')
        return float(match.group(1))
    return dict(world_scale=literal('map.rs', 'WORLD_SCALE'),
                river_width=literal('map.rs', 'RIVER_WIDTH'),
                camp_count=int(literal('jungle.rs', 'CAMP_COUNT')))


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
    """Hero classes in enum order, resolving reusable skill references.

    Keep the existing ``abilities`` view for QA consumers. Its damage field is
    informational; modular casts still require the aimed wire request.
    """
    directory = Path(directory)
    result = []
    skills = None
    for raw in _load(directory, 'heroes', 'classes'):
        hero = dict(raw)
        if 'skills' in hero:
            if skills is None:
                skills = {entry['id']: entry for entry in _load(directory, 'skills', 'skills')}
            abilities = []
            for identifier in hero['skills']:
                if identifier not in skills:
                    raise ValueError(f'unknown skill reference {identifier!r}')
                ability = dict(skills[identifier])
                damage = ability['effect'].get('damage')
                # These techniques author shield, delayed mark or buff strength,
                # not an immediate damaging cast for the transport QA consumer.
                support = ability['effect'].get('action') in {
                    'double_strike', 'vital_challenge', 'guard_leap', 'curse',
                    'detonation_mark', 'lantern', 'ally_leap', 'intercept_shield',
                }
                if damage is not None and damage > 0 and not support:
                    ability['projectile_damage'] = damage
                abilities.append(ability)
            hero['abilities'] = abilities
        result.append(hero)
    return tuple(result)


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
