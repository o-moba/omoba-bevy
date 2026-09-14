#!/usr/bin/env python3
"""Read packaged audio/source archives and write this task's measured audit JSON.

Requires Python standard library, ffmpeg, ffprobe, and the original source ZIPs
already in this task's downloads directory. Does not download, encode, play,
modify assets, or claim listening/physical-device verification.
"""

from array import array
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import math
from pathlib import Path
import subprocess
import sys
from zipfile import ZipFile


TASK = Path(__file__).resolve().parent
ROOT = TASK.parents[2]
ASSETS = ROOT / "client/assets"
AUDIO = ASSETS / "audio"
DOWNLOADS = TASK / "downloads"
REPORT = TASK / "raw/asset-audit.json"


def require(condition, explanation):
    if not condition:
        raise ValueError(explanation)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def db(value):
    return round(20 * math.log10(value), 6) if value > 0 else None


def command(args):
    result = subprocess.run(args, capture_output=True, check=True)
    require(not result.stderr, f"Unexpected decoder/probe diagnostics: {result.stderr.decode()}")
    return result.stdout


def probe(path):
    return json.loads(command([
        "ffprobe", "-v", "error", "-show_streams", "-show_format", "-of", "json", str(path)
    ]))


def decode(path):
    raw = command([
        "ffmpeg", "-v", "error", "-xerror", "-i", str(path),
        "-map", "0:a:0", "-f", "f32le", "-",
    ])
    pcm = array("f")
    pcm.frombytes(raw)
    if sys.byteorder != "little":
        pcm.byteswap()
    require(bool(pcm), f"Empty decode: {path}")
    require(all(math.isfinite(value) for value in pcm), f"Nonfinite samples: {path}")
    return pcm


def rms(values):
    return math.sqrt(math.fsum(value * value for value in values) / len(values))


def seam(path):
    info = probe(path)["streams"][0]
    channels, rate = info["channels"], int(info["sample_rate"])
    pcm = decode(path)
    frames = len(pcm) // channels
    measured = []
    for channel in range(channels):
        samples = pcm[channel::channels]
        # Compute the two windows separately; do not insert an artificial join.
        diffs = [samples[i] - samples[i - 1] for i in range(1, rate // 10)]
        diffs += [samples[i] - samples[i - 1] for i in range(frames - rate // 10 + 1, frames)]
        jump = abs(samples[0] - samples[-1])
        adjacent_rms = rms(diffs)
        row = {
            "channel": channel,
            "first_sample": samples[0], "last_sample": samples[-1],
            "boundary_jump": jump, "boundary_jump_dbfs": db(jump),
            "near_boundary_adjacent_difference_rms": adjacent_rms,
            "boundary_to_adjacent_rms_ratio": jump / adjacent_rms if adjacent_rms else None,
        }
        for milliseconds in (10, 100, 1000):
            count = round(rate * milliseconds / 1000)
            row[f"{milliseconds}ms_start_rms_dbfs"] = db(rms(samples[:count]))
            row[f"{milliseconds}ms_end_rms_dbfs"] = db(rms(samples[-count:]))
        measured.append(row)
    return {"path": str(path.relative_to(ROOT)), "decoded_frames": frames,
            "decoded_seconds": frames / rate, "channels": measured}


def main():
    manifest = json.loads((AUDIO / "manifest.json").read_text())
    music = json.loads((AUDIO / "music-provenance.json").read_text())
    sfx = json.loads((AUDIO / "sfx-provenance.json").read_text())
    entries = {"arena": manifest["music"], **manifest["cues"]}
    records = {"arena": music, **sfx["cues"]}
    require(manifest["version"] == 1, "Manifest version changed")
    require(len(entries) == 17 and len(manifest["cues"]) == 16, "Expected 1 music + 16 cues")
    require(set(entries) == set(records), "Manifest/provenance ID mismatch")
    require(set(ASSETS / entry["path"] for entry in entries.values()) == set(AUDIO.rglob("*.ogg")),
            "Manifest does not exactly cover packaged Ogg files")
    verified_hashes = []

    def verify_hash(path, expected, label=None, data=None):
        actual = sha(path.read_bytes() if data is None else data)
        require(actual == expected, f"SHA256 mismatch: {label or path}")
        verified_hashes.append({"path": label or str(path.relative_to(ROOT)), "sha256": actual})

    for name, entry in entries.items():
        path = ASSETS / entry["path"]
        require(path.is_file() and path.resolve().is_relative_to(AUDIO.resolve()), f"Missing/unsafe path: {path}")
        require(entry["path"] == records[name]["path"], f"Provenance path mismatch: {name}")
        require(math.isfinite(entry["gain"]) and 0 <= entry["gain"] <= 1, f"Invalid gain: {name}")
        verify_hash(path, records[name]["sha256"])
    for filename, key, record in (
        ("exploration-theme.zip", "archive_sha256", music),
        ("exploration-theme.wav", "source_sha256", music),
        ("kenney-rpg-audio.zip", "source_archive_sha256", sfx),
    ):
        verify_hash(DOWNLOADS / filename, record[key])
    with ZipFile(DOWNLOADS / "exploration-theme.zip") as archive:
        verify_hash(None, music["source_sha256"], "exploration-theme.zip::" + music["source_member"],
                    archive.read(music["source_member"]))
        require(archive.read("Exploration Theme/readme.txt") == (AUDIO / "licenses/exploration-theme.txt").read_bytes(),
                "Packaged music notice differs from source archive")
    with ZipFile(DOWNLOADS / "kenney-rpg-audio.zip") as archive:
        for cue, member in (("melee", "Audio/knifeSlice.ogg"), ("hit", "Audio/chop.ogg")):
            verify_hash(None, sfx["cues"][cue]["source_sha256"], "kenney-rpg-audio.zip::" + member,
                        archive.read(member))
        require(archive.read("License.txt") == (AUDIO / "licenses/kenney-rpg-audio.txt").read_bytes(),
                "Packaged Kenney notice differs from source archive")

    def measure(item):
        name, entry = item
        path = ASSETS / entry["path"]
        info = probe(path)
        require(len(info["streams"]) == 1, f"Expected one audio stream: {name}")
        stream = info["streams"][0]
        rate, channels = int(stream["sample_rate"]), stream["channels"]
        require(stream["codec_name"] == "vorbis" and rate == 44100, f"Codec/rate mismatch: {name}")
        require(channels == (2 if name == "arena" else 1), f"Channel mismatch: {name}")
        pcm = decode(path)
        peak = max(abs(value) for value in pcm)
        clipped = sum(abs(value) >= 1 for value in pcm)
        require(peak > 0 and clipped == 0, f"Silent/clipped asset: {name}")
        duration = float(stream.get("duration", info["format"]["duration"]))
        row = {"cue": name, "path": entry["path"], "bytes": path.stat().st_size,
               "codec": stream["codec_name"], "sample_rate": rate, "channels": channels,
               "duration_seconds": duration, "decoded_frames": len(pcm) // channels,
               "peak_dbfs": db(peak), "rms_dbfs": db(rms(pcm)), "clipped_samples": clipped,
               "all_samples_finite": True, "manifest_gain": entry["gain"],
               "decoder_diagnostics": ""}
        if name != "arena":
            delta = duration - records[name]["duration_seconds"]
            require(abs(delta) <= 1e-6, f"Duration/provenance mismatch: {name}")
            row["provenance_duration_difference_seconds"] = round(delta, 6)
        return row

    with ThreadPoolExecutor(max_workers=4) as pool:
        measurements = list(pool.map(measure, entries.items()))
    encoded, source = seam(ASSETS / music["path"]), seam(DOWNLOADS / "exploration-theme.wav")
    require(encoded["decoded_frames"] == source["decoded_frames"], "Music loop frame count changed")
    report = {
        "audit_version": 1, "status": "PASS", "script_sha256": sha(Path(__file__).read_bytes()),
        "tools": {tool: subprocess.check_output([tool, "-version"], text=True).splitlines()[0]
                  for tool in ("ffmpeg", "ffprobe")},
        "verified_sha256": verified_hashes, "notices_match_source_archives": True,
        "manifest_exactly_covers_ogg_files": True, "asset_count": len(measurements),
        "total_ogg_bytes": sum(row["bytes"] for row in measurements), "assets": measurements,
        "music_encoded_seam": encoded, "music_source_seam": source,
        "limitations": [
            "Read-only asset decoding/measurement; no subjective listening or physical-device acceptance.",
            "Seam metrics compare adjacent samples and 10/100/1000 ms windows; they do not establish perceptual seamlessness.",
            "Music preserves source frame count and boundaries; author marks source seamless.",
            "FFprobe reports container duration; some short Vorbis files decode to 128 fewer samples. Both quantities are recorded.",
            "Source-archive verification requires the original local downloads, which are not packaged game assets.",
        ],
    }
    REPORT.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(f"PASS: {len(measurements)} assets; {len(verified_hashes)} SHA256 checks; 2 archive-identical notices")
    print(f"Total Ogg bytes: {report['total_ogg_bytes']}; music: {encoded['decoded_seconds']:.3f}s / {encoded['decoded_frames']} frames")
    print(f"Report: {REPORT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
