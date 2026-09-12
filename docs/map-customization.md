# Map objects, tuning and presentation

Open Moba separates the arena's gameplay objects from their appearance. The
server selects positions and combat rules. Clients render the resulting objects
with a packaged visual registry. Editing a model never grants extra reach, HP
or a different collision shape.

## Tune towers without rebuilding

Start with `shared/assets/maps/verdant.json`, or use the complete
`examples/maps/two-tier.json` example. Pass your edited file to the server:

```sh
OMOBA_MAP_CONFIG=examples/maps/two-tier.json cargo run -p server --locked
```

The server validates and resolves the file before accepting players. A missing
or invalid explicitly selected file stops startup with a diagnostic; it does
not silently start the default arena. Without the environment variable, the
embedded default preserves the six lane towers and two bases. Settings stay
pinned for the process, including rematches. Restart the server to load edits.

The example adds a second tower for each team on mid lane, moves the outer mid
towers toward the middle and gives them 300 HP; inner towers have 420 HP.
It is a tuning example, not the default balance or a recommendation for ranked
matches. Clients learn the count, coordinates and HP from snapshots.

## Stable objects and reusable gameplay profiles

A map file has `format_version: 1`, `geometry_id`, `map_profile`, reusable
`profiles`, and a `structures` array. A tower entry looks like this:

```json
{
  "id": 9,
  "key": "mid_green_inner",
  "team": "green",
  "kind": "tower",
  "lane": "mid",
  "t": 0.22,
  "offset": [0.0, 0.0],
  "profile": "lane_tower",
  "visual_profile": "tower_green",
  "overrides": { "max_hp": 420.0 }
}
```

- `id` is a unique nonzero network ID. `key` is a unique readable identity used
  for per-instance presentation. Keep both stable when moving an existing tower.
- `lane` is `top`, `mid` or `bot`. `t` measures distance along that authored road
  from Green (0) to Blue (1), not a coordinate on a straight line. Green towers
  accept 0.08–0.48, Blue towers 0.52–0.92. `offset` is an optional XZ adjustment,
  at most three world units in length. Placement must clear fixed obstacles,
  arena edges, player spawn points and other structures. Offsets must also retain
  the physical outer-to-inner order along the lane. The final position must be
  within three metres of the route minions walk. Decorative dead-end road spurs
  are rejected even when their `t` falls inside the team's numeric bounds.
- `profile` selects gameplay numbers. Optional `overrides` replace individual
  fields: `max_hp`, `attack_range`, `attack_damage`, `attack_cooldown_ms`.
- `visual_profile` names a client presentation archetype. Missing cosmetic data
  keeps a built-in visual; it cannot change the gameplay profile.
- A base uses `kind: "base"`, with one fixed-pad base per team. Its stats and
  appearance are configurable; its anchor remains tied to the authored pad.

Use array entries to add or remove towers. The total limit is 32 structures,
including both bases, matching the navigation obstacle budget. Gameplay profile
bounds are HP 1–100000, range 1–60, damage 0–10000 and cooldown 100–60000 ms.
These are validation bounds, not balanced recommendations. Invalid or oversized
files, unknown geometry, duplicate identities and overlapping placements fail.

Siege order is derived from team and lane progress, independent of array order
or ID. Tier 0 is the outermost defending tower; inner tiers unlock as earlier
tiers fall. A base becomes vulnerable when a configured lane is fully cleared.
An empty lane does not bypass other configured defenses; if a team has no lane
towers at all, its base starts vulnerable. Minions and hero damage use the same
protection rules. Tier, lane, protection and object identity are replicated.

## Reuse and replace prop appearance

`client/assets/config/map_visuals.json` supplies the client registry. Version 1
uses `archetypes`, optional `instances` and reusable `palettes`. Existing Verdant
GLB roots carry `asset_id` and `role` metadata; their authored names are stable
instance keys. Trees, rocks, lanterns and other repeated objects therefore use
one archetype definition rather than individually hardcoded loader paths.
3D instance keys are `environment.glb:<authored Name>` or
`foliage.glb:<authored Name>`; use the exact exported name after the prefix.
2D keys are `world2d:tree:<column>:<row>` and `world2d:anchor:<index>`.
Live structures use their server `key`, such as `mid_green_inner`.
The client reads this packaged file once at startup; restart after editing it.
A missing or invalid document keeps the authored defaults. Unknown fields,
unsafe paths and out-of-range values reject the entire document.

```json
{
  "schema_version": 1,
  "palettes": { "warm": [1.0, 0.85, 0.65, 1.0] },
  "archetypes": {
    "lantern": {
      "palette": "warm",
      "model": { "path": "map-props/lantern.glb", "scene": 0 },
      "scale": [1.0, 1.0, 1.0]
    },
    "tower_green": { "sprite_key": "green_tower" }
  },
  "instances": {
    "environment.glb:lantern / 0001": {
      "model": { "path": "map-props/flowering_shrub.glb", "scene": 0 }
    },
    "mid_green_inner": { "tint": [0.8, 1.0, 0.85, 1.0] }
  }
}
```

Both model paths above are bundled original Verdant assets. This example warms
all lanterns and deliberately replaces one nonblocking lantern with a shrub.
To use your own model, package it under the client asset root and change `path`.
Models use glTF Y-up meters and a ground pivot; `scene` selects scene 0–31.
Paths are packaged relative paths, never arbitrary URLs or filesystem traversal.
Instances override matching archetype fields; omitted or null fields inherit.
An explicit `tint` takes precedence over a palette in the same profile.
The built-in structure keys are `tower_green`, `tower_blue`, `base_green` and
`base_blue`.

Nonblocking decoration can use `offset`, `rotation_degrees` and `scale` in the
authored instance's local frame. Solid trees, rocks, walls and live structures
ignore these transform overrides and retain their authoritative anchors and
collision. A solid replacement must set `preserve_collision: true` inside its
`model` object. It must also pass the renderer's local bounding-box check:
X/Z extents each remain within 85–115% of the original, X/Z center differences
are each at most 0.25 m, and bottom-Y difference is at most 0.25 m. This is an
artist assertion plus a bounds check, not a proof of matching trunks, openings
or other blocking silhouettes; authors must preserve those shapes. An
incompatible replacement keeps the original model. Terrain, roads, pads and
the adapted walking bridge are not cosmetic prop instances.
The original stays visible while a replacement loads or if it fails. Repeated
instances share loaded assets; child visuals remain attached to their owners.
Nonblocking replacement models must have local bounds no larger than 16 m on
any axis before the authored instance transform is applied.

Tint/palette colors use finite opaque RGBA (alpha 1); 3D tint multiplies the
existing material color. Decorative offsets are
bounded to ±3 m per axis, rotations to ±360°, and scale to 0.25–3 per axis.
A document is at most 256 KiB with 128 archetypes, 1024 instance overrides and
64 palettes. Runtime bindings are capped at 2048, model handles at 64, and
derived materials at 512. Stable objects skip repeated mesh/hierarchy work;
registry changes and pending asset loads trigger reconciliation.

The 2D contract uses `sprite_key` and palette/tint against the existing actor
and world atlases. A GLB replacement applies to 3D; it does not automatically
produce a 2D sprite. Supply the corresponding atlas artwork when adding a new
2D appearance. Adding atlas keys also requires updating the embedded
`world2d/manifest.json` or `presentation2d/manifest.json` metadata and rebuilding.
World prop overrides swap frames while keeping the original sprite size and
pivot; structure overrides use the selected actor's size and pivot. Nonblocking
2D anchors also accept transforms: offset `[x,y,z]` adds `[x,y+z]` in the render
plane, Y rotation becomes in-plane rotation, and X/Y scale controls sprite
width/height. Forest trees and live structures keep their existing transforms.
Unknown sprite keys keep the existing role fallback. Hero and
projectile skins have their own [combat registry](combat-cosmetics.md).

## Geometry is a separate versioned contract

The supported geometry is `verdant-confluence-v1`. Shared layout coordinates,
static collision and authored terrain must agree. Snapshots identify the arena;
a client with incompatible geometry refuses to apply it. This release edits
objects on the existing Verdant map. Rescaling the entire map, reshaping roads,
moving base pads, or moving solid forest obstacles requires a matching geometry
and collision export and a new geometry revision. A cosmetic model edit cannot
silently remove server obstacles.

## Before publishing a map profile

Check default and custom server startup, siege order, minion paths, spawn
clearance and a full rematch. Inspect desktop and mobile framing, minimap
markers, model footprints and 2D fallback. Run Rust tests and native capture
verification with real server snapshots; inspect the resulting frames. A native
touch preview does not establish performance or usability on Android/iOS.
