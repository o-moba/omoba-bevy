#!/usr/bin/env python3
"""Rebuild Omoba's original synthesized cues and two CC0 Kenney adaptations.

Uses Python's standard library plus an existing ffmpeg executable; no runtime
dependency. Supply the unmodified Kenney RPG Audio ZIP from the provenance file.
Generated waveforms contain no sampled instruments, voices or external melodies.
"""
import argparse
from array import array
import hashlib
import json
import math
from pathlib import Path
import random
import subprocess
import sys
import tempfile
import wave
from zipfile import ZipFile

RATE = 44100
TAU = math.tau


def blank(seconds):
    return array('f', [0.0]) * int(seconds * RATE)


def note(buf, at, duration, frequency, gain=0.3, kind='bell', end=None):
    start = int(at * RATE)
    count = min(int(duration * RATE), len(buf) - start)
    phase = 0.0
    for i in range(count):
        t = i / RATE
        f = frequency if end is None else frequency * (end / frequency) ** (t / duration)
        phase += TAU * f / RATE
        attack = min(1.0, t / 0.008)
        release = min(1.0, max(0.0, duration - t) / 0.05)
        decay = math.exp(-t * (4.5 if kind == 'bell' else 2.4) / duration)
        if kind == 'bell':
            value = math.sin(phase) + .24 * math.sin(phase * 2.003) + .08 * math.sin(phase * 3.97)
        elif kind == 'magic':
            value = math.sin(phase + 1.7 * math.exp(-t * 8) * math.sin(phase * 1.997))
        elif kind == 'warm':
            value = math.sin(phase) + .22 * math.sin(phase * 2) + .09 * math.sin(phase * 3)
        else:
            value = math.sin(phase)
        buf[start + i] += gain * attack * release * decay * value


def air(buf, at, duration, gain, seed=14, lowpass=.22, swell=False):
    rng = random.Random(seed)
    start = int(at * RATE)
    filtered = 0.0
    for i in range(min(int(duration * RATE), len(buf) - start)):
        t = i / RATE
        filtered += lowpass * (rng.uniform(-1, 1) - filtered)
        env = math.sin(math.pi * t / duration) ** 1.5 if swell else math.exp(-t * 7 / duration)
        env *= min(1, t / .004) * min(1, max(0, duration - t) / .02)
        buf[start + i] += gain * env * filtered


def echo(buf, taps=((.071, .12), (.113, .08), (.179, .045))):
    dry = array('f', buf)
    for delay, gain in taps:
        shift = int(delay * RATE)
        for i in range(shift, len(buf)):
            buf[i] += dry[i - shift] * gain


def motif(notes, spacing, duration, kind='bell', gain=.3, total=None):
    buf = blank(total or (len(notes) - 1) * spacing + duration + .22)
    for i, f in enumerate(notes):
        note(buf, i * spacing, duration, f, gain, kind)
    echo(buf)
    return buf


def original_cues():
    out = {}
    b = blank(.5)
    air(b, 0, .18, .85, swell=True)
    note(b, .09, .19, 390, .28, 'warm', 130)
    out['arrow'] = b
    b = blank(.8)
    note(b, 0, .46, 350, .42, 'magic', 950)
    note(b, .04, .62, 700, .12, 'bell', 1200)
    air(b, 0, .28, .2, seed=19, swell=True)
    echo(b)
    out['arcane'] = b
    b = blank(.95)
    for i, f in enumerate([523.25, 659.25, 783.99]):
        note(b, i * .035, .65, f, .17)
    echo(b)
    out['holy'] = b
    b = blank(.42)
    note(b, 0, .28, 520, .28, 'magic', 290)
    air(b, 0, .1, .22, seed=22)
    out['caster'] = b
    b = blank(.75)
    note(b, 0, .53, 190, .48, 'warm', 58)
    note(b, .014, .18, 930, .12, 'magic', 320)
    air(b, 0, .36, .5, seed=23, lowpass=.13)
    out['tower'] = b
    out['kill'] = motif([523.25, 783.99], .085, .38, gain=.23)
    out['death'] = motif([220, 174.61, 130.81], .14, .8, 'warm', .32)
    out['respawn'] = motif([261.63, 392, 523.25], .13, .62, gain=.25)
    out['level_up'] = motif([392, 523.25, 659.25, 783.99], .08, .58, gain=.23)
    out['match_start'] = motif([196, 293.66, 392, 587.33], .19, .82, 'warm', .28)
    b = motif([261.63, 329.63, 392, 523.25], .18, 1.4, 'warm', .23, total=3.05)
    for f in [261.63, 329.63, 392, 523.25]:
        note(b, .95, 1.85, f, .075, 'warm')
    note(b, 1.13, 1.2, 1046.5, .075)
    out['victory'] = b
    b = motif([293.66, 261.63, 220, 146.83], .21, 1.1, 'warm', .26, total=2.35)
    note(b, .66, 1.45, 174.61, .1, 'warm')
    out['defeat'] = b
    b = blank(.12)
    note(b, 0, .09, 740, .2, 'sine', 520)
    out['ui_click'] = b
    out['ui_confirm'] = motif([660, 880], .065, .2, gain=.2)
    return out


def pcm_normalized(buf, peak=.7):
    high = max(abs(v) for v in buf) or 1.0
    gain = peak / high
    fade = min(int(RATE * .008), len(buf) // 2)
    pcm = array('h')
    for i, value in enumerate(buf):
        envelope = min(1.0, i / max(1, fade), (len(buf) - 1 - i) / max(1, fade))
        pcm.append(round(max(-.98, min(.98, value * gain * envelope)) * 32767))
    if sys.byteorder != 'little':
        pcm.byteswap()
    return pcm


def encode(buf, target, temp, ffmpeg):
    source = temp / (target.stem + '.wav')
    with wave.open(str(source), 'wb') as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(pcm_normalized(buf).tobytes())
    subprocess.run([ffmpeg, '-hide_banner', '-loglevel', 'error', '-y', '-fflags', '+bitexact',
                    '-i', str(source), '-map_metadata', '-1', '-c:a', 'libvorbis', '-q:a', '4',
                    '-flags:a', '+bitexact', str(target)], check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--kenney-archive', type=Path, required=True)
    parser.add_argument('--ffmpeg', default='ffmpeg', help='ffmpeg build with the libvorbis encoder')
    parser.add_argument('--output', type=Path, default=Path(__file__).resolve().parents[1] / 'client/assets/audio')
    args = parser.parse_args()
    sounds = args.output / 'sfx'
    sounds.mkdir(parents=True, exist_ok=True)
    originals = original_cues()
    records = {}
    with tempfile.TemporaryDirectory(prefix='omoba-audio-') as directory:
        temp = Path(directory)
        with ZipFile(args.kenney_archive) as archive:
            for cue, member in {'melee': 'Audio/knifeSlice.ogg', 'hit': 'Audio/chop.ogg'}.items():
                source = temp / (cue + '-source.ogg')
                data = archive.read(member)
                source.write_bytes(data)
                raw = subprocess.check_output([args.ffmpeg, '-hide_banner', '-loglevel', 'error', '-i', str(source),
                    '-ac', '1', '-ar', str(RATE), '-f', 'f32le', '-'])
                decoded = array('f')
                decoded.frombytes(raw)
                if sys.byteorder != 'little':
                    decoded.byteswap()
                originals[cue] = decoded
                records[cue] = dict(origin='Kenney RPG Audio 1.0 / ' + member,
                    source_sha256=hashlib.sha256(data).hexdigest())
            license_dir = args.output / 'licenses'
            license_dir.mkdir(exist_ok=True)
            (license_dir / 'kenney-rpg-audio.txt').write_bytes(archive.read('License.txt'))
        for cue, buf in originals.items():
            target = sounds / (cue + '.ogg')
            encode(buf, target, temp, args.ffmpeg)
            records.setdefault(cue, dict(origin='Original mathematical synthesis; scripts/build_audio_palette.py'))
            records[cue].update(path='audio/sfx/' + target.name, license='CC0-1.0',
                duration_seconds=round(len(buf) / RATE, 6),
                sha256=hashlib.sha256(target.read_bytes()).hexdigest())
    (args.output / 'sfx-provenance.json').write_text(json.dumps(dict(
        version=1, sample_rate=RATE, channels=1, format='Ogg Vorbis',
        source_archive_sha256=hashlib.sha256(args.kenney_archive.read_bytes()).hexdigest(),
        adaptations='Mono 44.1 kHz, restrained peak normalization, 8 ms edge fades, Vorbis q4. Original cues add synthesis/short echoes.',
        cues=records), indent=2) + '\n')
    print(f'Built {len(records)} cues in {sounds}')


if __name__ == '__main__':
    main()
