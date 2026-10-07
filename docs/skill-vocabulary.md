# Skill presentation vocabulary

Generated from `client/src/skill_presentation/vocab.rs`. Do not edit by hand; run
`OMOBA_WRITE_SKILL_VOCABULARY=1 cargo test -p client --lib vocabulary_doc_is_current`.

These are the only IDs a row of `client/assets/config/skills.skillfx` (schema 2) or a
profile of `client/assets/config/combat_visuals.json` may name. Motion IDs are the clips
of `client/assets/animations/humanoid-motion-v1.json` (see `docs/humanoid-motion.md`).

## Motion phases

`motion.phase`. An explicit value must equal the phase derived for the skill, and a phase other than `instant` needs a `windup`.

| ID | What the player sees |
| --- | --- |
| `instant` | the release clip starts on the accepted cast |
| `warn_fire` | the windup is held while the hero's own warning exists; the release follows its change of kind |
| `fuse` | the windup starts on the accepted cast and is held while the hero's own telegraph exists, 1.2 s at most when that is never seen; the release follows its inferred firing |
| `parry` | the windup is held while the hero parries; the release follows the riposte |

## Body archetypes

`body.archetype`. The boundary parts of an archetype are drawn by the engine from replicated geometry.

| ID | What the player sees |
| --- | --- |
| `traveller` | an object moving along a heading |
| `orbiter` | a round object that follows, hovers or returns |
| `zone` | a circle on the ground |
| `lane` | a strip from the effect position to its end |
| `sector` | a cone opening from the effect position toward its end |
| `prop` | a placed object with a trigger or collision radius |
| `wall` | a wall across the facing |
| `cage` | a pentagon with breakable sides |

## Silhouette meshes

`mesh` of a body part and `silhouette` of a legacy projectile form.

| ID | What the player sees |
| --- | --- |
| `ball` | low-poly sphere |
| `cone` | five-sided cone |
| `block` | cube; also the bar primitive |
| `ring` | thin flat annulus at unit radius |
| `torus` | thin 3D ring: gyro ring, chain link, shackle |
| `shard` | elongated octahedron: crystal, needle, ice |
| `kite` | flat shield outline |
| `star` | flat four-point star |
| `chevron` | flat open V |
| `diamond` | flat rhombus |
| `arc` | flat half annulus |
| `drop` | flat teardrop: flame, leaf, feather, thorn when stretched |
| `cross` | flat plus sign |
| `crescent` | flat thick blade moon |
| `claw` | flat three-tine rake |

## Models

`body.model`.

| ID | What the player sees |
| --- | --- |
| `rocket` | the Wildspark rocket |
| `trap` | the Wildspark trap |
| `hook` | the Chainkeeper hook |
| `lantern` | the Chainkeeper lantern |
| `orb` | the Orbitwright sphere |

## Satellite layouts

`body.satellites.layout`.

| ID | What the player sees |
| --- | --- |
| `orbit` | copies on a horizontal ring around the centre, turning |
| `halo` | copies on a vertical ring around the heading axis |
| `helix` | copies corkscrewing behind a moving body |
| `column` | copies stacked upward |
| `quad_x` | copies in an X around the centre |
| `rim` | copies evenly on the boundary circle, static |
| `line` | copies evenly along the strip |
| `stagger` | copies alternating left and right along the strip |
| `fan` | copies spread across the cone |

## Trails

`body.trail`.

| ID | What the player sees |
| --- | --- |
| `none` | no trail |
| `ribbon` | two stretched bars between the last observed positions |
| `motes` | three small parts on the last observed positions |
| `chevrons` | three flat chevrons on the last observed positions |
| `links` | up to six chain links on the instance's own observed positions |

## Markers

`body.marker`.

| ID | What the player sees |
| --- | --- |
| `none` | no read-out |
| `remaining_ring` | a ring that shrinks with the remaining time |
| `arming_pips` | pips that light when the effect arms |
| `fill_to_edge` | an inner shape that grows to the boundary with telegraph progress |
| `owner_tether` | one thin bar from the body to its visible owner |

## Part behaviours

`behave` of a body part.

| ID | What the player sees |
| --- | --- |
| `steady` | does not move |
| `spin` | turns about the vertical axis |
| `pulse` | breathes |
| `bob` | floats up and down |
| `flicker` | flickers |
| `tumble` | turns end over end |
| `hide_in_telegraph` | hidden while the effect is a telegraph |
| `only_in_telegraph` | shown only while the effect is a telegraph |
| `rise_on_arm` | flat until the effect arms, then full height |
| `blink_last` | blinks in the last half second of the remaining time |
| `gyro` | orb only: rings spin level at rest and tilt along the travel |
| `only_after_renew` | hidden until this instance was seen to renew |
| `rise_on_spawn` | grows over the first 0.2 s of the instance's life |

## Altitudes

`body.altitude`.

| ID | What the player sees |
| --- | --- |
| `ground` | on the ground plane |
| `chest` | at chest height |
| `high` | above head height |

## Expire kinds

`body.expire`.

| ID | What the player sees |
| --- | --- |
| `none` | silent removal |
| `fade` | soft desaturated dissipation that does not read as a hit |
| `crumble` | the structure breaks into falling pieces |
| `discharge` | the telegraphed shape flashes as it releases |
| `detonate` | the zone bursts outward to its boundary |

## Cast accent patterns

`cast.pattern` and `cast.recast`. A recast accent always draws the default lead shape. `rune_mark` must name a lead of `arc`, `chevron`, `crescent` or `ringlet`, so it cannot be a recast accent. A row with a windup may use only a charge pattern.

| ID | What the player sees | Default lead shape | Base extent (units) | Charge pattern |
| --- | --- | --- | --- | --- |
| `arc_sweep` | one blade arc sweeping across the front | `crescent` | 1.6 | no |
| `double_arc` | two crossing arcs, 80 ms apart | `slash` | 1.6 | no |
| `rake_triple` | three short parallel rakes forward | `claw` | 1.4 | no |
| `thrust_line` | a forward streak from hand height along the aim | `streak` | 2.0 | no |
| `muzzle_flash` | one shape and a glow at hand height, pointing forward | `chevron` | 1.0 | no |
| `muzzle_burst` | three staggered shapes forward | `chevron` | 1.4 | no |
| `fan_spray` | shapes thrown in a forward fan | `drop` | 1.6 | no |
| `ground_ring` | a ring expanding on the ground around the caster | `ringlet` | 1.2 | no |
| `ground_slam` | a ground ring and shapes thrown upward at the caster | `diamond` | 1.4 | no |
| `rising_motes` | shapes rising around the caster | `glow` | 1.0 | yes |
| `inward_gather` | shapes converging on the caster | `glow` | 1.5 | yes |
| `spiral_up` | a helix of shapes around the caster | `star` | 1.0 | yes |
| `shield_flash` | one flat plate in front of the caster, facing the aim | `kite` | 1.0 | no |
| `rune_mark` | flat shapes on a small ring above the caster's head, turning | `diamond` | 0.8 | yes |
| `toss_arc` | shapes on a short upward arc ahead | `glow` | 1.5 | no |
| `strike_line` | shapes laid on the ground from the caster to the skill's own new effect | `diamond` | 0.0 | no |
| `none` | nothing; the telegraph body is the cast read | - | 0.0 | yes |

## Move patterns

`cast.move.pattern`.

| ID | What the player sees |
| --- | --- |
| `afterimage` | streak afterimages along the travelled line |
| `blink_pair` | a collapse at the origin and a burst at the arrival, nothing between |
| `leap_arc` | a dust ring at the origin, a landing ring and radial dust at the arrival |
| `charge_dust` | ground dust along the path and a thud ring at the arrival |
| `whirl_step` | shapes spiralling along the path |
| `veil_step` | a dark dissolve at the origin and a re-form at the arrival |

## Recast markers

`cast.recast_marker`.

| ID | What the player sees |
| --- | --- |
| `ring_pips` | pips on a small ring at the feet |
| `orbit_motes` | two marks circling the feet |
| `ground_arrows` | arrows pointing along the facing |

## Impact kinds

`impact.kind`. `pierce_through` needs a skill that pierces and `blast` one with area damage; `arc` may not lead `chain_snap` or `facet_pop`. Every kind closes its burst with the flash of the hit: a glow in the lead colour at the receipt.

| ID | What the player sees | Default lead shape |
| --- | --- | --- |
| `ring_burst` | an expanding ringlet with radiating glows | `ringlet` |
| `slash_cut` | one slash with sparks | `slash` |
| `glow_pop` | a glow pop with sparks | `glow` |
| `cross_cut` | two crossing slashes | `slash` |
| `claw_rake` | three parallel claw marks | `claw` |
| `pierce_through` | a streak continuing along the hit direction past the target | `streak` |
| `spark_fork` | three forked streaks | `streak` |
| `shard_burst` | shards thrown outward that fall with gravity | `diamond` |
| `flash_star` | one bright star, no debris | `star` |
| `ember_puff` | rising drops that darken | `drop` |
| `star_shards` | a static star and a few falling diamonds | `star` |
| `thud_ring` | a flat ground ringlet and low dust | `ringlet` |
| `facet_pop` | a flat shape and a ringlet that pop and vanish, no debris | `diamond` |
| `blast` | a large glow, a ringlet and debris | `glow` |
| `drain_wisp` | motes drifting from the hit toward the source | `glow` |
| `chain_snap` | chevrons snapping along the hit direction | `chevron` |
| `splinter` | short drops thrown sideways | `drop` |

## Particle shapes

`shape` of an accent, a move, a link or an impact.

| ID | What the player sees |
| --- | --- |
| `glow` | soft round glow |
| `ringlet` | thin ring |
| `slash` | curved cut |
| `streak` | thin line |
| `star` | four-point star |
| `chevron` | open V |
| `diamond` | rhombus |
| `arc` | half ring |
| `drop` | teardrop |
| `cross` | plus sign |
| `crescent` | blade moon |
| `claw` | three-tine rake |
| `kite` | shield outline |

## Palette slots

`slot` and `slots`. A basic-attack row may not name `primary`.

| ID | What the player sees |
| --- | --- |
| `primary` | the skill colour, drawn with the skill's HDR gain |
| `secondary` | the class matter colour, drawn without HDR gain |
| `accent` | the class spark colour, drawn with the skill's HDR gain |
| `white` | neutral white |

## Audio bases

`base` of a sound cue.

| ID | What the player sees |
| --- | --- |
| `melee` | the melee strike sample |
| `arrow` | the arrow sample |
| `arcane` | the arcane sample |
| `holy` | the holy sample |
| `caster` | the caster bolt sample |
| `tower` | the tower shot sample |
| `bluff` | the bluff sample; `dagger_bluff` only |

## Audio slices

`slice` of a sound cue.

| ID | What the player sees |
| --- | --- |
| `full` | the whole sample |
| `tick` | the first 0.12 s |
| `body` | 0.06 s to 0.40 s |
| `tail` | from 0.20 s on |

## Projectile forms

`form` of a profile in `combat_visuals.json`.

| ID | What the player sees |
| --- | --- |
| `dart` | one silhouette stretched along the heading |
| `comet` | a pulsing head and three shrinking afterimages of itself |
| `disc_skim` | a flat silhouette spinning about the vertical axis, skimming |
| `tumbler` | a silhouette tumbling end over end |
| `wavefront` | a wide silhouette perpendicular to the heading, hugging the ground |
| `volley` | three small silhouettes in a tight fan |
| `twin_helix` | two small silhouettes winding around the heading axis |

## Projectile presentations

`presentation` of a profile in `combat_visuals.json`.

| ID | What the player sees |
| --- | --- |
| `projectile` | a thrown body at chest height with a trail |
| `wave` | a ground-hugging body, no weapon model, no trail, no puffs |
| `melee_contact` | no thrown body; a short reach streak follows the server projectile |

## Preview shapes

Derived from the skill. Not authorable.

| ID | What the player sees |
| --- | --- |
| `none` | nothing |
| `lane` | rails of the real half-width from the caster to range |
| `lane_capsule` | the lane with round ends |
| `lane_to_point` | a narrow lane ending at the bounded aim point |
| `point_ring` | a ring of the real radius at the bounded aim point or at the orb |
| `trap_row` | the three trap rings |
| `sector` | the server's half-angle and radius from the caster |
| `self_ring` | a ring of the real radius around the caster |
| `self_pentagon` | the cage outline around the caster |
| `range_ring` | the cast-range ring |
| `dash_landing` | a line to the landing point and the strike ring there |
| `blink_landing` | a landing marker and the strike ring, no line |
| `unit_pick` | a pick ring at the aim point and a highlight on the unit the server would pick |
| `pick_then_lane` | the unit pick plus the push lane beyond the picked unit |
| `effect_origin_lane` | a lane starting at the owner's own replicated object |
| `wall_ahead` | the wall bar one unit ahead |

## Stage rules

Derived from the skill and the replicated kind. Not authorable.

| ID | What the player sees |
| --- | --- |
| `active` | live for its whole replicated life |
| `armed_gate` | a telegraph until the replicated `armed` flag is set |
| `kind_gate` | a telegraph while the replicated kind is a warning |
| `fuse` | a telegraph for its whole observed life; the release is inferred |
