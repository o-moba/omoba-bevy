# Equipped skills and admitted control

Version **0.40.0** uses **protocol 9**, recipe schema 1 and catalogue
**standard-kits-4**. This document describes implemented behavior; final
validation is tracked in the [progress note](progress/2026-10-04-equipped-skills.md).
Client and host must use the matching contract. Public selection remains
preset-based; arbitrary recipes are authorable through the opt-in local
laboratory, without a new public editor or account storage feature.

## Recipe and binding contract

[`BuildRecipe` and its resolver](../shared/src/loadout.rs) separate core,
passive and four skill IDs. The array index is the physical Q/W/E/R binding.
Any four **unique** skills may be placed in any order when their capability
requirements are satisfied. Four different ultimates are valid. Duplicate
skills, unknown versions and invalid dependencies are rejected; the equipped
metadata boundary also rejects a recipe whose core differs from the actor class.

Weapon switching requires a Repeater attack profile. Orbital Field and Orbital
Collapse require Orbital Command or Orbital Guard in the same recipe. The core
still owns base growth, resource/attack profile and associated fixed behavior.
This change does not turn the twenty legacy class abilities into modular skills.

[`EquippedSkills`](../shared/src/loadout/equipped.rs) is the shared read-only
metadata interface. An absent recipe selects the modular class preset or the
legacy class kit. A supplied malformed recipe produces an error, never a
different preset. Runtime callers can construct the same view from a previously
validated `ResolvedLoadout`.

| Property | Binding rule |
| --- | --- |
| Identity, targeting, range, names and icons | The skill equipped at that physical index |
| Unlock level | The skill's authored default role: Q/W/E/R roles unlock at levels 1/2/4/6 |
| Rank and cooldown state | The physical binding, using the equipped definition's limits and duration |
| Bot upgrade priority | Authored ultimate role first, then authored Q/W/E roles; moving a skill preserves its priority |
| Cast animation category | Authored skill role; the replicated action slot remains the physical binding |
| Recast eligibility | The active state must belong to the skill still equipped at that binding |

Thus Dawn Ray on Q remains locked until level 6, while Dawn Bind on R unlocks
at level 1. Four ultimates stay locked until level 6. Manual upgrades still
require points and respect the actual rank cap. Laboratory `unlock_all` is an
explicit override for casts/upgrades; the client honors the authoritative
sandbox unlock mask. Default preset and legacy progression rules are retained.

Modular cooldowns use the equipped skill, rank, hero growth and spell haste,
independently of button. Legacy basic-Q attack-speed behavior stays explicit.
Initial costs are rank-scaled; follow-up costs come from the skill catalogue:
**Echo Strike, Anchor Step and Thunder Pulse cost 25**, including on another
core. Other current follow-ups cost zero, including when borrowed by Stormfist.
Insufficient resources reject a follow-up without consuming its remaining use
or restarting the initial cooldown. The existing infinite-resource laboratory
modifier remains authoritative.

## Runtime and client ownership

The [common engine](../common/src/skills/mod.rs) serves online and offline play.
Persistent effects, orb hits, Brittle/Concussion marks and their damage receipts
retain the actual source binding. Recast advertisement, delayed grants and
consumption verify skill identity, so replacing a skill cannot inherit a stale
free follow-up. Passive and core identities remain explicit where they describe
that passive/core rather than a button.

The [client adapter](../client/src/equipped_skills.rs) supplies desktop, touch
and controller cast admission, aim previews, localized names/icons, skill
cards, upgrades and predicted total cooldowns. Server remaining cooldown and
recast snapshots retain authority. Recast mana displays and prechecks use the
same shared cost function as execution. Presentation resolves the accepted
skill using the replicated actor class; mismatched or malformed recipes cannot
select a skill or basic-attack motion through a fallback.

[`ActorConfig.recipe`](../shared/src/sandbox.rs) is optional in the existing
Combat Test configuration. The [server laboratory](../server/src/sandbox.rs)
validates the full configuration before applying it. A same-class recipe edit
preserves actor/session identity and clears previous skill effects, recasts,
recovery and skill cooldowns; it retains ordinary request sequencing and
unrelated basic-attack timing. Rejected configuration leaves the actor/config
unchanged. Explicit recipes require current-protocol negotiation before any
mutation; preset-only legacy laboratory commands retain their existing path.
Existing local laboratory admission remains in force; public
matches and allocated workers do not gain an authoring path.

## Shared admitted control

[`crowd_control`](../common/src/skills/crowd_control.rs) is used by generic
roots/slows, advanced stuns, charm and Bluff. It rejects structures, invalid
targets, dead/unjoined players and god-mode players; parry and untargetability
block applicable player control. An admitted immobilization interrupts recall
immediately. Slow-only application retains its existing behavior and does not
consume Brittle.

The intentional behavior fix is **Bluff × Brittle**: an admitted Bluff stun
consumes eligible Brittle marks once through the existing damage/event path,
retaining each mark owner's actual source binding and normal shield, damage
and reward processing. Repeated Bluff cannot trigger the same mark again.
Rejected control itself neither consumes Brittle nor cancels recall. Bluff's
pre-cast target rejection preserves mana/cooldown rules; this does not refund
an already accepted projectile whose later control application is blocked.

Preserved exceptions are deliberate:

- Unstoppable blocks Bluff and forced displacement. It does not grant blanket
  immunity to ordinary roots, advanced stuns or charm.
- Player charm keeps its forced-movement target and historically does not
  consume Brittle. NPC charm uses the stun/control path and can consume it.
- Bluff extends the existing stun deadline and turns a surviving admitted
  victim away; it does not add the generic root timer. Root and advanced-stun
  duration policies remain distinct.
- Forced displacement keeps its separate voluntary/forced movement admission
  and collision rules. Damage, self buffs and utility admission are not routed
  through a universal control hook.
- A parried immobilization reaching the shared helper records the control
  attempt. Bluff can be rejected earlier by its cast target checks.

## Verification and remaining work

Shared tests cover default equivalence, permutations, four ultimates,
dependency rejection and follow-up metadata. Common regressions cover
cross-kit Bluff/Brittle, rejected targets/resources, moved source identity,
recasts, unlocks and sandbox upgrades. Client tests cover cast prechecks,
cooldown authority, mobile cards, recast costs and invalid presentation data.
Server tests cover atomic laboratory admission and state replacement.

The new [framed UDP harness](../harness/tests/equipped_skills.rs) and the
English 852×393 affected-HUD fixture provide separate verified evidence:
the first exercises a real server; the second is synthetic accepted-loadout
presentation and does not prove server behavior or physical-phone performance.
Both evidence runs and the final full gate pass: 1,547 Rust tests (39 ignored),
201 Python tests, and 50 harness tests (24 unit / 26 integration). The held
card is checked through actual rendered text and computed bounds; one valid
HUD image was reused when only the card assertion changed. The final QA-only
Clippy predicate rewrite is equivalent to the captured build; production source
and appearance are unchanged. See the progress note for provenance and limits.

Public editing, persistent recipes, website equip, duplicate-skill instances,
new executable skill definitions, a general multi-phase animation protocol,
physical-device acceptance and competitive balance remain future work. Larger
actor/controller, capacity and module extractions remain in
[R18–R23](REFACTORING.md#post-programme-architecture-follow-ups--2026-10-04).
