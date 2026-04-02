# Balance tuning (gameplay slice)

## Authoritative server values

All simulation tuning for minions, towers, jungle neutrals, the level curve, mana regeneration, and the server-validated projectile ability lives in:

- `server/src/balance.rs`

Change constants there, rebuild the server, and re-run `cargo test -p server` (or full workspace tests) before playtesting.

Notable symbols:

- `PRIMARY_ABILITY_DAMAGE_BY_RANK` — five damage tiers for the primary server-validated ability (index = rank − 1). The live loop uses index `0` until skill rank affects combat.
- `SKILL_SLOT_COUNT` — matches the four UI skill slots / progression skill-point surface; only one cast is fully simulated today.

The `balance` module includes lightweight unit tests (`balance::tests::*`) that assert positive tuning, coherent spell economy, and `LEVEL_XP_THRESHOLDS.len() == MAX_LEVEL - STARTING_LEVEL` so accidental edits do not desync the level curve.

Jungle and minion kill XP both go through the same server level-up path (`grant_player_xp`) so XP thresholds stay coherent across sources.

## Client display baselines

The client uses local defaults for HUD and bars:

- `client/src/combat.rs` — `MAX_HP`, `MAX_MANA`

These **must match** the server `MAX_HP` and `MAX_MANA` in `balance.rs` so first-frame UI stays consistent until the first snapshot arrives.

## Skill power note

The server currently authorizes **one** ranged ability (mana cost, cooldown, projectile damage in `balance.rs`). Skill points accrue from leveling for UI and future skill work; per-rank curves for four distinct abilities are not yet driven from this module.
