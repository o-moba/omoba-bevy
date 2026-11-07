from pathlib import Path
from urllib.parse import quote,urlencode
import json,os,subprocess,time,uuid
root=Path.cwd();out=root/'.agent/codex-ranked-readiness/ready-runtime';own=out/'retention-background';own.mkdir(exist_ok=True)
container='omoba-mm-seasons-pg';data=json.loads(subprocess.check_output(['docker','inspect',container],text=True))[0];cfg=dict(v.split('=',1) for v in data['Config']['Env'] if '=' in v);user=cfg.get('POSTGRES_USER','postgres');database=cfg.get('POSTGRES_DB',user)
assert database=='omoba_test' and any(p['HostPort']=='55582' for p in data['NetworkSettings']['Ports']['5432/tcp'])
schema='career_retention_probe_'+uuid.uuid4().hex
assert schema.startswith('career_retention_probe_') and schema.replace('_','').isalnum()
def sql(query):
 return subprocess.check_output(['docker','exec','-i',container,'psql','-X','-qAt','-v','ON_ERROR_STOP=1','-U',user,'-d',database],input=query,text=True,stderr=subprocess.PIPE).strip()
url=f'postgres://{quote(user,safe="")}:{quote(cfg.get("POSTGRES_PASSWORD",""),safe="")}@127.0.0.1:55582/{database}?'+urlencode({'options':'-csearch_path='+schema})
env={k:v for k,v in os.environ.items() if not k.startswith('OMOBA_') and k!='SERVER_ADDR'};env['OMOBA_DATABASE_URL']=url
cmd=json.loads((out/'server-compile.json').read_text())['commands'][-1]['command'].copy();cmd.remove('--test');cmd.remove('--emit=metadata');cmd[cmd.index('--crate-name')+1]='prepare_retention_fixture';cmd[cmd.index(str(root/'server/src/main.rs'))]=str(out/'prepare_retention_fixture.rs');cmd[cmd.index('-o')+1]=str(out/'prepare-retention-fixture')
subprocess.run(cmd,env={**env,'CARGO_MANIFEST_DIR':str(root/'server')},check=True,stdout=subprocess.DEVNULL)
server=None;created=False
try:
 sql('CREATE SCHEMA '+schema);created=True
 with (own/'fixture.log').open('w') as log:subprocess.run([str(out/'prepare-retention-fixture')],env=env,stdout=log,stderr=log,check=True)
 assert sql('SELECT max(version) FROM '+schema+'.career_schema_version')=='13'
 sql("UPDATE "+schema+".career_ready_checks SET updated_at=clock_timestamp()-interval '8 days'")
 assert sql('SELECT count(*) FROM '+schema+'.career_ready_checks')=='1'
 env.update(SERVER_ADDR='127.0.0.1:47500',OMOBA_SERVER_ROLE='lobby',OMOBA_MATCH_ROOT=str(own/'matches'),OMOBA_CAREER_OUTBOX=str(own/'lobby-outbox'),OMOBA_MATCH_CAPACITY='1',OMOBA_MATCH_FIRST_PORT='47600',OMOBA_MATCH_PUBLIC_HOST='127.0.0.1',OMOBA_MATCH_BIND_IP='127.0.0.1')
 started=time.monotonic()
 with (own/'lobby.log').open('w') as log:
  server=subprocess.Popen([str(out/'server')],env=env,stdout=log,stderr=log)
  while time.monotonic()-started<390:
   if server.poll() is not None:raise RuntimeError('owned server exited before maintenance')
   if sql('SELECT count(*) FROM '+schema+'.career_ready_checks')=='0':break
   time.sleep(5)
  elapsed=time.monotonic()-started
  assert 295<=elapsed<390, 'actual five-minute warmup did not produce bounded background cleanup'
  assert 'READY_MAINTENANCE reconciled=0 pruned=1 deferred=0' in (own/'lobby.log').read_text()
  counts=json.loads(sql("SELECT json_build_object('profiles',(SELECT count(*) FROM "+schema+".career_profiles),'members',(SELECT count(*) FROM "+schema+".career_ready_members),'claims',(SELECT count(*) FROM "+schema+".career_ready_claims),'events',(SELECT count(*) FROM "+schema+".career_queue_dodge_events),'matches',(SELECT count(*) FROM "+schema+".career_matches),'played',(SELECT sum(matches_played) FROM "+schema+".career_profiles))"))
  assert counts=={'profiles':1,'members':0,'claims':0,'events':0,'matches':0,'played':0}
  report={'status':'PASS','schema':13,'scope':'actual production lobby and real closed-consent API fixture in isolated PostgreSQL18.6 schema; no players, WAN or load proof','warmup_seconds':round(elapsed,2),'pruned':1,'counts':counts,'source':'current native server, default300s production warmup/60s interval; no test timing override','notifications_sent':0}
finally:
 if server is not None:
  server.terminate()
  try:server.wait(timeout=10)
  except subprocess.TimeoutExpired:server.kill();server.wait(timeout=10)
 if created:sql('DROP SCHEMA '+schema+' CASCADE')
(own/'report.json').write_text(json.dumps({**report,'owned_schema_removed':True},indent=2)+'\n')
print(json.dumps({**report,'owned_schema_removed':True}),flush=True)
