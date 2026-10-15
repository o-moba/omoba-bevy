# Dawnweaver reference iteration

Branch `feat/dawnweaver-reference`, based on main `0c0585c`. Scope: Dawnweaver
(the light mage) — authoritative simulation, bots, animation, effects and audio.
No new production dependency, CI/deployment change or published binary.

## Owner question

"Does the ultimate deal more damage the closer a target is to the centre of the
beam?" It did not. Six independent audits, each adversarially re-verified, agreed:
Horizon Ray (`dawn_ray`) dealt a flat `72 × scale` to every hostile whose circle
touched the capsule (`width` 0.8 is a half-width, plus the target radius: a hero
was hit up to 1.42 from the axis). Distance along the ray and the number of
victims did not matter either. This iteration implements the falloff and makes the
visuals, audio, bots and tests agree with it.

## Simulation and contract

- `SkillEffect::Beam` gains `core_width` and `edge_damage_multiplier`. One shared
  curve, `shared::loadout::beam_damage_multiplier`: full damage within the core,
  then a smoothstep to the edge multiplier at the target's own graze point
  (`width + target radius`), so heroes and minions share the curve shape.
  `dawn_ray`: damage 72 → 80, core 0.35, edge 0.6 (centre 80, drawn edge ≈ 68,
  graze 48 at rank 1 / level 1). The higher centre damage keeps the average close
  to the old flat value; it is a tuning hypothesis, not a measured balance.
- The Radiance detonation and the re-applied mark keep their stored value.
- Receipts gain `core_hit` and `radiance_burst` (serde default, skipped when false;
  protocol 11 decodes their absence).
- Windup commitment: during a Beam windup the caster cannot move, dash or recall
  (`HeroSkills::cast_lock_until`), replicated as `movement_multiplier` 0 and
  predicted by the client. Basic attacks and the field recast remain allowed.
  Proposed for owner review: the visual plan forbade an *invented* cast lock; this
  one is real in the authority, so the held pose is honest. Reverting is one field.
- A tick that jumps past the 0.15 s linger window still fires a due ray once and
  keeps it replicated for a full tail (`BEAM_LINGER_SECS`, shared with the client).
  Death before firing still cancels it.
- A fully intercepted hit no longer arms Radiance; the mark includes the team
  damage buff.
- Catalog `standard-kits-8`, gameplay `dawnweaver-2026-10-11-reference`,
  regenerated `workshop.json`, repinned asset evidence, OpenAPI const.

## Bots

A live recast zone is kept until a visible enemy hero inside it is about to walk
out (it auto-detonates on expiry otherwise). Catalog damage skills are not spent
on structures. The beam leads moving heroes by the free part of the windup from
observed velocity (no velocity across a hold), prefers rooted/slowed heroes and
fires right after the bot's own root; a lone minion never draws it.

## Presentation

- Effects: the fired core is drawn at `core_width / width` of the lane (pinned by a
  parity test against the authority), with thin core lines in the warning; core
  hits lead with white and a wider flash, edge hits draw half the particles;
  Radiance detonations draw a gold pop instead of the hit recipe; the warning →
  beam flip plays an afterglow; the R impact leads with HDR slots; the field recast
  draws no hand-to-victim streaks; many-target piercing strikes share the particle
  budget, heroes first.
- Motion: `spell_charge` (0.8 s, two hands gathered then raised) and `ray_release`
  (two-hand push, recoil, settle), derived from the CC0 library; the lane facing
  lasts through the release; a late-seen windup joins at its progress.
- Audio: a telegraph moment plays a synthesized charge riser when the warning first
  appears, for every listener and independent of caster visibility, gained at the
  lane point nearest the listener; the release uses a new `radiant_beam` voice and
  the same lane gain; `core_hit` and `radiance_burst` have their own cues; the
  caster hears its own long-range hits. New samples come from
  `scripts/build_audio_palette.py` (`--only`, no external assets); existing sample
  bytes are unchanged.
- Aim: direction skills whose effect ignores aim distance (beam, linear projectile,
  returning shield, impact rocket) send the far point of the aimed ray, so the
  server line from its slightly stale origin meets the preview at the far end.

## Verification

See `.agent/tasks/DAWNWEAVER-REFERENCE-20261011/` (local, not committed) for the
audit, the per-stream reports and evidence. The final gate on the merged branch is
listed in the pull request. The cargo cache is shared between worktrees and cargo
judges workspace crates by file times, so every source file was touched before the
final build to force this branch's code.

Visual checks: one English 1280×720 offscreen capture of the windup and the fired
ray. The core/edge bursts, Radiance pop, afterglow and motions are covered by unit
tests and data checks only; no listening pass was done for the new samples.

## Remaining gaps and decisions

- Owner decisions: keep the windup movement lock; whether the ray should deal
  reduced damage to minions and monsters (it hits every non-structure along 45 m);
  whether Northwall should block it like other line skills.
- Not done: upper/lower-body animation masks; status replication (marks, roots,
  slows) on minions and monsters; reduced damage for the snare's second target;
  arm time for the field; telegraph dodging and ally shielding for bots; a balance
  probe for modular kits; spatial audio; the Marked diamond hidden while a higher
  status shows; the 0.5 s schema cap on the charge accent; direction-targeted
  technique lanes still use the raw cursor.
- Not verified on physical devices or with a listening pass.

## Combined dev integration (2026-10-15)

The owner requested combined testing in `dev`, so PR #85 now targets `dev` and
incorporates desktop aiming (#84), the skill-slot idea (#87), Astral Ivory (#100),
community sculptures (#102), and monthly Hero recognition/orbits (#103).

The bot merge retains all difficulty profiles, target/retreat gates, controller
construction, and both profile and Dawnweaver test modules. A beam may choose a
different hero than the normal attack target, so its candidate filter now reuses
the existing lower-level admission gates without allocating or changing cadence.
Elite behavior is unchanged by this filter. Four runtime cases cover Recruit's
minion-first gate and Regular's trade gate, plus positive controls. The rejected
hero is 10.8 m away: beyond Dawnweaver's 10 m basic reach plus the target radius,
but within Recruit's 11 m engagement range.

Cursor ownership/focus/release handling stays intact. Direction tests now require
the exact Last Spark endpoint 256 m along the cursor ray, matching the existing
Dawnweaver change instead of the old raw cursor-point expectation. The regenerated
workshop is paired with OMOBA Web PR #13. Matching client/server catalog and
gameplay revisions are required before hosted testing; this integration deploys
nothing.

Validation on the corrected combined source: formatting, both strict Clippy modes,
2,170 Rust tests (50 existing ignored) and 55 iOS packaging fixture tests pass.
A fresh reviewer independently reran 62 desktop/bot/Hero/sculpture tests. Clean
GitHub CI run 38745687316 passed PostgreSQL tests, bot difficulty smoke, the UDP
harness, Android compilation and Python tooling on identical production source.
Its Rust job passed the Linux Elite trace comparison: both scenarios' 600 digests
and summaries exactly match the existing glibc 2.39 baseline, so no baseline was
changed. That run's sole Rust failure was the earlier 8 m regression fixture,
which legitimately fell inside the minion-first basic-reach exception; the
corrected 10.8 m case passes locally. The final push reruns normal CI.

The aggregate local `make check` is **not fully green**: one of 392 Python tests
compares with an unrelated dirty sibling `omoba-ui` handoff, whose CJK font
manifest predates the sculpture glyph fix. The committed installed font, licence
files, inventory and package budget validate; the same Python tooling passes in
clean CI. Neither the user's handoff checkout nor the strict comparison test was
changed. Raw failed and passing logs, trace artifact provenance and independent
verification are retained under the existing task's `raw/integration` directory.
