#!/usr/bin/env python3
"""One English desktop viewport: live pilot, full-roster casts or three stills per skill.

No synthetic damage: a local dev server runs the Combat Sandbox and the client
sends ordinary cast commands. Examples (binaries are a dev build of this tree):

    python3 scripts/capture_standard_skills.py --phases --offscreen \\
        --client-bin target/debug/client --server-bin target/debug/server \\
        --output /tmp/skills --hero stormfist
    python3 scripts/build_skill_contact_sheets.py /tmp/skills

`--phases` writes `0-idle.png` and, per skill, `<n>-<key>-1-windup.png`,
`-2-release.png` and `-3-impact.png` (or `-3-settled.png` when no damage receipt
arrived). `--offscreen` hides the window and renders to an image, so a locked
or covered desktop still gives frames. `--skillfx` / `--combat-visuals` overlay
one registry file on a private copy of the asset `config/` directory. The
output directory receives one folder per class and a merged `manifest.json`.

For a look at motion clips, `--avatar SLUG` stages the hero on another shipped
rig and `--release-at contact|SECONDS` moves the release still of skills
without a telegraph to the clip's contact time or to a fixed time after the
accepted cast. For a look at projectile bodies, `--flight` stands the target of
every unit-target ability far enough for its projectile to be in flight at the
release still and adds the basic attack: `5-basic-flight.png`, its projectile
on its way (not for a melee core, which throws nothing), and
`5-basic-impact.png`, its hit. A repeater then casts its weapon switch and adds
`6-rockets-flight.png` and `6-rockets-impact.png`, the same of its rocket
round. Every still lists the hero's projectiles and what stands for each of
them. `--interleave` orders one
basic attack as soon as the cast of a skill with a telegraph is accepted; each
still names the hero's latest accepted action and the clip that carries its
pose, so the stills show whether the telegraph kept the body. `--aim` adds, for
every modular skill, `<n>-<key>-0-aim.png`: the aim preview with the skill key
held before the cast, and `<n>-<key>-4-recast-aim.png` when the slot offers a
recast after the cast (for a recast with a gate, once the hero is in reach).
Each aim still records the preview the game drew; they are listed under
`aim_stills`, apart from the three stills of the cast.

A phase run is judged still by still against the registry and the motion library
of its asset root (`hard_problems`): the clip that carries the pose, the body and
boundary of every replicated effect, the particle and part budgets, the kind of
the impact recipe, the state visuals and the fingerprint of the registry the
client loaded. Each `capture-run.json` lists, per slot, what the stills show next
to what the row of the skill says (`slots`). `--mixed-recipe` replaces the classes
by one launch of a hero that carries four skills of four other classes, none on
its home button: identity belongs to the skill, not to the class or the key. A
skill whose area or hit the closest view does not hold is captured from farther
away, with `<n>-<key>-0-idle.png` as the idle still of that view.
"""
import argparse
import datetime
import json
import os
import math
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import time

from capture_showcase import base_env, free_address, run_until_exit, sha256, source_identity, stop

# Catalog order (shared/assets/catalog/heroes.json).
HEROES = ["warrior", "mage", "ranger", "cleric", "warden", "dawnweaver", "wildspark", "cinderforge",
          "edgeweaver", "stormfist", "veilstalker", "emberveil", "orbitwright", "riftshot", "chainkeeper",
          "frostguard", "adventurer"]
PILOT_HEROES = ["dawnweaver", "wildspark"]
# The single-frame roster run never included the Adventurer (its own harness covers it).
ROSTER_HEROES = ["chainkeeper", "frostguard", "orbitwright", "cinderforge", "edgeweaver", "stormfist",
                 "veilstalker", "emberveil", "riftshot", "warrior", "mage", "ranger", "cleric", "warden"]
SLOT_KEYS = ["q", "w", "e", "r"]
# The action slot of a basic attack (`shared::BASIC_ATTACK_ACTION_SLOT`).
BASIC_SLOT = 255
OVERLAYS = {"skillfx": "skills.skillfx", "combat_visuals": "combat_visuals.json"}
ERROR_MARKERS = ("panicked at", "does not exist", "Path not found", "STANDARD_KITS_QA stage=",
                 "Skill presentation unavailable")
# Per-still fields the merged manifest keeps (the full state stays in qa-summary.json).
STILL_FIELDS = ("file", "phase", "gate", "slot", "skill", "since_edge_secs", "animation", "profile",
                "particles", "receipt", "damage_numbers", "effect_parts", "effect_visible_parts",
                "effect_lights", "registry_profiles", "mean_pixel", "hero_pixels", "target_pixels",
                "projectiles")
AIM_PHASES = ("aim", "recast_aim")
# Stills that are not one of the three stills of a cast.
SIDE_PHASES = ("idle", "slot_idle") + AIM_PHASES
STILL_FIELDS += ("zoom", "idle_file", "changed_pixels", "hero_from_home")
MIXED_NAME = "mixed-recipe"
# Four skills of four classes, each on another button than in its own kit, on the core of
# a fifth class: a lane, a travelling body, a cone and an orbiting body, all of which hit
# the staged target.
MIXED_RECIPE = dict(hero="stormfist", skills=["dawn_ray", "winter_shard", "furnace_breath", "wandering_ember"])
REGISTRY_ROWS = 68
# Animation labels of the three base states; every other clip is labelled by its id.
BASE_LABELS = {"attack": "Attack", "cast": "Cast", "idle": "Idle"}
# A clip counts as running until this long before its end (simulated seconds).
CLIP_TAIL = 0.05
# Budgets a still may not exceed (`docs/combat-cosmetics.md`; AC13).
ACCENT_PARTICLES, IMPACT_PARTICLES, PARTICLE_SLOTS, VISIBLE_PARTS, EFFECT_LIGHTS = 8, 12, 256, 400, 2
PART_BUDGETS = (12, 18)
STATE_PARTS_MOST = 4
# The boundary an archetype draws over the replicated fields of its effect.
ARCHETYPE_SHAPES = {"traveller": ("ring", "none"), "orbiter": ("ring",), "zone": ("ring",), "prop": ("ring",),
                    "lane": ("capsule", "lane"), "sector": ("sector",), "wall": ("segment",),
                    "cage": ("pentagon",)}
# Server literals the client mirrors (`client/src/skill_presentation/geometry.rs`): the wall
# stands one unit ahead of its caster, and the cosine of the half angle of Furnace Breath.
WALL_AHEAD, FURNACE_CONE_COS = 1.0, 0.6
# State visuals, highest rank first, with the mesh parts each may have
# (`client/src/skill_presentation/status.rs`).
STATE_PARTS = {"stunned": (3,), "rooted": (4,), "parry_stance": (2,), "shielded": (2,), "marked": (1,),
               "brittle": (3,), "concussed": (1, 2, 3), "slowed": (2,), "camouflage_veil": (3,),
               "forging": (3,)}
STATE_RANK = list(STATE_PARTS)


def overlay_assets(assets, workdir, overlays):
    """A private asset root: links to `assets`, with `config/` copied and the given files replaced."""
    root = Path(workdir) / "assets"
    root.mkdir()
    for entry in sorted(Path(assets).iterdir()):
        if entry.name == "config":
            shutil.copytree(entry, root / "config")
        else:
            os.symlink(entry, root / entry.name, target_is_directory=entry.is_dir())
    for name, source in overlays.items():
        shutil.copyfile(source, root / "config" / OVERLAYS[name])
    return root


def fnv64(data):
    """FNV-1a, 64 bit: the fingerprint the client reports for the registry file it loaded."""
    value = 0xcbf29ce484222325
    for byte in data:
        value = ((value ^ byte) * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
    return value


def expectations(assets):
    """What the stills of a run are judged against: the registry and motion library of its asset root."""
    registry = (Path(assets) / "config" / OVERLAYS["skillfx"]).read_bytes()
    library = json.loads((Path(assets) / "animations" / "humanoid-motion-v1.json").read_text())
    visuals = json.loads((Path(assets) / "config" / OVERLAYS["combat_visuals"]).read_text())
    rows = json.loads(registry)
    return dict(rows=rows["skills"], basic_attacks=rows.get("basic_attacks") or {},
                projectiles=visuals.get("profiles") or {},
                fingerprint=f"{fnv64(registry):016x}", clips=library["clips"])


def basic_rounds(summary):
    """The rounds of the basic attack a flight look captured: its record and, for a repeater, the
    record of its rocket round."""
    basic = summary.get("basic") or {}
    return [(name, record) for name, record in (("basic", basic), ("rockets", basic.get("rockets"))) if record]


def basic_problems(summary, expected, stills, flat):
    """Why the stills of the basic attack are not what its row and its projectile profile describe."""
    problems = []
    row = (expected.get("basic_attacks") or {}).get(summary.get("class")) or {}
    for name, record in basic_rounds(summary):
        fired = (row.get("rockets") or row) if name == "rockets" else row
        mine = {s["phase"]: s for s in stills if s["file"] in record.get("stills", [])}
        if name == "rockets" and record.get("weapon_mode") != "Rockets":
            problems.append(f"the rocket round was captured in weapon mode {record.get('weapon_mode')}")
        flight = mine.get("flight")
        for shot in (flight or {}).get("projectiles") or []:
            if shot.get("action_slot") != BASIC_SLOT:
                continue
            if name == "rockets" and shot.get("style") != "rocket":
                problems.append(f"{flight['file']}: the rocket round threw a projectile of style {shot.get('style')}")
            # A profile that names a form is drawn as that form, whatever model it carries.
            form = ((expected.get("projectiles") or {}).get(shot.get("profile")) or {}).get("form")
            if form and not flat and not str(shot.get("body")).startswith(f"{form}+"):
                problems.append(f"{flight['file']}: {shot.get('profile')} is drawn as {shot.get('body')}, "
                                f"its profile says the form {form}")
        impact = mine.get("impact")
        if impact is None:
            problems.append(f"the {name} round has no still of its hit")
            continue
        receipt = (impact.get("receipt") or {}).get("id")
        look = next((l for l in impact.get("receipt_looks") or [] if l["receipt"] == receipt), None)
        wanted = (fired.get("impact") or {}).get("kind")
        if look is None:
            problems.append(f"{impact['file']}: the receipt of the still was not drawn")
        elif look["impact"] != wanted:
            problems.append(f"{impact['file']}: the hit is drawn as {look['impact']}, the row of the "
                            f"{name} round says {wanted}")
        elif look["particles"] > IMPACT_PARTICLES:
            problems.append(f"{impact['file']}: {look['particles']} impact particles, at most {IMPACT_PARTICLES}")
    return problems


def label(clip):
    return BASE_LABELS.get(clip, clip)


def expected_label(row, clips, still):
    """The animation label a windup or release still has to show, or None when nothing is known:
    the clip of the cast has run out, or another accepted action has taken the body."""
    if still.get("phase") not in ("windup", "release"):
        return None
    if row.get("windup"):
        # A held windup stands until its telegraph fires, whatever is accepted meanwhile;
        # the release starts when it fires, a moment before its still.
        return label(row["windup"] if still["phase"] == "windup" else row["release"])
    if (still.get("latest_action") or {}).get("slot") != still.get("slot"):
        return None
    clip, motion = clips.get(row["release"]), row.get("motion") or {}
    after = still.get("since_edge_secs")
    if clip is None or after is None:
        return label(row["release"])
    runs_for = clip["duration"] * (1 - motion.get("start", 0)) / motion.get("rate", 1)
    return label(row["release"]) if after < runs_for - CLIP_TAIL else None


def close(a, b, slack=1e-3):
    return len(a) == len(b) and all(abs(x - y) <= slack for x, y in zip(a, b))


def boundary_problems(body, effect):
    """What of the boundary a body drew differs from the replicated fields of its effect."""
    boundary, engine = body["boundary"], body["engine_parts"]
    position, end, radius = effect["position"], effect["end"], effect["radius"]
    axis = (end[0] - position[0], end[1] - position[1])
    length = math.hypot(*axis)
    shape = boundary["shape"]
    if shape not in ARCHETYPE_SHAPES.get(body["archetype"], ()):
        return [f"a {shape} boundary on a {body['archetype']}"]
    problems, parts = [], None
    if shape == "ring":
        parts = 1
        if not close(boundary["center"], position) or abs(boundary["radius"] - radius) > 1e-4:
            problems.append("ring is not the replicated position and radius")
    elif shape in ("capsule", "lane"):
        parts = 2 if shape == "lane" or length <= 1e-4 else 4
        if not close(boundary["from"], position) or not close(boundary["to"], end):
            problems.append("strip is not the replicated segment")
        if abs(boundary["radius" if shape == "capsule" else "half_width"] - radius) > 1e-4:
            problems.append("strip is not as wide as the replicated radius")
    elif shape == "sector":
        parts = 8
        if not close(boundary["apex"], position) or abs(boundary["radius"] - length) > 1e-3:
            problems.append("cone is not the replicated apex and length")
        if length > 0 and not close(boundary["axis"], (axis[0] / length, axis[1] / length)):
            problems.append("cone does not point along the replicated segment")
        if abs(math.cos(boundary["half_angle"]) - FURNACE_CONE_COS) > 1e-4:
            problems.append("cone has another half angle than the server tests")
    elif shape == "segment":
        parts = 1
        if length <= 0:
            problems.append("wall without a replicated heading")
        else:
            heading = (axis[0] / length, axis[1] / length)
            centre = (position[0] + heading[0] * WALL_AHEAD, position[1] + heading[1] * WALL_AHEAD)
            side = (-heading[1] * radius, heading[0] * radius)
            wanted = sorted([[centre[0] - side[0], centre[1] - side[1]], [centre[0] + side[0], centre[1] + side[1]]])
            drawn = sorted([boundary["from"], boundary["to"]])
            if not (close(drawn[0], wanted[0]) and close(drawn[1], wanted[1])):
                problems.append("wall bar is not one unit ahead with the replicated radius to each side")
    elif shape == "pentagon":
        parts = 2 * (5 - bin(effect["consumed_segments"] & 31).count("1"))
        if not close(boundary["center"], position) or abs(boundary["radius"] - radius) > 1e-4:
            problems.append("cage is not the replicated position and radius")
    elif shape == "none":
        parts = 0
    if engine != parts:
        problems.append(f"{engine} boundary parts, expected {parts}")
    return problems


def body_problems(still, rows, flat=False):
    """Every replicated effect of the hero is drawn as the row of its skill says. The flat view
    draws effects with its own shapes: there no effect has a 3D body."""
    if flat:
        return [f"{root.get('name')}: a 3D body in the flat view"
                for root in still.get("skill_vfx") or [] if root.get("body")]
    problems = []
    roots = {root["effect_id"]: root for root in still.get("skill_vfx") or []}
    effects = {effect["id"]: effect for effect in still.get("effects") or []}
    for entry in still.get("effect_rows") or []:
        row = rows.get(entry["skill"]) or {}
        own = entry["block"] == "body"
        body = row.get("body") if own else (row.get("aux") or {}).get(entry["kind"])
        name = f"{entry['skill']} {entry['kind']}"
        if body is None:
            # The effect of a first cast has the body of its row; a secondary object the
            # row gives no body is not drawn.
            if own:
                problems.append(f"{name}: its row has no body")
            continue
        root = roots.get(entry["id"])
        if root is None or not root.get("visible"):
            problems.append(f"{name}: no visible root")
            continue
        drawn = root.get("body")
        if drawn is None:
            problems.append(f"{name}: its root is not the body of its row")
            continue
        if drawn["archetype"] != body["archetype"]:
            problems.append(f"{name}: archetype {drawn['archetype']}, the row says {body['archetype']}")
            continue
        problems += [f"{name}: {problem}" for problem in boundary_problems(drawn, effects[entry["id"]])]
        if entry["part_budget"] not in PART_BUDGETS or root["mesh_parts"] > entry["part_budget"]:
            problems.append(f"{name}: {root['mesh_parts']} mesh parts, budget {entry['part_budget']}")
    return problems


def state_problems(still, flat):
    """A hero shows the mesh visual of the highest state its replicated flags report, and no other."""
    problems, shown = [], {}
    flags = {entry["hero"]: entry["states"] for entry in still.get("hero_states") or []}
    for visual in still.get("state_visuals") or []:
        state, hero = visual["state"], visual["hero"]
        if hero in shown:
            problems.append(f"two state visuals on hero {hero}")
        shown[hero] = state
        if flags.get(hero, [None])[:1] != [state]:
            problems.append(f"state visual {state} on hero {hero}, its flags report {flags.get(hero)}")
        if visual["parts"] not in STATE_PARTS.get(state, ()) or visual["parts"] > STATE_PARTS_MOST:
            problems.append(f"state visual {state} has {visual['parts']} parts")
        if visual["visible_parts"] != visual["parts"]:
            problems.append(f"state visual {state} draws {visual['visible_parts']} of {visual['parts']} parts")
    for hero, states in flags.items():
        if any(state not in STATE_RANK for state in states) or states != sorted(states, key=STATE_RANK.index):
            problems.append(f"states of hero {hero} are out of rank: {states}")
        if flat and hero in shown:
            problems.append(f"a state mesh on hero {hero} in the flat view")
        if not flat and bool(states) != (hero in shown):
            problems.append(f"hero {hero} reports {states} and shows {shown.get(hero)}")
    return problems


def hard_problems(summary, expected):
    """Why the stills of a phase run are not what the registry of the run describes (empty list = fine)."""
    problems = []
    rows = expected["rows"]
    registry = summary.get("registry") or {}
    if registry.get("origin") != "packaged":
        problems.append(f"the client drew from the {registry.get('origin')} registry, not from the packaged file")
    elif registry.get("fnv64") != expected["fingerprint"]:
        problems.append(f"the client loaded registry {registry.get('fnv64')}, the asset root has {expected['fingerprint']}")
    if registry.get("profiles") != REGISTRY_ROWS or len(rows) != REGISTRY_ROWS:
        problems.append(f"the registry has {registry.get('profiles')} rows, expected {REGISTRY_ROWS}")
    flat = str(summary.get("visual_mode", "")).lower() == "sprite2d"
    stills = [c for c in summary.get("captures", []) if c.get("phase") and c["phase"] not in ("idle", "slot_idle")]
    for still in stills:
        where = still["file"]
        found = body_problems(still, rows, flat) + state_problems(still, flat)
        particles = still.get("particles") or {}
        for value, most, what in ((particles.get("live"), PARTICLE_SLOTS, "live particles"),
                                  (still.get("effect_visible_parts"), VISIBLE_PARTS, "visible effect parts"),
                                  (still.get("effect_lights"), EFFECT_LIGHTS, "effect lights")):
            if value is None or value > most:
                found.append(f"{value} {what}, at most {most}")
        problems += [f"{where}: {problem}" for problem in found]
    for record in summary.get("skills", []):
        skill, row = record["skill"], rows.get(record["skill"])
        if row is None:
            problems.append(f"{skill}: no row in the registry")
            continue
        mine = {s["phase"]: s for s in stills if s.get("slot") == record["slot"] and s["phase"] not in AIM_PHASES}
        for phase, still in mine.items():
            # A sprite has no clip of the motion library.
            wanted = None if flat else expected_label(row, expected["clips"], still)
            if wanted is not None and still.get("animation") != wanted:
                problems.append(f"{still['file']}: the body plays {still.get('animation')}, the row of {skill} "
                                f"says {wanted}")
            # A settled still may show nothing at all; the others show the cast.
            if phase != "settled" and not (still.get("changed_pixels") or 0) > 0:
                problems.append(f"{still['file']}: the stage does not differ from {still.get('idle_file')}")
        # The third still is an impact exactly when the staged cast has to hit.
        settles = (record.get("staging") or {}).get("settles")
        third = mine.get("impact") or mine.get("settled")
        if third is not None and settles is None and third["phase"] != "impact":
            problems.append(f"{skill}: the staged cast has to hit, its third still is settled ({third.get('gate')})")
        if third is not None and settles is not None and (third["phase"], third.get("gate")) != ("settled", settles):
            problems.append(f"{skill}: the staged cast yields no receipt ({settles}), its third still is "
                            f"{third['phase']} ({third.get('gate')})")
        peaks = record.get("peaks") or {}
        seen = peaks.get("accent_of_cast")
        if (row.get("cast") or {}).get("pattern") == "none":
            if seen != 0:
                problems.append(f"{skill}: {seen} accent particles, its row draws none")
        elif seen is None or not 0 < seen <= ACCENT_PARTICLES:
            problems.append(f"{skill}: {seen} accent particles of the cast, expected 1 to {ACCENT_PARTICLES}")
        for value, most, what in ((peaks.get("impact_of_one_receipt"), IMPACT_PARTICLES, "impact particles of one receipt"),
                                  (peaks.get("live_particles"), PARTICLE_SLOTS, "live particles"),
                                  (peaks.get("effect_visible_parts"), VISIBLE_PARTS, "visible effect parts"),
                                  (peaks.get("effect_lights"), EFFECT_LIGHTS, "effect lights")):
            if value is None or value > most:
                problems.append(f"{skill}: {value} {what} at once, at most {most}")
        impact = mine.get("impact")
        if impact is not None:
            receipt = (impact.get("receipt") or {}).get("id")
            look = next((l for l in impact.get("receipt_looks") or [] if l["receipt"] == receipt), None)
            wanted = (row.get("impact") or {}).get("kind")
            if look is None:
                problems.append(f"{impact['file']}: the receipt of the still was not drawn")
            elif look["impact"] != wanted:
                problems.append(f"{impact['file']}: the hit is drawn as {look['impact']}, the row of {skill} "
                                f"says {wanted}")
            elif look["particles"] > IMPACT_PARTICLES:
                problems.append(f"{impact['file']}: {look['particles']} impact particles, at most {IMPACT_PARTICLES}")
    return problems + basic_problems(summary, expected, stills, flat)


def slot_reports(summary, expected):
    """Per slot: the clip, body and impact the stills show, next to those of the row of the skill."""
    reports = []
    stills = [c for c in summary.get("captures", []) if c.get("phase") in ("windup", "release", "impact", "settled")]
    for record in summary.get("skills", []):
        row = expected["rows"].get(record["skill"]) or {}
        mine = {s["phase"]: s for s in stills if s.get("slot") == record["slot"]}
        bodies = sorted({root["body"]["archetype"] for still in mine.values()
                         for entry in still.get("effect_rows") or []
                         if entry["skill"] == record["skill"] and entry["block"] == "body"
                         for root in still.get("skill_vfx") or []
                         if root["effect_id"] == entry["id"] and root.get("body")})
        impact = mine.get("impact") or {}
        look = next((l for l in impact.get("receipt_looks") or []
                     if l["receipt"] == (impact.get("receipt") or {}).get("id")), None)
        # The still in which the row's release clip has to be on the body, if any.
        releasing = next((s for s in (mine.get("release"), mine.get("windup"))
                          if s and expected_label(row, expected["clips"], s) == label(row.get("release"))), None)
        shown = dict(clip=(releasing or {}).get("animation"), windup_clip=(mine.get("windup") or {}).get("animation"),
                     body=bodies, impact=(look or {}).get("impact"))
        profile = dict(clip=label(row.get("release")), windup_clip=label(row["windup"]) if row.get("windup") else None,
                       body=(row.get("body") or {}).get("archetype"), impact=(row.get("impact") or {}).get("kind"))
        reports.append(dict(
            slot=record["slot"], key=SLOT_KEYS[record["slot"]], skill=record["skill"], home=record.get("home"),
            identity=record.get("identity"), shown=shown, profile=profile,
            matches=dict(clip=shown["clip"] == profile["clip"],
                         body=bodies == ([profile["body"]] if profile["body"] else []),
                         # Without a receipt there is no impact to compare.
                         impact=shown["impact"] == profile["impact"] if impact else None)))
    return reports


def phase_problems(summary, directory):
    """Why a phase run is not usable evidence (empty list = fine)."""
    problems = []
    stills = [c for c in summary.get("captures", []) if c.get("phase")]
    for still in stills:
        path = Path(directory) / still["file"]
        if not path.is_file() or path.stat().st_size <= 32:
            problems.append(f"{still['file']} is missing")
        if not (still.get("mean_pixel") or 0) > 0:
            problems.append(f"{still['file']} is black or was not read back")
    if [s["file"] for s in stills if s["phase"] == "idle"] != ["0-idle.png"]:
        problems.append("expected exactly one idle baseline")
    for slot, key in enumerate(SLOT_KEYS):
        phases = [s["phase"] for s in stills if s["phase"] not in SIDE_PHASES and s.get("slot") == slot]
        if sorted(phases) not in (["impact", "release", "windup"], ["release", "settled", "windup"]):
            problems.append(f"slot {key} has stills {phases}, expected windup, release and impact or settled")
    if len(summary.get("skills", [])) != len(SLOT_KEYS):
        problems.append("expected four skill records")
    # A slot captured from farther away has the idle still of that view, and only such a slot.
    for record in summary.get("skills", []):
        framing = record.get("framing") or {}
        taken = [s["file"] for s in stills if s["phase"] == "slot_idle" and s.get("slot") == record.get("slot")]
        if taken != ([framing.get("idle_file")] if framing.get("widened") else []):
            problems.append(f"{record.get('skill')}: slot idle stills {taken} do not match its framing")
    # An aim still belongs to the skill record that names it and carries the preview the
    # game drew; with `--aim` every modular skill has the one taken before its cast.
    for record in summary.get("skills", []):
        key = SLOT_KEYS[record["slot"]] if record.get("slot") in range(len(SLOT_KEYS)) else "?"
        taken = [s for s in stills if s["phase"] in AIM_PHASES and s.get("slot") == record.get("slot")]
        if [s["file"] for s in taken] != (record.get("aim_stills") or []):
            problems.append(f"slot {key}: aim stills {[s['file'] for s in taken]} do not match its record")
        for still in taken:
            if not ((still.get("aim") or {}).get("preview") or {}).get("shape"):
                problems.append(f"{still['file']} records no aim preview")
        if summary.get("aim") and record.get("modular") and [s["phase"] for s in taken][:1] != ["aim"]:
            problems.append(f"slot {key} has no aim still")
    # A flight look ends with the basic attack; the records of its rounds name their stills.
    taken = [s["file"] for s in stills if s.get("slot") == BASIC_SLOT]
    named = [file for _, record in basic_rounds(summary) for file in record.get("stills", [])]
    if taken != named:
        problems.append(f"basic attack stills {taken} do not match the basic attack record")
    return problems


def capture(hero, client, server, assets, output, timeout=135, roster=False, handhelds=False, sdk_weapon=None,
            phases=False, offscreen=False, visual_mode="models3d", overlays=None, avatar=None, release_at=None,
            flight=False, interleave=False, aim=False, recipe=None):
    """One client against one local server. `recipe` gives the hero four named skills on its own core."""
    output.mkdir(parents=True)
    children = []
    result = dict(hero=hero, recipe=recipe, source=source_identity(), client_sha256=sha256(client),
                  locale="en", viewport=[1280, 720], physical_device_verified=False,
                  phases=phases, offscreen=offscreen, visual_mode=visual_mode,
                  avatar=avatar, release_at=release_at, flight=flight, interleave=interleave, aim=aim)
    expected = None
    with tempfile.TemporaryDirectory(prefix="omoba-skill-pilot-") as isolated:
        if overlays:
            assets = overlay_assets(assets, isolated, overlays)
        if phases:
            # Read while the private asset root exists: the stills are judged against it.
            expected = expectations(assets)
        env = base_env(dict(size=(1280, 720), profile="desktop"), assets, isolated, output)
        host, port = free_address()
        env.update(SERVER_ADDR=f"{host}:{port}", GAME_SERVER_ADDR=f"{host}:{port}",
                   OMOBA_MATCH_MODE="dev", OMOBA_TEAM_SIZE="5", OMOBA_LANGUAGE="en",
                   OMOBA_COMBAT_SANDBOX="1", OMOBA_PLAYER_VISUAL_MODE=visual_mode)
        try:
            with (output / "server.log").open("w") as log:
                children.append(subprocess.Popen([str(server)], cwd=isolated, env=env,
                                                  stdout=log, stderr=subprocess.STDOUT))
            deadline = time.monotonic() + 20
            while "is listening" not in (output / "server.log").read_text(errors="replace"):
                if children[0].poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError("Local server did not start")
                time.sleep(.05)
            env.update(OMOBA_STANDARD_QA_DIR=str(output), OMOBA_STANDARD_QA_CLASS=hero)
            if phases:
                env["OMOBA_STANDARD_QA_PHASES"] = "1"
                if avatar:
                    env["OMOBA_STANDARD_QA_AVATAR"] = avatar
                if release_at:
                    env["OMOBA_STANDARD_QA_RELEASE_AT"] = release_at
                if flight:
                    env["OMOBA_STANDARD_QA_FLIGHT"] = "1"
                if interleave:
                    env["OMOBA_STANDARD_QA_INTERLEAVE"] = "1"
                if aim:
                    env["OMOBA_STANDARD_QA_AIM"] = "1"
                if recipe:
                    env["OMOBA_STANDARD_QA_RECIPE"] = ",".join(recipe)
            elif roster or handhelds:
                env["OMOBA_ROSTER_SKILLS_QA"] = "1"
            if offscreen:
                env["OMOBA_STANDARD_QA_OFFSCREEN"] = "1"
            if handhelds:
                env["OMOBA_HANDHELD_QA"] = "1"
                if sdk_weapon:
                    env["OMOBA_HANDHELD_QA_IMPORT"] = sdk_weapon
            with (output / "client.log").open("w") as log:
                process = subprocess.Popen([str(client)], cwd=isolated, env=env,
                                           stdout=log, stderr=subprocess.STDOUT)
                children.append(process)
            result["client_exit_code"] = run_until_exit(children, process, timeout)
            if result["client_exit_code"] is None:
                result["error"] = f"Client did not exit within {timeout:g}s"
        except Exception as error:
            result["error"] = str(error)
        finally:
            stop(children)
    summary_path = output / "qa-summary.json"
    summary = json.loads(summary_path.read_text()) if summary_path.exists() else {}
    log = (output / "client.log").read_text(errors="replace") if (output / "client.log").exists() else ""
    result["errors"] = [line for line in log.splitlines() if any(marker in line for marker in ERROR_MARKERS)]
    if phases and summary:
        result["errors"] += phase_problems(summary, output) + hard_problems(summary, expected)
        if aim and summary.get("aim") is not True:
            result["errors"].append("the client took no aim stills")
        if (summary.get("recipe") or None) != recipe:
            result["errors"].append(f"the hero carries {summary.get('recipe')}, not the recipe {recipe}")
        result["registry_profiles"] = summary.get("registry_profiles")
        result["registry"] = dict(summary.get("registry") or {}, file_fnv64=expected["fingerprint"])
        result["slots"] = slot_reports(summary, expected)
        result["skills"] = summary.get("skills", [])
        result["basic"] = summary.get("basic")
        result["stills"] = [{field: capture.get(field) for field in STILL_FIELDS}
                            for capture in summary.get("captures", [])
                            if capture.get("phase") and capture["phase"] not in AIM_PHASES]
        result["peaks"] = {record["skill"]: record.get("peaks") for record in result["skills"]}
        result["aim_stills"] = [{field: capture.get(field) for field in STILL_FIELDS + ("aim",)}
                                for capture in summary.get("captures", [])
                                if capture.get("phase") in AIM_PHASES]
    result["pass"] = (result.get("client_exit_code") == 0 and summary.get("pass") is True
                      and not result["errors"])
    (output / "capture-run.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


def merge_manifest(path, header, results):
    """Add this run's folders (a class, or the mixed recipe) to the output directory's manifest and return it."""
    manifest = json.loads(path.read_text()) if path.exists() else dict(schema_version=1, classes={})
    manifest.update(header)
    manifest["classes"].update(results)
    manifest["pass"] = all(result["pass"] for result in manifest["classes"].values())
    path.write_text(json.dumps(manifest, indent=2) + "\n")
    return manifest


def avatar_slugs(assets):
    """Slugs of the rigs shipped in an asset root."""
    manifest = json.loads((Path(assets) / "avatars" / "manifest.json").read_text())
    return [avatar["slug"] for avatar in manifest["avatars"]]


def release_time(value):
    """`contact`, or seconds after the accepted cast."""
    if value != "contact" and not 0 <= float(value) <= 2:
        raise argparse.ArgumentTypeError("expected `contact` or 0..2 seconds")
    return value


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--client-bin", type=Path, required=True)
    parser.add_argument("--server-bin", type=Path, required=True)
    parser.add_argument("--assets", type=Path, default=Path("client/assets"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--hero", action="append", choices=HEROES)
    parser.add_argument("--handhelds", action="store_true", help="Warrior: equip sword/hammer/scepter, run, attack, swap avatar, unequip")
    parser.add_argument("--roster", action="store_true", help="Capture all remaining classes, each Q/W/E/R accepted by live server")
    parser.add_argument("--phases", action="store_true",
                        help="Three state-gated stills per skill (windup, release, impact or settled); default: all 17 classes")
    parser.add_argument("--offscreen", action="store_true",
                        help="Hidden window, main camera rendered to an image (works on a locked desktop)")
    parser.add_argument("--visual-mode", choices=["models3d", "sprite2d"], default="models3d")
    parser.add_argument("--skillfx", type=Path, help="Overlay this file as config/skills.skillfx")
    parser.add_argument("--combat-visuals", type=Path, help="Overlay this file as config/combat_visuals.json")
    parser.add_argument("--avatar", help="With --phases: the hero's rig (a shipped avatar slug; default agnes)")
    parser.add_argument("--release-at", type=release_time, metavar="contact|SECONDS",
                        help="With --phases: take the release still of a skill without a telegraph at its "
                             "clip's contact time, or this long after the accepted cast (default: 0.10 s)")
    parser.add_argument("--flight", action="store_true",
                        help="With --phases: unit-target abilities are cast from lane distance and the basic "
                             "attack gets a still of its projectile in flight and one of its hit (a "
                             "repeater also of its rocket round)")
    parser.add_argument("--interleave", action="store_true",
                        help="With --phases: a basic attack is ordered as soon as the cast of a skill "
                             "with a telegraph is accepted")
    parser.add_argument("--aim", action="store_true",
                        help="With --phases: one more still per modular skill with its key held, showing "
                             "the aim preview, and one in the recast window of a slot that offers one")
    parser.add_argument("--mixed-recipe", action="store_true",
                        help=f"With --phases: instead of the classes, one launch of a {MIXED_RECIPE['hero']} that "
                             f"carries {', '.join(MIXED_RECIPE['skills'])} (folder `{MIXED_NAME}`)")
    parser.add_argument("--timeout", type=float,
                        help="Per-client deadline in seconds (default: 135, or 300 with --phases)")
    args = parser.parse_args(argv)
    if args.phases and (args.roster or args.handhelds):
        parser.error("--phases replaces --roster and --handhelds")
    if ((args.avatar or args.release_at or args.flight or args.interleave or args.aim or args.mixed_recipe)
            and not args.phases):
        parser.error("--avatar, --release-at, --flight, --interleave, --aim and --mixed-recipe need --phases")
    if args.mixed_recipe and args.hero:
        parser.error("--mixed-recipe names its own hero")
    if args.timeout is None:
        args.timeout = 300 if args.phases else 135
    if not 0 < args.timeout <= 600:
        parser.error("--timeout must be between 0 and 600 seconds")
    if args.phases and args.timeout < 300:
        parser.error("--phases needs --timeout of at least 300 seconds (the harness stops itself at 280)")
    client, server, assets, output = [p.resolve() for p in
                                    (args.client_bin, args.server_bin, args.assets, args.output)]
    if not client.is_file() or not server.is_file() or not assets.is_dir():
        parser.error("Binaries and assets must exist")
    overlays = {name: path.resolve() for name, path in
                (("skillfx", args.skillfx), ("combat_visuals", args.combat_visuals)) if path}
    for path in overlays.values():
        if not path.is_file():
            parser.error(f"Overlay file not found: {path}")
    if args.avatar and args.avatar not in avatar_slugs(assets):
        parser.error(f"Unknown avatar: {args.avatar}")
    # Folder of the run and the class of its hero.
    if args.handhelds:
        heroes = ["warrior"]
    elif args.mixed_recipe:
        heroes = [MIXED_NAME]
    elif args.phases:
        heroes = args.hero or HEROES
    else:
        heroes = args.hero or (ROSTER_HEROES if args.roster else PILOT_HEROES)
        if "adventurer" in heroes:
            parser.error("adventurer needs --phases (the single-frame runs do not cover it)")
    if any((output / hero).exists() for hero in heroes):
        parser.error("Use a new output directory to preserve evidence")
    output.mkdir(parents=True, exist_ok=True)
    # The shared cargo cache can be rebuilt by another worktree during a long
    # run: every class of this run uses the same private copies.
    with tempfile.TemporaryDirectory(prefix="binaries-", dir=output) as private:
        binaries = {name: Path(shutil.copy2(source, Path(private) / name))
                    for name, source in (("client", client), ("server", server))}
        header = dict(generator="scripts/capture_standard_skills.py",
                      captured_on=datetime.date.today().isoformat(), source=source_identity(),
                      host=dict(os=platform.platform(), machine=platform.machine()),
                      binaries={name: dict(source=str(source), sha256=sha256(binaries[name]))
                                for name, source in (("client", client), ("server", server))},
                      assets=str(assets),
                      overlays={OVERLAYS[name]: dict(path=str(path), sha256=sha256(path))
                                for name, path in overlays.items()},
                      locale="en", viewport=[1280, 720], phases=args.phases, offscreen=args.offscreen,
                      visual_mode=args.visual_mode, avatar=args.avatar, release_at=args.release_at,
                      flight=args.flight, interleave=args.interleave, aim=args.aim)
        results = {}
        for name in heroes:
            mixed = MIXED_RECIPE if name == MIXED_NAME else {}
            hero = mixed.get("hero", name)
            results[name] = capture(hero, binaries["client"], binaries["server"], assets, output / name,
                                    args.timeout, args.roster or hero not in PILOT_HEROES, args.handhelds,
                                    phases=args.phases, offscreen=args.offscreen, visual_mode=args.visual_mode,
                                    overlays=overlays, avatar=args.avatar, release_at=args.release_at,
                                    flight=args.flight, interleave=args.interleave, aim=args.aim,
                                    recipe=mixed.get("skills"))
    merge_manifest(output / "manifest.json", header, results)
    print(json.dumps([dict(run=name, **{key: result.get(key) for key in
                                        ("hero", "pass", "client_exit_code", "error", "errors")})
                      for name, result in results.items()], indent=2))
    raise SystemExit(0 if all(result["pass"] for result in results.values()) else 1)


if __name__ == "__main__":
    main()
