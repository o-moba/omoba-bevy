#!/usr/bin/env python3
"""Capture the real Wildspark sandbox director, verify its receipts, and edit a social demo.

Uses existing binaries and one native English 960x1280 viewport. No build/deploy.
The final mix places packaged game SFX from recorded row-voice events in post;
it is not a recording of the system audio device. No simulated damage is added.
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
import time

from capture_showcase import ROOT, base_env, free_address, run_until_exit, sha256, source_identity, stop
from record_demo import find_ffmpeg

CHAPTERS = [
    ('repeater', 'СКОРОСТНАЯ ОЧЕРЕДЬ', 'Одна цель. Темп растёт с попаданиями.', '#3FE8FF'),
    ('rockets', 'ПЕРЕКЛЮЧАЕМ ПУШКУ', 'Ракеты бьют по площади · 4 маны за выстрел', '#FFD23F'),
    ('slow', 'ПОЙМАТЬ НА ПРИЦЕЛ', 'Дальний выстрел замедляет противника', '#3FE8FF'),
    ('traps', 'ЛОВУШКА НА ПУТИ', 'Сначала взводится. Затем удерживает врага.', '#FF6A1A'),
    ('rocket-near', 'УЛЬТА · БЛИЗКАЯ ЦЕЛЬ', 'Ракета разгоняется в полёте', '#FF4FA3'),
    ('rocket-far', 'ДАЛЬШЕ — СИЛЬНЕЕ', 'Один взрыв задевает обе цели', '#FF6A1A'),
]


def capture(args, raw):
    raw.mkdir(parents=True, exist_ok=False)
    processes = []
    with tempfile.TemporaryDirectory(prefix='omoba-wildspark-demo-') as work:
        host, port = free_address()
        env = base_env(dict(size=(960,1280),profile='desktop'), args.assets, work, raw)
        env.update(SERVER_ADDR=f'{host}:{port}',GAME_SERVER_ADDR=f'{host}:{port}',
                   OMOBA_MATCH_MODE='dev',OMOBA_TEAM_SIZE='5',OMOBA_LANGUAGE='en',
                   OMOBA_COMBAT_SANDBOX='1',OMOBA_PLAYER_VISUAL_MODE='models3d')
        try:
            with (raw/'server.log').open('w') as log:
                server=subprocess.Popen([str(args.server_bin)],cwd=work,env=env,stdout=log,stderr=subprocess.STDOUT)
            processes.append(server)
            end=time.monotonic()+20
            while 'is listening' not in (raw/'server.log').read_text(errors='replace'):
                if server.poll() is not None or time.monotonic()>end: raise RuntimeError('Server did not start')
                time.sleep(.05)
            env.update(OMOBA_STANDARD_QA_DIR=str(raw),OMOBA_STANDARD_QA_CLASS='wildspark',
                       OMOBA_STANDARD_QA_AVATAR='anna',OMOBA_STANDARD_QA_OFFSCREEN='1',OMOBA_WILDSPARK_DEMO='1')
            with (raw/'client.log').open('w') as log:
                client=subprocess.Popen([str(args.client_bin)],cwd=work,env=env,stdout=log,stderr=subprocess.STDOUT)
            processes.append(client)
            code=run_until_exit(processes,client,240)
            if code!=0: raise RuntimeError(f'Client exited {code}; inspect {raw}/client.log')
        finally: stop(processes)
    return json.loads((raw/'demo.json').read_text())


def validate(report, raw):
    errors=[]; facts={}
    if not report.get('capture_complete') or {c['chapter'] for c in report['chapters']} != {c[0] for c in CHAPTERS}:
        return dict(pass_=False, errors=['Incomplete chapter inventory'], facts={})
    for chapter in report['chapters']:
        name=chapter['chapter']; frames=chapter['frames']
        if len(frames)<20 or any(not (raw/f['file']).is_file() or not f.get('mean_pixel') for f in frames):
            errors.append(name+': incomplete or black frames')
        receipts={e['id']:e for f in frames for e in f['receipts']}
        player=next(a['id'] for a in frames[0]['actors'] if a['actor']=='player')
        hits=[e for e in receipts.values() if e['source']['id']==player and e['amount']>0]
        victims=sorted({e['target']['id'] for e in hits})
        states={state for f in frames for actor in f['states'] for state in actor['states']}
        mana=[a['mana'] for f in frames for a in f['actors'] if a['actor']=='player']
        facts[name]=dict(frames=len(frames),hits=len(hits),victims=victims,states=sorted(states),
                         mana_start=mana[0],mana_min=min(mana),damage=[e['amount'] for e in hits])
        if not hits: errors.append(name+': no accepted damage')
        if name=='repeater' and len(victims)!=1: errors.append('Repeater must hit only one actor')
        if name=='rockets' and (len(victims)<2 or min(mana)>=mana[0]-3): errors.append('Rocket splash/mana not demonstrated')
        if name in ('slow', 'traps'):
            positions=[a['position'] for f in frames for a in f['actors'] if a['actor']=='enemy']
            travel=sum((b-a)**2 for a,b in zip(positions[0],positions[-1]))**.5
            facts[name]['enemy_travel']=travel
            if travel<2: errors.append(name+': moving opponent not demonstrated')
        if name=='slow' and 'slowed' not in states: errors.append('No replicated slow')
        if name=='traps' and ('rooted' not in states or not any(e.get('trap_triggered') for e in receipts.values())): errors.append('No triggered trap/root')
        if name.startswith('rocket-') and not any(e.get('area_impact') for e in hits): errors.append(name+': no authoritative detonation')
    if facts['rocket-far']['damage'] and facts['rocket-near']['damage']:
        if max(facts['rocket-far']['damage'])<=max(facts['rocket-near']['damage']): errors.append('Far rocket must deal more damage')
    return dict(pass_=not errors,errors=errors,facts=facts)


def ffquote(p):
    return str(p).replace("'", "'\\''")


def edit(report, raw, out, assets):
    ffmpeg=find_ffmpeg()
    if not ffmpeg: raise RuntimeError('An installed ffmpeg with drawtext is required')
    font=assets/'ui/Inter.ttf'
    clips=[]; timeline=[]; sounds=[]; offset=0.0; clean_lines=['ffconcat version 1.0']
    audio=json.loads((assets/'audio/manifest.json').read_text())
    for index,(name,title,subtitle,color) in enumerate(CHAPTERS):
        frames=next(c['frames'] for c in report['chapters'] if c['chapter']==name)
        duration=frames[-1]['seconds']-frames[0]['seconds']+1/30
        concat=out/f'{name}.ffconcat'
        lines=['ffconcat version 1.0']
        for n,f in enumerate(frames):
            dt=frames[n+1]['seconds']-f['seconds'] if n+1<len(frames) else 1/30
            lines += [f"file '{ffquote(raw/f['file'])}'",f'duration {max(dt,.001):.6f}']
        clean_lines.extend(lines[1:])
        lines.append(f"file '{ffquote(raw/frames[-1]['file'])}'")
        concat.write_text('\n'.join(lines)+'\n')
        clip=out/f'{name}.mp4'
        # Chapter text is kept in separate UTF-8 files, avoiding filter-language escapes.
        title_file=out/f'{name}-title.txt';title_file.write_text(title)
        sub_file=out/f'{name}-subtitle.txt';sub_file.write_text(subtitle)
        vf=(f'scale=1080:1440,pad=1080:1920:0:220:color=0x0B0B14,'
            f'drawbox=x=70:y=178:w=940:h=4:color={color}:t=fill,'
            f"drawtext=fontfile='{font}':text='WILDSPARK / OMOBA':fontsize=32:fontcolor=white@0.65:x=70:y=65,"
            f"drawtext=fontfile='{font}':textfile='{title_file}':fontsize=48:fontcolor=white:x='70+30*max(0,1-t/0.25)':y=113,"
            f"drawtext=fontfile='{font}':textfile='{sub_file}':fontsize=32:fontcolor=white:x=(w-tw)/2:y=1740,"
            f"drawtext=fontfile='{font}':text='COMBAT TEST  /  {index+1:02d} OF 06':fontsize=22:fontcolor=white@0.45:x=70:y=1840,"
            f'drawbox=x=70:y=1810:w={round(940*(index+1)/6)}:h=4:color={color}:t=fill,format=yuv420p')
        subprocess.run([ffmpeg,'-y','-loglevel','error','-f','concat','-safe','0','-i',str(concat),'-vf',vf,
                        '-r','30','-t',str(duration),'-c:v','libx264','-preset','fast','-crf','18',
                        '-movflags','+faststart',str(clip)],check=True)
        clips.append(clip);timeline.append(dict(chapter=name,start=offset,duration=duration,title=title))
        def key(v):return (v['actor'],v['moment'],v['id'],v['row'])
        known={key(v) for v in frames[0]['audio']}
        for frame in frames[1:]:
            for voice in frame['audio']:
                k=key(voice)
                if k in known:continue
                known.add(k)
                cue=audio['cues'].get(voice['base'])
                if cue:
                    sounds.append(dict(time=offset+frame['seconds']-frames[0]['seconds'],
                                       path=str(assets/cue['path']),speed=voice['speed'],slice=voice['slice'],gain=voice['gain']*cue['gain']))
        offset+=duration
    joined=out/'clips.ffconcat';joined.write_text('\n'.join(f"file '{ffquote(c)}'" for c in clips)+'\n')
    silent=out/'wildspark-demo-silent.mp4'
    subprocess.run([ffmpeg,'-y','-loglevel','error','-f','concat','-safe','0','-i',str(joined),'-c','copy',str(silent)],check=True)
    # Reconstruct the game's own recorded row voices in the edit; keep the licensed bed quiet.
    inputs=['-stream_loop','-1','-i',str(assets/'audio/music/arena.ogg')]
    filters=[f'[0:a]atrim=duration={offset},asetpts=PTS-STARTPTS,volume=0.12,afade=t=in:d=0.4,afade=t=out:st={max(0,offset-.7)}:d=0.7[music]']
    labels=['[music]']
    for n,s in enumerate(sounds,1):
        inputs+=['-i',s['path']]
        trim={'tick':'atrim=duration=0.12,','body':'atrim=start=0.06:duration=0.34,','tail':'atrim=start=0.2,'}.get(s['slice'],'')
        filters.append(f'[{n}:a]{trim}asetpts=PTS-STARTPTS,aresample=48000,asetrate={round(48000*s["speed"])},aresample=48000,volume={s["gain"]},adelay={round(s["time"]*1000)}:all=1[s{n}]')
        labels.append(f'[s{n}]')
    filters.append(''.join(labels)+f'amix=inputs={len(labels)}:normalize=0,loudnorm=I=-14:TP=-2:LRA=9,aresample=48000,alimiter=limit=0.75:level=false,atrim=duration={offset}[mix]')
    graph=out/'audio.ffgraph';graph.write_text(';\n'.join(filters))
    soundtrack=out/'soundtrack.m4a'
    subprocess.run([ffmpeg,'-y','-loglevel','error',*inputs,'-filter_complex_script',str(graph),'-map','[mix]','-c:a','aac','-b:a','192k',str(soundtrack)],check=True)
    final=out/'wildspark-demo-social.mp4'
    subprocess.run([ffmpeg,'-y','-loglevel','error','-i',str(silent),'-i',str(soundtrack),'-map','0:v','-map','1:a','-c','copy','-movflags','+faststart',str(final)],check=True)
    clean_concat=out/'clean.ffconcat';clean_concat.write_text('\n'.join(clean_lines)+'\n')
    subprocess.run([ffmpeg,'-y','-loglevel','error','-f','concat','-safe','0','-i',str(clean_concat),'-i',str(soundtrack),'-map','0:v','-map','1:a','-r','30','-t',str(offset),'-c:v','libx264','-preset','fast','-crf','18','-pix_fmt','yuv420p','-c:a','copy','-movflags','+faststart',str(out/'wildspark-demo-clean.mp4')],check=True)
    (out/'edit.json').write_text(json.dumps(dict(timeline=timeline,sounds=sounds,duration=offset,audio='post-production reconstruction from recorded game row voices; packaged CC0 music'),ensure_ascii=False,indent=2)+'\n')
    return final


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--client-bin',type=Path,default=Path('/Users/wotori/git/ekza/omoba-bevy/target-b/debug/client'))
    p.add_argument('--server-bin',type=Path,default=Path('/Users/wotori/git/ekza/omoba-bevy/target-b/debug/server'))
    p.add_argument('--assets',type=Path,default=ROOT/'client/assets')
    p.add_argument('--edit-only',action='store_true')
    p.add_argument('--capture-only',action='store_true')
    a=p.parse_args();a.output=a.output.resolve();a.assets=a.assets.resolve();a.client_bin=a.client_bin.resolve();a.server_bin=a.server_bin.resolve()
    a.output.mkdir(parents=True,exist_ok=True);raw=a.output/'raw'
    report=json.loads((raw/'demo.json').read_text()) if a.edit_only else capture(a,raw)
    verdict=validate(report,raw)
    manifest=dict(source=source_identity(),client_sha256=sha256(a.client_bin),server_sha256=sha256(a.server_bin),validation=verdict)
    (a.output/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    if not verdict['pass_']:raise RuntimeError(verdict['errors'])
    if not a.capture_only: print(edit(report,raw,a.output,a.assets))
    print(json.dumps(verdict,indent=2))

if __name__=='__main__':main()
