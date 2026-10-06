#!/usr/bin/env python3
"""Contact sheets for a `capture_standard_skills.py --phases` run.

    python3 scripts/build_skill_contact_sheets.py /path/to/run

Writes into `<run>/sheets/`: one sheet per captured class (rows Q/W/E/R,
columns windup, release, impact or settled) and `wall-release.png`, the release
still of every captured skill (one row per class). Every tile is the same crop
of the 1280x720 still (the staged pair without the HUD), so tiles are
comparable across skills and runs. A missing or black still is an error, never
an empty tile. Needs Pillow.
"""
import argparse
import json
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont, ImageStat

from capture_standard_skills import HEROES, SLOT_KEYS

# The camera keeps the hero at the centre of the frame (640, 342) and its target
# stands up to seven units up the lane (857, 183 at the close zoom). This box
# holds both with room for a displaced target and leaves the HUD outside.
CROP = (330, 55, 1130, 505)
TILE = (400, 225)
WALL_TILE = (320, 180)
LABEL = 18
HEADER = 30
GAP = 4
INK, PAPER, MUTED = (236, 236, 228), (18, 20, 22), (150, 156, 150)
COLUMNS = ("windup", "release", "impact or settled")


def tile(path, size):
    """The fixed stage crop of one still, scaled to `size`."""
    with Image.open(path) as image:
        image = image.convert("RGB")
        if sum(ImageStat.Stat(image).mean) == 0:
            raise ValueError(f"{path} is black")
        return image.crop(CROP).resize(size, Image.LANCZOS)


def canvas(columns, rows, size, margin=0):
    width = margin + columns * (size[0] + GAP) + GAP
    height = HEADER + rows * (size[1] + LABEL + GAP) + GAP
    image = Image.new("RGB", (width, height), PAPER)
    return image, ImageDraw.Draw(image)


def place(sheet, draw, image, column, row, size, text, margin=0):
    x = margin + GAP + column * (size[0] + GAP)
    y = HEADER + row * (size[1] + LABEL + GAP)
    sheet.paste(image, (x, y))
    draw.text((x + 2, y + size[1] + 2), text, fill=MUTED, font=ImageFont.load_default(12))


def stills_of(run, hero):
    record = json.loads((run / hero / "capture-run.json").read_text())
    if not record.get("pass"):
        raise ValueError(f"{hero}: the capture run did not pass")
    return [still for still in record["stills"] if still["phase"] != "idle"]


def class_sheet(run, hero):
    stills = stills_of(run, hero)
    sheet, draw = canvas(3, len(SLOT_KEYS), TILE)
    draw.text((GAP + 2, 6), f"{hero}  ·  columns: {', '.join(COLUMNS)}", fill=INK,
              font=ImageFont.load_default(16))
    for slot, key in enumerate(SLOT_KEYS):
        row = sorted((s for s in stills if s["slot"] == slot), key=lambda s: s["file"])
        if len(row) != 3:
            raise ValueError(f"{hero} {key}: expected three stills, found {len(row)}")
        for column, still in enumerate(row):
            text = f"{key.upper()} {still['skill']} · {still['phase']} · {still['animation'] or '-'}"
            place(sheet, draw, tile(run / hero / still["file"], TILE), column, slot, TILE, text)
    return sheet


def release_wall(run, heroes):
    margin = 104
    sheet, draw = canvas(len(SLOT_KEYS), len(heroes), WALL_TILE, margin)
    draw.text((GAP + 2, 6), f"release stills · {len(heroes) * len(SLOT_KEYS)} skills", fill=INK,
              font=ImageFont.load_default(16))
    for row, hero in enumerate(heroes):
        draw.text((GAP + 2, HEADER + row * (WALL_TILE[1] + LABEL + GAP) + 4), hero, fill=INK,
                  font=ImageFont.load_default(14))
        releases = {s["slot"]: s for s in stills_of(run, hero) if s["phase"] == "release"}
        if sorted(releases) != list(range(len(SLOT_KEYS))):
            raise ValueError(f"{hero}: expected one release still per slot")
        for slot, key in enumerate(SLOT_KEYS):
            still = releases[slot]
            place(sheet, draw, tile(run / hero / still["file"], WALL_TILE), slot, row, WALL_TILE,
                  f"{key.upper()} {still['skill']}", margin)
    return sheet


def build(run):
    """Write the sheets of `run` and return their index (also saved as sheets/index.json)."""
    heroes = [hero for hero in HEROES if (run / hero / "capture-run.json").is_file()]
    if not heroes:
        raise ValueError(f"{run} has no phase capture")
    sheets = run / "sheets"
    sheets.mkdir(exist_ok=True)
    index = dict(crop=CROP, tile=TILE, wall_tile=WALL_TILE, classes={}, skills=len(heroes) * len(SLOT_KEYS))
    for hero in heroes:
        class_sheet(run, hero).save(sheets / f"{hero}.png")
        index["classes"][hero] = f"{hero}.png"
    release_wall(run, heroes).save(sheets / "wall-release.png")
    index["wall"] = "wall-release.png"
    (sheets / "index.json").write_text(json.dumps(index, indent=2) + "\n")
    return index


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("run", type=Path, help="output directory of capture_standard_skills.py --phases")
    args = parser.parse_args(argv)
    try:
        index = build(args.run.resolve())
    except (ValueError, OSError) as error:
        raise SystemExit(f"contact sheets: {error}")
    print(json.dumps(index, indent=2))


if __name__ == "__main__":
    main()
