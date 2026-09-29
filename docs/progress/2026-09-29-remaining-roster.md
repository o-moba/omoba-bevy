# Remaining standard roster — implementation and TODO

Version 0.30.0; protocol 4; catalogue `standard-kits-2`. Nine more presets use the
same ID-only loadout boundary as Dawnweaver and Wildspark. There are now eleven
standard kits, 44 standard skills and five retained legacy classes. An avatar
still contributes no combat numbers. The public class constructor remains a
separate milestone.

## Implementation progress

- [x] Cinderforge: defensive passive, safe field forging, one permanent equipment
  upgrade per eligible hero, delayed solid pillar, Brittle breath, collision
  charge and redirectable incoming colossus.
- [x] Edgeweaver: directional vitals, lunge/refund, damage/control parry, paired
  attacks and a four-side challenge with an allied healing field.
- [x] Stormfist: fixed Energy pool, two attack refunds, marked-target follow-up,
  ally/anchor step and sustain recast, reveal/slow pulse, chained displacement.
- [x] Veilstalker: out-of-combat recovery and level-six camouflage, prepared
  curse, marked spike recasts, empowered approach and untargetable retreat.
- [x] Emberveil: kill/assist healing, independently tracked outgoing and returning
  orb, guided fires, forced-walk charm and bounded dash recasts/refunds.
- [x] Orbitwright: one persistent orb, travel/path damage, attachment and leash,
  speed/slow field, ally shield/defensive aura and delayed pull.
- [x] Riftshot: hit attack-speed stacks, on-hit shot with cooldown refund,
  owner-only detonation mark, repositioning shot and arena-range piercing wave.
- [x] Chainkeeper: bounded collectible souls, hook/follow-up, shielding lantern
  with explicit allied interaction, charged attack/sweep and consumable cage sides.
- [x] Frostguard: four allied hits into stun and immunity, allied leap/defenses,
  frontal projectile interception and knock-up/slow fissure.
- [x] Versioned catalogue, independent core/passive/skills and orb capability
  validation. Compatible mixed recipes execute through the same runtime.
- [x] Original skill glyphs, English/Russian/Chinese copy, scrolling class selection,
  aim/recast/resource/status feedback and temporary-terrain route integration.
- [x] Final workspace gate, live two-peer UDP checks and focused native captures.

## Code boundaries

- `shared/assets/catalog/{heroes,skills}.json`: authored stats and skill definitions.
- `shared/src/loadout.rs`: strict IDs, recipes, technique metadata, prerequisites
  and snapshot DTOs. A recipe cannot supply a script, avatar, arbitrary numbers or
  an unknown skill. Orb field/pull requires an orb controller in the same recipe.
- `server/src/skills/mod.rs`: common admission, receipts, shield/control plumbing,
  limits and effect replication; `advanced.rs`: staged techniques, persistent
  objects and new passive state. Dispatch follows the equipped skill/passive,
  not a hardcoded class switch. Core selects stats/resource profile only.
- Existing damage, movement, projectile, vision, shop and progression boundaries
  enforce the new state. Forced relocation advances the existing authoritative
  movement sequence. Temporary terrain is included in server and client routes.
- `ClientPacket::Interact` carries object ID, match/epoch and the durable skill
  request sequence. Lantern use requires a living nearby ally, a live owner and
  a live owned lantern. It is explicit: F, left-stick click, or the touch button.
- `client/src/combat/standard.rs`: snapshot presentation and input adapters.
  The shared aim/cast paths continue to serve keyboard, touch and controller.

## OMOBA adaptations and remaining work

The first numbers target the existing ten-level, three-rank rules. These are
playable prototypes for testing, not a finished competitive balance pass.
Targeted skills acquire an eligible visible unit near the aimed point. The
support step creates its own short-lived anchor when aimed away from allies.
Field forging uses the existing six-item shop: after a safe channel, the smith
can buy away from base. Nearby level-six allies with an item receive at most one
masterwork defense bonus. There is no separate crafting inventory.

- [ ] Human playtests: lane matchups, burst/CC chaining, teamfight readability,
  controller/touch comfort and mixed-skill abuse cases.
- [ ] Balance budget and rated-play restrictions for user-authored recipes.
- [ ] Public constructor in game and on the website, saved builds, server-side
  validation/equip API and catalogue revision migration.
- [ ] Refine the initial geometric VFX, custom animation timing and sound cues.
- [x] Chinese localization and refreshed display-font subset; verified by the
  existing dictionary/glyph coverage gates. Native captures remain English only.
- [ ] Physical mobile device verification; this task uses one English desktop
  viewport and does not claim a device/language matrix.

## Verification

Evidence is kept in `.agent/tasks/ST-ROSTER-2026-09-29/`, including the frozen
specification/numbers, failed checks and subsequent fixes. Final verdict: PASS
for AC1–AC9 in the frozen prototype scope.

- `make check`: formatting, both Clippy configurations, 1,330 Rust tests and
  192 Python tests passed. The existing 39 environment-dependent Rust tests remain
  ignored; no database, production service or physical device was exercised.
- 28 new combat outcome/adversarial tests pass within the 371-test server suite.
- Live two-peer UDP: 46 new-roster checks cover nine presets, all 36 normal casts
  and replay rejection; 22 original-kit checks cover prior combat and reconnect.
- Native client/server build passed. Inspected one English 1280×720 selection
  capture and all nine kit captures; repeated only five affected combat states
  after feedback corrections. These are scripted inputs, not manual playtests.
- Source handoff/font synchronization, 63-screen UI inventory, unchanged original
  catalogue entries, original-name audit and `git diff --check` passed.

To play locally from this checkout, run `make practice`, select a class (the class
list scrolls), choose any supported avatar and join. Client and server must both
use protocol 4. Online deployment is a separate action.
