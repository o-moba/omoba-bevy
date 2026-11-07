from pathlib import Path
import json,os,subprocess,sys,tomllib
root=Path.cwd();out=root/'.agent/codex-ranked-readiness/ready-runtime';record=json.loads((out/'server-compile.json').read_text())['commands'][-1];cmd=record['command'].copy();cmd.remove('--emit=metadata');cmd[cmd.index('-o')+1]=str(out/('server-tests' if sys.argv[1]=='tests' else 'server'))
if sys.argv[1]!='tests':
 cmd.remove('--test')
 common=json.loads((out/'server-compile.json').read_text())['commands'][0]['command'].copy()
 index=common.index('--cfg');del common[index:index+2]
 common[common.index('-o')+1]=str(out/'libcommon-production.rlib')
 env_common={k:v for k,v in os.environ.items() if not k.startswith('OMOBA_')};env_common.update(CARGO_MANIFEST_DIR=str(root/'common'),CARGO_PKG_VERSION=tomllib.loads((root/'Cargo.toml').read_text())['workspace']['package']['version'])
 subprocess.run(common,env=env_common,check=True)
 cmd=[('common='+str(out/'libcommon-production.rlib')) if x.startswith('common=') else x for x in cmd]

env={k:v for k,v in os.environ.items() if not k.startswith('OMOBA_')};env.update(CARGO_MANIFEST_DIR=str(root/'server'),CARGO_PKG_VERSION=tomllib.loads((root/'Cargo.toml').read_text())['workspace']['package']['version'])
subprocess.run(cmd,env=env,check=True)
print('PASS: actual current headless server '+sys.argv[1]+' linked from existing read-only third-party libraries; no Cargo cache writes',flush=True)
