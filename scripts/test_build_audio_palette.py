"""The palette script synthesizes the Dawnweaver cues and rebuilds only what it is asked to."""
from array import array
import contextlib
import hashlib
import io
import json
import math
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import build_audio_palette as palette

ROOT = Path(__file__).resolve().parents[1]
AUDIO = ROOT / 'client/assets/audio'
DAWN = ['radiant_charge', 'radiant_beam', 'radiant_core', 'radiance_burst']
CUES = palette.original_cues()

# Writes the WAV it is handed as the "encoded" file: the selection and the provenance are
# tested without an encoder.
FAKE_FFMPEG = '''#!{python}
import shutil, sys
args = sys.argv[1:]
shutil.copyfile(args[args.index('-i') + 1], args[-1])
'''


def rms(buf, start, end):
    window = buf[int(start * palette.RATE):int(end * palette.RATE)]
    return math.sqrt(sum(value * value for value in window) / max(1, len(window)))


def loudest(buf, step=.02):
    """Where the loudest window of `step` seconds starts."""
    starts = [i * step for i in range(int(len(buf) / palette.RATE / step))]
    return max(starts, key=lambda start: rms(buf, start, start + step))


def ffmpeg():
    """An ffmpeg with the libvorbis encoder, or None."""
    for candidate in [os.environ.get('OMOBA_FFMPEG'), shutil.which('ffmpeg'),
                      '/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg']:
        if candidate and Path(candidate).exists():
            encoders = subprocess.run([candidate, '-hide_banner', '-encoders'],
                                      capture_output=True, text=True).stdout
            if 'libvorbis' in encoders:
                return candidate
    return None


def decoded(path, binary):
    raw = subprocess.check_output([binary, '-hide_banner', '-loglevel', 'error', '-i', str(path),
                                   '-ac', '1', '-ar', str(palette.RATE), '-f', 'f32le', '-'])
    samples = array('f')
    samples.frombytes(raw)
    if sys.byteorder != 'little':
        samples.byteswap()
    return samples


class DawnCueTests(unittest.TestCase):
    def test_the_charge_is_an_ignition_and_a_swell_that_peaks_when_the_ray_fires(self):
        charge = CUES['radiant_charge']
        peak = max(abs(value) for value in charge)
        # The cast plays the first 0.12 s; the ignition is over before then, and the
        # warning, from 0.20 s on, starts from silence.
        self.assertGreater(rms(charge, 0, .05), .2 * peak)
        self.assertLess(rms(charge, .1, .2), .01 * peak)
        # The warning grows for the whole windup and is loudest as the ray fires.
        fire = palette.TAIL_START + palette.RAY_WINDUP
        rising = [rms(charge, at, at + .1) for at in [.3, .45, .6, .75, .88]]
        self.assertEqual(rising, sorted(rising))
        self.assertAlmostEqual(loudest(charge), fire, delta=.06)
        self.assertLess(rms(charge, fire + .05, fire + .12), .2 * rms(charge, fire - .06, fire))

    def test_the_charge_is_timed_by_the_windup_of_the_catalog(self):
        catalog = json.loads((ROOT / 'shared/assets/catalog/skills.json').read_text())['skills']
        self.assertEqual(palette.RAY_WINDUP, next(row['windup_secs'] for row in catalog if row['id'] == 'dawn_ray'))
        # Another windup moves the whole warning: it rises until that ray fires, is loudest
        # then, and nothing of it starts later.
        with mock.patch.object(palette, 'RAY_WINDUP', .6):
            charge = palette.dawn_cues()['radiant_charge']
        fire = palette.TAIL_START + .6
        self.assertAlmostEqual(len(charge) / palette.RATE, fire + .12, delta=.001)
        self.assertAlmostEqual(loudest(charge), fire, delta=.06)
        rising = [rms(charge, at, at + .1) for at in [.25, .35, .45, .55, .65]]
        self.assertEqual(rising, sorted(rising))
        self.assertLess(rms(charge, fire + .05, fire + .12), .2 * rms(charge, fire - .06, fire))

    def test_the_beam_core_and_burst_are_short_and_strike_at_once(self):
        for cue, longest in [('radiant_beam', 1.0), ('radiant_core', .6), ('radiance_burst', .6)]:
            buf = CUES[cue]
            self.assertLessEqual(len(buf) / palette.RATE, longest, cue)
            self.assertLess(loudest(buf), .06, cue)

    def test_every_cue_is_deterministic_and_the_new_ones_are_distinct(self):
        again = palette.original_cues()
        self.assertEqual(list(again), list(CUES))
        for cue in CUES:
            self.assertEqual(again[cue], CUES[cue], cue)
        digests = {hashlib.sha256(palette.pcm_normalized(CUES[cue]).tobytes()).digest() for cue in DAWN}
        self.assertEqual(len(digests), len(DAWN))

    def test_the_packaged_cues_are_the_ones_the_script_synthesizes(self):
        binary = shutil.which('ffmpeg') or ffmpeg()
        if not binary:
            self.skipTest('ffmpeg is not installed')
        records = json.loads((AUDIO / 'sfx-provenance.json').read_text())['cues']
        manifest = json.loads((AUDIO / 'manifest.json').read_text())['cues']
        self.assertEqual(set(records), set(manifest))
        self.assertEqual(set(records) - set(palette.KENNEY), set(CUES))
        for cue, buf in CUES.items():
            packaged = AUDIO.parent / records[cue]['path']
            self.assertEqual(hashlib.sha256(packaged.read_bytes()).hexdigest(), records[cue]['sha256'], cue)
            self.assertEqual(records[cue]['duration_seconds'], round(len(buf) / palette.RATE, 6), cue)
            # Vorbis is lossy, most of all on noise: the decoded file follows the synthesized
            # wave closely, where an unrelated cue would not correlate at all.
            heard = decoded(packaged, binary)
            made = [value / 32767 for value in palette.pcm_normalized(buf)]
            count = min(len(heard), len(made))
            dot = sum(heard[i] * made[i] for i in range(count))
            norm = math.sqrt(sum(heard[i] ** 2 for i in range(count)) * sum(made[i] ** 2 for i in range(count)))
            self.assertGreater(dot / norm, .95, cue)


class RebuildTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        self.output = self.root / 'audio'
        (self.output / 'sfx').mkdir(parents=True)
        self.provenance = (AUDIO / 'sfx-provenance.json').read_text()
        (self.output / 'sfx-provenance.json').write_text(self.provenance)
        for cue in json.loads(self.provenance)['cues']:
            (self.output / 'sfx' / (cue + '.ogg')).write_bytes(b'packaged ' + cue.encode())
        self.fake = self.root / 'ffmpeg'
        self.fake.write_text(FAKE_FFMPEG.format(python=sys.executable))
        self.fake.chmod(self.fake.stat().st_mode | stat.S_IEXEC)

    def tearDown(self):
        self.directory.cleanup()

    def build(self, *args):
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            palette.main([*args, '--ffmpeg', str(self.fake), '--output', str(self.output)])
        return json.loads((self.output / 'sfx-provenance.json').read_text())

    def file(self, cue):
        return (self.output / 'sfx' / (cue + '.ogg')).read_bytes()

    def test_only_rebuilds_the_named_cues_and_keeps_every_other_file_and_record(self):
        before = json.loads(self.provenance)
        after = self.build('--only', 'radiance_burst', 'radiant_core')
        self.assertEqual(list(after['cues']), list(before['cues']))
        for cue, record in before['cues'].items():
            if cue in ('radiance_burst', 'radiant_core'):
                self.assertNotEqual(self.file(cue), b'packaged ' + cue.encode())
                self.assertEqual(record['origin'], palette.SYNTHESIS)
                self.assertEqual(after['cues'][cue]['sha256'], hashlib.sha256(self.file(cue)).hexdigest())
                self.assertEqual(after['cues'][cue]['duration_seconds'], record['duration_seconds'])
            else:
                self.assertEqual(self.file(cue), b'packaged ' + cue.encode(), cue)
                self.assertEqual(after['cues'][cue], record, cue)
        self.assertEqual(after['source_archive_sha256'], before['source_archive_sha256'])
        self.assertFalse((self.output / 'licenses').exists())

    def test_a_rebuild_without_the_archive_synthesizes_everything_and_keeps_the_adaptations(self):
        before = json.loads(self.provenance)
        after = self.build()
        for cue in palette.KENNEY:
            self.assertEqual(self.file(cue), b'packaged ' + cue.encode())
            self.assertEqual(after['cues'][cue], before['cues'][cue])
        for cue in CUES:
            self.assertEqual(after['cues'][cue]['sha256'], hashlib.sha256(self.file(cue)).hexdigest(), cue)
        self.assertEqual(after['source_archive_sha256'], before['source_archive_sha256'])

    def test_a_new_cue_is_appended_to_the_records(self):
        records = json.loads(self.provenance)
        del records['cues']['radiant_beam']
        (self.output / 'sfx-provenance.json').write_text(json.dumps(records))
        after = self.build('--only', 'radiant_beam')
        self.assertEqual(list(after['cues'])[-1], 'radiant_beam')
        self.assertEqual(list(after['cues'])[:-1], list(records['cues']))

    def test_unknown_cues_and_adaptations_without_the_archive_are_refused(self):
        for args in [['--only', 'not_a_cue'], ['--only', 'melee']]:
            with self.assertRaises(SystemExit):
                self.build(*args)
        (self.output / 'sfx-provenance.json').unlink()
        with self.assertRaises(SystemExit):
            self.build('--only', 'radiant_core')
        self.assertEqual(self.file('radiant_core'), b'packaged radiant_core')

    def test_one_encoder_writes_the_same_bytes_twice(self):
        binary = ffmpeg()
        if not binary:
            self.skipTest('no ffmpeg with the libvorbis encoder')
        target = self.root / 'radiant_core.ogg'
        palette.encode(CUES['radiant_core'], target, self.root, binary)
        first = target.read_bytes()
        palette.encode(CUES['radiant_core'], target, self.root, binary)
        self.assertEqual(target.read_bytes(), first)


if __name__ == '__main__':
    unittest.main()
