#!/usr/bin/env python3
"""Current worker Resume/crash recovery against a physically restored schema13. Never deploys.
All ratings/history below are explicit direct-SQL synthetic fixtures, not games.
"""
from __future__ import annotations
import argparse, hashlib, json, os, re, secrets, select, shutil, socket, socketserver, subprocess, threading, time
from urllib.parse import quote
from pathlib import Path


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',required=True,type=Path)
    parser.add_argument('--native-server',required=True,type=Path)
    parser.add_argument('--lobby-port',type=int,default=46530)
    parser.add_argument('--worker-port',type=int,default=46630)
    parser.add_argument('--image',default='omoba-mm-pitr-kit-rehearsal:20261107')
    args=parser.parse_args();out=args.output.resolve()
    assert not out.exists(),'Use a fresh output directory'
    assert shutil.disk_usage('/').free>8*1024**3,'Need8GiB for this small fixture'
    out.mkdir(parents=True)
    root=Path.cwd();run_id=secrets.token_hex(5)
    network='omoba-mm-pitr-'+run_id;primary=network+'-primary';repository=network+'-repo'
    native=None;native_log=None;proxy=None
    fixture_limits={'max_connections':80,'max_worker_processes':12,'max_wal_senders':14,'max_prepared_transactions':4,'max_locks_per_transaction':128}
    limit_config=''.join(name+'='+str(value)+'\n' for name,value in fixture_limits.items())
    class Checks(list):
        def append(self,item):
            super().append(item)
            print(json.dumps({'check':item,'status':'PASS'}),flush=True)
    created=[];checks=Checks();report={'status':'RUNNING','run_id':run_id,'checks':checks,
        'scope':'Local three-container physical recovery plus real one-human/nine-server-bot public worker Resume/crash recovery; not independent off-host/RU or Ranked ten-human acceptance',
        'source_parent':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),
        'fixture':'Synthetic legacy recovery checks followed by a separate actual signed public bot-practice game and current worker recovery',
        'wal_segment_mib':1, 'base_script_sha256':'fb244dcecf0f0db7e7b699c4890f0afd0735f45b8120ac56aee5d14544d6b940'}
    report['source_sha256']={str(p):hashlib.sha256(p.read_bytes()).hexdigest()
        for group in ('career-store/migrations/postgres/*.sql','account-api/migrations/*.sql','account-api/ops/grants.sql')
        for p in sorted(root.glob(group))}
    draft=root/'.agent/codex-ranked-readiness/pitr-parameters/current-tree/ops/vps/ru'
    report['review_helper_sha256']={n:hashlib.sha256((draft/n).read_bytes()).hexdigest() for n in ('ru_backup.py','ru_pitr.py')}
    historical=json.loads((root/'docs/evidence/pitr-recovery-limits-2026-11-07/candidate.json').read_text())
    for n,h in report['review_helper_sha256'].items():
        assert next(x['sha256'] for x in historical['applied_current_files'] if x['path']=='ops/vps/ru/'+n)==h,n
    native_path=args.native_server.resolve()
    prior=json.loads((root/'docs/evidence/critical-replies-2026-11-07/summary.json').read_text())
    assert hashlib.sha256(native_path.read_bytes()).hexdigest()==prior['binary_sha256']['server'],'Use the exact accepted current server binary'
    for path,h in prior['source_sha256'].items():
        if path.startswith(('server/','common/','shared/','passport/','career-store/')):
            assert hashlib.sha256((root/path).read_bytes()).hexdigest()==h,path
    report['native_server_sha256']=prior['binary_sha256']['server']
    report['fixture_recovery_limits']=fixture_limits
    report['public_probe_sha256']=hashlib.sha256((root/'scripts/test_public_mvp.mjs').read_bytes()).hexdigest()
    report['base_runner_sha256']='091adda542718ea988b8f8928035618a609b3004b313593158394f1570d16e74'
    def call(command,*,data=None,check=True,timeout=120):
        r=subprocess.run(command,input=data,text=True,capture_output=True,timeout=timeout)
        if check and r.returncode:
            # Secrets are never put in command arguments or printed from settings.
            raise RuntimeError(f'{command[0]} {command[1] if len(command)>1 else ""} failed({r.returncode}): {(r.stdout+r.stderr)[-1600:]}')
        return r
    def execute(container,*command,data=None,user=None,check=True,timeout=120):
        cmd=['docker','exec','-i']+(['--user',user] if user else [])+[container,*command]
        return call(cmd,data=data,check=check,timeout=timeout)
    def sql(statement,database='omoba_test',socket=None):
        cmd=['psql','-X','-qAt','-v','ON_ERROR_STOP=1','-U','postgres','-d',database]
        if socket:cmd+=['-h',socket,'-p','6545']
        return execute(primary,*cmd,data=statement,user='postgres').stdout.strip()
    def engine(*command,check=True,timeout=120):
        return execute(primary,'pgbackrest','--config=/tmp/pitr/pgbackrest.conf',
            '--config-include-path=/tmp/pitr/empty','--stanza=omoba-ru','--log-level-console=warn',
            '--log-level-file=off',*command,user='postgres',check=check,timeout=timeout)
    def kit(action,check=True):
        code="""import json,sys
sys.path.insert(0,'/tmp/pitr-code')
import ru_backup as rb,ru_pitr as rp
cfg=rb.Config('/tmp/pitr-code/backup.env');p=rp.Pitr(cfg)
command=sys.argv[1]
if command=='run-backup':
 try:result=rb.run_backup(rb.Context(cfg),reason='manual')
 except rb.BackupError as error:
  target=rb.Context(cfg).target()
  raise rb.BackupError(str(error)+' SFTP_RAW='+repr(target._batch(['-ls -ln '+target.path]))) from None
elif command=='run-drill':result=rb.run_drill(rb.Context(cfg))
elif command=='root-health':result=rb.health(rb.Context(cfg),remote=True)
elif command=='escrow':
 with open('/tmp/pitr-code/escrow.gpg','wb') as output:result=rb.run_escrow(rb.Context(cfg),open(cfg.recipient).read(),None,output,True)
elif command=='run-incomplete':
 from pathlib import Path
 path=Path(p.path);original=path.read_text()
 try:
  path.write_text(original.replace('archive-async = n','archive-async = y'))
  rb.run_backup(rb.Context(cfg),reason='pre-migration',release='pitr-fixture')
  raise AssertionError('Incomplete PITR unexpectedly certified success')
 except rb.BackupError:
  context=rb.Context(cfg)
  assert context.state['last_attempt']['ok'] is False
  logical=context.state['last_logical_success']['name']
  assert logical in context.target().list()
  result={'combined_success':False,'logical_archive_preserved':True}
 finally:path.write_text(original)
elif command=='cli-check':
 import io
 before=open(context_path:='/tmp/pitr-kit-backups/state.json','rb').read()
 output=io.StringIO();code=rb.main(['--env-file','/tmp/pitr-code/backup.env','pitr-check','--alert'],stdout=output)
 assert open(context_path,'rb').read()==before,'PITR alert/check overwrote backup metadata'
 result={'exit_code':code,'metadata_preserved':True}
elif command=='command':result=p.archive_command
elif command=='config':result=p.problems()
elif command=='health':result=rp.health(rb.Context(cfg))
else:result=getattr(p,command)()
print(json.dumps(result))
"""
        result=execute(primary,'python3','-c',code,action,check=check,timeout=180)
        return json.loads(result.stdout) if check else result
    def snapshot(socket=None):
        return json.loads(sql("""SELECT json_build_object(
'rating',rating,'rated_matches',rated_matches,'matches_played',matches_played,
'history',(SELECT count(*) FROM career_participants WHERE profile_id=repeat('a',64)),
'career_versions',(SELECT json_agg(version ORDER BY version) FROM career_schema_version),
'portal_versions',(SELECT json_agg(version ORDER BY version) FROM portal.schema_version),
'ready',(SELECT json_agg(to_jsonb(r) ORDER BY check_id) FROM career_ready_checks r),
'members',(SELECT json_agg(to_jsonb(m) ORDER BY check_id,profile_id) FROM career_ready_members m),
'claims',(SELECT json_agg(to_jsonb(c) ORDER BY profile_id) FROM career_ready_claims c),
'queue_events',(SELECT json_agg(to_jsonb(e) ORDER BY event_id) FROM career_queue_dodge_events e),
'queue_penalties',(SELECT json_agg(to_jsonb(p) ORDER BY profile_id) FROM career_queue_penalties p))
FROM career_profiles WHERE profile_id=repeat('a',64)""",socket=socket))
    def rights(socket=None):
        result=json.loads(sql("""SELECT json_build_object(
'game_retention',has_function_privilege('pitr_game','career_prune_closed_ready(integer)','EXECUTE'),
'portal_retention',has_function_privilege('pitr_portal','career_prune_closed_ready(integer)','EXECUTE'),
'game_consent_delete',has_table_privilege('pitr_game','career_ready_checks','DELETE'),
'portal_consent_read',has_table_privilege('pitr_portal','career_ready_checks','SELECT'),
'game_season_update',has_table_privilege('pitr_game','career_ranked_seasons','UPDATE'),
'portal_season_update',has_table_privilege('pitr_portal','career_ranked_seasons','UPDATE'),
'game_queue_append',has_table_privilege('pitr_game','career_queue_dodge_events','INSERT'),
'game_queue_delete',has_table_privilege('pitr_game','career_queue_dodge_events','DELETE'),
'game_schema_create',has_schema_privilege('pitr_game','public','CREATE'),
'portal_schema_create',has_schema_privilege('pitr_portal','public','CREATE'),
'definer_policy', (SELECT prosecdef AND proconfig = ARRAY['search_path=public, pg_temp'] FROM pg_proc WHERE oid='career_prune_closed_ready(integer)'::regprocedure),
'roles_unprivileged',(SELECT bool_and(NOT rolsuper AND NOT rolcreaterole AND NOT rolcreatedb AND NOT rolreplication AND NOT rolbypassrls) FROM pg_roles WHERE rolname IN ('pitr_game','pitr_portal')))
""",socket=socket))
        expected={key: key in ('game_retention','game_queue_append','definer_policy','roles_unprivileged') for key in result}
        assert result==expected,result
        # Exercise the restored SECURITY DEFINER policy through the actual role.
        # Roll back so verification never changes the recovered evidence.
        assert sql('BEGIN; SET LOCAL ROLE pitr_game; SELECT career_prune_closed_ready(1); ROLLBACK;',socket=socket)=='1'
        for role,statement in [('pitr_game','DELETE FROM career_ready_checks'),
                               ('pitr_portal','SELECT career_prune_closed_ready(1)'),
                               ('pitr_game','DELETE FROM career_queue_dodge_events'),
                               ('pitr_game','UPDATE career_ranked_seasons SET closed_at_ms=ends_at_ms')]:
            cmd=['psql','-X','-qAt','-v','ON_ERROR_STOP=1','-U','postgres','-d','omoba_test']
            if socket:cmd+=['-h',socket,'-p','6545']
            refused=execute(primary,*cmd,data='BEGIN; SET LOCAL ROLE '+role+'; '+statement+'; ROLLBACK;',user='postgres',check=False)
            assert refused.returncode!=0 and 'permission denied' in refused.stderr,(role,refused.returncode)
        result['actual_retention_rollback']=True
        result['forbidden_operations_rejected']=4
        return result
    def receipt(n):
        sql(f"""BEGIN;
INSERT INTO career_matches(result_id,server_epoch,match_id,owner_id,lease_until,allocation,checkpoint) VALUES('synthetic-pitr-{n}','1','{n}','fixture',clock_timestamp()+interval '1 hour','{{"rated":true}}','{{}}');
INSERT INTO career_participants(result_id,seat,player_id,profile_id) VALUES('synthetic-pitr-{n}',0,'1',repeat('a',64));
UPDATE career_matches SET status='pending',intent='{{"synthetic":true}}' WHERE result_id='synthetic-pitr-{n}';
UPDATE career_matches SET status='settled',result='{{"synthetic":true}}',settled_at=clock_timestamp() WHERE result_id='synthetic-pitr-{n}';
UPDATE career_profiles SET rating=1000+24*{n},rated_matches=20+{n},matches_played=20+{n},wins=10+{n},completed_public_matches=20+{n} WHERE profile_id=repeat('a',64);
COMMIT;""")
    restores=[]
    def restore(name,target=None,negative=False):
        directory='/tmp/pitr-runtime/native-'+name;socket=directory+'-socket'
        execute(primary,'mkdir','-m','700',directory,socket,user='postgres')
        command=['--pg1-path='+directory,'--type=name','--target='+target] if target else ['--pg1-path='+directory,'--type=default']
        engine(*command,*(['--target-action=promote'] if target else []),'restore',timeout=180)
        # Never reuse source listen, archive, preload, HBA or connection settings.
        cfg=f"listen_addresses=''\nport=6545\nunix_socket_directories='{socket}'\nshared_buffers='16MB'\n{limit_config}archive_mode=off\nssl=off\n"
        execute(primary,'python3','-c',"import pathlib,sys; p=pathlib.Path(sys.argv[1]); (p/'postgresql.conf').write_text(sys.stdin.read()); (p/'pg_hba.conf').write_text('local all all trust\\n')",directory,data=cfg,user='postgres')
        restores.append(directory)
        start=execute(primary,'pg_ctl','-D',directory,'-l',directory+'.log','-w','-t','20','start',user='postgres',check=False,timeout=30)
        deadline=time.monotonic()+25
        while time.monotonic()<deadline:
            log=execute(primary,'cat',directory+'.log').stdout
            status=execute(primary,'pg_ctl','-D',directory,'status',user='postgres',check=False)
            if negative:
                if status.returncode and 'recovery ended before configured recovery target was reached' in log:
                    (out/(name+'.log')).write_text(log);return None
            elif not status.returncode:
                r=execute(primary,'psql','-X','-qAt','-U','postgres','-h',socket,'-p','6545','-d','postgres','-c','SELECT NOT pg_is_in_recovery()',user='postgres',check=False)
                if r.returncode==0 and r.stdout.strip()=='t':
                    assert sql('SELECT system_identifier FROM pg_control_system()',database='postgres',socket=socket)==report['system_identifier']
                    value=snapshot(socket);report.setdefault('restored_rights',{})[name]=rights(socket);(out/(name+'.log')).write_text(log)
                    execute(primary,'pg_ctl','-D',directory,'-m','immediate','stop',user='postgres')
                    return value
            time.sleep(.2)
        (out/(name+'.log')).write_text(log)
        raise AssertionError(f'{name} did not reach the required terminal recovery state; pg_ctl={start.returncode}: '+log[-1600:])
    try:
        report['image_id']=call(['docker','image','inspect',args.image,'--format','{{.Id}}']).stdout.strip()
        call(['docker','network','create',network]);created.append(('network',network))
        call(['docker','run','-d','--init','--name',repository,'--network',network,'--network-alias','repository',
            '--entrypoint','/bin/sleep',args.image,'7200']);created.append(('container',repository))
        call(['docker','run','-d','--init','--name',primary,'--network',network,
            '-e','POSTGRES_HOST_AUTH_METHOD=trust','-e','POSTGRES_INITDB_ARGS=--wal-segsize=1',
            '-e','PGDATA=/tmp/primary-data','-p','127.0.0.1::5432',args.image,'-c','listen_addresses=0.0.0.0',
            '-c','archive_mode=on','-c','archive_timeout=60','-c','shared_buffers=32MB',
            *[arg for name,value in fixture_limits.items() for arg in ('-c',name+'='+str(value))]]);created.append(('container',primary))
        deadline=time.monotonic()+45
        while time.monotonic()<deadline:
            if execute(primary,'pg_isready','-U','postgres',check=False).returncode==0:break
            time.sleep(.25)
        else:raise RuntimeError('Disposable primary did not start')
        report['postgres']=execute(primary,'postgres','--version').stdout.strip()
        report['pgbackrest']=execute(primary,'pgbackrest','version').stdout.strip()
        execute(repository,'python3','-c',"""import pathlib,secrets,subprocess
subprocess.run(['adduser','-D','-h','/srv/backup','backup'],check=True)
subprocess.run(['chpasswd'],input=('backup:'+secrets.token_hex(32)+'\\n').encode(),check=True)
subprocess.run(['ssh-keygen','-A'],check=True,stdout=subprocess.DEVNULL)
p=pathlib.Path('/srv/backup/.ssh');p.mkdir(mode=0o700)
(p/'authorized_keys').touch(mode=0o600)
pathlib.Path('/srv/backup/pitr').mkdir(mode=0o700)
subprocess.run(['chown','-R','backup:backup','/srv/backup'],check=True)
pathlib.Path('/tmp/sshd.conf').write_text('Port 22\\nHostKey /etc/ssh/ssh_host_ed25519_key\\nPasswordAuthentication no\\nPermitRootLogin no\\nPubkeyAuthentication yes\\nAuthorizedKeysFile .ssh/authorized_keys\\nSubsystem sftp internal-sftp\\nAllowUsers backup\\n')
subprocess.run(['/usr/sbin/sshd','-f','/tmp/sshd.conf','-E','/tmp/sshd.log'],check=True)
""")
        execute(primary,'mkdir','-m','700','/tmp/pitr','/tmp/pitr/empty')
        execute(primary,'ssh-keygen','-q','-t','ed25519','-N','','-f','/tmp/pitr/client-key')
        public=execute(primary,'cat','/tmp/pitr/client-key.pub').stdout
        execute(repository,'python3','-c',"import pathlib,sys;pathlib.Path('/srv/backup/.ssh/authorized_keys').write_text(sys.stdin.read())",data=public)
        hostkey=execute(repository,'cat','/etc/ssh/ssh_host_ed25519_key.pub').stdout.split()
        known='repository '+hostkey[0]+' '+hostkey[1]+'\n'
        execute(primary,'python3','-c',"import pathlib,sys;pathlib.Path('/tmp/pitr/known_hosts').write_text(sys.stdin.read())",data=known)
        execute(primary,'python3','-c',"""import pathlib,secrets,os
p=pathlib.Path('/tmp/pitr/pgbackrest.conf')
p.write_text('[global]\\nrepo1-type=sftp\\nrepo1-path=/srv/backup/pitr\\nrepo1-sftp-host=repository\\nrepo1-sftp-host-user=backup\\nrepo1-sftp-private-key-file=/tmp/pitr/client-key\\nrepo1-sftp-public-key-file=/tmp/pitr/client-key.pub\\nrepo1-sftp-host-key-check-type=strict\\nrepo1-sftp-host-key-hash-type=sha256\\nrepo1-sftp-known-host=/tmp/pitr/known_hosts\\nrepo1-cipher-type=aes-256-cbc\\nrepo1-cipher-pass='+secrets.token_hex(32)+'\\nrepo1-retention-full=2\\nstart-fast=y\\nprocess-max=1\\narchive-timeout=15\\ncompress-type=gz\\nlock-path=/tmp/pitr/lock\\nlog-level-file=off\\n[omoba-ru]\\npg1-path=/tmp/primary-data\\npg1-socket-path=/var/run/postgresql\\npg1-user=postgres\\n')
os.chmod(p,0o600)
""")
        execute(primary,'chown','-R','postgres:postgres','/tmp/pitr')
        engine('stanza-create')
        sql("ALTER SYSTEM SET archive_command='pgbackrest --config=/tmp/pitr/pgbackrest.conf --config-include-path=/tmp/pitr/empty --stanza=omoba-ru --log-level-console=warn --log-level-file=off archive-push %p'; SELECT pg_reload_conf();",database='postgres')
        engine('check')
        execute(primary,'mkdir','-m','755','/tmp/pitr-code')
        draft=Path.cwd()/'.agent/codex-ranked-readiness/pitr-parameters/current-tree/ops/vps/ru'
        for name in ['ru_backup.py','ru_pitr.py']:
            call(['docker','cp',str(draft/name),primary+':/tmp/pitr-code/'+name])
        execute(primary,'python3','-c',"""import configparser,grp,json,os,pathlib
p=pathlib.Path('/tmp/pitr/pgbackrest.conf');cfg=configparser.ConfigParser(interpolation=None);cfg.read(p)
cfg['omoba-ru']['pg1-port']='5432'
g=cfg['global'];runtime=pathlib.Path('/tmp/pitr-runtime');runtime.mkdir(mode=0o700)
pg=grp.getgrnam('postgres').gr_gid;os.chown(runtime,pg,pg)
g.update({'repo1-retention-full':'3','archive-async':'n','archive-check':'y','archive-copy':'y','archive-mode-check':'y','repo1-retention-diff':'14','process-max':'1','io-timeout':'15','lock-path':str(runtime/'lock'),'log-path':str(runtime/'log'),'spool-path':str(runtime/'spool')})
with p.open('w') as stream:cfg.write(stream)
os.chown(p,0,pg);os.chmod(p,0o640)
for name in ['/tmp/pitr','/tmp/pitr/empty']:os.chown(name,0,0);os.chmod(name,0o755)
os.chown('/tmp/pitr/known_hosts',0,0);os.chmod('/tmp/pitr/known_hosts',0o644)
values={'OMOBA_BACKUP_DIR':'/tmp/pitr-kit-backups','OMOBA_BACKUP_DATABASE':'omoba_test','OMOBA_BACKUP_RUN_AS':'postgres','OMOBA_BACKUP_ESCROW':'/tmp/pitr','OMOBA_BACKUP_PITR_REQUIRED':'1','OMOBA_BACKUP_PITR_CONFIG':str(p),'OMOBA_BACKUP_PITR_RUNTIME_DIR':str(runtime),'OMOBA_ALERTS_ENV':'/tmp/no-alerts.env'}
pathlib.Path('/tmp/pitr-code/backup.env').write_text(''.join(k+'='+v+'\\n' for k,v in values.items()));os.chmod('/tmp/pitr-code/backup.env',0o600)
""")
        execute(primary,'python3','-c',"""import pathlib,sys,os,shutil
sys.path.insert(0,'/tmp/pitr-code');import ru_backup as rb
keys=rb.run_keygen('/tmp/pitr-code/gpg-keys')
shutil.copyfile('/tmp/pitr/client-key','/tmp/pitr-code/gpg-keys/ssh-key');os.chmod('/tmp/pitr-code/gpg-keys/ssh-key',0o600)
p=pathlib.Path('/tmp/pitr-code/backup.env')
extra={'OMOBA_BACKUP_TARGET':'sftp://backup@repository/srv/backup/logical','OMOBA_BACKUP_GPG_RECIPIENT_FILE':keys['public'],'OMOBA_BACKUP_GPG_IDENTITY_FILE':keys['secret'],'OMOBA_BACKUP_SSH_KEY':'/tmp/pitr-code/gpg-keys/ssh-key','OMOBA_BACKUP_SSH_KNOWN_HOSTS':'/tmp/pitr/known_hosts','OMOBA_BACKUP_STATE_DIRS':'/tmp/pitr-code/app-state','OMOBA_BACKUP_ESCROW':'/tmp/pitr:/tmp/pitr-code/gpg-keys:/tmp/pitr-code/backup.env'}
text=p.read_text();text='\\n'.join(line for line in text.splitlines() if not any(line.startswith(key+'=') for key in extra))+'\\n'
p.write_text(text+''.join(key+'='+value+'\\n' for key,value in extra.items()));os.chmod(p,0o600)
state=pathlib.Path('/tmp/pitr-code/app-state');state.mkdir(mode=0o700);(state/'synthetic.json').write_text('{"synthetic":true}')
""")
        assert kit('config')==[],kit('config')
        archive=kit('command').replace("'","''")
        sql("ALTER SYSTEM SET archive_command='"+archive+"'; SELECT pg_reload_conf();",database='postgres')
        sql('CREATE DATABASE omoba_test',database='postgres')
        sql('CREATE TABLE career_schema_version(version integer primary key)')
        for migration in sorted((root/'career-store/migrations/postgres').glob('[0-9]*.sql')):
            version=int(migration.name.split('_',1)[0]);text=migration.read_text()
            if 'INSERT INTO career_schema_version' not in text:text+=f'\nINSERT INTO career_schema_version VALUES({version});'
            sql('BEGIN;'+text+'\nCOMMIT;')
        for migration in sorted((root/'account-api/migrations').glob('[0-9]*.sql')):
            sql('BEGIN;'+migration.read_text()+'\nCOMMIT;')
        report['schema']=json.loads(sql("SELECT json_build_object('career',(SELECT max(version) FROM career_schema_version),'portal',(SELECT max(version) FROM portal.schema_version))"))
        assert report['schema']=={'career':13,'portal':11},report['schema']
        report['system_identifier']=sql('SELECT system_identifier FROM pg_control_system()',database='postgres')
        sql("INSERT INTO career_profiles(profile_id,nickname,rating,rated_matches,matches_played,wins,losses,progression_xp,completed_public_matches) VALUES(repeat('a',64),'PITR#1234',1000,20,20,10,10,1000,20)")
        sql('CREATE ROLE pitr_game NOLOGIN; CREATE ROLE pitr_portal NOLOGIN; REVOKE CREATE ON SCHEMA public FROM PUBLIC;')
        grants=(root/'account-api/ops/grants.sql').read_text()
        sql('\\set game_role pitr_game\n\\set portal_role pitr_portal\n'+grants)
        sql("""INSERT INTO career_profiles(profile_id,nickname) VALUES(repeat('b',64),'QUEUE#1234');
INSERT INTO career_ready_checks(check_id,owner_id,queue,member_count,state,deadline_ms,live_until_ms)
VALUES(repeat('c',32),'fixture','humans_only',1,'forming',floor(extract(epoch FROM clock_timestamp())*1000)::bigint+3600000,floor(extract(epoch FROM clock_timestamp())*1000)::bigint+3600000),
(repeat('d',32),'fixture','quick',1,'cancelled',0,0);
UPDATE career_ready_checks SET updated_at=clock_timestamp()-interval '8 days' WHERE check_id=repeat('d',32);
INSERT INTO career_ready_members(check_id,profile_id,session_id) VALUES(repeat('c',32),repeat('b',64),'queue-fixture');
INSERT INTO career_ready_claims(profile_id,check_id) VALUES(repeat('b',64),repeat('c',32));""")
        report['source_rights']=rights()
        checks.append('current actual grants restrict runtime roles and execute bounded retention without direct consent DELETE')
        combined=kit('run-backup');physical=combined['pitr'];assert physical['type']=='full';report['kit_backup']=physical
        report['logical_backup']={'name':combined['name'],'schema_versions':combined['schema_versions']}
        checks.append('real ru_backup combines signed encrypted logical SFTP backup with the new physical/WAL pipeline')
        checks.append('actual host-helper config/full-backup/live-check path executes with sanitized environment')
        report['base_snapshot']=snapshot();assert report['base_snapshot']['rating']==1000
        info=json.loads(engine('--output=json','info').stdout);report['backup_labels']=[b['label'] for b in info[0]['backup']]
        assert info[0]['cipher']=='aes-256-cbc' and info[0]['status']['code']==0
        report['repository_cipher']=info[0]['cipher']
        checks.append('actual career13/portal11 physical backup encrypted and stored through strict-host-key SFTP')
        receipt(1)
        sql("UPDATE career_ready_checks SET state='approved',opened_at_ms=deadline_ms-15000 WHERE check_id=repeat('c',32); UPDATE career_ready_members SET accepted=true WHERE check_id=repeat('c',32);")
        target='omoba_fixture_earned';expected_point=snapshot()
        report['target_lsn']=sql(f"SELECT pg_create_restore_point('{target}')",database='postgres')
        receipt(2)
        sql("""UPDATE career_ready_checks SET state='consumed' WHERE check_id=repeat('c',32);
DELETE FROM career_ready_claims WHERE check_id=repeat('c',32);
INSERT INTO career_queue_dodge_events(event_id,profile_id,observed_at_ms) VALUES(repeat('e',32),repeat('b',64),1700000000000);
INSERT INTO career_queue_penalties(profile_id,last_observed_ms,cooldown_until,strikes) VALUES(repeat('b',64),1700000000000,to_timestamp(1700000060),1);""")
        expected_latest=snapshot()
        segment=sql('SELECT pg_walfile_name(pg_current_wal_insert_lsn())',database='postgres');report['post_base_segment']=segment
        engine('check')
        report['point']={'expected':expected_point,'actual':restore('point',target)}
        assert report['point']['actual']==expected_point,report['point'];checks.append('named-point recovery returns exact post-base rating1024/history1')
        report['latest']={'expected':expected_latest,'actual':restore('latest')}
        assert report['latest']['actual']==expected_latest,report['latest'];checks.append('latest archive recovery returns exact post-base rating1048/history2')
        checks.append('named/latest recovery preserve exact ready membership, claims, queue discipline and all13/11 migrations with restored runtime rights')
        combined_drill=kit('run-drill');assert combined_drill['ok'];report['kit_drill']=combined_drill['pitr']
        checks.append('real ru_backup monthly drill verifies the signed logical snapshot and isolated physical target recovery')
        assert report['kit_drill']['ok'] and report['kit_drill']['cleanup_complete']
        assert kit('health')[0]==[],kit('health')
        checks.append('actual host-helper isolated named-target drill proves source system/schema/replay and cleans its scratch cluster')
        # Missing post-base WAL must be a hard restore failure, never old-data success.
        archive_files=execute(repository,'find','/srv/backup/pitr/archive','-type','f').stdout.splitlines()
        matched=[p for p in archive_files if Path(p).name.startswith(segment+'-')]
        assert len(matched)==1,matched
        missing=matched[0];execute(repository,'mv',missing,missing+'.fixture-missing')
        try:restore('missing',target,negative=True)
        finally:execute(repository,'mv',missing+'.fixture-missing',missing)
        checks.append('missing post-base encrypted WAL refuses target recovery instead of returning stale data')
        execute(repository,'cp',missing,missing+'.fixture-original')
        execute(repository,'python3','-c',"import pathlib,sys;p=pathlib.Path(sys.argv[1]);b=bytearray(p.read_bytes());b[len(b)//2]^=1;p.write_bytes(b)",missing)
        try:restore('corrupt',target,negative=True)
        finally:execute(repository,'mv',missing+'.fixture-original',missing)
        checks.append('corrupt encrypted post-base WAL refuses target recovery')
        execute(primary,'python3','-c',"import pathlib;p=pathlib.Path('/tmp/pitr/known_hosts');p.write_text(p.read_text().replace('repository ','not-this-repository ',1))")
        try:
            rejected=engine('--output=json','info',check=False)
            statuses=[]
            if rejected.returncode==0:
                parsed=json.loads(rejected.stdout)
                statuses=[item['status']['code'] for item in parsed]
                assert statuses and any(code!=0 for code in statuses),'Wrong pinned host unexpectedly accepted'
            report['wrong_sftp_pin']={'exit_code':rejected.returncode,'status_codes':statuses,'rejected':True}
        finally:execute(primary,'python3','-c',"import pathlib,sys;pathlib.Path('/tmp/pitr/known_hosts').write_text(sys.stdin.read())",data=known)
        checks.append('wrong SFTP host pin refuses repository access')
        call(['docker','stop','-t','1',repository])
        try:
            failed=engine('check',check=False,timeout=45)
            assert failed.returncode!=0,'An unreachable archive must fail the check'
            kit_failed=kit('check',check=False)
            assert kit_failed.returncode!=0
            cli_failed=kit('cli-check')
            assert cli_failed['exit_code']==1 and cli_failed['metadata_preserved']
            checks.append('actual minute-check CLI alerts in its own journal while preserving logical backup metadata')
            bad_health=kit('health')
            assert 'pitr-archive' in [code for code,_ in bad_health[0]],bad_health
            checks.append('actual host-helper records archive outage and health refuses inherited success')
        finally:
            call(['docker','start',repository]);execute(repository,'/usr/sbin/sshd','-f','/tmp/sshd.conf','-E','/tmp/sshd.log')
        checks.append('unreachable SFTP archive fails health rather than certifying freshness')
        original_label=report['backup_labels'][0]
        for _ in range(3):engine('--type=full','backup',timeout=180)
        retained=json.loads(engine('--output=json','info').stdout)[0]['backup']
        assert len(retained)==3 and all(value['type']=='full' for value in retained)
        assert original_label not in [value['label'] for value in retained]
        report['retained_full_labels']=[value['label'] for value in retained]
        checks.append('actual full-backup retention keeps three completed bases and expires the older fourth only after success')
        report['incomplete_pitr']=kit('run-incomplete')
        assert report['incomplete_pitr']['logical_archive_preserved'] and not report['incomplete_pitr']['combined_success']
        checks.append('real incomplete PITR still uploads independent signed logical backup while refusing combined success')
        kit('run-backup');kit('escrow')
        engine('check');kit('check');assert kit('health')[0]==[]
        cli_recovered=kit('cli-check')
        assert cli_recovered['exit_code']==0 and cli_recovered['metadata_preserved']
        checks.append('actual repaired minute-check CLI resolves its own alert without changing backup metadata')
        root_health=kit('root-health');assert root_health['ok'],root_health['problems']
        report['root_health']={'ok':root_health['ok'],'codes':root_health['codes']}
        checks.append('real complete backup health requires archive/base/drill and actual secrets escrow together')
        checks.append('actual host-helper health recovers only after a successful repaired-archive check')
        # The following game uses the actual native server and pitr_game grants;
        # none of its profiles/results are inserted or repaired by SQL.
        password=secrets.token_hex(32)
        sql("ALTER ROLE pitr_game LOGIN PASSWORD '"+password+"';")
        execute(primary,'python3','-c',"import pathlib;p=pathlib.Path('/tmp/primary-data/pg_hba.conf');p.write_text('host all pitr_game all scram-sha-256\\n'+p.read_text())")
        sql('SELECT pg_reload_conf();',database='postgres')
        source_port=int(call(['docker','port',primary,'5432/tcp']).stdout.strip().split(':')[-1])
        class ProxyHandler(socketserver.BaseRequestHandler):
            def handle(self):
                with proxy.guard:
                    target=proxy.target
                try:
                    upstream=socket.create_connection(target,timeout=5)
                except OSError:
                    return
                pair=(self.request,upstream)
                with proxy.guard:proxy.connections.update(pair)
                try:
                    while True:
                        readable,_,_=select.select(pair,[],[],1)
                        for incoming in readable:
                            data=incoming.recv(65536)
                            if not data:return
                            (pair[1] if incoming is pair[0] else pair[0]).sendall(data)
                except OSError:pass
                finally:
                    with proxy.guard:proxy.connections.difference_update(pair)
                    upstream.close()
        class Proxy(socketserver.ThreadingTCPServer):
            daemon_threads=True
            allow_reuse_address=False
            def __init__(self):
                self.guard=threading.Lock();self.target=('127.0.0.1',source_port);self.connections=set()
                super().__init__(('127.0.0.1',0),ProxyHandler)
            def switch(self,port):
                with self.guard:
                    self.target=('127.0.0.1',port)
                    connections=list(self.connections)
                for connection in connections:
                    try:connection.shutdown(socket.SHUT_RDWR)
                    except OSError:pass
        proxy=Proxy();threading.Thread(target=proxy.serve_forever,daemon=True).start()
        runtime=out/'runtime';control=runtime/'control';control.mkdir(parents=True)
        native_env={k:v for k,v in os.environ.items() if not k.startswith(('OMOBA_','GAME_'))}
        native_env.update(HARNESS_SERVER_BIN=str(native_path),OMOBA_TEST_DATABASE_URL=f'postgres://pitr_game:{quote(password,safe="")}@127.0.0.1:{proxy.server_address[1]}/omoba_test')
        native_log=(out/'native-probe.log').open('w')
        native=subprocess.Popen(['node','scripts/test_public_mvp.mjs','--scenario=recovery','--clients=1','--preference=bot_practice','--seconds=1','--capacity=1','--port='+str(args.lobby_port),'--worker-port='+str(args.worker_port),'--database-restore-control='+str(control),'--output='+str(runtime)],env=native_env,stdout=native_log,stderr=subprocess.STDOUT)
        deadline=time.monotonic()+90
        request=control/'request.json'
        while not request.exists():
            assert native.poll() is None,'Native probe exited before the physical-backup gate; inspect its own log'
            assert time.monotonic()<deadline,'Native game did not reach its backup gate'
            time.sleep(.1)
        req=json.loads(request.read_text());assert re.fullmatch('[0-9a-f]{32}',req['nonce']) and re.fullmatch('[0-9a-f]{32}',req['result_id'])
        match=json.loads(sql("SELECT json_build_object('status',status,'allocation',allocation,'checkpoint',checkpoint) FROM career_matches WHERE result_id='"+req['result_id']+"'"))
        assert match['status']=='running' and isinstance(match['checkpoint'],dict)
        # MatchResult serializes false explicitly and omits true; the canonical
        # serde default is true for existing recovery receipts.
        assert match['checkpoint'].get('gameplay_started',True) is True
        assert match['checkpoint']['result_id']==req['result_id']
        report['native_source_admission']={'result_id':req['result_id'],'status':match['status'],'gameplay_started':True,'runtime_role':'pitr_game','role_auth':'SCRAM over loopback-only fixture proxy'}
        engine('--type=full','backup',timeout=180);engine('check')
        restored=network+'-restored'
        call(['docker','run','-d','--init','--name',restored,'--network',network,'-p','127.0.0.1::5432','--entrypoint','/bin/sleep',args.image,'7200']);created.append(('container',restored))
        # Transfer generated recovery secrets directly between own containers;
        # never write them, a DSN, or a password to a host artifact/log.
        archive=subprocess.run(['docker','exec',primary,'tar','-C','/tmp','-cf','-','pitr'],capture_output=True,check=True).stdout
        subprocess.run(['docker','exec','-i',restored,'tar','-C','/tmp','-xf','-'],input=archive,capture_output=True,check=True)
        del archive
        execute(restored,'mkdir','-m','700','/tmp/recovered-data','/tmp/pitr-runtime')
        execute(restored,'chown','postgres:postgres','/tmp/recovered-data','/tmp/pitr-runtime')
        # A final actual source shutdown ends its WAL stream. Only this owned
        # fixture is stopped; immutable game files and native worker keep running.
        call(['docker','stop','-t','20',primary],timeout=30)
        execute(restored,'pgbackrest','--config=/tmp/pitr/pgbackrest.conf','--config-include-path=/tmp/pitr/empty','--stanza=omoba-ru','--log-level-console=warn','--log-level-file=off','--pg1-path=/tmp/recovered-data','--type=default','restore',user='postgres',timeout=180)
        config=f"listen_addresses='0.0.0.0'\nport=5432\nunix_socket_directories='/var/run/postgresql'\nshared_buffers='32MB'\n{limit_config}archive_mode=off\nssl=off\n"
        execute(restored,'python3','-c',"import pathlib,sys;p=pathlib.Path('/tmp/recovered-data');(p/'postgresql.conf').write_text(sys.stdin.read());(p/'pg_hba.conf').write_text('local all all trust\\nhost all pitr_game all scram-sha-256\\n')",data=config,user='postgres')
        execute(restored,'pg_ctl','-D','/tmp/recovered-data','-l','/tmp/restored-native.log','-w','-t','30','start',user='postgres',timeout=40)
        def restored_sql(query,database='omoba_test'):
            return execute(restored,'psql','-X','-qAt','-v','ON_ERROR_STOP=1','-U','postgres','-d',database,data=query,user='postgres').stdout.strip()
        # pg_ctl accepts a consistent read-only hot standby as ready. Wait for
        # archive recovery to promote before routing writable game traffic.
        promotion_deadline=time.monotonic()+40
        while restored_sql('SELECT NOT pg_is_in_recovery()',database='postgres')!='t':
            assert time.monotonic()<promotion_deadline,'Physical restore did not promote to a writable primary'
            assert native.poll() is None,'Native probe exited while waiting for physical promotion'
            time.sleep(.1)
        report['native_restore_promoted']=True
        assert restored_sql('SELECT system_identifier FROM pg_control_system()',database='postgres')==report['system_identifier']
        assert restored_sql('SELECT max(version) FROM career_schema_version')=='13'
        assert restored_sql("SELECT status FROM career_matches WHERE result_id='"+req['result_id']+"'")=='running'
        restored_port=int(call(['docker','port',restored,'5432/tcp']).stdout.strip().split(':')[-1])
        proxy.switch(restored_port)
        response={'nonce':req['nonce'],'status':'PASS','physical_restored':True,'system_identifier_matched':True,'career_schema':13}
        pending=control/'response.tmp';pending.write_text(json.dumps(response));pending.replace(control/'response.json')
        deadline=time.monotonic()+430
        while native.poll() is None:
            assert time.monotonic()<deadline,'Native recovery exceeded its real-timer budget; preserve diagnostics'
            time.sleep(.2)
        native_log.close();native_log=None
        native_report=json.loads((runtime/'report.json').read_text())
        assert native.returncode==0 and native_report['status']=='PASS','Current worker recovery failed; inspect the native report'
        assert native_report['database_restore']['resumed_running'] and native_report['database_restore']['authenticated_sql_ready']
        assert 'worker_crash_recovers_original_epoch_without_xp_or_elo' in native_report['scenarios']
        saved=json.loads(restored_sql("SELECT json_build_object('status',status,'outcome',result->>'outcome','participants',(SELECT count(*) FROM career_participants p WHERE p.result_id=m.result_id)) FROM career_matches m WHERE result_id='"+req['result_id']+"'"))
        assert saved['status']=='settled' and saved['outcome']=='interrupted'
        report['native_restored_settlement']=saved
        report['native_probe']={'status':'PASS','report':'runtime/report.json','scope':'actual public one-human/nine-bot worker, real SCRAM game role, physical SQL replacement, authenticated Resume, lobby adoption and real-timer crash recovery'}
        (out/'restored-native.log').write_text(execute(restored,'cat','/tmp/restored-native.log').stdout)
        checks.append('actual current public worker keeps its epoch/profile and resumes through authenticated schema13 physical restoration')
        checks.append('actual current worker/lobby recover original interrupted history once after real SIGKILL and unmodified recovery timers')
        report['status']='PASS'
    except BaseException as error:
        report['status']='FAIL';report['error']=str(error)
        diagnostic=call(['docker','logs',primary],check=False)
        (out/'source.log').write_text(diagnostic.stdout+diagnostic.stderr)
        raise
    finally:
        if native is not None and native.poll() is None:
            native.terminate()
            try:native.wait(timeout=15)
            except subprocess.TimeoutExpired:
                native.kill();native.wait(timeout=10)
        if native_log is not None:native_log.close()
        if proxy is not None:
            proxy.switch(1);proxy.shutdown();proxy.server_close()
        for directory in restores:
            execute(primary,'pg_ctl','-D',directory,'-m','immediate','stop',user='postgres',check=False)
        for kind,name in reversed(created):
            command=['docker','rm','-f','-v',name] if kind=='container' else ['docker','network','rm',name]
            call(command,check=False)
        remaining=[]
        for kind,name in created:
            inspected=call(['docker','inspect',name] if kind=='container' else ['docker','network','inspect',name],check=False)
            if inspected.returncode==0:remaining.append(name)
        report['owned_containers_network_removed']=not remaining
        if remaining:report['status']='FAIL';report['cleanup_error']='owned resources remain'
        (out/'summary.json').write_text(json.dumps(report,indent=2)+'\n')
        print(json.dumps({'status':report['status'],'checks':len(checks),'output':str(out)}))
if __name__=='__main__':main()
