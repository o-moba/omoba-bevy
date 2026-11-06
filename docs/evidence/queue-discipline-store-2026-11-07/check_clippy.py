from pathlib import Path
import json,os,subprocess,tomllib
root=Path.cwd();out=root/'.agent/codex-ranked-readiness/queue-discipline';meta=json.loads((out/'compile.json').read_text())['commands'][-1]
manifest=tomllib.loads((root/'Cargo.toml').read_text())
env={k:v for k,v in os.environ.items() if not k.startswith('OMOBA_')};env.update(CARGO_MANIFEST_DIR=str(root/'career-store'),CARGO_PKG_VERSION=manifest['workspace']['package']['version'])
cmd=['clippy-driver','--edition=2024','--crate-name','omoba_career_store',str(root/'career-store/src/lib.rs'),'--test','--emit=metadata','-o',str(out/'store-check.rmeta'),'-L','dependency=/Volumes/Orico/omoba-build-cache/target-b/debug/deps','-Wclippy::all','-Dwarnings']
for key,value in manifest['workspace']['lints']['clippy'].items():
 cmd+=['-'+{'allow':'A','warn':'W','deny':'D','forbid':'F'}[value]+'clippy::'+key]
for key,value in meta['dependencies'].items():cmd+=['--extern',key+'='+value]
result=subprocess.run(cmd,env=env);raise SystemExit(result.returncode)
