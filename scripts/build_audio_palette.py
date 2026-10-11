#!/usr/bin/env python3
"""Rebuild Omoba's original synthesized cues and two CC0 Kenney adaptations.

Uses Python's standard library plus an existing ffmpeg executable; no runtime
dependency. Supply the unmodified Kenney RPG Audio ZIP from the provenance file
to rebuild the two adaptations; without it only the synthesized cues are built
and the recorded adaptations are kept. `--only` rebuilds the named cues and
leaves every other file and provenance record as it is.
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


def note(buf, at, duration, frequency, gain=0.3, kind='bell', end=None, swell=False):
    """A tone that decays, or with `swell` one that grows to its end."""
    start = int(at * RATE)
    count = min(int(duration * RATE), len(buf) - start)
    phase = 0.0
    for i in range(count):
        t = i / RATE
        f = frequency if end is None else frequency * (end / frequency) ** (t / duration)
        phase += TAU * f / RATE
        attack = min(1.0, t / 0.008)
        release = min(1.0, max(0.0, duration - t) / 0.05)
        if swell:
            decay = (t / duration) ** 2
        else:
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
    out['butterfly'] = motif([880, 1174.66, 1567.98], .055, .3, gain=.22)
    b = blank(.55)
    air(b, 0, .14, .8, seed=83, lowpass=.6)
    note(b, .035, .22, 740, .3, 'magic', 210)
    note(b, .12, .31, 1046.5, .15, 'bell')
    out['trap_trigger'] = b
    b = blank(.42)
    air(b, 0, .20, .5, seed=173, swell=True)
    note(b, .04, .28, 510, .26, 'magic', 1350)
    out['bluff'] = b
    b = blank(.85)
    air(b, 0, .09, .65, seed=179, lowpass=.75)
    note(b, 0, .40, 1320, .29, 'bell', 440)
    note(b, .07, .30, 95, .40, 'warm', 55)
    note(b, .18, .50, 1760, .16, 'bell')
    echo(b)
    out['vital_break'] = b
    out.update(dawn_cues())
    return out


def windup_secs(skill):
    """A skill's windup in the shared catalog, which the server times its release by."""
    catalog = Path(__file__).resolve().parents[1] / 'shared/assets/catalog/skills.json'
    rows = json.loads(catalog.read_text())['skills']
    return next(row['windup_secs'] for row in rows if row['id'] == skill)


# The Horizon Ray fires this long after its warning appears; the charge is built to peak then.
RAY_WINDUP = windup_secs('dawn_ray')
# Where the `tail` slice of a row voice starts (client AudioSlice::Tail). If the two
# disagree, the client test of the decoded charge no longer finds its peak one windup in.
TAIL_START = .2
C_MAJOR = [523.25, 659.25, 783.99, 1046.5, 1318.51, 1567.98, 2093.0, 2637.02]


def dawn_cues():
    """The Dawnweaver's light: the Horizon Ray charge and beam, its core and the Radiance burst."""
    out = {}
    # The ray's charge. Its `tick` is the cast: a short ignition that is over before the
    # tail starts. Its `tail` is the warning: a glide, a climbing arpeggio and air that
    # swell for the whole windup and are loudest when the ray fires.
    fire = TAIL_START + RAY_WINDUP
    b = blank(fire + .12)
    note(b, 0, .1, 783.99, .2, 'magic', 1567.98)
    note(b, .006, .09, 1567.98, .1)
    air(b, 0, .05, .3, seed=211, lowpass=.5)
    # A swell is loudest just before its 50 ms release, so it runs that much past the fire.
    rise = RAY_WINDUP + .04
    note(b, TAIL_START, rise, 261.63, .22, 'magic', 1046.5, swell=True)
    note(b, TAIL_START, rise, 523.25, .07, 'bell', 2093.0, swell=True)
    # The arpeggio and the air enter at shares of the windup, so they follow its length.
    for i, (share, f) in enumerate(zip([.025, .225, .4, .55, .675, .775, .8625, .9375], C_MAJOR)):
        note(b, TAIL_START + share * RAY_WINDUP, .14, f, .05 + .015 * i)
    for i, share in enumerate([.25, .5, .7]):
        air(b, TAIL_START + share * RAY_WINDUP, .26, .08 + .07 * i, seed=223 + i, lowpass=.35, swell=True)
    note(b, fire - .02, .12, 3135.96, .1)
    out['radiant_charge'] = b
    # The ray fires: a warm low thump, a bright bell cluster and a burst of air.
    b = blank(.95)
    note(b, 0, .5, 130.81, .5, 'warm', 65.41)
    for i, f in enumerate([1046.5, 1318.51, 1567.98]):
        note(b, .006 * i, .62, f, .11)
    air(b, 0, .16, .7, seed=227, lowpass=.6)
    air(b, .03, .4, .16, seed=229, lowpass=.3, swell=True)
    echo(b)
    out['radiant_beam'] = b
    # A hit inside the core of the ray: one pure, high note above the beam.
    b = blank(.52)
    note(b, 0, .34, 2093.0, .26)
    note(b, .01, .3, 3135.96, .1, 'sine')
    echo(b)
    out['radiant_core'] = b
    # A Radiance mark bursts: an airy pop, an upward flick and a short sparkle.
    b = blank(.5)
    air(b, 0, .05, .75, seed=233, lowpass=.75)
    note(b, 0, .1, 620, .3, 'magic', 1860)
    note(b, .025, .24, 1318.51, .14)
    note(b, .045, .22, 1567.98, .12)
    echo(b)
    out['radiance_burst'] = b
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
    # `+bitexact` on the output also fixes the Ogg stream serial, so one encoder build
    # writes the same bytes on every run.
    subprocess.run([ffmpeg, '-hide_banner', '-loglevel', 'error', '-y', '-fflags', '+bitexact',
                    '-i', str(source), '-map_metadata', '-1', '-c:a', 'libvorbis', '-q:a', '4',
                    '-flags:a', '+bitexact', '-fflags', '+bitexact', str(target)], check=True)


KENNEY = {'melee': 'Audio/knifeSlice.ogg', 'hit': 'Audio/chop.ogg'}
SYNTHESIS = 'Original mathematical synthesis; scripts/build_audio_palette.py'


def adapt(archive, member, temp, ffmpeg):
    """A Kenney clip as mono float samples at RATE."""
    source = temp / (Path(member).stem + '-source.ogg')
    source.write_bytes(archive.read(member))
    raw = subprocess.check_output([ffmpeg, '-hide_banner', '-loglevel', 'error', '-i', str(source),
        '-ac', '1', '-ar', str(RATE), '-f', 'f32le', '-'])
    decoded = array('f')
    decoded.frombytes(raw)
    if sys.byteorder != 'little':
        decoded.byteswap()
    return decoded


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--kenney-archive', type=Path,
                        help='the unmodified Kenney RPG Audio ZIP; only needed to rebuild melee and hit')
    parser.add_argument('--only', nargs='+', metavar='CUE',
                        help='rebuild only these cues and keep every other file and record')
    parser.add_argument('--ffmpeg', default='ffmpeg', help='ffmpeg build with the libvorbis encoder')
    parser.add_argument('--output', type=Path, default=Path(__file__).resolve().parents[1] / 'client/assets/audio')
    args = parser.parse_args(argv)
    synthesized = original_cues()
    known = [*KENNEY, *synthesized]
    selected = args.only or [cue for cue in known if cue in synthesized or args.kenney_archive]
    unknown = sorted(set(selected) - set(known))
    if unknown:
        parser.error('unknown cue: ' + ', '.join(unknown))
    adapted = [cue for cue in KENNEY if cue in selected]
    if adapted and not args.kenney_archive:
        parser.error('--kenney-archive is needed to rebuild ' + ', '.join(adapted))
    provenance = args.output / 'sfx-provenance.json'
    previous = json.loads(provenance.read_text()) if provenance.exists() else {}
    if not args.kenney_archive and 'source_archive_sha256' not in previous:
        parser.error(f'{provenance} records no adaptations to keep; pass --kenney-archive')
    # Records of the cues that are not rebuilt stay as they are, in their order.
    records = dict(previous.get('cues', {}))
    sounds = args.output / 'sfx'
    sounds.mkdir(parents=True, exist_ok=True)
    built = {}
    with tempfile.TemporaryDirectory(prefix='omoba-audio-') as directory:
        temp = Path(directory)
        if adapted:
            with ZipFile(args.kenney_archive) as archive:
                for cue in adapted:
                    built[cue] = adapt(archive, KENNEY[cue], temp, args.ffmpeg), dict(
                        origin='Kenney RPG Audio 1.0 / ' + KENNEY[cue],
                        source_sha256=hashlib.sha256(archive.read(KENNEY[cue])).hexdigest())
                license_dir = args.output / 'licenses'
                license_dir.mkdir(exist_ok=True)
                (license_dir / 'kenney-rpg-audio.txt').write_bytes(archive.read('License.txt'))
        for cue in known:
            if cue in selected and cue in synthesized:
                built[cue] = synthesized[cue], dict(origin=SYNTHESIS)
        for cue, (buf, record) in built.items():
            target = sounds / (cue + '.ogg')
            encode(buf, target, temp, args.ffmpeg)
            records[cue] = dict(record, path='audio/sfx/' + target.name, license='CC0-1.0',
                duration_seconds=round(len(buf) / RATE, 6),
                sha256=hashlib.sha256(target.read_bytes()).hexdigest())
    archive_sha256 = (hashlib.sha256(args.kenney_archive.read_bytes()).hexdigest() if args.kenney_archive
                      else previous['source_archive_sha256'])
    provenance.write_text(json.dumps(dict(
        version=1, sample_rate=RATE, channels=1, format='Ogg Vorbis',
        source_archive_sha256=archive_sha256,
        adaptations='Mono 44.1 kHz, restrained peak normalization, 8 ms edge fades, Vorbis q4. Original cues add synthesis/short echoes.',
        cues=records), indent=2) + '\n')
    print(f'Built {len(built)} of {len(records)} cues in {sounds}')


if __name__ == '__main__':
    main()
