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
scene = bpy.data.scenes.new("Roster skill props")
bpy.context.window.scene = scene


def material(name, color, emission=0.0):
    result = bpy.data.materials.new(name)
    result.diffuse_color = (*color, 1)
    result.use_nodes = True
    # Skill props keep their painted palette under the arena's strong daylight.
    # The export step normalizes this emission surface to KHR_materials_unlit.
    result.node_tree.nodes.clear()
    surface = result.node_tree.nodes.new("ShaderNodeEmission")
    output = result.node_tree.nodes.new("ShaderNodeOutputMaterial")
    surface.inputs["Color"].default_value = (*color, 1)
    surface.inputs["Strength"].default_value = 1.0
    result.node_tree.links.new(surface.outputs["Emission"], output.inputs["Surface"])
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
    # Blender exports direct emission as a black PBR surface plus emission.
    # Convert that exact painted color to unlit without touching geometry/BIN.
    gltf.setdefault("extensionsUsed", []).append("KHR_materials_unlit")
    for mat in gltf["materials"]:
        color = mat.pop("emissiveFactor")
        mat["pbrMetallicRoughness"]["baseColorFactor"] = [*color, 1]
        mat.setdefault("extensions", {})["KHR_materials_unlit"] = {}
    encoded = json.dumps(gltf, separators=(",", ":")).encode()
    encoded += b" " * (-len(encoded) % 4)
    binary = data[20 + length:]
    path.write_bytes(struct.pack("<4sII", b"glTF", 2, 20 + len(encoded) + len(binary)) + struct.pack("<I4s", len(encoded), b"JSON") + encoded + binary)
    assert len(gltf["scenes"]) == 1 and gltf.get("scene", 0) == 0, "Runtime requires Scene0"
    roots = gltf["scenes"][0]["nodes"]
    assert len(roots) == 1 and gltf["nodes"][roots[0]]["name"] == name, "Unexpected exported object"
    return obj



cyan = material("Glacial crystal", (0.16, 0.75, 1.0), 2.2)
jade = material("Soul jade", (0.10, 1.0, 0.46), 2.0)
props = []
# Hook silhouette points toward glTF +Z, with an open crescent and brass barb.
h = []
for i in range(7):
    a = -0.5 + i * 0.48
    h.append(mesh("Hook segment", "cube", (math.cos(a)*.38, math.sin(a)*.38, 0), (.16,.08,.09), steel, (0,0,a+math.pi/2)))
h.append(mesh("Barb", "cone", (-.30,-.35,0), (.12,.12,.24), jade, (math.pi/2,0,0)))
props.append(export("hook", h))
l = [mesh("Lantern heart", "cone", (0,0,.55), (.22,.22,.38), jade), mesh("Lantern foot", "cylinder", (0,0,.10), (.38,.38,.10), gold), mesh("Lantern crown", "cone", (0,0,1.0), (.42,.42,.20), steel)]
for i in range(4):
    a=i*math.pi/2
    l.append(mesh("Lantern cage", "cube", (math.cos(a)*.29,math.sin(a)*.29,.55), (.045,.045,.43), gold))
props.append(export("lantern",l))
o=[mesh("Orb core", "cone", (0,0,0), (.32,.32,.40), cyan), mesh("Orb equator", "cylinder", (0,0,0), (.48,.48,.075), gold)]
for i in range(4):
    a=i*math.pi/2
    o.append(mesh("Orb vane", "cube", (math.cos(a)*.36,math.sin(a)*.36,0), (.075,.075,.30), steel))
props.append(export("orb",o))
p=[mesh("Basalt pillar", "cone", (0,0,.85), (.72,.65,.85), steel)]
for i in range(5):
    a=i*math.tau/5
    p.append(mesh("Magma seam", "cone", (math.cos(a)*.35,math.sin(a)*.35,.6), (.12,.12,.68), orange))
props.append(export("pillar",p))
c=[mesh("Colossus shoulders", "cube", (0,0,.62), (.9,.42,.48), steel),mesh("Colossus head", "cube", (0,-.47,.85), (.36,.32,.35), gold)]
for side in [-1,1]:
    c.append(mesh("Forge horn", "cone", (side*.6,-.30,1.02), (.20,.20,.48), orange, (.3,side*.4,0)))
    c.append(mesh("Forge runner", "cube", (side*.65,0,.13), (.21,.65,.12), orange))
props.append(export("colossus",c))
for i,prop in enumerate(props): prop.location.x=i*3
bpy.data.libraries.write(str(SOURCE / "roster-skill-props.blend"), {scene})
bpy.context.window.scene = original_scene
print("Exported five original roster props and editable source")
