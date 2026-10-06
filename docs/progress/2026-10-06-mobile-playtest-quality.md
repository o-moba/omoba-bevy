# Mobile playtest quality — 2026-10-06

Branch: `fix/mobile-playtest-quality`; base: `92101d0`.
Candidate: 0.44.0, separate from the frozen 0.43.0 beta release.

## Priority order

- P0 — unblock play: historical abandonment overlays, explicit leave/rematch lifecycle, overlapping spawns, unintended gameplay overlay.
- P1 — mobile usability and readability: shared button backgrounds, bounded collection, aligned responsive menu, concealment/fog, measured FPS, forgiving rocket contact/precision aiming.
- P2 — match variety: three dragon objectives and team rewards, with a versioned shared protocol.

## Scope and decisions

1. Lifecycle: historical receipts are data; live, scoped PostMatch owns navigation. Explicit leave must return Home.
2. Shared UI: remove duplicate slab backgrounds, fix collection scroll ownership and phone preview allocation, unify menu/debug buttons and tablet height.
3. Rendering: remove brush silhouettes from the screen-space fog mask and invalidate its projection on camera movement. Preserve opaque/masked VRM depth with muted concealment tint and existing eye-off icon. Full silhouette compositing is a separate rendering feature; per-mesh alpha blending is deliberately removed.
4. FPS: count renderer submissions over monotonic time; do not report the selected cap or display-link callbacks as actual rendered FPS.
5. Combat: safe separated spawns, 0.35 m additional rocket radius, progressively finer distant touch aiming.
6. Objectives: three sequential dragons in the shared simulation. Wind/Stone/Flame grant +8% movement, +20 physical/magical protection, +12% damage for 180 seconds to the killer's team. First spawn 90 seconds, successors 75 seconds after each kill. No fourth dragon; rematch resets the sequence.

## Verification

Local verification passed: `make check` (1,602 Rust tests, 39 ignored;
171 Python script tests, 45 iOS-tooling tests), focused native English 852×393
frontend/actual offline-flow captures, and all six server-backed team-vision
transitions. The bot exit/respawn regression also passed ten independent worlds.
The menu was measured at phone and 1180×820 iPad viewports: equal action sizes,
zero iPad body overflow, bounded phone scrolling. Final QA-only route fixes were
rebuilt, linted and executed against the real local server.

Raw logs, images, criterion mapping, initial failures and fresh review are in
`.agent/tasks/MOBILE-QUALITY-20261006/`. The capture driver was repaired to route
around the compact map's rock and stationary observer without bypassing collision.

Physical iPhone/iPad acceptance remains: sustained FPS/thermal behavior, actual
touch aiming and mobile rendering. Desktop rendering does not certify those.
No tag, commit, push, merge, production deployment or TestFlight upload occurred.
This 0.44.0 candidate requires a coordinated client/server release.

## Follow-up: physical-device fog and debug toggle feedback

The owner reproduced the central translucent rectangle on their iPad build.
Earlier AC4 proof was insufficient: a full-screen ComputedNode does not imply
a full-screen image in Bevy 0.19. ImageNode Auto contains its 160×96 mask at its
source aspect ratio. World and minimap fog now explicitly Stretch to their UV
domain. A native negative-control capture checks actual pixels, not node bounds.

Debug rows already had 16 px horizontal padding, but ImageNode ContentBox drew
their decorative frames inside that padding. Kit frame images now use BorderBox;
icons retain ContentBox. Follow-up evidence: `.agent/tasks/FOG-VIEWPORT-20261006/`.

## Second follow-up: exact iPad screenshot attribution

The owner still saw a centered square after the fog fix. Their 2360×1640 capture
marks x=356–360 and x=2000–2004, matching a centered square exactly (360..2000).
The separate 256×256 cosmetic battlefield vignette remained Auto; it now uses
Stretch. Fog-only uniform-raster evidence did not certify the full overlay stack.
New actual-game Auto/Stretch/hidden-mist captures and Home footer evidence are
recorded under `.agent/tasks/IPAD-MIST-20261006/`. Build metadata is now a physical
viewport footer, outside the scaled Home canvas and clear of the ornament.

## Integration — 2026-10-06

The owner accepted the current playtest result and requested an immediate PR merge
without waiting for CI. This integration includes the 0.44.0 changes and the
Bevy presentation experiment TODO. Local formatting and whitespace checks were
rerun; earlier functional/visual evidence is recorded above and in the follow-up
notes. This is not full release certification. No production deployment, mobile
upload or release tag is included. The startup artwork/loading screen remains
unimplemented and is follow-up work. Machine-specific Xcode settings stay local.
