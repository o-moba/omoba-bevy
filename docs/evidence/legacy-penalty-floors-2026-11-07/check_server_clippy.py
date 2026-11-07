from pathlib import Path
import json,os,subprocess,tomllib
root=Path.cwd();out=root/'.agent/codex-ranked-readiness/ready-runtime';m=tomllib.loads((root/'Cargo.toml').read_text());env={k:v for k,v in os.environ.items() if not k.startswith('OMOBA_')};env.update(CARGO_MANIFEST_DIR=str(root/'server'),CARGO_PKG_VERSION=m['workspace']['package']['version']);base=json.loads((out/'server-compile.json').read_text())['commands'][-1]['command']
for kind in ['tests','production']:
 cmd=base.copy();cmd[0]='clippy-driver';cmd[cmd.index('-o')+1]=str(out/f'legacy-clippy-{kind}.rmeta')
 if kind=='production':
  cmd.remove('--test');cmd=[('common='+str(out/'libcommon-production.rlib')) if x=='common='+str(out/'libcommon.rlib') else x for x in cmd]
 cmd+=['-Wclippy::all','-Dwarnings']
 for key,value in m['workspace']['lints']['clippy'].items():cmd+=['-'+{'allow':'A','warn':'W','deny':'D','forbid':'F'}[value]+'clippy::'+key]
 subprocess.run(cmd,env=env,check=True);print('PASS strict full-server '+kind,flush=True)
