#!/usr/bin/env python3
"""One native Warrior check with an actual Ekza SDK-imported prop.
The HTTP fixture is explicit local approval evidence, not a live Space listing.
"""
import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import threading
from urllib.parse import urlparse, parse_qs
from capture_standard_skills import capture


def verify_attachments(directory, imported_id):
    summary = json.loads((directory / 'qa-summary.json').read_text())
    frames = {frame['file']: frame for frame in summary['captures']}
    expected = {
        '06-idle.png': ('forge-sword', 'Idle'),
        '07-running.png': ('forge-sword', 'Run'),
        '02-sword-attack.png': ('forge-sword', 'Attack'),
        '03-hammer-attack.png': ('forge-hammer', 'Attack'),
        '04-scepter-avatar-swap.png': (imported_id, 'Attack'),
        '05-empty-hands.png': (None, 'Attack'),
    }
    assert set(frames) == set(expected) | {'01-selection.png'}
    sequence = 0
    for name, (item, animation) in expected.items():
        frame = frames[name]
        assert (directory / name).is_file(), name
        attached = frame['handhelds']
        assert len(attached) == (2 if item else 1), name
        assert all(w['loaded'] and w['attachment_error'] is not None
                   and w['attachment_error'] < 1e-4 for w in attached), name
        assert [w['id'] for w in attached if w['owner'] == 1] == ([item] if item else []), name
        assert [w['id'] for w in attached if w['owner'] == 2] == ['forge-sword'], name
        assert frame['animations']['1'][0] == animation, name
        avatar = 'orion' if name in ('04-scepter-avatar-swap.png', '05-empty-hands.png') else 'agnes'
        assert any(r['owner'] == 1 and r['avatar'] == avatar and r['bound_to_model']
                   and r['model'] == f'avatars/{avatar}.glb' for r in frame['rigs']), name
        if animation == 'Attack':
            assert frame['action']['slot'] == 255 and frame['action']['sequence'] > sequence, name
            sequence = frame['action']['sequence']
    return {'states': len(expected), 'independent_owners': 2,
            'max_attachment_error': max(w['attachment_error'] for f in frames.values() for w in f['handhelds'])}


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ('client-bin','server-bin','import-bin','builder-bin','assets','output'):
        p.add_argument('--'+name,type=Path,required=True)
    a=p.parse_args()
    a.output=a.output.resolve();a.output.mkdir(parents=True,exist_ok=False)
    assets=a.output/'assets'
    shutil.copytree(a.assets.resolve(),assets,copy_function=os.link,ignore=shutil.ignore_patterns('imported','ekza-manifest.json'))
    build=subprocess.run([str(a.builder_bin.resolve()),'--source',str(assets/'weapons/dawn-scepter.glb'),'--output-dir',str(a.output/'rendition')],capture_output=True,text=True,check=True)
    report=json.loads(build.stdout)
    (a.output/'builder-report.json').write_text(json.dumps(report,indent=2)+'\n')
    model=Path(report['assetPath']).read_bytes()
    assert report['sha256']==hashlib.sha256(model).hexdigest() and report['sizeBytes']==len(model)
    requests=[]
    class Handler(BaseHTTPRequestHandler):
        def log_message(self,*_): pass
        def do_GET(self):
            u=urlparse(self.path);requests.append(self.path)
            if u.path=='/v2/avatars':
                assert parse_qs(u.query)=={'project':['omoba'],'platform':['desktop'],'profile':['handheld-glb-v1']}
                entry=dict(id='ekza:avatar:3d11cb41-7c3a-4c32-bf90-b97431235935',name='SDK Dawn Scepter',access='free',projectSupport=[dict(projectId='omoba',platform='desktop',profile='handheld-glb-v1',status='approved')],renditions=[dict(platform='desktop',profile='handheld-glb-v1',format='glb',sha256=report['sha256'],sizeBytes=len(model),downloadUrl=f'http://127.0.0.1:{server.server_port}/scepter.glb')])
                body=json.dumps(dict(schema='ekza.avatar.catalog.v2',count=1,items=[entry])).encode()
            elif u.path=='/scepter.glb':body=model
            else:self.send_error(404);return
            self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
    server=ThreadingHTTPServer(('127.0.0.1',0),Handler)
    thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
    try:
        imported=subprocess.run([str(a.import_bin.resolve()),f'http://127.0.0.1:{server.server_port}'],env=dict(os.environ,OMOBA_ASSET_DIR=str(assets)),capture_output=True,text=True,check=True)
        (a.output/'import.log').write_text(imported.stdout+imported.stderr)
    finally:server.shutdown();server.server_close();thread.join()
    manifest=assets/'weapons/ekza-manifest.json'
    entry=json.loads(manifest.read_text())['items'][0]
    assert (assets/entry['model']).read_bytes()==model
    os.environ['OMOBA_WEAPON_MANIFEST']=str(manifest)
    result=capture('warrior',a.client_bin.resolve(),a.server_bin.resolve(),assets,a.output/'native',timeout=90,roster=True,handhelds=True,sdk_weapon=entry['id'])
    if result['pass']:
        try:
            result['attachment_checks'] = verify_attachments(a.output/'native', entry['id'])
        except (AssertionError, KeyError, ValueError) as error:
            result['pass'] = False
            result['attachment_error'] = str(error)
    (a.output/'sdk-proof.json').write_text(json.dumps(dict(fixture=True,requests=requests,imported_item=entry,result=result),indent=2)+'\n')
    print(json.dumps(result,indent=2))
    raise SystemExit(0 if result['pass'] else 1)

if __name__=='__main__':main()
