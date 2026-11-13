import struct
import unittest
import zlib

from snapshot_payload import MAX_SNAPSHOT_BYTES, decode_snapshot_payload


def envelope(raw):
    return b"OMZ1" + struct.pack("<I", len(raw)) + zlib.compress(raw, 1)


class SnapshotPayloadTests(unittest.TestCase):
    def test_exact_bytes_and_plain_compatibility(self):
        raw = b'{"id":18446744073709551615,"x":1.0000000000000002}'
        for data in (raw, bytes(range(256)) * 200, b"x" * MAX_SNAPSHOT_BYTES):
            self.assertEqual(decode_snapshot_payload(envelope(data)), data)
            self.assertEqual(decode_snapshot_payload(data), data)

    def test_truncation_trailing_concatenation_and_length_lies(self):
        raw = b"x" * MAX_SNAPSHOT_BYTES
        good = envelope(raw)
        for size in range(4, len(good)):
            with self.assertRaises(ValueError):
                decode_snapshot_payload(good[:size])
        bad = [good + b"x", good + good[8:], b"", b"x" * (MAX_SNAPSHOT_BYTES + 1)]
        for length in (0, 1, len(raw) - 1, len(raw) + 1, 0xffffffff):
            bad.append(good[:4] + struct.pack("<I", length) + good[8:])
        for data in bad:
            with self.assertRaises(ValueError):
                decode_snapshot_payload(data)

    def test_checksum_corruption_and_bounded_expansion(self):
        data = bytearray(envelope(b"hello" * 1000))
        data[-1] ^= 1
        with self.assertRaises(ValueError):
            decode_snapshot_payload(bytes(data))
        bomb = b"OMZ1" + struct.pack("<I", 4) + zlib.compress(b"x" * 1_000_000)
        with self.assertRaises(ValueError):
            decode_snapshot_payload(bomb)


if __name__ == "__main__":
    unittest.main()
