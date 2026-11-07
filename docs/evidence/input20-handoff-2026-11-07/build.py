from pathlib import Path
import hashlib,json,os,subprocess,tomllib
root=Path.cwd();out=root/'.agent/codex-ranked-readiness/load-input20'
base=json.loads((root/'.agent/codex-ranked-readiness/load-opt1/build.json').read_text())
env={k:v for k,v in os.environ.items() if not k.startswith('OMOBA_')};env.update(CARGO_MANIFEST_DIR=str(root/'harness'),CARGO_PKG_VERSION=tomllib.loads((root/'Cargo.toml').read_text())['workspace']['package']['version'],CARGO=str(Path.home()/'.cargo/bin/cargo'))
report={'source_parent':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'commands':[],'scope':'Only actual harness/library+bot+UDP test, opt1 and coherent read-only optimized shared/dependency graph; no Cargo/graphics/cache build'}
for item in base['commands']:
 if item['name'] not in ('harness','bots','transport-tests'):continue
 cmd=item['command'].copy();target=out/('libharness.rlib' if item['name']=='harness' else item['name']);cmd[cmd.index('-o')+1]=str(target);cmd=[('harness='+str(out/'libharness.rlib')) if x.startswith('harness=') else x for x in cmd];cmd+=['-L','dependency='+str(out)]
 with (out/(item['name']+'.log')).open('w') as log:result=subprocess.run(cmd,env=env,stdout=log,stderr=subprocess.STDOUT)
 if result.returncode:print((out/(item['name']+'.log')).read_text()[-3000:]);raise SystemExit(result.returncode)
 report['commands'].append({'name':item['name'],'command':cmd,'sha256':hashlib.sha256(target.read_bytes()).hexdigest()});print('PASS build '+item['name'],flush=True)
report['source_sha256']={str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted((root/'harness/src').rglob('*.rs'))}
(out/'build.json').write_text(json.dumps(report,indent=2)+'\n')
subprocess.run([str(out/'transport-tests'),'--test-threads=1'],check=True)
