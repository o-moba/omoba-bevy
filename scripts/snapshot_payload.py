"""Bounded lossless world-snapshot envelope decoder for native QA observers."""
import struct
import zlib

MAX_SNAPSHOT_BYTES = 65_507


def decode_snapshot_payload(payload):
    """Decode a complete framed payload; never accept a partial zlib stream."""
    if not payload or len(payload) > MAX_SNAPSHOT_BYTES:
        raise ValueError("snapshot payload exceeds transport budget")
    if not payload.startswith(b"OMZ1"):
        return payload
    if len(payload) <= 8:
        raise ValueError("truncated compressed snapshot header")
    expected = struct.unpack_from("<I", payload, 4)[0]
    if not 0 < expected <= MAX_SNAPSHOT_BYTES:
        raise ValueError("invalid decompressed snapshot length")
    codec = zlib.decompressobj()
    try:
        plain = codec.decompress(payload[8:], expected + 1)
    except zlib.error as error:
        raise ValueError("invalid compressed snapshot") from error
    if (len(plain) != expected or not codec.eof or codec.unused_data
            or codec.unconsumed_tail):
        raise ValueError("incomplete, oversized or trailing compressed snapshot")
    return plain
