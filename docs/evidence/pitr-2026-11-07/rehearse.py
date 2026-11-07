#!/usr/bin/env python3
"""Disposable PostgreSQL18 WAL/PITR mechanism proof; never a live-host backup.
Requires preinstalled PG18 tools and a prebuilt current Account API migrator.
All data/ports are newly owned; output contains no keys or production records.
"""
from __future__ import annotations
import argparse, hashlib, json, os, re, shlex, shutil, socket, subprocess, sys, tempfile, time
from pathlib import Path


def digest(path):
    value=hashlib.sha256()
    with Path(path).open('rb') as stream:
        for chunk in iter(lambda:stream.read(1024*1024),b''):value.update(chunk)
    return value.hexdigest()


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pg-bin',required=True,type=Path)
    parser.add_argument('--account-api-bin',required=True,type=Path)
    parser.add_argument('--output',required=True,type=Path)
    args=parser.parse_args()
    out=args.output.resolve();assert not out.exists(),'Use a fresh output directory'
    out.mkdir(parents=True)
    root=Path.cwd().resolve();assert (root/'career-store/migrations/postgres/010_ranked_seasons.sql').is_file()
    assert shutil.disk_usage('/').free>8*2**30,'Need8GiB free before this small disposable fixture'
    pg=args.pg_bin.resolve();api=args.account_api_bin.resolve();assert api.is_file()
    env={k:v for k,v in os.environ.items() if not k.startswith(('PG','OMOBA_'))}
    env.update(PGCONNECT_TIMEOUT='5',PGPASSFILE=str(out/'empty-pgpass'),NO_PROXY='127.0.0.1,localhost,::1',no_proxy='127.0.0.1,localhost,::1')
    (out/'empty-pgpass').write_text('');os.chmod(out/'empty-pgpass',0o600)
    scratch=Path(tempfile.mkdtemp(prefix='owned-',dir=out));clusters=[];checks=[]
    report={'status':'RUNNING','scope':'Disposable PG18 native loopback physical-base/WAL proof only; no production configuration, off-host encryption/storage, RPO or game-completion proof.',
            'source_parent':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),
            'migrator_sha256':digest(api),'wal_segment_mib':1,'checks':checks,'minimum_root_gib':shutil.disk_usage('/').free/2**30}
    def guard():
        report['minimum_root_gib']=min(report['minimum_root_gib'],shutil.disk_usage('/').free/2**30)
        assert report['minimum_root_gib']>=8,'Disk guard stopped disposable proof'
    def run(command,*,data=None,timeout=90,check=True,child_env=None,guard_disk=True):
        if guard_disk:guard()
        r=subprocess.run([str(x) for x in command],input=data,text=True,capture_output=True,timeout=timeout,env=child_env or env)
        if check and r.returncode:raise RuntimeError(f'{Path(str(command[0])).name} failed({r.returncode}): {(r.stdout+r.stderr)[-1800:]}')
        return r
    def free_port():
        with socket.socket() as s:s.bind(('127.0.0.1',0));return s.getsockname()[1]
    def sql(port,statement,database='omoba_test'):
        return run([pg/'psql','-X','-qAt','--no-password','-v','ON_ERROR_STOP=1','-h','127.0.0.1','-p',port,'-U','omoba_fixture','-d',database],data=statement).stdout.strip()
    def conf_string(text):return "'"+str(text).replace("'","''")+"'"
    def config(directory,port,archive_command=None,restore_command=None,target=None):
        text=f"listen_addresses='127.0.0.1'\nport={port}\nunix_socket_directories=''\nshared_buffers='16MB'\nmax_connections=20\nmax_wal_senders=3\nmin_wal_size='2MB'\nmax_wal_size='16MB'\nfsync=on\nsynchronous_commit=on\nfull_page_writes=on\n"
        text+="archive_mode="+('on' if archive_command else 'off')+'\n'
        if archive_command:text+='archive_command='+conf_string(archive_command)+'\n'
        if restore_command:text+='restore_command='+conf_string(restore_command)+"\nrecovery_target_timeline='current'\n"
        if target:text+='recovery_target_name='+conf_string(target)+"\nrecovery_target_action='promote'\n"
        (directory/'postgresql.conf').write_text(text)
        (directory/'postgresql.auto.conf').write_text('')
    def start(directory,port,*,negative=False):
        clusters.append(directory)
        r=run([pg/'pg_ctl','-D',directory,'-l',out/(directory.name+'.log'),'-w','-t','30','start'],timeout=40,check=not negative)
        if negative:
            # pg_ctl can acknowledge hot-standby readiness before recovery
            # reaches its target. Wait for the actual terminal unmet-target
            # failure; an accepting/promoted old snapshot is never success.
            deadline=time.monotonic()+15
            while time.monotonic()<deadline:
                log=(out/(directory.name+'.log')).read_text()
                status=run([pg/'pg_ctl','-D',directory,'status'],check=False)
                if status.returncode!=0 and 'recovery ended before configured recovery target was reached' in log:break
                time.sleep(.1)
            else:raise AssertionError('Missing-WAL restore did not terminate with an unmet-target failure: '+log[-1800:])
        else:
            deadline=time.monotonic()+15
            while time.monotonic()<deadline:
                if sql(port,'SELECT NOT pg_is_in_recovery()',database='postgres')=='t':break
                time.sleep(.1)
            else:raise AssertionError('Recovery did not reach a writable promoted state')
    def snapshot(port):
        return json.loads(sql(port,"SELECT json_build_object('rating',rating,'rated_matches',rated_matches,'matches_played',matches_played,'history',(SELECT count(*) FROM career_participants WHERE profile_id=repeat('a',64))) FROM career_profiles WHERE profile_id=repeat('a',64)"))
    try:
        version=run([pg/'postgres','--version']).stdout.strip();assert re.search(r'\b18\.',version),version;report['postgres']=version
        source=scratch/'source';archive=scratch/'archive';archive.mkdir()
        callback=out/'wal-copy.py'
        callback.write_text('''import hashlib,os,pathlib,re,sys,uuid
mode,archive,name,path=sys.argv[1:];archive=pathlib.Path(archive);path=pathlib.Path(path)
if not re.fullmatch(r"[0-9A-F]{24}(?:\\.[0-9A-F]{8}\\.backup|\\.partial)?|[0-9A-F]{8}\\.history",name):raise SystemExit(2)
source,destination=(path,archive/name) if mode=="archive" else (archive/name,path)
if not source.is_file():raise SystemExit(1)
data=source.read_bytes()
if mode=="archive" and destination.exists():raise SystemExit(0 if destination.read_bytes()==data else 3)
temporary=destination.with_name(destination.name+".pending-"+uuid.uuid4().hex)
with temporary.open("xb") as stream:stream.write(data);stream.flush();os.fsync(stream.fileno())
try:
 if mode=="archive":os.link(temporary,destination);temporary.unlink()
 else:os.replace(temporary,destination)
finally:
 if temporary.exists():temporary.unlink()
fd=os.open(destination.parent,os.O_RDONLY);os.fsync(fd);os.close(fd)
''')
        def command(mode,directory):return ' '.join([shlex.quote(sys.executable),shlex.quote(str(callback)),mode,shlex.quote(str(directory)),"'%f'","'%p'"])
        run([pg/'initdb','-D',source,'--username=omoba_fixture','--no-locale','--encoding=UTF8','--auth-local=trust','--auth-host=trust','--wal-segsize=1'])
        port=free_port();config(source,port,archive_command=command('archive',archive));start(source,port)
        system_id=sql(port,'SELECT system_identifier FROM pg_control_system()',database='postgres');report['system_identifier']=system_id
        sql(port,'CREATE DATABASE omoba_test',database='postgres')
        migrate_env={**env,'OMOBA_DATABASE_URL':f'postgres://omoba_fixture@127.0.0.1:{port}/omoba_test'}
        run([api,'migrate'],child_env=migrate_env)
        schema=sql(port,"SELECT json_build_object('career',(SELECT max(version) FROM career_schema_version),'portal',(SELECT max(version) FROM portal.schema_version))")
        report['schema']=json.loads(schema);assert report['schema']=={'career':10,'portal':11},schema
        sql(port,"INSERT INTO career_profiles(profile_id,nickname,rating,rated_matches,matches_played,wins,losses,progression_xp,completed_public_matches) VALUES(repeat('a',64),'PITR#1234',1000,20,20,10,10,1000,20)")
        base=scratch/'base';run([pg/'pg_basebackup','-h','127.0.0.1','-p',port,'-U','omoba_fixture','--no-password','-D',base,'--wal-method=stream','--checkpoint=fast'])
        run([pg/'pg_verifybackup','--exit-on-error',base]);report['base_manifest_sha256']=digest(base/'backup_manifest')
        report['base_snapshot']=snapshot(port);assert report['base_snapshot']['rating']==1000
        checks.append('actual schema10/portal11 physical base backup verified')
        sql(port,'SELECT pg_switch_wal()')
        def receipt(number):
            # Explicit direct-SQL fixtures preserve real append-only lifecycle
            # transitions. These JSON payloads are not played-game receipts.
            sql(port,f"""BEGIN;
INSERT INTO career_matches(result_id,server_epoch,match_id,owner_id,lease_until,allocation,checkpoint) VALUES('synthetic-pitr-{number}','1','{number}','fixture',clock_timestamp()+interval '1 hour','{{"rated":true}}','{{}}');
INSERT INTO career_participants(result_id,seat,player_id,profile_id) VALUES('synthetic-pitr-{number}',0,'1',repeat('a',64));
UPDATE career_matches SET status='pending',intent='{{"synthetic":true}}' WHERE result_id='synthetic-pitr-{number}';
UPDATE career_matches SET status='settled',result='{{"synthetic":true}}',settled_at=clock_timestamp() WHERE result_id='synthetic-pitr-{number}';
UPDATE career_profiles SET rating=1000+24*{number},rated_matches=20+{number},matches_played=20+{number},wins=10+{number},completed_public_matches=20+{number} WHERE profile_id=repeat('a',64);
COMMIT;""")
        receipt(1);target_snapshot=snapshot(port);target='omoba_fixture_earned'
        target_lsn=sql(port,f"SELECT pg_create_restore_point('{target}')");report['target_lsn']=target_lsn
        receipt(2);latest_snapshot=snapshot(port)
        segment=sql(port,'SELECT pg_walfile_name(pg_current_wal_insert_lsn())');sql(port,'SELECT pg_switch_wal()')
        assert not (base/'pg_wal'/segment).exists(),'Must prove recovery from archived post-base WAL, not backup-contained WAL'
        deadline=time.monotonic()+20
        while not (archive/segment).exists() and time.monotonic()<deadline:guard();time.sleep(.1)
        assert (archive/segment).is_file(),'Post-base commit segment was not archived'
        assert sql(port,f"SELECT count(*) FROM pg_stat_archiver WHERE last_archived_wal >= '{segment}' AND failed_count=0",database='postgres')=='1'
        report['post_base_segment']={'name':segment,'bytes':(archive/segment).stat().st_size,'sha256':digest(archive/segment)}
        checks.append('two post-base rating/history fixtures committed and their WAL durably archived')
        for name,target_name,expected in [('point',target,target_snapshot),('latest',None,latest_snapshot)]:
            restored=scratch/name;shutil.copytree(base,restored);recovered_port=free_port()
            config(restored,recovered_port,restore_command=command('restore',archive),target=target_name);(restored/'recovery.signal').touch()
            start(restored,recovered_port)
            assert sql(recovered_port,'SELECT system_identifier FROM pg_control_system()',database='postgres')==system_id
            actual=snapshot(recovered_port);assert actual==expected,(name,actual,expected)
            report[name]={'expected':expected,'actual':actual,'recovered_without_primary_connection':not (restored/'standby.signal').exists()}
            checks.append(name+' recovery replays real career rows to the exact expected rating/history state')
        missing=scratch/'missing-archive';missing.mkdir()
        for entry in archive.iterdir():
            if entry.name!=segment:os.link(entry,missing/entry.name)
        negative=scratch/'missing';shutil.copytree(base,negative);negative_port=free_port()
        config(negative,negative_port,restore_command=command('restore',missing),target=target);(negative/'recovery.signal').touch()
        start(negative,negative_port,negative=True)
        checks.append('missing post-base WAL refuses target recovery instead of reporting an old snapshot as success')
        report['status']='PASS'
    except BaseException as error:
        report['status']='FAIL';report['error']=str(error);raise
    finally:
        stopped=True
        for directory in reversed(clusters):
            status=run([pg/'pg_ctl','-D',directory,'status'],check=False,guard_disk=False)
            if status.returncode==0:
                result=run([pg/'pg_ctl','-D',directory,'-m','fast','-w','-t','15','stop'],check=False,timeout=20,guard_disk=False)
                if result.returncode:result=run([pg/'pg_ctl','-D',directory,'-m','immediate','-w','-t','10','stop'],check=False,timeout=15,guard_disk=False)
                if result.returncode:stopped=False
        report['all_owned_postgres_stopped']=stopped
        if stopped:shutil.rmtree(scratch);report['temporary_cluster_data_removed']=True
        else:report['retained_owned_data']=str(scratch)
        report['rehearsal_sha256']=digest(__file__);report['wal_callback_sha256']=digest(out/'wal-copy.py') if (out/'wal-copy.py').exists() else None
        (out/'report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
        print(json.dumps({'status':report['status'],'checks':checks,'cleanup':stopped,'output':str(out)},ensure_ascii=False),flush=True)

if __name__=='__main__':main()
