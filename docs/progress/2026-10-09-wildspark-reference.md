# Wildspark reference iteration

Branch: `feat/wildspark-reference`, based on main `00cff781179e448c34835fa93f39e24901e7e328`.
Scope: Wildspark only; original project art inspired by the requested gameplay,
without third-party character models or textures. No new production dependency,
CI/deployment change or published binary.

## Gameplay

Q retains the accepted-attack contract: repeater stacks attack speed and hits one
target; launcher shots are slower, spend 4 mana and splash in 2 m. The projectile
keeps the mode it was fired in even after a switch. Insufficient mana prevents
acceptance; the sandbox's infinite-resource switch deliberately bypasses costs.
W hits the first enemy at range and slows it by 65% for 1.5 s. E arms for 0.6 s;
three traps share per-cast victim protection and apply their existing root.

R now integrates acceleration from 18 to 45 m/s at 54 m/s², reaching top speed
after 0.5 s. Analytic integration keeps travel independent of tick partitioning;
the range lifetime uses the same curve. Swept collision still ignores minions
as detonation triggers, respects interception and chooses the first enemy hero.
The existing distance multiplier rises from 0.5 to 1 over 20 m, and the splash
adds 25% of each victim's missing health. The focused five-victim test includes
a healthy survivor, a distant enemy and an ally as controls.

A bounded optional `area_impact` on receipts carries the actual detonation id,
skill, centre and radius. It survives a hidden caster, is stripped for a hidden
centre, and allows a shielded explosion without damage or kill fabrication.
Matching client/server gameplay and catalog identities are updated; protocol 11
can still decode absent/additive receipt fields.

## Presentation and editable sources

`scripts/build_wildspark_models.py` exports five original GLBs and the editable
`assets-src/skills/wildspark-reference.blend`. A new default-only repeater path
preserves the old prop used by Riftshot and explicitly equipped legacy skins.
The palette is graphite enamel, brass, safety orange and cyan arc cells.
Open muzzles, separate rotor/recoil assemblies, fins and hinged jaws replace
simple joined pilot geometry. All five exports are validated with Assimp and
recorded in the candidate asset policy.

The client drives rotation and recoil from new accepted action sequences,
resolves the actual skill in the equipped slot, draws the launcher for W/R,
and freezes moving parts with the sandbox clock. Flashes originate at the
current hand socket. Trap opening follows authoritative arming state. A burst
motion replaces the repeater's single pistol shot; W aims rather than lunging;
E uses an underhand throw; launcher attacks use the heavy shot.

One detonation uses twelve pooled particles: a brief compact ignition, one
expanding ground wave, six falling casing fragments and four delayed embers.
Visual and audio cursors deduplicate it across victim receipts and snapshots.
Damage numbers remain per target. Despawn alone stays silent. Existing flat
rendering and global particle/part/light budgets remain applicable.

## Verification status

Final local verification on 2026-10-09:

- Baseline and current live phase/flight captures passed for Wildspark only,
  English 1280×720. Current capture includes 202 motion samples: the repeater
  rotor has 10 distinct transforms, launcher slide 19, each trap jaw 11.
  Before/after stills and the actual moving scenes were inspected.
- Workspace Rust tests passed: client 1289, common 173, shared 130, server 333,
  passport 30, account API 13 and career store 6. Environment-dependent ignored
  tests remain ignored; this run does not certify PostgreSQL or physical phones.
- Formatting, workspace/all-target Clippy and client no-QA Clippy passed.
  The initial script gate caught an unregistered demo module; its map and generated
  documentation were fixed. The final script suites passed: 212 + 45 iOS tooling tests.
- Asset policy, Assimp and the reproducible 55-clip motion-library audit passed.
- The social demo's second take passed all six receipt checks, with normal mana
  and cooldowns. Repeater hits one victim, launcher hits two; W slows a moving
  opponent, E roots a walking opponent, and both R shots hit two victims.
  Against these identical targets, near/far R damage was approximately 69 / 113.
- The approved intermediate cache was cleared after builds; source files,
  dependencies and binaries were preserved. No production deployment occurred.

Local evidence: `.agent/tasks/WILDSPARK-REFERENCE-20261009/` and
`.agent/tasks/WILDSPARK-DEMO-20261009/take-02/`. The final media is local and is
not included in Git. Physical mobile performance remains unverified.

## Reproducible social demo

`scripts/record_wildspark_demo.py --output builds/wildspark-demo` uses existing
client/server binaries (override `--client-bin` and `--server-bin`) and an isolated
local sandbox. The opt-in director chooses Anna, stages two opposing actors,
and sends normal attacks and Q/W/E/R casts with real mana and cooldowns. Setup
resets are excluded from the edit; the opponent walks onto traps. Six chapters
compare repeater, launcher, slow, traps and near/far ultimate shots. A receipt
validator rejects missing damage, splash, mana cost, slow, root, movement or
near/far scaling before editing.

The native capture is English 960×1280. The social export is Russian-labelled
1080×1920 H.264/AAC at 30 fps; a clean native master is also exported. Packaged
SFX are reconstructed from recorded row-voice events in post, with the packaged
CC0 arena music underneath. The manifest records binary hashes and source state;
`demo.json` preserves per-frame actors, effects and receipts. This is scripted
sandbox footage, not a competitive match or a physical-phone performance test.
