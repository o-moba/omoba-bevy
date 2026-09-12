# Combat presentation and minion roles — 2026-09-12

Version: `0.19.0-rc.4`. Task: `COMBAT-FEEL-2026-09-12`.

## Behavior and ownership

The server remains the only authority for strikes, projectiles, HP and rewards.
Class attacks now carry a semantic presentation style and optional ability slot.
Positive HP mutations generate typed damage receipts with actual HP removed,
including lethal overkill clamping. The last second of receipts is retained up
to 96 entries; clients deduplicate by server epoch, match and event ID. Initial
join and round changes establish a baseline. Healing, respawns, protected
structures and invulnerable targets cannot create damage numbers.

The client renders bounded impact bursts and up to 48 short-lived damage labels
inside the viewed battle, including minimap/free-camera focus. Outgoing damage is gold, incoming damage is red, nearby
combat is quieter. Projection uses the existing 3D or genuine 2D camera and the
same desktop/mobile UI profiles. These effects do not capture pointer input.

The [cosmetic registry](../combat-cosmetics.md) is a packaged, versioned extension
point. Profiles select projectile geometry or GLB scenes, optional animated PNG
atlases, colors, scale, trails, impacts and exact avatar animation-clip aliases.
Class/action defaults can be overridden by avatar or sprite identity. Immutable
procedural defaults remain available if custom data or an asset is unavailable.
Configuration does not contain gameplay tuning or authorize remote downloads.

Every three-unit lane wave now contains two shield/blade melee fighters and one
staff caster. The caster trades HP for range and only damages on projectile
arrival. Existing melee parameters, wave size/cadence, paths and rewards remain.
Model height rises from 0.69 to 1.035 world units, below 2.1-unit heroes; collision
radii are unchanged. Replicated attack sequences drive one release/recovery pose
per strike, suppressing duplicate packets and pre-join animation history.

## Initial tuning

| Role | HP | Damage | Base reach | Cooldown |
| --- | ---: | ---: | ---: | ---: |
| Melee minion | 65 | 8 | 2.4 | 0.95 s |
| Caster minion | 45 | 7 | 8 | 1.2 s |

Effective attack reach also includes the existing target collision radius.
Hero damage, skills, attack cooldowns and projectile speed have not changed in
this iteration. Ranked balance needs actual match telemetry and human sessions;
unit tests establish mechanics rather than player enjoyment or competitive
fairness. No mobile-device performance or signed phone package is implied by a
native desktop touch preview.

## Existing draft sprite repair

Native 2D checking exposed a pre-existing unfinished Orchard Comet Centaur pack:
only a static master exists; its referenced locomotion/action PNGs were never
produced. The roster now explicitly declares a render fallback. Its stable ID
and portrait slot remain reserved, new selection is disabled and labeled art
pending, and old saved/network identities render the declared Mossback Teapot
fallback. No substitute file is passed off as Orchard artwork. The nine active
sprite pairs are complete; finishing Orchard art is separate work.

## Verification

All requested checks passed on macOS arm64:

- Full workspace: **453 Rust tests passed**, zero failed or ignored.
- Final affected-source recheck: 105 server and 7 passport tests passed after
  mechanical Clippy cleanup; all three UDP combat tests passed against the
  rebuilt current server, including normal wave release/impact/cooldown timing.
- Strict workspace/all-target Clippy (`-D warnings`), formatting and native
  client/server builds passed.
- Capture verifier: 18 rejection/acceptance checks passed. Six native scenarios
  each captured ready, flight and confirmed impact, matched against an independent
  UDP observer. All six passed without runtime errors or synthetic damage.

| Native scenario | Published frame |
| --- | --- |
| Ranger, desktop 3D | [Arrow flight](2026-09-12-combat/ranger-arrow.png) |
| Mage, 844 × 390 touch preview | [Confirmed magic hit](2026-09-12-combat/mage-mobile.png) |
| Cleric, desktop 3D | [Holy projectile](2026-09-12-combat/cleric-holy.png) |
| Warrior, desktop 3D | [Crescent projectile](2026-09-12-combat/warrior-crescent.png) |
| Ranger, genuine 2D | [Sprite combat](2026-09-12-combat/ranger-2d.png) |
| Ordinary mixed lane wave | [Caster combat](2026-09-12-combat/mixed-wave.png) |

The final client binary is byte-identical to the captured client. The capture
server predates only three equivalent Option early-return cleanups; its exact
hash is retained in [capture provenance](2026-09-12-combat/captures.json).
The rebuilt server was separately verified with the current-source server and
UDP suites. Champion scenes use developer initial placement and ordinary
scripted commands; the wave scenario uses production spawning and routing.
These are real rendered and network-correlated checks, not physical phone tests
or a human multiplayer balance session.

The committed [task evidence](../../.agent/tasks/COMBAT-FEEL-2026-09-12/evidence.md)
maps AC1–AC6 to logs, compressed raw observer snapshots and capture reports.
