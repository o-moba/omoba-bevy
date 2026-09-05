#!/usr/bin/env python3
"""Validate occupied-pixel readability for Omoba's genuine 2D actors."""

from __future__ import annotations

import argparse
import json
import statistics
import sys
import unittest
from pathlib import Path

from validate_sprite_assets import read_rgba_png

CELL_PIXELS = 256
ALPHA_THRESHOLD = 16
CAMERA2D_BASE_SCALE = 0.08
DEFAULT_ZOOM = 1.0
MAX_ZOOM_OUT = 2.25

HERO_MIN_MAJOR_PX_AT_MAX_ZOOM = 7.0
ACTOR_MIN_MAJOR_PX_AT_MAX_ZOOM = {
    "tower": 12.0,
    "base": 14.0,
    "minion": 5.0,
    "neutral": 5.0,
    "boss": 14.0,
    "projectile": 2.0,
}


class ReadabilityError(ValueError):
    pass


def occupied_bounds(
    pixels: bytes,
    image_width: int,
    frame: int,
    columns: int,
    *,
    alpha_threshold: int = ALPHA_THRESHOLD,
) -> tuple[int, int, int, int]:
    cell_x = (frame % columns) * CELL_PIXELS
    cell_y = (frame // columns) * CELL_PIXELS
    min_x = min_y = CELL_PIXELS
    max_x = max_y = -1
    for local_y in range(CELL_PIXELS):
        row = (cell_y + local_y) * image_width
        for local_x in range(CELL_PIXELS):
            alpha = pixels[(row + cell_x + local_x) * 4 + 3]
            if alpha >= alpha_threshold:
                min_x = min(min_x, local_x)
                min_y = min(min_y, local_y)
                max_x = max(max_x, local_x)
                max_y = max(max_y, local_y)
    if max_x < min_x or max_y < min_y:
        raise ReadabilityError(f"frame {frame} has no alpha >= {alpha_threshold}")
    return min_x, min_y, max_x + 1, max_y + 1


def projected_major_pixels(bounds: tuple[int, int, int, int], world_height: float, zoom: float) -> float:
    width = bounds[2] - bounds[0]
    height = bounds[3] - bounds[1]
    occupied_ratio = max(width, height) / CELL_PIXELS
    return occupied_ratio * world_height / (CAMERA2D_BASE_SCALE * zoom)


def classify_actor(identifier: str) -> str:
    if "base_tower" in identifier:
        return "base"
    if identifier.endswith("tower"):
        return "tower"
    if "minion" in identifier:
        return "minion"
    if identifier.startswith(("wendigo", "king_mutatio")):
        return "boss"
    if "projectile" in identifier:
        return "projectile"
    return "neutral"


def validate(repo_root: Path) -> list[dict[str, object]]:
    sprite_root = repo_root / "client/assets/sprites"
    presentation_root = repo_root / "client/assets/presentation2d"
    sprite_manifest = json.loads((sprite_root / "manifest.json").read_text(encoding="utf-8"))
    presentation = json.loads((presentation_root / "manifest.json").read_text(encoding="utf-8"))
    report: list[dict[str, object]] = []

    for character in sprite_manifest["characters"]:
        values: list[float] = []
        for field in ("sheet", "action_sheet"):
            path = sprite_root / character[field]
            if not path.is_file():
                raise ReadabilityError(f"{character['id']}: missing runtime sheet {path.name}")
            width, height, pixels = read_rgba_png(path)
            frames = (width // CELL_PIXELS) * (height // CELL_PIXELS)
            for frame in range(frames):
                bounds = occupied_bounds(pixels, width, frame, width // CELL_PIXELS)
                values.append(
                    projected_major_pixels(bounds, float(character["world_height"]), MAX_ZOOM_OUT)
                )
        minimum = min(values)
        if minimum < HERO_MIN_MAJOR_PX_AT_MAX_ZOOM:
            raise ReadabilityError(
                f"{character['id']}: worst-state major axis {minimum:.2f}px is below "
                f"{HERO_MIN_MAJOR_PX_AT_MAX_ZOOM:.2f}px at maximum zoom-out"
            )
        report.append(
            {
                "id": character["id"],
                "category": "hero",
                "min_major_px_max_zoom": round(minimum, 2),
                "median_major_px_max_zoom": round(statistics.median(values), 2),
            }
        )

    actor_path = presentation_root / presentation["actors_sheet"]
    width, _, pixels = read_rgba_png(actor_path)
    for identifier, actor in sorted(presentation["actors"].items()):
        bounds = occupied_bounds(pixels, width, int(actor["frame"]), presentation["actors_grid"][0])
        default_pixels = projected_major_pixels(bounds, float(actor["world_height"]), DEFAULT_ZOOM)
        max_zoom_pixels = projected_major_pixels(bounds, float(actor["world_height"]), MAX_ZOOM_OUT)
        category = classify_actor(identifier)
        minimum = ACTOR_MIN_MAJOR_PX_AT_MAX_ZOOM[category]
        if max_zoom_pixels < minimum:
            raise ReadabilityError(
                f"{identifier}: occupied major axis {max_zoom_pixels:.2f}px is below "
                f"{minimum:.2f}px at maximum zoom-out"
            )
        report.append(
            {
                "id": identifier,
                "category": category,
                "major_px_default_zoom": round(default_pixels, 2),
                "major_px_max_zoom": round(max_zoom_pixels, 2),
            }
        )

    tower_min = min(
        float(item["major_px_max_zoom"])
        for item in report
        if item["category"] == "tower"
    )
    hero_median = statistics.median(
        float(item["median_major_px_max_zoom"])
        for item in report
        if item["category"] == "hero"
    )
    minion_max = max(
        float(item["major_px_max_zoom"])
        for item in report
        if item["category"] == "minion"
    )
    if not tower_min > hero_median > minion_max:
        raise ReadabilityError(
            "occupied-pixel hierarchy must be tower > median hero > minion at maximum zoom-out "
            f"(got {tower_min:.2f} > {hero_median:.2f} > {minion_max:.2f})"
        )
    return report


class ContractTests(unittest.TestCase):
    def test_projection_uses_occupied_axis_and_camera_zoom(self) -> None:
        bounds = (20, 30, 148, 158)
        self.assertAlmostEqual(projected_major_pixels(bounds, 2.0, 1.0), 12.5)
        self.assertAlmostEqual(projected_major_pixels(bounds, 2.0, 2.0), 6.25)

    def test_actor_categories_are_stable(self) -> None:
        self.assertEqual(classify_actor("green_tower"), "tower")
        self.assertEqual(classify_actor("blue_base_tower"), "base")
        self.assertEqual(classify_actor("green_minion_march"), "minion")
        self.assertEqual(classify_actor("wendigo_idle"), "boss")
        self.assertEqual(classify_actor("green_projectile"), "projectile")

    def test_empty_frame_is_rejected(self) -> None:
        with self.assertRaises(ReadabilityError):
            occupied_bounds(bytes(CELL_PIXELS * CELL_PIXELS * 4), CELL_PIXELS, 0, 1)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=Path(__file__).resolve().parents[1],
    )
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(ContractTests)
        if not unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful():
            return 1
    try:
        report = validate(args.repo_root)
    except (OSError, json.JSONDecodeError, ReadabilityError, ValueError) as error:
        print(f"2D readability validation failed: {error}", file=sys.stderr)
        return 1
    if args.json:
        print(json.dumps(report, indent=2))
    else:
        for item in report:
            print(" ".join(f"{key}={value}" for key, value in item.items()))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
