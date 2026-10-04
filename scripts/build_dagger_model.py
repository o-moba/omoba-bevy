"""Original Adventurer dagger; isolated Blender scene, canonical handheld grip.

Blender --background --python scripts/build_dagger_model.py
Only dagger.glb and dagger.blend are written; existing armory scenes are untouched.
"""
import json
import struct
from pathlib import Path

import bpy

ROOT = Path(__file__).resolve().parents[1]
previous = bpy.context.window.scene
scene = bpy.data.scenes.new("Adventurer dagger source")
bpy.context.window.scene = scene


def material(name, color):
    mat = bpy.data.materials.new(name)
    mat.diffuse_color = (*color, 1)
    mat.use_nodes = True
    mat.node_tree.nodes.clear()
    emission = mat.node_tree.nodes.new("ShaderNodeEmission")
    emission.inputs["Color"].default_value = (*color, 1)
    output = mat.node_tree.nodes.new("ShaderNodeOutputMaterial")
    mat.node_tree.links.new(emission.outputs[0], output.inputs["Surface"])
    return mat


steel = material("Midnight steel ridge", (0.10, 0.20, 0.25))
edge = material("Bright silver cutting edge", (0.72, 0.91, 0.92))
leather = material("Burgundy leather", (0.22, 0.045, 0.055))
brass = material("Warm brass guard", (0.83, 0.53, 0.18))


def cylinder(name, radius, depth, z, mat):
    bpy.ops.mesh.primitive_cylinder_add(vertices=8, radius=radius, depth=depth)
    obj = bpy.context.object
    obj.name = name
    obj.location.z = z
    obj.data.materials.append(mat)
    return obj


objects = [cylinder("Leather grip", 0.032, 0.16, 0, leather),
           cylinder("Octagonal pommel", 0.047, 0.035, -0.095, brass)]
for z in (-0.06, -0.03, 0, 0.03, 0.06):
    objects.append(cylinder("Grip wrap", 0.035, 0.006, z, brass))
bpy.ops.mesh.primitive_cube_add(size=2)
guard = bpy.context.object
guard.name = "Compact swept guard"
guard.location.z = 0.10
guard.scale = (0.11, 0.026, 0.024)
guard.data.materials.append(brass)
objects.append(guard)
# The short diamond blade has real edge facets and a needle point, not a sword scale clone.
vertices = [(-0.055, 0, 0.13), (0, -0.023, 0.13), (0.055, 0, 0.13),
            (0, 0.023, 0.13), (-0.045, 0, 0.37), (0, -0.018, 0.37),
            (0.045, 0, 0.37), (0, 0.018, 0.37), (0, 0, 0.53)]
faces = [(0, 4, 5, 1), (1, 5, 6, 2), (2, 6, 7, 3), (3, 7, 4, 0),
         (4, 8, 5), (5, 8, 6), (6, 8, 7), (7, 8, 4), (0, 1, 2, 3)]
mesh = bpy.data.meshes.new("Diamond dagger blade")
mesh.from_pydata(vertices, [], faces)
mesh.materials.append(edge)
mesh.materials.append(steel)
blade = bpy.data.objects.new("Dagger blade", mesh)
scene.collection.objects.link(blade)
for index, polygon in enumerate(mesh.polygons):
    polygon.material_index = index % 2
objects.append(blade)
bpy.ops.object.select_all(action="DESELECT")
for obj in objects:
    obj.select_set(True)
bpy.context.view_layer.objects.active = objects[0]
bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
bpy.ops.object.join()
bpy.context.object.name = "Adventurer dagger"
scene.cursor.location = (0, 0, 0)
bpy.ops.object.origin_set(type="ORIGIN_CURSOR")
path = ROOT / "client/assets/weapons/dagger.glb"
bpy.ops.export_scene.gltf(filepath=str(path), export_format="GLB", use_selection=True,
                          use_active_scene=True, export_yup=True)
data = path.read_bytes()
size = struct.unpack_from("<I", data, 12)[0]
doc = json.loads(data[20:20 + size])
doc["asset"]["extras"] = {"ekza_handheld_v1": {
    "bone": "rightHand", "offset": [0, 0, 0], "rotation_degrees": [0, 0, 0], "scale": 1}}
doc.setdefault("extensionsUsed", []).append("KHR_materials_unlit")
for mat in doc["materials"]:
    color = mat.pop("emissiveFactor")
    mat["pbrMetallicRoughness"]["baseColorFactor"] = [*color, 1]
    mat.setdefault("extensions", {})["KHR_materials_unlit"] = {}
encoded = json.dumps(doc, separators=(",", ":")).encode()
encoded += b" " * (-len(encoded) % 4)
binary = data[20 + size:]
path.write_bytes(struct.pack("<4sII", b"glTF", 2, 20 + len(encoded) + len(binary))
                 + struct.pack("<I4s", len(encoded), b"JSON") + encoded + binary)
bpy.data.libraries.write(str(ROOT / "assets-src/weapons/dagger.blend"), {scene})
bpy.context.window.scene = previous
print("Original dagger exported: grip origin, +Y blade, +Z fingertip, metres")
