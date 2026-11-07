from pathlib import Path
from urllib.parse import quote
import hashlib, json, os, secrets, signal, subprocess, sys, time

root=Path.cwd(); out=root/'.agent/codex-ranked-readiness/load-input20/live-v2'; out.mkdir(exist_ok=False)
name='omoba-mm-input20-20261107-v2'
image=subprocess.check_output(['docker','inspect','omoba-mm-seasons-pg','--format','{{.Config.Image}}'],text=True).strip()
env={k:v for k,v in os.environ.items() if not k.startswith('OMOBA_')}
env['PATH']='/Applications/Postgres.app/Contents/Versions/18/bin:/opt/homebrew/bin:'+env['PATH']
password=secrets.token_hex(24)
created=False
report={'status':'FAIL','image':image,'owned_container':name,'runs':[],'migration_sha256':{},'scope':'Loopback-only disposable current13/11 PostgreSQL, actual restricted runtime grants; same local engine and AI generators, not RU/WAN/device/SLA acceptance'}
def call(args, **kwargs):
    result=subprocess.run(args,capture_output=True,text=True,**kwargs)
    if result.returncode:
        raise RuntimeError(result.stderr.replace(password,'[fixture-secret]')[-2000:])
    return result.stdout.strip()
def sql(text):
    return call(['docker','exec','-i',name,'psql','-X','-U','postgres','-d','omoba_test','-At','-v','ON_ERROR_STOP=1'],input=text)
try:
    docker_env=env|{'POSTGRES_PASSWORD':password,'POSTGRES_DB':'omoba_test'}
    call(['docker','run','-d','--name',name,'-e','POSTGRES_PASSWORD','-e','POSTGRES_DB','-p','127.0.0.1::5432',image,'-c','max_connections=160'],env=docker_env)
    created=True
    deadline=time.monotonic()+40
    while subprocess.run(['docker','exec',name,'pg_isready','-h','127.0.0.1','-U','postgres','-d','omoba_test'],capture_output=True).returncode:
        assert time.monotonic()<deadline,'isolated DB startup timed out'
        time.sleep(.2)
    sql('CREATE TABLE career_schema_version(version integer primary key)')
    for migration in sorted((root/'career-store/migrations/postgres').glob('[0-9]*.sql')):
        version=int(migration.name.split('_',1)[0]); text=migration.read_text()
        report['migration_sha256'][str(migration.relative_to(root))]=hashlib.sha256(migration.read_bytes()).hexdigest()
        if 'INSERT INTO career_schema_version' not in text:
            text+=f'\nINSERT INTO career_schema_version VALUES({version});'
        sql('BEGIN;'+text+'\nCOMMIT;')
    for migration in sorted((root/'account-api/migrations').glob('[0-9]*.sql')):
        report['migration_sha256'][str(migration.relative_to(root))]=hashlib.sha256(migration.read_bytes()).hexdigest()
        sql('BEGIN;'+migration.read_text()+'\nCOMMIT;')
    sql(f"CREATE ROLE cadence_game LOGIN PASSWORD '{password}'; CREATE ROLE cadence_portal NOLOGIN; REVOKE CREATE ON SCHEMA public FROM PUBLIC;")
    grants=root/'account-api/ops/grants.sql'
    report['migration_sha256'][str(grants.relative_to(root))]=hashlib.sha256(grants.read_bytes()).hexdigest()
    sql('\\set game_role cadence_game\n\\set portal_role cadence_portal\n'+grants.read_text())
    port=call(['docker','port',name,'5432/tcp']).split(':')[-1]
    env['OMOBA_TEST_DATABASE_URL']=f'postgres://cadence_game:{quote(password,safe="")}@127.0.0.1:{port}/omoba_test'
    report['postgres']=sql('SELECT version()')
    report['max_connections']=int(sql('SHOW max_connections'))
    report['career_schema']=int(sql('SELECT max(version) FROM career_schema_version'))
    report['portal_schema']=int(sql('SELECT max(version) FROM portal.schema_version'))
    report['runtime_role']=json.loads(sql("SELECT json_build_object('rolsuper',rolsuper,'rolcreaterole',rolcreaterole,'rolcreatedb',rolcreatedb) FROM pg_roles WHERE rolname='cadence_game'"))
    assert report['career_schema']==13 and report['portal_schema']==11
    assert all(value is False for value in report['runtime_role'].values())
    for clients, seconds in [(10,30),(1000,120)]:
        label=f'instrumented-{clients}'
        command=[sys.executable,'scripts/public_playing_capacity.py','--server-bin',str(root/'.agent/codex-ranked-readiness/load-opt1/server'),'--bots-bin',str(root/'.agent/codex-ranked-readiness/load-input20/bots'),'--output',str(out/label),'--clients',str(clients),'--seconds',str(seconds),'--timeout','450','--port','47600','--worker-port','47700']
        if clients>100: command+=['--allow-large']
        with (out/(label+'.log')).open('w') as log:
            result=subprocess.run(command,env=env,stdout=log,stderr=subprocess.STDOUT)
        run=json.loads((out/label/'report.json').read_text())
        report['runs'].append({k:run.get(k) for k in ('status','error','clients','elapsed_seconds','all_running_after_seconds','wire')})
        print(json.dumps(report['runs'][-1]),flush=True)
        assert result.returncode==0, f'{label} rejected; preserve raw reports'
    report['status']='PASS'
except Exception as error:
    report['error']=type(error).__name__+': '+str(error)
    raise
finally:
    groups={x['process_group'] for path in out.glob('*/owned-processes.json') for x in json.loads(path.read_text())}
    allowed={str(root/'.agent/codex-ranked-readiness/load-opt1/server'),str(root/'.agent/codex-ranked-readiness/load-input20/bots')}
    def inventory():
        found=[]
        for line in call(['ps','-axo','pid=,uid=,pgid=,state=,comm=']).splitlines():
            parts=line.strip().split(None,4)
            if len(parts)!=5:continue
            pid,uid,pgid=map(int,parts[:3])
            if pgid in groups:
                found.append({'pid':pid,'uid':uid,'pgid':pgid,'state':parts[3],'command':parts[4],
                              'verified':uid==os.getuid() and parts[4] in allowed and not parts[3].startswith('Z')})
        return found
    try:
        remaining=inventory(); report['initial_cleanup_inventory']=remaining
        for row in remaining:
            if row['verified']:
                try:os.kill(row['pid'],signal.SIGTERM)
                except ProcessLookupError:pass
        deadline=time.monotonic()+20
        while inventory() and time.monotonic()<deadline:time.sleep(.2)
        remaining=inventory()
        for row in remaining:
            if row['verified']:
                try:os.kill(row['pid'],signal.SIGKILL)
                except ProcessLookupError:pass
        deadline=time.monotonic()+5
        while inventory() and time.monotonic()<deadline:time.sleep(.2)
        report['final_cleanup_inventory']=inventory()
        report['no_owned_process_groups_remain']=not report['final_cleanup_inventory']
    except Exception as error:
        report['cleanup_error']=type(error).__name__+': '+str(error)
        report['no_owned_process_groups_remain']=False
    try:
        if created:
            logs=subprocess.run(['docker','logs',name],capture_output=True,text=True)
            (out/'postgres.log').write_text((logs.stdout+logs.stderr).replace(password,'[fixture-secret]'))
            if report['no_owned_process_groups_remain']:
                subprocess.run(['docker','rm','-f','-v',name],check=True,capture_output=True)
        report['cleanup_complete']=name not in call(['docker','ps','-a','--format','{{.Names}}']).splitlines()
    except Exception as error:
        report['cleanup_error']=type(error).__name__+': '+str(error)
        report['cleanup_complete']=False
    if not report['cleanup_complete'] or not report['no_owned_process_groups_remain']:report['status']='FAIL'
    (out/'isolated-summary.json').write_text(json.dumps(report,indent=2)+'\n')
assert report['cleanup_complete'] and report['no_owned_process_groups_remain'],'cleanup incomplete; inspect preserved audit'
