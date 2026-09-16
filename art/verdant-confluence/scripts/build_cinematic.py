"""Add an editable cinematic rig to an already opened source scene, saving a copy.

blender -b verdant-confluence.blend --python scripts/build_cinematic.py
"""
import hashlib
import json
from pathlib import Path

import bpy


ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "cinematic"
PLAN = OUT / "shots.json"


def keyframe(object_, field, values):
    for frame, value in values:
        setattr(object_, field, value)
        object_.keyframe_insert(data_path=field, frame=frame)


def main():
    plan = json.loads(PLAN.read_text())
    source = Path(bpy.data.filepath)
    source_hash = hashlib.sha256(source.read_bytes()).hexdigest()
    scene = bpy.context.scene
    if bpy.data.collections.get("04 CINEMATIC / animated presentation"):
        raise RuntimeError("Open the original source scene before rebuilding the rig")
    collection = bpy.data.collections.new("04 CINEMATIC / animated presentation")
    scene.collection.children.link(collection)
    scene.timeline_markers.clear()
    scene.render.engine = "BLENDER_EEVEE"
    scene.eevee.taa_render_samples = 32
    scene.render.resolution_x = plan["width"]
    scene.render.resolution_y = plan["height"]
    scene.render.resolution_percentage = 100
    scene.render.fps = plan["fps"]
    scene.render.fps_base = 1
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGB"
    scene.render.image_settings.color_depth = "8"
    scene.render.image_settings.compression = 15
    scene.render.film_transparent = False
    scene.render.use_file_extension = True
    scene.render.use_compositing = False
    scene.render.use_sequencer = False
    scene.render.use_motion_blur = False
    scene.view_settings.view_transform = "AgX"
    scene.view_settings.look = "AgX - Medium High Contrast"
    scene.view_settings.exposure = 0.35
    scene.eevee.use_raytracing = True
    scene.eevee.use_fast_gi = True
    scene.eevee.fast_gi_quality = 0.5
    scene.eevee.shadow_ray_count = 3
    scene.eevee.shadow_step_count = 6
    frames = []
    cursor = 1
    for shot in plan["shots"]:
        count = round(shot["duration_seconds"] * plan["fps"])
        last = cursor + count - 1
        data = bpy.data.cameras.new("CINEMA / " + shot["id"])
        cam = bpy.data.objects.new(data.name, data)
        collection.objects.link(cam)
        data.type = shot["type"]
        data.sensor_width = 36
        data.clip_start = 0.1
        data.clip_end = 2000
        data.lens = shot.get("lens", 50)
        # A stable tracking constraint keeps the horizon level throughout each pan.
        focus = bpy.data.objects.new("FOCUS / " + shot["id"], None)
        collection.objects.link(focus)
        focus.location = shot["target"]
        focus.empty_display_size = 2
        constraint = cam.constraints.new("TRACK_TO")
        constraint.target = focus
        constraint.track_axis = "TRACK_NEGATIVE_Z"
        constraint.up_axis = "UP_Y"
        keyframe(cam, "location", [(cursor, shot["start"]), (last, shot["end"])])
        if data.type == "ORTHO":
            keyframe(data, "ortho_scale", [(cursor, shot["scale_start"]), (last, shot["scale_end"])])
        # Blender's default Bezier keys give a gentle stationary start and finish.
        marker = scene.timeline_markers.new(shot["label"], frame=cursor)
        marker.camera = cam
        frames.append({**shot, "camera": cam.name, "frame_start": cursor,
                       "frame_end": last, "frame_count": count})
        cursor = last + 1
    scene.frame_start = 1
    scene.frame_end = cursor - 1
    scene.frame_set(1)
    scene.camera = bpy.data.objects[frames[0]["camera"]]
    scene.render.filepath = "//frames/frame_"
    scene["cinematic_shots"] = json.dumps(frames)
    scene["cinematic_source_sha256"] = source_hash
    scene["cinematic_note"] = "Camera-only presentation copy; encoded film uses 0.5 second dissolves."
    bpy.context.view_layer.update()
    for screen in bpy.data.screens:
        for area in screen.areas:
            if area.type == "VIEW_3D":
                area.spaces.active.region_3d.view_perspective = "CAMERA"
                area.spaces.active.overlay.show_overlays = False
    OUT.mkdir(exist_ok=True)
    bpy.ops.wm.save_as_mainfile(filepath=str(OUT / "verdant-confluence-cinematic.blend"), compress=True)
    manifest = {"blender_version": bpy.app.version_string, "source_sha256": source_hash,
                "plan_sha256": hashlib.sha256(PLAN.read_bytes()).hexdigest(),
                "engine": scene.render.engine, "samples": scene.eevee.taa_render_samples,
                "fps": plan["fps"], "width": plan["width"], "height": plan["height"],
                "transition_seconds": plan["transition_seconds"], "shots": frames,
                "rendered_seconds": scene.frame_end / plan["fps"],
                "film_seconds": scene.frame_end / plan["fps"] - (len(frames)-1)*plan["transition_seconds"]}
    (OUT / "render-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print("CINEMATIC_READY " + json.dumps(manifest), flush=True)


if __name__ == "__main__":
    main()
