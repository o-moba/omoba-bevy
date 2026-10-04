"""Local-only read-only probe and one EN/852x393 native home capture."""
import json, os, pathlib, socket, subprocess, threading, sys
ROOT = pathlib.Path(__file__).resolve().parents[3]
OUT = pathlib.Path(__file__).resolve().parent / 'raw'
BIN = pathlib.Path('/Users/wotori/git/ekza/omoba-bevy/target/debug')
TOOL = BIN / 'examples/compatibility'

def cli(*args):
    p = subprocess.run([str(TOOL), *map(str,args)], capture_output=True, text=True, timeout=10)
    return dict(code=p.returncode, stdout=json.loads(p.stdout), stderr=p.stderr)

manifest = cli('manifest')['stdout']
(OUT / 'compatibility.json').write_text(json.dumps(manifest,indent=2))

def fixture(sock, contract):
    sock.settimeout(.2)
    while not stop.is_set():
        try: data, peer = sock.recvfrom(2049)
        except socket.timeout: continue
        packet = json.loads(data)
        if packet.get('type') == 'compatibility_probe':
            sock.sendto(json.dumps(dict(type='compatibility_report', nonce=packet['nonce'], server=contract)).encode(),peer)

if '--ui-only' not in sys.argv:
    results=[]
    for field in [None, 'protocol', 'catalog', 'geometry', 'gameplay', 'handshake', 'release']:
        contract=dict(manifest)
        if field: contract[field] = contract[field]+1 if isinstance(contract[field],int) else contract[field]+'x'
        stop=threading.Event()
        with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as sock:
            sock.bind(('127.0.0.1',0))
            thread=threading.Thread(target=fixture,args=(sock,contract)); thread.start()
            result=cli('check',f'127.0.0.1:{sock.getsockname()[1]}', OUT / 'compatibility.json')
            stop.set();thread.join()
        expected=0 if field in (None,'release') else 2
        assert result['code']==expected,result
        results.append(dict(field=field,**result))
    with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as sock:
        sock.bind(('127.0.0.1',0))
        result=cli('check', f'127.0.0.1:{sock.getsockname()[1]}', OUT/'compatibility.json')
        assert result['code']==3 and result['stdout']['issue']=='unavailable',result
        results.append(dict(field='silent-server',**result))
    (OUT/'cli-matrix.json').write_text(json.dumps(results,indent=2))
    # Real compiled standalone server: compatibility traffic must not allocate a player.
    with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as sock:
        sock.bind(('127.0.0.1',0)); address=f'127.0.0.1:{sock.getsockname()[1]}'
    env=dict(os.environ,SERVER_ADDR=address,OMOBA_MATCH_MODE='dev',OMOBA_CAREER_BACKEND='memory')
    for key in list(env):
        if key.startswith('OMOBA_DATABASE') or key in ['DATABASE_URL','OMOBA_SERVER_ROLE']:env.pop(key)
    with (OUT/'live-server.log').open('w') as log:
        server=subprocess.Popen([str(BIN/'server')],cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT)
        try:
            import time
            deadline=time.monotonic()+30
            while 'is listening' not in (OUT/'live-server.log').read_text(errors='replace'):
                if server.poll() is not None or time.monotonic()>deadline:
                    raise RuntimeError('local server did not report readiness')
                time.sleep(.1)
            result=cli('check',address,OUT/'compatibility.json')
            assert result['code']==0,result
            (OUT/'live-server-probe.json').write_text(json.dumps(result,indent=2))
        finally:
            server.terminate();server.wait(timeout=10)
# One actual native UI render, fed an explicit synthetic mismatch report.
stop=threading.Event()
contract=dict(manifest,protocol=manifest['protocol']+1,release='0.42.0')
with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as sock:
    sock.bind(('127.0.0.1',0))
    thread=threading.Thread(target=fixture,args=(sock,contract));thread.start()
    env=dict(os.environ,GAME_SERVER_ADDR=f'127.0.0.1:{sock.getsockname()[1]}',
       OMOBA_CLIENT_CONFIG_DIR=str(OUT/'ui-config'),OMOBA_ASSET_DIR=str(ROOT/'client/assets'),
       OMOBA_PLAYER_VISUAL_MODE='models3d',OMOBA_TOUCH_CONTROLS='1',OMOBA_LANGUAGE='en',
       OMOBA_FRONTEND_QA_OUTPUT=str(OUT/'ui'),OMOBA_HOME_ONLY_QA='1',OMOBA_QA_WIDTH='852',OMOBA_QA_HEIGHT='393',OMOBA_DEBUG_UI='0',OMOBA_QA_SYNTHETIC_FOCUS='1')
    with (OUT/'native.log').open('w') as log:
        try:
            result=subprocess.run([str(BIN/'client')],cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT,timeout=200)
            (OUT/'native-exit.json').write_text(json.dumps({'exit':result.returncode,'synthetic_report':True,'physical_device':False}))
        finally:stop.set();thread.join()
print('Native capture exited',result.returncode)
