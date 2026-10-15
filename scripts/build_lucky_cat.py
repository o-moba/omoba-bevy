#!/usr/bin/env python3
"""Build the original Jade Fortune community sculpture with Blender.

/Applications/Blender.app/Contents/MacOS/Blender --background --factory-startup \
  --python scripts/build_lucky_cat.py

Writes an editable .blend, an embedded GLB with a two-second paw animation,
a cover and a manifest under assets-src/community/lucky-cat, and copies the
same GLB to client/assets/community for the explicit local demonstration.
No downloaded geometry, textures, fonts or additional Python packages.
"""
from __future__ import annotations

import hashlib
import json
import math
from pathlib import Path
import shutil

import bpy
from mathutils import Vector

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "assets-src/community/lucky-cat"
OUT.mkdir(parents=True, exist_ok=True)
bpy.context.preferences.filepaths.save_version = 0
bpy.ops.object.select_all(action="SELECT")
bpy.ops.object.delete(use_global=False)


def material(name, color, metallic=0.0, roughness=0.38):
    mat = bpy.data.materials.new(name)
    mat.diffuse_color = (*color, 1)
    mat.use_nodes = True
    shader = mat.node_tree.nodes.get("Principled BSDF")
    shader.inputs["Base Color"].default_value = (*color, 1)
    shader.inputs["Metallic"].default_value = metallic
    shader.inputs["Roughness"].default_value = roughness
    return mat


ivory = material("Warm porcelain", (0.86, 0.80, 0.62), 0.08, 0.27)
gold = material("Brushed gold", (0.72, 0.39, 0.065), 0.72, 0.28)
jade = material("Deep jade", (0.025, 0.22, 0.19), 0.25, 0.32)
pink = material("Rose clay", (0.65, 0.17, 0.18), 0.05, 0.42)
ink = material("Ink glaze", (0.012, 0.026, 0.035), 0.1, 0.35)
model = []


def finish(obj, name, mat, parent=None):
    obj.name = name
    obj.data.materials.append(mat)
    for poly in obj.data.polygons:
        poly.use_smooth = True
    if parent:
        transform = obj.matrix_world.copy()
        obj.parent = parent
        obj.matrix_world = transform
    model.append(obj)
    return obj


def ellipsoid(name, position, scale, mat, parent=None):
    bpy.ops.mesh.primitive_uv_sphere_add(segments=24, ring_count=12, location=position)
    obj = bpy.context.object
    obj.scale = scale
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    return finish(obj, name, mat, parent)


def cylinder(name, position, radius, depth, mat, vertices=48):
    bpy.ops.mesh.primitive_cylinder_add(vertices=vertices, radius=radius, depth=depth,
                                      location=position)
    obj = bpy.context.object
    bevel = obj.modifiers.new("Soft edge", "BEVEL")
    bevel.width = min(depth / 4, 0.025)
    bevel.segments = 2
    bpy.ops.object.modifier_apply(modifier=bevel.name)
    return finish(obj, name, mat)


def line(name, points, radius, mat, parent=None):
    curve = bpy.data.curves.new(name, "CURVE")
    curve.dimensions = "3D"
    curve.resolution_u = 8
    curve.bevel_depth = radius
    curve.bevel_resolution = 2
    curve.use_fill_caps = True
    spline = curve.splines.new("POLY")
    spline.points.add(len(points) - 1)
    for point, xyz in zip(spline.points, points):
        point.co = (*xyz, 1)
    obj = bpy.data.objects.new(name, curve)
    bpy.context.collection.objects.link(obj)
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)
    bpy.ops.object.convert(target="MESH")
    obj.select_set(False)
    return finish(obj, name, mat, parent)


def ear(name, side, mat, inset=False):
    x = side * 0.30
    width = 0.105 if inset else 0.19
    low = 1.62 if inset else 1.55
    top = 1.89 if inset else 1.99
    front = -0.165 if inset else -0.115
    verts = [(x-width, front, low), (x+width, front, low),
             (x+side*0.08, front+0.07, top), (x, 0.14, low)]
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(verts, [], [(0, 1, 2), (0, 3, 1), (1, 3, 2), (2, 3, 0)])
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.collection.objects.link(obj)
    return finish(obj, name, mat)


# Blender Z-up, face towards -Y; glTF exporter converts to Y-up/+Z front.
cylinder("Jade plinth", (0, 0, 0.08), 0.61, 0.16, jade)
cylinder("Gold plinth rim", (0, 0, 0.18), 0.58, 0.055, gold)
ellipsoid("Body", (0, 0.015, 0.75), (0.40, 0.31, 0.57), ivory)
ellipsoid("Left foot", (-0.24, -0.22, 0.31), (0.19, 0.22, 0.13), ivory)
ellipsoid("Right foot", (0.24, -0.22, 0.31), (0.19, 0.22, 0.13), ivory)
ellipsoid("Head", (0, -0.015, 1.46), (0.48, 0.34, 0.39), ivory)
for side in (-1, 1):
    ear(f"Ear {side}", side, ivory)
    ear(f"Rose ear {side}", side, pink, True)
    # Curved closed eyes: a friendly smile even at small gameplay scale.
    line(f"Smile eye {side}", [(side*0.195+t*0.10, -0.333, 1.49+0.032*(1-t*t))
                              for t in (-1, -.66, -.33, 0, .33, .66, 1)], .018, ink)
    ellipsoid(f"Cheek {side}", (side*.31, -.29, 1.38), (.065, .018, .035), pink)
    for row in (-1, 1):
        line(f"Whisker {side} {row}", [(side*.30, -.292, 1.39+row*.048),
                                      (side*.405, -.225, 1.40+row*.063)], .009, gold)
ellipsoid("Nose", (0, -.355, 1.39), (.040, .022, .025), pink)
line("Smile", [(-.08, -.345, 1.34), (-.04, -.359, 1.315), (0, -.363, 1.34),
               (.04, -.359, 1.315), (.08, -.345, 1.34)], .011, ink)
ellipsoid("Jade collar", (0, -.015, 1.12), (.35, .285, .065), jade)
ellipsoid("Gold bell", (0, -.325, 1.095), (.085, .060, .085), gold)
line("Bell slit", [(0, -.388, 1.06), (0, -.388, 1.10)], .009, ink)

# A gold coin held by the quiet paw. No font dependency or third-party symbol.
coin = cylinder("Fortune coin", (0, -.332, .70), .19, .045, gold)
coin.rotation_euler[0] = math.pi / 2
line("Coin diamond", [(0, -.363, .83), (.09, -.363, .70), (0, -.363, .57),
                      (-.09, -.363, .70), (0, -.363, .83)], .015, jade)
ellipsoid("Resting arm", (-.30, -.23, .85), (.16, .15, .28), ivory)
ellipsoid("Holding paw", (-.15, -.38, .70), (.15, .10, .12), ivory)
line("Curled tail", [(.30, .20, .46), (.43, .29, .56), (.47, .29, .77),
                     (.44, .23, .88)], .075, ivory)
ellipsoid("Tail tip", (.44, .23, .88), (.075, .075, .075), ivory)

# One pivot, one rotation channel; the rest of the sculpture is static.
pivot = bpy.data.objects.new("LuckyPawWave", None)
bpy.context.collection.objects.link(pivot)
pivot.location = (.39, -.035, 1.10)
model.append(pivot)
ellipsoid("Raised arm", (.43, -.03, 1.30), (.13, .14, .30), ivory, pivot)
ellipsoid("Raised paw", (.43, -.03, 1.58), (.17, .13, .19), ivory, pivot)
ellipsoid("Paw pad", (.43, -.157, 1.56), (.075, .012, .085), pink, pivot)
for x in (.365, .43, .495):
    ellipsoid("Toe pad", (x, -.146, 1.67), (.025, .012, .030), pink, pivot)
bpy.context.scene.render.fps = 24
bpy.context.scene.frame_start = 0
bpy.context.scene.frame_end = 48
for frame, angle in [(0, -12), (12, 18), (24, -12), (36, 18), (48, -12)]:
    pivot.rotation_euler[0] = math.radians(angle)
    pivot.keyframe_insert(data_path="rotation_euler", frame=frame)
pivot.animation_data.action.name = "Wave"
for layer in pivot.animation_data.action.layers:
    for strip in layer.strips:
        bag = strip.channelbag(pivot.animation_data.action_slot)
        if bag:
            for curve in bag.fcurves:
                for point in curve.keyframe_points:
                    point.interpolation = "LINEAR"
bpy.context.scene.frame_set(0)

# Collapse static pieces by material to keep the import graph and draw calls small.
for mat in (ivory, gold, jade, pink, ink):
    pieces = [obj for obj in model if obj.type == "MESH" and obj.parent is None
              and obj.data.materials[0] == mat]
    if len(pieces) < 2:
        continue
    bpy.ops.object.select_all(action="DESELECT")
    for obj in pieces:
        obj.select_set(True)
    bpy.context.view_layer.objects.active = pieces[0]
    bpy.ops.object.join()
    pieces[0].name = mat.name
    model = [obj for obj in model if obj not in pieces[1:]]

bpy.ops.object.select_all(action="DESELECT")
for obj in model:
    obj.select_set(True)
glb = OUT / "lucky-cat.glb"
bpy.ops.export_scene.gltf(filepath=str(glb), export_format="GLB", use_selection=True,
                          export_animations=True, export_animation_mode="ACTIONS",
                          export_frame_range=True, export_force_sampling=False,
                          export_skins=False, export_morph=False, export_extras=False,
                          export_cameras=False, export_lights=False)
packaged = ROOT / "client/assets/community/lucky-cat.glb"
packaged.parent.mkdir(parents=True, exist_ok=True)
shutil.copyfile(glb, packaged)

# Neutral studio preview. Camera/lights are intentionally excluded from the GLB.
scene = bpy.context.scene
scene.render.engine = "CYCLES"
scene.cycles.samples = 32
scene.cycles.device = "CPU"
scene.render.resolution_x = 768
scene.render.resolution_y = 768
scene.render.resolution_percentage = 100
scene.world.color = (.12, .12, .12)
scene.view_settings.view_transform = "AgX"


def point_at(obj, target):
    obj.rotation_euler = (Vector(target) - obj.location).to_track_quat("-Z", "Y").to_euler()


for name, pos, power, size, color in [
    ("Key", (-3, -4, 5), 450, 4, (1, .88, .72)),
    ("Fill", (4, -2, 3), 300, 3, (.6, .84, 1)),
    ("Rim", (1, 3, 4), 500, 3, (.5, 1, .88)),
]:
    light = bpy.data.lights.new(name, "AREA")
    light.energy, light.shape, light.size, light.color = power, "DISK", size, color
    obj = bpy.data.objects.new(name, light)
    scene.collection.objects.link(obj)
    obj.location = pos
    point_at(obj, (0, 0, 1))
camera = bpy.data.objects.new("Preview camera", bpy.data.cameras.new("Preview camera"))
scene.collection.objects.link(camera)
camera.location = (3.2, -6, 3.0)
point_at(camera, (0, 0, 1))
camera.data.type = "ORTHO"
camera.data.ortho_scale = 2.65
scene.camera = camera
bpy.ops.mesh.primitive_plane_add(size=200, location=(0, 0, -.01))
floor = bpy.context.object
floor.name = "Preview floor (not exported)"
floor.data.materials.append(material("Preview background", (.025, .055, .065), 0, .65))
scene.render.filepath = str(OUT / "cover.png")
bpy.ops.wm.save_as_mainfile(filepath=str(OUT / "lucky-cat.blend"))
bpy.ops.render.render(write_still=True)
data = glb.read_bytes()
(OUT / "manifest.json").write_text(json.dumps({
    "name": "Jade Fortune", "assetKind": "map_prop", "profile": "map-prop-glb-v1",
    "license": "CC-BY-4.0", "credit": "Jade Fortune — original sculpture by OMOBA contributors",
    "generator": "scripts/build_lucky_cat.py", "sha256": hashlib.sha256(data).hexdigest(),
    "sizeBytes": len(data), "animation": "Wave", "durationSeconds": 2,
    "coordinates": "Y up, +Z front, metres; foot at Y=0", "externalResources": False,
}, indent=2) + "\n")
print(f"LUCKY_CAT_READY {glb} {len(data)} bytes")
