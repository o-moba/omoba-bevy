from pathlib import Path
from urllib.parse import quote
import json,os,subprocess,sys
root=Path.cwd();out=root/'.agent/codex-ranked-readiness/ready-runtime';scenario=sys.argv[1]
data=json.loads(subprocess.check_output(['docker','inspect','omoba-mm-seasons-pg'],text=True))[0];cfg=dict(v.split('=',1) for v in data['Config']['Env'] if '=' in v);user=cfg.get('POSTGRES_USER','postgres');database=cfg.get('POSTGRES_DB',user);assert database=='omoba_test';assert any(p['HostPort']=='55582' for p in data['NetworkSettings']['Ports']['5432/tcp'])
env={k:v for k,v in os.environ.items() if not k.startswith('OMOBA_')};env.update(HARNESS_SERVER_BIN=str(out/'server'),HARNESS_RANKED_FIXTURE_BIN=str(out/'qualify-ranked-fixture'),OMOBA_TEST_DATABASE_URL=f'postgres://{quote(user,safe="")}:{quote(cfg.get("POSTGRES_PASSWORD",""),safe="")}@127.0.0.1:55582/{database}')
args=['node','scripts/test_public_mvp.mjs','--scenario='+scenario,'--clients='+('1' if scenario in ('ready-check','cross-lobby-ready','legacy-penalty','drain') else '10'),'--preference='+('bot_practice' if scenario in ('ready-check','cross-lobby-ready','legacy-penalty','drain') else 'humans_only'),'--seconds=1','--port=46500','--worker-port=46600','--output='+str(out/('live-'+scenario+('-'+sys.argv[2] if len(sys.argv)>2 else '')))]
if scenario=='handoff-release':args=[('--preference=quick' if a.startswith('--preference=') else a) for a in args]
if scenario=='party-draft':args+=['--exercise-stale-handoff']
if scenario=='drain':args+=['--capacity=2']
result=subprocess.run(args,env=env);raise SystemExit(result.returncode)
