"""Compile actual renderer-free backend/port/admission/journal and real store.
No Cargo cache writes or Bevy build; only the transport capacity constant is
copied verbatim from its engine-coupled module into the standalone crate root.
"""
from pathlib import Path
import hashlib,json,os,re,subprocess,tomllib
root=Path.cwd();out=root/'.agent/codex-ranked-readiness/queue-delivery';cache=Path('/Volumes/Orico/omoba-build-cache/target-b/debug');fp=cache/'.fingerprint'
previous=json.loads((root/'.agent/codex-ranked-readiness/queue-discipline/compile.json').read_text())
storedeps=previous['commands'][-1]['dependencies']
manifest=tomllib.loads((root/'Cargo.toml').read_text())
env={k:v for k,v in os.environ.items() if not k.startswith('OMOBA_')};env.update(CARGO_PKG_VERSION=manifest['workspace']['package']['version'],CARGO_MANIFEST_DIR=str(root/'career-store'))
def run_compile(cmd):
 for path in cache.glob('build/*/out'):
  if path.is_dir():cmd+=['-L','native='+str(path)]
 subprocess.run(cmd,env=env,check=True)
base=['rustc','--edition=2024','-C','debuginfo=0','-L','dependency='+str(cache/'deps')]
cmd=base+['--crate-name','omoba_career_store','--crate-type','rlib',str(root/'career-store/src/lib.rs'),'-o',str(out/'libomoba_career_store.rlib')]
for name,path in storedeps.items():cmd+=['--extern',name+'='+path]
run_compile(cmd)
source=(root/'server/src/public_transport.rs').read_text();constant=re.findall(r'^pub\(crate\) const MAX_ENDPOINTS: usize = \d+;$',source,re.M);assert len(constant)==1
wrapper=out/'backend_tests.rs';text='#![allow(dead_code)]\nmod public_transport { '+constant[0]+' }\n'
for module in ['admission','career_port','career_backend']:text+='#[path = '+json.dumps(str(root/'server/src'/f'{module}.rs'))+']\nmod '+module+';\n'
wrapper.write_text(text)
selected=fp/'server-efdea35d96196698/test-bin-server.json';dependencies={}
for _,name,_,number in json.loads(selected.read_text())['deps']:
 if name not in ('tokio','ed25519_dalek','serde','serde_json','getrandom','sqlx'):continue
 wanted=f'{number:016x}';matches=[]
 for p in fp.glob('*/lib-'+name):
  raw=p.read_text().strip()
  if raw==wanted or bytes.fromhex(raw)[::-1].hex()==wanted:
   suffix=p.parent.name.rsplit('-',1)[1];lib=cache/'deps'/f'lib{name}-{suffix}.rlib'
   if lib.is_file():matches.append(lib)
 assert len(matches)==1,(name,len(matches));dependencies[name]=str(matches[0])
dependencies['shared']=storedeps['shared'];dependencies['omoba_career_store']=str(out/'libomoba_career_store.rlib')
env['CARGO_MANIFEST_DIR']=str(root/'server')
cmd=base+['--crate-name','omoba_backend_proof','--test',str(wrapper),'-o',str(out/'backend-tests')]
for name,path in dependencies.items():cmd+=['--extern',name+'='+path]
run_compile(cmd)
(out/'backend-compile.json').write_text(json.dumps({'status':'PASS','scope':'Actual current backend, port, admission, queue journal and store; engine-coupled public transport contributes only its verbatim capacity constant; no MatchService/engine integration certification','constant':constant[0],'dependencies':dependencies,'wrapper_sha256':hashlib.sha256(wrapper.read_bytes()).hexdigest(),'rustc':subprocess.check_output(['rustc','--version'],text=True).strip()},indent=2)+'\n')
print('PASS: current actual native backend/store source compiled',flush=True)
# Run the complete non-DB unit set first.
subprocess.run([str(out/'backend-tests'),'--test-threads=2'],env=env,check=True)
# Then the new actual worker/DB failure paths in the task-owned local fixture.
from urllib.parse import quote
container=json.loads(subprocess.check_output(['docker','inspect','omoba-mm-seasons-pg'],text=True))[0]
config=dict(x.split('=',1) for x in container['Config']['Env'] if '=' in x)
user=config.get('POSTGRES_USER','postgres');password=config.get('POSTGRES_PASSWORD','');database=config.get('POSTGRES_DB',user)
env['OMOBA_TEST_DATABASE_URL']=f'postgres://{quote(user,safe="")}:{quote(password,safe="")}@127.0.0.1:55582/{quote(database,safe="")}'
subprocess.run([str(out/'backend-tests'),'queue_dodge_','--ignored','--test-threads=1','--nocapture'],env=env,check=True)
