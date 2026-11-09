from pathlib import Path
import hashlib, json, os, shutil, subprocess, time, tomllib

root=Path.cwd(); out=root/'.agent/codex-ranked-readiness/load-server-metrics'; out.mkdir(exist_ok=True)
records=json.loads((root/'.agent/codex-ranked-readiness/ready-runtime/server-compile.json').read_text())['commands']
cache=root/'.agent/codex-ranked-readiness/load-opt1'
report={'status':'building','source':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'commands':[],'minimum_root_gib':shutil.disk_usage('/').free/2**30,'scope':'Only the changed server root and its existing server test harness, opt1; all other coherent workspace/third-party rlibs are read-only','opt_flags':['opt-level=1','debug-assertions=yes','overflow-checks=yes','debuginfo=0','codegen-units=256']}
env={k:v for k,v in os.environ.items() if not k.startswith('OMOBA_')}
env.update(CARGO_PKG_VERSION=tomllib.loads((root/'Cargo.toml').read_text())['workspace']['package']['version'],CARGO=shutil.which('cargo'),CARGO_MANIFEST_DIR=str(root/'server'))
for name,original in [('server',records[2]['command']),('server-tests',records[2]['command'])]:
    cmd=[v for v in original.copy() if v!='--emit=metadata']
    if name=='server':
        cmd.remove('--test')
        for i,value in enumerate(cmd):
            if '=' in value and value.split('=',1)[0] in ('shared','common','omoba_passport','omoba_career_store'):
                key=value.split('=',1)[0];cmd[i]=key+'='+str(cache/('lib'+key+'.rlib'))
    target=out/name
    cmd[cmd.index('-o')+1]=str(target)
    cmd+=['-L','dependency='+str(cache)]
    for flag in report['opt_flags']:cmd+=['-C',flag]
    cmd+=['-C','metadata=omoba-server-load-metrics-20261109']
    started=time.monotonic()
    with (out/(name+'.log')).open('w') as log:
        result=subprocess.run(cmd,env=env,stdout=log,stderr=subprocess.STDOUT,timeout=480)
    item={'name':name,'command':cmd,'exit':result.returncode,'seconds':time.monotonic()-started,'output_sha256':hashlib.sha256(target.read_bytes()).hexdigest() if target.exists() else None}
    report['commands'].append(item);report['minimum_root_gib']=min(report['minimum_root_gib'],shutil.disk_usage('/').free/2**30)
    (out/'build.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({k:item[k] for k in ('name','exit','seconds')}),flush=True)
    assert result.returncode==0,(out/(name+'.log')).read_text()[-5000:]
with (out/'server-tests.log').open('w') as log:
    result=subprocess.run([str(out/'server-tests'),'load_metrics::','--test-threads=1'],stdout=log,stderr=subprocess.STDOUT)
print('load_metrics tests exit',result.returncode,flush=True)
assert result.returncode==0,(out/'server-tests.log').read_text()[-5000:]
report['status']='PASS'
report['source_sha256']={str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted((root/'server/src').rglob('*.rs'))}
(out/'build.json').write_text(json.dumps(report,indent=2)+'\n')
