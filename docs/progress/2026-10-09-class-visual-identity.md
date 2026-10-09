# Class visual identity and skill mechanic fixes — 2026-10-09

Candidate: 0.44.0 (no version bump; the candidate is unreleased and already
requires client and server to update together). Branch
`feat/class-visual-identity`, with `feat/skill-mechanic-fixes` merged into it.
Nothing was pushed, deployed, packaged for a device or uploaded.

## Goal

The owner's request (2026-10-06): class animations and effects were mediocre
and often identical, characters "shoot rockets", class character was unclear.
Give every skill of every class its own behaviour pattern and visual identity
with no copy-paste between classes, make fights more spectacular and classes
distinct, and make sure the skills really work and do what they say.

## What changed

Presentation (client only):

- `client/assets/config/skills.skillfx` is schema 2: 68 skill rows, 17 class
  themes, 17 basic-attack rows. Every row has a cast accent, a motion, a cast
  voice; every row of a replicated effect has a body; every damaging row has a
  hit recipe. The legacy `effect` styles, the shared cast puff
  (`emit_skill_cast_particles`) and the built-in basic-motion table are
  deleted. The authoring vocabulary is closed and generated into
  `docs/skill-vocabulary.md`.
- Engine: a cast observer and choreography, a stage tracker with end
  classification, a body renderer over a shared mesh library (eight
  archetypes), projectile forms for the legacy kits, per-skill playback with
  four phases, state visuals from replicated flags, recast markers, per-skill
  voices from the existing samples, aim previews from one table that mirrors
  the server rule.
- Motion library: 20 to 55 clips, 38 of them derived from the checked-in CC0
  sources by `assets-src/animations/derived-motions.json`; three clips
  retired. No new source asset, licence or audio file.
- Capture tooling: offscreen phase capture with hard assertions per still,
  contact sheets and a look-alike report with an energy column.
- A defect of `main` found on the way and fixed: a remote hero that stood
  still could animate a sprint on the spot, because the animation driver was
  not ordered after network grounding.

Mechanics (authoritative, gameplay revision changes):

- Bot self-sustain by need; a weapon toggle keeps concealment; receipts from
  hidden heroes reach the victim with the source withheld; the Fault Line
  pillar and the Furnace Breath telegraph are replicated as the server uses
  them; hero-only picks for Fourfold Duel, Patient Curse and Orbital Guard and
  no structure for Thunder Kick; a phase-aware rooted gate; honest recasts for
  Thunder Pulse, Thorn Volley and Iron Hook; orb leash, field haste and guard
  aura no longer undo themselves; Rift Seal marks unprotected enemy
  structures; Sheltering Leap and Anchor Step no longer double the self
  shield; zone slows keyed by effect; Northwall publishes its interception
  count. Four description strings corrected in three languages.

Client fixes for input and gating:

- Legacy self casts keep the move order; the Mountain Echo recast is offered
  only inside its gate; an illegal Rift Step landing and an ally skill without
  an ally are refused with feedback before they are sent; the lantern prompt
  follows the server conditions; taps and stick-less presses of Sheltering
  Leap and Orbital Guard aim at an ally; pick previews follow the server's
  kinds; Northwall shows its blocks; a hit by an unseen hero has no direction;
  a Fault Line pillar crumbles without its owner.

The player-facing list is in `CHANGELOG.md` under the 0.44.0 candidate; the
contract is in `docs/combat-cosmetics.md`.

## Compatibility

`GAMEPLAY_REVISION` is `combat-2026-10-08-mechanics` (was
`combat-2026-10-06-blink`), changed once. `PROTOCOL_VERSION` 11,
`CATALOG_REVISION` `standard-kits-6`, the wire structs and every catalog number
are unchanged; the catalog diff is four `description` strings. **Client and
server must ship together.** The Northwall count uses the existing
`consumed_segments` byte of an effect.

## Checks

All on the final tree, English, one viewport (1280x720, `models3d`,
offscreen).

**The full gate passed** on the final tree (`f255d14`), once disk space was
freed (the owner approved removing the compiler's incremental directories from
the two shared caches):

| Check | Result |
| --- | --- |
| `make check` (fmt, workspace clippy with warnings as errors, clippy without the `qa` feature, workspace tests, script tests) | exit 0, no warning |
| workspace tests inside it | client 1268, common 168, shared 129, server 333, passport 30, career-store 13, account-api 6 passed; 0 failed |
| script tests inside it | 208 and 45 passed |
| `cargo build -p server && cargo test -p harness -- --test-threads=1` | exit 0; 24 unit and 26 integration tests passed |
| `python3 scripts/export_humanoid_motion.py --check` | exit 0, 55 motions |
| captures: 17 classes, the mixed recipe and a Sprite2d smoke (taken two commits before the final tree; the three classes whose rows changed afterwards were captured again on the final tree) | every `capture-run.json` passes; no black frame, no panic |

The outputs are `raw/make-check.txt` and `raw/verify-gameplay.txt` in the task
folder.

One test of `main` was unreliable under load and is fixed:
`frontend::draft::tests::phone_avatar_picker_has_visible_tiles_and_raw_taps_switch_tabs_and_select`.
The test drained the network-command messages four `app.update()` calls after
the tap that sent the request. With `TimePlugin` in the app, Bevy swaps its
message buffers only after a fixed-timestep tick, that is after wall time, so
on a loaded machine the request had already been dropped (27 failures in 623
runs under load, every one at the last assertion; none in 8 idle runs). The
game was never affected: its network system reads the messages every frame.
The test now records the request in the frame it is written; ten runs in a
row pass.

## Where the new look is weaker

Measured against a baseline taken with the standing-hero fix, three runs a
side: the median changed-pixel area of the 114 comparable stills is 1.30 of
the old look; 17 stills are below it. By the measurer's per-class table 14
classes read better than their old look and three are level (Edgeweaver,
Emberveil, Riftshot); no class as a whole is weaker.

- Was genuinely weaker and is tuned: the release of Wandering Ember measured
  0.87, a pale orb on pale stone. A larger toss accent, wider satellites and a
  deeper accent colour bring it to 0.98 in three trial runs (level, inside the
  run-to-run noise). The same change gives the basic-attack hits of
  Veilstalker and Cinderforge more pieces and a darker slot.
- Smaller on screen: Edge Lunge and Rift Step (a hit stays within 1.5 units of
  its receipt); the dash line of Flame Dance.
- Smaller for an honesty reason: Horizon Wave, Echo Strike, Rampage (old
  shapes larger than the replicated geometry or drawn around one victim);
  Pyroblast and Longshot on their first frames (the body grows from the hand).
- Pale basic-attack hits: Warrior, Mage, Ranger, Orbitwright, Cinderforge,
  Veilstalker and the launcher round.
- Longshot and Piercing Arrow read alike on the release still.
- The broken Northwall keystone is partly hidden by its holder; a block has no
  sound.

## Not verified

- No physical device: no phone or tablet, no touch on glass, no frame time,
  no thermal or battery behaviour. The budgets are counts read from data.
  Overdraw of the translucent fills and the first-use retarget hitch with 55
  clips were not measured.
- Nothing in motion. Every judgement of a look is from stills (three or four
  per skill); no clip, flight or transition was watched as it plays.
- No sound was heard. The voices are checked as resolved values.
- No match with people. Multiplayer was exercised only by the headless
  harness and by a sandbox with a passive target.
- English only, one desktop viewport (1280x720), one avatar rig for the
  captures. No Russian or Chinese screen, no other aspect ratio.
- The flat view: one Sprite2d smoke of one class; no 2D capture matrix.
- A hit from an unseen hero, a later Northwall block, two walls of one hero, a
  renewed Wandering Ember, a real walk during the Furnace Breath windup and a
  team fight with many casters were tested in code and never seen in a
  capture. Pool pressure in a team fight was not checked (the largest count in
  any still is 37 of 256 live particles, with two heroes).
- The four client input fixes (Field Dressing, Mountain Echo, Rift Step,
  lantern) and the two ally-cast fixes are proven by unit, ECS and authority
  parity tests only.

## Owner decisions waiting

1. **Anchor Step self shield.** It lost its doubled self shield together with
   Sheltering Leap (50 to 25 at rank 1), because both run through one branch.
   Bots always self-pick both leaps. Keeping the doubling for Anchor Step alone
   is one condition.
2. **Rift Seal on structures.** A tower in front of a hero now takes the seal,
   and the owner's next hit adds 34 x scale magic damage to an unprotected
   tower once per 11 s cooldown. It matches the skill's text; it is a real
   rule change.
3. **Real melee for Warrior and Warden.** Both kits are still homing server
   projectiles; only the picture is a melee contact.
4. **Hunter's Mark text.** The tooltip promises a pin; reword it, or add a real
   root (a gameplay change). Shown as damage only until decided.
5. **Nightfall text.** "Execute" promises more than the skill does. Untouched.
6. **Technique effects that outlive their caster** (Horizon Wave, Winter
   Shard, Winter Divide): one roster-wide ruling. The Fault Line pillar already
   outlives its owner, because it blocks.
7. **A unified status system.** Raised by the coordinator during the run; no
   design for it exists in the repository. The reason is visible in this
   candidate: slows, hastes, shields and defenses from several sources each
   needed their own fix (a per-effect slow key, "never shorten a longer haste",
   "do not replace a stronger defense").

Smaller ones from the analysis, unchanged: whether the caster of Horizon Ray is
anchored during the windup; the Edge Lunge refund, the Mirror Guard riposte
anchor and who sees a Fourfold Duel challenge; Patient Curse "weaken" without
mitigation and Shadow Lash on a structure; whether the lantern rescue passes
terrain; class-default handhelds for Cinderforge and Edgeweaver; the Sprite2d
team colour of hostile lanes, cones, cages and walls; the HUD line of Dagger
Mastery and the Ranger tagline, which still lack the corrections their skill
descriptions got.

## Deferred

Needs a protocol change:

- Patient Curse: the mark, its target and ripeness are not replicated. Nothing
  is drawn on the target.
- Rift Seal: the attached seal and its detonation are not observable, on a
  hero or a tower. The seal has a release and travel identity only.

Cut for size or rejected:

- Victim hit reaction on the body; fuse-corrected remaining ring for Orbital
  Collapse; facing satellites on a fog-cut lane.
- Preview `lane_to_point_rear`, `BasicProfile.color`, `cast.recast_link`,
  `stack_speed_step`, the `quickened` and `soul_stack` states, `owner_tether`
  on the lantern, `crumble` on a lane, `body.anchor: owner`.
- Smoothing of legacy homing projectiles, root interpolation of replicated
  bodies, catalog-timed caster states, kill moments and camera effects.
- A true 2D world-effect renderer, new 2D projectile shapes, per-skill 2D hero
  motion, new GLB props, new or synthesised audio samples.
- Hoisting the mirrored server literals into `shared`; bot AI beyond the
  self-sustain fix (bots aim ally skills at themselves; the Wildspark bot stays
  in Rockets mode); Echo Strike follow-up facing.

Known and left as they are:

- About 75 comments in `client/src` cite line numbers of
  `common/src/skills/advanced.rs` as they were before the mechanic fixes moved
  them.
- On a controller, a press without the stick of a skill that is not cast on an
  ally goes straight ahead while its preview points at the nearest enemy; a
  cast on an enemy without a target is dropped without a word; a rooted
  Sheltering Leap or Rift Step is still sent.
- A fuse can release without its clip at sandbox speed 2.0; a bolt first seen
  more than 0.25 s after it fired plays no release.
- The style defaults of an unseen Warrior and Cleric still draw a burst that
  reaches past 1.5 units.

## Evidence

Raw logs, captures, sheets and the per-package reports are under
`.agent/tasks/CLASS-VISUAL-IDENTITY-20261006/` in the primary checkout (local,
deliberately untracked): `evidence.md`, `evidence.json`, `raw/final`,
`raw/final-flight`, `raw/final-sheets`, `raw/WP24`, `raw/reports`.
