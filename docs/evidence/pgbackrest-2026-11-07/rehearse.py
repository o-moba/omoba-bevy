#!/usr/bin/env python3
"""Disposable two-container PG18 + encrypted pgBackRest/SFTP proof. Never deploys.
All ratings/history below are explicit direct-SQL synthetic fixtures, not games.
"""
from __future__ import annotations
import argparse, hashlib, json, re, secrets, shutil, subprocess, time
from pathlib import Path


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',required=True,type=Path)
    parser.add_argument('--image',default='omoba-mm-pitr-rehearsal:20261107')
    args=parser.parse_args();out=args.output.resolve()
    assert not out.exists(),'Use a fresh output directory'
    assert shutil.disk_usage('/').free>8*1024**3,'Need8GiB for this small fixture'
    out.mkdir(parents=True)
    root=Path.cwd();run_id=secrets.token_hex(5)
    network='omoba-mm-pitr-'+run_id;primary=network+'-primary';repository=network+'-repo'
    created=[];checks=[];report={'status':'RUNNING','run_id':run_id,'checks':checks,
        'scope':'Disposable two-container encrypted SFTP/PG18 recovery only; not real off-host/RU deployment, production RPO, physical UI or played-game evidence',
        'source_parent':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),
        'fixture':'Direct SQL synthetic rating/history receipts with actual career/portal migrations',
        'wal_segment_mib':1}
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
    def snapshot(socket=None):
        return json.loads(sql("SELECT json_build_object('rating',rating,'rated_matches',rated_matches,'matches_played',matches_played,'history',(SELECT count(*) FROM career_participants WHERE profile_id=repeat('a',64))) FROM career_profiles WHERE profile_id=repeat('a',64)",socket=socket))
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
        directory='/tmp/pitr/'+name;socket=directory+'-socket'
        execute(primary,'mkdir','-m','700',directory,socket,user='postgres')
        command=['--pg1-path='+directory,'--type=name','--target='+target] if target else ['--pg1-path='+directory,'--type=default']
        engine(*command,*(['--target-action=promote'] if target else []),'restore',timeout=180)
        # Never reuse source listen, archive, preload, HBA or connection settings.
        cfg=f"listen_addresses=''\nport=6545\nunix_socket_directories='{socket}'\nshared_buffers='16MB'\nmax_connections=20\narchive_mode=off\nssl=off\n"
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
                    value=snapshot(socket);(out/(name+'.log')).write_text(log)
                    execute(primary,'pg_ctl','-D',directory,'-m','immediate','stop',user='postgres')
                    return value
            time.sleep(.2)
        (out/(name+'.log')).write_text(log)
        raise AssertionError(f'{name} did not reach the required terminal recovery state; pg_ctl={start.returncode}: '+log[-1600:])
    try:
        report['image_id']=call(['docker','image','inspect',args.image,'--format','{{.Id}}']).stdout.strip()
        call(['docker','network','create','--internal',network]);created.append(('network',network))
        call(['docker','run','-d','--name',repository,'--network',network,'--network-alias','repository',
            '--entrypoint','/bin/sleep',args.image,'7200']);created.append(('container',repository))
        call(['docker','run','-d','--name',primary,'--network',network,
            '-e','POSTGRES_HOST_AUTH_METHOD=trust','-e','POSTGRES_INITDB_ARGS=--wal-segsize=1',
            '-e','PGDATA=/tmp/primary-data',args.image,'-c','listen_addresses=127.0.0.1',
            '-c','archive_mode=on','-c','archive_timeout=60','-c','shared_buffers=32MB',
            '-c','max_connections=20']);created.append(('container',primary))
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
        sql('CREATE DATABASE omoba_test',database='postgres')
        sql('CREATE TABLE career_schema_version(version integer primary key)')
        for migration in sorted((root/'career-store/migrations/postgres').glob('[0-9]*.sql')):
            version=int(migration.name.split('_',1)[0]);text=migration.read_text()
            if 'INSERT INTO career_schema_version' not in text:text+=f'\nINSERT INTO career_schema_version VALUES({version});'
            sql('BEGIN;'+text+'\nCOMMIT;')
        for migration in sorted((root/'account-api/migrations').glob('[0-9]*.sql')):
            sql('BEGIN;'+migration.read_text()+'\nCOMMIT;')
        report['schema']=json.loads(sql("SELECT json_build_object('career',(SELECT max(version) FROM career_schema_version),'portal',(SELECT max(version) FROM portal.schema_version))"))
        assert report['schema']=={'career':10,'portal':11},report['schema']
        report['system_identifier']=sql('SELECT system_identifier FROM pg_control_system()',database='postgres')
        sql("INSERT INTO career_profiles(profile_id,nickname,rating,rated_matches,matches_played,wins,losses,progression_xp,completed_public_matches) VALUES(repeat('a',64),'PITR#1234',1000,20,20,10,10,1000,20)")
        engine('--type=full','backup',timeout=180)
        report['base_snapshot']=snapshot();assert report['base_snapshot']['rating']==1000
        info=json.loads(engine('--output=json','info').stdout);report['backup_labels']=[b['label'] for b in info[0]['backup']]
        assert info[0]['cipher']=='aes-256-cbc' and info[0]['status']['code']==0
        report['repository_cipher']=info[0]['cipher']
        checks.append('actual career10/portal11 physical backup encrypted and stored through strict-host-key SFTP')
        receipt(1);target='omoba_fixture_earned';expected_point=snapshot()
        report['target_lsn']=sql(f"SELECT pg_create_restore_point('{target}')",database='postgres')
        receipt(2);expected_latest=snapshot()
        segment=sql('SELECT pg_walfile_name(pg_current_wal_insert_lsn())',database='postgres');report['post_base_segment']=segment
        engine('check')
        report['point']={'expected':expected_point,'actual':restore('point',target)}
        assert report['point']['actual']==expected_point,report['point'];checks.append('named-point recovery returns exact post-base rating1024/history1')
        report['latest']={'expected':expected_latest,'actual':restore('latest')}
        assert report['latest']['actual']==expected_latest,report['latest'];checks.append('latest archive recovery returns exact post-base rating1048/history2')
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
        finally:
            call(['docker','start',repository]);execute(repository,'/usr/sbin/sshd','-f','/tmp/sshd.conf','-E','/tmp/sshd.log')
        checks.append('unreachable SFTP archive fails health rather than certifying freshness')
        engine('check');report['status']='PASS'
    except BaseException as error:
        report['status']='FAIL';report['error']=str(error)
        raise
    finally:
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
