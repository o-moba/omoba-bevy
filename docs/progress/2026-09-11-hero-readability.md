# 2026-09-11 — Desktop minimap and hero readability

Candidate: `0.18.0-rc.8`, based on the native mobile beta `7c4c1e8`.

The desktop minimap now sits at the upper left. The objective panel reserves
space beside it; the equipment panel still fits a 960px desktop window. The
phone HUD keeps its separate anchors and scale, and minimap input still uses
the actual computed UI bounds.

## Why the characters looked too small

The shipped models were being normalized correctly, but the art proportions
did not support readable heroes. All 60 reed instances were taller than the
1.45-unit hero: 1.989–3.534 units, with a median of 2.787. The 241 grass fans
reached 0.914–1.818 units. The existing follow camera was 36.88 units away.

| Presentation parameter | Before | Now |
| --- | ---: | ---: |
| Normalized hero height | 1.45 | 2.1 |
| 3D camera horizontal offset | 24 | 20.4 |
| 3D camera vertical offset | 28 | 23.8 |
| Reed height range | 1.989–3.534 | 0.995–1.767 |
| Grass-fan height range | 0.914–1.818 | 0.594–1.182 |

The camera moves 15% closer without changing its angle. Players retain the zoom
range; the default view trades some lane coverage for a more legible hero.
The larger hero is a visual model adjustment: authoritative body size, movement
clearance, attack ranges and creature/boss reference sizes do not change.

The plant adjustment is reproduced by `scripts/stage_verdant.py` from the same
source export. Only root Y scale for `river_reeds` and `grass_fan` changes.
Their mesh bottoms coincide with their root origins, so their feet, positions
and horizontal footprints stay fixed. All tree/rock geometry is unchanged.
Collision generation updates source hashes only; all 236 polygons remain equal.
The saved Blender scene and all source art files remain unchanged.

Preference schema 4 migrates schema 3's default 1.45 to 2.1 once. Custom values
survive, including 1.15 deliberately saved under schema 3 and either old default
deliberately saved under schema 4. Pre-schema-3 migration rules remain intact.

## Evidence

Task-local source, logs, manifests and native captures are under
`.agent/tasks/HERO-READABILITY-2026-09-11/`. Native screenshots use the actual
Bevy renderer and a temporary local server. The Verdant composition includes
five explicitly tagged creature render fixtures; it is not a complete human
match. Phone captures remain a desktop development preview, not physical-device
certification.

Full-frame native comparisons:

- [Before: desktop follow view](2026-09-11-hero-readability/before-desktop.png)
- [After: desktop follow view](2026-09-11-hero-readability/after-desktop.png)
- [Desktop at 960×540](2026-09-11-hero-readability/desktop-960.png)
- [Mobile interface preview at 844×390](2026-09-11-hero-readability/mobile-preview.png)

The adjacent [capture manifest](2026-09-11-hero-readability/captures.json) records
image hashes, dimensions, versions and actual client/server binary fingerprints.
The before/after follow composition uses the same nominal gameplay view; the
after camera deliberately sits closer. Animation poses and live simulation
timing are not synchronized frame for frame.

An independent geometric comparison of the recorded follow cameras projects a
nominal head-to-foot segment from 32.50px to 56.81px at 1600×1000 (+74.8%). This
estimates standing height, not occupied pixels in the animated silhouette.
The nominal target-plane width retains 84.85% of the old view and the flat-ground
area 72.23%; zooming out remains available. Visual inspection confirms that the
bridge, nearby heroes and lane creatures still fit the comparison composition.

Current-source checks passed: 219 client tests, 22 relevant Python tests,
rustfmt, native client/server builds and strict workspace/all-target Clippy.
The independent asset validator reproduced all six GLBs and the manifest byte
for byte, checked 2,123 walkable-surface rays and 1,980 joins, and verified all
40 source-art files remained unchanged from the task's base commit.

All five current renderer runs passed: five Verdant views, seven desktop stages
at each of 1280×720 and 960×540, seven mobile-preview stages, and four navigation
captures backed by authoritative snapshots. Desktop gameplay readbacks assert
the actual minimap is 252×252 at logical `(16,16)`. Navigation uses ordinary
input and local-server movement, with no teleport or movement fixture.

Initial local-socket tests were restricted by the sandbox; the unchanged suites
passed with loopback access enabled. No installer, public server or site
deployment is part of this presentation change. Actual phone behavior and
subjective balance still require device/human playtests.
