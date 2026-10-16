#!/usr/bin/env python3
"""Generate the OMOBA Dev app icons: the production icons with a red DEV ribbon.

    python3 scripts/make_dev_icons.py           # (re)write the dev icons
    python3 scripts/make_dev_icons.py --check   # fail when a committed icon is stale

Sources (production, unchanged):
    mobile/ios/Assets.xcassets/AppIcon.appiconset/AppIcon.png       1024 px
    mobile/android/res/mipmap-*/ic_launcher.png                     48-192 px
Outputs (committed; the dev contour selects them, scripts/contours.py):
    mobile/ios/Assets.xcassets/AppIconDev.appiconset/AppIcon.png (+ Contents.json)
    mobile/android/res-dev/mipmap-*/ic_launcher.png

Deterministic and dependency-free (zlib + struct): the ribbon is computed per
pixel with a fixed 4x4 supersample, so the same sources always give the same
pixels. `--check` compares decoded pixels, not PNG bytes, so another zlib
version cannot make it fail.
"""
from __future__ import annotations

import argparse
import json
import math
import struct
import sys
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
IOS_SOURCE = ROOT / "mobile/ios/Assets.xcassets/AppIcon.appiconset"
IOS_TARGET = ROOT / "mobile/ios/Assets.xcassets/AppIconDev.appiconset"
ANDROID_SOURCE = ROOT / "mobile/android/res"
ANDROID_TARGET = ROOT / "mobile/android/res-dev"

# The ribbon crosses the top-right corner along u - v = OFFSET (unit square, v
# down), HALF_WIDTH either side; the text reads down the ribbon.
OFFSET = 0.55
HALF_WIDTH = 0.078
EDGE = 0.008
RIBBON = (200, 34, 44)
RIBBON_EDGE = (120, 12, 20)
INK = (255, 255, 255)
SAMPLES = 4
GLYPHS = {
    "D": ("####.", "#...#", "#...#", "#...#", "#...#", "#...#", "####."),
    "E": ("#####", "#....", "#....", "####.", "#....", "#....", "#####"),
    "V": ("#...#", "#...#", "#...#", "#...#", ".#.#.", ".#.#.", "..#.."),
}
LABEL = "DEV"
TEXT_HEIGHT = 0.62  # of the ribbon's width


# ---------------------------------------------------------------- PNG

def _chunks(data: bytes):
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("not a PNG file")
    offset = 8
    while offset < len(data):
        length, kind = struct.unpack(">I4s", data[offset:offset + 8])
        yield kind, data[offset + 8:offset + 8 + length]
        offset += 12 + length


def _paeth(a: int, b: int, c: int) -> int:
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    if pa <= pb and pa <= pc:
        return a
    return b if pb <= pc else c


def read_png(path: Path) -> tuple[int, int, int, bytearray]:
    """(width, height, channels, rows): 8-bit RGB or RGBA, non-interlaced."""
    header, compressed = None, b""
    for kind, body in _chunks(path.read_bytes()):
        if kind == b"IHDR":
            header = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            compressed += body
    if header is None:
        raise ValueError(f"{path}: no IHDR")
    width, height, depth, color, _, _, interlace = header
    if depth != 8 or color not in (2, 6) or interlace:
        raise ValueError(f"{path}: only 8-bit RGB/RGBA non-interlaced PNGs are supported")
    channels = 3 if color == 2 else 4
    stride = width * channels
    raw = zlib.decompress(compressed)
    pixels = bytearray(height * stride)
    previous = bytearray(stride)
    for y in range(height):
        start = y * (stride + 1)
        kind = raw[start]
        line = bytearray(raw[start + 1:start + 1 + stride])
        if kind == 1:
            for i in range(channels, stride):
                line[i] = (line[i] + line[i - channels]) & 0xFF
        elif kind == 2:
            for i in range(stride):
                line[i] = (line[i] + previous[i]) & 0xFF
        elif kind == 3:
            for i in range(stride):
                left = line[i - channels] if i >= channels else 0
                line[i] = (line[i] + ((left + previous[i]) >> 1)) & 0xFF
        elif kind == 4:
            for i in range(stride):
                left = line[i - channels] if i >= channels else 0
                upper_left = previous[i - channels] if i >= channels else 0
                line[i] = (line[i] + _paeth(left, previous[i], upper_left)) & 0xFF
        elif kind != 0:
            raise ValueError(f"{path}: bad filter {kind}")
        pixels[y * stride:(y + 1) * stride] = line
        previous = line
    return width, height, channels, pixels


def png_bytes(width: int, height: int, channels: int, pixels: bytes) -> bytes:
    stride = width * channels
    raw = b"".join(b"\x00" + bytes(pixels[y * stride:(y + 1) * stride]) for y in range(height))

    def chunk(kind: bytes, body: bytes) -> bytes:
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)

    color = 2 if channels == 3 else 6
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, color, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


# ---------------------------------------------------------------- ribbon

def _ink_at(along: float, across: float) -> bool:
    """Whether a ribbon point (unit coordinates from its centre) is lettering."""
    unit = 2 * HALF_WIDTH * TEXT_HEIGHT / 7
    columns = len(LABEL) * 6 - 1
    gx = along / unit + columns / 2
    gy = 3.5 - across / unit
    if not (0 <= gx < columns and 0 <= gy < 7):
        return False
    column = int(gx)
    letter, inner = divmod(column, 6)
    return inner < 5 and GLYPHS[LABEL[letter]][int(gy)][inner] == "#"


def _sample(u: float, v: float) -> tuple[int, int, int] | None:
    """The ribbon colour at a unit point, none outside the ribbon."""
    across = ((u - v) - OFFSET) / math.sqrt(2)
    if abs(across) >= HALF_WIDTH:
        return None
    if abs(across) >= HALF_WIDTH - EDGE:
        return RIBBON_EDGE
    along = (u + v) / math.sqrt(2) - 1 / math.sqrt(2)
    return INK if _ink_at(along, across) else RIBBON


def add_ribbon(width: int, height: int, channels: int, pixels: bytes) -> bytearray:
    """A copy of the image with the DEV ribbon blended in (4x4 supersampled)."""
    out = bytearray(pixels)
    size = float(min(width, height))
    reach = HALF_WIDTH * math.sqrt(2) + 2 / size
    for y in range(height):
        for x in range(width):
            centre = ((x + 0.5) - (y + 0.5)) / size - OFFSET
            if abs(centre) > reach:
                continue
            total = [0, 0, 0]
            covered = 0
            for sy in range(SAMPLES):
                for sx in range(SAMPLES):
                    colour = _sample((x + (sx + 0.5) / SAMPLES) / size, (y + (sy + 0.5) / SAMPLES) / size)
                    if colour is None:
                        continue
                    covered += 1
                    for channel in range(3):
                        total[channel] += colour[channel]
            if not covered:
                continue
            index = (y * width + x) * channels
            samples = SAMPLES * SAMPLES
            for channel in range(3):
                base = out[index + channel] * (samples - covered)
                out[index + channel] = (base + total[channel] + samples // 2) // samples
    return out


# ---------------------------------------------------------------- outputs

def icon_pairs(root: Path = ROOT) -> list[tuple[Path, Path]]:
    """(production source, dev output) for every icon."""
    ios_source = root / IOS_SOURCE.relative_to(ROOT)
    ios_target = root / IOS_TARGET.relative_to(ROOT)
    android_source = root / ANDROID_SOURCE.relative_to(ROOT)
    android_target = root / ANDROID_TARGET.relative_to(ROOT)
    pairs = [(ios_source / "AppIcon.png", ios_target / "AppIcon.png")]
    for folder in sorted(android_source.glob("mipmap-*")):
        pairs.append((folder / "ic_launcher.png", android_target / folder.name / "ic_launcher.png"))
    return pairs


def dev_icon(source: Path) -> tuple[int, int, int, bytearray]:
    width, height, channels, pixels = read_png(source)
    return width, height, channels, add_ribbon(width, height, channels, pixels)


def ios_contents(root: Path = ROOT) -> str:
    """The dev app icon set's Contents.json: the production one's layout."""
    contents = json.loads((root / IOS_SOURCE.relative_to(ROOT) / "Contents.json").read_text())
    return json.dumps(contents, indent=2) + "\n"


def write(root: Path = ROOT) -> list[Path]:
    written = []
    for source, target in icon_pairs(root):
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(png_bytes(*dev_icon(source)))
        written.append(target)
    contents = root / IOS_TARGET.relative_to(ROOT) / "Contents.json"
    contents.write_text(ios_contents(root))
    written.append(contents)
    return written


def stale(root: Path = ROOT) -> list[Path]:
    """Dev icons that are missing or whose pixels differ from a fresh generation."""
    found = []
    for source, target in icon_pairs(root):
        if not target.is_file() or read_png(target) != dev_icon(source):
            found.append(target)
    contents = root / IOS_TARGET.relative_to(ROOT) / "Contents.json"
    if not contents.is_file() or contents.read_text() != ios_contents(root):
        found.append(contents)
    return found


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", action="store_true", help="fail when a committed dev icon is stale")
    args = parser.parse_args(argv)
    if args.check:
        outdated = stale()
        for path in outdated:
            print(f"stale: {path.relative_to(ROOT)}", file=sys.stderr)
        if outdated:
            print("run: python3 scripts/make_dev_icons.py", file=sys.stderr)
        return 1 if outdated else 0
    for path in write():
        print(path.relative_to(ROOT))
    return 0


if __name__ == "__main__":
    sys.exit(main())
