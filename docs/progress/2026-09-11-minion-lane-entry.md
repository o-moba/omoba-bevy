# 2026-09-11 — Direct minion lane entry

Candidate `0.18.0-rc.10`, based on `fab5f23`.

The dragon/bottom route contained a waypoint at the unused outer corner beyond
its base entrance. Green waves moved 17 world units away from the lane, turned
around and retraced that segment before advancing. The mirrored upper route
made blue waves do the same. In the other direction each route repeated the
spur near the enemy base. At 3.1 units/second this added about 11 seconds over
34 unnecessary units of travel.

`build_minion_path` now omits the unused corner before reversing the path for
blue. The authored road geometry remains the source for tower placement and
client artwork. Both teams keep their base spawn, three-member formation,
initial facing, movement speed and wave cadence. The direct middle route is
unchanged. No protocol or packet change is needed.

## Verification

The frozen task and raw proof are in `.agent/tasks/DRAGON-LANE-2026-09-11/`.
Before the fix, two new regressions failed on the actual production marching
system; the preservation regression passed. A separate live UDP observer also
recorded the wrong-way departure of all three green bottom and blue top
minions, with roughly 16.9 units of reverse travel per actor. It observed all
18 first-wave minions with ordinary server time, unmodified spawn/AI and two
passive joined players. The observer never sends movement or combat commands.

After the fix, the live observer recorded all 18 wave members with zero
wrong-way travel: 476 authoritative snapshots, including 307 per minion over
18 seconds after spawning. The full departure and far-base approach are also
covered by the three deterministic production-system regressions. All eight
structure anchors match the before trace exactly.

Current checks passed: 86 server tests, 31 shared tests and 9 client map tests,
both native binaries, rustfmt and strict workspace/all-target Clippy. Source
hashes and the observed server executable match the checked build. Results and
before/after trace summaries are in
[`2026-09-11-minion-lane-entry.json`](2026-09-11-minion-lane-entry.json).
These are authoritative server traces and deterministic movement simulations,
not native renderer screenshots or a human playtest.
