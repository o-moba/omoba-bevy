# Adventurer — dagger class

An original OMOBA adaptation of the close-range positional dagger play described by the owner. The reference names and backward-turning idea appear in [Lineage II's official Battle Chronicle notes](https://www.lineage2.com/en-us/news/battle-chronicle-patch-notes); the later Lethal Blow progression appears in the [official Classic Saviors notes](https://www.lineage2.com/en-us/news/lineage-ii-classic-saviors-patch-notes). These references inform the fantasy, not the damage numbers or assets. This kit does not claim exact behavior from any particular Lineage chronicle.

## Kit and initial balance

| Slot | Skill | Rank-one damage | Cooldown | Mana | Behavior |
|---|---|---:|---:|---:|---|
| Q | Deadly Blow | 34 | 5s | 12 | Reliable single-target dagger strike in melee. |
| W | Bluff | 0 | 14s | 20 | Stuns a hostile hero for 1s while preserving their facing. |
| E | Backstab | 24 / 60 from behind an enemy hero | 9s | 18 | Rear strike on an enemy hero deals 2.5× damage. |
| R | Lethal Blow | 48 / 72 from behind an enemy hero | 22s | 28 | Stronger mastery strike; rear bonus 1.5× against enemy heroes. Unlocks through the existing R progression. |

Numbers are those of catalogue `standard-kits-6` (`shared/assets/catalog/skills.json`, `heroes.json`); the dagger damage was tuned in 0.42.0. The rear multiplier of Backstab and Lethal Blow, like Dagger Mastery, applies to enemy heroes only: a minion, a monster or a structure has no rear, and the skill descriptions say so since the 0.44.0 candidate.

Strikes reach 2.6 world units, Bluff 2.8, with the existing target collision-radius allowance. Existing rank, level, item and team-buff scaling applies. Hero baseline: 190 HP, 24 basic damage, 0.7s basic interval, 2.6 basic range; primary role Jungle. Dagger Mastery adds 10% to basic attacks from behind enemy heroes. The recommended purchase path prioritizes physical damage, movement and lifesteal within the existing 16-item economy.

The rear region is a 120-degree cone behind the victim. The server compares victim facing with the vector toward the attacker. Coincident or nonfinite positions never qualify. Accepted attacks update the attacker's authoritative and animation facing. Bluff stuns the victim in their current orientation, giving Adventurer a brief opportunity to flank. A follow-up Backstab from the front deals its base damage.

## Combat receipts

Backstab resolves one ordinary physical hit in both online and offline play. Normal shields, mitigation, immunity and lethal outcomes use the shared damage path. The former 2% effect that left a survivor at 1 HP was removed from live combat. Clients retain the optional `near_lethal` receipt and its presentation for compatibility with older snapshots and QA fixtures.

## Integration

Hero/core id: `adventurer`. Skill ids: `dagger_deadly_blow`, `dagger_bluff`, `dagger_backstab`, `dagger_lethal_blow`. The preset assigns Q/W/E/R; the reusable skill ids can be placed in other validated hybrid recipes. Default equipment uses the built-in `dagger` through the normal hand-bone attachment catalogue, while normal cosmetic selection can override it independently of combat statistics.

Presentation: each of the four skills has its own row in `client/assets/config/skills.skillfx` (motions `dagger_stab`, `dagger_feint`, `dagger_backstab`, `dagger_heavy_thrust`; the Backstab clip was re-keyed so its stab pose comes 0.11 s into the clip), and Bluff's sound follows distance like every other cast voice. See [combat cosmetics](combat-cosmetics.md#the-68-rows).

The reusable recipe/authority and animation paths are covered here. A player-facing custom-kit editor is not included. Existing generic mobile input/HUD paths still derive some ability metadata from the class preset, so arbitrary custom recipes need a separate end-to-end UI integration pass; the shipped Adventurer preset uses matching definitions throughout.

Introduced in version 0.38.0 (catalogue `standard-kits-3`, protocol 7); the table above follows the current catalogue. The strict core/skill/passive enums require matching client and server. No new production dependency, server rollout or mobile upload is part of this implementation task. See [verification and delivery state](progress/2026-10-04-adventurer.md).
