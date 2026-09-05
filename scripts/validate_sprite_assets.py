#!/usr/bin/env python3
"""Validate the release-slice sprite and 2D presentation asset contracts."""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
import sys
import tempfile
import unittest
import zlib
from pathlib import Path

FROZEN_IDS = [
    "mossback-teapot",
    "neon-axolotl-courier",
    "origami-storm-heron",
    "clockwork-turnip-oracle",
    "void-jelly-astronaut",
    "cathedral-moth-bellringer",
    "tidal-typewriter-crab",
    "lunar-marionette-giraffe",
    "aurora-magnet-ram",
    "orchard-comet-centaur",
]
ORIGINAL_IDS = FROZEN_IDS[:5]
NEW_IDS = FROZEN_IDS[5:]
TASK_ID = "TASK-2D-CHARACTER-PACK-02"
RUNTIME_BUDGET_BYTES = 80 * 1024 * 1024
FROZEN_NEW_METADATA = {
    "cathedral-moth-bellringer": (
        "Cathedral Moth Bellringer",
        ["#17203D", "#F1E5C5", "#3BA99C", "#D6A33D", "#B84052"],
    ),
    "tidal-typewriter-crab": (
        "Tidal Typewriter Crab",
        ["#D85C53", "#66C7B5", "#172B45", "#F3E7C8", "#B88A3B"],
    ),
    "lunar-marionette-giraffe": (
        "Lunar Marionette Giraffe",
        ["#E7E2D2", "#5D3D73", "#202B55", "#D8646F", "#8FD5D2"],
    ),
    "aurora-magnet-ram": (
        "Aurora Magnet Ram",
        ["#75D5D8", "#244E8A", "#D94B59", "#222A35", "#A8D95B"],
    ),
    "orchard-comet-centaur": (
        "Orchard Comet Centaur",
        ["#67452F", "#2F887C", "#F0A24A", "#604A91", "#F5E8C8"],
    ),
}
ORIGINAL_ASSET_SHA256 = {
    "mossback-teapot.png": "fcd60ae6bc4e194c16165ee859ecf0dd6c5387133df3e7050a39c8119acfa505",
    "actions/mossback-teapot.png": "a46be181d39e9bf55c1b800a5bb8ef3a92f13f466cb32bdbf78729ef1812891c",
    "neon-axolotl-courier.png": "8dcb76599069cf0f6ed351cd64d9a0031313ebc9b8e18836981ccb29cd6a21f7",
    "actions/neon-axolotl-courier.png": "ddcacaf1291626b4565b054bd3bfd833c98e14f3a25a66eb87cbed4c1bb5f75b",
    "origami-storm-heron.png": "78c6ddd9b873080728c78aba8eeb08db578271082e9c9bf5cb6b927932b25940",
    "actions/origami-storm-heron.png": "97d3086698ce0233ba9c6515d2806fd60372a2fb9c3f35f2710607f1a74fa832",
    "clockwork-turnip-oracle.png": "d455f92a67921fbaa96da2cadb7397650728d43eff10cc5bc91ddca7b3adf1fd",
    "actions/clockwork-turnip-oracle.png": "ac3bc22c218e411c19dae76b7014b3ba9d91d2c9df0fca801f63005c30ee8412",
    "void-jelly-astronaut.png": "eba68957c1871a5991dd23b94f13e8c3f650e6b6a7319b67efa411aca4777c5f",
    "actions/void-jelly-astronaut.png": "03b9dc072ccd8d60ae50d088a187649dab74851d9ef108a629e49214247035b4",
}
ORIGINAL_PORTRAIT_RGBA_SHA256 = [
    "7f2d15bae5d701ea81bb95941b39df2ef386ab9febb7b33ff00335749e6f8904",
    "d62944b34bab93efb6dcf2eb4cd18e0d0ad7e9ad3d7c87d4d6be5bc44afd5c20",
    "0073be61dba3c7ca13ca795747ce752b5b8172d08ef77cd11292f7cc14c57110",
    "5dd66deabc6d3fb88d2c8efa79b2e7862446fb6481acb3814876f33867e7218d",
    "6fe90880bd736c78d7785ebb9cec9d18fd736694debf2a6518caa1d5947d6cf7",
]
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
OCCUPIED_ALPHA_THRESHOLD = 16


class ValidationError(ValueError):
    pass


def _paeth(a: int, b: int, c: int) -> int:
    estimate = a + b - c
    distances = abs(estimate - a), abs(estimate - b), abs(estimate - c)
    return (a, b, c)[distances.index(min(distances))]


def read_rgba_png(path: Path) -> tuple[int, int, bytes]:
    raw = path.read_bytes()
    if not raw.startswith(PNG_SIGNATURE):
        raise ValidationError(f"{path.name}: not a PNG")
    offset = len(PNG_SIGNATURE)
    width = height = color_type = None
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
            if depth != 8 or color_type not in (2, 6) or (compression, filtering, interlace) != (
                0,
                0,
                0,
            ):
                raise ValidationError(
                    f"{path.name}: expected non-interlaced 8-bit RGB/RGBA"
                )
        elif kind == b"IDAT":
            compressed.extend(data)
        elif kind == b"IEND":
            break
    if width is None or height is None:
        raise ValidationError(f"{path.name}: missing IHDR")
    try:
        scanlines = zlib.decompress(compressed)
    except zlib.error as error:
        raise ValidationError(f"{path.name}: invalid compressed pixels: {error}") from error
    bpp = 4 if color_type == 6 else 3
    stride = width * bpp
    if len(scanlines) != height * (stride + 1):
        raise ValidationError(f"{path.name}: unexpected decompressed byte count")
    pixels = bytearray(height * stride)
    src = 0
    for y in range(height):
        filter_kind = scanlines[src]
        src += 1
        row = bytearray(scanlines[src : src + stride])
        src += stride
        previous = pixels[(y - 1) * stride : y * stride] if y else bytes(stride)
        for x in range(stride):
            left = row[x - bpp] if x >= bpp else 0
            above = previous[x]
            upper_left = previous[x - bpp] if x >= bpp else 0
            if filter_kind == 1:
                row[x] = (row[x] + left) & 0xFF
            elif filter_kind == 2:
                row[x] = (row[x] + above) & 0xFF
            elif filter_kind == 3:
                row[x] = (row[x] + ((left + above) // 2)) & 0xFF
            elif filter_kind == 4:
                row[x] = (row[x] + _paeth(left, above, upper_left)) & 0xFF
            elif filter_kind != 0:
                raise ValidationError(f"{path.name}: unsupported PNG filter {filter_kind}")
        pixels[y * stride : (y + 1) * stride] = row
    if bpp == 3:
        rgba = bytearray(width * height * 4)
        for pixel in range(width * height):
            rgba[pixel * 4 : pixel * 4 + 3] = pixels[pixel * 3 : pixel * 3 + 3]
            rgba[pixel * 4 + 3] = 255
        pixels = rgba
    return width, height, bytes(pixels)


def _require_rgba_png(path: Path) -> None:
    raw = path.read_bytes()
    if len(raw) < 26 or not raw.startswith(PNG_SIGNATURE) or raw[25] != 6:
        raise ValidationError(f"{path.name}: expected 8-bit RGBA PNG")


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _validate_sha256(path: Path, expected: str, label: str) -> None:
    if not path.is_file():
        raise ValidationError(f"{label}: missing {path.name}")
    actual = _sha256_file(path)
    if actual != expected:
        raise ValidationError(f"{label}: expected SHA-256 {expected}, got {actual}")


def _safe_relative_png(value: object, *, allow_subdir: bool = False) -> bool:
    if not isinstance(value, str) or not value or Path(value).suffix.lower() != ".png":
        return False
    path = Path(value)
    if path.is_absolute() or ".." in path.parts:
        return False
    return allow_subdir or Path(value).name == value


def validate_manifest(data: dict) -> list[dict]:
    if data.get("schema_version") != 2:
        raise ValidationError("manifest schema_version must be 2")
    characters = data.get("characters")
    if not isinstance(characters, list) or len(characters) != len(FROZEN_IDS):
        raise ValidationError("manifest must contain exactly ten characters")
    ids = [entry.get("id") for entry in characters]
    sheets = [entry.get("sheet") for entry in characters]
    action_sheets = [entry.get("action_sheet") for entry in characters]
    if ids != FROZEN_IDS or len(set(ids)) != len(ids):
        raise ValidationError("character IDs must exactly match the frozen ordered roster")
    if len(set(sheets)) != len(sheets):
        raise ValidationError("sheet file names must be unique")
    if len(set(action_sheets)) != len(action_sheets):
        raise ValidationError("action sheet file names must be unique")
    for entry in characters:
        identifier = entry["id"]
        required_text = ["display_name", "theme", "license", "provenance"]
        if any(not isinstance(entry.get(key), str) or not entry[key] for key in required_text):
            raise ValidationError(f"{identifier}: missing required text metadata")
        if entry["license"] != "CC0-1.0" or not entry.get("palette"):
            raise ValidationError(f"{identifier}: invalid license or palette")
        if identifier in FROZEN_NEW_METADATA:
            expected_name, expected_palette = FROZEN_NEW_METADATA[identifier]
            if entry["display_name"] != expected_name or entry["palette"] != expected_palette:
                raise ValidationError(f"{identifier}: frozen name or palette changed")
            record = f".agent/tasks/{TASK_ID}/raw/{identifier}/provenance.json"
            if record not in entry["provenance"]:
                raise ValidationError(f"{identifier}: provenance does not cross-link {record}")
        sheet = entry["sheet"]
        if not _safe_relative_png(sheet):
            raise ValidationError(f"{identifier}: unsafe sheet path")
        action_sheet = entry.get("action_sheet")
        if not _safe_relative_png(action_sheet, allow_subdir=True) or Path(action_sheet).parts[:1] != (
            "actions",
        ):
            raise ValidationError(f"{identifier}: unsafe action_sheet path")
        if entry.get("frame_size") != [256, 256] or entry.get("columns") != 8 or entry.get("rows") != 2:
            raise ValidationError(f"{identifier}: grid must be 8x2 of 256px frames")
        if entry.get("action_columns") != 8 or entry.get("action_rows") != 4:
            raise ValidationError(f"{identifier}: action grid must be 8x4")
        pivot = entry.get("pivot")
        if not isinstance(pivot, list) or len(pivot) != 2 or any(not 0 <= value <= 1 for value in pivot):
            raise ValidationError(f"{identifier}: invalid pivot")
        # True-2D uses render-only exaggeration so occupied character pixels
        # remain readable at the maximum supported orthographic zoom-out.
        if not 1.8 <= entry.get("world_height", 0) <= 3.2:
            raise ValidationError(f"{identifier}: invalid world_height")
        expected = [
            ("idle", 0, 6.0, "locomotion", "loop", 16),
            ("run", 8, 12.0, "locomotion", "loop", 16),
            ("attack", 0, 12.0, "actions", "once", 32),
            ("cast", 8, 10.0, "actions", "once", 32),
            ("hit", 16, 14.0, "actions", "once", 32),
            ("death", 24, 8.0, "actions", "hold_last", 32),
        ]
        for name, expected_start, expected_fps, expected_sheet, playback, frame_count in expected:
            animation = entry.get("animations", {}).get(name, {})
            start, count, fps = animation.get("start"), animation.get("count"), animation.get("fps")
            if (
                start != expected_start
                or count != 8
                or fps != expected_fps
                or fps <= 0
                or animation.get("sheet") != expected_sheet
                or animation.get("playback") != playback
            ):
                raise ValidationError(f"{identifier}: invalid {name} animation")
            if start < 0 or count <= 0 or start + count > frame_count:
                raise ValidationError(f"{identifier}: {name} frame range is out of bounds")
    return characters


def _validate_sheet(path: Path, expected_size: tuple[int, int], columns: int, rows: int) -> None:
    _require_rgba_png(path)
    width, height, pixels = read_rgba_png(path)
    if (width, height) != expected_size:
        raise ValidationError(
            f"{path.name}: expected {expected_size[0]}x{expected_size[1]}, got {width}x{height}"
        )
    if all(pixels[index] == 255 for index in range(3, len(pixels), 4)):
        raise ValidationError(f"{path.name}: expected transparent RGBA content")
    frame_hashes: list[int] = []
    for frame in range(columns * rows):
        cell_x, cell_y = (frame % columns) * 256, (frame // columns) * 256
        cell_pixels = bytearray()
        visible = False
        touches_boundary = False
        for y in range(cell_y, cell_y + 256):
            for x in range(cell_x, cell_x + 256):
                offset = (y * width + x) * 4
                alpha = pixels[offset + 3]
                cell_pixels.extend(pixels[offset : offset + 4])
                if alpha:
                    visible = True
                    if (
                        x - cell_x < 2
                        or cell_x + 255 - x < 2
                        or y - cell_y < 2
                        or cell_y + 255 - y < 2
                    ):
                        touches_boundary = True
        if not visible:
            raise ValidationError(f"{path.name}: frame {frame} is empty")
        if touches_boundary:
            raise ValidationError(f"{path.name}: frame {frame} touches its two-pixel boundary")
        frame_hashes.append(zlib.crc32(cell_pixels))
    for row in range(rows):
        row_hashes = frame_hashes[row * columns : (row + 1) * columns]
        if len(set(row_hashes)) != columns:
            raise ValidationError(f"{path.name}: row {row} must contain distinct frames")


def _validate_selected_frames(
    path: Path, columns: int, frames: list[int], *, safe_border: bool = True
) -> list[str]:
    width, _, pixels = read_rgba_png(path)
    hashes: list[str] = []
    for frame in frames:
        cell_x, cell_y = (frame % columns) * 256, (frame // columns) * 256
        visible = False
        touches_boundary = False
        cell_pixels = bytearray()
        for y in range(cell_y, cell_y + 256):
            for x in range(cell_x, cell_x + 256):
                offset = (y * width + x) * 4
                alpha = pixels[offset + 3]
                cell_pixels.extend(pixels[offset : offset + 4])
                if alpha:
                    visible = True
                    if (
                        x - cell_x < 2
                        or cell_x + 255 - x < 2
                        or y - cell_y < 2
                        or cell_y + 255 - y < 2
                    ):
                        touches_boundary = True
        if not visible:
            raise ValidationError(f"{path.name}: required frame {frame} is empty")
        if safe_border and touches_boundary:
            raise ValidationError(f"{path.name}: required frame {frame} touches its boundary")
        hashes.append(hashlib.sha256(cell_pixels).hexdigest())
    return hashes


def validate_presentation_contract(data: dict) -> None:
    if data.get("schema_version") != 1:
        raise ValidationError("presentation manifest schema_version must be 1")
    if data.get("actors_grid") != [8, 8] or data.get("effects_grid") != [8, 4]:
        raise ValidationError("presentation: invalid atlas grid")
    if data.get("portraits_grid") != [len(FROZEN_IDS), 1]:
        raise ValidationError("presentation: portrait grid must be 10x1")
    if data.get("portrait_character_ids") != FROZEN_IDS:
        raise ValidationError("presentation: portrait IDs must match the frozen roster order")
    required_actors = {
        "green_tower",
        "blue_tower",
        "green_base_tower",
        "blue_base_tower",
        "green_minion_idle",
        "green_minion_march",
        "blue_minion_idle",
        "blue_minion_march",
        "neutral",
        "wendigo_idle",
        "wendigo_aggro",
        "king_mutatio_idle",
        "king_mutatio_aggro",
        "green_projectile",
        "blue_projectile",
    }
    actors = data.get("actors", {})
    if not required_actors.issubset(actors):
        raise ValidationError("presentation: missing required actor categories")
    for identifier, actor in actors.items():
        bounds = actor.get("occupied_bounds")
        if (
            not isinstance(bounds, list)
            or len(bounds) != 4
            or any(not isinstance(value, int) for value in bounds)
            or bounds[0] < 0
            or bounds[1] < 0
            or bounds[2] <= 0
            or bounds[3] <= 0
            or bounds[0] + bounds[2] > 256
            or bounds[1] + bounds[3] > 256
        ):
            raise ValidationError(
                f"presentation: {identifier} invalid occupied_bounds [x, y, width, height]"
            )
    required_effects = {
        "green_cast",
        "blue_cast",
        "green_hit",
        "blue_hit",
        "heal",
        "death",
    }
    if set(data.get("effects", {})) != required_effects:
        raise ValidationError("presentation: missing team cast/hit or heal/death effects")
    for field in ("actors_sheet", "effects_sheet", "arena_texture", "ui_frame", "portraits"):
        if not _safe_relative_png(data.get(field)):
            raise ValidationError(f"presentation: unsafe {field}")


def validate_presentation_directory(root: Path) -> None:
    manifest_path = root / "manifest.json"
    if not manifest_path.is_file():
        raise ValidationError("presentation2d/manifest.json is required")
    data = json.loads(manifest_path.read_text(encoding="utf-8"))
    validate_presentation_contract(data)
    required_files = {
        "actors_sheet": (2048, 2048),
        "effects_sheet": (2048, 1024),
        "arena_texture": (2048, 2048),
        "ui_frame": (1024, 1024),
        "portraits": (2560, 256),
    }
    for field, expected_size in required_files.items():
        name = data.get(field)
        if not _safe_relative_png(name):
            raise ValidationError(f"presentation: unsafe {field}")
        path = root / name
        if not path.is_file():
            raise ValidationError(f"presentation: missing {name}")
        width, height, pixels = read_rgba_png(path)
        if (width, height) != expected_size:
            raise ValidationError(
                f"presentation: {name} expected {expected_size}, got {(width, height)}"
            )
        if field != "arena_texture":
            _require_rgba_png(path)
        if field in {"actors_sheet", "effects_sheet", "ui_frame"} and all(
            pixels[index] == 255 for index in range(3, len(pixels), 4)
        ):
            raise ValidationError(f"presentation: {name} requires transparent content")
    actor_frames = 64
    actors = data.get("actors", {})
    for identifier, actor in actors.items():
        frame = actor.get("frame")
        pivot = actor.get("pivot")
        if not isinstance(frame, int) or not 0 <= frame < actor_frames:
            raise ValidationError(f"presentation: {identifier} invalid frame")
        if not 0.2 <= actor.get("world_height", 0) <= 6.5:
            raise ValidationError(f"presentation: {identifier} invalid world_height")
        if not isinstance(pivot, list) or len(pivot) != 2 or any(not 0 <= v <= 1 for v in pivot):
            raise ValidationError(f"presentation: {identifier} invalid pivot")
    actor_hashes = _validate_selected_frames(
        root / data["actors_sheet"],
        8,
        sorted({actor["frame"] for actor in actors.values()}),
    )
    if len(set(actor_hashes)) != len(actor_hashes):
        raise ValidationError("presentation: required actor frames must be distinct")
    actor_width, _, actor_pixels = read_rgba_png(root / data["actors_sheet"])
    for identifier, actor in actors.items():
        frame = actor["frame"]
        cell_x, cell_y = (frame % 8) * 256, (frame // 8) * 256
        visible_x: list[int] = []
        visible_y: list[int] = []
        for local_y in range(256):
            row = (cell_y + local_y) * actor_width
            for local_x in range(256):
                alpha = actor_pixels[(row + cell_x + local_x) * 4 + 3]
                if alpha >= OCCUPIED_ALPHA_THRESHOLD:
                    visible_x.append(local_x)
                    visible_y.append(local_y)
        if not visible_x:
            raise ValidationError(
                f"presentation: {identifier} has no pixels at alpha >= "
                f"{OCCUPIED_ALPHA_THRESHOLD}"
            )
        actual_bounds = [
            min(visible_x),
            min(visible_y),
            max(visible_x) - min(visible_x) + 1,
            max(visible_y) - min(visible_y) + 1,
        ]
        if actor["occupied_bounds"] != actual_bounds:
            raise ValidationError(
                f"presentation: {identifier} occupied_bounds must be {actual_bounds} "
                f"at alpha >= {OCCUPIED_ALPHA_THRESHOLD}"
            )
    effects = data.get("effects", {})
    for identifier, effect in effects.items():
        start, count, fps = effect.get("start"), effect.get("count"), effect.get("fps")
        if (
            not isinstance(start, int)
            or not isinstance(count, int)
            or start < 0
            or count <= 0
            or start + count > 32
            or not isinstance(fps, (int, float))
            or fps <= 0
        ):
            raise ValidationError(f"presentation: {identifier} invalid range/fps")
    effect_hashes = _validate_selected_frames(root / data["effects_sheet"], 8, list(range(32)))
    for row in range(4):
        if len(set(effect_hashes[row * 8 : (row + 1) * 8])) != 8:
            raise ValidationError(f"presentation: effect row {row} must contain distinct frames")
    portrait_hashes = _validate_selected_frames(
        root / data["portraits"], len(FROZEN_IDS), list(range(len(FROZEN_IDS))), safe_border=False
    )
    if len(set(portrait_hashes)) != len(FROZEN_IDS):
        raise ValidationError("presentation: ten portraits must be distinct")
    if portrait_hashes[: len(ORIGINAL_IDS)] != ORIGINAL_PORTRAIT_RGBA_SHA256:
        raise ValidationError("presentation: original portrait cells 0-4 changed")


def _validate_file_sets(root: Path, characters: list[dict]) -> None:
    listed = {entry["sheet"] for entry in characters}
    actual = {path.name for path in root.glob("*.png")}
    if actual != listed:
        raise ValidationError(f"sheet set mismatch: expected {sorted(listed)}, found {sorted(actual)}")
    action_listed = {entry["action_sheet"] for entry in characters}
    action_actual = {str(path.relative_to(root)) for path in (root / "actions").glob("*.png")}
    if action_actual != action_listed:
        raise ValidationError(
            f"action sheet set mismatch: expected {sorted(action_listed)}, found {sorted(action_actual)}"
        )


def _validate_original_asset_hashes(root: Path) -> None:
    for relative, expected in ORIGINAL_ASSET_SHA256.items():
        _validate_sha256(root / relative, expected, f"original sprite {relative}")


def _first_text(mapping: object, *keys: str) -> str | None:
    if not isinstance(mapping, dict):
        return None
    for key in keys:
        value = mapping.get(key)
        if isinstance(value, str) and value.strip():
            return value
    return None


def _nested_numeric_values(value: object, key: str) -> list[float]:
    found: list[float] = []
    if isinstance(value, dict):
        for current_key, current_value in value.items():
            if current_key == key and isinstance(current_value, (int, float)):
                found.append(float(current_value))
            found.extend(_nested_numeric_values(current_value, key))
    elif isinstance(value, list):
        for item in value:
            found.extend(_nested_numeric_values(item, key))
    return found


def _require_relative_source(record_dir: Path, value: object, label: str) -> Path:
    if not isinstance(value, str) or not value or Path(value).is_absolute() or ".." in Path(value).parts:
        raise ValidationError(f"{label}: missing or unsafe source path")
    path = record_dir / value
    if not path.is_file():
        raise ValidationError(f"{label}: source file {value} is missing")
    return path


def validate_provenance_record(record: dict, record_path: Path, identifier: str) -> None:
    if record.get("task_id") != TASK_ID or record.get("character_id") != identifier:
        raise ValidationError(f"{identifier}: provenance task/character cross-link is invalid")
    if record.get("license") != "CC0-1.0":
        raise ValidationError(f"{identifier}: provenance license must be CC0-1.0")
    if not _first_text(record, "designer_agent", "designer_agent_task"):
        raise ValidationError(f"{identifier}: provenance is missing designer authorship")
    if not _first_text(record, "original_work_declaration"):
        raise ValidationError(f"{identifier}: provenance is missing original-work declaration")
    pipeline = _first_text(record, "pipeline", "production_branch")
    if pipeline != "seedance-video-to-spritesheet":
        raise ValidationError(f"{identifier}: provenance pipeline is invalid")
    skill = _first_text(record, "skill", "skill_path")
    if skill != ".claude/skills/omoba-sprite-character/SKILL.md":
        raise ValidationError(f"{identifier}: provenance skill cross-link is invalid")

    master = record.get("master") or record.get("approved_master")
    master_id = _first_text(master, "job_id", "job_and_reference_media_handle", "media_id")
    if not master_id or not _first_text(
        master,
        "model",
        "requested_model",
        "returned_model",
        "catalog_model_requested",
        "result_model",
    ):
        raise ValidationError(f"{identifier}: master model/job/media ID is missing")
    master_source = _first_text(master, "source", "local_path")
    _require_relative_source(record_path.parent, master_source, f"{identifier}: master")

    portrait = record.get("portrait")
    portrait_id = _first_text(portrait, "job_id", "job_and_reference_media_handle", "media_id")
    if not portrait_id:
        raise ValidationError(f"{identifier}: portrait job/media ID is missing")
    portrait_source = _first_text(
        portrait,
        "output_256",
        "deterministic_256_source",
        "source",
        "higgsfield_source",
        "local_path",
    )
    _require_relative_source(record_path.parent, portrait_source, f"{identifier}: portrait")

    video_common = record.get("video_common")
    if not _first_text(video_common, "model", "catalog_model", "returned_parameter_model"):
        raise ValidationError(f"{identifier}: Seedance model identifier is missing")
    if not isinstance(video_common, dict) or video_common.get("requested_resolution") != "720p":
        raise ValidationError(f"{identifier}: Seedance source resolution must be recorded as 720p")
    states = record.get("states") or record.get("clips")
    expected_states = {"idle", "run", "attack", "cast", "hit", "death"}
    if not isinstance(states, dict) or set(states) != expected_states:
        raise ValidationError(f"{identifier}: provenance must contain exactly six source clips")
    inline_frame_map = True
    for state in expected_states:
        source = states[state]
        if not _first_text(source, "job_id", "media_id"):
            raise ValidationError(f"{identifier}: {state} job/media ID is missing")
        source_path = _first_text(source, "source", "local_path")
        if _require_relative_source(
            record_path.parent, source_path, f"{identifier}: {state}"
        ).suffix.lower() != ".mp4":
            raise ValidationError(f"{identifier}: {state} source clip must be MP4")
        source_hash = _first_text(source, "sha256")
        if not source_hash:
            raise ValidationError(f"{identifier}: {state} source hash is missing")
        if _sha256_file(record_path.parent / source_path) != source_hash:
            raise ValidationError(f"{identifier}: {state} source hash does not match")
        timecodes = source.get("timecodes") or source.get("timecodes_seconds")
        source_frames = source.get("source_frames") or source.get("source_frames_at_24fps")
        if not (
            isinstance(timecodes, list)
            and len(timecodes) == 8
            and isinstance(source_frames, list)
            and len(source_frames) == 8
        ):
            inline_frame_map = False
    if not inline_frame_map:
        frame_selection = _first_text(record, "frame_selection")
        _require_relative_source(
            record_path.parent, frame_selection, f"{identifier}: frame selection"
        )

    costs = record.get("cost") or record.get("costs")
    if not costs or not _nested_numeric_values(costs, "total"):
        raise ValidationError(f"{identifier}: exact cost total is missing")
    unit_text = json.dumps(costs).lower()
    if "credit" not in unit_text:
        raise ValidationError(f"{identifier}: exact cost unit is missing")


def _validate_provenance(repo_root: Path, characters: list[dict]) -> None:
    for entry in characters:
        identifier = entry["id"]
        if identifier not in NEW_IDS:
            continue
        relative = Path(".agent") / "tasks" / TASK_ID / "raw" / identifier / "provenance.json"
        record_path = repo_root / relative
        if not record_path.is_file():
            raise ValidationError(f"{identifier}: missing {relative}")
        validate_provenance_record(
            json.loads(record_path.read_text(encoding="utf-8")), record_path, identifier
        )


def _validate_license_coverage(license_path: Path) -> None:
    license_text = license_path.read_text(encoding="utf-8")
    for identifier in FROZEN_IDS:
        if f"{identifier}.png" not in license_text or f"actions/{identifier}.png" not in license_text:
            raise ValidationError(f"license: missing locomotion/action coverage for {identifier}")


def _validate_runtime_budget(directories: tuple[Path, ...]) -> None:
    total = sum(
        path.stat().st_size
        for directory in directories
        for path in directory.rglob("*")
        if path.is_file()
    )
    if total > RUNTIME_BUDGET_BYTES:
        raise ValidationError(f"2D asset directories are {total} bytes, exceeding 80 MiB")


def validate_directory(root: Path) -> None:
    manifest_path = root / "manifest.json"
    license_path = root / "LICENSE.md"
    if not manifest_path.is_file() or not license_path.is_file():
        raise ValidationError("manifest.json and LICENSE.md are required")
    characters = validate_manifest(json.loads(manifest_path.read_text(encoding="utf-8")))
    _validate_file_sets(root, characters)
    _validate_original_asset_hashes(root)
    _validate_license_coverage(license_path)
    for entry in characters:
        _validate_sheet(root / entry["sheet"], (2048, 512), 8, 2)
        _validate_sheet(root / entry["action_sheet"], (2048, 1024), 8, 4)
    presentation_root = root.parent / "presentation2d"
    validate_presentation_directory(presentation_root)
    _validate_provenance(root.resolve().parents[2], characters)
    _validate_runtime_budget((root, presentation_root))


class NegativeContractTests(unittest.TestCase):
    def base(self) -> dict:
        source = Path(__file__).resolve().parents[1] / "client/assets/sprites/manifest.json"
        return json.loads(source.read_text(encoding="utf-8"))

    def assert_invalid(self, mutate) -> None:
        data = self.base()
        mutate(data)
        with self.assertRaises(ValidationError):
            validate_manifest(data)

    def test_duplicate_or_unknown_id(self) -> None:
        self.assert_invalid(lambda data: data["characters"][1].update(id=data["characters"][0]["id"]))
        self.assert_invalid(lambda data: data["characters"][1].update(id="unknown"))

    def test_reordered_id(self) -> None:
        def reorder(data: dict) -> None:
            data["characters"][5], data["characters"][6] = (
                data["characters"][6],
                data["characters"][5],
            )

        self.assert_invalid(reorder)

    def test_bad_frame_bounds(self) -> None:
        self.assert_invalid(lambda data: data["characters"][0]["animations"]["run"].update(start=15))

    def test_bad_dimensions(self) -> None:
        self.assert_invalid(lambda data: data["characters"][0].update(frame_size=[128, 256]))

    def test_unreadable_world_height(self) -> None:
        self.assert_invalid(lambda data: data["characters"][0].update(world_height=0.5))
        self.assert_invalid(lambda data: data["characters"][0].update(world_height=9.0))

    def test_missing_required_action(self) -> None:
        self.assert_invalid(lambda data: data["characters"][0]["animations"].pop("death"))

    def test_bad_playback(self) -> None:
        self.assert_invalid(
            lambda data: data["characters"][0]["animations"]["death"].update(playback="loop")
        )

    def test_missing_provenance_cross_link(self) -> None:
        self.assert_invalid(
            lambda data: data["characters"][5].update(provenance="Higgsfield original")
        )

    def test_unsafe_path(self) -> None:
        self.assert_invalid(lambda data: data["characters"][0].update(sheet="../escape.png"))
        self.assert_invalid(
            lambda data: data["characters"][0].update(action_sheet="actions/../../escape.png")
        )

    def test_missing_presentation_category(self) -> None:
        source = (
            Path(__file__).resolve().parents[1]
            / "client/assets/presentation2d/manifest.json"
        )
        data = json.loads(source.read_text(encoding="utf-8"))
        data["actors"].pop("wendigo_idle")
        with self.assertRaises(ValidationError):
            validate_presentation_contract(data)

    def test_missing_or_out_of_cell_presentation_bounds(self) -> None:
        source = (
            Path(__file__).resolve().parents[1]
            / "client/assets/presentation2d/manifest.json"
        )
        data = json.loads(source.read_text(encoding="utf-8"))
        data["actors"]["green_minion_idle"].pop("occupied_bounds")
        with self.assertRaises(ValidationError):
            validate_presentation_contract(data)
        data = json.loads(source.read_text(encoding="utf-8"))
        data["actors"]["green_minion_idle"]["occupied_bounds"] = [240, 240, 32, 32]
        with self.assertRaises(ValidationError):
            validate_presentation_contract(data)

    def test_portrait_order_or_count_mismatch(self) -> None:
        source = (
            Path(__file__).resolve().parents[1]
            / "client/assets/presentation2d/manifest.json"
        )
        data = json.loads(source.read_text(encoding="utf-8"))
        data["portraits_grid"] = [5, 1]
        with self.assertRaises(ValidationError):
            validate_presentation_contract(data)
        data = json.loads(source.read_text(encoding="utf-8"))
        data["portrait_character_ids"][5], data["portrait_character_ids"][6] = (
            data["portrait_character_ids"][6],
            data["portrait_character_ids"][5],
        )
        with self.assertRaises(ValidationError):
            validate_presentation_contract(data)

    @staticmethod
    def _write_rgba_png(path: Path, width: int, height: int, pixels: bytes) -> None:
        def chunk(kind: bytes, payload: bytes) -> bytes:
            return (
                struct.pack(">I", len(payload))
                + kind
                + payload
                + struct.pack(">I", zlib.crc32(kind + payload) & 0xFFFFFFFF)
            )

        scanlines = b"".join(
            b"\x00" + pixels[y * width * 4 : (y + 1) * width * 4]
            for y in range(height)
        )
        path.write_bytes(
            PNG_SIGNATURE
            + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(scanlines, 1))
            + chunk(b"IEND", b"")
        )

    @staticmethod
    def _valid_sheet_pixels(columns: int = 8, rows: int = 2) -> bytearray:
        width, height = columns * 256, rows * 256
        pixels = bytearray(width * height * 4)
        for frame in range(columns * rows):
            cell_x = (frame % columns) * 256
            cell_y = (frame // columns) * 256
            x, y = cell_x + 80 + frame, cell_y + 100
            offset = (y * width + x) * 4
            pixels[offset : offset + 4] = bytes((frame + 1, 64, 128, 255))
        return pixels

    def test_opaque_empty_boundary_and_duplicate_frames(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / "sheet.png"
            width, height = 2048, 512

            opaque = bytearray([255]) * (width * height * 4)
            self._write_rgba_png(path, width, height, opaque)
            with self.assertRaises(ValidationError):
                _validate_sheet(path, (width, height), 8, 2)

            empty = self._valid_sheet_pixels()
            empty_offset = ((100 * width) + 80) * 4
            empty[empty_offset : empty_offset + 4] = b"\x00\x00\x00\x00"
            self._write_rgba_png(path, width, height, empty)
            with self.assertRaises(ValidationError):
                _validate_sheet(path, (width, height), 8, 2)

            boundary = self._valid_sheet_pixels()
            boundary_offset = ((100 * width) + 1) * 4
            boundary[boundary_offset : boundary_offset + 4] = b"\x20\x40\x60\xff"
            self._write_rgba_png(path, width, height, boundary)
            with self.assertRaises(ValidationError):
                _validate_sheet(path, (width, height), 8, 2)

            duplicate = self._valid_sheet_pixels()
            for y in range(256):
                row_start = y * width * 4
                duplicate[row_start + 256 * 4 : row_start + 512 * 4] = duplicate[
                    row_start : row_start + 256 * 4
                ]
            self._write_rgba_png(path, width, height, duplicate)
            with self.assertRaises(ValidationError):
                _validate_sheet(path, (width, height), 8, 2)

    @staticmethod
    def _provenance_fixture(record_dir: Path) -> dict:
        (record_dir / "master").mkdir(parents=True, exist_ok=True)
        (record_dir / "portrait").mkdir(exist_ok=True)
        (record_dir / "clips").mkdir(exist_ok=True)
        (record_dir / "master/master.png").write_bytes(b"master")
        (record_dir / "portrait/portrait.png").write_bytes(b"portrait")
        states = {}
        for state in ("idle", "run", "attack", "cast", "hit", "death"):
            (record_dir / f"clips/{state}.mp4").write_bytes(state.encode())
            states[state] = {
                "media_id": f"media-{state}",
                "source": f"clips/{state}.mp4",
                "sha256": hashlib.sha256(state.encode()).hexdigest(),
                "timecodes": list(range(8)),
                "source_frames": list(range(8)),
            }
        return {
            "task_id": TASK_ID,
            "character_id": NEW_IDS[0],
            "license": "CC0-1.0",
            "designer_agent": "/root/test-designer",
            "original_work_declaration": "Original Higgsfield work.",
            "pipeline": "seedance-video-to-spritesheet",
            "skill": ".claude/skills/omoba-sprite-character/SKILL.md",
            "master": {
                "media_id": "media-master",
                "model": "recraft_v4_1",
                "source": "master/master.png",
            },
            "portrait": {"media_id": "media-portrait", "source": "portrait/portrait.png"},
            "video_common": {"model": "seedance_2_0", "requested_resolution": "720p"},
            "states": states,
            "cost": {"unit": "credits", "total": 100.0},
        }

    def test_missing_provenance_job_media_cost_or_source_clip(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            record_dir = Path(temporary)
            record_path = record_dir / "provenance.json"

            record = self._provenance_fixture(record_dir)
            record["master"].pop("media_id")
            with self.assertRaises(ValidationError):
                validate_provenance_record(record, record_path, NEW_IDS[0])

            record = self._provenance_fixture(record_dir)
            record["portrait"].pop("media_id")
            with self.assertRaises(ValidationError):
                validate_provenance_record(record, record_path, NEW_IDS[0])

            record = self._provenance_fixture(record_dir)
            record.pop("cost")
            with self.assertRaises(ValidationError):
                validate_provenance_record(record, record_path, NEW_IDS[0])

            record = self._provenance_fixture(record_dir)
            (record_dir / "clips/death.mp4").unlink()
            with self.assertRaises(ValidationError):
                validate_provenance_record(record, record_path, NEW_IDS[0])

    def test_orphan_files_altered_hash_and_size_overflow(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "sprites"
            (root / "actions").mkdir(parents=True)
            characters = self.base()["characters"]
            for entry in characters:
                (root / entry["sheet"]).touch()
                (root / entry["action_sheet"]).touch()
            (root / "orphan.png").touch()
            with self.assertRaises(ValidationError):
                _validate_file_sets(root, characters)

            (root / "orphan.png").unlink()
            missing_sheet = root / characters[0]["sheet"]
            missing_sheet.unlink()
            with self.assertRaises(ValidationError):
                _validate_file_sets(root, characters)
            missing_sheet.touch()
            missing_action = root / characters[0]["action_sheet"]
            missing_action.unlink()
            with self.assertRaises(ValidationError):
                _validate_file_sets(root, characters)

            changed = root / "changed.bin"
            changed.write_bytes(b"changed")
            with self.assertRaises(ValidationError):
                _validate_sha256(changed, "0" * 64, "original sprite")

            oversized = root / "oversized.bin"
            with oversized.open("wb") as output:
                output.truncate(RUNTIME_BUDGET_BYTES + 1)
            with self.assertRaises(ValidationError):
                _validate_runtime_budget((root,))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "root",
        nargs="?",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "client/assets/sprites",
    )
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(NegativeContractTests)
        if not unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful():
            return 1
    try:
        validate_directory(args.root)
    except (OSError, json.JSONDecodeError, ValidationError) as error:
        print(f"sprite validation failed: {error}", file=sys.stderr)
        return 1
    print(f"sprite validation passed: {args.root}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
