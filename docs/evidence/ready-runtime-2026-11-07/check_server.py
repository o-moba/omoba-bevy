"""Typecheck actual whole server test target with read-only cached third-party
libraries; rebuild only current common/passport libraries in this owned folder.
No Cargo target writes or third-party/graphics compilation.
"""
from pathlib import Path
import json,os,subprocess,tomllib
root=Path.cwd();out=root/'.agent/codex-ranked-readiness/ready-runtime';cache=Path('/Volumes/Orico/omoba-build-cache/target-b/debug');graph=json.loads((out/'cached-graph.json').read_text());version=tomllib.loads((root/'Cargo.toml').read_text())['workspace']['package']['version']
env={k:v for k,v in os.environ.items() if not k.startswith('OMOBA_')};env['CARGO_PKG_VERSION']=version
base=['rustc','--edition=2024','-C','debuginfo=0','-L','dependency='+str(cache/'deps'),'-L','dependency='+str(out),'-L','dependency='+str(root/'.agent/codex-ranked-readiness/queue-discipline'),'-L','dependency='+str(root/'.agent/codex-ranked-readiness/queue-delivery')]
for path in cache.glob('build/*/out'):
 if path.is_dir():base+=['-L','native='+str(path)]
commands=[]
for name,folder in [('common','common'),('omoba_passport','passport'),('server','server')]:
 deps=graph[name]['dependencies'].copy();deps['shared']=str(root/'.agent/codex-ranked-readiness/queue-discipline/libshared.rlib')
 if name=='server':
  deps.update(common=str(out/'libcommon.rlib'),omoba_passport=str(out/'libomoba_passport.rlib'),omoba_career_store=str(root/'.agent/codex-ranked-readiness/queue-delivery/libomoba_career_store.rlib'))
 source=root/folder/'src'/('main.rs' if name=='server' else 'lib.rs');env['CARGO_MANIFEST_DIR']=str(root/folder)
 cmd=base+['--crate-name',name,str(source)]
 if name=='server':cmd+=['--test','--emit=metadata','-o',str(out/'server-check.rmeta')]
 else:
  cmd+=['--crate-type','rlib','-o',str(out/f'lib{name}.rlib')]
  if name=='common':cmd+=['--cfg','feature="test-support"']
 for dep,path in deps.items():cmd+=['--extern',dep+'='+path]
 commands.append({'name':name,'source':str(source),'dependencies':deps,'command':cmd})
 subprocess.run(cmd,env=env,check=True)
 print('PASS: actual current '+name+' source '+('test target metadata' if name=='server' else 'library'),flush=True)
(out/'server-compile.json').write_text(json.dumps({'scope':'Actual whole current server test target; current common/passport/shared/store, read-only coherent cached third-party graph; metadata only, no test execution or graphics compilation','commands':commands},indent=2)+'\n')
