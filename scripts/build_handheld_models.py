"""Original handheld pilot. Blender --background --python scripts/build_handheld_models.py.
GLB: grip origin, +Y shaft/blade, +Z fingertip direction. Metres. No textures.
"""
import json, math, struct
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'client/assets/weapons'
SOURCE=ROOT/'assets-src/weapons'
OUT.mkdir(parents=True,exist_ok=True);SOURCE.mkdir(parents=True,exist_ok=True)
previous=bpy.context.window.scene
scene=bpy.data.scenes.new('Handheld armory');bpy.context.window.scene=scene

def mat(name,color):
    m=bpy.data.materials.new(name);m.diffuse_color=(*color,1);m.use_nodes=True
    m.node_tree.nodes.clear();a=m.node_tree.nodes.new('ShaderNodeEmission');b=m.node_tree.nodes.new('ShaderNodeOutputMaterial');a.inputs['Color'].default_value=(*color,1);a.inputs['Strength'].default_value=1
    m.node_tree.links.new(a.outputs[0],b.inputs['Surface']);return m
steel=mat('Blue steel',(0.20,.34,.45));edge=mat('Silver edge',(.72,.89,.95));dark=mat('Leather grip',(.09,.045,.028));gold=mat('Brass',(.78,.43,.10));glow=mat('Cyan crystal',(.14,.82,1.0))

def primitive(name,kind,loc,scale,material,rotation=(0,0,0)):
    if kind=='cylinder':bpy.ops.mesh.primitive_cylinder_add(vertices=10,radius=1,depth=2)
    elif kind=='crystal':bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=1,radius=1)
    else:bpy.ops.mesh.primitive_cube_add(size=2)
    o=bpy.context.object;o.name=name;o.location=loc;o.scale=scale;o.rotation_euler=rotation;o.data.materials.append(material);return o

def handle(length=.22):
    out=[primitive('Leather handle','cylinder',(0,0,0),(.042,.042,length/2),dark),primitive('Pommel','crystal',(0,0,-length/2-.045),(.073,.065,.07),gold)]
    for z in [-.07,-.035,0,.035,.07]:out.append(primitive('Grip wrap','cylinder',(0,0,z),(.046,.046,.006),gold))
    return out

def export(name,objects):
    bpy.ops.object.select_all(action='DESELECT')
    for o in objects:o.select_set(True)
    bpy.context.view_layer.objects.active=objects[0]
    bpy.ops.object.transform_apply(location=False,rotation=False,scale=True);bpy.ops.object.join();o=bpy.context.object;o.name=name
    bpy.context.scene.cursor.location=(0,0,0);bpy.ops.object.origin_set(type='ORIGIN_CURSOR')
    path=OUT/f'{name}.glb';bpy.ops.export_scene.gltf(filepath=str(path),export_format='GLB',use_selection=True,use_active_scene=True,export_yup=True)
    data=path.read_bytes();n=struct.unpack_from('<I',data,12)[0];d=json.loads(data[20:20+n])
    d['asset']['extras']={'ekza_handheld_v1':dict(bone='rightHand',offset=[0,0,0],rotation_degrees=[0,0,0],scale=1)}
    d.setdefault('extensionsUsed',[]).append('KHR_materials_unlit')
    for m in d['materials']:
        c=m.pop('emissiveFactor');m['pbrMetallicRoughness']['baseColorFactor']=[*c,1];m.setdefault('extensions',{})['KHR_materials_unlit']={}
    raw=json.dumps(d,separators=(',',':')).encode();raw+=b' '*(-len(raw)%4);binary=data[20+n:]
    path.write_bytes(struct.pack('<4sII',b'glTF',2,20+len(raw)+len(binary))+struct.pack('<I4s',len(raw),b'JSON')+raw+binary)
    assert len(d['scenes'])==1
    return o

objects=handle()
objects += [primitive('Crossguard','cube',(0,0,.14),(.22,.035,.035),gold),primitive('Guard jewel','crystal',(0,-.041,.145),(.055,.025,.052),glow)]
# Diamond cross-section, broad taper and pointed blade. Light edge + dark ridge.
verts=[(-.095,0,.19),(0,-.027,.19),(.095,0,.19),(0,.027,.19),(-.075,0,.85),(0,-.024,.85),(.075,0,.85),(0,.024,.85),(0,0,1.12)]
faces=[(0,4,5,1),(1,5,6,2),(2,6,7,3),(3,7,4,0),(4,8,5),(5,8,6),(6,8,7),(7,8,4),(0,1,2,3)]
m=bpy.data.meshes.new('Forged blade');m.from_pydata(verts,[],faces);m.materials.append(edge);m.materials.append(steel)
o=bpy.data.objects.new('Diamond blade',m);scene.collection.objects.link(o)
for i,p in enumerate(m.polygons):p.material_index=i%2
objects.append(o)
sword=export('forge-sword',objects)
objects=handle();objects += [primitive('Hammer shaft','cylinder',(0,0,.31),(.045,.045,.32),dark),primitive('Forged hammer head','cube',(0,0,.68),(.29,.13,.14),steel),primitive('Head inlay','cube',(0,-.134,.68),(.075,.008,.15),gold)]
for side in [-1,1]:objects += [primitive('Brass striking rim','cube',(side*.29,0,.68),(.032,.144,.154),gold),primitive('Silver striking face','cube',(side*.328,0,.68),(.012,.12,.13),edge)]
hammer=export('forge-hammer',objects)
objects=handle();objects += [primitive('Scepter shaft','cylinder',(0,0,.28),(.037,.037,.31),steel),primitive('Crown base','cylinder',(0,0,.59),(.11,.11,.035),gold),primitive('Faceted crystal','crystal',(0,0,.78),(.13,.13,.24),glow)]
for side in [-1,1]:objects.append(primitive('Crown prong','cube',(side*.105,0,.69),(.018,.025,.13),gold,(0,side*-.25,0)))
scepter=export('dawn-scepter',objects)
for i,o in enumerate([sword,hammer,scepter]):o.location.x=i*1.5
bpy.data.libraries.write(str(SOURCE/'handheld-pilot.blend'),{scene});bpy.context.window.scene=previous
print('Exported sword, hammer, scepter; immutable grip metadata embedded in each GLB')
