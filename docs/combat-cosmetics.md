# Combat cosmetics

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
