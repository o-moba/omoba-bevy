# SPDX-License-Identifier: MPL-2.0
"""Build the OMOBA low-poly art test in Blender 5 and export its assets."""

import math
import sys
from pathlib import Path

import bpy
from mathutils import Matrix, Quaternion, Vector


TEAM_COLORS = {
    "blue": ("4167DF", "718EF1"),
    "orange": ("DF5035", "F18558"),
}


def linear_rgba(hex_color):
    channels = [int(hex_color[i:i + 2], 16) / 255 for i in (0, 2, 4)]
    return tuple(c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4
                 for c in channels) + (1.0,)


def material(name, color, glow=0.14):
    mat = bpy.data.materials.new(name)
    rgba = linear_rgba(color)
    mat.diffuse_color = rgba
    mat.use_nodes = True
    shader = next(node for node in mat.node_tree.nodes if node.type == "BSDF_PRINCIPLED")
    shader.inputs["Base Color"].default_value = rgba
    shader.inputs["Roughness"].default_value = 0.84
    shader.inputs["Specular IOR Level"].default_value = 0.24
    shader.inputs["Emission Color"].default_value = rgba
    shader.inputs["Emission Strength"].default_value = glow
    return mat


def mesh_object(group, name, vertices, faces, *materials):
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    group.objects.link(obj)
    for mat in materials:
        mesh.materials.append(mat)
    return obj


def collection(scene, name):
    group = bpy.data.collections.new(name)
    scene.collection.children.link(group)
    return group


def profile(group, name, rings, *materials, facet=None):
    """Build connected, consistently oriented CCW plan rings from bottom to top."""
    count = len(rings[0])
    faces = [tuple(reversed(range(count)))]
    for level in range(len(rings) - 1):
        for i in range(count):
            low = level * count + i
            following = level * count + (i + 1) % count
            faces.append((low, following, following + count, low + count))
    faces.append(tuple(range((len(rings) - 1) * count, len(rings) * count)))
    obj = mesh_object(group, name, [point for ring in rings for point in ring], faces, *materials)
    if facet:
        for face in obj.data.polygons:
            face.material_index = facet(face.index)
    return obj


def ellipse(center, radii, sides, angle=0):
    x, y, z = center
    rx, ry = radii
    return [(x + rx * math.cos(angle + 2 * math.pi * i / sides),
             y + ry * math.sin(angle + 2 * math.pi * i / sides), z)
            for i in range(sides)]


def pebble(group, name, center, scale, mat, sides=9, angle=0):
    x, y, z = center
    rx, ry, rz = scale
    rings = [ellipse((x - rx * 0.12, y, z - rz), (rx * 0.37, ry * 0.43), sides, angle),
             ellipse((x, y, z - rz * 0.37), (rx, ry), sides, angle),
             ellipse((x + rx * 0.09, y, z + rz * 0.42), (rx * 0.84, ry * 0.84), sides, angle),
             ellipse((x + rx * 0.21, y - ry * 0.09, z + rz), (rx * 0.30, ry * 0.36), sides, angle)]
    return profile(group, name, rings, mat)


def clipped_rect(x, y, half_x, half_y, corner, z):
    return [(x + dx, y + dy, z) for dx, dy in [
        (-half_x + corner, -half_y), (half_x - corner, -half_y),
        (half_x, -half_y + corner), (half_x, half_y - corner),
        (half_x - corner, half_y), (-half_x + corner, half_y),
        (-half_x, half_y - corner), (-half_x, -half_y + corner),
    ]]


def slab(group, name, x, y, hx, hy, corner, low, high, mat):
    return profile(group, name, [clipped_rect(x, y, hx, hy, corner, z) for z in (low, high)], mat)


def arc(group, name, center, radius, width, degrees, mat):
    x, y, z = center
    start, end = map(math.radians, degrees)
    angles = [start + (end - start) * i / 20 for i in range(21)]
    vertices = [(x + r * math.cos(a), y + r * math.sin(a), z)
                for a in angles for r in (radius - width / 2, radius + width / 2)]
    return mesh_object(group, name, vertices,
                       [(i, i + 1, i + 3, i + 2) for i in range(0, 40, 2)], mat)


def structure_foundation(group, mats, name, x, radius, team):
    prefix = f"{team.title()} {name}"
    profile(group, f"{name} stone footprint • {team}",
            [ellipse((x, 0, height), (radius * factor, radius * factor), 10, math.pi / 10)
             for height, factor in ((0.035, 1.09), (0.19, 1.03), (0.27, 0.94))],
            mats["bridge_side"], mats["bridge"], facet=lambda face: int(face > 10))
    for offset in (0, 180):
        arc(group, f"{prefix} inlay", (x, 0, 0.275), radius * 0.81, 0.08,
            (18 + offset, 154 + offset), mats[team][0])


def aram_tower(group, mats, x, team, angle):
    prefix = f"{team.title()} Tower"
    cosine, sine = math.cos(angle), math.sin(angle)

    def placed(point):
        px, py, z = point
        return x + px * cosine - py * sine, px * sine + py * cosine, z

    def ring(shift, z, half_x, half_y, corner, depth=0):
        return [placed(point) for point in clipped_rect(shift, depth, half_x, half_y, corner, z)]

    def carved(name, levels, *stone, facet=None):
        return profile(group, f"{prefix} {name}", [ring(*level) for level in levels], *stone, facet=facet)

    stone, edge, dark = (mats[key] for key in ("tower_stone", "tower_edge", "tower_dark"))
    bronze, bronze_edge = mats["tower_bronze"], mats["tower_bronze_edge"]
    primary, trim = mats[team]

    carved("stepped plinth", (
        (0, 0.25, 1.21, 1.02, 0.32), (0, 0.42, 1.32, 1.12, 0.35),
        (0, 0.57, 1.19, 1.01, 0.31), (0, 0.62, 0.99, 0.86, 0.25),
    ), stone, dark, edge, facet=lambda face: 1 if face < 9 else 2 if face > 16 else 0)
    carved("tapered body", (
        (0, 0.55, 0.91, 0.79, 0.26), (0, 0.89, 0.76, 0.66, 0.24),
        (0, 1.63, 0.57, 0.49, 0.19), (0, 1.89, 0.69, 0.57, 0.20),
    ), stone, dark, facet=lambda face: int(face in (4, 7, 12, 15, 20, 23)))
    for hand in (-1, 1):
        carved("body ribs", (
            (hand * 0.77, 0.51, 0.27, 0.72, 0.15),
            (hand * 0.71, 0.75, 0.22, 0.59, 0.13),
            (hand * 0.52, 1.69, 0.145, 0.35, 0.10),
            (hand * 0.54, 1.88, 0.17, 0.37, 0.10),
        ), stone, edge, facet=lambda face: int(face in (2, 6, 10, 14, 18, 22)))

    carved("bronze seam", (
        (0, 1.82, 0.74, 0.63, 0.22), (0, 1.93, 0.91, 0.70, 0.24),
        (0, 2.05, 0.97, 0.72, 0.23),
    ), bronze, bronze_edge, facet=lambda face: int(face > 8))
    carved("crown shelf", (
        (0, 1.98, 0.96, 0.71, 0.27), (0, 2.12, 1.20, 0.79, 0.32),
        (0, 2.26, 1.09, 0.69, 0.28),
    ), stone, edge, dark, facet=lambda face: 2 if face < 9 else 1 if face > 16 else 0)

    # Angled stone wings leave a clear gap around the crystal at match-camera scale.
    for hand in (-1, 1):
        carved("swept wing", (
            (hand * 0.70, 2.21, 0.38, 0.56, 0.14),
            (hand * 0.96, 2.42, 0.36, 0.45, 0.15),
            (hand * 1.08, 2.69, 0.29, 0.34, 0.14),
            (hand * 1.03, 2.94, 0.24, 0.25, 0.11),
            (hand * 0.90, 3.14, 0.16, 0.18, 0.08),
            (hand * 0.77, 3.24, 0.09, 0.11, 0.06),
        ), stone, edge, dark,
            facet=lambda face: 1 if face in (9, 10, 18, 21, 27, 29, 35, 38, 41)
            else 2 if face in (4, 12, 20, 28) else 0)

    for front in (-1, 1):
        # A small enamel badge with a single bronze rim marks the team on both sides.
        carved("badge frame", (
            (0, 0.94, 0.10, 0.06, 0.045, front * 0.68),
            (0, 1.14, 0.30, 0.076, 0.06, front * 0.62),
            (0, 1.58, 0.30, 0.076, 0.06, front * 0.53),
            (0, 1.67, 0.24, 0.063, 0.056, front * 0.52),
        ), bronze, bronze_edge, facet=lambda face: int(face in (2, 6, 12, 15, 23, 25)))
        carved("team enamel", (
            (0, 1.06, 0.067, 0.029, 0.025, front * 0.731),
            (0, 1.21, 0.218, 0.033, 0.028, front * 0.687),
            (0, 1.56, 0.218, 0.034, 0.031, front * 0.598),
            (0, 1.60, 0.169, 0.031, 0.027, front * 0.585),
        ), primary, trim, facet=lambda face: int(face in (5, 8, 12, 15, 19, 23, 25)))

    points = [(0.055, 0.02, 2.40)]
    points += ellipse((-0.045, -0.018, 2.88), (0.49, 0.365), 6, 0.12)
    points += ellipse((0.035, 0.012, 3.16), (0.55, 0.345), 6, 0.12)
    points.append((0.11, 0.057, 3.66))
    facets = []
    for index in range(6):
        low, low_next = index + 1, (index + 1) % 6 + 1
        upper, upper_next = low + 6, low_next + 6
        facets.extend(((0, low_next, low), (low, low_next, upper_next),
                       (low, upper_next, upper), (upper, upper_next, 13)))
    gem = mesh_object(group, f"{prefix} suspended crystal", [placed(point) for point in points],
                      facets, *mats[f"{team}_gem"])
    for face in gem.data.polygons:
        face.material_index = (2, 2, 0, 0, 2, 0, 0, 1, 2, 0, 1, 0, 2, 0, 1, 1, 0, 0, 1, 0, 2, 0, 0, 1)[face.index]


def aram_structures(group, mats, side, team):
    """Place a fortified tower in front of each team's crystalline core."""
    color, trim = mats[team]
    tower_x, nexus_x = side * 11, side * 22
    bridge, edge = mats["bridge"], mats["bridge_side"]
    structure_foundation(group, mats, "Tower", tower_x, 1.63, team)
    structure_foundation(group, mats, "Nexus", nexus_x, 1.91, team)
    aram_tower(group, mats, tower_x, team, side * math.radians(12))

    prefix = f"{team.title()} Nexus"
    profile(group, f"{prefix} seat",
            [ellipse((nexus_x, 0, height), (radius, radius * 0.91), 7, math.pi / 14)
             for height, radius in ((0.27, 1.17), (0.39, 1.34), (0.57, 1.22), (0.66, 0.94))],
            trim, color, facet=lambda f: int(f > 7 and f < 15))
    profile(group, f"{prefix} crystal",
            [ellipse((nexus_x + side * shift, 0, height), radii, 7, math.pi / 14)
             for shift, height, radii in (
                 (0, 0.65, (0.53, 0.49)), (0, 1.28, (1.13, 0.91)),
                 (0.12, 1.96, (0.77, 0.73)), (0.31, 2.42, (0.18, 0.22))
             )], color, trim, facet=lambda f: int(f in (3, 10, 11, 15, 16, 18, 21, 22)))
    for angle in (math.pi * 0.12, math.pi * 0.78, math.pi * 1.45):
        dx, dy = math.cos(angle), math.sin(angle)
        profile(group, f"{prefix} cradle",
                [ellipse((nexus_x + dx * offset, dy * offset, height), (radius, radius), 5, angle)
                 for offset, height, radius in ((1.26, 0.27, 0.27), (1.37, 0.63, 0.29),
                                                (1.35, 1.13, 0.20), (1.22, 1.38, 0.08))],
                bridge, edge, facet=lambda f: int(f in (3, 8, 13)))


def arena(scene, mats, *, aram=False):
    ground = collection(scene, "01 Terrain")
    lane = collection(scene, "02 Lane and bridge")
    props = collection(scene, "03 Groves and landmarks")
    top = mats["ground"]
    board_width = 25 if aram else 19
    profile(ground, "Clipped earth board",
            [clipped_rect(0, 0, board_width - 0.4, 10.6, 2.9, -1.20),
             clipped_rect(0, 0, board_width, 11.0, 2.9, -0.80),
             clipped_rect(0, 0, board_width, 11.0, 2.9, -0.16),
             clipped_rect(0, 0, board_width - 0.17, 10.83, 2.86, 0.0)],
            mats["earth"], mats["earth_light"], top,
            facet=lambda f: 2 if f == 25 else (1 if f >= 17 else 0))
    slab(lane, "Ivory lane", 0, 0, 24.2 if aram else 17.2, 3.13, 0.76,
         -0.04, 0.035, mats["lane"])
    # Ribbon deliberately changes direction at the crossing and reaches both board edges.
    river_banks = [(-1.2, -10.77), (0.7, -10.77), (1.5, -7.9), (1.28, -5.2),
                   (0.87, -2.9), (0.91, 2.9), (1.8, 5.75), (1.5, 8.4),
                   (0.6, 10.76), (-1.32, 10.76), (-0.42, 8.16), (-0.28, 6.05),
                   (-1.02, 3.17), (-1.01, -3.12), (-0.57, -5.4), (-0.47, -7.63)]
    mesh_object(ground, "Mint river", [(x, y, 0.012) for x, y in river_banks],
                [tuple(range(len(river_banks)))], mats["water"])
    for x, y, length in [(0.18, -9.48, 0.6), (0.5, -7.2, 0.8), (0.38, -4.78, 1.04),
                         (0.47, 4.68, 0.7), (0.98, 7.32, 1.0), (0.47, 9.25, 0.62)]:
        slab(ground, "Water glint", x, y, 0.038, length / 2, 0.032, 0.013, 0.018, mats["foam"])

    slab(lane, "Bridge apron", 0.03, 0, 1.89, 3.34, 0.32, 0.039, 0.16, mats["bridge_side"])
    for index in range(5):
        slab(lane, "Bridge stone", -1.39 + index * 0.71, 0, 0.347, 3.18, 0.12,
             0.16, 0.205, mats["bridge"] if index % 2 == 0 else mats["lane"])
    for side in (-1, 1):
        slab(lane, "Bridge curb", 0.03, 3.21 * side, 1.92, 0.14, 0.10,
             0.17, 0.40, mats["bridge_side"])
        for direction in (-1, 1):
            for dx in (2.7, 3.48, 4.26):
                slab(lane, "Crossing mark", dx * direction, 2.61 * side,
                     0.19, 0.052, 0.04, 0.035, 0.041, mats["lane_marks"])

    for side, palette in [(-1, "blue"), (1, "orange")]:
        if aram:
            aram_structures(props, mats, side, palette)
            for y in (-2.55, 2.55):
                for x in (12.60, 13.26, 14.02, 18.8, 19.45):
                    slab(lane, "Side approach mark", side * x, y, 0.18, 0.055, 0.04,
                         0.035, 0.041, mats["lane_marks"])
            continue
        base_x = side * 13.7
        color, trim = mats[palette]
        profile(props, "Home • " + palette,
                [ellipse((base_x, 0, z), radii, 12, math.pi / 12)
                 for z, radii in [(0.035, (2.25, 2.25)), (0.19, (2.18, 2.18)),
                                  (0.29, (1.99, 1.99))]], mats["bridge_side"], mats["bridge"],
                facet=lambda f: int(f > 12))
        for arc_index in (0, 1):
            arc(props, "Home inlay " + palette, (base_x, 0, 0.295), 1.68, 0.13,
                (25 + 180 * arc_index, 155 + 180 * arc_index), color)
        post_x = base_x + side * 0.24
        for y in (-0.83, 0.83):
            profile(props, "Gate footing",
                    [ellipse((post_x, y, z), r, 6) for z, r in
                     [(0.28, (0.68, 0.54)), (0.46, (0.64, 0.49)), (0.59, (0.46, 0.38))]],
                    mats["bridge_side"])
            profile(props, "Gate fin • " + palette,
                    [ellipse((post_x + side * dx, y, z), r, 5, 0.22) for dx, z, r in
                     [(0, 0.51, (0.46, 0.36)), (0, 1.73, (0.55, 0.39)),
                      (0.28, 2.49, (0.23, 0.28))]],
                    color, trim, facet=lambda f: int(f in (3, 8, 11)))
        for index in range(3):
            x = side * (10.84 - index * 0.66)
            slab(props, "Home approach mark", x, 0, 0.14, 0.44 - index * 0.09,
                 0.11, 0.035, 0.043, trim)

    grove_sites = [
        (-1, -10.6, 6.5, (4.0, 2.38), 0.40), (-1, -7.8, -6.6, (3.7, 2.29), 0.34),
        (1, 9.3, 6.6, (3.7, 2.3), 0.33), (1, 11.1, -6.7, (4.0, 2.49), 0.41),
        (-1, -3.65, 7.9, (1.6, 1.22), 0.22), (1, 4.7, -7.9, (1.7, 1.20), 0.22),
    ]
    if aram:
        grove_sites = [
            (side, x * 1.27, y, radii, height) for side, x, y, radii, height in grove_sites
        ] + [(-1, -19.2, -6.8, (3.7, 2.22), 0.31), (1, 19.2, 6.8, (3.7, 2.22), 0.31)]
    for side, x, y, radii, height in grove_sites:
        rx, ry = radii
        angle = 0.17 * side
        small = rx < 2
        top_mat = mats["grove_light"] if side == -1 else mats["grove"]
        profile(props, "Grove plateau",
                [ellipse((x, y, z), (rx * f, ry * f), 8, angle)
                 for z, f in [(0, 1), (height * 0.62, 1), (height, 0.88)]],
                mats["grove_side"], top_mat, facet=lambda f: int(f > 8))
        if small:
            for dx, dy, s in [(-0.41, -0.18, 0.7), (0.31, 0.42, 0.57), (0.62, -0.39, 0.28)]:
                pebble(props, "River-bank pebble", (x + dx, y + dy, height + s * 0.19),
                       (s, s * 0.65, s * 0.28), mats["rock"], sides=6, angle=angle)
            continue
        for dx, dy, sz in [(-1.62, 0.12, 0.89), (0.32, 0.38, 1.14), (1.69, -0.35, 0.71)]:
            tree_x, tree_y = x + side * dx, y + dy
            profile(props, "Tree stem",
                    [ellipse((tree_x, tree_y, height + z * sz), (0.13 * sz, 0.13 * sz), 6)
                     for z in (0, 1.48)], mats["bark"])
            profile(props, "Canopy",
                    [ellipse((tree_x + side * offset * sz, tree_y, height + z * sz),
                             (rx * sz, ry * sz), 7, 0.16 + side * 0.12)
                     for offset, z, rx, ry in [(0, 0.95, 0.33, 0.30), (0, 1.48, 0.92, 0.78),
                                               (0.14, 2.22, 0.71, 0.62), (0.33, 2.68, 0.17, 0.20)]],
                    mats["foliage"] if side == 1 else mats["yellow_leaf"],
                    mats["leaf_light"] if side == 1 else mats["yellow_tip"],
                    facet=lambda f: int(f >= 15))
        for dx, dy, size in [(-2.35, -0.73, 0.48), (0.76, -1.12, 0.37), (2.37, 0.47, 0.58)]:
            pebble(props, "Low grove stone", (x + side * dx, y + dy, height + size * 0.14),
                   (size, size * 0.64, size * 0.26), mats["rock"], sides=6, angle=angle)

    grass_sites = [(-15.7, 5.9), (-14.0, -6.6), (-5.4, 4.91), (-3.3, -6.0), (3.05, 5.1),
                   (5.8, -5.05), (14.4, 5.5), (15.76, -4.65), (4.14, 8.55), (-7.15, 9.14)]
    if aram:
        grass_sites.extend([(-22.1, 4.7), (-17.5, 8.1), (-20.1, 5.3), (22.1, -4.7),
                            (17.2, -7.7), (21.0, -6.6)])
    for x, y in grass_sites:
        for i in range(3):
            dx, dy = [(0, 0), (0.24, 0.26), (0.50, -0.08)][i]
            pebble(props, "Sparse grass", (x + dx, y + dy, 0.12 - i * 0.025),
                   (0.075, 0.091, 0.20 - i * 0.035), mats["grove_side"], 5, 0.13)
    for x, y in [(-1.08, -5.4), (1.61, -7.83), (-1.22, 6.13), (2.04, 7.05)]:
        pebble(props, "Bank stone", (x, y, 0.12), (0.34, 0.24, 0.16), mats["rock"], 7, 0.38)
    return ground, lane, props


def bone_layout():
    bones = {
        "hips": ((0, 0, 0.77), (0, 0, 0.95), None, 0.0),
        "spine": ((0, 0, 0.95), (0, 0, 1.14), "hips", 0.0),
        "chest": ((0, 0, 1.14), (0, 0, 1.34), "spine", 0.0),
        "neck": ((0, 0, 1.34), (0, 0, 1.56), "chest", 0.0),
        "head": ((0, 0, 1.56), (0, 0, 2.14), "neck", 0.0),
    }
    for side, d in [("left", 1), ("right", -1)]:
        shoulder = (d * 0.35, 0, 1.30)
        elbow = (d * 0.72, 0, 1.30)
        wrist = (d * 1.04, 0, 1.30)
        groin = (d * 0.17, 0, 0.76)
        knee = (d * 0.23, -0.015, 0.44)
        ankle = (d * 0.245, 0.01, 0.135)
        toe = (d * 0.245, -0.185, 0.08)
        bones.update({
            side + "Shoulder": ((d * 0.13, 0, 1.30), shoulder, "chest", 0.0),
            side + "UpperArm": (shoulder, elbow, side + "Shoulder", 0.0),
            side + "LowerArm": (elbow, wrist, side + "UpperArm", 0.0),
            side + "Hand": (wrist, (d * 1.19, 0, 1.30), side + "LowerArm", 0.0),
            side + "UpperLeg": (groin, knee, "hips", 0.0),
            side + "LowerLeg": (knee, ankle, side + "UpperLeg", 0.0),
            side + "Foot": (ankle, toe, side + "LowerLeg", 0.0),
            side + "Toes": (toe, (d * 0.245, -0.27, 0.075), side + "Foot", 0.0),
        })
    return bones


def activate_scene(scene):
    bpy.context.window.scene = scene


def select_only(objects, active):
    bpy.ops.object.select_all(action="DESELECT")
    for obj in objects:
        obj.select_set(True)
    bpy.context.view_layer.objects.active = active


def segment(group, name, start, end, radius, mat, sides=8):
    a, b = Vector(start), Vector(end)
    basis = (b - a).to_track_quat("Z", "Y")
    length = (b - a).length
    rings = [[a + basis @ Vector(v) for v in ellipse((0, 0, z), (r, r), sides, math.pi / 8)]
             for z, r in [(-radius * 0.28, radius * 0.70), (radius * 0.34, radius),
                          (length - radius * 0.34, radius * 0.88),
                          (length + radius * 0.27, radius * 0.63)]]
    return profile(group, name, rings, mat)


def character(scene, group, mats):
    activate_scene(scene)
    data = bpy.data.armatures.new("Seed • VRM humanoid")
    rig = bpy.data.objects.new("Seed • VRM humanoid", data)
    group.objects.link(rig)
    rig.show_in_front = True
    data.display_type = "STICK"
    select_only((rig,), rig)
    bpy.ops.object.mode_set(mode="EDIT")
    bones = bone_layout()
    for name, (head, tail, parent, roll) in bones.items():
        bone = data.edit_bones.new(name)
        bone.head, bone.tail, bone.roll = head, tail, roll
        if parent:
            bone.parent = data.edit_bones[parent]
    bpy.ops.object.mode_set(mode="OBJECT")

    cloth, trim = mats["blue"]
    ink, cream = mats["ink"], mats["cream"]
    parts = []

    def bind(obj, bone, extra=None):
        low = obj.vertex_groups.new(name=bone)
        low.add(list(range(len(obj.data.vertices))), 1.0, "REPLACE")
        if extra:
            for name, indices in extra.items():
                joint = obj.vertex_groups.new(name=name)
                low.remove(indices)
                joint.add(indices, 1.0, "REPLACE")
        parts.append(obj)

    pelvis = pebble(group, "Pelvis", (0, 0, 0.79), (0.27, 0.19, 0.13), ink, 9)
    bind(pelvis, "hips")
    coat = profile(group, "Bell coat",
                   [ellipse((0, y, z), (rx, ry), 10, math.pi / 10)
                    for y, z, rx, ry in [(0.008, 0.82, 0.45, 0.28), (0.012, 0.92, 0.46, 0.31),
                                         (0.009, 1.19, 0.34, 0.255), (0, 1.36, 0.28, 0.21),
                                         (0, 1.41, 0.16, 0.13)]],
                   cloth, trim, facet=lambda f: int(f in (1, 2, 3, 43)))
    bind(coat, "spine", {"chest": list(range(30, 50))})
    collar = profile(group, "Collar",
                     [ellipse((0, -0.006, z), (r, r * 0.85), 9) for z, r in
                      [(1.39, 0.165), (1.48, 0.16), (1.55, 0.12)]], cream)
    bind(collar, "neck")
    hood = profile(group, "Asymmetric seed hood",
                   [ellipse((x, y, z), (rx, ry), 9, math.pi / 18)
                    for x, y, z, rx, ry in [
                        (0.025, 0.015, 1.51, 0.24, 0.23), (0.007, 0.01, 1.72, 0.46, 0.36),
                        (0.025, 0.02, 1.99, 0.47, 0.36), (0.137, 0.055, 2.22, 0.31, 0.29),
                        (0.264, 0.082, 2.36, 0.088, 0.12)]],
                   cloth, trim, facet=lambda f: int(f in (14, 23, 24, 34, 35, 37)))
    bind(hood, "head")
    mask = pebble(group, "Ink face inset", (0.024, -0.275, 1.878), (0.333, 0.118, 0.253), ink, 10)
    bind(mask, "head")
    for x, y, z, h in [(-0.078, -0.390, 1.910, 0.049), (0.117, -0.389, 1.925, 0.058)]:
        eye = pebble(group, "Cream eye", (x, y, z), (0.028, 0.019, h), cream, 7)
        bind(eye, "head")

    for side, d in [("left", 1), ("right", -1)]:
        shoulder, elbow, wrist = (bones[side + bone][0] for bone in ("UpperArm", "LowerArm", "Hand"))
        bind(pebble(group, "Shoulder cuff", (d * 0.33, 0, 1.30), (0.12, 0.11, 0.11), trim),
             side + "Shoulder")
        bind(segment(group, "Upper sleeve", shoulder, elbow, 0.105, ink),
             side + "UpperArm")
        bind(segment(group, "Forearm", elbow, wrist, 0.082, ink), side + "LowerArm")
        bind(pebble(group, "Mitten", (d * 1.108, 0, 1.30), (0.109, 0.092, 0.105), cream), side + "Hand")
        groin, knee, ankle = (bones[side + bone][0] for bone in ("UpperLeg", "LowerLeg", "Foot"))
        bind(segment(group, "Thigh", groin, knee, 0.108, ink), side + "UpperLeg")
        bind(segment(group, "Shin", knee, ankle, 0.087, ink), side + "LowerLeg")
        boot = pebble(group, "Ankle boot", (d * 0.245, -0.051, 0.118), (0.126, 0.164, 0.118), cream, 9)
        bind(boot, side + "Foot")
        toe = pebble(group, "Boot toe", (d * 0.245, -0.149, 0.093), (0.124, 0.123, 0.092), cream, 9)
        bind(toe, side + "Toes")
    select_only(parts, coat)
    bpy.ops.object.join()
    coat.name = "Seed • skinned mesh"
    coat.data.name = "Seed • flat-shaded geometry"
    coat.parent = rig
    modifier = coat.modifiers.new("Humanoid skin", "ARMATURE")
    modifier.object = rig
    return rig, coat


def relaxed_pose(rig, lean=1):
    """Make a presentation pose while keeping the asset's local rest pose a T-pose."""
    def turn(name, axis, angle):
        bone = rig.pose.bones[name]
        local_axis = bone.bone.matrix_local.to_quaternion().inverted() @ Vector(axis)
        bone.rotation_quaternion = Quaternion(local_axis, math.radians(angle))

    turn("head", (0.1, 1, 0), 7 * lean)
    turn("chest", (0, 1, 0), -4 * lean)
    for side, d in [("left", 1), ("right", -1)]:
        turn(side + "UpperArm", (0, 1, 0.20 * lean), d * (62 + d * lean * 8))
        turn(side + "LowerArm", (1, 0, 0.26 * d), (12 - d * lean * 5))
        turn(side + "UpperLeg", (0, 1, 0), -8 * d)
        turn(side + "LowerLeg", (1, 0, 0), 3 * d * lean)


def actor(group, source, name, mats, palette, location, heading, lean=1):
    source_rig, source_mesh = source
    rig = source_rig.copy()
    rig.name = name + " • rig"
    group.objects.link(rig)
    rig.location = location
    rig.rotation_euler.z = math.radians(heading)
    mesh = source_mesh.copy()
    mesh.name = name + " • mesh"
    group.objects.link(mesh)
    mesh.parent = rig
    mesh.modifiers[0].object = rig
    color_map = dict(zip(mats["blue"], mats[palette]))
    for slot in mesh.material_slots:
        if slot.material in color_map:
            replacement = color_map[slot.material]
            slot.link = "OBJECT"
            slot.material = replacement
    relaxed_pose(rig, lean)
    return rig, mesh


def camera(scene, group, name, position, target, scale):
    data = bpy.data.cameras.new(name)
    view = bpy.data.objects.new(name, data)
    group.objects.link(view)
    view.location = position
    view.rotation_euler = (Vector(target) - view.location).to_track_quat("-Z", "Y").to_euler()
    data.type = "ORTHO"
    data.ortho_scale = scale
    scene.camera = view
    return view


def light(group, name, kind, position, energy, color, target, size):
    data = bpy.data.lights.new(name, kind)
    obj = bpy.data.objects.new(name, data)
    group.objects.link(obj)
    obj.location = position
    obj.rotation_euler = (Vector(target) - obj.location).to_track_quat("-Z", "Y").to_euler()
    data.energy = energy
    data.color = linear_rgba(color)[:3]
    if kind == "SUN":
        data.angle = math.radians(size)
    else:
        data.shape = "DISK"
        data.size = size


def stage(scene, group, backdrop, arena_view=False):
    world = bpy.data.worlds.new(scene.name + " • world")
    world.use_nodes = True
    background = next(n for n in world.node_tree.nodes if n.type == "BACKGROUND")
    background.inputs["Color"].default_value = linear_rgba("E9EFE8")
    background.inputs["Strength"].default_value = 0.32
    scene.world = world
    z = -1.27 if arena_view else 0
    floor = [(-100, -100, z), (100, -100, z), (100, 100, z), (-100, 100, z)]
    mesh_object(group, "Backdrop", floor, [(0, 1, 2, 3)], backdrop)
    if arena_view:
        light(group, "Warm key", "SUN", (-12, -18, 24), 1.58, "FFF4E4", (0, 0, 0), 14)
        light(group, "Cool fill", "AREA", (12, -13, 18), 1050, "E3F2FF", (0, 0, 0), 22)
        light(group, "Rim", "AREA", (1, 18, 20), 1300, "EAF7FF", (0, 0, 0), 20)
    else:
        light(group, "Warm key", "AREA", (-3.5, -4.8, 7.5), 820, "FFF5EB", (0, 0, 1), 5)
        light(group, "Cool fill", "AREA", (5, -1.5, 4), 425, "E9F1FF", (0, 0, 1), 6)
        light(group, "Rim", "AREA", (0, 4.5, 5), 640, "FFF1DF", (0, 0, 1), 5)
    scene.render.engine = "BLENDER_EEVEE"
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGB"
    scene.render.resolution_percentage = 100
    scene.view_settings.view_transform = "AgX"
    scene.view_settings.look = "AgX - Medium High Contrast"
    scene.view_settings.exposure = 0.55


def build_aram(source, mats):
    scene = bpy.data.scenes.new("04 ARAM")
    activate_scene(scene)
    groups = arena(scene, mats, aram=True)
    units = collection(scene, "04 Teams and wave")
    for side, palette in [(-1, "blue"), (1, "orange")]:
        color, trim = mats[palette]
        for role, x, y, scale in [
            ("ADC", 8.2, -1.50, 0.94), ("Tank", 5.05, 0.0, 1.13), ("Support", 7.8, 1.52, 0.90)
        ]:
            x *= side
            name = f"ARAM {palette.title()} {role}"
            rig, _ = actor(units, source, name, mats, palette, (x, y, 0.037), -side * 44, side)
            rig.scale = (scale,) * 3
            arc(units, f"{name} ring", (x, y, 0.046), 0.76 * scale, 0.053,
                (-15, 245), color)
        for x, y in [(3.04, -0.75), (2.45, 0.14), (3.16, 1.15)]:
            pebble(units, f"ARAM {palette.title()} minion", (x * side, y, 0.35),
                   (0.29, 0.27, 0.31), trim, sides=7, angle=0.22)

    group = collection(scene, "05 Lighting and camera")
    camera(scene, group, "ARAM match camera", (7, -35, 30), (0, 0, -0.2), 56)
    stage(scene, group, mats["map_backdrop"], arena_view=True)
    scene.render.resolution_x, scene.render.resolution_y = 1600, 900
    return scene, groups


def build_tower_study(mats):
    scene = bpy.data.scenes.new("05 Towers")
    activate_scene(scene)
    group = collection(scene, "Team towers")
    for x, team, angle in [(-2.06, "blue", -24), (2.06, "orange", 24)]:
        structure_foundation(group, mats, "Tower", x, 1.63, team)
        aram_tower(group, mats, x, team, math.radians(angle))
    studio = collection(scene, "Tower study camera and lights")
    camera(scene, studio, "Tower study camera", (1.65, -12, 6.5), (0, 0, 1.76), 9.35)
    stage(scene, studio, mats["tower_backdrop"])
    scene.render.resolution_x, scene.render.resolution_y = 1440, 900
    return scene


def build():
    """Create new, isolated scenes. Existing Blender scenes are left intact."""
    map_scene = bpy.data.scenes.new("01 Arena")
    study_scene = bpy.data.scenes.new("02 Team colors")
    rig_scene = bpy.data.scenes.new("03 VRM source • T-pose")
    mats = {name: material(label, color) for name, (label, color) in {
        "ground": ("Sage field", "BCC9A0"), "earth": ("Board deep edge", "665A47"),
        "earth_light": ("Board clay edge", "A2805B"), "lane": ("Ivory path", "F1E5C9"),
        "lane_marks": ("Path inlays", "BA9D72"), "bridge": ("Bridge limestone", "FFF0CF"),
        "bridge_side": ("Bridge edge", "C1A680"), "water": ("Mint river", "74B9B9"),
        "foam": ("River glint", "B7DFD7"), "grove": ("Cool grove", "A4BF9C"),
        "grove_light": ("Warm grove", "C5C794"), "grove_side": ("Plateau edge", "889F7C"),
        "bark": ("Tree stem", "75644D"), "foliage": ("Juniper", "477C70"),
        "leaf_light": ("Juniper tips", "64A38D"), "yellow_leaf": ("Honey leaves", "B68D42"),
        "yellow_tip": ("Honey tips", "DDAD55"), "rock": ("River stone", "9AA99C"),
        "ink": ("Ink", "232B39"), "cream": ("Warm ceramic", "FFF0CC"),
        "map_backdrop": ("Arena backdrop", "D6E3D8"), "study_backdrop": ("Studio backdrop", "BCCFC4"),
        "tower_stone": ("Tower slate", "656D7B"), "tower_dark": ("Tower slate in shadow", "46505E"),
        "tower_edge": ("Tower cut stone", "A4A6AA"), "tower_bronze": ("Tower patinated bronze", "977448"),
        "tower_bronze_edge": ("Tower bronze bevel", "DDB06D"),
        "tower_backdrop": ("Tower studio backdrop", "4C5B70"),
    }.items()}
    mats["blue"] = tuple(material(name, color) for name, color in
                         zip(("Team Cloth", "Team Trim"), TEAM_COLORS["blue"]))
    mats["orange"] = tuple(material(name, color) for name, color in
                           zip(("Orange Cloth", "Orange Trim"), TEAM_COLORS["orange"]))
    for team, colors in [("blue", ("4274EF", "8BDCFF", "2543B2")),
                         ("orange", ("F65343", "FFAF79", "A62936"))]:
        mats[f"{team}_gem"] = tuple(
            material(f"{team.title()} tower crystal {facet}", value, glow=glow)
            for facet, value, glow in zip(("color", "light", "shade"), colors, (0.45, 0.60, 0.12))
        )
    rig_group = collection(rig_scene, "Humanoid source")
    source = character(rig_scene, rig_group, mats)

    activate_scene(map_scene)
    arena_groups = arena(map_scene, mats)
    teams = collection(map_scene, "04 Team samples")
    actor(teams, source, "Blue", mats, "blue", (-5.5, -1.39, 0.036), 18)
    actor(teams, source, "Orange", mats, "orange", (5.5, 1.2, 0.036), -22, -1)
    for palette, x, y in [("blue", -5.5, -1.39), ("orange", 5.5, 1.2)]:
        for offset in (0, 180):
            arc(teams, palette + " ring", (x, y, 0.047), 0.88, 0.073,
                (15 + offset, 153 + offset), mats[palette][0])
    group = collection(map_scene, "05 Lighting and camera")
    camera(map_scene, group, "Match camera", (7, -30, 26), (0, 0, -0.10), 44.1)
    stage(map_scene, group, mats["map_backdrop"], arena_view=True)
    map_scene.render.resolution_x, map_scene.render.resolution_y = 1600, 900

    activate_scene(study_scene)
    teams = collection(study_scene, "Team samples")
    blue = actor(teams, source, "Blue", mats, "blue", (-1.36, 0, 0), 13)
    actor(teams, source, "Orange", mats, "orange", (1.36, 0.18, 0), -17, -1)
    group = collection(study_scene, "Studio")
    for palette, x, y in [("blue", -1.36, 0), ("orange", 1.36, 0.18)]:
        for offset in (0, 180):
            arc(group, palette + " ring", (x, y, 0.006), 0.77, 0.060,
                (15 + offset, 153 + offset), mats[palette][0])
    camera(study_scene, group, "Team comparison camera", (0.6, -8, 3.85), (0, 0, 1.10), 5.55)
    stage(study_scene, group, mats["study_backdrop"])
    study_scene.render.resolution_x, study_scene.render.resolution_y = 1280, 832

    aram_scene, aram_groups = build_aram(source, mats)
    tower_scene = build_tower_study(mats)
    activate_scene(aram_scene)
    for area in bpy.context.window.screen.areas:
        if area.type == "VIEW_3D":
            space = area.spaces.active
            space.region_3d.view_perspective = "CAMERA"
            space.region_3d.view_camera_zoom = 12
            space.shading.type = "SOLID"
            space.shading.color_type = "MATERIAL"
            space.shading.show_cavity = True
            space.overlay.show_overlays = False
    return map_scene, study_scene, rig_scene, arena_groups, source, blue, aram_scene, aram_groups, tower_scene


def export_glb(scene, objects, path, active):
    activate_scene(scene)
    select_only(objects, active)
    bpy.ops.export_scene.gltf(filepath=str(path), export_format="GLB", use_selection=True,
                              use_active_scene=True,
                              export_animations=False, export_cameras=False, export_lights=False,
                              export_yup=True, export_apply=False)


def export_art(scenes, out):
    from tempfile import TemporaryDirectory
    from export_vrm import write_vrm

    map_scene, study_scene, rig_scene, arena_groups, source, blue = scenes[:6]
    map_objects = [obj for group in arena_groups for obj in group.objects]
    export_glb(map_scene, map_objects, out / "arena.glb", map_objects[0])
    with TemporaryDirectory(prefix="omoba-seed-") as tmp:
        raw = Path(tmp) / "character.glb"
        export_glb(rig_scene, source, raw, source[0])
        write_vrm(raw, out / "character.vrm", name="OMOBA Seed")

    activate_scene(study_scene)
    _, blue_mesh = blue
    depsgraph = bpy.context.evaluated_depsgraph_get()
    posed = bpy.data.meshes.new_from_object(blue_mesh.evaluated_get(depsgraph))
    preview = bpy.data.objects.new("Seed • posed preview", posed)
    study_scene.collection.objects.link(preview)
    # The baked mesh uses local rig space, clearing the comparison scene's placement.
    preview.matrix_world = Matrix.Identity(4)
    export_glb(study_scene, [preview], out / "character-preview.glb", preview)
    bpy.data.objects.remove(preview, do_unlink=True)
    bpy.data.meshes.remove(posed)


def main():
    out = Path(__file__).resolve().parent
    args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    scenes = build()
    map_scene, study_scene, *_ = scenes
    aram_scene, aram_groups, tower_scene = scenes[-3:]
    for scene in list(bpy.data.scenes):
        if scene not in (*scenes[:3], aram_scene, tower_scene):
            bpy.data.scenes.remove(scene)
    if "--renders-only" not in args:
        activate_scene(aram_scene)
        select_only((), None)
        bpy.context.preferences.filepaths.save_version = 0
        bpy.ops.wm.save_as_mainfile(filepath=str(out / "low-poly.blend"), compress=True)
        if "--blend-only" not in args:
            if "--aram-only" not in args:
                sys.path.insert(0, str(out))
                export_art(scenes, out)
            aram_objects = [obj for group in aram_groups for obj in group.objects]
            export_glb(aram_scene, aram_objects, out / "aram-arena.glb", aram_objects[0])
    if "--assets-only" not in args and "--blend-only" not in args:
        renders = [(aram_scene, "aram-arena.png"), (tower_scene, "tower-study.png")]
        if "--aram-only" not in args:
            renders.extend([(map_scene, "arena.png"), (study_scene, "team-colors.png")])
        for scene, name in renders:
            activate_scene(scene)
            scene.render.filepath = str(out / name)
            bpy.ops.render.render(write_still=True)


if __name__ == "__main__":
    main()
