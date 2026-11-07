from pathlib import Path
import json,os,subprocess,tomllib
root=Path.cwd();out=root/'.agent/codex-ranked-readiness/load-input20';m=tomllib.loads((root/'Cargo.toml').read_text());env={k:v for k,v in os.environ.items() if not k.startswith('OMOBA_')};env.update(CARGO_MANIFEST_DIR=str(root/'harness'),CARGO_PKG_VERSION=m['workspace']['package']['version'],CARGO=str(Path.home()/'.cargo/bin/cargo'))
b=json.loads((out/'build.json').read_text());commands=[x['command'].copy() for x in b['commands']];libtest=[x for x in commands[0] if not x.startswith('--crate-type=')];libtest+=['--test'];libtest[libtest.index('-o')+1]=str(out/'library-tests');commands.append(libtest)
with (out/'library-build.log').open('w') as log:subprocess.run(libtest,env=env,check=True,stdout=log,stderr=subprocess.STDOUT)
with (out/'library-tests.log').open('w') as log:subprocess.run([str(out/'library-tests'),'--test-threads=1'],check=True,stdout=log,stderr=subprocess.STDOUT)
print('PASS whole harness library',flush=True)
results=[]
for i,original in enumerate(commands):
 cmd=original.copy();cmd[0]='clippy-driver';cmd[cmd.index('-o')+1]=str(out/f'clippy-{i}.rmeta');cmd+=['--emit=metadata','-Wclippy::all','-Dwarnings']
 for key,value in m['workspace']['lints']['clippy'].items():cmd+=['-'+{'allow':'A','warn':'W','deny':'D','forbid':'F'}[value]+'clippy::'+key]
 with (out/f'clippy-{i}.log').open('w') as log:r=subprocess.run(cmd,env=env,stdout=log,stderr=subprocess.STDOUT)
 results.append({'command':cmd,'exit':r.returncode});print('strict Clippy',i,r.returncode,flush=True)
 if r.returncode:print((out/f'clippy-{i}.log').read_text());break
(out/'clippy-checks.json').write_text(json.dumps(results,indent=2)+'\n');assert len(results)==4 and all(x['exit']==0 for x in results)
