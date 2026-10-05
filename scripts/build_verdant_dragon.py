"""Original Verdant Dragon with skeletal idle/walk; no downloaded geometry.
Blender --background --python scripts/build_verdant_dragon.py
"""
import math
from pathlib import Path
import bpy
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[1]
previous=bpy.context.window.scene
scene=bpy.data.scenes.new('Verdant Dragon source');bpy.context.window.scene=scene
scene.render.fps=24

def material(name,rgb,emission=0):
    m=bpy.data.materials.new(name);m.diffuse_color=(*rgb,1);m.use_nodes=True
    p=m.node_tree.nodes.get('Principled BSDF');p.inputs['Base Color'].default_value=(*rgb,1)
    p.inputs['Roughness'].default_value=.65
    if emission:
        p.inputs['Emission Color'].default_value=(*rgb,1);p.inputs['Emission Strength'].default_value=emission
    return m
jade=material('Dragon deep jade scales',(.075,.31,.23));light=material('Dragon turquoise plates',(.12,.57,.42))
gold=material('Dragon warm ivory horns',(.87,.67,.34));membrane=material('Dragon orange wing membranes',(.77,.27,.08))
eye=material('Dragon luminous amber eyes',(1,.56,.05),2);dark=material('Dragon nostrils',(.025,.045,.035))
parts=[]
def bind(o,name,mat,bone):
    o.name=name;o.data.materials.append(mat)
    o.vertex_groups.new(name=bone).add(list(range(len(o.data.vertices))),1,'REPLACE');parts.append(o);return o

def ellipsoid(name,p,scale,mat,bone='root'):
    bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=2,radius=1,location=p)
    o=bpy.context.object;o.scale=scale;return bind(o,name,mat,bone)

def rod(name,a,b,r1,r2,mat,bone='root'):
    a,b=Vector(a),Vector(b);bpy.ops.mesh.primitive_cone_add(vertices=8,radius1=r1,radius2=r2,depth=(b-a).length,location=(a+b)/2)
    o=bpy.context.object;o.rotation_mode='QUATERNION';o.rotation_quaternion=(b-a).to_track_quat('Z','Y')
    return bind(o,name,mat,bone)

ellipsoid('Powerful scaled body',(0,0,.96),(.57,.93,.48),jade)
ellipsoid('Golden chest',(0,.48,.91),(.44,.37,.40),light)
ellipsoid('Raised neck',(0,.83,1.36),(.32,.42,.5),jade)
ellipsoid('Angular dragon head',(0,1.11,1.73),(.32,.45,.29),light,'head')
ellipsoid('Long dragon muzzle',(0,1.46,1.60),(.27,.34,.16),jade,'head')
ellipsoid('Lower jaw',(0,1.47,1.48),(.24,.29,.075),gold,'head')
for side in [-1,1]:
    ellipsoid('Amber eye',(side*.275,1.25,1.80),(.057,.09,.06),eye,'head')
    ellipsoid('Nostril',(side*.15,1.73,1.65),(.036,.025,.02),dark,'head')
    rod('Swept ivory horn',(side*.23,.98,1.95),(side*.42,.52,2.25),.09,0,gold,'head')
    for y in [-.53,.58]:
        bone=('leg_l_' if side<0 else 'leg_r_')+('rear' if y<0 else 'front')
        ellipsoid('Muscular haunch',(side*.46,y,.67),(.22,.27,.42),jade,bone)
        ellipsoid('Clawed foot',(side*.49,y+.12,.17),(.21,.29,.13),light,bone)
        for dx in [-.11,0,.11]:
            rod('Ivory claw',(side*.49+dx,y+.29,.17),(side*.49+dx,y+.48,.10),.044,0,gold,bone)
    bone='wing_l' if side<0 else 'wing_r'
    anchor=(side*.38,.18,1.36)
    elbow=(side*1.14,.09,2.04)
    tip=(side*2.02,.46,1.87)
    rod('Strong wing arm',anchor,elbow,.095,.065,light,bone)
    rod('Wing leading spar',elbow,tip,.065,.025,gold,bone)
    fingers=[tip,(side*1.85,-.47,1.59),(side*1.4,-.88,1.37),(side*.65,-.76,1.17)]
    for f in fingers[1:]:rod('Wing finger',elbow,f,.035,.012,gold,bone)
    vertices=[elbow,*fingers,anchor]
    faces=[(0,1,2),(0,2,3),(0,3,4),(0,4,5)]
    mesh=bpy.data.meshes.new('Scalloped wing membrane');mesh.from_pydata(vertices,[],faces);mesh.update()
    obj=bpy.data.objects.new('Broad dragon wing',mesh);scene.collection.objects.link(obj);bind(obj,'Broad dragon wing',membrane,bone)
for i in range(7):
    y=.55-i*.23;z=1.43-abs(y)*.1
    rod('Back ridge',(0,y,z),(0,y-.11,z+.28),.09,0,gold)
rod('Heavy tail',(0,-.70,.99),(.13,-1.45,.65),.29,.17,jade,'tail')
rod('Tapered tail',(.13,-1.45,.65),(.5,-2.12,.37),.17,.045,light,'tail')
rod('Tail spear',(.5,-2.12,.37),(.56,-2.36,.44),.13,0,gold,'tail')

# Keep rigid prop parts in one low-poly skinned mesh, using semantic animation bones.
bpy.ops.object.select_all(action='DESELECT')
for p in parts:p.select_set(True)
bpy.context.view_layer.objects.active=parts[0];bpy.ops.object.transform_apply(location=False,rotation=False,scale=True);bpy.ops.object.join()
mesh=bpy.context.object;mesh.name='VerdantDragon';scene.cursor.location=(0,0,0);bpy.ops.object.origin_set(type='ORIGIN_CURSOR')
armdata=bpy.data.armatures.new('VerdantDragonRig');rig=bpy.data.objects.new('VerdantDragonRig',armdata);scene.collection.objects.link(rig)
bpy.ops.object.select_all(action='DESELECT');rig.select_set(True);bpy.context.view_layer.objects.active=rig;bpy.ops.object.mode_set(mode='EDIT')
roots={'root':(0,0,.95),'head':(0,.8,1.5),'wing_l':(-.38,.18,1.36),'wing_r':(.38,.18,1.36),'tail':(0,-.7,.99)}
for side in [-1,1]:
 for y in [-.53,.58]:roots[('leg_l_' if side<0 else 'leg_r_')+('rear' if y<0 else 'front')]=(side*.46,y,.67)
for name,p in roots.items():
 b=armdata.edit_bones.new(name);b.head=p;b.tail=Vector(p)+Vector((0,.25,0))
 if name!='root':b.parent=armdata.edit_bones['root']
bpy.ops.object.mode_set(mode='OBJECT');mesh.parent=rig
mod=mesh.modifiers.new('Dragon skeleton','ARMATURE');mod.object=rig
rig.animation_data_create()
for action_name in ['Idle','Walk']:
 action=bpy.data.actions.new(action_name);rig.animation_data.action=action
 for frame in range(1,50,4):
  phase=(frame-1)/48*math.tau
  for bone in rig.pose.bones:
   bone.rotation_mode='XYZ';bone.rotation_euler=(0,0,0);bone.location=(0,0,0)
   if bone.name.startswith('wing_'):
    side=-1 if bone.name.endswith('l') else 1
    bone.rotation_euler.y=side*(.09+.12*math.sin(phase))
   elif bone.name=='tail':bone.rotation_euler.z=.12*math.sin(phase+.7)
   elif bone.name=='head':bone.rotation_euler.x=.05*math.sin(phase)
   elif bone.name=='root':bone.location.z=.035*math.sin(phase*2)
   elif action_name=='Walk' and bone.name.startswith('leg_'):
    offset=0 if bone.name in ['leg_l_front','leg_r_rear'] else math.pi
    bone.rotation_euler.x=.24*math.sin(phase*2+offset)
   bone.keyframe_insert(data_path='rotation_euler',frame=frame,group=bone.name)
   bone.keyframe_insert(data_path='location',frame=frame,group=bone.name)
 track=rig.animation_data.nla_tracks.new();track.name=action_name;strip=track.strips.new(action_name,1,action)
 rig.animation_data.action=None
scene.frame_set(1)
for p in [rig,mesh]:p.select_set(True)
path=ROOT/'client/assets/bosses/verdant-dragon.glb';path.parent.mkdir(parents=True,exist_ok=True)
bpy.ops.export_scene.gltf(filepath=str(path),export_format='GLB',use_selection=True,use_active_scene=True,
 export_yup=True,export_animations=True,export_animation_mode='NLA_TRACKS',export_force_sampling=True)
source=ROOT/'assets-src/bosses/verdant-dragon.blend';source.parent.mkdir(parents=True,exist_ok=True)
bpy.data.libraries.write(str(source),{scene})
bpy.context.window.scene=previous
print('Original Verdant Dragon',path.stat().st_size,'bytes')
