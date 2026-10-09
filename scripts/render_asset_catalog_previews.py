"""Render licensed bundled handhelds for the web catalogue with Blender.

blender --background --factory-startup --python scripts/render_asset_catalog_previews.py -- --output /tmp/omoba-previews

Offline authoring utility; never rewrites source GLBs. PNG names identify source
model bytes. The exporter hashes the resulting PNG bytes independently. These
are static studio-lit model references, not native combat/attachment evidence.
"""

import argparse
import hashlib
import json
import math
from pathlib import Path
import sys

import bpy
from mathutils import Vector


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else [])
    root = Path(__file__).resolve().parents[1]
    asset_root = root / "client/assets"
    manifest = json.loads((asset_root / "weapons/manifest.json").read_text())
    args.output.mkdir(parents=True, exist_ok=True)
    records = []
    for item in manifest["items"]:
        model = (asset_root / item["model"]).resolve()
        if not model.is_relative_to(asset_root) or model.suffix != ".glb":
            raise ValueError("Only bundled GLB weapons may be rendered")
        model_hash = hashlib.sha256(model.read_bytes()).hexdigest()
        bpy.ops.wm.read_factory_settings(use_empty=True)
        bpy.ops.import_scene.gltf(filepath=str(model))
        meshes = [o for o in bpy.context.scene.objects if o.type == "MESH"]
        if not meshes:
            raise ValueError(f"No meshes in {model.name}")
        corners = [o.matrix_world @ Vector(c) for o in meshes for c in o.bound_box]
        lo = Vector(tuple(min(c[i] for c in corners) for i in range(3)))
        hi = Vector(tuple(max(c[i] for c in corners) for i in range(3)))
        center = (lo + hi) / 2
        diagonal = max((hi - lo).length, 0.01)
        camera_data = bpy.data.cameras.new("Catalogue camera")
        camera = bpy.data.objects.new("Catalogue camera", camera_data)
        bpy.context.scene.collection.objects.link(camera)
        camera.location = center + Vector((4, -6, 3)).normalized() * diagonal * 3
        camera.rotation_euler = (center - camera.location).to_track_quat("-Z", "Y").to_euler()
        camera_data.type = "ORTHO"
        inverse_rotation = camera.rotation_euler.to_matrix().transposed()
        projected = [inverse_rotation @ (c - center) for c in corners]
        width = max(c.x for c in projected) - min(c.x for c in projected)
        height = max(c.y for c in projected) - min(c.y for c in projected)
        camera_data.ortho_scale = max(width, height * 4 / 3) * 1.22
        camera_data.clip_start = 0.001
        camera_data.clip_end = max(100, diagonal * 10)
        scene = bpy.context.scene
        scene.camera = camera
        scene.render.engine = "BLENDER_WORKBENCH"
        scene.render.resolution_x = 640
        scene.render.resolution_y = 480
        scene.render.resolution_percentage = 100
        scene.render.image_settings.file_format = "PNG"
        scene.render.film_transparent = False
        shading = scene.display.shading
        shading.light = "STUDIO"
        shading.studiolight_rotate_z = math.radians(30)
        shading.color_type = "MATERIAL"
        shading.show_shadows = True
        shading.show_cavity = True
        shading.cavity_type = "BOTH"
        shading.show_specular_highlight = True
        shading.background_type = "WORLD"
        scene.world = bpy.data.worlds.new("Catalogue background")
        scene.world.color = (0.025, 0.04, 0.045)
        scene.view_settings.view_transform = "Standard"
        path = args.output / f"{model_hash}.png"
        scene.render.filepath = str(path)
        bpy.ops.render.render(write_still=True)
        records.append({"source": item["model"], "modelSha256": model_hash,
                        "preview": path.name, "previewSha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                        "width": 640, "height": 480, "license": "CC-BY-4.0",
                        "attribution": "Open Moba contributors", "renderer": bpy.app.version_string})
    (args.output / "provenance.json").write_text(json.dumps(records, indent=2) + "\n")
    print(f"Rendered {len(records)} catalogue model previews to {args.output}")


if __name__ == "__main__":
    main()
