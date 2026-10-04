# Adventurer — dagger class

An original OMOBA adaptation of the close-range positional dagger play described by the owner. The reference names and backward-turning idea appear in [Lineage II's official Battle Chronicle notes](https://www.lineage2.com/en-us/news/battle-chronicle-patch-notes); the later Lethal Blow progression appears in the [official Classic Saviors notes](https://www.lineage2.com/en-us/news/lineage-ii-classic-saviors-patch-notes). These references inform the fantasy, not the damage numbers or assets. This kit does not claim exact behavior from any particular Lineage chronicle.

## Kit and initial balance

| Slot | Skill | Rank-one damage | Cooldown | Mana | Behavior |
|---|---|---:|---:|---:|---|
| Q | Deadly Blow | 22 | 5s | 12 | Reliable single-target dagger strike in melee. |
| W | Bluff | 0 | 14s | 20 | Stuns a hostile hero for 1s and turns their back toward the caster. |
| E | Backstab | 16 / 40 from behind | 9s | 18 | Rear strike deals 2.5× damage; eligible survivors have a 2% vital-break chance. |
| R | Lethal Blow | 38 / 57 from behind | 22s | 28 | Stronger mastery strike; rear bonus 1.5×. Unlocks through the existing R progression. |

Strikes reach 2.6 world units, Bluff 2.8, with the existing target collision-radius allowance. Existing rank, level, item and team-buff scaling applies. Hero baseline: 190 HP, 18 basic damage, 0.85s basic interval, 2.6 basic range; primary role Jungle. Dagger Mastery adds 10% to basic attacks from behind enemy heroes. The recommended purchase path prioritizes physical damage, movement and lifesteal within the existing 16-item economy.

The rear region is a 120-degree cone behind the victim. The server compares victim facing with the vector toward the attacker; coincident/nonfinite positions never qualify. Accepted attacks update authoritative facing as well as animation facing. Bluff holds the victim under the ordinary stun gates, so movement, basic attacks, skills and utilities cannot silently turn the victim out of the setup.

## Vital break

Backstab rolls only after an accepted rear hit actually reduces an enemy hero's HP and that hero still has more than 1 HP. Two percent of those hits can remove the remaining HP down to 1. An ordinary lethal hit still kills normally; the special outcome never creates a false kill or bounty. Front hits, minions, bosses, structures, immune targets, fully absorbed hits and rejected/replayed inputs cannot trigger it. Normal shields and immunity remain part of the damage path.

The authority owns private per-world keyed randomness and a private eligible-draw counter. Client request IDs, timestamps, aim vectors and public match IDs do not seed the outcome. Deterministic forced outcomes exist only in Rust unit-test builds. The same execution runs in offline practice. The visible/audio cue follows the confirmed `near_lethal` combat receipt, including normal visibility filtering and event deduplication.

This is a deliberately rare swing, not a dependable execution ability. The initial balance should be tuned using match results and feedback; no competitive-balance guarantee is implied.

## Integration

Hero/core id: `adventurer`. Skill ids: `dagger_deadly_blow`, `dagger_bluff`, `dagger_backstab`, `dagger_lethal_blow`. The preset assigns Q/W/E/R; the reusable skill ids can be placed in other validated hybrid recipes. Default equipment uses the built-in `dagger` through the normal hand-bone attachment catalogue, while normal cosmetic selection can override it independently of combat statistics.

The reusable recipe/authority and animation paths are covered here. A player-facing custom-kit editor is not included. Existing generic mobile input/HUD paths still derive some ability metadata from the class preset, so arbitrary custom recipes need a separate end-to-end UI integration pass; the shipped Adventurer preset uses matching definitions throughout.

Version 0.38.0, catalogue `standard-kits-3`, protocol 7. The strict core/skill/passive enums require matching client and server. No new production dependency, server rollout or mobile upload is part of this implementation task. See [verification and delivery state](progress/2026-10-04-adventurer.md).
