"""Original low-poly ranged props, palm origin; Blender --background --python this_file."""
import json
import math
import struct
from pathlib import Path
import bpy
from mathutils import Matrix, Vector

ROOT = Path(__file__).resolve().parents[1]
previous = bpy.context.window.scene
source_scenes = set()

def mat(name, rgb):
    m = bpy.data.materials.new(name)
    m.diffuse_color = (*rgb, 1)
    m.use_nodes = True
    p = m.node_tree.nodes.get('Principled BSDF')
    p.inputs['Base Color'].default_value = (*rgb, 1)
    p.inputs['Metallic'].default_value = 0.35
    p.inputs['Roughness'].default_value = 0.55
    return m

steel = mat('Midnight gunmetal', (.10, .16, .22))
bronze = mat('Warm brass detail', (.76, .43, .15))
cyan = mat('Arc cell cyan', (.12, .82, .90))
wood = mat('Verdant bow limbs', (.12, .42, .25))
light = mat('Pale string', (.84, .91, .77))


def rod(name, a, b, radius, material, vertices=8):
    a,b = Vector(a),Vector(b)
    bpy.ops.mesh.primitive_cylinder_add(vertices=vertices, radius=radius, depth=(b-a).length)
    o = bpy.context.object
    o.name=name; o.location=(a+b)/2
    o.rotation_mode='QUATERNION';o.rotation_quaternion=(b-a).to_track_quat('Z','Y')
    o.data.materials.append(material)
    return o

def box(name, p, size, material):
    bpy.ops.mesh.primitive_cube_add(size=1, location=p)
    o=bpy.context.object;o.name=name;o.scale=size;o.data.materials.append(material)
    return o

def rocket():
    rod('Rocket casing',(0,0,0),(0,0,.34),.07,steel)
    rod('Rocket exhaust',(0,0,-.10),(0,0,0),.035,cyan)
    bpy.ops.mesh.primitive_cone_add(vertices=8, radius1=.071, radius2=0, depth=.14, location=(0,0,.41))
    bpy.context.object.name='Rocket brass nose';bpy.context.object.data.materials.append(bronze)
    for a in range(4):
        q=a*math.pi/2
        box('Rocket fin',(.085*math.cos(q),.085*math.sin(q),.04),(.025,.025,.15),bronze)

for identity in ['wild-repeater','wild-launcher','verdant-bow','wild-rocket']:
    scene=bpy.data.scenes.new(identity);bpy.context.window.scene=scene;source_scenes.add(scene)
    if identity in ['wild-repeater','wild-launcher']:
        # Geometry is authored in canonical glTF palm coordinates.
        rod('Palm grip',(0,-.10,0),(0,.10,0),.035,steel)
        box('Receiver',(0,.08,.12),(.14,.14,.28),steel)
        rod('Rear power cell',(0,.07,-.18),(0,.07,.02),.07,cyan)
        if identity=='wild-repeater':
            for a in range(6):
                q=a*math.pi/3
                x=.055*math.cos(q);y=.08+.055*math.sin(q)
                rod('Repeater barrel',(x,y,.20),(x,y,.63),.019,bronze)
            rod('Barrel collar',(0,.08,.39),(0,.08,.44),.088,steel)
        else:
            rod('Launcher tube',(0,.08,.10),(0,.08,.62),.125,steel,12)
            rod('Launcher muzzle',(0,.08,.60),(0,.08,.65),.137,bronze,12)
            rod('Muzzle bore',(0,.08,.65),(0,.08,.654),.095,steel,12)
            box('Sight',(0,.23,.25),(.035,.08,.14),cyan)
    elif identity=='verdant-bow':
        # Vertical bow in glTF; grip at palm; swept limbs toward fingertips.
        for side in [-1,1]:
            limb=[(0,0,0),(0,side*.20,.07),(0,side*.43,.15),(0,side*.57,.06)]
            for i in range(3): rod('Swept bow limb',limb[i],limb[i+1],.027-i*.005,wood)
        rod('Bow string',(0,-.57,.06),(0,.57,.06),.0035,light)
        rod('Leather palm grip',(0,.08,0),(0,.08,0),.039,bronze)
    else:
        rocket()
    bpy.ops.object.select_all(action='SELECT')
    bpy.context.view_layer.objects.active=list(scene.objects)[0]
    bpy.ops.object.transform_apply(location=False,rotation=False,scale=True)
    bpy.ops.object.join();bpy.context.object.name=identity
    scene.cursor.location=(0,0,0);bpy.ops.object.origin_set(type='ORIGIN_CURSOR')
    # glTF exporter converts Blender Z-up back to Y-up; bake the inverse first.
    bpy.context.object.data.transform(Matrix.Rotation(math.pi / 2, 4, 'X'))
    path=ROOT/f'client/assets/weapons/{identity}.glb'
    bpy.ops.export_scene.gltf(filepath=str(path),export_format='GLB',use_selection=True,use_active_scene=True,export_yup=True)
    data=path.read_bytes();size=struct.unpack_from('<I',data,12)[0];doc=json.loads(data[20:20+size])
    doc['asset']['extras']={'ekza_handheld_v1':{'bone':'rightHand','offset':[0,0,0],'rotation_degrees':[0,0,0],'scale':1}}
    encoded=json.dumps(doc,separators=(',',':')).encode();encoded+=b' '*(-len(encoded)%4);binary=data[20+size:]
    path.write_bytes(struct.pack('<4sII',b'glTF',2,20+len(encoded)+len(binary))+struct.pack('<I4s',len(encoded),b'JSON')+encoded+binary)
    print(identity, 'vertices',sum(len(m.vertices) for m in [bpy.context.object.data]), 'bytes',path.stat().st_size)
bpy.data.libraries.write(str(ROOT/'assets-src/weapons/ranged-handhelds.blend'),source_scenes)
bpy.context.window.scene=previous
