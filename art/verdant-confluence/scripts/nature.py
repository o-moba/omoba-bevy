"""Original sculpted low-poly nature kit for Verdant Confluence.

Run inside Blender. build_library(palette) returns origin/ground-pivot Empty
roots containing one mesh each. Materials are shared, no operators or textures.
All shape variation is deterministic; geometry is authored in meters, Z up.
"""

import math
import random

import bpy
from mathutils import Vector


class Sculpt:
    def __init__(self, palette, seed):
        self.palette = palette
        self.rng = random.Random(seed)
        self.vertices = []
        self.faces = []
        self.indices = []
        self.materials = []

    def material(self, name):
        material = self.palette[name]
        if material not in self.materials:
            self.materials.append(material)
        return self.materials.index(material)

    def mesh(self, vertices, faces, material, variations=None):
        start = len(self.vertices)
        self.vertices.extend(tuple(v) for v in vertices)
        for i, face in enumerate(faces):
            self.faces.append(tuple(start + j for j in face))
            self.indices.append(self.material(variations[i % len(variations)] if variations else material))

    def tube(self, points, radii, material="wood", sides=7, twist=0.12):
        """Tapered bent trunk, root, twig or stem with a custom faceted profile."""
        points = [Vector(p) for p in points]
        vertices, faces = [], []
        for i, point in enumerate(points):
            direction = (points[min(i + 1, len(points) - 1)] - points[max(0, i - 1)]).normalized()
            u = direction.cross(Vector((0, 1, 0))).normalized()
            if u.length < 0.01:
                u = direction.cross(Vector((1, 0, 0))).normalized()
            v = direction.cross(u).normalized()
            for j in range(sides):
                angle = 2 * math.pi * j / sides + i * twist
                flute = 1 + 0.09 * math.cos(j * 2.4)
                vertices.append(point + radii[i] * flute * (u * math.cos(angle) + v * math.sin(angle)))
        faces.append(tuple(reversed(range(sides))))
        for i in range(len(points) - 1):
            for j in range(sides):
                a, b = i * sides + j, i * sides + (j + 1) % sides
                faces.append((a, b, b + sides, a + sides))
        faces.append(tuple((len(points) - 1) * sides + j for j in range(sides)))
        self.mesh(vertices, faces, material)

    def crown(self, center, scale, material="leaf_mid", pointed=False):
        """Asymmetric sculpted foliage shell: broad ledge, broken crown, pointed tip."""
        center = Vector(center)
        n = 9
        profile = [(0.0, 0.18), (0.16, 0.86), (0.43, 1.0), (0.77, 0.72), (1.0, 0.12)]
        if pointed:
            profile = [(0.0, 0.14), (0.10, 1.0), (0.31, 0.79), (0.65, 0.43), (1.0, 0.015)]
        phase = self.rng.uniform(0, math.tau)
        irregular = [self.rng.uniform(0.80, 1.16) for _ in range(n)]
        vertices, faces, shades = [], [], []
        for ring, (height, radius) in enumerate(profile):
            for j in range(n):
                angle = phase + j * math.tau / n + ring * 0.075
                z = height + (self.rng.uniform(-0.07, 0.07) if ring not in (0, 4) else 0)
                vertices.append(center + Vector((math.cos(angle) * radius * irregular[j] * scale[0],
                                                math.sin(angle) * radius * irregular[j] * scale[1],
                                                z * scale[2])))
        faces.append(tuple(reversed(range(n))))
        shades.append("leaf_dark")
        for ring in range(len(profile) - 1):
            for j in range(n):
                a, b = ring * n + j, ring * n + (j + 1) % n
                faces.extend(((a, b, a + n), (b, b + n, a + n)))
                shade = "leaf_dark" if ring == 0 else material
                if ring >= 2 and (j + ring) % 4 == 0:
                    shade = "leaf_light"
                shades.extend((shade, shade if j % 3 else material))
        faces.append(tuple(4 * n + j for j in range(n)))
        shades.append("leaf_light")
        self.mesh(vertices, faces, material, shades)

    def blade(self, start, end, width, material="grass", arch=0.16):
        """Folded leaf with curved central ridge and a tapered pointed silhouette."""
        a, b = Vector(start), Vector(end)
        direction = b - a
        side = direction.cross(Vector((0, 0, 1))).normalized()
        if side.length < 0.01:
            side = Vector((1, 0, 0))
        middle = a.lerp(b, 0.48) + Vector((0, 0, arch))
        vertices = [a, middle + side * width, middle + Vector((0, 0, width * 0.24)),
                    middle - side * width, b]
        self.mesh(vertices, [(0, 1, 2), (0, 2, 3), (2, 1, 4), (3, 2, 4)], material)

    def rock(self, center, scale, phase=0, moss=True):
        """Eroded stratified stone with offset ledges and a fractured top."""
        n = 7
        offsets = [(0, 0, 0, 0.74), (-0.10, 0.05, 0.18, 1.0),
                   (0.02, -0.08, 0.48, 0.93), (-0.14, 0.03, 0.58, 0.81),
                   (-0.06, -0.08, 0.94, 0.60)]
        jitter = [self.rng.uniform(0.8, 1.15) for _ in range(n)]
        vertices, faces, shades = [], [], []
        for k, (ox, oy, z, radius) in enumerate(offsets):
            for j in range(n):
                angle = j * math.tau / n + phase
                vertices.append((center[0] + (ox + math.cos(angle) * radius * jitter[j]) * scale[0],
                                 center[1] + (oy + math.sin(angle) * radius * jitter[j]) * scale[1],
                                 center[2] + (z + (0.06 * math.sin(j * 3) if k == 4 else 0)) * scale[2]))
        faces.append(tuple(reversed(range(n))))
        shades.append("rock")
        for k in range(4):
            for j in range(n):
                a, b = k * n + j, k * n + (j + 1) % n
                faces.append((a, b, b + n, a + n))
                shades.append("stone_dark" if k == 2 else ("stone_light" if (j + k) % 5 == 0 else "rock"))
        top = len(vertices)
        vertices.append((center[0] - scale[0] * 0.07, center[1], center[2] + scale[2]))
        for j in range(n):
            faces.append((4 * n + j, 4 * n + (j + 1) % n, top))
            shades.append("grass" if moss and j % 3 != 1 else "rock")
        self.mesh(vertices, faces, "rock", shades)

    def finish(self, name):
        # Guarantee a true ground pivot even for oblique root/twig cross-sections.
        minimum_z = min(v[2] for v in self.vertices)
        self.vertices = [(x, y, z - minimum_z) for x, y, z in self.vertices]
        root = bpy.data.objects.new(name, None)
        bpy.context.collection.objects.link(root)
        root.empty_display_type = "PLAIN_AXES"
        root.empty_display_size = 0.5
        root["asset_family"] = "nature"
        root["origin"] = "original procedural sculpt; Verdant Confluence"
        mesh = bpy.data.meshes.new(name + "_Mesh")
        mesh.from_pydata(self.vertices, [], self.faces)
        mesh.update()
        for material in self.materials:
            mesh.materials.append(material)
        for polygon, material_index in zip(mesh.polygons, self.indices):
            polygon.material_index = material_index
            polygon.use_smooth = False
        obj = bpy.data.objects.new(name + "_Geometry", mesh)
        bpy.context.collection.objects.link(obj)
        obj.parent = root
        return root


def _broadleaf(palette, variant):
    s = Sculpt(palette, 710 + variant)
    h = (11.6, 13.6, 10.3)[variant]
    bend = ((0.9, 0.3), (-0.8, 0.5), (1.8, -0.5))[variant]
    trunk = [(0, 0, 0.18), (-0.18, 0.1, h * .20), (bend[0] * .45, bend[1], h * .43),
             (bend[0], bend[1] * 1.5, h * .65), (bend[0] * .8, bend[1], h * .85)]
    s.tube(trunk, [0.84, 0.65, 0.47, 0.29, 0.035], sides=9)
    for j in range(7):
        a = j * math.tau / 7 + variant
        length = 1.7 + s.rng.random() * 0.8
        s.tube([(math.cos(a) * .3, math.sin(a) * .3, .95),
                (math.cos(a) * .85, math.sin(a) * .85, .28),
                (math.cos(a) * length, math.sin(a) * length, .075)], [.29, .24, .035], sides=6)
    # Each branch has its own bent bough and layered shoulder of foliage.
    for j in range(7):
        a = j * 2.39996 + variant * .85
        distance = (2.65, 3.45, 2.7)[variant] * (1 + .12 * math.sin(j * 2))
        height = h * (.53 + .055 * (j % 3))
        end = (math.cos(a) * distance + bend[0] * .5, math.sin(a) * distance, height + h * .10)
        start = (bend[0] * .45, bend[1], height - h * .20)
        middle = (end[0] * .65, end[1] * .65, height - .2)
        s.tube([start, middle, end], [.29, .18, .045], sides=7)
        s.crown((end[0], end[1], end[2] - .25), (2.2, 1.9, 2.6), "leaf_mid")
        if j % 2 == 0:
            s.crown((end[0] * .82 + .35, end[1] * .9, end[2] + .75), (1.6, 1.5, 1.7), "leaf_light")
    s.crown((bend[0] * .8, bend[1], h - 3.1), (2.5, 2.15, 3.1), "leaf_mid")
    # Small under-canopy islands make the trunk join look naturally grown.
    s.crown((bend[0] - 1, bend[1] + .2, h * .51), (1.3, 1.5, 1.5), "leaf_dark")
    return s.finish(("tree_jade_canopy", "tree_sage_elder", "tree_windswept_oak")[variant])


def _conifer(palette, variant):
    s = Sculpt(palette, 810 + variant)
    height = (14.8, 12.8)[variant]
    drift = (0.6, -0.8)[variant]
    s.tube([(0, 0, .1), (.1, 0, height * .3), (drift, .3, height * .7),
            (drift * 1.2, 0, height)], [.6, .43, .2, .025], sides=8)
    for j in range(5):
        a = math.tau * j / 5
        s.tube([(0, 0, .6), (math.cos(a), math.sin(a), .18),
                (math.cos(a) * 1.65, math.sin(a) * 1.65, .04)], [.2, .18, .02], sides=5)
    for layer in range(5):
        z = 2.2 + layer * (height - 6.3) / 4
        radius = 3.15 - layer * .48
        s.crown((drift * layer / 4, 0, z), (radius, radius * .80, 4.1),
                "leaf_dark" if layer < 2 else "leaf_mid", pointed=True)
        for j in range(3):
            a = j * math.tau / 3 + layer * 1.2
            end = (math.cos(a) * radius * .76 + drift * layer / 4, math.sin(a) * radius * .7, z + .65)
            s.tube([(drift * layer / 4, 0, z + 1), end], [.16, .035], sides=5)
    return s.finish(("tree_cypress_spire", "tree_river_pine")[variant])


def _fern(s, center, radius=1, leaves=7):
    for j in range(leaves):
        a = j * math.tau / leaves + s.rng.uniform(-.12, .12)
        direction = Vector((math.cos(a), math.sin(a), 0))
        side = Vector((-math.sin(a), math.cos(a), 0))
        origin = Vector(center)
        length = radius * s.rng.uniform(.83, 1.1)
        tip = origin + direction * length + Vector((0, 0, length * .48))
        s.tube([origin, origin + direction * length * .45 + Vector((0, 0, length * .50)), tip],
               [.022, .016, .004], "grass", sides=4)
        for k in range(1, 6):
            t = k / 7
            mid = origin + direction * length * t + Vector((0, 0, length * (.48 * t + .24 * math.sin(t * math.pi))))
            for sign in (-1, 1):
                end = mid + side * sign * length * .24 * (1 - t * .6) + direction * length * .14
                s.blade(mid, end, length * .065 * (1 - t * .5), "leaf_mid" if j % 2 else "leaf_light", arch=.015)
        s.blade(tip - direction * .2, tip + direction * .18, .09, "leaf_light", arch=.025)


def _groundcover(palette, kind):
    s = Sculpt(palette, {"fern_cluster": 901, "grass_fan": 902, "flowering_shrub": 903, "river_reeds": 904}[kind])
    if kind == "fern_cluster":
        _fern(s, (0, 0, .04), 1.2, 7)
        _fern(s, (.6, .3, .03), .7, 5)
    elif kind == "grass_fan":
        for j in range(25):
            a = s.rng.random() * math.tau
            origin = (s.rng.uniform(-.35, .35), s.rng.uniform(-.35, .35), .025)
            height = s.rng.uniform(.55, 1.35)
            s.blade(origin, (origin[0] + math.cos(a) * .7, origin[1] + math.sin(a) * .7, height),
                    .075, "grass" if j % 3 else "leaf_light", arch=.25)
    elif kind == "flowering_shrub":
        for j in range(6):
            a = j * 2.4
            x, y, z = math.cos(a) * .7, math.sin(a) * .6, .3 + (j % 3) * .28
            s.tube([(0, 0, .05), (x * .4, y * .4, z), (x, y, z + .35)], [.08, .055, .015], sides=5)
            s.crown((x, y, z), (.65, .55, .8), "leaf_mid")
        for j in range(13):
            a = j * 2.4
            center = Vector((math.cos(a) * (.55 + .18 * (j % 3)), math.sin(a) * .73, 1.0 + (j % 4) * .11))
            for k in range(5):
                angle = k * math.tau / 5
                s.blade(center, center + Vector((math.cos(angle) * .20, math.sin(angle) * .20, .04)), .085, "flower", arch=.055)
            s.tube([center + Vector((0, 0, .015)), center + Vector((0, 0, .06))],
                   [.045, .035], "brass", sides=5)
    elif kind == "river_reeds":
        for j in range(12):
            x, y = s.rng.uniform(-.7, .7), s.rng.uniform(-.6, .6)
            z = s.rng.uniform(1.5, 2.45)
            drift = s.rng.uniform(.05, .25)
            s.tube([(x, y, .02), (x + drift, y, z * .65), (x + drift * 1.4, y, z)],
                   [.025, .022, .015], "grass", sides=5)
            s.tube([(x + drift * 1.4, y, z - .28), (x + drift * 1.4, y, z + .1)], [.065, .055], "wood", sides=6)
            for sign in (-1, 1):
                s.blade((x, y, z * .3), (x + sign * .7, y + .25, z * .74), .07, "leaf_mid", arch=.2)
    return s.finish(kind)


def _rock_asset(palette, variant):
    s = Sculpt(palette, 950 + variant)
    if variant == 0:
        s.rock((-1.35, 0, 0), (2.0, 1.75, 4.5), .2)
        s.rock((1.0, .45, 0), (2.05, 1.35, 3.25), .7)
        s.rock((2.55, .2, 0), (.92, 1.20, 1.8), .2)
        s.rock((-.7, -1.2, 0), (1.45, .82, 1.05), 1.1)
        _fern(s, (.8, -.7, .04), .85, 5)
    elif variant == 1:
        s.rock((-.2, 0, 0), (2.15, 1.55, 2.5), .4)
        s.rock((1.5, .3, 0), (.62, .75, .78), .9)
    else:
        s.rock((0, 0, 0), (1.55, 1.75, 1.4), .8)
        s.rock((-.9, -.7, 0), (.65, .67, .58), .3)
        _fern(s, (.5, .15, 1.36), .55, 5)
    return s.finish(("rock_stratified_outcrop", "boulder_moss_tall", "boulder_moss_flat")[variant])


def _deadwood(palette, stump=False):
    s = Sculpt(palette, 980 + int(stump))
    if stump:
        s.tube([(0, 0, .1), (.10, .05, .7), (-.12, 0, 1.65)], [.8, .62, .48], sides=9)
        for j in range(6):
            a = j * math.tau / 6
            s.tube([(0, 0, .55), (math.cos(a) * .8, math.sin(a) * .8, .18),
                    (math.cos(a) * 1.55, math.sin(a) * 1.55, .03)], [.23, .21, .025], sides=6)
        # Exposed sapwood inset plus a dark scar, represented as real geometry.
        s.tube([(-.12, 0, 1.651), (-.12, 0, 1.669)], [.405, .40], "stone_light", sides=9)
        s.tube([(-.12, 0, 1.67), (-.12, 0, 1.675)], [.22, .22], "wood", sides=8)
        s.tube([(-.15, .05, 1.4), (-.3, .15, 2.0), (-.12, .24, 2.35)], [.20, .13, .012], sides=5)
        _fern(s, (.7, .3, .03), .6, 5)
    else:
        s.tube([(-2.6, -.2, .62), (-1.5, .08, .61), (.1, 0, .64), (1.5, .2, .74), (2.5, .32, .85)],
               [.48, .62, .58, .45, .34], sides=9)
        s.tube([(-2.607, -.2, .62), (-2.62, -.2, .62)], [.4, .40], "stone_light", sides=9)
        s.tube([(-2.621, -.2, .62), (-2.625, -.2, .62)], [.22, .22], "wood", sides=8)
        for j in range(3):
            x = -1.5 + j * 1.1
            s.tube([(x, .04, .6), (x + .15, .55, 1.2), (x + .4, .62, 1.5)], [.20, .11, .025], sides=6)
        for j in range(4):
            s.crown((-1.7 + j * .8, .06, 1.03), (.46, .32, .20), "grass")
        _fern(s, (1.4, -.45, .03), .8, 5)
    return s.finish("rooted_stump" if stump else "fallen_log")


def build_library(palette):
    """Build fourteen reusable original nature assets; caller owns placement/export."""
    roots = [_broadleaf(palette, i) for i in range(3)]
    roots.extend(_conifer(palette, i) for i in range(2))
    roots.extend(_rock_asset(palette, i) for i in range(3))
    roots.extend(_groundcover(palette, name) for name in
                 ("fern_cluster", "grass_fan", "flowering_shrub", "river_reeds"))
    roots.extend((_deadwood(palette), _deadwood(palette, stump=True)))
    return {root.name: root for root in roots}
