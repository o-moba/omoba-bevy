"""Render the saved scene: blender --background FILE.blend --python SCRIPT."""
import bpy
import sys
import json
from pathlib import Path

OUT=Path(__file__).resolve().parent.parent
scene=bpy.context.scene
views=[
    ('01 / Atlas overview','01-overview.png',1600,1400,48),
    ('02 / Sanctuary garden','02-sanctuary.png',1400,1400,48),
    ('03 / River gameplay','03-river-gameplay.png',1600,1200,48),
    ('04 / Tactical plan','04-tactical-plan.png',1400,1400,24),
]
selected=sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else []
for name,filename,w,h,samples in views:
    if selected and filename not in selected: continue
    scene.camera=bpy.data.objects[name]
    scene.render.resolution_x=w; scene.render.resolution_y=h
    scene.render.resolution_percentage=100
    scene.cycles.samples=samples
    scene.render.filepath=str(OUT/'renders'/filename)
    bpy.ops.render.render(write_still=True)
    print('VC_RENDER_COMPLETE '+filename,flush=True)
