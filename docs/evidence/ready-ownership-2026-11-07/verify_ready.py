"""Small renderer-free verification; cached third-party libraries are read only.
Compile actual current shared/store source into owned artifacts, never a Cargo cache.
"""
from pathlib import Path
import hashlib,json,os,subprocess,tomllib
ROOT=Path.cwd();OUT=ROOT/'.agent/codex-ranked-readiness/queue-discipline'
CACHE=Path('/Volumes/Orico/omoba-build-cache/target-b/debug');FP=CACHE/'.fingerprint'
metadata=FP/'omoba-career-store-37a0b9f09a4c254c/test-lib-omoba_career_store.json'

def resolve(name,number):
 wanted=f'{number:016x}'
 matches=[]
 for p in FP.glob('*/lib-'+name):
  raw=p.read_text().strip()
  if raw==wanted or bytes.fromhex(raw)[::-1].hex()==wanted:
   suffix=p.parent.name.rsplit('-',1)[1]
   lib=CACHE/'deps'/f'lib{name}-{suffix}.rlib'
   if lib.is_file():matches.append(lib)
 assert len(matches)==1,(name,len(matches))
 return matches[0]

def deps(p):return {name:resolve(name,number) for _,name,_,number in json.loads(p.read_text())['deps']}
store=deps(metadata);shared=deps(FP/'shared-1fea36d842216582/lib-shared.json')
assert store['serde_json']==shared['serde_json']
choices=[]
for p in FP.glob('flate2-*/lib-flate2.json'):
 suffix=p.parent.name.rsplit('-',1)[1];lib=CACHE/'deps'/f'libflate2-{suffix}.rlib';dep=CACHE/'deps'/f'flate2-{suffix}.d'
 if lib.is_file() and dep.is_file() and '/flate2-1.1.0/' in dep.read_text() and 'rust_backend' in p.read_text():choices.append((p.stat().st_mtime,lib))
assert choices,'No approved cached flate2 1.1.0 Rust backend library'
shared['flate2']=max(choices)[1]
version=tomllib.loads((ROOT/'Cargo.toml').read_text())['workspace']['package']['version']
env={k:v for k,v in os.environ.items() if not k.startswith('OMOBA_')}
env.update(CARGO_PKG_VERSION=version,CARGO_MANIFEST_DIR=str(ROOT/'shared'))

commands=[]
def compile_source(source,name,dependencies,output,test=False):
 cmd=['rustc','--edition=2024','--crate-name',name,str(ROOT/source),'-C','debuginfo=0','-C','opt-level=0','-L','dependency='+str(CACHE/'deps'),'-o',str(output)]
 cmd+=['--test'] if test else ['--crate-type','rlib']
 for key,value in dependencies.items():cmd+=['--extern',key+'='+str(value)]
 # Needed only by libraries that did not bundle their native archive in the rlib.
 for path in CACHE.glob('build/*/out'):
  if path.is_dir():cmd+=['-L','native='+str(path)]
 commands.append({'crate':name,'source':source,'dependencies':{k:str(v) for k,v in dependencies.items()},'output':str(output)})
 subprocess.run(cmd,env=env,check=True)

compile_source('shared/src/lib.rs','shared',shared,OUT/'libshared.rlib')
store['shared']=OUT/'libshared.rlib'
env['CARGO_MANIFEST_DIR']=str(ROOT/'career-store')
compile_source('career-store/src/lib.rs','omoba_career_store',store,OUT/'store-tests',True)
(OUT/'compile.json').write_text(json.dumps({'status':'PASS','scope':'Actual current renderer-free shared/store source; coherent cached third-party dependencies; no Cargo/Bevy build or cache mutation','rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'commands':commands},indent=2)+'\n')
print('PASS: actual current shared/store tests compiled with read-only cached dependencies',flush=True)
# Only the pre-existing owned local fixture; credentials never printed or passed in argv.
data=json.loads(subprocess.check_output(['docker','inspect','omoba-mm-seasons-pg'],text=True))[0]
config=dict(value.split('=',1) for value in data['Config']['Env'] if '=' in value)
from urllib.parse import quote
user=config.get('POSTGRES_USER','postgres');password=config.get('POSTGRES_PASSWORD','');database=config.get('POSTGRES_DB',user)
ports=data['NetworkSettings']['Ports']['5432/tcp'];assert any(p['HostPort']=='55582' for p in ports)
env['OMOBA_TEST_DATABASE_URL']=f'postgres://{quote(user,safe="")}:{quote(password,safe="")}@127.0.0.1:55582/{quote(database,safe="")}'
subprocess.run([str(OUT/'store-tests'),'ready_ownership','--ignored','--nocapture'],env=env,check=True)
print('PASS: owned-schema PostgreSQL ready ownership API regressions',flush=True)
