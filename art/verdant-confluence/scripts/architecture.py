"""Original Verdant Confluence modular architecture. Blender 4/5, meters, Z up.

build_library(palette) creates origin-rooted reusable assets in current collection.
All meshes are original procedural craftsmanship; no imported assets or textures.
"""
import math
import bpy
from mathutils import Vector

TAU = math.tau


def _root(name):
    obj = bpy.data.objects.new(name, None)
    bpy.context.collection.objects.link(obj)
    obj['asset_family'] = 'architecture'
    obj['units'] = 'meters'
    obj['ground_pivot'] = True
    return obj


def _mesh(root, name, vertices, faces, material, bevel=0):
    mesh = bpy.data.meshes.new(root.name + '_' + name)
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(mesh.name, mesh)
    bpy.context.collection.objects.link(obj)
    obj.parent = root
    obj.data.materials.append(material)
    if bevel:
        mod = obj.modifiers.new('Hand softened edges', 'BEVEL')
        mod.width = bevel
        mod.segments = 1
        mod.affect = 'EDGES'
    return obj


def _box(root, name, loc, size, mat, bevel=0.05, angle=0):
    x,y,z = (v / 2 for v in size)
    obj = _mesh(root, name, [(-x,-y,-z),(x,-y,-z),(x,y,-z),(-x,y,-z),
                            (-x,-y,z),(x,-y,z),(x,y,z),(-x,y,z)],
                [(0,3,2,1),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)],mat,bevel)
    obj.location = loc
    obj.rotation_euler.z = angle
    return obj


def _lathe(root,name,profile,mat,n=12,bevel=0.025):
    # profile sequence (z, radius), closed end caps, outward normals.
    verts=[(r*math.cos(i*TAU/n),r*math.sin(i*TAU/n),z) for z,r in profile for i in range(n)]
    faces=[tuple(reversed(range(n))),tuple((len(profile)-1)*n+i for i in range(n))]
    for j in range(len(profile)-1):
        for i in range(n):
            k=(i+1)%n
            faces.append((j*n+i,j*n+k,(j+1)*n+k,(j+1)*n+i))
    return _mesh(root,name,verts,faces,mat,bevel)


def _ring(root,name,radius,tube,mat,loc=(0,0,0),rotation=(0,0,0),n=40,sides=6):
    verts=[]
    for i in range(n):
        a=TAU*i/n
        for j in range(sides):
            b=TAU*j/sides
            r=radius+tube*math.cos(b)
            verts.append((r*math.cos(a),r*math.sin(a),tube*math.sin(b)))
    faces=[]
    for i in range(n):
        for j in range(sides):
            faces.append((i*sides+j,((i+1)%n)*sides+j,((i+1)%n)*sides+(j+1)%sides,i*sides+(j+1)%sides))
    obj=_mesh(root,name,verts,faces,mat)
    obj.location=loc
    obj.rotation_euler=rotation
    return obj


def _beam(root,name,start,end,width,depth,mat,bevel=0.025):
    a,b=Vector(start),Vector(end)
    obj=_box(root,name,(a+b)/2,(width,depth,(b-a).length),mat,bevel)
    obj.rotation_euler=(b-a).to_track_quat('Z','Y').to_euler()
    return obj


def _crystal(root,name,loc,radius,height,mat):
    obj=_lathe(root,name,[(0,0.035),(height*.22,radius*.75),(height*.70,radius),(height,0.01)],mat,n=6,bevel=0)
    obj.location=loc
    return obj


def _arch(root,name,radius,thickness,depth,spring_z,mat,segments=13,center=(0,0,0),angle=0,missing=()):
    # Individual real voussoirs, inner negative space remains empty.
    for i in range(segments):
        if i in missing: continue
        a=math.pi*i/segments+.008
        b=math.pi*(i+1)/segments-.008
        vertices=[]
        for y in (-depth/2,depth/2):
            for r,t in [(radius,a),(radius,b),(radius+thickness,b),(radius+thickness,a)]:
                vertices.append((r*math.cos(t),y,spring_z+r*math.sin(t)))
        obj=_mesh(root,name+'_voussoir_%02d'%i,vertices,
                  [(3,2,1,0),(4,5,6,7),(1,5,4,0),(2,6,5,1),(3,7,6,2),(0,4,7,3)],mat,.035)
        obj.location=center
        obj.rotation_euler.z=angle


def _sanctuary(p,team):
    root=_root('sanctuary_'+team)
    glow=p[team+'_glow']
    for i,(r,z) in enumerate([(9.5,0),(8.8,.30),(8.1,.60)]):
        _lathe(root,'Terraced sandstone dais %d'%i,[(z,r),(z+.24,r),(z+.32,r-.18)],p['stone_light' if i==2 else 'stone'],n=24)
    _lathe(root,'Inset garden medallion',[(.91,5.9),(.99,5.9)],p['stone_dark'],n=32)
    _ring(root,'Outer brass inlay',7.5,.055,p['brass'],(0,0,.96),n=48)
    _ring(root,'Celestial floor inlay',5.7,.07,p['brass'],(0,0,1.04),n=48)
    for i in range(12):
        a=i*TAU/12
        _box(root,'Radial inlay %02d'%i,(6.6*math.cos(a),6.6*math.sin(a),.97),(.08,1.35,.035),p['brass'],0,a-math.pi/2)
    for i in range(4):
        a=math.pi/4+i*TAU/4
        u=Vector((math.cos(a),math.sin(a),0)); v=Vector((-math.sin(a),math.cos(a),0))
        x,y=7.15*u.x,7.15*u.y
        _box(root,'Buttress foot',(x,y,1.40),(2.05,2.05,.85),p['stone_dark'],.13,a)
        _box(root,'Carved rib pedestal',(x,y,2.15),(1.65,1.65,1.35),p['stone_light'],.10,a)
        # Swept, faceted inward curving architectural rib, with tapered radial thickness.
        verts=[]
        for j in range(13):
            t=(j/12)*math.pi/2
            center=u*(7.15*math.cos(t))+Vector((0,0,2.75+11.8*math.sin(t)))
            width=.70-.26*j/12
            for du,dv in [(-width,-.43),(width,-.43),(width,.43),(-width,.43)]:
                verts.append(tuple(center+u*du+v*dv))
        faces=[(3,2,1,0),(48,49,50,51)]
        for j in range(12):
            for k in range(4): faces.append((j*4+k,j*4+(k+1)%4,(j+1)*4+(k+1)%4,(j+1)*4+k))
        _mesh(root,'Curved ivory rib %d'%i,verts,faces,p['stone_light'],.065)
        _beam(root,'Brass pillar seam',(x,y,2.7),(x*.91,y*.91,6.2),.10,.16,p['brass'])
        _crystal(root,'Pedestal votive',(x,y,2.85),.28,1.05,glow)
    _lathe(root,'Center octagonal altar',[(.99,2.1),(1.35,2.1),(1.48,1.65),(2.00,1.6),(2.18,1.85)],p['stone_light'],n=8)
    _ring(root,'Altar brass collar',1.72,.1,p['brass'],(0,0,1.96),n=24)
    _crystal(root,'Heart of concord',(0,0,5.0),1.55,5.8,glow)
    _crystal(root,'Heart dark inset',(0,-.82,5.6),.50,3.5,p['teal'])
    _ring(root,'Armillary meridian',3.25,.13,p['brass'],(0,0,8.4),(math.pi/2,.24,.3),n=48)
    _ring(root,'Armillary equator',3.5,.105,p['brass'],(0,0,8.4),(.3,-.30,0),n=48)
    _ring(root,'Armillary ecliptic',3.05,.07,glow,(0,0,8.4),(1.15,.7,.8),n=40)
    _lathe(root,'Rib keystone',[(14.25,.9),(14.8,1.15),(15.05,.75)],p['brass'],n=8)
    _crystal(root,'Crown pinnacle',(0,0,15.05),.65,2.9,glow)
    for i in range(4):
        a=i*TAU/4
        _crystal(root,'Orbital shard',(2.4*math.cos(a),2.4*math.sin(a),7.3+(.3 if i%2 else 0)),.28,1.3,glow)
    return root


def _watchtower(p,team):
    root=_root('watchtower_'+team); glow=p[team+'_glow']
    _lathe(root,'Broad chamfered foundation',[(0,3),(.30,3),(.52,2.55)],p['stone_dark'],n=8,bevel=.05)
    _lathe(root,'Tapered masonry shaft',[(.50,2.36),(1.15,2.36),(1.35,1.95),(6.6,1.52),(6.95,2.0)],p['stone'],n=8,bevel=.07)
    for z,r in [(1.22,2.37),(3.65,1.81),(6.65,1.97)]:
        _lathe(root,'Dressed belt course',[(z-.10,r),(z+.10,r)],p['stone_light'],n=8)
    for i in range(4):
        a=math.pi/4+i*TAU/4
        _box(root,'Inset narrow relief',(1.83*math.cos(a),1.83*math.sin(a),4.6),(.54,.08,2.55),p['stone_dark'],.015,a-math.pi/2)
        _box(root,'Relief brass accent',(1.88*math.cos(a),1.88*math.sin(a),4.6),(.07,.06,1.55),p['brass'],.01,a-math.pi/2)
    _lathe(root,'Overhanging crown balcony',[(6.95,2),(7.18,2.52),(7.45,2.52),(7.57,2.32)],p['stone_light'],n=8)
    # Open crown has actual window negative spaces between four slender columns.
    for i in range(4):
        a=math.pi/4+i*TAU/4
        x,y=1.72*math.cos(a),1.72*math.sin(a)
        _box(root,'Open crown column',(x,y,8.40),(.60,.60,1.90),p['stone_light'],.06,a)
        _box(root,'Brass column cap',(x,y,9.35),(.77,.77,.20),p['brass'],.035,a)
    _lathe(root,'Crown canopy',[(9.35,2.15),(9.6,2.15),(10.1,1.25),(10.23,1.3)],p['stone_dark'],n=8)
    _crystal(root,'Tower lantern',(0,0,7.6),.66,1.6,glow)
    _crystal(root,'Tower beacon',(0,0,10.22),.48,1.75,glow)
    _ring(root,'Floating beacon collar',.95,.08,p['brass'],(0,0,10.75),n=20)
    return root


def _bridge(p):
    root=_root('bridge')
    # Full navigable 28 x 12 deck, slightly crowned; each paving slab follows slope.
    for i in range(14):
        x=-13+(i*2)
        z=.21+.52*math.cos((x/14)*math.pi/2)
        for j in range(4):
            _box(root,'Individual deck paver',(x,-4.5+j*3,z-.17),(1.98,2.98,.34),p['stone_light' if (i+j)%3 else 'stone'],.025)
    # Thin shaped spandrel strips, genuine arched clear opening below deck.
    for side in [-1,1]:
        y=side*5.9
        verts=[]
        for i in range(25):
            x=-14+28*i/24
            top=.14+.52*math.cos((x/14)*math.pi/2)
            bottom=top-.6-1.10*(abs(x)/14)**4
            verts.extend([(x,y-.28,bottom),(x,y+.28,bottom),(x,y+.28,top),(x,y-.28,top)])
        faces=[(3,2,1,0),(96,97,98,99)]
        for j in range(24):
            for k in range(4): faces.append((4*j+k,4*j+(k+1)%4,4*(j+1)+(k+1)%4,4*(j+1)+k))
        _mesh(root,'Curved bridge fascia',verts,faces,p['stone'],.03)
        for i,x in enumerate([-12,-6,0,6,12]):
            z=.21+.52*math.cos((x/14)*math.pi/2)
            _box(root,'Parapet post base',(x,y,z+.16),(.85,.85,.40),p['stone_dark'],.06)
            _box(root,'Parapet pier',(x,y,z+.72),(.58,.58,.94),p['stone_light'],.045)
            _lathe(root,'Pier crown',[(0,.52),(.16,.52),(.29,.30)],p['brass'],n=4).location=(x,y,z+1.2)
            if i<4:
                end=x+6; zend=.21+.52*math.cos((end/14)*math.pi/2)
                _beam(root,'Brass parapet rail',(x,y,z+1),(end,y,zend+1),.11,.13,p['brass'])
        for x in [-12,12]:
            _box(root,'Bridge abutment',(x,y,-.65),(3.0,1.55,1.3),p['stone_dark'],.08)
    return root


def _ruined_arch(p):
    root=_root('ruined_arch')
    for side in [-1,1]:
        x=side*4.15
        _box(root,'Broken portal footing',(x,0,.3),(2,2.8,.6),p['stone_dark'],.08)
        for j in range(4):
            _box(root,'Portal ashlar',(x,0,.64+(j+.5)*.82),(1.15,1.7,.80),p['stone_light' if j%2 else 'stone'],.06)
        _box(root,'Portal capital',(x,0,4.0),(1.7,2.1,.38),p['stone_light'],.06)
    _arch(root,'Ancient arch',3.62,1.02,1.65,4.04,p['stone'],segments=13,missing=(1,))
    _arch(root,'Outer brass inset',4.56,.055,1.76,4.04,p['brass'],segments=13,missing=(1,))
    _box(root,'Fallen voussoir',(5.7,-.75,.40),(1.3,1.25,.75),p['stone'],.11,.35)
    _box(root,'Keystone crest',(0,-.93,8.0),(.7,.18,1.0),p['brass'],.04)
    return root


def _lantern(p):
    root=_root('lantern')
    _lathe(root,'Octagonal foot',[(0,.60),(.18,.60),(.35,.40)],p['stone_dark'],n=8)
    _lathe(root,'Fluted lantern standard',[(.35,.20),(2.05,.13),(2.17,.40)],p['brass'],n=8)
    _lathe(root,'Lantern sill',[(2.14,.47),(2.28,.47)],p['stone_light'],n=6)
    for i in range(4):
        a=math.pi/4+i*TAU/4
        _beam(root,'Cage stem',(.29*math.cos(a),.29*math.sin(a),2.28),(.29*math.cos(a),.29*math.sin(a),2.91),.055,.055,p['brass'])
    _crystal(root,'Warm green votive',(0,0,2.32),.22,.53,p['green_glow'])
    _lathe(root,'Pagoda lantern cap',[(2.91,.50),(3.02,.50),(3.29,.10)],p['stone_dark'],n=6)
    return root


def _banner(p,team):
    root=_root('banner_'+team)
    _lathe(root,'Banner socket',[(0,.6),(.2,.6),(.45,.35)],p['stone'],n=8)
    _lathe(root,'Bronze flagstaff',[(.44,.095),(5.0,.07)],p['brass'],n=8)
    _crystal(root,'Finial',(0,0,4.95),.17,.46,p[team+'_glow'])
    _beam(root,'Banner crossbar',(-.15,0,4.65),(2.20,0,4.65),.075,.075,p['brass'])
    # Sculpted double-sided fabric: subtle wind curl, scalloped/forked lower edge.
    verts=[]; nx,nz=12,10
    for j in range(nz+1):
        for i in range(nx+1):
            u=i/nx; v=j/nz
            x=.20+1.85*u
            y=.17*math.sin(u*math.pi*2+.45)*(.25+.75*v)+.26*u*u
            z=4.56-2.25*v+.42*v**7*abs(2*u-1)
            verts.append((x,y,z))
    faces=[]
    for j in range(nz):
        for i in range(nx):
            a=j*(nx+1)+i
            faces.append((a,a+1,a+nx+2,a+nx+1))
    clothmat=p['teal'] if team=='blue' else p['leaf_dark']
    cloth=_mesh(root,'Wind sculpted pennant',verts,faces,clothmat)
    solid=cloth.modifiers.new('Woven thickness','SOLIDIFY'); solid.thickness=.022
    for x in [.24,2.01]:
        _beam(root,'Hem gilding',(x,.12,2.75),(x,.10,4.5),.035,.026,p['brass'],.005)
    # Readable heraldic diamond, suspended a few centimeters in front of cloth.
    _mesh(root,'Concord diamond sigil',[(1.125,-.075,4.04),(1.60,-.075,3.58),(1.125,-.075,3.12),(.65,-.075,3.58)],[(0,1,2,3)],p[team+'_glow'])
    return root


def _ruin_wall(p):
    root=_root('ruin_wall')
    for layer in range(4):
        count=5 if layer<2 else (4 if layer==2 else 2)
        for i in range(count):
            x=-3.2+i*1.6+(.35 if layer%2 else 0)
            _box(root,'Weathered ashlar',(x,0,.39+layer*.73),(1.55,1.18,.70),p['stone' if (i+layer)%3 else 'stone_light'],.08)
    for x,z in [(3.7,.28),(2.8,.22)]:
        _box(root,'Tumbled masonry',(x,-1.0,z),(1.1,.80,.45),p['stone'],.07,.55)
    _box(root,'Remnant carved frieze',(-2.5,-.61,2.65),(2.1,.08,.18),p['brass'],.02)
    return root


def build_library(palette):
    """Return 10 named EMPTY roots with local child geometry; do not save/export."""
    roots=[]
    for team in ('green','blue'):
        roots.append(_sanctuary(palette,team))
        roots.append(_watchtower(palette,team))
        roots.append(_banner(palette,team))
    roots += [_bridge(palette),_ruined_arch(palette),_lantern(palette),_ruin_wall(palette)]
    return {obj.name:obj for obj in roots}
