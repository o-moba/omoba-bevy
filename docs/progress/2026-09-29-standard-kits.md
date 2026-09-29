# Dawnweaver, Wildspark and reusable class recipes

Version 0.29.0. Scope: two fixed playable presets plus the composition boundary
for future classes. The five existing classes and independent avatar selection
remain available. The public constructor is a later milestone.

## Implemented shape

`shared/assets/catalog/skills.json` owns eight skill definitions, with original
names, costs, cooldowns, targeting and effect parameters. The new entries in
`heroes.json` reference these IDs instead of duplicating their abilities.
`shared/src/loadout.rs` owns revisioned recipes and the pure resolver. A recipe
contains only `schema_version`, `catalog_revision`, `core`, `passive` and four
skill IDs; avatars and user-supplied combat numbers are excluded. Private fields
prevent constructing an executable `ResolvedLoadout` without validation.

The first core supplies base stats and an attack profile. Skills must fit their
slot; weapon switching requires the repeater profile. Compatible substitutions
are already executable in authoritative tests. This is a capability foundation,
not a completed balance budget or public custom-build API.

`server/src/skills/` executes seven reusable effect families: linear projectile,
returning shield, recast zone, delayed beam, weapon toggle, trap line and impact
rocket. Per-hero runtime holds cooldown-stage state, weapon mode, control,
shields and passive state; world runtime owns bounded traveling/ground effects,
marks and assist participation. Existing damage, vision, movement, bot and
lifecycle systems consume this state. Fixed presets resolve when the hero is
accepted. Runtime dispatch is by resolved skill effect, not a class-name switch.

`CastSkill` carries slot, world-XZ aim, server epoch, match ID and request sequence.
The server validates admission, life, unlocked slot, stage, resources and replay
before accepting an effect. Snapshot state includes the accepted recipe, effective
basic range/cost, recast availability, status/passive timers and fog-filtered
geometry. Protocol 3 requires matching peers. Existing packet omissions default
inertly for stored fixtures; this does not promise live compatibility with v2.

The client uses these snapshots for mode/recast/status feedback and combat
geometry. Desktop holds a new skill key to preview aim and releases to cast;
touch and controller feed the same aimed command. Original icons and embedded
English, Russian and Simplified Chinese descriptions are included. Authoritative
practice supports the kits; in-process offline practice explicitly limits its
picker and admission to the five classes that simulator supports.

## Kits and initial tuning

Dawnweaver: Radiance marks; Gleaming Snare (two rooted targets); Returning Aegis
(outgoing and returning shields); Luminous Field (slow, free recast or expiry
blast); Horizon Ray (warning, line damage and mark detonation/reapplication).

Wildspark: Momentum from qualifying takedowns; Switchfire (stacking repeater or
Mana-consuming splash rockets); Shockline (first-hit damage, slow and reveal);
Snapline (three armed hero-triggered traps); Last Spark (long-range hero-collision
rocket, distance/missing-health scaling). Initial values are authored tuning
hypotheses. Geometry and control durations are fixed across ranks; new Q uses
spell haste. Legacy free universal dash/haste is absent on these two kits.

## How the later constructor fits

1. Expose the shared skill/core/passive catalogue and its immutable revision to
   the website and game UI. Generate their schema from the actual contract.
2. Save a `BuildRecipe` through the account service and validate it server-side
   with this resolver plus the future power-budget/mode rules. Return a stable
   recipe ID/revision; keep avatar selection a separate field.
3. Resolve and freeze that recipe in prematch, acknowledge the accepted result to
   clients, and record the revision in match history. Change the fixed-preset hotbar
   name/icon lookups to the accepted recipe skill IDs before exposing mixed kits.
   Both website and in-game
   editors submit the same data contract; neither uploads executable code.
4. Fetch/equip before the match and retain a known-good local catalogue. Catalogue
   updates must be versioned; no arbitrary mid-match ability/stat replacement.

The current build embeds the catalogue. It does not yet fetch recipes from a
website, equip arbitrary network recipes, change base HP freely, or expose a
skill editor.

## Try the classes

Run `make practice` in the game checkout, open the class picker and choose either
preset with any bundled avatar. This launches the local authoritative server and
bots. Desktop: hold Q/W/E/R to preview, aim with the pointer and release to cast;
Esc cancels. The field's second E is free while its recast indicator is active.
Wildspark Q switches basic weapons. Skills still unlock at levels1/2/4/6; use
normal progression or the existing opt-in combat lab to inspect every skill.

## Verification

Completed against the final code in `.agent/tasks/ST-TWO-KITS-2026-09-29/`:

- `make check`: formatter, all-target workspace Clippy, client without QA tools,
  **1,302 Rust tests passed / 39 existing ignored**, **149 script + 43 iOS-tool
  unit tests passed**. The iOS-tool tests use mocks; no phone was installed.
- Shared103, server343 and client811 passed, including mixed recipes, full kit
  outcomes, cadence, resources/caps, fog, lifecycle and fast input buffering.
- `scripts/verify_standard_kits.py`: **22/22 passed** on the final server binary
  with two real local UDP peers. Verified all eight casts, public state equality,
  owner-only request counters, replay rejection and reconnect state.
- Native English1280×720: eight screenshots across the two presets. Root reviewed
  the seven-card selection, independent avatar, field/traps, HUD mode/recast,
  effects and held aim; both scripted key releases produced an accepted server
  projectile. Initial clipped class cards and open debug panel were corrected.
  First-frame transient screenshots can precede their DTO by one rendered frame;
  exact warning/projectile lifetime and damage are proven by server/UDP checks.

This is a working first gameplay implementation with simple effect geometry.
Balance, final VFX/audio polish, physical devices and human team-match usability
remain separate playtests. No broad visual matrix or production deployment was
performed. Failed intermediate checks and their fixes remain in `problems.md`.

## Completed implementation TODO

- [x] Shared skill catalogue, fixed presets and pure recipe resolver.
- [x] Complete Dawnweaver and Wildspark passive/basic/Q/W/E/R runtime.
- [x] Authoritative protocol, status/damage/control/vision and lifecycle handling.
- [x] Selection, icons/text, aimed input, mode/recast feedback and simple VFX.
- [x] Authoritative bots/practice and explicit offline compatibility gate.
- [x] Unit/lint/script gates, two-client proof and focused native review.
- [x] Record the architecture and future constructor work below.

## TODO after this implementation

- [ ] Design and playtest a power budget, exclusions and beginner presets for mixed builds.
- [ ] Add account-backed recipe save/version/equip and catalogue delivery.
- [ ] Add the in-game and website editors using the same validated contract.
- [ ] Switch fixed-preset client hotbar/name/icon lookups to accepted recipe skill IDs for mixed kits.
- [ ] Record accepted recipe/catalogue revisions in career history before custom matches ship.
- [ ] Reach offline simulation parity before enabling the new kits there.
- [ ] Playtest balance and readability in real team matches; improve final art/audio and animation cues.
- [ ] Verify physical touch/controller devices and broader release configurations separately.
- [ ] Add the remaining classes only after reusable effects and two-kit playtests stabilize.
