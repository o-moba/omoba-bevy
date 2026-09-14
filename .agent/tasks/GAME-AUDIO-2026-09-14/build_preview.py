"""Render a labelled-by-timeline audio palette preview; this is not game capture."""
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[3]
TASK = Path(__file__).resolve().parent
ASSETS = ROOT / 'client/assets/audio'
manifest = json.loads((ASSETS / 'manifest.json').read_text())
order = ['ui_click', 'ui_confirm', 'melee', 'arrow', 'arcane', 'holy', 'caster',
         'tower', 'hit', 'kill', 'level_up', 'death', 'respawn', 'match_start', 'defeat', 'victory']
times = [1.0, 2.0, 3.5, 5.0, 6.5, 8.0, 9.5, 11.0, 12.5, 14.0, 15.5,
         17.0, 19.0, 21.0, 23.5, 27.0]
args = ['ffmpeg', '-hide_banner', '-loglevel', 'error', '-y', '-i', str(ASSETS/'music/arena.ogg')]
for cue in order:
    args += ['-i', str(ASSETS/f'sfx/{cue}.ogg')]
filters = ['[0:a]atrim=0:32,asetpts=PTS-STARTPTS,volume=0.65,afade=t=in:d=0.5,afade=t=out:st=30:d=2[music]']
for i, (cue, at) in enumerate(zip(order, times), 1):
    gain = .8 * (.6 if cue.startswith('ui_') else .7) * manifest['cues'][cue]['gain']
    filters.append(f'[{i}:a]volume={gain},adelay={int(at*1000)}:all=1[c{i}]')
filters.append('[music]' + ''.join(f'[c{i}]' for i in range(1,17)) + 'amix=inputs=17:normalize=0:duration=first,alimiter=limit=0.95:level=0[out]')
args += ['-filter_complex', ';'.join(filters), '-map', '[out]', '-ar', '44100',
         '-c:a', 'libmp3lame', '-b:a', '192k', str(TASK/'raw/audio-preview.mp3')]
subprocess.run(args, check=True)
(TASK/'raw/preview-timeline.json').write_text(json.dumps(dict(
    kind='authored palette preview, not in-game recording', music='Exploration Theme / Cleyton Kauffman / CC0',
    music_gain=.65, effects_gain='default master and effects/UI plus manifest cue gain',
    cues=[dict(at_seconds=at,cue=cue) for at,cue in zip(times,order)],duration_seconds=32),indent=2)+'\n')
print('Rendered 32-second audio preview and cue timeline')
