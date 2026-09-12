from pathlib import Path
import subprocess, sys
root=Path.cwd()
target=Path('/Users/wotori/git/ekza/omoba-bevy-mobile-beta/.agent/tasks/MOBILE-BETA-2026-09-11/target-native/debug')
raw=root/'.agent/tasks/COMBAT-FEEL-2026-09-12/raw/final'
raw.mkdir(exist_ok=True)
cases=[('ranger-final','ranger','models3d',[]),('mage-mobile','mage','models3d',['--touch-controls','--width','844','--height','390']),('cleric-desktop','cleric','models3d',[]),('warrior-desktop','warrior','models3d',[]),('ranger-2d','ranger','sprite2d',[]),('mixed-wave','ranger','models3d',['--waves','--timeout','180'])]
for name,hero,mode,extra in cases:
    cmd=[sys.executable,'scripts/capture_combat.py','--client-bin',str(target/'client'),'--server-bin',str(target/'server'),'--assets','client/assets','--output',str(raw/name),'--class',hero,'--mode',mode,'--timeout','90',*extra]
    with (raw/(name+'.log')).open('w') as log:
        result=subprocess.run(cmd,stdout=log,stderr=subprocess.STDOUT)
    print(name, result.returncode, flush=True)
    if result.returncode: sys.exit(result.returncode)
