# Full 2D world — 2026-07-28

## Outcome

The `sprite2d` compatibility identifier now selects a genuine Bevy 2D render
path. It starts one orthographic `Camera2d` and a planar XY world; it does not
show the previous 3D map, GLB actors, mesh billboards, perspective camera, or
directional-light scene. The optional `models3d` path remains isolated and
selectable.

Authoritative gameplay is unchanged. A centralized projection converts
simulation `(x, z)` to render `(x, y)` and performs the inverse conversion for
cursor input. The camera follows the local hero, supports bounded free panning,
clamped zoom, viewport resize, and minimap focus. All render-side movement,
targeting, animation, VFX, and UI remain consumers of server state.

## World and presentation

The deterministic 55×55 tile layout represents the existing three lanes,
diagonal traversable river, forest belts, two bases, six lane towers, two base
objectives, three normal camps, and two boss pits. Original terrain and prop
atlases were produced for Omoba through Higgsfield Recraft V4.1 and dedicated
to CC0-1.0. Exact prompts, job/media IDs, costs, source hashes, construction
logs, contact boards, and the labeled topology overlay live below
`.agent/tasks/TASK-FULL-2D-WORLD/raw/`.

The runtime uses cached atlas handles and explicit layer bands. Terrain,
decals, low props, actors/structures, projectiles/VFX, overhead indicators, and
screen UI are separated; actors use stable foot-Y sorting. The deterministic
world stays below 4,096 static entities. Transient VFX are capped at 256,
evicted oldest-first, and normally expire within two seconds.

## Run and validate

```sh
make server-dev
make game2d
python3 scripts/validate_world2d_assets.py --self-test
python3 scripts/validate_sprite_assets.py --self-test
```

`make game2d` forces `OMOBA_PLAYER_VISUAL_MODE=sprite2d`, defaults to
`127.0.0.1:4000`, and preserves an explicit `GAME_SERVER_ADDR` override.
Mouse-wheel zoom is clamped; Alt or right click toggles follow; arrow keys pan
while unlocked; Space restores hero follow.

## Verification limits

Workspace format/build/test, strict clippy, focused camera/projection/map/
sorting/lifecycle tests, real-UDP combat coverage, world validation, Makefile
dry runs, and diff checks pass in the recorded proof run. The current machine
has no usable GPU exposed to the process: the native client exits from Bevy
with `Unable to find a GPU!`. Contact boards and headless ECS/network tests are
therefore asset/layout and logic evidence, not native-render screenshots.

The concurrently staged ten-character expansion is intentionally tracked by
`TASK-2D-CHARACTER-PACK-02`. Until its final Orchard Comet Centaur files exist,
the expanded sprite validator reports that specific missing sheet; no
placeholder is used to hide the incomplete pack.
