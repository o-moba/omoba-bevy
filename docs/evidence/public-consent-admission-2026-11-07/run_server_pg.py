from pathlib import Path
from urllib.parse import quote
import json,os,subprocess,sys
root=Path.cwd();out=root/'.agent/codex-ranked-readiness/ready-runtime';data=json.loads(subprocess.check_output(['docker','inspect','omoba-mm-seasons-pg'],text=True))[0];cfg=dict(v.split('=',1) for v in data['Config']['Env'] if '=' in v);user=cfg.get('POSTGRES_USER','postgres');database=cfg.get('POSTGRES_DB',user);assert database=='omoba_test';assert any(p['HostPort']=='55582' for p in data['NetworkSettings']['Ports']['5432/tcp']);env={k:v for k,v in os.environ.items() if not k.startswith('OMOBA_')};env['OMOBA_TEST_DATABASE_URL']=f'postgres://{quote(user,safe="")}:{quote(cfg.get("POSTGRES_PASSWORD",""),safe="")}@127.0.0.1:55582/{database}'
subprocess.run([str(out/'server-tests'),*sys.argv[1:],'--include-ignored','--test-threads=1'],env=env,check=True)
