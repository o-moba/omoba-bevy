# Combat cosmetics

The [combat visual uplift TODO](plans/combat-visual-uplift.md) records the
September inventory of 16 classes and 64 skills, proposed animations/props and
acceptance checks. Adventurer expands the current roster to 17 classes and 68
active skills. Presentation belongs to the skill, independently of class and
input slot.

As of **0.40.0 / protocol 9 / standard-kits-4**, the shared resolver and runtime
also accept arbitrary unique compatible bindings, including four ultimates.
Authored unlock roles, skill-owned follow-up costs and capability requirements
remain enforced. The client metadata adapter supplies names, icons, aiming,
cards and cooldown totals from that accepted kit. Local 0.40 validation passes; see [the contract](equipped-skills.md) and
[progress](progress/2026-10-04-equipped-skills.md). Public recipe editing,
full animation phases and physical-device acceptance remain separate work.

## Class visual identity: presentation v2 (0.44.0 candidate)

Every one of the 68 skills and 17 basic attacks has a staged presentation of
its own. [`config/skills.skillfx`](../client/assets/config/skills.skillfx) is
**schema 2**: 68 skill rows, 17 class themes and 17 basic-attack rows. The IDs
a row may name are listed in [the skill vocabulary](skill-vocabulary.md), which
is generated from `client/src/skill_presentation/vocab.rs`; a new ID is a code
change with a test, never a data edit. The sections "Skill presentation
pilot" and "Full roster presentation pass" further down describe how the
system got here and are kept as history.

This is presentation only. `PROTOCOL_VERSION`, `CATALOG_REVISION`, the wire
structs and every catalog number are unchanged by it. The same candidate also
carries authoritative skill fixes under a new gameplay revision; those are
listed in the changelog and in
[the iteration note](progress/2026-10-09-class-visual-identity.md).

### What a row holds

```json
"northwall": {
  "home": "frostguard",
  "release": "shoulder_drive",
  "color": [0.34, 0.8, 1.0],
  "hdr_gain": 4.0,
  "motion": {"rate": 0.9, "start": 0.12},
  "cast": {"pattern": "toss_arc", "shape": "kite", "count": 5, "scale": 0.9,
           "lifetime": 0.4, "slots": ["primary", "secondary"]},
  "body": {"archetype": "wall", "altitude": "ground", "fill": false, "expire": "fade",
           "core": {"mesh": "kite", "slot": "primary", "size": [0.4, 1.0, 0.05], "behave": "blink_last"},
           "satellites": {"mesh": "kite", "layout": "line", "count": 4,
                          "size": [0.4, 0.8, 0.04], "slot": "primary", "behave": "rise_on_spawn"}},
  "sound": {"cast": {"base": "holy", "speed": 1.25, "slice": "tail", "gain": 0.9}}
}
```

| Block | Meaning |
| --- | --- |
| `home` | the class whose theme (`secondary` matter colour, `accent` spark colour) the row paints with |
| `release`, `windup`, `motion` | the clip of the cast and, for a telegraphed skill, the pose held before it; `rate`, `start`, `phase`, `fit_windup`, and `recast` / `recast_rate` for a second press |
| `color`, `hdr_gain`, `secondary`, `accent` | the skill colour and its brightness (1 to 8); optional overrides of the two theme colours |
| `cast` | the accent at the accepted cast: `pattern`, lead `shape`, `count`, `scale`, `lifetime`, palette `slots`; optional `move` (a dash, leap or blink cue between the two authoritative positions), `link` (a line to what a confirmed hit struck), `recast` and `recast_marker`, and `area` |
| `body` | what stands for a replicated effect: `archetype`, `altitude`, `core` / `shell` / `satellites` parts, `trail`, `marker`, `fill`, `expire`, optional `model` |
| `aux` | bodies for the secondary objects of a skill (the orb, a soul, a healing mark) |
| `impact` | the recipe drawn on a confirmed hit: `kind`, lead `shape`, `count`, `scale`, `lifetime`, `slots` |
| `sound` | `cast` (required), `recast`, `release`, `impact`: a base sample, a speed, a slice and a gain; up to two later notes |

The parser treats every rule as a hard error. A file with another
`schema_version`, an unknown field (the legacy `effect` among them), an unknown
skill, a row without `cast` or `sound.cast`, a row of a replicated effect
without `body`, a damaging row without `impact`, or a registry without a basic
row for each of the 17 classes is rejected as a whole, and the embedded copy
stays active. The packaged file applies once per launch.

Identity is tested, not promised: over the shipped data no two skills share
(motion family, body, impact), every pair differs on at least two of the three,
each class has four motion families and four body silhouettes and repeats no
impact kind, no motion family releases more than three skills, no impact kind
serves more than five, the 68 cast voices are distinct and the thrown bodies of
the basic attacks differ pairwise (`shipped_identity_ratchet`, every counter
zero). Identity belongs to the skill: a skill moved to another button or into
another class's recipe keeps its clip, body and hit.

### How a cast is staged

| Moment | What decides it | What is drawn or heard |
| --- | --- | --- |
| accepted cast | a new server action sequence of a hero the client sees | the cast accent, the release clip (or the windup pose), the cast voice, a `move` cue between the two replicated positions, the area ring for the three signed-off skills |
| telegraph | the replicated effect while its stage rule says "not yet" (`armed_gate`, `kind_gate`, `fuse`) | the body in its telegraph state, the boundary from replicated geometry, a marker (`fill_to_edge`, `arming_pips`, `remaining_ring`) |
| release | the replicated kind flip, the `armed` flag or an inferred firing inside the server's own timing window | the release clip, the release voice, a one-shot of the body |
| live effect | every snapshot that holds the effect | the body at the replicated position; parts follow their `behave`; a trail built from observed positions only |
| confirmed hit | an accepted `CombatEvent` | the impact recipe, the link, the damage number, the impact voice |
| end | the effect leaves the snapshots | silence, unless the end classifies as a true expiry, a release or a detonation |
| state | `LoadoutState` flags of a visible hero | the status visual (stun, root, parry, shield, mark, brittle, concussion, slow, camouflage, forging) and the recast marker |

### Honesty rules

These are enforced by tests and by the parser; a design row that disagrees
loses.

- **No fabricated hit.** Impact recipes, links, numbers and reactions come only
  from an accepted combat receipt. A zero-amount trap receipt draws the trap
  cue and nothing else.
- **No fabricated status.** State visuals read replicated flags. No row, impact
  kind or accent depicts a stun, root, slow, shield or mark.
- **No fabricated pierce or area.** `pierce_through` needs a skill that pierces,
  `blast` one with area damage. `cast.area` is drawn only for Thunder Pulse,
  Chain Sweep and Anvil Charge, on a first-cast edge, at the catalog radius. A
  hit that is not an area stays within 1.5 units of its receipt.
- **Boundaries equal replicated geometry** for all eight archetypes, the capsule,
  the fog-clipped sector and the cage sides included, in 3D and in the flat
  view.
- **A vanished effect is not a hit.** Travelling and auxiliary bodies always end
  silently.
- **Nothing reveals a fogged hero.** The observer starts again from the current
  state when a hero is revealed; a hidden destination draws only a puff without
  a direction at the departure; an effect whose owner the client does not hold
  drives no motion, tether, release or direction. A receipt whose source is
  withheld is a burst without a direction: a ring where a melee hit has its
  slash, no link to an attacker, drawn and heard once.
- **Animation never moves the authoritative root.** Every derived clip keeps
  hips X/Z at zero; effect roots stay at the replicated position.
- **No added gameplay delay.** For a clip released on the cast edge the contact
  pose comes at most 0.15 s after the accepted cast
  (`(contact - start * duration) / rate`); nothing waits for a clip marker and
  no input is delayed.

### Motion

Rows name clips of the shared library
([humanoid motion](humanoid-motion.md), 55 clips). `motion.phase` is derived
from the skill and may only be restated:

- `instant`: the release clip starts on the accepted cast.
- `warn_fire` (Horizon Ray, Horizon Wave): the windup pose is held while the
  hero's own warning exists and the release follows its change of kind. The
  pose and the facing survive a basic attack or a recast accepted in between;
  the release plays once, and never because a warning vanished. A bolt first
  seen more than 0.25 s after it fired plays no release.
- `fuse` (Furnace Breath, Orbital Collapse): the windup starts on the accepted
  cast and is held while the hero's own telegraph exists, 1.2 s at most when
  the telegraph is never seen; the release follows its inferred firing. At
  sandbox speed 2.0 a fuse can release without its clip.
- `parry` (Mirror Guard): the windup is held while the hero parries and the
  release follows the riposte.

`rate` and `start` let two skills share a clip and still read differently;
`fit_windup` fits a non-looping windup to the skill's telegraph time. Eight skills have
a `recast` clip of their own. Basic attacks read their motions from the 17
basic rows; a row may list two motions that alternate.

`flying_knee`, `aim_loose_r` and `two_hand_push` are stand-ins for a kick, a bow
release and a shield bash: the source library has none of the three. Rows that
use them (Thunder Kick, the Ranger basic attack and Shield Bash among them)
are honest about the timing, not about the anatomy.

### Bodies

A body is assembled from a library of shared meshes around a boundary that the
engine draws itself from the replicated position, end, radius and facing. Rows
never draw a boundary and cannot move it.

- Sizes are full extents in units of the effect: a zone `core` of `1.0` spans
  half the radius on each side of the centre. The thinnest authored extent of a
  flat mesh is its normal.
- A `lane` part's x and y are multiples of the replicated half-width and its z
  is a share of the received segment; `line` satellites spread over that
  segment however long it is. The inside of a cone is filled with a kite.
- The low bars of a cage reach 0.4 outside the pentagon, which is the band the
  server uses for its sides; a consumed side is removed.
- A long body grows from the hand: its nose is the replicated position and it
  is as long as the distance flown, so a lance or a fireball is short on its
  first frames.
- Effect and projectile parts cast and receive no shadows. Area fills are a
  tint of the skill hue.

Northwall shows what it stops: each rise of the replicated interception count
lights the wall, pops a star on its keystone and throws chips along it, and
once the first, free block is spent the keystone stands broken. Where the shot
struck is not replicated, so nothing in the picture names a side. A Fault Line
pillar outlives its owner on the server and crumbles when its time ends.

### Legacy projectile forms

The 13 projectile abilities of the five legacy kits and the ranged basic
attacks get their bodies from
[`combat_visuals.json`](../client/assets/config/combat_visuals.json), where a
profile may now name a `form`, a `silhouette` and a `presentation` (see
"Legacy projectile cosmetics" below).

- `projectile`: a thrown body at chest height with a trail.
- `wave`: a ground-hugging body with no weapon model, trail or puffs; it needs
  a form and may be at most 1.3 units wide after scale (a `volley` 0.6).
- `melee_contact`: no thrown body. The Warrior and Warden basic attacks draw a
  short reach streak that follows the server projectile, and only for the
  basic attack of a hero whose class the client knows. The mechanics are
  unchanged: both kits still deal their basic damage through a homing server
  projectile, and a real melee for them is an open owner decision.

A profile drawn with a form leaves no flight puff, and a profile without a
trail keeps no trail sample. The Wildspark launcher round is the form body (a
tumbling canister); the rocket model belongs to Last Spark alone.

### Aim previews, state visuals, recast markers

Aim previews of all 48 modular skills are derived from one table that mirrors
the server rule (`Preview shapes` in the vocabulary): the real half-width,
radius, half-angle, landing point and unit pick. A unit pick marks the unit the
server would pick, with the server's kinds: heroes only for Fourfold Duel,
Patient Curse and Orbital Guard, no structure for Thunder Kick. A landing or a
pick the server would refuse is painted in the refusal colour. A recast window
previews what the recast does (the range ring of Thorn Volley, the gated lane
of Mountain Echo) or nothing.

In 3D the ten replicated states are mesh visuals on the hero, at most four
parts per hero; the flat view keeps its gizmo rings, and a slow has a small
ground ring whenever no mesh shows it. Seven skills show a recast marker at the
feet (`ring_pips`, `orbit_motes`, `ground_arrows`) for as long as the replicated
`can_recast` flag is set and, for Mountain Echo, the hero stands inside the
server's gate.

### Flat view

Sprite2d has no bodies and no per-skill hero motion. It draws the same accents
and hit recipes as flat particles, the exact replicated boundary of every
effect as gizmo lines in the skill colour (capsule, sector, collapse ring with
a progress ring, cage sides), a crack across a wall that has stopped a
projectile, and the state rings. A true 2D effect renderer is not part of this
pass.

### Budgets

Read from data and tested (`budget_from_data`): an accent has at most 8
particles and lasts at most 0.5 s, a move cue 8 and 0.6 s, an impact 12 and
1.7 s; a body has at most 12 parts (18 for a cooldown of 40 s or more); a hero
shows at most 4 state parts; the material rule allows 272 materials (the
shipped rows use 256), the library has 24 shared meshes, at most 2 effect
lights burn at once, and the part budget is 400. The particle pool (256), the
burst and the bloom settings are unchanged. These are counts, not a measured
frame time: no phone was measured.

### The 68 rows

Generated from the shipped files on 2026-10-09. "Body" is the archetype of the
replicated effect, or the projectile form of a legacy ability; "-" means the
skill has no world object of its own (its read is the motion, the accent and
the hit).

| Class | Key | Skill | Motion | Cast accent | Body | Hit |
| --- | --- | --- | --- | --- | --- | --- |
| Warrior | Q | Shield Bash (`shield_bash`) | `two_hand_push` | `thrust_line` / `kite` | projectile `disc_skim` + `kite` | `thud_ring` / `kite` |
| Warrior | W | Battle Rally (`battle_rally`) | `rally_raise` | `rising_motes` / `chevron` | - | - |
| Warrior | E | Heroic Strike (`heroic_strike`) | `slash_down` | `muzzle_burst` / `crescent` | projectile `wavefront` + `crescent` | `slash_cut` |
| Warrior | R | Rampage (`rampage`) | `spin_cleave` | `arc_sweep` / `arc` | projectile `tumbler` + `diamond` | `cross_cut` |
| Mage | Q | Arc Bolt (`arc_bolt`) | `cast_thrust_r` | `muzzle_burst` / `arc` | projectile `twin_helix` + `star` | `ring_burst` / `ringlet` |
| Mage | W | Mana Surge (`mana_surge`) | `draw_in` | `inward_gather` / `chevron` | - | - |
| Mage | E | Frost Lance (`frost_lance`) | `cast` | `thrust_line` / `star` | projectile `dart` + `shard` | `star_shards` / `star` |
| Mage | R | Pyroblast (`pyroblast`) | `two_hand_push` | `muzzle_flash` / `drop` | projectile `comet` + `drop` | `ember_puff` / `drop` |
| Ranger | Q | Quick Shot (`quick_shot`) | `burst_fire` | `muzzle_flash` / `arc` | projectile `dart` + `star` | `splinter` / `streak` |
| Ranger | W | Field Dressing (`field_dressing`) | `kneel_plant` | `spiral_up` / `drop` | - | - |
| Ranger | E | Piercing Arrow (`piercing_arrow`) | `sky_shot` | `muzzle_burst` / `chevron` | projectile `dart` + `chevron` | `flash_star` / `star` |
| Ranger | R | Longshot (`longshot`) | `shot_heavy` | `thrust_line` / `streak` | projectile `dart` + `diamond` | `facet_pop` / `diamond` |
| Cleric | Q | Smite (`smite`) | `rally_raise` | `muzzle_burst` / `cross` | projectile `disc_skim` + `cross` | `flash_star` / `cross` |
| Cleric | W | Renew (`renew`) | `spell_finish` | `rune_mark` / `ringlet` | - | - |
| Cleric | E | Divine Favor (`divine_favor`) | `point_command` | `inward_gather` / `star` | - | - |
| Cleric | R | Guardian's Blessing (`guardians_blessing`) | `hover_pulse` | `spiral_up` / `crescent` | - | - |
| Warden | Q | Feral Swipe (`feral_swipe`) | `jab_cross` | `double_arc` / `claw` | projectile `volley` + `claw` | `claw_rake` / `claw` |
| Warden | W | Barkskin (`barkskin`) | `guard` | `inward_gather` / `drop` | - | - |
| Warden | E | Hunter's Mark (`hunters_mark`) | `hurl_overhand` | `muzzle_burst` / `drop` | projectile `dart` + `cone` | `splinter` / `chevron` |
| Warden | R | Primal Maul (`primal_maul`) | `cleave_slam` | `ground_slam` / `drop` | projectile `wavefront` + `chevron` | `thud_ring` / `drop` |
| Dawnweaver | Q | Gleaming Snare (`dawn_bind`) | `cast_thrust_r` | `muzzle_burst` / `streak` | `traveller` | `facet_pop` / `star` |
| Dawnweaver | W | Returning Aegis (`dawn_barrier`) | `toss_underhand` | `muzzle_flash` / `kite` | `orbiter` | - |
| Dawnweaver | E | Luminous Field (`dawn_field`) | `kneel_plant` | `ground_ring` / `star` | `zone` | `ring_burst` / `star` |
| Dawnweaver | R | Horizon Ray (`dawn_ray`) | `spell_prepare` → `cast` (`warn_fire`) | `inward_gather` / `streak` | `lane` | `pierce_through` / `streak` |
| Wildspark | Q | Switchfire (`wild_switch`) | `reload_snap` | `toss_arc` / `star` | - | - |
| Wildspark | W | Shockline (`wild_zap`) | `thrust_lunge` | `muzzle_flash` / `claw` | `traveller` | `spark_fork` / `chevron` |
| Wildspark | E | Snapline (`wild_traps`) | `ground_shot` | `muzzle_burst` / `ringlet` | `prop` | `chain_snap` / `chevron` |
| Wildspark | R | Last Spark (`wild_rocket`) | `shot_heavy` | `ground_slam` / `streak` | `traveller` | `blast` / `star` |
| Cinderforge | Q | Fault Line (`fault_line`) | `overhead_plant` | `strike_line` / `diamond` | `prop` | `splinter` / `diamond` |
| Cinderforge | W | Furnace Breath (`furnace_breath`) | `fist_guard_loop` → `two_hand_push` (`fuse`) | `inward_gather` / `diamond` | `sector` | `spark_fork` / `drop` |
| Cinderforge | E | Anvil Charge (`anvil_charge`) | `dive_lunge` | `ground_slam` / `diamond`, move `charge_dust` | - | `shard_burst` / `diamond` |
| Cinderforge | R | Mountain Echo (`mountain_echo`) | `raise_from_earth` | `rising_motes` / `diamond` | `traveller` | `thud_ring` / `diamond` |
| Edgeweaver | Q | Edge Lunge (`edge_lunge`) | `thrust_lunge` | `thrust_line` / `streak`, move `afterimage` | - | `slash_cut` / `slash` |
| Edgeweaver | W | Mirror Guard (`mirror_guard`) | `blade_ready_loop` → `slash_rising` (`parry`) | `inward_gather` / `kite` | `prop` | `cross_cut` / `slash` |
| Edgeweaver | E | Twin Tempo (`twin_tempo`) | `blade_flourish` | `double_arc` / `arc` | - | - |
| Edgeweaver | R | Fourfold Duel (`fourfold_duel`) | `slash_down` | `rune_mark` / `chevron` | - | - |
| Stormfist | Q | Echo Strike (`echo_strike`) | `punch` | `muzzle_flash` / `arc`, move `afterimage` | `traveller` | `spark_fork` / `arc` |
| Stormfist | W | Anchor Step (`anchor_step`) | `leap_land` | `shield_flash` / `kite`, move `leap_arc` | - | - |
| Stormfist | E | Thunder Pulse (`thunder_pulse`) | `ground_pound` | `ground_ring` / `ringlet` | - | `ring_burst` / `ringlet` |
| Stormfist | R | Thunder Kick (`thunder_kick`) | `flying_knee` | `fan_spray` / `chevron` | - | `thud_ring` / `star` |
| Veilstalker | Q | Thorn Volley (`thorn_volley`) | `hurl_overhand` | `muzzle_flash` / `drop` | `traveller` | `splinter` / `drop` |
| Veilstalker | W | Patient Curse (`patient_curse`) | `cast` | `rune_mark` / `crescent` | - | - |
| Veilstalker | E | Shadow Lash (`shadow_lash`) | `slash_rising_m` | `arc_sweep` / `slash`, move `afterimage` | - | `claw_rake` / `slash` |
| Veilstalker | R | Nightfall (`nightfall`) | `backflip_retreat` | `fan_spray` / `drop`, move `veil_step` | - | `cross_cut` / `slash` |
| Emberveil | Q | Wandering Ember (`wandering_ember`) | `toss_underhand` | `toss_arc` / `drop` | `orbiter` | `ember_puff` / `drop` |
| Emberveil | W | Kindled Wisps (`kindled_wisps`) | `hover_pulse` | `rune_mark` / `arc` | - | `flash_star` / `star` |
| Emberveil | E | Heart Tether (`heart_tether`) | `draw_in` | `thrust_line` / `drop` | `traveller` | `drain_wisp` / `drop` |
| Emberveil | R | Flame Dance (`flame_dance`) | `vault_flip` | `arc_sweep` / `crescent`, move `whirl_step` | - | `slash_cut` / `crescent` |
| Orbitwright | Q | Orbital Command (`orbital_command`) | `point_command` | `toss_arc` / `glow` | - | `facet_pop` / `ringlet` |
| Orbitwright | W | Orbital Field (`orbital_field`) | `spell_finish` | `none` | `zone` | `ring_burst` / `ringlet` |
| Orbitwright | E | Orbital Guard (`orbital_guard`) | `cast_thrust_r` | `shield_flash` / `ringlet` | - | `glow_pop` / `glow` |
| Orbitwright | R | Orbital Collapse (`orbital_collapse`) | `levitate_loop` → `draw_in` (`fuse`) | `rune_mark` / `arc` | `zone` | `blast` |
| Riftshot | Q | Rift Needle (`rift_needle`) | `burst_fire` | `muzzle_burst` / `diamond` | `traveller` | `glow_pop` / `diamond` |
| Riftshot | W | Rift Seal (`rift_seal`) | `sky_shot` | `toss_arc` / `ringlet` | `traveller` | `facet_pop` / `ringlet` |
| Riftshot | E | Rift Step (`rift_step`) | `leap_land` | `rising_motes` / `streak`, move `blink_pair` | - | `flash_star` / `diamond` |
| Riftshot | R | Horizon Wave (`horizon_wave`) | `aim_hold_loop` → `shot_heavy` (`warn_fire`) | `inward_gather` / `crescent` | `traveller` | `pierce_through` / `chevron` |
| Chainkeeper | Q | Iron Hook (`iron_hook`) | `hurl_overhand` | `thrust_line` / `chevron`, move `charge_dust` | `traveller` | `chain_snap` / `chevron` |
| Chainkeeper | W | Guiding Lantern (`guiding_lantern`) | `place_quick` | `rising_motes` / `drop` | `zone` | - |
| Chainkeeper | E | Chain Sweep (`chain_sweep`) | `spin_cleave` | `ground_ring` / `chevron` | - | `thud_ring` / `chevron` |
| Chainkeeper | R | Iron Boundary (`iron_boundary`) | `ground_pound` | `toss_arc` / `arc` | `cage` | `shard_burst` / `arc` |
| Frostguard | Q | Winter Shard (`winter_shard`) | `punch` | `muzzle_flash` / `diamond` | `traveller` | `shard_burst` / `diamond` |
| Frostguard | W | Sheltering Leap (`sheltering_leap`) | `dive_lunge` | `rising_motes` / `star`, move `whirl_step` | - | - |
| Frostguard | E | Northwall (`northwall`) | `shoulder_drive` | `toss_arc` / `kite` | `wall` | - |
| Frostguard | R | Winter Divide (`winter_divide`) | `overhead_plant` | `fan_spray` / `diamond` | `lane` | `star_shards` / `streak` |
| Adventurer | Q | Deadly Blow (`dagger_deadly_blow`) | `dagger_stab` | `thrust_line` / `chevron` | - | `slash_cut` / `slash` |
| Adventurer | W | Bluff (`dagger_bluff`) | `dagger_feint` | `muzzle_flash` / `star` | - | - |
| Adventurer | E | Backstab (`dagger_backstab`) | `dagger_backstab` | `rake_triple` / `slash` | - | `flash_star` / `star` |
| Adventurer | R | Lethal Blow (`dagger_lethal_blow`) | `dagger_heavy_thrust` | `double_arc` / `crescent` | - | `cross_cut` / `crescent` |

Basic attacks: the six melee cores (Cinderforge, Edgeweaver, Stormfist,
Veilstalker, Frostguard, Adventurer) throw nothing and show the hit of their
row; Warrior and Warden draw the reach streak; the ranged cores throw a form of
their own (Mage a tumbling star, Cleric a comet, Dawnweaver a twin helix,
Emberveil a spinning petal, Orbitwright a spinning cog, Riftshot a tracer dart,
Chainkeeper a comet of links, Wildspark a bullet dart or the tumbling launcher
round); the Ranger keeps its arrow.

### Capture and reports

One English 1280 x 720 desktop viewport, `models3d`, a hidden window rendered
to an image (so a locked screen still gives frames), the Combat Sandbox on
loopback with real cast commands:

```sh
cargo build -p client -p server
python3 scripts/capture_standard_skills.py --phases --offscreen --timeout 300 \
  --client-bin "$CARGO_TARGET_DIR/debug/client" --server-bin "$CARGO_TARGET_DIR/debug/server" \
  --assets client/assets --output /tmp/skills --hero stormfist
python3 scripts/build_skill_contact_sheets.py /tmp/skills
```

A phase run writes `0-idle.png` and three state-gated stills per skill (windup,
release, impact or settled) and judges each still against the registry and the
motion library of its asset root: the clip that carries the pose, the archetype
and boundary of every replicated effect, the particle and part budgets, the
kind of the impact recipe, the state visuals, the fingerprint of the registry
the client loaded. `capture-run.json` lists, per slot, what the stills show
next to what the row says (`slots`), and every still records the replicated
effects, the hero's position, the live particles by source and the voices that
were asked for. Options: `--flight` (bodies in flight and the basic attack),
`--aim` (aim previews), `--interleave` (a basic attack during a windup),
`--displace` (a caster moved during its telegraph), `--block` (a wall that
stops a projectile), `--mixed-recipe` (four skills of four classes on one
hero), `--visual-mode sprite2d`, `--skillfx` / `--combat-visuals` (overlay a
registry file), `--avatar`, `--release-at`. The sheet builder writes one sheet
per class, a wall of the 68 release stills and a look-alike report (difference
hashes of the release crops and the changed-pixel "energy" of each still
against the idle still).

Ally-targeted skills are cast on the caster or on an allied minion in these
runs, the target does nothing unless `--block` asks it to, and the sandbox
gives the hero's team sight of everything, so no capture shows a hit from an
unseen hero. A pass is a statement about stills of one viewport; it says
nothing about motion, sound, other languages, a phone or a match with people.

### Where the new look is weaker

Measured against a baseline taken with the standing-hero animation fix (three
runs a side, 114 comparable stills): the median changed-pixel area is 1.30 of
the old look and 17 stills are below it. Sixteen of those are an oversized old
shape that the rules above no longer allow, the same replicated shape, a pose
or noise. Stated plainly:

- **Wandering Ember** was weaker on its release still (0.87): a pale yellow orb
  on pale stone. Its accent, satellites and accent colour were tuned and it
  now measures 0.98, level with the old look.
- **Edge Lunge and Rift Step** are smaller on screen than before: a hit is held
  inside 1.5 units of its receipt where the old ring was about 3 units across.
- **The dash line of Flame Dance** is weaker than the shared cyan streak it
  replaces; the move patterns have fixed sizes in the engine.
- **Horizon Wave, Echo Strike, Rampage** are smaller because the old shapes
  were wider than the replicated geometry or drawn around one victim.
- **Pyroblast and Longshot** are smaller on their first frames, because the
  body grows from the hand.
- **Basic-attack hits** of the Warrior, Mage, Ranger, Orbitwright, Cinderforge,
  Veilstalker and of the launcher round are pale on the pale floor.
- **Longshot and Piercing Arrow** read alike on the release still (two green
  darts); their hits and motions differ.
- **The broken Northwall keystone** is partly hidden by the hero who holds the
  wall, and a block has no sound.

## Skill presentation pilot

History (2026-09-30). The pilot introduced [`config/skills.skillfx`](../client/assets/config/skills.skillfx)
with one row per skill: all 48 modular skills plus the 20 canonical legacy
ability IDs. Its schema 1 row had `release`, optional `windup`, a shared
`effect` style, RGB `color` and optional `hdr_gain`. Schema 2 (above) replaced
it: the `effect` field and the shared styles no longer exist, and a schema 1
file is rejected. What is still true of the pilot's contract: unknown fields,
skills and motion IDs are rejected, the packaged file applies once per launch
(restart after editing it), and a missing or malformed file leaves the
embedded version active.

The animation selector resolves the accepted recipe against the replicated
actor class, reads the skill at `action.slot`, then selects its profile. The
slot remains the physical binding, while the authored skill role determines
the generic Attack/Cast category. Malformed or mismatched recipes cannot select
skill or basic-attack motion by silently falling back to another kit. Moving
Dawn Ray to Q is supported by authority and presentation; its unlock remains
level 6. The old `WrongSlot` restriction is removed, while duplicate and
capability checks remain. Local Combat Test configuration can author a recipe;
there is no public class editor.

The table shows the pilot as it shipped then. The clips `interact`,
`pistol_reload` and `roll` have since been retired and every row has changed;
the current rows are in "The 68 rows" above.

| Skill | Motion | 3D presentation |
| --- | --- | --- |
| Dawn Bind | `cast` | Golden lance, streaks and team accent |
| Dawn Barrier | `interact` | Returning orb with intersecting shield rings |
| Dawn Field | `spell_finish` | Filled field, exact-radius boundary, orbiting light motes |
| Dawn Ray | `spell_prepare` → `cast` | Corridor warning → luminous beam, on server phase |
| Wild Switch | `pistol_reload` | Weapon-mode gesture; existing mode HUD |
| Wild Zap | `pistol_shoot` | Cyan bolt with streaks |
| Wild Traps | `interact` | Blender trap props, spinning while arming, radius outline |
| Wild Rocket | `pistol_shoot` | Blender rocket with exhaust streaks |

Repeater basic attacks use `pistol_shoot`. Confirmed hit particles use the
visible caster's skill color; hidden sources keep generic feedback. All skills
retain server damage, range, cooldown and movement behavior. In 2D the
existing renderer stays active.

World effects retain replicated IDs, follow received positions (including fog
clipping), and disappear silently on omission. Round and 2D-mode changes clear
instances. At most `MAX_ACTIVE_EFFECTS` objects are considered per snapshot.
Meshes and materials are shared; missing GLB assets keep a procedural fallback.
The Blender sources and deterministic exporter are in
[`assets-src/skills`](../assets-src/skills/README.md).

Ray preparation can arrive after the action and holds until the real beam,
also through a basic attack or a recast accepted in between. Warning
disappearance never fabricates a release. Repeated snapshots do not restart
the clip, and death/respawn take priority. Four motion phases and per-row
playback rate and start exist now (see "Motion" above); body masks are still
TODO. Semantic hand sockets
now have a [Warrior handheld pilot](handheld-weapons.md); finger posing, two-handed
grips and IK remain open. Teleport/jump/fly movement policies and a generic
timeline editor are not implemented. Normal animation changes now blend over
120 ms; death, round/model changes and sandbox previews cut immediately.

Reproduce the native English 1280×720 scripted sandbox check:

```sh
python3 scripts/capture_standard_skills.py --client-bin /path/to/client --server-bin /path/to/server --assets client/assets --output /tmp/skill-pilot
```

It captures the persistent effects, ultimate warning/flight, held aim,
released Q/W projectile, and Dawn Ray release. The JSON records authoritative
effects, actual motion readouts and rendered effect entities. This is a
desktop capture, not physical mobile or manual multiplayer certification.

Pilot verification on 2026-09-30: native client/server build passed; both
English 1280×720 sandbox runs completed with exit code 0 and the expected
captures. Ray readout changed `spell_prepare` → `Cast`; all three traps and
the rocket reported `model_ready: true`. Final client SHA-256:
`06527c577f1982cecd8f844916352cbf6ee98581c1706a9bfad4f4449c8b2595`.
Local evidence is under `.agent/tasks/SKILL-PILOT-2026-09-30/final/` (ignored
work artifacts). Client tests: 806 passed, zero failed, one existing ignored.
The nine motion-export tests, thirty candidate-asset tests,
deterministic exporter check and both GLB inspections passed. No full release
package, additional viewport/language or physical mobile performance was tested.

## Full roster presentation pass

History (2026-09-30). The looks in the table below were replaced by the
presentation v2 rows; the lookup rules of this section still hold.

Each remaining class has four skill-owned profiles in `skills.skillfx`, with
reusable silhouettes instead of a model file per skill. Accepted recipes
always win over class defaults. The five legacy classes resolve canonical
ability IDs when no modular recipe exists. The original four-ultimate fixture
proved presentation independence only. Version 0.40 adds resolver/runtime and
client regressions for actual accepted permutations and four ultimates; its
current final validation status is recorded separately above.

| Class | Added presentation |
| --- | --- |
| Chainkeeper | Blender hook and lantern, green cast sweep, five cage sides with consumed-side removal; souls stay separate from hooks |
| Frostguard | Narrow ice bolt, defensive cast ring, directional crystal wall, immediately visible full-length fissure borders |
| Orbitwright | Blender sphere follows replicated position, orbital field, guard gesture, inward collapse ring |
| Cinderforge | Delayed basalt pillar, fire cone and embers, charge accent, moving Blender colossus |
| Edgeweaver | Blade slash, parry rings, second-strike accent, challenge cast pulse; existing vital markers retained |
| Stormfist | Punch motion and blue strike, anchor shield, pulse, heavy strike accent |
| Veilstalker | Violet needles, curse pulse, lash and retreat slash |
| Emberveil | Pulsing ember, wisp accent, tether lance, roll follow-through for dash |
| Riftshot | Thin blue needle, seal rings, blink-shot accent, real warning → travelling wave |
| Warrior | Warm shield accent, rally ring, separate strike/ultimate projectile profiles |
| Mage | Violet orb, resource pulse, cyan lance, orange fireball |
| Ranger | Green arrow profiles with distinct reach silhouettes/trails, dressing gesture |
| Cleric | Gold smite, renew pulse, favor rings, blessing pulse |
| Warden | Green claw, defensive ring, mark projectile, heavy maul accent |

The shared CC0 motion library had 20 clips at the end of this pass and has 55
now (see [humanoid motion](humanoid-motion.md)). `punch`, `guard` and
`shoulder_drive` come from Quaternius Punch_Cross, Punch_Enter and Punch_Jab;
`roll` was retired with the presentation v2 pass.
The guard and heavy strike are reusable gesture fallbacks, not bespoke shield
or kick animations. They are retargeted semantically to the existing VRM rigs;
avatar GLBs are unchanged.

Instant actions emit one small cast accent from a new server-confirmed action
sequence. First-seen, duplicate, older, hidden and dead actor states do not
replay it. These accents share the existing 256-particle budget with trails;
they never fabricate damage or an impact on the target. Windup skills keep
server warnings. Auxiliary world objects choose their actual silhouette from
the replicated effect kind, not the source ability's primary silhouette.

Run the remaining 14 classes in the same desktop camera:

```sh
python3 scripts/capture_standard_skills.py --roster --client-bin /path/to/client --server-bin /path/to/server --assets client/assets --output /tmp/roster-skills
```

This uses a live development sandbox and checks accepted Q/W/E/R sequences,
PNG readbacks and actual effect entities. It resets the sandbox between skills,
uses infinite resources/cooldowns and an invulnerable stationary enemy. Ally-only
skills target the caster in this harness; this verifies their accepted cast,
not an ally-to-ally travel sequence. It does not certify recast combos,
physical mobile performance, all skins visually, or manual multiplayer play.

The [handheld pilot](handheld-weapons.md) adds sword/hammer/scepter props, semantic
VRM sockets and a shared built-in/Ekza SDK import contract. Grip/IK corrections,
dedicated bow/kick clips and a generic server-driven jump/flight timeline remain
separate tasks. No synthetic flight
or new cast delay is added to the currently instantaneous movement mechanics.

### Facing the accepted attack

The server records optional `action_yaw` alongside the accepted action sequence,
using the basic attack target or the skill's resolved target/aim. Locomotion yaw
stays separate. Rejected casts cannot change it; self casts clear the previous
direction. Older packets without this optional field remain readable.

In 3D, both local and remote heroes face that direction after movement and
network interpolation, before transform propagation: 450 ms for basic attacks,
700 ms for skills. An authoritative Dawn Ray/Horizon Wave warning keeps the
preparation facing active until release, also when a basic attack or a recast
is accepted during the warning. This changes rotation only, respects
sandbox visual time, and does not steer movement or change damage. Initial and
duplicate snapshots do not replay it; death and round changes clear it. Sprite2d
keeps its existing behavior. Both client and server must be rebuilt/restarted
to transmit and display the new direction.

## Contrast and glow

The 3D gameplay camera uses TonyMcMapface tonemapping, mild post-saturation
(1.08), and selective additive bloom (intensity 0.12, threshold 1.0). The key
light is near-neutral daylight (sRGB 1, 0.98, 0.94) so it preserves the cool
floor instead of adding a strong yellow cast. Exposure, light intensity and
the player's lighting preferences are unchanged. Broad field fills stay
at ordinary brightness and 12% opacity; small skill cores/rims use `hdr_gain`.
White streaks use gain 5, team boundaries gain 2, and pooled combat particles
gain 2.5 throughout their fade. Unlit Bevy materials ignore emissive, so these
drawables multiply **linear base RGB**, preserving alpha. Combat drawables
ignore distance fog; server visibility still controls whether they exist.

Verdant's environment/foliage materials use cached, runtime-only copies with
a cooler slate floor, deeper water/greens and less glossy reflection. This
also grades the active procedural river's vertex colors through its material. The
palette is in `environment_palette` in `client/src/verdant3d.rs`; changing it
requires a rebuild. Source GLBs, character materials and live structures are
preserved. Sprite2d keeps its camera and particle colors. No additional
dependency, texture, light or particle count is required by this pass. Bloom
adds GPU postprocessing; mobile performance still needs device measurement.

Contrast verification on 2026-09-30: the client library suite passed (810 tests,
one existing ignored); the subsequent eight Verdant tests passed, including a
new integration check that loads the shipped GLBs and actual map cosmetics.
Both final native scenarios passed with exit code 0 at English 1280×720.
Their JSON confirms 24 cached palette copies, the tuned paving binding and
TonyMcMapface/bloom on the camera. Evidence is in
`.agent/tasks/COMBAT-CONTRAST-2026-09-30/verified/`; final client SHA-256:
`dbd7cac8f7a54050e9b51c76618cfb06094a32218882952b48888154b6b19490`.
An initial native shutdown timeout was fixed by closing the primary window
before AppExit, matching the production Exit button and other capture harnesses.
The capture script now also accepts `--timeout` and reports watchdog expiry
explicitly. Visual review covered beam, field, rocket/traps and hit/projectile
states. No physical mobile, crowded-fight performance or other locale was tested.

## Legacy projectile cosmetics

Combat presentation is configured in
[`client/assets/config/combat_visuals.json`](../client/assets/config/combat_visuals.json).
It changes rendering and animation selection only. Damage, reach, projectile
motion, cooldowns, minion roles and confirmed hits remain authoritative server
data; the schema rejects unrelated fields such as `damage` or `speed`.

The client reads this packaged JSON through Bevy's AssetServer on desktop,
Android and iOS. An independent set of built-in class profiles is embedded in
the client. Missing files, unsupported schema versions or invalid configuration
leave those defaults active. Invalid custom JSON cannot break the built-in
profiles even if the client is rebuilt with that JSON in the asset directory.
No remote URL, filesystem override or downloaded cosmetic is requested.

The registry applies a valid packaged manifest once per client launch. Restart
after changing it. Existing in-flight projectiles keep the profile they began
with. Avatar animation bindings refresh when the packaged registry becomes
available; this allows clip aliases to work even when avatar models load first.

## Profiles and lookup

Version 1 has `profiles`, `defaults`, `classes`, `avatar_overrides`,
`sprite_overrides` and `animation_aliases`. A partial manifest extends the
built-ins. A profile defines a procedural `shape`, RGBA `color`, uniform `scale`,
`trail`, `impact`, and optional `model` or `sprite`. Three more optional fields
choose the 3D body from the shared mesh library: `form` (how the body moves:
`dart`, `comet`, `disc_skim`, `tumbler`, `wavefront`, `volley`, `twin_helix`),
`silhouette` (which mesh it is made of; the form's default when absent) and
`presentation` (`projectile`, `wave` or `melee_contact`). A form beats a packaged
model. A `wave` needs a form; a `wave` wider than 1.3 units or a `volley` wider
than 0.6 after scale rejects the manifest. Without the three fields a profile is
drawn as its `shape`, as before.

The lookup order is:

1. Matching sprite ID: action-specific profile, then its `default`.
2. Matching avatar slug: action-specific profile, then its `default`.
3. Hero class: action-specific profile, then its `default`.
4. Authoritative projectile style default, then the built-in standard profile.

Actions are `basic`, `q`, `w`, `e`, `r`, and `default`. The wire's absent action
slot means `basic`; legacy slot 255 is treated the same way. Unknown slots use
`default`. Avatar slugs and sprite IDs are case-sensitive and must match the
identity actually sent by the server. Sprite overrides have priority in both
render modes so projectile and impact settings resolve consistently.

Class defaults are Warrior crescent, Ranger arrow, Mage arcane bolt and Cleric
holy bolt. Minion caster bolts and structure bolts have separate style defaults.
Projectile shapes use shared procedural meshes in 3D and colored sprite pieces
in 2D; their silhouettes work without custom model or image assets. Small team
color accents remain visible on the procedural shapes. The Warrior and Warden
basic attacks are `melee_contact`: no blade is thrown and a short reach streak
follows the server projectile. That is a picture, not a mechanic: the attack
is still a homing server projectile, and its damage arrives when that
projectile does. A projectile of an unknown owner that falls through to one of
those two style defaults keeps a thrown body.

## Example: one avatar's basic attack

Add this as a partial manifest, or merge its sections into the shipped file:

```json
{
  "schema_version": 1,
  "profiles": {
    "agnes_leaf_arrow": {
      "shape": "arrow",
      "color": [0.35, 0.95, 0.60, 1.0],
      "scale": 1.1,
      "trail": {"seconds": 0.18, "width": 0.06, "samples": 8},
      "impact": {
        "color": [0.50, 1.0, 0.70, 1.0],
        "scale": 0.8,
        "lifetime": 0.30
      },
      "model": {
        "path": "cosmetics/agnes/leaf-arrow.glb",
        "scene": 0,
        "scale": 1.0,
        "rotation_degrees": [0.0, 0.0, 0.0]
      },
      "sprite": {
        "path": "cosmetics/agnes/leaf-arrow.png",
        "frame_size": [64, 64],
        "columns": 4,
        "rows": 1,
        "first_frame": 0,
        "frames": 4,
        "fps": 16.0,
        "world_height": 1.4
      }
    }
  },
  "avatar_overrides": {"agnes": {"basic": "agnes_leaf_arrow"}},
  "animation_aliases": {
    "agnes": {
      "idle": ["Standing_Idle"],
      "walk": ["Walk_Forward"],
      "attack": ["Bow_Release"],
      "cast": ["Spell_Cast"],
      "death": ["Falling_Death"]
    }
  }
}
```

This is an authoring example, not a claim that those files or named animation
clips are bundled. Replace them with your own permitted files and exact clip
names. The procedural arrow remains available when either example asset is
missing. Omit `model` and `sprite` entirely for a fully procedural customization.

Put model/image files under `client/assets/` using the relative paths above.
Add the files and their permission/provenance notices to version control;
new asset directories may need an explicit `.gitignore` exception. Native
packaging copies versioned assets; Android and iOS package the client asset
tree. Check the actual resulting package before release. Configuration does
not grant permission to redistribute an avatar, model, image or animation.

Hero 2D artwork uses the existing [sprite roster manifest](../client/assets/sprites/manifest.json).
Each character defines locomotion/action sheets and `idle`, `run`, `attack`,
`cast`, `hit`, and `death` frame ranges. Add the art there, then use that same
character ID in `sprite_overrides` to pair its attacks with a projectile skin.
The hero roster manifest is embedded at compile time: adding a new identity
also requires updating the shared sprite ID list in `shared/src/lib.rs` and
rebuilding the client/server. The `animation_aliases` section below applies
to skeletal 3D avatars.

## Model, sprite and animation contracts

- A model is a packaged `.glb`, centered around its origin, with **+Z forward**
  and **+Y up**. Author a small effect near unit scale. `scene` selects a glTF
  scene and `rotation_degrees` corrects asset axes. The procedural shape stays
  visible until the selected scene and its dependencies load and contain mesh
  geometry; failed or empty scenes keep the procedural fallback.
- A sprite is a packaged `.png` atlas with **+X forward**, equal frame cells,
  and no inter-cell padding. Its image dimensions must exactly equal
  `frame_size × [columns, rows]`. Frames loop from `first_frame` at `fps` and
  retain the cell aspect ratio. Missing images or mismatched dimensions keep
  the procedural sprite pieces visible. Atlas handles are cached per profile.
- Animation aliases are ordered, exact, case-sensitive named glTF animation
  clips. Supported fields are `idle`, `walk`, `run`, `attack`, `cast`, and
  `death`. The player renderer tries aliases before its existing name
  heuristics. Keys are avatar slugs, or `character:<id>` for legacy characters.
  Two distinct idle and walk/run clips must resolve before an animation set
  (including its optional attack/cast/death clips) is registered. `run` aliases
  are fallbacks for the walking locomotion slot, not a separate speed state.
  No clip is manufactured if the model does not contain it.
- Paths must be asset-root-relative ASCII paths using letters, digits, `_`,
  `-`, `.`, and `/`. Absolute paths, drive prefixes, URLs, `..`, backslashes,
  and glTF `#Scene` labels in the path are rejected. Use the `scene` field.

## Bounds and verification

Version 1 permits at most 128 profiles, 256 avatar overrides, 256 sprite
overrides and 256 animation alias entries. Each override has at most six action
keys; an alias field has at most eight clip names. Unknown schema fields and
unresolved profile references reject the custom manifest.

Profile scale is 0.25–3.0. Colors are finite RGBA components in 0–1, with alpha
at least 0.25. Trail duration is 0–0.6 seconds, width 0.02–0.35 world units and
history 1–12 samples. Impact scale is 0.1–2.0 and duration 0.08–1.2 seconds.
Model scale is 0.01–4.0, scene index 0–31, and rotations −360–360 degrees.
Sprite grids are at most 16×16 cells, cells at most 1024×1024 pixels, and the
complete atlas at most 4096×4096. Sprite playback is 1–60 fps and rendered
height 0.3–4.0 units. These limits constrain settings; authors must also keep
model geometry and textures economical for mobile devices.

3D trails use short histories of actual network positions; a profile without
a trail keeps no history. A large position correction clears the history.
Only a thrown `shape` body of a magic style leaves glow puffs in flight; a
form, a `wave` and a `melee_contact` never do. Removing a projectile removes its presentation;
it does not emit an impact. Impacts and floating damage require confirmed server
combat events. The 2D minion staff/crystal and shield/blade cues distinguish
caster and melee roles without replacing the existing actor atlas.

Before contributing, validate the manifest, exercise its avatar/class/action
lookup, check both presentation modes, test missing custom assets, and inspect
readability at desktop and mobile camera sizes. Registry, resource lifecycle,
atlas fallback and role-cue regressions live beside the corresponding Rust
modules. A native touch preview is useful evidence, but does not replace a
physical Android/iOS performance and readability check.
