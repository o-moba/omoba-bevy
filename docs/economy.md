# Economy and respawn balance —0.37.0

## Pacing and options

Starter prices are unchanged: 80g damage, attack speed, movement, and vitality; 100g mana/haste; 120g hybrid crest. New critical and basic-lifesteal components cost 150g. Tier-two boots cost600g; caster/health upgrades650g; damage/crit or damage/lifesteal700g. Major items cost1,400–1,500g. Components are fully credited, including nested recipe leaves; an owned intermediate takes precedence over its leaves, preventing double credit.

| Build option | Total price | Result |
|---|---:|---|
| Windrunner Boots |600g|+16% movement, +22% attack speed |
| Duelist Edge |700g|+24% damage, +20% critical rate |
| Vampiric Fang |700g|+22% damage, +14% basic lifesteal |
| Arcane Focus |650g|+60 HP, +60 mana, +25% spell haste |
| Bulwark |650g|+120 HP, +10% damage |
| Tempest Blade |1,400g|+40% damage, +35% attack speed, +30% critical rate |
| Bloodreaver |1,500g|+90 HP, +38% damage, +22% basic lifesteal |
| Aether Crown |1,400g|+100 HP/mana, +30% damage, +35% spell haste |

Critical hits are authority-side accumulation: accepted non-structure basic strikes accumulate the equipped rate and each whole point yields1.75× damage. Rejected/replayed requests do not advance the meter. Thus25% yields every fourth accepted strike, with reproducible long-run balance rather than client randomness. Basic lifesteal heals from actual primary HP loss, excluding overkill, structures, spell hits, mark bonus damage, and splash. Combined equipment caps are75% critical rate,35% lifesteal, and30% bonus movement. The existing damage/attack-speed/haste rules otherwise remain additive.

A legitimate opposing hero kill pays500g. Repeated defeats without a kill reduce that victim's next bounty to400,300,then250g minimum; getting a kill resets the killer's streak. Eligible assist contributors split one100g pool with deterministic remainder assignment. Rewards are applied only after per-life death receipt deduplication, and never from a repeated network snapshot. This makes a kill approximately2.9 solo-lane farming minutes while limiting simple feeding; it is an initial balance pass, not a claim of competitive balance.

The reproducible model reads the current catalog and constants: `python3 scripts/economy_model.py --output economy-model.json`. `economy-model.json` records every item and all sixteen class purchase plans. At2g/sec, passive income is120g/min. One lane has3×18g per60-second wave, a54g team pool:174g/min solo or147g/min with two equally eligible allies. The timing model credits the first wave at60s (spawn is10s, allowing travel/clearing), excludes jungle and unrelated spending, and includes the initial80g.

| Target item | Solo lane | Shared lane (2) | Passive only | Solo +500g hero kill at2:00 |
|---|---:|---:|---:|---:|
|150g component |0:35|0:35|0:35|0:35|
|600g boots |3:00|3:40|4:20|2:00|
|700g damage upgrade |3:49|4:16|5:10|2:00|
|1,400g major |7:51|9:00|11:00|5:00|
|1,500g major |8:14|9:49|11:50|5:25|

An80g starter is immediately available. Buying it first and then a150g independent component takes up to1:00 with the first lane clear (1:15 passive only). Recipe-path purchases reach their final upgrade for the same total price as a direct purchase; buying a separate item delays it. Missed waves, travel back to base, or deaths delay the model; jungle rewards accelerate it. The data therefore supports early components, a roughly3–4min middle tier, and roughly8min majors for uncontested solo farming without hero kills.

Respawn curve:5s through minute2, then+1.5s per match minute, capped35s at minute22. Checkpoints: minute5→9.5s;10→17s;15→24.5s;20→32s. Early errors remain recoverable; later deaths create larger objective windows. No UI/client time controls this duration.
