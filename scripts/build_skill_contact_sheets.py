#!/usr/bin/env python3
"""Contact sheets and the look-alike report for a `capture_standard_skills.py --phases` run.

    python3 scripts/build_skill_contact_sheets.py /path/to/run [--baseline /path/to/earlier/run]

Writes into `<run>/sheets/`: one sheet per captured class (rows Q/W/E/R,
columns windup, release, impact or settled) and `wall-release.png`, the release
still of every captured skill (one row per class). Every tile is the same crop
of the 1280x720 still (the staged pair without the HUD), so tiles are
comparable across skills and runs; a tile taken from farther away says so in
its label. A missing or black still is an error, never an empty tile.

`lookalike.json` and `lookalike.md` compare the release tiles: a difference hash
per tile, the nearest tile of the same class and of the whole run and, with
`--baseline`, the distance to the same skill's tile of that run. A distance of
zero is an error (exit code 1); every other number is a report. The energy
columns give the changed area of the release and third stills against the idle
still of their view, for this run and for the baseline. An energy is compared
with the baseline only when both stills show the stage from the closest view:
a still taken from farther away, or after the cast moved the hero and the camera
with it, is listed with its reason instead. Needs Pillow.
"""
import argparse
import itertools
import json
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFont, ImageStat

from capture_standard_skills import HEROES, MIXED_NAME, SLOT_KEYS

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
# Columns and rows of the difference hash: 576 comparisons between square cells of 25 pixels
# of the crop. The staged pair takes a small part of the crop: on a 16 x 16 grid two casts
# with different poses and particles came out equal.
HASH_GRID = (32, 18)
# Pairs of one class the report lists by name, nearest first.
CLOSEST_LISTED = 12
# A pixel has changed when one of its channels differs by more than this
# (`CHANGED_LEVEL` in `client/src/qa/standard_kits_qa.rs`).
CHANGED_LEVEL = 32
# The closest camera distance (`CAMERA_MIN_ZOOM`); a still taken from farther away has its own idle still.
CLOSE_ZOOM = 0.55


def tile(path, size):
    """The fixed stage crop of one still, scaled to `size`."""
    with Image.open(path) as image:
        image = image.convert("RGB")
        if sum(ImageStat.Stat(image).mean) == 0:
            raise ValueError(f"{path} is black")
        return image.crop(CROP).resize(size, Image.LANCZOS)


def stage(path):
    """The stage crop of one still."""
    with Image.open(path) as image:
        return image.convert("RGB").crop(CROP)


def difference_hash(image):
    """One bit per pair of neighbours in a row of a grey thumbnail: set where the left one is brighter."""
    columns, rows = HASH_GRID
    small = image.convert("L").resize((columns + 1, rows), Image.LANCZOS)
    pixels = list(small.getdata())
    bits = 0
    for row in range(rows):
        for column in range(columns):
            at = row * (columns + 1) + column
            bits = (bits << 1) | (pixels[at] > pixels[at + 1])
    return bits


def distance(a, b):
    return bin(a ^ b).count("1")


def changed_pixels(still, idle):
    """Pixels of a stage crop that differ from the idle crop of the same view: the energy of a still."""
    bands = ImageChops.difference(still, idle).split()
    most = ImageChops.lighter(ImageChops.lighter(bands[0], bands[1]), bands[2])
    return sum(most.histogram()[CHANGED_LEVEL + 1:])


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
    """The three stills of every cast of one class folder."""
    record = json.loads((run / hero / "capture-run.json").read_text())
    if not record.get("pass"):
        raise ValueError(f"{hero}: the capture run did not pass")
    return [still for still in record["stills"] if still["phase"] in ("windup", "release", "impact", "settled")]


def widened(still):
    return (still.get("zoom") or CLOSE_ZOOM) > CLOSE_ZOOM + 1e-3


def view(still):
    """`close` for the view the idle baseline has; else why the still shows something else."""
    if widened(still):
        return f"zoom {still['zoom']:.2f}"
    # The camera follows the hero: away from its spot the whole ground differs from the idle still.
    return "moved" if (still.get("hero_from_home") or 0) > 0.05 else "close"


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
            # The time since the accepted cast orders the three stills: an instant hit is
            # captured before the release window.
            after = still.get("since_edge_secs")
            text = (f"{key.upper()} {still['skill']} · {still['phase']} · {still['animation'] or '-'}"
                    + (f" · +{after:.2f}s" if after is not None else "")
                    + (f" · zoom {still['zoom']:.2f}" if widened(still) else ""))
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
                  f"{key.upper()} {still['skill']}" + (f" · zoom {still['zoom']:.2f}" if widened(still) else ""),
                  margin)
    return sheet


def folders(run):
    """The captured class folders of a run, in catalog order, and the mixed recipe after them."""
    return [hero for hero in HEROES + [MIXED_NAME] if (run / hero / "capture-run.json").is_file()]


def energy(run, hero, still):
    """The changed area of one still against the idle still of its view."""
    return changed_pixels(stage(run / hero / still["file"]), stage(run / hero / (still.get("idle_file") or "0-idle.png")))


def lookalike(run, heroes, baseline=None):
    """How far apart the release tiles of a run are, and the energy of each cast against an earlier run."""
    known = folders(baseline) if baseline else []
    skills = {}
    for hero in heroes:
        stills = stills_of(run, hero)
        earlier = {(s["skill"], s["phase"]): s for s in stills_of(baseline, hero)} if hero in known else {}
        for release in (s for s in stills if s["phase"] == "release"):
            skill = release["skill"]
            entry = dict(hero=hero, skill=skill, file=f"{hero}/{release['file']}",
                         hash=difference_hash(stage(run / hero / release["file"])),
                         energy={s["phase"]: dict(new=energy(run, hero, s), view=view(s)) for s in stills
                                 if s["slot"] == release["slot"] and s["phase"] != "windup"})
            before = earlier.get((skill, "release"))
            if before:
                entry["baseline_distance"] = distance(
                    entry["hash"], difference_hash(stage(baseline / hero / before["file"])))
            for phase, values in entry["energy"].items():
                if (skill, phase) in earlier:
                    values["baseline"] = energy(baseline, hero, earlier[skill, phase])
                    values["baseline_view"] = view(earlier[skill, phase])
            skills[f"{hero}/{skill}"] = entry
    # The nearest other tile of the same class and of the whole run. One skill captured in two
    # folders (its class and a mixed recipe) is meant to look the same and is not a pair.
    pairs = sorted((distance(a["hash"], b["hash"]), ka, kb)
                   for (ka, a), (kb, b) in itertools.combinations(skills.items(), 2)
                   if a["skill"] != b["skill"])
    for key, entry in skills.items():
        for field, same_class in (("nearest_in_class", True), ("nearest_in_run", False)):
            near = next(((d, kb if ka == key else ka) for d, ka, kb in pairs if key in (ka, kb)
                         and (not same_class or skills[ka]["hero"] == skills[kb]["hero"])), None)
            entry[field] = dict(skill=near[1], distance=near[0]) if near else None
    # What a reader looks at first: the pairs of one class that are nearest to each other.
    closest = [dict(distance=d, skills=[ka, kb]) for d, ka, kb in pairs
               if skills[ka]["hero"] == skills[kb]["hero"]][:CLOSEST_LISTED]
    zero = [f"{ka} and {kb} have the same release tile" for d, ka, kb in pairs if d == 0]
    zero += [f"{key} has the release tile of the baseline" for key, entry in skills.items()
             if entry.get("baseline_distance") == 0]
    weaker = [f"{key} {phase}: {values['new']} changed pixels, baseline {values['baseline']}"
              for key, entry in skills.items() for phase, values in entry["energy"].items()
              if (values["view"], values.get("baseline_view")) == ("close", "close")
              and values["new"] < values["baseline"]]
    for entry in skills.values():
        entry["hash"] = f"{entry['hash']:0{HASH_GRID[0] * HASH_GRID[1] // 4}x}"
    classes = {hero: min((entry["nearest_in_class"]["distance"] for entry in skills.values()
                          if entry["hero"] == hero and entry["nearest_in_class"]), default=None)
               for hero in heroes}
    return dict(hash=f"difference hash of the stage crop, {HASH_GRID[0]} x {HASH_GRID[1]} bits",
                energy=f"pixels of the stage crop with a channel more than {CHANGED_LEVEL} away from the idle still",
                baseline=str(baseline) if baseline else None, skills=skills, min_distance_in_class=classes,
                closest_in_class=closest,
                min_distance_in_run=pairs[0][0] if pairs else None, zero_distance=zero,
                below_baseline_energy=weaker)


def lookalike_table(report):
    """The look-alike report as a Markdown table, one row per skill."""
    def cell(values):
        if values is None:
            return "-"
        views = {values["view"], values.get("baseline_view", "close")} - {"close"}
        before = values.get("baseline")
        share = f", x{values['new'] / before:.2f}" if before and not views else ""
        return (f"{values['new']}" + (f" ({before}{share})" if before is not None else "")
                + (f" [{', '.join(sorted(views))}]" if views else ""))

    def near(entry, field):
        return f"{entry[field]['distance']} {entry[field]['skill']}" if entry.get(field) else "-"

    lines = ["| skill | nearest in class | nearest in run | to baseline | energy release (baseline) | "
             "energy impact (baseline) | energy settled (baseline) |",
             "| --- | --- | --- | --- | --- | --- | --- |"]
    for key, entry in report["skills"].items():
        lines.append(f"| {key} | {near(entry, 'nearest_in_class')} | {near(entry, 'nearest_in_run')} | "
                     f"{entry.get('baseline_distance', '-')} | {cell(entry['energy'].get('release'))} | "
                     f"{cell(entry['energy'].get('impact'))} | {cell(entry['energy'].get('settled'))} |")
    lines += ["", "Nearest pairs of one class: "
              + ("; ".join(f"{pair['distance']} {pair['skills'][0]} and {pair['skills'][1].split('/')[1]}"
                           for pair in report["closest_in_class"]) or "none") + "."]
    lines += ["", f"Distances: {report['hash']}. Two captures of one unchanged look are some bits apart as "
              "well (pose phase, particles); measure that distance on a rerun before reading a small number as "
              f"a look-alike. Energy: {report['energy']}. The factor is this run over the "
              "baseline. A value in square brackets says why an energy is not compared with the baseline: the still was taken from farther away (zoom) or "
              "after the cast moved the hero and the camera (moved).", "",
              "Zero distance (errors): " + ("; ".join(report["zero_distance"]) or "none") + ".", "",
              "Below the baseline energy: " + ("; ".join(report["below_baseline_energy"]) or "none") + "."]
    return "\n".join(lines) + "\n"


def build(run, baseline=None):
    """Write the sheets and the look-alike report of `run`; return their index (also sheets/index.json)."""
    heroes = folders(run)
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
    report = lookalike(run, heroes, baseline)
    (sheets / "lookalike.json").write_text(json.dumps(report, indent=2) + "\n")
    (sheets / "lookalike.md").write_text(lookalike_table(report))
    index["lookalike"] = "lookalike.json"
    index["zero_distance"] = report["zero_distance"]
    (sheets / "index.json").write_text(json.dumps(index, indent=2) + "\n")
    return index


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("run", type=Path, help="output directory of capture_standard_skills.py --phases")
    parser.add_argument("--baseline", type=Path,
                        help="an earlier run of the same classes: the look-alike report compares each release "
                             "tile and each energy with it")
    args = parser.parse_args(argv)
    try:
        index = build(args.run.resolve(), args.baseline.resolve() if args.baseline else None)
    except (ValueError, OSError) as error:
        raise SystemExit(f"contact sheets: {error}")
    print(json.dumps(index, indent=2))
    if index["zero_distance"]:
        raise SystemExit("look-alike report: " + "; ".join(index["zero_distance"]))


if __name__ == "__main__":
    main()
