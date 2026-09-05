"""Build the offline Verdant Confluence arena in Blender 5.x.

Run in a dedicated Blender process: blender --factory-startup --python PATH.
The script replaces that process's startup scene; do not run in an unsaved document.
"""
import bpy
import math
import random
import json
import sys
from pathlib import Path
from mathutils import Vector

HERE = Path(__file__).resolve().parent
OUT = HERE.parent
sys.path.insert(0, str(HERE))
SEED = 260905
RNG = random.Random(SEED)
HALF_BASE = 225 / math.sqrt(2) / 2
HALF = HALF_BASE + 29
LANE_WIDTH = 12.0
RIVER_WIDTH = 18.0
SQRT2 = math.sqrt(2)
HOME = (-HALF_BASE, HALF_BASE)
AWAY = (HALF_BASE, -HALF_BASE)
EDGE = HALF - 12
LANES = [
    [HOME, AWAY],
    [HOME, (-EDGE, HOME[1]), (-EDGE, -EDGE), (EDGE, -EDGE), (AWAY[0], -EDGE), AWAY],
    [HOME, (HOME[0], EDGE), (-EDGE, EDGE), (EDGE, EDGE), (EDGE, AWAY[1]), AWAY],
]
CAMPS = [(-HALF*2*.34, -HALF*2*.22), (HALF*2*.34, HALF*2*.22), (-HALF*2*.22, HALF*2*.34)]
BOSSES = [(HALF*2*.22, HALF*2*.34), (-HALF*2*.22, -HALF*2*.34)]
PLACEMENTS = []


def collection(name):
    c = bpy.data.collections.new(name)
    bpy.context.scene.collection.children.link(c)
    return c


def move(obj, col):
    for c in list(obj.users_collection):
        c.objects.unlink(obj)
    col.objects.link(obj)
    return obj


def material(name, rgb, rough=.85, metallic=0, emission=0):
    m = bpy.data.materials.new('VC / ' + name)
    m.diffuse_color = (*rgb, 1)
    m.use_nodes = True
    p = m.node_tree.nodes.get('Principled BSDF')
    p.inputs['Base Color'].default_value = (*rgb, 1)
    p.inputs['Roughness'].default_value = rough
    p.inputs['Metallic'].default_value = metallic
    if emission:
        p.inputs['Emission Color'].default_value = (*rgb, 1)
        p.inputs['Emission Strength'].default_value = emission
    return m


def mesh(name, verts, faces, mats, col, indices=None):
    data = bpy.data.meshes.new(name)
    data.from_pydata(verts, [], faces)
    data.update()
    if len({round(v[2], 6) for v in verts}) == 1 or name == 'Base / walk-up ramp':
        for p in data.polygons:
            if p.normal.z < 0: p.flip()
        data.update()
    obj = bpy.data.objects.new(name, data)
    col.objects.link(obj)
    for m in mats:
        data.materials.append(m)
    if indices:
        for p, i in zip(data.polygons, indices):
            p.material_index = i
    return obj


def box(name, xyz, size, mat, col, bevel=0):
    bpy.ops.mesh.primitive_cube_add(size=1, location=xyz)
    o = move(bpy.context.object, col)
    o.name = name
    o.scale = size
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    o.data.materials.append(mat)
    if bevel:
        b = o.modifiers.new('Hand softened edges', 'BEVEL')
        b.width = bevel
        b.segments = 2
        o.modifiers.new('Corner normals', 'WEIGHTED_NORMAL')
    return o


def disk(name, xy, radius, z, mat, col, depth=.16, vertices=48):
    bpy.ops.mesh.primitive_cylinder_add(vertices=vertices, radius=radius, depth=depth, location=(*xy,z-depth/2))
    o = move(bpy.context.object, col)
    o.name = name
    o.data.materials.append(mat)
    return o


def ring(name, xy, radius, width, z, mat, col, start=0, end=math.tau, n=64):
    verts=[]; faces=[]
    for i in range(n+1):
        a=start+(end-start)*i/n
        for r in (radius-width/2, radius+width/2):
            verts.append((xy[0]+r*math.cos(a),xy[1]+r*math.sin(a),z))
    for i in range(n): faces.append((i*2,i*2+1,i*2+3,i*2+2))
    return mesh(name,verts,faces,[mat],col)


def segment_distance(p, a, b):
    dx,dy=b[0]-a[0],b[1]-a[1]
    t=max(0,min(1,((p[0]-a[0])*dx+(p[1]-a[1])*dy)/(dx*dx+dy*dy)))
    return math.hypot(p[0]-a[0]-t*dx,p[1]-a[1]-t*dy)


def lane_distance(p):
    return min(segment_distance(p,a,b) for lane in LANES for a,b in zip(lane,lane[1:]))


def sample_line(points, fraction):
    lengths=[math.dist(a,b) for a,b in zip(points,points[1:])]
    d=sum(lengths)*fraction
    for a,b,l in zip(points,points[1:],lengths):
        if d<=l: return (a[0]+(b[0]-a[0])*d/l,a[1]+(b[1]-a[1])*d/l)
        d-=l
    return points[-1]


TOWERS = [sample_line(lane,t) for lane in LANES for t in (.3,.7)]


def clear(p, radius=0):
    if max(abs(p[0]),abs(p[1]))>HALF-3-radius: return False
    if lane_distance(p)<6+radius+1: return False
    if abs(p[1]-p[0])/SQRT2<9+radius+1.5: return False
    if any(max(abs(p[0]-b[0]),abs(p[1]-b[1]))<29+radius for b in (HOME,AWAY)): return False
    if any(math.dist(p,c)<12+radius for c in CAMPS+BOSSES): return False
    if any(math.dist(p,c)<5+radius for c in TOWERS): return False
    return True


def descendants(root):
    return [root]+list(root.children_recursive)


def instance(key, xy, z=0, scale=1, angle=0, role='decoration'):
    src=LIB[key]
    root=bpy.data.objects.new(key+' / %04d'%len(PLACEMENTS),None)
    ENV.objects.link(root)
    root.location=(*xy,z); root.rotation_euler[2]=angle; root.scale=(scale,)*3
    root['asset_id']=key; root['role']=role
    for child in src.children:
        o=child.copy(); o.data=child.data
        ENV.objects.link(o); o.parent=root; o.matrix_parent_inverse.identity(); o.matrix_basis=child.matrix_basis.copy()
    PLACEMENTS.append({'name':root.name,'asset':key,'position_blender':list(root.location),'rotation_z':angle,'scale':scale,'role':role})
    return root


def normalize_asset(root):
    # Collapse modeled pieces into one reusable multi-material mesh, baking modifiers.
    meshes=[o for o in root.children_recursive if o.type=='MESH']
    bpy.context.view_layer.update()
    depsgraph=bpy.context.evaluated_depsgraph_get()
    vertices=[]; faces=[]; materials=[]; indices=[]
    for child in meshes:
        evaluated=child.evaluated_get(depsgraph)
        data=evaluated.to_mesh()
        transform=root.matrix_world.inverted()@child.matrix_world
        offset=len(vertices)
        vertices.extend(transform@v.co for v in data.vertices)
        mapping=[]
        for mat in data.materials:
            mat=mat.original
            if mat not in materials: materials.append(mat)
            mapping.append(materials.index(mat))
        for face in data.polygons:
            faces.append(tuple(offset+i for i in face.vertices))
            indices.append(mapping[face.material_index])
        evaluated.to_mesh_clear()
    o=mesh(root.name+' / mesh',vertices,faces,materials,root.users_collection[0],indices)
    o.parent=root
    for child in meshes: bpy.data.objects.remove(child,do_unlink=True)
    # Remove zero-area faces and loose edges from narrow beveled model details.
    import bmesh
    bm=bmesh.new(); bm.from_mesh(o.data)
    bad=[f for f in bm.faces if f.calc_area()<1e-10]
    bmesh.ops.delete(bm,geom=bad,context='FACES')
    loose=[e for e in bm.edges if not e.link_faces]
    bmesh.ops.delete(bm,geom=loose,context='EDGES')
    isolated=[v for v in bm.verts if not v.link_edges]
    bmesh.ops.delete(bm,geom=isolated,context='VERTS')
    bm.to_mesh(o.data); bm.free(); o.data.update()
    for child in list(root.children_recursive):
        if child!=o and child.type=='EMPTY': bpy.data.objects.remove(child,do_unlink=True)
    return root


def terrain():
    # Diamond coordinates allow a real open river channel rather than water over a plane.
    grassm=[P['grass'],material('moss shadow',(.14,.23,.13)),material('moss light',(.23,.32,.16))]
    for sign in (-1,1):
        verts=[]; faces=[]; indices=[]
        rows=110; cols=19; smax=HALF*SQRT2-10
        for i in range(rows+1):
            s=-smax+2*smax*i/rows
            extent=HALF*SQRT2-abs(s)
            bank=RIVER_WIDTH/2
            bank=min(bank,extent)
            for j in range(cols+1):
                t=sign*(bank+(extent-bank)*j/cols)
                x=(s-t)/SQRT2; y=(s+t)/SQRT2
                verts.append((x,y,-.035))
        for i in range(rows):
            for j in range(cols):
                a=i*(cols+1)+j; b=a+cols+1
                face=(a,a+1,b+1,b) if sign<0 else (a,b,b+1,a+1)
                faces.append(face)
                indices.append(0 if RNG.random()<.88 else RNG.choice([1,2]))
        mesh('Landscape / meadow %d'%sign,verts,faces,grassm,ENV,indices)
        # Continuous exposed strata along the river bank.
        v=[]; f=[]
        for i in range(rows+1):
            s=-smax+2*smax*i/rows; t=sign*RIVER_WIDTH/2
            x=(s-t)/SQRT2; y=(s+t)/SQRT2
            v.extend([(x,y,-.04),(x,y,-1.6),(x-sign*.8,y+sign*.8,-3)])
        for i in range(rows):
            for j in range(2): f.append((i*3+j,i*3+j+1,(i+1)*3+j+1,(i+1)*3+j))
        mesh('Landscape / eroded bank %d'%sign,v,f,[P['earth'],P['rock']],ENV,[i%2 for i in range(len(f))])
    # Solid low diorama foundation with hand-faceted cliff strata.
    top=[(-HALF,-HALF),(HALF,-HALF),(HALF,HALF),(-HALF,HALF)]
    verts=[]
    n=96
    outline=[]
    for k in range(4):
        a,b=top[k],top[(k+1)%4]
        for i in range(n//4): outline.append((a[0]+(b[0]-a[0])*i/(n//4),a[1]+(b[1]-a[1])*i/(n//4)))
    for level,z in enumerate([-.12,-2.8,-6.5,-8.5]):
        for i,(x,y) in enumerate(outline):
            s=1 if level==0 else 1-.005*level+.007*math.sin(i*2.13+level)
            verts.append((x*s,y*s,z+(0 if level==0 else .45*math.sin(i*1.34))))
    faces=[]; ids=[]
    for level in range(3):
        for i in range(n):
            faces.append((level*n+i,level*n+(i+1)%n,(level+1)*n+(i+1)%n,(level+1)*n+i)); ids.append(level%3)
    faces.append(tuple(reversed(range(n*3,n*4)))); ids.append(2)
    mesh('Landscape / stratified island',verts,faces,[P['earth'],P['rock'],P['stone_dark']],ENV,ids)
    # Water surface at -0.6; opaque glTF material keeps runtime handoff deterministic.
    waterverts=[]
    smax=HALF*SQRT2
    for i in range(121):
        s=-smax+2*smax*i/120; extent=min(RIVER_WIDTH/2,max(0,smax-abs(s)))
        for t in (-extent,extent): waterverts.append(((s-t)/SQRT2,(s+t)/SQRT2,-.6))
    mesh('River / turquoise channel',waterverts,[(i*2,i*2+1,i*2+3,i*2+2) for i in range(120)],[P['water']],ENV)
    for sign in (-1,1):
        verts=[]
        for i in range(111):
            s=-smax+12+(2*smax-24)*i/110
            for t in (sign*7.0,sign*RIVER_WIDTH/2): verts.append(((s-t)/SQRT2,(s+t)/SQRT2,-.585))
        mesh('River / jade shallows',verts,[(i*2,i*2+1,i*2+3,i*2+2) for i in range(110)],[P['teal']],ENV)
    # Fine current glints stay parallel to the water; no foam across dry terrain.
    for i in range(45):
        s=RNG.uniform(-HALF*1.25,HALF*1.25); t=RNG.uniform(-6.5,6.5)
        if abs(s)<12: continue
        x,y=(s-t)/SQRT2,(s+t)/SQRT2
        o=box('River / current highlight',(x,y,-.575),(RNG.uniform(1,4),.075,.012),P['foam'],ENV)
        o.rotation_euler[2]=math.pi/4


def paths_and_bases():
    road=P['path']
    for li,points in enumerate(LANES):
        for a,b in zip(points,points[1:]):
            length=math.dist(a,b)
            if length<.1: continue
            o=box('Route %d / continuous stone bed'%li,((a[0]+b[0])/2,(a[1]+b[1])/2,.04),(length,12,.12),road,ENV)
            o.rotation_euler[2]=math.atan2(b[1]-a[1],b[0]-a[0])
            # Broken broad paving joints use few long meshes, quiet from gameplay height.
            for j in range(int(length/4)):
                t=(j+.5)*4/length; x=a[0]+(b[0]-a[0])*t; y=a[1]+(b[1]-a[1])*t
                if abs(y-x)/SQRT2<11: continue
                if any(max(abs(x-q[0]),abs(y-q[1]))<24 for q in (HOME,AWAY)): continue
                dx,dy=(b[0]-a[0])/length,(b[1]-a[1])/length
                o=box('Route / worn joint',(x,y,.109),(.075,11.5,.008),P['stone_dark'],ENV)
                o.rotation_euler[2]=math.atan2(dy,dx)
                if j%4==0:
                    for side in (-1,1):
                        o=box('Route / verge curb',(x-dy*side*6.3,y+dx*side*6.3,.12),(3.5,.48,.26),P['stone'],ENV,bevel=.07)
                        o.rotation_euler[2]=math.atan2(dy,dx)
    for faction,center in [('green',HOME),('blue',AWAY)]:
        box('Base %s / 46m pad'%faction,(*center,.35),(46,46,.7),P['stone_dark'],ENV,.18)
        box('Base %s / paving'%faction,(*center,.705),(44.5,44.5,.025),P['path'],ENV,.12)
        # Mitered six-meter skirt: four trapezoids share corner edges without overlap.
        corners=[(-1,-1),(1,-1),(1,1),(-1,1)]
        for i in range(4):
            a,b=corners[i],corners[(i+1)%4]
            v=[(center[0]+23*a[0],center[1]+23*a[1],.7),
               (center[0]+23*b[0],center[1]+23*b[1],.7),
               (center[0]+29*b[0],center[1]+29*b[1],0),
               (center[0]+29*a[0],center[1]+29*a[1],0)]
            mesh('Base / walk-up ramp',v,[(0,1,2,3)],[P['path']],ENV)
        ring('Base / ceremonial outer circuit',center,19,.22,.732,P['brass'],ENV)
        ring('Base / inner circuit',center,12,.3,.735,P['stone_light'],ENV)
        for i in range(16):
            a=math.tau*i/16
            o=box('Base / radial inlay',(center[0]+16*math.cos(a),center[1]+16*math.sin(a),.733),(1.1,.2,.01),P['brass'],ENV)
            o.rotation_euler[2]=a
        instance('sanctuary_'+faction,center,z=.72,role='base_landmark')
        for dx,dy in [(-19,-19),(19,-19),(-19,19),(19,19)]:
            instance('lantern',(center[0]+dx,center[1]+dy),z=.72,role='base_accent')
        for dx,dy in [(-17,-17),(17,-17),(-17,17),(17,17)]:
            p=(center[0]+dx,center[1]+dy)
            disk('Base / corner garden rim',p,2.8,.93,P['stone_light'],ENV,depth=.22,vertices=12)
            disk('Base / garden soil',p,2.56,.95,P['earth'],ENV,depth=.025,vertices=12)
            instance('flowering_shrub',p,z=.97,scale=1.2,angle=.7,role='base_garden')
            instance('fern_cluster',(p[0]+1.5,p[1]-.8),z=.97,scale=.75,angle=2,role='base_garden')
        for dx,dy in [(-15,-15),(15,15)]:
            instance('banner_'+faction,(center[0]+dx,center[1]+dy),z=.72,angle=math.pi/4,role='base_accent')
    for i,p in enumerate(TOWERS):
        faction='green' if i%2==0 else 'blue'
        disk('Objective / tower foundation',p,4.2,.16,P['stone_dark'],ENV)
        ring('Objective / tower rune rim',p,3.8,.13,.17,P['brass'],ENV)
        instance('watchtower_'+faction,p,z=.17,role='lane_tower')
    # Central bridge and both outer-lane river crossings, directed across the channel.
    instance('bridge',(0,0),angle=-math.pi/4,role='river_crossing')
    for p in [(-EDGE,-EDGE),(EDGE,EDGE)]:
        # A square watergate supports the L-shaped crossing at each outer lane corner.
        box('River crossing / corner watergate',(*p,.09),(25,25,.32),P['stone'],ENV,.20)
        ring('River crossing / watergate inlay',p,9,.18,.255,P['brass'],ENV)
        for dx,dy in [(-10,-10),(10,-10),(-10,10),(10,10)]:
            box('River crossing / foundation',(p[0]+dx,p[1]+dy,-1),(2.5,2.5,2.4),P['stone_dark'],ENV,.16)
            instance('lantern',(p[0]+dx,p[1]+dy),z=.27,role='river_crossing')


def landmarks():
    for i,p in enumerate(CAMPS):
        disk('Camp %d / clearing'%i,p,11.6,.012,P['earth'],ENV,depth=.05)
        ring('Camp / weathered perimeter',p,10.8,.42,.03,P['stone_dark'],ENV,start=.2,end=5.1,n=36)
        for j in range(5):
            a=1+j*.53
            instance('ruin_wall',(p[0]+11*math.cos(a),p[1]+11*math.sin(a)),scale=.55,angle=a+math.pi/2,role='camp_boundary')
        # A composed supply corner signals inhabited, purposeful clearings.
        instance('lantern',(p[0]-7,p[1]+5),scale=.8,role='camp_accent')
    for i,p in enumerate(BOSSES):
        disk('Boss %d / arena floor'%i,p,12,.025,P['stone_dark'],ENV,depth=.08)
        ring('Boss / stone dial',p,9.6,.5,.04,P['stone'],ENV)
        ring('Boss / inner inscription',p,7.2,.13,.045,P['brass'],ENV)
        if i==0:
            for j in range(3):
                a=.6+j*1.05
                instance('ruined_arch',(p[0]+12*math.cos(a),p[1]+12*math.sin(a)),angle=a+math.pi/2,role='boss_boundary')
        else:
            for j in range(5):
                a=.2+j*.64
                instance('rock_stratified_outcrop',(p[0]+12*math.cos(a),p[1]+12*math.sin(a)),scale=.85,angle=a,role='boss_boundary')


def planting():
    trees=[k for k in LIB if k.startswith('tree_')]
    shrubs=[k for k in LIB if 'shrub' in k]
    rocks=[k for k in LIB if k.startswith('boulder')]
    grass=[k for k in LIB if k.startswith('grass') or k.startswith('fern')]
    if not trees or not shrubs or not rocks or not grass:
        raise RuntimeError('Nature asset naming contract not met: '+str(list(LIB)))
    centers=[]
    for _ in range(1700):
        p=(RNG.uniform(-HALF+5,HALF-5),RNG.uniform(-HALF+5,HALF-5))
        if clear(p,4.7) and all(math.dist(p,c)>8.5 for c in centers): centers.append(p)
        if len(centers)>=230: break
    for i,p in enumerate(centers):
        size=RNG.uniform(.72,1.05)
        key=trees[i%len(trees)]
        instance(key,p,scale=size,angle=RNG.random()*math.tau,role='forest_tree')
        if i%2==0:
            for j in range(2):
                a=RNG.random()*math.tau; q=(p[0]+3.5*math.cos(a),p[1]+3.5*math.sin(a))
                if clear(q,1.8): instance(shrubs[j%len(shrubs)],q,scale=RNG.uniform(.6,1.1),angle=a,role='understory')
    for i in range(1200):
        p=(RNG.uniform(-HALF+3,HALF-3),RNG.uniform(-HALF+3,HALF-3))
        if i<200:
            if clear(p,3): instance(rocks[i%len(rocks)],p,scale=RNG.uniform(.55,1.15),angle=RNG.random()*math.tau,role='forest_rock')
        elif clear(p,.7):
            instance(grass[i%len(grass)],p,scale=RNG.uniform(.7,1.4),angle=RNG.random()*math.tau,role='groundcover')
    # Rhythmic riverbank groups, with quiet gaps around the crossings.
    reeds=next(k for k in LIB if 'reed' in k)
    for sign in (-1,1):
        for i in range(48):
            s=-HALF*1.3+i*(HALF*2.6/47)
            if abs(s)<18 or abs(s)>HALF*1.2: continue
            t=sign*RNG.uniform(10.2,12.8)
            p=((s-t)/SQRT2,(s+t)/SQRT2)
            if lane_distance(p)>8 and all(math.dist(p,c)>13 for c in CAMPS+BOSSES):
                instance(reeds,p,z=-.12,scale=RNG.uniform(.8,1.35),angle=RNG.random()*math.tau,role='riverbank')
                if i%3==0 and clear(p,1): instance(rocks[i%len(rocks)],p,scale=.6,angle=s,role='riverbank_rock')
        # Composed shelf groups interrupt the thin waterline with overlapping stone,
        # reeds and ferns. The intentionally embedded stones belong to the bank.
        for i,station in enumerate([-119,-93,-67,-38,33,57,84,113]):
            s=station+sign*4; t=sign*(11.8+.6*math.sin(station))
            p=((s-t)/SQRT2,(s+t)/SQRT2)
            if lane_distance(p)<11 or any(math.dist(p,c)<16 for c in CAMPS+BOSSES): continue
            instance('boulder_moss_flat',p,z=-.28,scale=1.15,angle=.6+i*.7,role='riverbank_shelf')
            for ds,dt,scale in [(-2.4,1.7,.75),(2.6,2.3,.65)]:
                q=((s+ds-t-sign*dt)/SQRT2,(s+ds+t+sign*dt)/SQRT2)
                instance('boulder_moss_tall',q,z=-.16,scale=scale,angle=i,role='riverbank_shelf')
                instance('fern_cluster',(q[0]+.8,q[1]+.8),scale=.9,angle=i,role='riverbank_detail')
            q=((s+2-t-sign*2.8)/SQRT2,(s+2+t+sign*2.8)/SQRT2)
            instance(reeds,q,scale=1.45,angle=i*.9,role='riverbank_detail')
    # Place fallen timber on the interior fringes; no props on the lanes.
    logs=[k for k in LIB if 'log' in k or 'stump' in k]
    for i,p in enumerate(centers[::12]):
        q=(p[0]+4,p[1]+3)
        if logs and clear(q,3): instance(logs[i%len(logs)],q,angle=RNG.random()*math.tau,role='forest_story')


def camera(name, xyz, target, ortho):
    d=bpy.data.cameras.new(name)
    o=bpy.data.objects.new(name,d); STUDIO.objects.link(o)
    o.location=xyz; o.rotation_euler=(Vector(target)-o.location).to_track_quat('-Z','Y').to_euler()
    d.type='ORTHO'; d.ortho_scale=ortho; d.clip_end=1500
    return o


def presentation():
    scene=bpy.context.scene
    bg=material('studio background',(.035,.065,.073))
    box('Presentation / horizon',(0,0,-10.2),(3000,3000,.4),bg,STUDIO)
    d=bpy.data.lights.new('Late afternoon sun','SUN'); d.energy=2.2; d.angle=math.radians(18); d.color=(1,.88,.73)
    o=bpy.data.objects.new('Late afternoon sun',d); STUDIO.objects.link(o); o.rotation_euler=(.45,-.55,-.65)
    for name,loc,power,size,color in [('Sky softbox',(-90,-60,150),100000,160,(.68,.84,1)),('Warm reflected light',(40,100,100),55000,140,(1,.86,.63))]:
        d=bpy.data.lights.new(name,'AREA'); d.energy=power; d.shape='DISK'; d.size=size; d.color=color
        o=bpy.data.objects.new(name,d); STUDIO.objects.link(o); o.location=loc; o.rotation_euler=(-o.location).to_track_quat('-Z','Y').to_euler()
    scene.world=bpy.data.worlds.new('Verdant sky'); scene.world.use_nodes=True
    scene.world.node_tree.nodes['Background'].inputs[0].default_value=(.22,.32,.38,1)
    scene.world.node_tree.nodes['Background'].inputs[1].default_value=.45
    camera('01 / Atlas overview',(230,-300,290),(0,0,-1),315)
    camera('02 / Sanctuary garden',(HOME[0]-12,HOME[1]+68,44),(*HOME,5),64)
    camera('03 / River gameplay',(44,-55,49),(2,-1,1),79)
    camera('04 / Tactical plan',(0,0,310),(0,0,0),239)
    scene.camera=bpy.data.objects['01 / Atlas overview']
    scene.render.engine='CYCLES'; scene.cycles.samples=48; scene.cycles.use_denoising=True
    scene.render.resolution_x=1600; scene.render.resolution_y=1400; scene.render.resolution_percentage=100
    scene.view_settings.view_transform='AgX'
    scene.render.image_settings.file_format='PNG'
    for screen in bpy.data.screens:
        for a in screen.areas:
            if a.type=='VIEW_3D':
                a.spaces.active.region_3d.view_perspective='CAMERA'
                a.spaces.active.region_3d.view_camera_zoom=0
                a.spaces.active.overlay.show_overlays=False
                a.spaces.active.shading.type='MATERIAL'


def export_selection(path, objects):
    bpy.context.view_layer.update()
    bpy.ops.object.select_all(action='DESELECT')
    for o in objects: o.select_set(True)
    bpy.context.view_layer.objects.active=next((o for o in objects if o.type=='MESH'),objects[0])
    bpy.ops.export_scene.gltf(filepath=str(path),export_format='GLB',use_selection=True,export_apply=True,export_extras=True,export_cameras=False,export_lights=False)


def inventory(root):
    points=[]; tris=0
    for o in root.children_recursive:
        if o.type!='MESH': continue
        points.extend(o.matrix_world@Vector(p) for p in o.bound_box)
        o.data.calc_loop_triangles(); tris+=len(o.data.loop_triangles)
    lo=[min(p[i] for p in points) for i in range(3)]; hi=[max(p[i] for p in points) for i in range(3)]
    return {'bounds_blender':{'min':lo,'max':hi},'dimensions':[hi[i]-lo[i] for i in range(3)],'triangles':tris}


def main():
    global P,ENV,STUDIO,LIB
    # Dedicated factory-startup process keeps the user's existing Blender window intact.
    scene=bpy.context.scene
    for o in list(scene.objects): bpy.data.objects.remove(o,do_unlink=True)
    scene.name='Verdant Confluence / assembled arena'
    scene.unit_settings.system='METRIC'; scene.unit_settings.scale_length=1
    ENV=collection('01 ENVIRONMENT / exportable')
    library=collection('02 LIBRARY / original modular assets')
    STUDIO=collection('03 PRESENTATION / cameras and lighting')
    P={
        'stone':material('ivory limestone',(.61,.57,.43)),
        'stone_light':material('cut limestone',(.77,.73,.58)),
        'stone_dark':material('weathered stone',(.22,.29,.25)),
        'brass':material('aged brass',(.43,.29,.105),.42,.62),
        'wood':material('warm heartwood',(.18,.09,.045)),
        'teal':material('jade shallows',(.08,.39,.36),.3),
        'green_glow':material('emerald beacon',(.12,.75,.39),.28,.1,1.0),
        'blue_glow':material('azure beacon',(.13,.42,.95),.28,.1,1.0),
        'water':material('deep turquoise water',(.025,.24,.27),.22,.25),
        'leaf_dark':material('deep teal canopy',(.045,.15,.105)),
        'leaf_mid':material('jade canopy',(.10,.30,.145)),
        'leaf_light':material('sage canopy',(.28,.44,.20)),
        'grass':material('moss meadow',(.18,.275,.135)),
        'earth':material('earth and clay',(.23,.185,.11)),
        'rock':material('slate strata',(.25,.30,.265)),
        'flower':material('warm wildflower',(.87,.66,.29)),
        'cloth':material('undyed linen',(.67,.61,.41)),
        'path':material('worn ceremonial paving',(.49,.455,.34)),
        'foam':material('water glints',(.35,.67,.60),.3),
    }
    import importlib, architecture, nature
    importlib.reload(architecture); importlib.reload(nature)
    print('VC: creating architecture',flush=True)
    LIB=architecture.build_library(P)
    print('VC: creating nature',flush=True)
    LIB.update(nature.build_library(P))
    bpy.context.view_layer.update()
    for key,root in LIB.items():
        print('VC: normalizing',key,flush=True)
        normalize_asset(root)
        root.name='Asset / '+key; root['asset_id']=key; root['provenance']='Original geometry authored for this project'
        for o in descendants(root): move(o,library)
    assets=[]
    for key,root in LIB.items():
        export_selection(OUT/'library'/(key+'.glb'),descendants(root))
        assets.append({'id':key,'path':'library/'+key+'.glb',**inventory(root),'origin':'ground-level local origin','provenance':'original project-authored geometry'})
    print('VC: modular kit ready',len(LIB),flush=True)
    terrain(); paths_and_bases(); landmarks(); planting(); presentation()
    print('VC: assembly ready',len(PLACEMENTS),'placements',flush=True)
    export_selection(OUT/'exports'/'verdant-confluence.glb',list(ENV.objects))
    library.hide_render=True; library.hide_viewport=True
    bpy.ops.object.select_all(action='DESELECT')
    scene['art_direction']='Verdant Concord / ruins of a living observatory'
    scene['seed']=SEED
    scene['axis_mapping']='Blender (x,y,z) -> Bevy/glTF (x,z,-y)'
    layout={'half_extent':HALF,'base_centers_blender':[HOME,AWAY],'base_pad_size':46,'base_pad_height':.7,'ramp_length':6,'lane_width':12,'lanes_blender':LANES,'river_width':18,'river_endpoints_blender':[[-EDGE,-EDGE],[EDGE,EDGE]],'camps_blender':CAMPS,'bosses_blender':BOSSES,'towers_blender':TOWERS}
    manifest={'title':'Verdant Confluence','source':'verdant-confluence.blend','assembled_glb':'exports/verdant-confluence.glb','seed':SEED,'units':'1 Blender meter = 1 game unit','blender_to_bevy':'(x,y,z) -> (x,z,-y); glTF exporter performs this rotation','layout':layout,'assets':assets,'placements':PLACEMENTS,'presentation_only':['cameras','studio lights','horizon plane'],'limitations':['No collision meshes or navigation data yet','No LODs or in-game performance measurement','Water is an opaque PBR surface; shader motion belongs to integration','No gameplay actors or gameplay code integrated']}
    (OUT/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'verdant-confluence.blend'))
    print('VC_BUILD_COMPLETE',flush=True)


if __name__=='__main__': main()
