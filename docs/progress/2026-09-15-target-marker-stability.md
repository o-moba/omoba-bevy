# Stable target presentation — 2026-09-15

The target frame mixed a current local transform with the previous propagated
camera transform during Update, before remote interpolation finished. The second
world marker also rotated, pulsed and bobbed around the center of animated model
bounds. Both mechanisms could make a selected player look unstable.

Selection presentation now runs in PostUpdate after movement, terrain grounding
and camera projection updates, before UI preparation/layout. Bevy UI layout itself
precedes global transform propagation, so TransformHelper computes the current
hierarchical target and camera poses directly. The ring then participates in normal
transform propagation and visibility processing in the same frame. No extra
smoothing or interpolation delay is introduced.

The world marker is a fixed annulus on the terrain, with 3D/2D materials and no
shadow casting. Skin bounds, rotation and animation no longer determine its
height. The persistent screen frame is thinner. Both indicators validate the
selected identity and disappear for dead, hidden, stale or despawned targets.
Click/touch acquisition, attack rules and network protocol are unchanged.

Verification includes regression tests for late target/camera motion before UI
layout, 60 animation/rotation frames, hierarchical movement, immediate target
switching and invalidation in both render modes. The full client test suite has
350 tests. Native scripted desktop and mobile captures use real mouse/TouchInput
and a local server with passive player targets; they check selection, attack,
server-observed damage and cancellation. Captures are development fixtures, not
manual or physical-phone playtests.

The first and one repeat mobile capture intermittently failed to establish its
initial drag preview before a selection existed. Diagnostic runs subsequently
passed without changing touch behavior; additional failure telemetry is retained.
This transient synthetic-input failure is not claimed fixed by the marker change.
Physical-phone movement/keyboard/touch testing remains outside this verification.

Evidence: `.agent/tasks/TARGET-MARKER-STABILITY-2026-09-15/`, with complete capture
provenance, snapshots, image hashes and test logs. Prepared as 0.20.0-rc.3 on
`fix/target-marker-stability-2026-09-15`, in the existing isolated player-portal
worktree. No main merge, push, production migration or deployment in this task.
