"""Actual source SQL under a temporary NOLOGIN role; entire probe rolls back."""
from pathlib import Path
import json,os,re,secrets,subprocess,time
root=Path.cwd();out=root/'.agent/codex-ranked-readiness/queue-delivery';role=f'omoba_queue_acl_{os.getpid()}_{secrets.token_hex(3)}';profile=secrets.token_hex(32);event=secrets.token_hex(16);at=int(time.time()*1000)
source=(root/'career-store/src/career_store/queue_discipline.rs').read_text()
def query(prefix):
 matches=re.findall(r'"('+re.escape(prefix)+r'[^"\n]*)"',source);assert len(matches)==1,(prefix,len(matches));return matches[0]
def bind(sql,values):
 for n,value in enumerate(values,1):sql=sql.replace('$'+str(n),str(value))
 return sql+';'
grant_lines=[l for l in (root/'account-api/ops/grants.sql').read_text().splitlines() if l.startswith('GRANT ') and 'career_queue_' in l];assert len(grant_lines)==2
grants='\n'.join(grant_lines).replace(':"game_role"','"'+role+'"')
insert=query('INSERT INTO career_queue_dodge_events(');penalty=query('INSERT INTO career_queue_penalties(');active=re.search(r'const ACTIVE_SQL: &str = "([^"\n]*)";',source)[1]
statements=['BEGIN;',f'CREATE ROLE "{role}" NOLOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION;',f"INSERT INTO career_profiles(profile_id,nickname) VALUES('{profile}','QueueRights{event[:6]}#0001');",f'GRANT USAGE ON SCHEMA public TO "{role}";',f'GRANT SELECT ON career_schema_version TO "{role}";',f'GRANT SELECT,UPDATE ON career_profiles TO "{role}";',grants,f'SET LOCAL ROLE "{role}";',bind(query('SELECT profile_id FROM career_profiles'),["'"+profile+"'"]),bind(insert,["'"+event+"'","'"+profile+"'",at]),bind(insert,["'"+event+"'","'"+profile+"'",at]),bind(penalty,["'"+profile+"'",at,at+60000,1]),bind(penalty,["'"+profile+"'",at,at+300000,2]),bind(active,["'"+profile+"'"]),'''DO $$ BEGIN
 BEGIN DELETE FROM career_queue_dodge_events; RAISE EXCEPTION 'event deletion unexpectedly permitted'; EXCEPTION WHEN insufficient_privilege THEN NULL; END;
 BEGIN UPDATE career_queue_dodge_events SET observed_at_ms=0; RAISE EXCEPTION 'event mutation unexpectedly permitted'; EXCEPTION WHEN insufficient_privilege THEN NULL; END;
END $$;''','ROLLBACK;']
config=json.loads(subprocess.check_output(['docker','inspect','omoba-mm-seasons-pg'],text=True))[0]['Config']['Env'];cfg=dict(x.split('=',1) for x in config if '=' in x);user=cfg.get('POSTGRES_USER','postgres');db=cfg.get('POSTGRES_DB',user)
cmd=['docker','exec','-i','omoba-mm-seasons-pg','psql','-X','-qAt','-v','ON_ERROR_STOP=1','-U',user,'-d',db]
result=subprocess.run(cmd,input='\n'.join(statements),text=True,capture_output=True)
if result.returncode:print(result.stderr);raise SystemExit(result.returncode)
assert len(result.stdout.strip().splitlines())==2,result.stdout
check=f"SELECT json_build_object('roles',(SELECT count(*) FROM pg_roles WHERE rolname='{role}'),'profiles',(SELECT count(*) FROM career_profiles WHERE profile_id='{profile}'),'events',(SELECT count(*) FROM career_queue_dodge_events WHERE event_id='{event}'));"
left=json.loads(subprocess.check_output(cmd,input=check,text=True));assert left=={'roles':0,'profiles':0,'events':0},left
report={'status':'PASS','scope':'Exact new source grants/SQL under temporary NOLOGIN role in owned local PG18.6; existing baseline profile/schema privileges supplied; all changes rolled back, not a production role rollout','allowed':['profile row lock','append/deduplicate queue event','insert/update queue penalty','read active cooldown'],'denied':['update event ledger','delete event ledger'],'remaining':left}
(out/'grants-proof.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
