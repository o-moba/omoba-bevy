#!/usr/bin/env python3
"""Fetch a VRM avatar and stage it as a Bevy-loadable GLB.

VRM 0.x files are glTF 2.0 *binary* containers (the same `glTF`-magic / version-2
GLB layout Bevy's `GltfLoader` already understands). The VRM-specific data lives
in extra glTF extensions (`VRM`, spring bones, blendshapes) that are listed under
`extensionsUsed` only — never `extensionsRequired` — so Bevy ignores them and
still loads the mesh + skeleton. That makes the "conversion" a validate-and-copy:
we confirm the bytes really are a version-2 GLB and write them out with a `.glb`
extension so the asset server selects the glTF loader by file extension.

Usage:
    python3 scripts/convert_vrm_to_glb.py <source-url-or-path> <output.glb>

Example (re-fetch the bundled Paco avatar):
    python3 scripts/convert_vrm_to_glb.py \
        https://arweave.net/0i-EEnHlcq1EZ1-sMi8DTZhesqGLqtf30WuCknfTHjA \
        client/assets/downloaded/paco.glb
"""

import struct
import sys
import urllib.request
from pathlib import Path

GLB_MAGIC = b"glTF"
GLB_VERSION = 2


def read_source(source: str) -> bytes:
    if source.startswith(("http://", "https://")):
        with urllib.request.urlopen(source, timeout=120) as response:
            return response.read()
    return Path(source).read_bytes()


def validate_glb(data: bytes) -> None:
    if len(data) < 12:
        raise ValueError(f"file too small to be a GLB ({len(data)} bytes)")
    magic, version, declared_len = struct.unpack("<4sII", data[:12])
    if magic != GLB_MAGIC:
        raise ValueError(f"bad GLB magic: {magic!r} (expected {GLB_MAGIC!r})")
    if version != GLB_VERSION:
        raise ValueError(f"unsupported GLB version {version} (expected {GLB_VERSION})")
    if declared_len > len(data):
        raise ValueError(
            f"declared length {declared_len} exceeds buffer {len(data)}"
        )


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    source, output = sys.argv[1], sys.argv[2]
    data = read_source(source)
    validate_glb(data)
    out_path = Path(output)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_bytes(data)
    print(f"Wrote {len(data)} bytes to {out_path} (valid glTF 2.0 binary).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
