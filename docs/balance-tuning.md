# Balance tuning

Map-object settings live in the versioned `shared/assets/maps/verdant.json`
default and optional `OMOBA_MAP_CONFIG` server startup file. This includes tower
count, lane placement, HP, range, damage and cooldown. See
[map customization](map-customization.md) and `examples/maps/two-tier.json`.
The active profile stays pinned across rematches; restart to load edits.

Other authoritative simulation values remain in `server/src/balance.rs`:
minion roles and wave cadence, jungle/boss stats and rewards, hero baselines,
projectile speed, regeneration and the level curve. Change these constants and
rebuild the server before testing. Tower constants there remain compatibility
baselines for existing fixtures; the resolved map profile supplies live towers.

Class skill kits and rank scaling live in `shared::HeroClass`; the server
resolves all four ability slots and basic attacks authoritatively. Cosmetic
profiles only change presentation. Hero client display defaults in
`client/src/combat.rs` should agree with server baselines until a snapshot arrives.

Both jungle and minion XP use server progression. Evaluate pacing through real
matches and telemetry alongside mechanical regression tests. A successful test
suite is evidence of consistent rules, not proof of competitive balance.
