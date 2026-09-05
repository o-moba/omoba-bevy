#!/usr/bin/env python3
"""Validate the deterministic Omoba full-2D world-art contract."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import math
import struct
import sys
import tempfile
import unittest
import zlib
from pathlib import Path


PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
REQUIRED_TILE_CATEGORIES = {
    "grass",
    "forest_floor",
    "path",
    "shallow_water",
    "path_transition",
    "water_transition",
    "forest_transition",
    "crossing",
    "base",
    "camp_clearing",
    "boss_clearing",
    "objective_decal",
}
REQUIRED_PROP_CATEGORIES = {
    "tree",
    "shrub",
    "flowers",
    "reeds",
    "rock_log",
    "camp",
    "objective",
    "team_accent",
}
REQUIRED_TRANSITIONS = {
    "path_edge",
    "path_outer_corner",
    "path_inner_corner",
    "water_edge",
    "water_outer_corner",
    "water_inner_corner",
    "forest_edge",
    "stone_crossing",
}
REQUIRED_FILES = {"manifest.json", "LICENSE.md", "terrain-atlas.png", "props-atlas.png"}


class ValidationError(ValueError):
    pass


def reject_duplicate_keys(pairs: list[tuple[str, object]]) -> dict:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise ValidationError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def load_json(path: Path) -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=reject_duplicate_keys)
    except (OSError, json.JSONDecodeError) as error:
        raise ValidationError(f"cannot read {path}: {error}") from error
    if not isinstance(value, dict):
        raise ValidationError(f"{path}: root must be an object")
    return value


def paeth(a: int, b: int, c: int) -> int:
    estimate = a + b - c
    options = (abs(estimate - a), abs(estimate - b), abs(estimate - c))
    return (a, b, c)[options.index(min(options))]


def read_rgba_png(path: Path) -> tuple[int, int, bytes]:
    try:
        raw = path.read_bytes()
    except OSError as error:
        raise ValidationError(f"missing asset: {path.name}") from error
    if not raw.startswith(PNG_SIGNATURE):
        raise ValidationError(f"{path.name}: not a PNG")
    offset = len(PNG_SIGNATURE)
    width = height = None
    color_type = None
    compressed = bytearray()
    while offset + 12 <= len(raw):
        length = struct.unpack(">I", raw[offset : offset + 4])[0]
        kind = raw[offset + 4 : offset + 8]
        data = raw[offset + 8 : offset + 8 + length]
        offset += length + 12
        if kind == b"IHDR":
            width, height, depth, color_type, compression, filtering, interlace = struct.unpack(
                ">IIBBBBB", data
            )
            if (depth, color_type, compression, filtering, interlace) != (8, 6, 0, 0, 0):
                raise ValidationError(f"{path.name}: expected non-interlaced 8-bit RGBA")
        elif kind == b"IDAT":
            compressed.extend(data)
        elif kind == b"IEND":
            break
    if width is None or height is None or color_type != 6:
        raise ValidationError(f"{path.name}: missing valid RGBA header")
    try:
        scanlines = zlib.decompress(compressed)
    except zlib.error as error:
        raise ValidationError(f"{path.name}: invalid image data: {error}") from error
    stride = width * 4
    if len(scanlines) != height * (stride + 1):
        raise ValidationError(f"{path.name}: unexpected decoded byte count")
    pixels = bytearray(height * stride)
    source = 0
    for y in range(height):
        filter_kind = scanlines[source]
        source += 1
        row = bytearray(scanlines[source : source + stride])
        source += stride
        previous = pixels[(y - 1) * stride : y * stride] if y else bytes(stride)
        for index in range(stride):
            left = row[index - 4] if index >= 4 else 0
            above = previous[index]
            upper_left = previous[index - 4] if index >= 4 else 0
            if filter_kind == 1:
                row[index] = (row[index] + left) & 0xFF
            elif filter_kind == 2:
                row[index] = (row[index] + above) & 0xFF
            elif filter_kind == 3:
                row[index] = (row[index] + ((left + above) // 2)) & 0xFF
            elif filter_kind == 4:
                row[index] = (row[index] + paeth(left, above, upper_left)) & 0xFF
            elif filter_kind != 0:
                raise ValidationError(f"{path.name}: unsupported PNG filter {filter_kind}")
        pixels[y * stride : (y + 1) * stride] = row
    return width, height, bytes(pixels)


def safe_relative_png(value: object) -> bool:
    if not isinstance(value, str) or Path(value).suffix.lower() != ".png":
        return False
    path = Path(value)
    return bool(value) and not path.is_absolute() and ".." not in path.parts and path.name == value


def numeric_pair(value: object, label: str) -> list[float]:
    if not isinstance(value, list) or len(value) != 2 or any(
        not isinstance(item, (int, float)) or not math.isfinite(item) for item in value
    ):
        raise ValidationError(f"{label}: expected finite numeric pair")
    return [float(value[0]), float(value[1])]


def close_pair(actual: object, expected: tuple[float, float], label: str, epsilon: float = 1e-5) -> None:
    pair = numeric_pair(actual, label)
    if math.dist(pair, expected) > epsilon:
        raise ValidationError(f"{label}: topology mismatch {pair} != {expected}")


def frame_pixels(pixels: bytes, atlas_width: int, frame: int, columns: int, cell: int) -> bytes:
    cell_x, cell_y = (frame % columns) * cell, (frame // columns) * cell
    output = bytearray()
    for y in range(cell_y, cell_y + cell):
        start = (y * atlas_width + cell_x) * 4
        output.extend(pixels[start : start + cell * 4])
    return bytes(output)


def validate_manifest(data: dict, root: Path, *, check_pixels: bool = True) -> dict[str, int]:
    if data.get("schema_version") != 1:
        raise ValidationError("schema_version must be 1")
    if data.get("license") != "CC0-1.0":
        raise ValidationError("world art license must be CC0-1.0")
    budget = data.get("repository_size_budget_bytes")
    if not isinstance(budget, int) or not 1_000_000 <= budget <= 20_000_000:
        raise ValidationError("invalid repository_size_budget_bytes")
    if data.get("tile_pixels") != 128 or data.get("tile_world_size") != 4.0:
        raise ValidationError("tile pixel/world size contract mismatch")

    atlases = data.get("atlases")
    if not isinstance(atlases, dict) or set(atlases) != {"terrain", "props"}:
        raise ValidationError("atlases must contain exactly terrain and props")
    decoded: dict[str, tuple[int, int, bytes]] = {}
    for name in ("terrain", "props"):
        atlas = atlases[name]
        if not isinstance(atlas, dict) or atlas.get("grid") != [8, 4] or atlas.get("frame_size") != [128, 128]:
            raise ValidationError(f"{name}: expected 8x4 atlas of 128px cells")
        if not safe_relative_png(atlas.get("path")):
            raise ValidationError(f"{name}: unsafe atlas path")
        asset_path = root / atlas["path"]
        if not asset_path.is_file():
            raise ValidationError(f"{name}: missing asset {atlas['path']}")
        if check_pixels:
            decoded[name] = read_rgba_png(asset_path)
            if decoded[name][:2] != (1024, 512):
                raise ValidationError(f"{name}: atlas must be 1024x512 RGBA")

    tiles = data.get("tiles")
    props = data.get("props")
    if not isinstance(tiles, dict) or not isinstance(props, dict):
        raise ValidationError("tiles and props must be objects")
    categories = {tile.get("category") for tile in tiles.values() if isinstance(tile, dict)}
    if not REQUIRED_TILE_CATEGORIES.issubset(categories):
        raise ValidationError("missing required terrain categories")
    if not REQUIRED_TRANSITIONS.issubset(tiles):
        raise ValidationError("missing edge/corner/transition tile family")
    prop_categories = {prop.get("category") for prop in props.values() if isinstance(prop, dict)}
    if not REQUIRED_PROP_CATEGORIES.issubset(prop_categories):
        raise ValidationError("missing required prop categories")

    tile_frames: list[int] = []
    for identifier, tile in tiles.items():
        if not isinstance(tile, dict):
            raise ValidationError(f"{identifier}: tile entry must be an object")
        frame = tile.get("frame")
        if not isinstance(frame, int) or not 0 <= frame < 32:
            raise ValidationError(f"{identifier}: frame is out of bounds")
        tile_frames.append(frame)
        if tile.get("walkability") not in {"traversable", "cosmetic_traversable"}:
            raise ValidationError(f"{identifier}: world art may not invent collision")
        rotations = tile.get("rotations")
        if not isinstance(rotations, list) or not rotations or any(value not in {0, 90, 180, 270} for value in rotations):
            raise ValidationError(f"{identifier}: invalid rotation family")
        pivot = numeric_pair(tile.get("pivot"), f"{identifier}.pivot")
        if any(not 0 <= value <= 1 for value in pivot):
            raise ValidationError(f"{identifier}: pivot is out of range")
    if len(tile_frames) != 32 or len(set(tile_frames)) != 32:
        raise ValidationError("terrain atlas must define 32 unique frame mappings")

    prop_frames: list[int] = []
    for identifier, prop in props.items():
        if not isinstance(prop, dict):
            raise ValidationError(f"{identifier}: prop entry must be an object")
        frame = prop.get("frame")
        if not isinstance(frame, int) or not 0 <= frame < 32:
            raise ValidationError(f"{identifier}: prop frame is out of bounds")
        prop_frames.append(frame)
        pivot = numeric_pair(prop.get("pivot"), f"{identifier}.pivot")
        size = numeric_pair(prop.get("world_size"), f"{identifier}.world_size")
        if any(not 0 <= value <= 1 for value in pivot) or any(not 0 < value <= 12 for value in size):
            raise ValidationError(f"{identifier}: invalid pivot/world_size")
        if not isinstance(prop.get("clearance_radius"), (int, float)) or not 0 <= prop["clearance_radius"] <= 6:
            raise ValidationError(f"{identifier}: invalid clearance_radius")
    if len(prop_frames) != 16 or len(set(prop_frames)) != 16:
        raise ValidationError("prop atlas must define 16 unique occupied frames")

    layers = data.get("layers")
    required_layers = {"ground", "transitions_decals", "low_props_shadows", "actors_structures", "projectiles_vfx", "overhead_markers", "screen_ui", "y_sort_scale", "stable_tie_break_scale"}
    if not isinstance(layers, dict) or not required_layers.issubset(layers):
        raise ValidationError("missing deterministic layer/sort bands")
    ordered = [layers[name] for name in ["ground", "transitions_decals", "low_props_shadows", "actors_structures", "projectiles_vfx", "overhead_markers", "screen_ui"]]
    if any(not isinstance(value, (int, float)) for value in ordered) or ordered != sorted(ordered) or len(set(ordered)) != len(ordered):
        raise ValidationError("layer bands must be strictly increasing")

    teams = data.get("teams")
    if not isinstance(teams, dict) or set(teams) != {"green", "blue", "neutral"}:
        raise ValidationError("missing team/neutral art language")
    if teams["green"].get("shape") == teams["blue"].get("shape"):
        raise ValidationError("teams must differ by shape, not color alone")
    for team in ("green", "blue"):
        if teams[team].get("base_tile") not in tiles or teams[team].get("base_prop") not in props:
            raise ValidationError(f"{team}: invalid manifest topology reference")

    topology = data.get("topology")
    if not isinstance(topology, dict) or topology.get("coordinate_domain") != "simulation_xz":
        raise ValidationError("topology must use simulation_xz")
    target = 5.0 * 45.0
    half_inner = (target / math.sqrt(2.0)) * 0.5
    half_map = half_inner + 23.0 + 6.0
    map_size = half_map * 2.0
    outer, inner = map_size * 0.34, map_size * 0.22
    close_pair(topology.get("bounds", {}).get("min"), (-half_map, -half_map), "bounds.min", 1e-4)
    close_pair(topology.get("bounds", {}).get("max"), (half_map, half_map), "bounds.max", 1e-4)
    close_pair(topology.get("bases", {}).get("green"), (-half_inner, -half_inner), "bases.green", 1e-4)
    close_pair(topology.get("bases", {}).get("blue"), (half_inner, half_inner), "bases.blue", 1e-4)
    expected_camps = [(-outer, inner), (outer, -inner), (-inner, -outer)]
    expected_bosses = [(inner, -outer), (-inner, outer)]
    camps, bosses = topology.get("camps"), topology.get("boss_pits")
    if not isinstance(camps, list) or len(camps) != 3 or not isinstance(bosses, list) or len(bosses) != 2:
        raise ValidationError("topology must define three camps and two boss pits")
    for index, expected in enumerate(expected_camps):
        close_pair(camps[index], expected, f"camps[{index}]", 1e-4)
    for index, expected in enumerate(expected_bosses):
        close_pair(bosses[index], expected, f"boss_pits[{index}]", 1e-4)
    lanes = topology.get("lanes")
    if not isinstance(lanes, dict) or set(lanes) != {"mid", "top", "bot"} or [len(lanes[name]) for name in ("mid", "top", "bot")] != [2, 6, 6]:
        raise ValidationError("topology must define all three lane polylines")
    river = topology.get("river")
    if not isinstance(river, dict) or river.get("width") != 18.0 or river.get("traversable") is not True or len(river.get("polyline", [])) != 2:
        raise ValidationError("invalid traversable river topology")
    towers = topology.get("lane_towers")
    bases = topology.get("base_objectives")
    if not isinstance(towers, list) or len(towers) != 6 or not isinstance(bases, list) or len(bases) != 2:
        raise ValidationError("topology must define six lane and two base objectives")
    if {(tower.get("team"), tower.get("lane")) for tower in towers} != {(team, lane) for team in ("green", "blue") for lane in ("mid", "top", "bot")}:
        raise ValidationError("tower topology team/lane coverage mismatch")

    generation = data.get("generation")
    grid = generation.get("grid", {}) if isinstance(generation, dict) else {}
    columns, rows = grid.get("columns"), grid.get("rows")
    if (columns, rows) != (55, 55) or grid.get("static_tile_count") != columns * rows:
        raise ValidationError("deterministic grid dimensions/count mismatch")
    if grid.get("static_tile_count") > grid.get("max_static_entities", 0) or grid.get("max_static_entities") > 4096:
        raise ValidationError("static world entity budget exceeded")
    if generation.get("seed") != 73129 or generation.get("algorithm") != "omoba_world2d_v1":
        raise ValidationError("deterministic generation algorithm/seed mismatch")
    if generation.get("prop_policy", {}).get("server_collision") is not False or generation["prop_policy"].get("max_props", 9999) > 256:
        raise ValidationError("prop collision/entity policy mismatch")

    if not (root / "LICENSE.md").is_file():
        raise ValidationError("LICENSE.md is required")
    actual_files = {path.name for path in root.iterdir() if path.is_file()}
    if actual_files != REQUIRED_FILES:
        raise ValidationError(f"orphaned/missing world2d files: {sorted(actual_files ^ REQUIRED_FILES)}")
    total_bytes = sum((root / name).stat().st_size for name in REQUIRED_FILES)
    if total_bytes > budget:
        raise ValidationError(f"world2d repository budget exceeded: {total_bytes} > {budget}")

    if check_pixels:
        terrain_width, _terrain_height, terrain_pixels = decoded["terrain"]
        terrain_hashes = []
        for frame in tile_frames:
            cell = frame_pixels(terrain_pixels, terrain_width, frame, 8, 128)
            if not any(cell[index] for index in range(3, len(cell), 4)):
                raise ValidationError(f"terrain frame {frame} is empty")
            terrain_hashes.append(hashlib.sha256(cell).digest())
        if len(set(terrain_hashes)) != len(terrain_hashes):
            raise ValidationError("terrain frames must be visually distinct")

        props_width, _props_height, props_pixels = decoded["props"]
        prop_hashes = []
        for frame in prop_frames:
            cell = frame_pixels(props_pixels, props_width, frame, 8, 128)
            alphas = cell[3::4]
            if not any(alphas) or all(alpha == 255 for alpha in alphas):
                raise ValidationError(f"prop frame {frame} requires isolated transparent RGBA content")
            for y in (*range(2), *range(126, 128)):
                if any(cell[(y * 128 + x) * 4 + 3] for x in range(128)):
                    raise ValidationError(f"prop frame {frame} touches vertical safety border")
            for y in range(128):
                if any(cell[(y * 128 + x) * 4 + 3] for x in (0, 1, 126, 127)):
                    raise ValidationError(f"prop frame {frame} touches horizontal safety border")
            visible = 0
            hot_magenta = 0
            for index in range(0, len(cell), 4):
                r, g, b, a = cell[index : index + 4]
                if a:
                    visible += 1
                    hot_magenta += int(r > 190 and b > 130 and g < 100)
            if visible and hot_magenta / visible > 0.015:
                raise ValidationError(f"prop frame {frame} retains magenta matte")
            prop_hashes.append(hashlib.sha256(cell).digest())
        if len(set(prop_hashes)) != len(prop_hashes):
            raise ValidationError("prop frames must be distinct")

    return {
        "terrain_frames": len(tile_frames),
        "prop_frames": len(prop_frames),
        "static_tiles": grid["static_tile_count"],
        "max_props": generation["prop_policy"]["max_props"],
        "asset_bytes": total_bytes,
    }


def validate_provenance(repo: Path, root: Path) -> None:
    task = repo / ".agent/tasks/TASK-FULL-2D-WORLD"
    provenance_path = task / "raw/higgsfield/provenance.json"
    prompts_path = task / "raw/higgsfield/prompts.md"
    art_direction = task / "art-direction.md"
    for required in (provenance_path, prompts_path, art_direction, task / "raw/world2d-contact-board.png", task / "raw/world2d-map-preview.png", task / "raw/world2d-topology-overlay.png"):
        if not required.is_file():
            raise ValidationError(f"missing task provenance/evidence: {required.name}")
    provenance = load_json(provenance_path)
    if provenance.get("license") != "CC0-1.0" or provenance.get("third_party_art_used") is not False or provenance.get("existing_game_reference_used") is not False:
        raise ValidationError("invalid originality/license provenance")
    preflight = provenance.get("preflight", {})
    if preflight.get("get_cost") is not True or preflight.get("total_credits") != 24 or preflight.get("total_credits", 9999) > preflight.get("authorization_ceiling_credits", 0):
        raise ValidationError("invalid Higgsfield cost preflight provenance")
    sources = provenance.get("sources")
    if not isinstance(sources, list) or len(sources) != 3:
        raise ValidationError("provenance must cover all three Higgsfield sources")
    for source in sources:
        if source.get("model") != "recraft_v4_1" or source.get("status") != "completed" or not source.get("job_id") or not str(source.get("source_url", "")).startswith("https://"):
            raise ValidationError("incomplete Higgsfield job/source provenance")
        source_path = provenance_path.parent / source["local_path"]
        if not source_path.is_file() or hashlib.sha256(source_path.read_bytes()).hexdigest() != source.get("sha256"):
            raise ValidationError(f"source hash mismatch: {source.get('id')}")
    outputs = provenance.get("outputs")
    if not isinstance(outputs, list) or len(outputs) != 2:
        raise ValidationError("provenance must cover both runtime atlases")
    for output in outputs:
        output_path = repo / output["path"]
        if not output_path.is_file() or hashlib.sha256(output_path.read_bytes()).hexdigest() != output.get("sha256"):
            raise ValidationError(f"output hash mismatch: {output.get('path')}")


class NegativeContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.repo = Path(__file__).resolve().parents[1]
        cls.root = cls.repo / "client/assets/world2d"
        cls.base = load_json(cls.root / "manifest.json")

    def rejected(self, mutate, pattern: str) -> None:
        data = copy.deepcopy(self.base)
        mutate(data)
        with self.assertRaisesRegex(ValidationError, pattern):
            validate_manifest(data, self.root, check_pixels=False)

    def test_unsafe_path(self) -> None:
        self.rejected(lambda data: data["atlases"]["terrain"].update(path="../escape.png"), "unsafe atlas path")

    def test_malformed_frame_range(self) -> None:
        self.rejected(lambda data: data["tiles"]["grass_a"].update(frame=32), "out of bounds")

    def test_missing_category(self) -> None:
        def mutate(data):
            for tile in data["tiles"].values():
                if tile["category"] == "boss_clearing":
                    tile["category"] = "grass"
        self.rejected(mutate, "missing required terrain categories")

    def test_missing_asset(self) -> None:
        self.rejected(lambda data: data["atlases"]["props"].update(path="absent.png"), "missing asset")

    def test_missing_license(self) -> None:
        self.rejected(lambda data: data.update(license=""), "license")

    def test_invalid_topology_reference(self) -> None:
        self.rejected(lambda data: data["teams"]["green"].update(base_prop="absent"), "invalid manifest topology reference")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[1]
    root = args.root or repo / "client/assets/world2d"
    if args.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(NegativeContractTests)
        return 0 if unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful() else 1
    try:
        result = validate_manifest(load_json(root / "manifest.json"), root)
        validate_provenance(repo, root)
    except ValidationError as error:
        print(f"world2d asset validation failed: {error}", file=sys.stderr)
        return 1
    print("world2d asset validation passed")
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
