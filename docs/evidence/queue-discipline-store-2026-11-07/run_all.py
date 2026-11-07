from pathlib import Path
import hashlib,json,os,subprocess
from urllib.parse import quote
root=Path.cwd();out=root/'.agent/codex-ranked-readiness/queue-discipline';container='omoba-mm-seasons-pg'
data=json.loads(subprocess.check_output(['docker','inspect',container],text=True))[0]
config=dict(value.split('=',1) for value in data['Config']['Env'] if '=' in value)
user=config.get('POSTGRES_USER','postgres');password=config.get('POSTGRES_PASSWORD','');database=config.get('POSTGRES_DB',user)
def sql(statement):
 return subprocess.check_output(['docker','exec','-i',container,'psql','-X','-qAt','-v','ON_ERROR_STOP=1','-U',user,'-d',database],input=statement,text=True).strip()
before=sql('SELECT json_agg(version ORDER BY version) FROM public.career_schema_version')
env={k:v for k,v in os.environ.items() if not k.startswith('OMOBA_')}
env['OMOBA_TEST_DATABASE_URL']=f'postgres://{quote(user,safe="")}:{quote(password,safe="")}@127.0.0.1:55582/{quote(database,safe="")}'
process=subprocess.Popen([str(out/'store-tests'),'--include-ignored','--test-threads=2'],env=env)
pid=process.pid
assert process.wait()==0
left=int(sql(f"SELECT count(*) FROM pg_namespace WHERE nspname LIKE 'career_test_{pid}_%'"))
after=sql('SELECT json_agg(version ORDER BY version) FROM public.career_schema_version')
assert left==0 and before==after
sources={}
for folder,pattern in [('shared/src','*.rs'),('career-store/src','*.rs'),('career-store/migrations','*.sql')]:
 for path in (root/folder).rglob(pattern):sources[str(path.relative_to(root))]=hashlib.sha256(path.read_bytes()).hexdigest()
report={'status':'PASS','test_process_id':pid,'owned_schemas_remaining':left,'fixture_public_schema_before':json.loads(before),'fixture_public_schema_after':json.loads(after),'postgres_version':sql('SHOW server_version'),'scope':'Current source in temporary per-test schema11; existing fixture public schema untouched','source_sha256':sources,'test_binary_sha256':hashlib.sha256((out/'store-tests').read_bytes()).hexdigest()}
(out/'full-proof.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps({k:v for k,v in report.items() if k!='source_sha256'}))
