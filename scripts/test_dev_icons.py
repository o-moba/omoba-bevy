"""The OMOBA Dev icons: committed, current, and the production icons plus a ribbon."""
import struct
import tempfile
import unittest
import zlib
from pathlib import Path

import make_dev_icons as icons


def filtered_png(width, height, rows, kinds):
    """A PNG whose rows use the given filter types (the encoder side of read_png)."""
    channels = 3
    stride = width * channels
    raw = b""
    previous = bytearray(stride)
    for row, kind in zip(rows, kinds):
        line = bytearray(row)
        encoded = bytearray(stride)
        for i in range(stride):
            left = line[i - channels] if i >= channels else 0
            up = previous[i]
            upper_left = previous[i - channels] if i >= channels else 0
            predictor = {0: 0, 1: left, 2: up, 3: (left + up) >> 1,
                         4: icons._paeth(left, up, upper_left)}[kind]
            encoded[i] = (line[i] - predictor) & 0xFF
        raw += bytes([kind]) + bytes(encoded)
        previous = line

    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


class DevIconTests(unittest.TestCase):
    def test_the_committed_dev_icons_are_current(self):
        self.assertEqual(icons.stale(), [], "run: python3 scripts/make_dev_icons.py")

    def test_every_production_icon_has_a_dev_twin_of_the_same_size(self):
        pairs = icons.icon_pairs()
        self.assertEqual(len(pairs), 6, "the iOS icon and five Android densities")
        for source, target in pairs:
            width, height, channels, _ = icons.read_png(source)
            self.assertEqual(icons.read_png(target)[:3], (width, height, channels), target)
            self.assertNotEqual(source.read_bytes(), target.read_bytes())

    def test_only_the_ribbon_changes_and_it_is_red_with_white_lettering(self):
        width, height, channels, pixels = icons.read_png(icons.IOS_SOURCE / "AppIcon.png")
        dev = icons.add_ribbon(width, height, channels, pixels)

        def pixel(data, x, y):
            index = (y * width + x) * channels
            return tuple(data[index:index + 3])

        for x, y in ((10, height - 10), (width // 2, height // 2), (40, 40), (width - 40, height - 40)):
            self.assertEqual(pixel(dev, x, y), pixel(pixels, x, y), (x, y))
        # On the ribbon's centre line, away from the lettering.
        d = icons.OFFSET * width
        x, y = int(d + 0.03 * width), int(0.03 * width)
        self.assertEqual(pixel(dev, x, y), icons.RIBBON)
        colours = {pixel(dev, x, y) for y in range(height // 3) for x in range(width // 2, width)}
        self.assertIn(icons.INK, colours, "the DEV lettering is drawn")

    def test_the_decoder_reads_every_filter_and_round_trips(self):
        rows = [bytes((x * 37 + y * 11 + c * 5) & 0xFF for x in range(5) for c in range(3)) for y in range(5)]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory, "filters.png")
            path.write_bytes(filtered_png(5, 5, rows, [0, 1, 2, 3, 4]))
            width, height, channels, pixels = icons.read_png(path)
            self.assertEqual((width, height, channels, bytes(pixels)), (5, 5, 3, b"".join(rows)))
            path.write_bytes(icons.png_bytes(width, height, channels, pixels))
            self.assertEqual(icons.read_png(path)[3], pixels)

    def test_generation_is_deterministic_into_a_fresh_tree(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for source, _ in icons.icon_pairs():
                target = root / source.relative_to(icons.ROOT)
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(source.read_bytes())
            contents = root / (icons.IOS_SOURCE / "Contents.json").relative_to(icons.ROOT)
            contents.write_text((icons.IOS_SOURCE / "Contents.json").read_text())
            self.assertTrue(icons.stale(root))
            icons.write(root)
            self.assertEqual(icons.stale(root), [])
            for _, target in icons.icon_pairs(root):
                committed = icons.ROOT / target.relative_to(root)
                self.assertEqual(icons.read_png(target), icons.read_png(committed))


if __name__ == "__main__":
    unittest.main()
