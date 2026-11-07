from pathlib import Path
import json,os,subprocess
from urllib.parse import quote
root=Path.cwd();out=root/'.agent/codex-ranked-readiness/queue-delivery';data=json.loads(subprocess.check_output(['docker','inspect','omoba-mm-seasons-pg'],text=True))[0]
cfg=dict(x.split('=',1) for x in data['Config']['Env'] if '=' in x);user=cfg.get('POSTGRES_USER','postgres');password=cfg.get('POSTGRES_PASSWORD','');database=cfg.get('POSTGRES_DB',user)
env={k:v for k,v in os.environ.items() if not k.startswith('OMOBA_')};env['OMOBA_TEST_DATABASE_URL']=f'postgres://{quote(user,safe="")}:{quote(password,safe="")}@127.0.0.1:55582/{quote(database,safe="")}'
result=subprocess.run([str(out/'backend-tests'),'--include-ignored','--test-threads=1'],env=env)
raise SystemExit(result.returncode)
