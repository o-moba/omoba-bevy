"""Reproducible original low-poly skill props. Run with Blender --background --python.

No downloaded assets or textures. +Y up / +Z forward in exported glTF.
The saved .blend and this script are the editable authoring sources.
"""
from pathlib import Path
import math
import json
import struct
import bpy

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "client/assets/cosmetics/standard"
SOURCE = ROOT / "assets-src/skills"
OUTPUT.mkdir(parents=True, exist_ok=True)
SOURCE.mkdir(parents=True, exist_ok=True)
# Author in a separate scene; the script never deletes the user's objects.
original_scene = bpy.context.window.scene
scene = bpy.data.scenes.new("Standard skill props")
bpy.context.window.scene = scene


def material(name, color, emission=0.0):
    result = bpy.data.materials.new(name)
    result.diffuse_color = (*color, 1)
    result.use_nodes = True
    bsdf = next(n for n in result.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
    bsdf.inputs["Base Color"].default_value = (*color, 1)
    bsdf.inputs["Roughness"].default_value = 0.7
    bsdf.inputs["Emission Color"].default_value = (*color, 1)
    bsdf.inputs["Emission Strength"].default_value = emission
    return result


steel = material("Painted midnight steel", (0.12, 0.20, 0.26), 0.3)
gold = material("Warm brass", (0.85, 0.53, 0.16), 0.3)
orange = material("Orange warhead", (0.95, 0.30, 0.07), 0.3)
glow = material("Engine glow", (0.25, 0.85, 1.0), 1.5)


def mesh(name, kind, loc, scale, mat, rotation=(0, 0, 0)):
    if kind == "cone":
        bpy.ops.mesh.primitive_cone_add(vertices=8, radius1=1, radius2=0, depth=2)
    elif kind == "cylinder":
        bpy.ops.mesh.primitive_cylinder_add(vertices=12, radius=1, depth=2)
    else:
        bpy.ops.mesh.primitive_cube_add(size=2)
    obj = bpy.context.object
    obj.name = name
    obj.location = loc
    obj.scale = scale
    obj.rotation_euler = rotation
    obj.data.materials.append(mat)
    return obj


def export(name, objects):
    bpy.ops.object.select_all(action="DESELECT")
    for obj in objects:
        obj.select_set(True)
    bpy.context.view_layer.objects.active = objects[0]
    # Keep few meshes/draw calls, preserving the small material palette.
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    bpy.ops.object.join()
    obj = bpy.context.object
    obj.name = name
    bpy.context.scene.cursor.location = (0, 0, 0)
    bpy.ops.object.origin_set(type="ORIGIN_CURSOR")
    path = OUTPUT / f"{name}.glb"
    bpy.ops.export_scene.gltf(filepath=str(path), export_format="GLB", use_selection=True, use_active_scene=True, export_yup=True)
    data = path.read_bytes()
    length = struct.unpack_from("<I", data, 12)[0]
    gltf = json.loads(data[20:20 + length])
    assert len(gltf["scenes"]) == 1 and gltf.get("scene", 0) == 0, "Runtime requires Scene0"
    roots = gltf["scenes"][0]["nodes"]
    assert len(roots) == 1 and gltf["nodes"][roots[0]]["name"] == name, "Unexpected exported object"
    return obj


# Blender -Y becomes glTF +Z. Body and nozzle sit behind the nose.
r = []
r.append(mesh("Rocket body", "cylinder", (0, 0.10, 0), (0.22, 0.22, 0.56), steel, (math.pi / 2, 0, 0)))
r.append(mesh("Warhead", "cone", (0, -0.66, 0), (0.25, 0.25, 0.30), orange, (math.pi / 2, 0, 0)))
r.append(mesh("Collar", "cylinder", (0, -0.34, 0), (0.27, 0.27, 0.08), gold, (math.pi / 2, 0, 0)))
r.append(mesh("Engine", "cylinder", (0, 0.70, 0), (0.15, 0.15, 0.09), glow, (math.pi / 2, 0, 0)))
for a in range(4):
    angle = a * math.pi / 2
    r.append(mesh("Fin", "cube", (math.cos(angle) * 0.27, 0.48, math.sin(angle) * 0.27), (0.20, 0.22, 0.035), gold, (0, -angle, 0)))
rocket = export("rocket", r)
rocket.hide_set(True)

t = [mesh("Trap base", "cylinder", (0, 0, 0.07), (0.45, 0.45, 0.07), steel),
     mesh("Trigger", "cylinder", (0, 0, 0.18), (0.22, 0.22, 0.04), orange)]
for i in range(8):
    a = i * math.tau / 8
    t.append(mesh("Jaw tooth", "cone", (math.cos(a) * 0.36, math.sin(a) * 0.36, 0.23), (0.095, 0.095, 0.17), gold))
trap = export("trap", t)
rocket.hide_set(False)
# Source keeps both props separately visible, exported props remain at their own origin.
rocket.location.x = -1.5
trap.location.x = 1.5
bpy.data.libraries.write(str(SOURCE / "standard-skill-props.blend"), {scene})
bpy.context.window.scene = original_scene
print("Exported rocket.glb and trap.glb; saved editable Blender source")
