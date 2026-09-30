# Full roster skill presentation and attack facing · 2026-09-30

The remaining 14 classes now have a first presentation pass for all 56 skills.
Together with Dawnweaver/Wildspark, the registry covers 64 canonical abilities.
Profiles follow accepted skill recipes and input slots, including a tested
four-ultimate fixture; the public server loadout restrictions are unchanged.
The per-class map and remaining work live in
[combat cosmetics](../combat-cosmetics.md#full-roster-presentation-pass) and the
[visual uplift TODO](../plans/combat-visual-uplift.md).

## Changes

- Shared readable silhouettes for hooks, lanterns, orbs, pillars, colossi,
  cones, ice walls/fissures, cages, needles, slashes and pulses. Auxiliary
  objects use their actual replicated kind; consumed cage sides disappear.
- Five small original Blender props with reproducible sources, unlit
  materials, GLB inspections and asset-policy hashes. Missing scenes retain
  procedural fallback geometry. Avatar GLBs were not modified.
- Four additional CC0 humanoid motions: punch, guard, shoulder drive and roll;
  16 shared clips in total. Normal transitions blend over 120 ms.
- Confirmed cast accents and distinct legacy projectile profiles, brighter
  cores, existing particle budgets and server-authoritative damage/visibility.
- Accepted attack direction is separate from locomotion yaw. Local and remote
  3D heroes face the basic attack target or resolved skill direction after
  movement/interpolation. Rejected casts, duplicate snapshots and death cannot
  replay the turn. Position and movement trajectory are unchanged.

## Verification

- `make check`: PASS. Client 818 passed / 1 existing ignored; common 87 passed;
  server 296 passed / 3 existing ignored; shared 103 passed. Other workspace
  suites passed, with existing database-dependent skips. Python 152 passed;
  iOS tooling 44 passed. Format, workspace Clippy and no-QA client Clippy passed.
- New tests cover basic and skill attack directions, rejected casts, movement
  independence, local/remote transforms, preparation holds, death and duplicate
  snapshots. Existing protocol golden JSON remains valid with absent optional
  `action_yaw`.
- Earlier native roster run: 14/14 scenarios and all 56 Q/W/E/R casts accepted,
  plus Riftshot's actual warning/release. Evidence is in the ignored directory
  `.agent/tasks/ROSTER-VISUALS-2026-09-30/verified/`.
- Final native run after the facing fix: all 14 roster scenarios and both
  Dawnweaver/Wildspark regressions passed with process exit 0. All 48 captured
  frames with a confirmed directed action aligned the rendered root's forward
  vector with `action_yaw` (minimum dot product 0.99999993). Visual spot checks
  covered Chainkeeper's hook, Warrior's directed strike and Cinderforge's
  visible moving colossus. Other class effects were reviewed in the earlier
  roster run. Evidence, logs and the facing audit script are in
  `.agent/tasks/ROSTER-VISUALS-2026-09-30/facing-verified/`. Client SHA-256:
  `b93671bbf40d65b6ee5470685c0c2b012ae95b6803a8aa85b68dce5999182a0a`.
- Blender exports passed Assimp inspection and the asset-policy audit.
  The humanoid exporter check and numerical Run retarget audit passed for 15
  rigs; this is not a visual certification of every new gesture on every rig.

The native harness uses one English 1280×720 desktop viewport, live isolated
client/server processes, infinite resources/cooldowns and an invulnerable
stationary enemy. Ally-only casts target self. It verifies accepted actions and
real effect entities, not normal resource balance, every recast/combination,
ally-to-ally travel, manual multiplayer or physical mobile performance.

## Still open

Hand weapons and grips/IK, dedicated bow/kick/parry clips, generic server-driven
jump/flight phases, all recasts and mobile performance remain on the TODO.
The shared guard/shoulder motions are fallback gestures. This is a complete
baseline roster pass, not completion of every original visual acceptance item.

Client and server must both be rebuilt/restarted for the new action direction.
