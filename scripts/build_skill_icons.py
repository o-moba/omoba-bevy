#!/usr/bin/env python3
"""Pack approved GPT Image 2.5 sheets; never generate placeholder artwork.

Requires the existing developer Pillow installation (no runtime dependency).
python3 scripts/build_skill_icons.py --sources <downloaded-source-directory>
python3 scripts/build_skill_icons.py --check
"""
import argparse
import hashlib
import json
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / "client/assets"
MANIFEST = ASSETS / "ui/skills/manifest.json"


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def mask(size):
    alpha = Image.new("L", (size * 4, size * 4))
    ImageDraw.Draw(alpha).ellipse((8, 8, size * 4 - 9, size * 4 - 9), fill=255)
    return alpha.resize((size, size), Image.Resampling.LANCZOS)


def build(manifest, directory):
    size = manifest["cell_size"]
    rows = {}
    for source in manifest["sources"]:
        path = directory / source["file"]
        if sha256(path) != source["sha256"]:
            raise ValueError(f"Unapproved source bytes: {path}")
        with Image.open(path) as original:
            if list(original.size) != source["dimensions"]:
                raise ValueError(f"Source dimensions changed: {path}")
            width, height = original.size
            if width != height or width % 2:
                raise ValueError(f"Expected an even square 2x2 sheet: {path}")
            cell = width // 2
            icons = []
            for index in range(4):
                x, y = index % 2 * cell, index // 2 * cell
                tile = original.crop((x, y, x + cell, y + cell)).convert("RGBA")
                tile = tile.resize((size, size), Image.Resampling.LANCZOS)
                tile.putalpha(mask(size))
                icons.append(tile)
            rows[source["class_id"]] = icons
    for replacement in manifest.get("overrides", []):
        path = directory / replacement["file"]
        if sha256(path) != replacement["sha256"]:
            raise ValueError(f"Unapproved replacement bytes: {path}")
        with Image.open(path) as original:
            if list(original.size) != replacement["dimensions"]:
                raise ValueError(f"Replacement dimensions changed: {path}")
            tile = original.convert("RGBA").resize((size, size), Image.Resampling.LANCZOS)
            tile.putalpha(mask(size))
        source = next(s for s in manifest["sources"]
                      if replacement["ability_id"] in s["abilities"])
        rows[source["class_id"]][source["abilities"].index(replacement["ability_id"])] = tile
    for atlas in manifest["atlases"]:
        packed = Image.new("RGBA", (4 * size, len(atlas["classes"]) * size))
        for row, class_id in enumerate(atlas["classes"]):
            for col, tile in enumerate(rows[class_id]):
                packed.paste(tile, (col * size, row * size))
        path = ASSETS / atlas["path"]
        packed.save(path, optimize=True)
        atlas.update(width=packed.width, height=packed.height, sha256=sha256(path))
    MANIFEST.write_text(json.dumps(manifest, indent=2) + "\n")


def check(manifest):
    size = manifest["cell_size"]
    ids = [ability for source in manifest["sources"] for ability in source["abilities"]]
    descriptions = json.loads((ROOT / "client/i18n/en/ability.json").read_text())
    expected = {key.removeprefix("ability.").removesuffix(".name")
                for key in descriptions if key.endswith(".name")}
    if len(ids) != len(set(ids)) or set(ids) != expected:
        raise ValueError("Missing or duplicated gameplay skill art")
    classes = [source["class_id"] for source in manifest["sources"]]
    if classes != [name for atlas in manifest["atlases"] for name in atlas["classes"]]:
        raise ValueError("Atlas class order differs from approved sources")
    pixels = set()
    for atlas in manifest["atlases"]:
        path = ASSETS / atlas["path"]
        if sha256(path) != atlas["sha256"]:
            raise ValueError(f"Atlas digest changed: {path}")
        with Image.open(path) as image:
            expected_size = (4 * size, len(atlas["classes"]) * size)
            if image.mode != "RGBA" or image.size != expected_size:
                raise ValueError(f"Invalid atlas dimensions/mode: {path}")
            for row in range(atlas["rows"]):
                for col in range(4):
                    icon = image.crop((col*size, row*size, (col+1)*size, (row+1)*size))
                    digest = hashlib.sha256(icon.tobytes()).hexdigest()
                    if digest in pixels or icon.getpixel((0, 0))[3] != 0:
                        raise ValueError(f"Duplicate icon or opaque corner: {path}, {row}, {col}")
                    pixels.add(digest)
    if len(pixels) != len(ids):
        raise ValueError("Icon count does not match skill count")
    print(json.dumps(dict(result="PASS", skills=len(ids), classes=len(classes),
                          atlases=len(manifest["atlases"]), cell_size=size)))


def contact_sheet(manifest, output):
    # Actual 64px faces, plus labels outside the art; two groups of eight rows.
    sheet = Image.new("RGB", (1160, 880), "#071b19")
    draw = ImageDraw.Draw(sheet)
    font = ImageFont.load_default()
    sources = {source["class_id"]: source for source in manifest["sources"]}
    row_index = 0
    size = manifest["cell_size"]
    for atlas in manifest["atlases"]:
        with Image.open(ASSETS / atlas["path"]) as image:
            for row, class_id in enumerate(atlas["classes"]):
                x = 24 + (row_index // 8) * 580
                y = 20 + (row_index % 8) * 108
                draw.text((x, y), class_id.upper(), fill="#e6cf9c", font=font)
                for col, ability in enumerate(sources[class_id]["abilities"]):
                    tile = image.crop((col*size, row*size, (col+1)*size, (row+1)*size))
                    tile = tile.resize((64, 64), Image.Resampling.LANCZOS)
                    sheet.paste(tile, (x + col*136, y + 18), tile)
                    draw.text((x + col*136, y + 85), ability, fill="#d6e5df", font=font)
                row_index += 1
    sheet.save(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sources", type=Path)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--contact-sheet", type=Path)
    args = parser.parse_args()
    if not (args.sources or args.check or args.contact_sheet):
        parser.error("Choose --sources, --check or --contact-sheet")
    manifest = json.loads(MANIFEST.read_text())
    if args.sources:
        build(manifest, args.sources)
    check(manifest)
    if args.contact_sheet:
        contact_sheet(manifest, args.contact_sheet)


if __name__ == "__main__":
    main()
