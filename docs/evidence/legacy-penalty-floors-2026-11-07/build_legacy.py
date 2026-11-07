from pathlib import Path
import json,os,subprocess,tomllib
root=Path.cwd();out=root/'.agent/codex-ranked-readiness/ready-runtime';cache=Path('/Volumes/Orico/omoba-build-cache/target-b/debug');meta=json.loads((root/'.agent/codex-ranked-readiness/queue-discipline/compile.json').read_text())['commands'][-1];env={k:v for k,v in os.environ.items() if not k.startswith('OMOBA_')};env.update(CARGO_MANIFEST_DIR=str(root/'career-store'),CARGO_PKG_VERSION=tomllib.loads((root/'Cargo.toml').read_text())['workspace']['package']['version']);commands=[]
for kind,target in [('rlib',root/'.agent/codex-ranked-readiness/queue-delivery/libomoba_career_store.rlib'),('test',root/'.agent/codex-ranked-readiness/queue-discipline/store-tests')]:
 cmd=['rustc','--edition=2024','--crate-name','omoba_career_store',str(root/'career-store/src/lib.rs'),'-C','debuginfo=0','-L','dependency='+str(cache/'deps'),'-o',str(target)]+(['--test'] if kind=='test' else ['--crate-type=rlib'])
 for key,value in meta['dependencies'].items():cmd+=['--extern',key+'='+value]
 for p in cache.glob('build/*/out'):
  if p.is_dir():cmd+=['-L','native='+str(p)]
 subprocess.run(cmd,env=env,check=True);commands.append(cmd);print('PASS current store '+kind,flush=True)
(out/'legacy-build-compile.json').write_text(json.dumps({'commands':commands,'scope':'actual current store source, coherent pinned read-only cached graph'},indent=2)+'\n')
for script,args in [('check_server.py',[]),('link_server.py',['tests']),('link_server.py',['server'])]:subprocess.run(['python3.13',str(out/script),*args],check=True)
