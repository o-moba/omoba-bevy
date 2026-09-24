# Combat balance: measured first pass (0.23.0-rc.2)

This pass gives an unprotected level-one hero time to react, then makes ordinary
level progression increase mobility and damage throughput. It is a controlled
mechanical baseline, **not a claim of competitive balance or human win rates**.

## Diagnosis and design sources

At `75fff6e` all four heroes started at 100 HP. Q could fire every
0.25–0.6 seconds alongside an independent basic attack. Different Q/W/E/R slots
could execute in the same tick. Levelling added HP and mana but no innate
movement speed, attack rate or per-hit growth. In the stationary target probe,
level-one median time to kill (TTK) was 1.67 seconds, even shorter than the
reported 3–4 seconds in real movement. Some classes actually took longer to kill
an equal-level opponent at level ten.

The design principles come from primary developer explanations:

- [Riot: Champion Counterplay (May 2021)](https://www.leagueoflegends.com/en-us/news/dev/quick-gameplay-thoughts-may-14/)
  describes response opportunities, tactical choices and avoiding fights decided
  merely by who fires first. Here that motivates early durability and a shared
  interval between skills, while retaining independent basic attacks.
- [Riot: Durability Update thoughts (May 2022)](https://www.leagueoflegends.com/en-us/news/dev/quick-gameplay-thoughts-5-6/)
  explains the agency/readability cost of excessive damage and the need to check
  sustain, mana and surrounding systems when changing durability. Here that
  motivates the mana/sustain samples and the tower follow-up.
- [Wild Rift: Balance Framework Update (March 2021)](https://wildrift.leagueoflegends.com/en-us/news/dev/dev-balance-framework-update/)
  uses player populations, roles and multiple measures rather than one average.
  Here that motivates publishing each ordered class matchup and explicitly
  reserving human skill, role, pick-rate and win-rate assessment for playtests.

These sources supply principles, **not OMOBA's numbers**. Before implementation,
the task specification chose a level-one minimum TTK of 6 seconds and a median
of 8–14 seconds in the controlled scenario; level-ten mean must be at least 20%
lower, every attacker's mean must improve, and no level-ten row may be below
2 seconds. These bounds are local design targets, not promises under focus fire,
gear advantages, unequal levels or different spacing.

## Shared growth and combat rules

`shared/src/hero_balance.rs` is the single source of the innate growth curve
and the values every class shares. The per-class numbers in the table below
(starting HP, the Q interval, the level-ten caps) and the basic attacks and
kits are data in `shared/assets/catalog/heroes.json` (`base_hp`,
`growth.basic_damage_cap`, `growth.attack_rate_cap`, `basic_attack`,
`abilities`); item costs and bonuses are in `shared/assets/catalog/items.json`.
Retune by editing those files; see "Adding content" in
[ARCHITECTURE.md](ARCHITECTURE.md) for the schema. For each multiplier,
`g(level, cap) = 1 + (cap - 1) × (clamp(level, 1, 10) - 1) / 9`.
Items multiply this once; team damage buffs continue to multiply the final
outgoing damage. Ranges and class abilities keep their existing identities.

| Class | Starting HP | Q interval at L1 | L10 basic damage multiplier | L10 basic frequency multiplier |
|---|---:|---:|---:|---:|
| Warrior | 220 | 2.0s | 1.80 | 1.60 |
| Mage | 180 | 2.2s | 1.60 | 1.50 |
| Ranger | 185 | 1.8s | 1.85 | 1.80 |
| Cleric | 200 | 2.0s | 1.50 | 1.55 |

- All classes move at 5.0 m/s at level one, growing to 6.2 m/s at level ten
  (+24%), before boots and utility haste. Human prediction, authoritative
  movement checks, bot movement and Combat Test telemetry use the same curve.
- Each level still grants 18 maximum HP and 12 maximum mana. Base mana remains
  100; ordinary regeneration remains 8 mana/s. Skill unlocks remain 1/2/4/6.
- Basic damage and basic frequency use their class curves. Q frequency uses
  that same attack-rate curve; W/E/R frequency grows to 1.20× at level ten.
- Damage, self-healing and self-mana-restoration effects grow to 1.35× at level
  ten, composed with the existing 10% per extra skill rank. Rank cooldown
  reduction remains 6% per extra rank. Resource restoration clamps at normal
  maxima and cannot resurrect dead players.
- Q/W/E/R share a recovery interval: 0.45s at level one, declining to 0.30s at
  level ten. A second slot cannot bypass this interval; rejected casts spend
  neither mana nor their slot's cooldown. Basic attacks retain a separate
  clock and may accompany a skill, so holding Attack remains useful.
- The client buffers the latest pending skill during this short recovery.
  Server snapshots carry all four skill deadlines and shared recovery, so
  reconnect and respawn restore the correct indicators. A bounded 0.3s local
  prediction grace avoids accepting an older pre-cast snapshot as a rejection.
- Combat Test's explicit no-cooldowns option bypasses recovery. Its custom HP,
  damage and speed controls still work; ordinary matches receive no bypass.
  Its existing test defaults/presets keep an explicit 100 base HP override. To
  reproduce normal durability there, set base HP to 220/180/185/200 for
  Warrior/Mage/Ranger/Cleric, or use a normal bot match. Changing the selected
  class in Combat Test does not silently overwrite a custom HP setting.

Warrior retains strong close-range pressure, Mage lower durability and longer
range, Ranger the strongest innate basic attack rate, and Cleric lower offensive
throughput with substantial healing. The close stationary probe favors Warrior:
it does not price in the difficulty of closing distance or support value.

## Scenario and measured results

The harness calls production Join, XP/upgrade, cast, basic attack, mana regen,
projectile damage and death paths. No alternate damage calculator is used.
It tests all 16 ordered class pairings at levels 1, 5 and 10 (48 rows). Actors
start three metres apart, full normal HP/mana, no items, buffs, fountain,
structures or sandbox overrides. The victim stands still without attacking or
healing. The attack policy requests a basic, then damaging R/E/Q every 1/120s,
using normal mana restoration when useful. Actual cooldown/mana/target checks
accept or reject each request. TTK includes projectile travel; timeout is 60s.

Levels are reached with normal XP; legal skill points are spent in
Q,Q,W,W,E,E,R,R order, skipping locked slots. Level one therefore has only Q;
level five gains E and invested ranks, and level ten has R and further ranks.
The pronounced level-five drop reflects these unlocks, not a linear TTK curve.
The 120 Hz probe provides repeatable measurement resolution; real server tick
quantization, latency and player behavior can change encounter times.

| Level | Baseline mean | Current mean | Current median | Current min–max |
|---|---:|---:|---:|---:|
| 1 | 1.81s | 10.04s | 10.53s | 6.12–13.35s |
| 5 | 2.22s | 5.73s | 5.43s | 3.28–10.03s |
| 10 | 2.95s | 4.24s | 3.05s | 2.38–9.22s |

All six damaging-slot priorities were also checked across the 16 pairings at
levels one and ten (192 additional cases). This exposed a 1.95s Warrior E/Q/R
rotation; increasing only Heroic Strike (E) from 2.5s to 2.8s base cooldown
raises the fastest measured late opening to 2.18s. The primary minimum is 2.38s.
The early matrix is unchanged.

The level-ten mean is 57.8% below the current level-one mean. The baseline
level-one median increases by 8.85s. Full matrices, basic-only/Q-only samples,
an alternative Q/E/R opening and bidirectional same-class samples with a 0.5s
reply delay are stored in the task evidence. The reply-delay sample includes
self-healing and is **not a win-rate estimate**.

### Ordered TTK matrices

Rows are attackers, columns stationary defenders. All entries are seconds.

#### Level 1

| Attacker | Warrior | Mage | Ranger | Cleric |
|---|---:|---:|---:|---:|
| warrior | 8.12s | 6.12s | 6.43s | 7.33s |
| mage | 13.35s | 11.14s | 11.14s | 12.27s |
| ranger | 10.93s | 8.66s | 9.12s | 9.51s |
| cleric | 13.13s | 10.12s | 11.12s | 12.13s |

#### Level 5

| Attacker | Warrior | Mage | Ranger | Cleric |
|---|---:|---:|---:|---:|
| warrior | 4.42s | 3.28s | 3.28s | 3.70s |
| mage | 6.43s | 5.67s | 5.67s | 5.67s |
| ranger | 5.18s | 3.92s | 3.92s | 4.28s |
| cleric | 10.03s | 8.62s | 8.62s | 9.01s |

#### Level 10

| Attacker | Warrior | Mage | Ranger | Cleric |
|---|---:|---:|---:|---:|
| warrior | 2.48s | 2.38s | 2.48s | 2.48s |
| mage | 3.78s | 3.05s | 3.05s | 3.30s |
| ranger | 3.37s | 2.49s | 2.49s | 2.97s |
| cleric | 9.22s | 8.11s | 8.11s | 8.11s |

### Mana and healing

Each matrix row records normal starting mana, regeneration, net mana spent
per request step, underfunded request attempts, first-second damage and accepted
actions. `mana_net_spent` is deliberately net of same-step restoration, not gross
spell cost. `mana_short_attempts` counts requested casts with insufficient mana,
even if another gate such as cooldown also prevents that request; it is not a
count of lost casts. No primary row relies on infinite mana, times out, or records an underfunded
request. For same-class targets, net mana spent / regenerated is:

| Class | Level 1 | Level 10 |
|---|---:|---:|
| Warrior | 50.0 / 40.9 | 124.8 / 18.0 |
| Mage | 132.0 / 89.1 | 180.0 / 24.4 |
| Ranger | 84.0 / 70.9 | 151.2 / 19.9 |
| Cleric | 84.0 / 72.9 | 115.2 / 58.2 |

The 30-second sustain sample starts at half HP at level ten, applies 10 external
HP/s (nonlethal floor of 1), and requests self skills under normal cooldowns/mana.
It measures actual healing rather than attempted overheal. Warrior restores
226.80 HP, Ranger 204.12 HP, Cleric 474.53 HP; Mage has no self-heal and restores
0 HP. The nonlethal floor makes this a sustain-capacity sample, not a survival
or duel test. Fountain healing remains 12% maximum HP/s inside the own-base
shop zone, only while alive; death and level-up regressions remain covered.

### Towers, waves and objectives

Higher hero HP needs corresponding tower safety. In a production-path isolated
level-one siege at three metres, a 220-HP Warrior defeated the old outer tower
in 8.23s with 94 HP left. Doubling damage to **heroes only** makes that tower
kill the same solo Warrior in 6.54s with 48 tower HP left. Every class fails this
unsupported level-one solo siege. The historical-HP100 comparison in the
objective report holds the **new kit constant** to isolate HP; it is not a
second full historical-commit replay.

The built-in profile uses a hero-target damage multiplier of 2: effective lane
and base shots are 28 and 36 against heroes, while minion damage stays 14 and 18.
Custom profiles without the optional multiplier retain 1. Blanket doubling
would reduce a stationary standard wave from 14 shots to 8 (11.7s to 6.3s between
first and final shot); that change was rejected to preserve lane siege windows.
Tower HP, ranges, targeting priorities, minion/jungle stats, XP and gold rewards
remain unchanged. Stronger late heroes naturally clear objectives faster.

## Reproduction and interpretation

From the repository root (Cargo dependencies already installed):

```sh
TASK=.agent/tasks/TASK-COMBAT-BALANCE-2026-09-23
OMOBA_BALANCE_REPORT="$PWD/$TASK/raw/candidate.json" cargo test --locked --offline -p server --bin server balance_probe::measured_balance_matrix -- --nocapture
OMOBA_BALANCE_REPORT="$PWD/$TASK/raw/repeat.json" cargo test --locked --offline -p server --bin server balance_probe::measured_balance_matrix -- --nocapture
python3 scripts/check_combat_balance.py "$TASK/raw/baseline.json" "$TASK/raw/candidate.json" "$TASK/raw/repeat.json"
```

The harness binds an ephemeral loopback UDP socket, so restricted sandboxes need
local socket permission. The reference `baseline.json` was captured before any
production edits at `75fff6e`. `raw/baseline-provenance.json` records the test-only
instrumentation hash and command; `raw/baseline-instrumentation.rs` preserves the
exact harness. To regenerate the baseline, check out `75fff6e` in a separate
worktree, copy that file (or the equivalent checked-in `server/src/balance_probe.rs`
on a fresh clone) to `server/src/balance_probe.rs`, add
`#[cfg(test)] mod balance_probe;` to server main, and run the same command there.
Do not run candidate-target assertions against the intentionally failing old
balance. Task artifacts are local proof records under `.agent/tasks/` (normally
ignored by Git); the production test and validation script are checked in.

Run shared/server suites and the client tests for integration, not just this
matrix. For hands-on checks, rebuild **both** server and client, including iPad,
and use Combat Test or a normal bot match. Normal levelling, resource use,
retreats and supported sieges still need human playtests. Before another tuning
pass, collect role/class, level, inventory, engagement distance, reaction time,
outnumbering, damage sources, deaths and match length. Separate skill brackets
and bot games; do not infer human win rate from the deterministic matrix.

Map-object settings live in `shared/assets/maps/verdant.json` or a server's
`OMOBA_MAP_CONFIG`, pinned across rematches. See [map customization](map-customization.md).
Other authoritative world constants remain in `server/src/balance.rs`; class
kits, base HP, growth caps and basic attacks in
`shared/assets/catalog/heroes.json`; item costs and bonuses in
`shared/assets/catalog/items.json`; the shared progression curve in
`shared/src/hero_balance.rs`.

## Five classes and the Warden (0.23.0-rc.6)

The Warden fills the Jungle duty. Its duel strength sits deliberately just below
the Warrior: its value is farming the forest, not winning an equal lane fight.
Same stationary probe (`OMOBA_BALANCE_REPORT=<file> cargo test -p server
measured_balance_matrix`), mean TTK in seconds as attacker against all five
classes:

| Attacker | Level 1 | Level 10 |
|----------|--------:|---------:|
| Warrior  | 7.23 | 2.46 |
| Mage     | 12.25 | 3.30 |
| Ranger   | 9.72 | 2.86 |
| Cleric   | 11.73 | 8.42 |
| Warden   | 8.08 | 3.19 |

All 75 rows per level keep the design bounds: level-one minimum 6.12s and median
9.51s, level-ten minimum 2.38s.

Jungle economy per clear of one team's three camps (40s respawn): any class
earns 115 gold and 190 XP; the Warden earns 161 gold and 238 XP and kills each
camp about 26% faster. One lane wave pays 54 gold and 270 XP per minute, so the
forest is now the Warden's best income without making it a better lane for
other classes. Boss rewards are flat for everyone; the Warden only hits bosses
15% harder. These numbers are beta tuning, not measured human win rates.
