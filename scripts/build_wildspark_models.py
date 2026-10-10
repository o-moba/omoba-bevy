"""Original Wildspark ordnance, no external meshes/textures.

Blender --background --factory-startup --python scripts/build_wildspark_models.py
Canonical export: palm origin, +Y up, +Z muzzle. Named moving parts keep pivots.
Supersedes only Wildspark exports of the older standard/ranged generators.
"""
import json
import math
import struct
from pathlib import Path
import bpy
from mathutils import Matrix, Vector

ROOT = Path(__file__).resolve().parents[1]
SCENES = set()
previous = bpy.context.window.scene

def material(name, color, metal=0.6, glow=0):
    m = bpy.data.materials.new(name)
    m.diffuse_color = (*color, 1)
    m.use_nodes = True
    p = m.node_tree.nodes.get('Principled BSDF')
    for key, value in [('Base Color', (*color, 1)), ('Metallic', metal), ('Roughness', .36),
                       ('Emission Color', (*color, 1)), ('Emission Strength', glow)]:
        p.inputs[key].default_value = value
    return m

steel = material('Wildspark | graphite enamel', (.075, .13, .18))
brass = material('Wildspark | worn brass', (.72, .43, .13))
orange = material('Wildspark | safety orange', (.92, .19, .025), .25)
cyan = material('Wildspark | arc cell', (.04, .55, .8), .2, .8)
black = material('Wildspark | bore and rubber', (.012, .023, .03), .05)
parts = []

def keep(o, name, mat):
    o.name = name
    o.data.materials.append(mat)
    parts.append(o)
    return o

def box(name, at, size, mat, bevel=.008):
    bpy.ops.mesh.primitive_cube_add(size=1, location=at)
    o = keep(bpy.context.object, name, mat)
    o.scale = size
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    if bevel:
        mod = o.modifiers.new('Machined edges', 'BEVEL'); mod.width = bevel; mod.segments = 1
        bpy.ops.object.modifier_apply(modifier=mod.name)
    return o

def rod(name, a, b, radius, mat, n=12):
    a, b = Vector(a), Vector(b)
    bpy.ops.mesh.primitive_cylinder_add(vertices=n, radius=radius, depth=(b-a).length)
    o = keep(bpy.context.object, name, mat)
    o.location = (a+b)/2
    o.rotation_mode = 'QUATERNION'; o.rotation_quaternion = (b-a).to_track_quat('Z', 'Y')
    return o

def tube(name, x, y, start, end, outer, inner, mat, n=16):
    # True open bore with inner wall and annular end caps (no black disc over muzzle).
    verts = [(x+r*math.cos(a*math.tau/n), y+r*math.sin(a*math.tau/n), z)
             for z, r in [(start,outer),(end,outer),(start,inner),(end,inner)] for a in range(n)]
    faces = []
    for j in range(n):
        k = (j+1)%n
        faces.extend([(j,k,n+k,n+j), (2*n+k,2*n+j,3*n+j,3*n+k),
                      (n+j,n+k,3*n+k,3*n+j), (k,j,2*n+j,2*n+k)])
    mesh = bpy.data.meshes.new(name); mesh.from_pydata(verts, [], faces); mesh.update()
    o = bpy.data.objects.new(name, mesh); bpy.context.scene.collection.objects.link(o)
    return keep(o, name, mat)

def join(name, pivot=(0,0,0)):
    global parts
    bpy.ops.object.select_all(action='DESELECT')
    for o in parts: o.select_set(True)
    bpy.context.view_layer.objects.active = parts[0]
    bpy.ops.object.join(); o = bpy.context.object; o.name = name
    bpy.context.scene.cursor.location = pivot
    bpy.ops.object.origin_set(type='ORIGIN_CURSOR')
    bpy.ops.object.transform_apply(location=False, rotation=True, scale=True)
    # Bake inverse exporter conversion into each local mesh, preserving canonical pivots.
    inverse = Matrix.Rotation(math.pi/2, 4, 'X')
    o.data.transform(inverse); o.location = inverse @ Vector(pivot)
    parts = []
    return o

def rocket(scale=1):
    rod('Motor casing', (0,0,-.56), (0,0,.29), .195, steel)
    for z in [-.45, -.23, .24]: tube('Reinforcing collar',0,0,z,z+.055,.222,.194,brass)
    bpy.ops.mesh.primitive_cone_add(vertices=12, radius1=.235, radius2=.035, depth=.5, location=(0,0,.55))
    keep(bpy.context.object,'Armoured nose',orange)
    tube('Nozzle',0,0,-.73,-.52,.18,.12,steel)
    rod('Motor core',(0,0,-.69),(0,0,-.68),.117,cyan)
    for i in range(4):
        a=i*math.pi/2
        fin=box('Stabilising fin',(.29*math.cos(a),.29*math.sin(a),-.36),(.30,.035,.38),brass)
        fin.rotation_euler.z=a
    o=join('RocketHull')
    o.data.transform(Matrix.Scale(scale,4))
    return o

for identity, path in [
    ('wild-repeater','weapons/wildspark-repeater.glb'), ('wild-launcher','weapons/wild-launcher.glb'),
    ('rocket','cosmetics/standard/rocket.glb'), ('wild-rocket','weapons/wild-rocket.glb'),
    ('trap','cosmetics/standard/trap.glb')]:
    scene=bpy.data.scenes.new('Wildspark '+identity); bpy.context.window.scene=scene; SCENES.add(scene)
    root=bpy.data.objects.new(identity,None); scene.collection.objects.link(root)
    children=[]
    if identity.startswith('wild-') and identity != 'wild-rocket':
        box('Palm grip',(0,-.035,0),(.08,.24,.10),black)
        box('Receiver',(0,.11,.06),(.20,.19,.32),steel)
        box('Stock',(0,.12,-.19),(.14,.13,.22),steel)
        box('Butt plate',(0,.12,-.32),(.16,.18,.035),black)
        for x in [-.106,.106]:
            box('Side panel',(x,.12,.05),(.018,.11,.23),orange)
            box('Arc strip',(x,.17,.05),(.021,.022,.15),cyan,.002)
        rod('Ammo drum',(-.17,.06,.06),(.17,.06,.06),.095,brass)
        box('Sight rail',(0,.245,.12),(.045,.025,.26),brass)
        children.append(join('WildsparkReceiver'))
        if identity=='wild-repeater':
            for i in range(6):
                a=i*math.tau/6
                x=.069*math.cos(a); y=.12+.069*math.sin(a)
                tube('Rotating barrel',x,y,.23,.73,.028,.016,steel,8)
                tube('Brass muzzle',x,y,.66,.735,.033,.016,brass,8)
            tube('Rotor rear',0,.12,.24,.30,.119,.038,brass)
            tube('Rotor brace',0,.12,.52,.57,.114,.09,steel)
            children.append(join('WildsparkRotor',(0,.12,.25)))
        else:
            tube('Recoil tube',0,.13,.21,.79,.161,.112,steel)
            tube('Muzzle ring',0,.13,.73,.81,.179,.113,brass)
            tube('Orange band',0,.13,.57,.63,.164,.159,orange)
            rod('Bore depth',(0,.13,.22),(0,.13,.23),.11,black)
            for x in [-.17,.17]:
                box('Cooling rail',(x,.13,.43),(.025,.10,.24),brass,.004)
            box('Front sight',(0,.32,.65),(.035,.08,.10),steel,.004)
            box('Arc sight',(0,.363,.65),(.018,.015,.04),cyan,.002)
            children.append(join('WildsparkSlide',(0,.13,.20)))
    elif identity in ['rocket','wild-rocket']:
        children.append(rocket(.24 if identity=='wild-rocket' else 1))
    else:
        rod('Trap base',(0,.035,0),(0,.09,0),.36,steel,16)
        rod('Pressure plate',(0,.09,0),(0,.115,0),.17,orange)
        for x in [-.30,.30]:
            rod('Jaw hinge',(x,.115,-.27),(x,.115,.27),.045,brass)
        for z in [-.23,.23]: box('Live arc indicator',(0,.1,z),(.14,.022,.03),cyan,.003)
        children.append(join('WildsparkTrapBase'))
        for side, name in [(-1,'WildsparkJawLeft'),(1,'WildsparkJawRight')]:
            x=side*.30
            box('Jaw spine',(side*.39,.125,0),(.10,.065,.48),steel)
            for z in [-.19,-.095,0,.095,.19]:
                tooth=box('Interlocking tooth',(side*.365,.20,z),(.08,.15,.045),brass,.004)
                tooth.rotation_euler.z=side*.28
            children.append(join(name,(x,.115,0)))
    for o in children: o.parent=root
    bpy.ops.object.select_all(action='SELECT')
    dest=ROOT/'client/assets'/path
    bpy.ops.export_scene.gltf(filepath=str(dest),export_format='GLB',use_selection=True,use_active_scene=True,export_yup=True)
    data=dest.read_bytes(); length=struct.unpack_from('<I',data,12)[0]; doc=json.loads(data[20:20+length])
    doc['asset']['copyright']='Original art © 2026 OpenMoba contributors; CC-BY-4.0'
    if identity in ['wild-repeater','wild-launcher']:
        doc['asset']['extras']={'ekza_handheld_v1':{'bone':'rightHand','offset':[0,0,0],'rotation_degrees':[0,0,0],'scale':1}}
    encoded=json.dumps(doc,separators=(',',':')).encode(); encoded+=b' '*(-len(encoded)%4); binary=data[20+length:]
    dest.write_bytes(struct.pack('<4sII',b'glTF',2,20+len(encoded)+len(binary))+struct.pack('<I4s',len(encoded),b'JSON')+encoded+binary)
    assert len(doc['scenes'])==1 and doc.get('scene',0)==0
    print('WILDSPARK',path,'meshes',len(doc['meshes']),'vertices',sum(len(o.data.vertices) for o in children),'bytes',dest.stat().st_size)
bpy.data.libraries.write(str(ROOT/'assets-src/skills/wildspark-reference.blend'),SCENES)
bpy.context.window.scene=previous
