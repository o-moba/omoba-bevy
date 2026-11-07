from pathlib import Path
import json,os,subprocess,tomllib
root=Path.cwd();out=root/'.agent/codex-ranked-readiness/queue-delivery';m=tomllib.loads((root/'Cargo.toml').read_text());meta=json.loads((out/'backend-compile.json').read_text())
env={k:v for k,v in os.environ.items() if not k.startswith('OMOBA_')};env.update(CARGO_PKG_VERSION=m['workspace']['package']['version'],CARGO_MANIFEST_DIR=str(root/'server'))
cmd=['clippy-driver','--edition=2024','--crate-name','omoba_backend_proof','--test',str(out/'backend_tests.rs'),'--emit=metadata','-o',str(out/'backend-check.rmeta'),'-L','dependency=/Volumes/Orico/omoba-build-cache/target-b/debug/deps','-Wclippy::all','-Dwarnings']
for name,level in m['workspace']['lints']['clippy'].items():cmd+=['-'+{'allow':'A','warn':'W','deny':'D','forbid':'F'}[level]+'clippy::'+name]
for name,path in meta['dependencies'].items():cmd+=['--extern',name+'='+path]
raise SystemExit(subprocess.run(cmd,env=env).returncode)
