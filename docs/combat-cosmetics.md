# Combat cosmetics

The [combat visual uplift TODO](plans/combat-visual-uplift.md) maps all 16
classes and 64 active skills to proposed animations, props and effects, with
implementation order and acceptance checks. Its required design supports future
hybrid recipes, including four ultimate abilities or different weapon attacks:
presentation belongs to the skill, independently of class and input slot. It
separates existing mechanics from planned presentation work; arbitrary slot
assignment is supported by presentation; the server loadout resolver still enforces its existing slot rules.

## Skill presentation pilot

[`config/skills.skillfx`](../client/assets/config/skills.skillfx) now owns the
64 skill presentations: all 44 modular skills plus the 20 canonical legacy
ability IDs. Legacy projectile shapes use the complementary registry below. Fields are `release`, optional `windup`,
`effect`, RGB `color`, and optional `hdr_gain` (linear brightness, 1–8,
default 3); unknown fields, skills and motion IDs are rejected.
The packaged file applies once per launch. Restart after editing it. Missing
or malformed files retain the embedded version; invalid rebuilt defaults fall
back to the previous geometric renderer.

```json
"dawn_ray": {
  "release": "cast",
  "windup": "spell_prepare",
  "effect": "beam",
  "color": [1.0, 0.72, 0.16],
  "hdr_gain": 4.0
}
```

The animation selector reads `recipe.skills[action.slot]`, then the skill
profile. It does not derive the motion from the class or default catalogue
slot. Moving Dawn Ray into a Wildspark recipe's Q is covered by a client ECS
test; this is an accepted-recipe fixture, **not** a claim that the server's
current `WrongSlot` restriction or public class editor has been removed.

| Skill | Motion | 3D presentation |
| --- | --- | --- |
| Dawn Bind | `cast` | Golden lance, streaks and team accent |
| Dawn Barrier | `interact` | Returning orb with intersecting shield rings |
| Dawn Field | `spell_finish` | Filled field, exact-radius boundary, orbiting light motes |
| Dawn Ray | `spell_prepare` → `cast` | Corridor warning → luminous beam, on server phase |
| Wild Switch | `pistol_reload` | Weapon-mode gesture; existing mode HUD |
| Wild Zap | `pistol_shoot` | Cyan bolt with streaks |
| Wild Traps | `interact` | Blender trap props, spinning while arming, radius outline |
| Wild Rocket | `pistol_shoot` | Blender rocket with exhaust streaks |

Repeater basic attacks use `pistol_shoot`. Confirmed hit particles use the
visible caster's skill color; hidden sources keep generic feedback. All skills
retain server damage, range, cooldown and movement behavior. In 2D the
existing renderer stays active.

World effects retain replicated IDs, follow received positions (including fog
clipping), and disappear silently on omission. Round and 2D-mode changes clear
instances. At most `MAX_ACTIVE_EFFECTS` objects are considered per snapshot.
Meshes and materials are shared; missing GLB assets keep a procedural fallback.
The Blender sources and deterministic exporter are in
[`assets-src/skills`](../assets-src/skills/README.md).

Ray preparation can arrive after the action and holds until the real beam.
Warning disappearance never fabricates a release. Repeated snapshots do not
restart the clip, and death/respawn take priority. Additional phase types,
body masks and playback-rate alignment are still TODO. Semantic hand sockets
now have a [Warrior handheld pilot](handheld-weapons.md); finger posing, two-handed
grips and IK remain open. Teleport/jump/fly movement policies and a generic
timeline editor are not implemented. Normal animation changes now blend over
120 ms; death, round/model changes and sandbox previews cut immediately.

Reproduce the native English 1280×720 scripted sandbox check:

```sh
python3 scripts/capture_standard_skills.py --client-bin /path/to/client --server-bin /path/to/server --assets client/assets --output /tmp/skill-pilot
```

It captures the persistent effects, ultimate warning/flight, held aim,
released Q/W projectile, and Dawn Ray release. The JSON records authoritative
effects, actual motion readouts and rendered effect entities. This is a
desktop capture, not physical mobile or manual multiplayer certification.

Pilot verification on 2026-09-30: native client/server build passed; both
English 1280×720 sandbox runs completed with exit code 0 and the expected
captures. Ray readout changed `spell_prepare` → `Cast`; all three traps and
the rocket reported `model_ready: true`. Final client SHA-256:
`06527c577f1982cecd8f844916352cbf6ee98581c1706a9bfad4f4449c8b2595`.
Local evidence is under `.agent/tasks/SKILL-PILOT-2026-09-30/final/` (ignored
work artifacts). Client tests: 806 passed, zero failed, one existing ignored.
The nine motion-export tests, thirty candidate-asset tests,
deterministic exporter check and both GLB inspections passed. No full release
package, additional viewport/language or physical mobile performance was tested.

## Full roster presentation pass

Each remaining class has four skill-owned profiles in `skills.skillfx`, with
reusable silhouettes instead of 64 independent model files. Accepted recipes
always win over class defaults. The five legacy classes resolve canonical
ability IDs when no modular recipe exists. A four-ultimate presentation fixture
covers slot independence without changing server validation.

| Class | Added presentation |
| --- | --- |
| Chainkeeper | Blender hook and lantern, green cast sweep, five cage sides with consumed-side removal; souls stay separate from hooks |
| Frostguard | Narrow ice bolt, defensive cast ring, directional crystal wall, immediately visible full-length fissure borders |
| Orbitwright | Blender sphere follows replicated position, orbital field, guard gesture, inward collapse ring |
| Cinderforge | Delayed basalt pillar, fire cone and embers, charge accent, moving Blender colossus |
| Edgeweaver | Blade slash, parry rings, second-strike accent, challenge cast pulse; existing vital markers retained |
| Stormfist | Punch motion and blue strike, anchor shield, pulse, heavy strike accent |
| Veilstalker | Violet needles, curse pulse, lash and retreat slash |
| Emberveil | Pulsing ember, wisp accent, tether lance, roll follow-through for dash |
| Riftshot | Thin blue needle, seal rings, blink-shot accent, real warning → travelling wave |
| Warrior | Warm shield accent, rally ring, separate strike/ultimate projectile profiles |
| Mage | Violet orb, resource pulse, cyan lance, orange fireball |
| Ranger | Green arrow profiles with distinct reach silhouettes/trails, dressing gesture |
| Cleric | Gold smite, renew pulse, favor rings, blessing pulse |
| Warden | Green claw, defensive ring, mark projectile, heavy maul accent |

The shared CC0 motion library has 16 clips. `punch`, `guard`, `shoulder_drive`
and `roll` come from Quaternius Punch_Cross, Punch_Enter, Punch_Jab and Roll.
The guard and heavy strike are reusable gesture fallbacks, not bespoke shield
or kick animations. They are retargeted semantically to the existing VRM rigs;
avatar GLBs are unchanged.

Instant actions emit one small cast accent from a new server-confirmed action
sequence. First-seen, duplicate, older, hidden and dead actor states do not
replay it. These accents share the existing 256-particle budget with trails;
they never fabricate damage or an impact on the target. Windup skills keep
server warnings. Auxiliary world objects choose their actual silhouette from
the replicated effect kind, not the source ability's primary silhouette.

Run the remaining 14 classes in the same desktop camera:

```sh
python3 scripts/capture_standard_skills.py --roster --client-bin /path/to/client --server-bin /path/to/server --assets client/assets --output /tmp/roster-skills
```

This uses a live development sandbox and checks accepted Q/W/E/R sequences,
PNG readbacks and actual effect entities. It resets the sandbox between skills,
uses infinite resources/cooldowns and an invulnerable stationary enemy. Ally-only
skills target the caster in this harness; this verifies their accepted cast,
not an ally-to-ally travel sequence. It does not certify recast combos,
physical mobile performance, all skins visually, or manual multiplayer play.

The [handheld pilot](handheld-weapons.md) adds sword/hammer/scepter props, semantic
VRM sockets and a shared built-in/Ekza SDK import contract. Grip/IK corrections,
dedicated bow/kick clips and a generic server-driven jump/flight timeline remain
separate tasks. No synthetic flight
or new cast delay is added to the currently instantaneous movement mechanics.

### Facing the accepted attack

The server records optional `action_yaw` alongside the accepted action sequence,
using the basic attack target or the skill's resolved target/aim. Locomotion yaw
stays separate. Rejected casts cannot change it; self casts clear the previous
direction. Older packets without this optional field remain readable.

In 3D, both local and remote heroes face that direction after movement and
network interpolation, before transform propagation: 450 ms for basic attacks,
700 ms for skills. An authoritative Dawn Ray/Horizon Wave warning keeps the
preparation facing active until release. This changes rotation only, respects
sandbox visual time, and does not steer movement or change damage. Initial and
duplicate snapshots do not replay it; death and round changes clear it. Sprite2d
keeps its existing behavior. Both client and server must be rebuilt/restarted
to transmit and display the new direction.

## Contrast and glow

The 3D gameplay camera uses TonyMcMapface tonemapping, mild post-saturation
(1.08), and selective additive bloom (intensity 0.12, threshold 1.0). The key
light is near-neutral daylight (sRGB 1, 0.98, 0.94) so it preserves the cool
floor instead of adding a strong yellow cast. Exposure, light intensity and
the player's lighting preferences are unchanged. Broad field fills stay
at ordinary brightness and 12% opacity; small skill cores/rims use `hdr_gain`.
White streaks use gain 5, team boundaries gain 2, and pooled combat particles
gain 2.5 throughout their fade. Unlit Bevy materials ignore emissive, so these
drawables multiply **linear base RGB**, preserving alpha. Combat drawables
ignore distance fog; server visibility still controls whether they exist.

Verdant's environment/foliage materials use cached, runtime-only copies with
a cooler slate floor, deeper water/greens and less glossy reflection. This
also grades the active procedural river's vertex colors through its material. The
palette is in `environment_palette` in `client/src/verdant3d.rs`; changing it
requires a rebuild. Source GLBs, character materials and live structures are
preserved. Sprite2d keeps its camera and particle colors. No additional
dependency, texture, light or particle count is required by this pass. Bloom
adds GPU postprocessing; mobile performance still needs device measurement.

Contrast verification on 2026-09-30: the client library suite passed (810 tests,
one existing ignored); the subsequent eight Verdant tests passed, including a
new integration check that loads the shipped GLBs and actual map cosmetics.
Both final native scenarios passed with exit code 0 at English 1280×720.
Their JSON confirms 24 cached palette copies, the tuned paving binding and
TonyMcMapface/bloom on the camera. Evidence is in
`.agent/tasks/COMBAT-CONTRAST-2026-09-30/verified/`; final client SHA-256:
`dbd7cac8f7a54050e9b51c76618cfb06094a32218882952b48888154b6b19490`.
An initial native shutdown timeout was fixed by closing the primary window
before AppExit, matching the production Exit button and other capture harnesses.
The capture script now also accepts `--timeout` and reports watchdog expiry
explicitly. Visual review covered beam, field, rocket/traps and hit/projectile
states. No physical mobile, crowded-fight performance or other locale was tested.

## Legacy projectile cosmetics

Combat presentation is configured in
[`client/assets/config/combat_visuals.json`](../client/assets/config/combat_visuals.json).
It changes rendering and animation selection only. Damage, reach, projectile
motion, cooldowns, minion roles and confirmed hits remain authoritative server
data; the schema rejects unrelated fields such as `damage` or `speed`.

The client reads this packaged JSON through Bevy's AssetServer on desktop,
Android and iOS. An independent set of built-in class profiles is embedded in
the client. Missing files, unsupported schema versions or invalid configuration
leave those defaults active. Invalid custom JSON cannot break the built-in
profiles even if the client is rebuilt with that JSON in the asset directory.
No remote URL, filesystem override or downloaded cosmetic is requested.

The registry applies a valid packaged manifest once per client launch. Restart
after changing it. Existing in-flight projectiles keep the profile they began
with. Avatar animation bindings refresh when the packaged registry becomes
available; this allows clip aliases to work even when avatar models load first.

## Profiles and lookup

Version 1 has `profiles`, `defaults`, `classes`, `avatar_overrides`,
`sprite_overrides` and `animation_aliases`. A partial manifest extends the
built-ins. A profile defines a procedural `shape`, RGBA `color`, uniform `scale`,
`trail`, `impact`, and optional `model` or `sprite`.

The lookup order is:

1. Matching sprite ID: action-specific profile, then its `default`.
2. Matching avatar slug: action-specific profile, then its `default`.
3. Hero class: action-specific profile, then its `default`.
4. Authoritative projectile style default, then the built-in standard profile.

Actions are `basic`, `q`, `w`, `e`, `r`, and `default`. The wire's absent action
slot means `basic`; legacy slot 255 is treated the same way. Unknown slots use
`default`. Avatar slugs and sprite IDs are case-sensitive and must match the
identity actually sent by the server. Sprite overrides have priority in both
render modes so projectile and impact settings resolve consistently.

Class defaults are Warrior crescent, Ranger arrow, Mage arcane bolt and Cleric
holy bolt. Minion caster bolts and structure bolts have separate style defaults.
Projectile shapes use shared procedural meshes in 3D and colored sprite pieces
in 2D; their silhouettes work without custom model or image assets. Small team
color accents remain visible on the procedural shapes. A Warrior crescent still
follows the server projectile: its cosmetic shape does not make a ranged skill
an instantaneous melee strike.

## Example: one avatar's basic attack

Add this as a partial manifest, or merge its sections into the shipped file:

```json
{
  "schema_version": 1,
  "profiles": {
    "agnes_leaf_arrow": {
      "shape": "arrow",
      "color": [0.35, 0.95, 0.60, 1.0],
      "scale": 1.1,
      "trail": {"seconds": 0.18, "width": 0.06, "samples": 8},
      "impact": {
        "color": [0.50, 1.0, 0.70, 1.0],
        "scale": 0.8,
        "lifetime": 0.30
      },
      "model": {
        "path": "cosmetics/agnes/leaf-arrow.glb",
        "scene": 0,
        "scale": 1.0,
        "rotation_degrees": [0.0, 0.0, 0.0]
      },
      "sprite": {
        "path": "cosmetics/agnes/leaf-arrow.png",
        "frame_size": [64, 64],
        "columns": 4,
        "rows": 1,
        "first_frame": 0,
        "frames": 4,
        "fps": 16.0,
        "world_height": 1.4
      }
    }
  },
  "avatar_overrides": {"agnes": {"basic": "agnes_leaf_arrow"}},
  "animation_aliases": {
    "agnes": {
      "idle": ["Standing_Idle"],
      "walk": ["Walk_Forward"],
      "attack": ["Bow_Release"],
      "cast": ["Spell_Cast"],
      "death": ["Falling_Death"]
    }
  }
}
```

This is an authoring example, not a claim that those files or named animation
clips are bundled. Replace them with your own permitted files and exact clip
names. The procedural arrow remains available when either example asset is
missing. Omit `model` and `sprite` entirely for a fully procedural customization.

Put model/image files under `client/assets/` using the relative paths above.
Add the files and their permission/provenance notices to version control;
new asset directories may need an explicit `.gitignore` exception. Native
packaging copies versioned assets; Android and iOS package the client asset
tree. Check the actual resulting package before release. Configuration does
not grant permission to redistribute an avatar, model, image or animation.

Hero 2D artwork uses the existing [sprite roster manifest](../client/assets/sprites/manifest.json).
Each character defines locomotion/action sheets and `idle`, `run`, `attack`,
`cast`, `hit`, and `death` frame ranges. Add the art there, then use that same
character ID in `sprite_overrides` to pair its attacks with a projectile skin.
The hero roster manifest is embedded at compile time: adding a new identity
also requires updating the shared sprite ID list in `shared/src/lib.rs` and
rebuilding the client/server. The `animation_aliases` section below applies
to skeletal 3D avatars.

## Model, sprite and animation contracts

- A model is a packaged `.glb`, centered around its origin, with **+Z forward**
  and **+Y up**. Author a small effect near unit scale. `scene` selects a glTF
  scene and `rotation_degrees` corrects asset axes. The procedural shape stays
  visible until the selected scene and its dependencies load and contain mesh
  geometry; failed or empty scenes keep the procedural fallback.
- A sprite is a packaged `.png` atlas with **+X forward**, equal frame cells,
  and no inter-cell padding. Its image dimensions must exactly equal
  `frame_size × [columns, rows]`. Frames loop from `first_frame` at `fps` and
  retain the cell aspect ratio. Missing images or mismatched dimensions keep
  the procedural sprite pieces visible. Atlas handles are cached per profile.
- Animation aliases are ordered, exact, case-sensitive named glTF animation
  clips. Supported fields are `idle`, `walk`, `run`, `attack`, `cast`, and
  `death`. The player renderer tries aliases before its existing name
  heuristics. Keys are avatar slugs, or `character:<id>` for legacy characters.
  Two distinct idle and walk/run clips must resolve before an animation set
  (including its optional attack/cast/death clips) is registered. `run` aliases
  are fallbacks for the walking locomotion slot, not a separate speed state.
  No clip is manufactured if the model does not contain it.
- Paths must be asset-root-relative ASCII paths using letters, digits, `_`,
  `-`, `.`, and `/`. Absolute paths, drive prefixes, URLs, `..`, backslashes,
  and glTF `#Scene` labels in the path are rejected. Use the `scene` field.

## Bounds and verification

Version 1 permits at most 128 profiles, 256 avatar overrides, 256 sprite
overrides and 256 animation alias entries. Each override has at most six action
keys; an alias field has at most eight clip names. Unknown schema fields and
unresolved profile references reject the custom manifest.

Profile scale is 0.25–3.0. Colors are finite RGBA components in 0–1, with alpha
at least 0.25. Trail duration is 0–0.6 seconds, width 0.02–0.35 world units and
history 1–12 samples. Impact scale is 0.1–2.0 and duration 0.08–1.2 seconds.
Model scale is 0.01–4.0, scene index 0–31, and rotations −360–360 degrees.
Sprite grids are at most 16×16 cells, cells at most 1024×1024 pixels, and the
complete atlas at most 4096×4096. Sprite playback is 1–60 fps and rendered
height 0.3–4.0 units. These limits constrain settings; authors must also keep
model geometry and textures economical for mobile devices.

3D trails use short histories of actual network positions. A large position
correction clears the history. Removing a projectile removes its presentation;
it does not emit an impact. Impacts and floating damage require confirmed server
combat events. The 2D minion staff/crystal and shield/blade cues distinguish
caster and melee roles without replacing the existing actor atlas.

Before contributing, validate the manifest, exercise its avatar/class/action
lookup, check both presentation modes, test missing custom assets, and inspect
readability at desktop and mobile camera sizes. Registry, resource lifecycle,
atlas fallback and role-cue regressions live beside the corresponding Rust
modules. A native touch preview is useful evidence, but does not replace a
physical Android/iOS performance and readability check.
