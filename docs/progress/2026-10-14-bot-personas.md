# Bot personas: S3 difficulty simulator and calibration — 2026-10-14

The S3 package of [docs/plans/bot-personas.md](../plans/bot-personas.md): a headless simulator of full bot-only 5v5 rounds, and the bot-level calibration it supports. The raw report is [2026-10-14-bot-difficulty-report.json](2026-10-14-bot-difficulty-report.json).

## Setup

- Code: `feat/bot-levels`, the tree of the S3 calibration commit (simulator `common/tests/bot_difficulty_sim.rs`, Veteran without the outnumbered hold and threat response).
- Host: the dev VPS builder (2 vCPU, ubuntu 26.04, glibc 2.43), release build, two threads at nice 19 outside the build lock. Wall time 23,506 s (6.5 h).
- Rounds: the standard practice fill on both sides, sides alternating, 50 ms steps with a seeded ±20% jitter (base seed 0). A round ends at Victory or at 20 game minutes; a capped round goes to the side that removed more enemy structure health, and a lead under 0.05 structures is a draw.
- Pairings: Elite against Elite and each lower level against Elite, 200 rounds each; Regular against Veteran and Recruit against Regular, 100 rounds each.

Command (on the VPS; the second line runs the built binary directly):

```sh
cargo test --release --locked -p common --features test-support --test bot_difficulty_sim --no-run
OMOBA_BOT_SIM_PAIRINGS='elite:elite@200,veteran:elite@200,regular:elite@200,recruit:elite@200,regular:veteran@100,recruit:regular@100' \
  OMOBA_BOT_SIM_THREADS=2 OMOBA_BOT_SIM_REPORT=report.json \
  nice -n 19 target/release/deps/bot_difficulty_sim-<hash> --ignored full_matrix > /dev/null
```

The result is a deterministic function of the seeds on one `libm`. Another C library plays different, equally valid rounds.

## Outcomes

"Subject win rate" counts decisive rounds only. The structure lead is the subject's mean lead in structures removed, with a 95% normal interval. "Green share" is the share of decisive rounds Green won.

| Pairing (subject : opponent) | Rounds | Subject W–L–D | Subject win rate (decisive) | 95% Wilson | Victories | Structure lead (subject) | Green share |
| --- | --- | --- | --- | --- | --- | --- | --- |
| elite : elite | 200 | 99–89–12 | 0.527 | [0.455, 0.597] | 0 | +0.05 ± 0.29 | 0.59 |
| veteran : elite | 200 | 33–164–3 | 0.168 | [0.122, 0.226] | 5 | -2.61 ± 0.36 | 0.65 |
| regular : elite | 200 | 2–196–2 | 0.010 | [0.003, 0.036] | 13 | -4.58 ± 0.26 | 0.51 |
| recruit : elite | 200 | 1–199–0 | 0.005 | [0.001, 0.028] | 35 | -6.00 ± 0.23 | 0.51 |
| regular : veteran | 100 | 24–76–0 | 0.240 | [0.167, 0.332] | 0 | -1.70 ± 0.47 | 0.60 |
| recruit : regular | 100 | 17–81–2 | 0.173 | [0.111, 0.260] | 0 | -1.86 ± 0.42 | 0.54 |

## Behaviour per pairing side

Rates are per bot-minute on the field. Health retreats are the health-triggered starts only; re-triggers during a retreat are not counted. A tower dive is a hero's first tower shot after at least 3 s without one.

| Pairing, side | Kills / bot-min | Deaths / bot-min | Health retreats: n, median HP [95% CI], max | Gate retreats / bot-min (outnumbered, threat) | Threat retaliations / bot-min | Tower shots / bot-min | Tower dives / bot-min | Outnumbered share of hero fights |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| elite:elite, elite | 0.818 | 0.834 | 18626, 0.229 [0.228, 0.230], 0.280 | 0.000, 0.000 | 0.000 | 0.024 | 0.012 | 0.065 |
| elite:elite, elite | 0.825 | 0.828 | 18579, 0.228 [0.228, 0.229], 0.280 | 0.000, 0.000 | 0.000 | 0.015 | 0.009 | 0.066 |
| veteran:elite, veteran | 0.788 | 0.730 | 18893, 0.285 [0.284, 0.286], 0.330 | 0.000, 0.000 | 0.000 | 0.031 | 0.019 | 0.082 |
| veteran:elite, elite | 0.725 | 0.799 | 17655, 0.230 [0.228, 0.231], 0.280 | 0.000, 0.000 | 0.000 | 0.039 | 0.021 | 0.056 |
| regular:elite, regular | 0.563 | 0.554 | 7818, 0.376 [0.374, 0.376], 0.400 | 0.001, 0.887 | 0.013 | 0.005 | 0.001 | 0.009 |
| regular:elite, elite | 0.546 | 0.589 | 15265, 0.240 [0.239, 0.241], 0.280 | 0.000, 0.000 | 0.000 | 0.068 | 0.031 | 0.044 |
| recruit:elite, recruit | 0.482 | 0.419 | 7122, 0.483 [0.482, 0.483], 0.500 | 0.439, 0.683 | 0.033 | 0.000 | 0.000 | 0.010 |
| recruit:elite, elite | 0.406 | 0.515 | 13433, 0.248 [0.247, 0.248], 0.280 | 0.000, 0.000 | 0.000 | 0.209 | 0.059 | 0.057 |
| regular:veteran, regular | 0.407 | 0.484 | 3983, 0.379 [0.377, 0.380], 0.400 | 0.001, 0.808 | 0.011 | 0.043 | 0.003 | 0.006 |
| regular:veteran, veteran | 0.476 | 0.423 | 7780, 0.297 [0.296, 0.299], 0.330 | 0.000, 0.000 | 0.000 | 0.111 | 0.044 | 0.058 |
| recruit:regular, recruit | 0.130 | 0.207 | 4799, 0.486 [0.486, 0.487], 0.500 | 0.225, 0.346 | 0.001 | 0.000 | 0.000 | 0.005 |
| recruit:regular, regular | 0.183 | 0.146 | 4331, 0.387 [0.386, 0.387], 0.400 | 0.001, 0.489 | 0.000 | 0.039 | 0.005 | 0.004 |

The highest health at a retreat start prints as the level's line after rounding. Every start was strictly below it, as the `health_retreats_*` checks confirm.

## Checks

- PASS `elite_vs_elite_contains_half`: Wilson95 [0.455, 0.597]
- FAIL `vs_elite_recruit_below_regular`: 0.005 < 0.010, one-sided p = 0.2782; both lose nearly every round to Elite (Wilson upper < 0.10), so this is not gating
- PASS `vs_elite_regular_below_veteran`: 0.010 < 0.168, one-sided p = 0.0000
- PASS `vs_elite_veteran_below_elite`: 0.168 < 0.527, one-sided p = 0.0000
- PASS `vs_elite_lead_recruit_below_regular`: -6.00 ± 0.23 < -4.58 ± 0.26 structures
- PASS `vs_elite_lead_regular_below_veteran`: -4.58 ± 0.26 < -2.61 ± 0.36 structures
- PASS `vs_elite_lead_veteran_below_elite`: -2.61 ± 0.36 < 0.05 ± 0.29 structures
- PASS `recruit_loses_to_regular`: subject Wilson95 [0.111, 0.260]
- PASS `regular_loses_to_veteran`: subject Wilson95 [0.167, 0.332]
- PASS `veteran_loses_to_elite`: subject Wilson95 [0.122, 0.226]
- PASS `draws_at_most_10pct_elite:elite`: 12 of 200 rounds
- PASS `draws_at_most_10pct_veteran:elite`: 3 of 200 rounds
- PASS `draws_at_most_10pct_regular:elite`: 2 of 200 rounds
- PASS `draws_at_most_10pct_recruit:elite`: 0 of 200 rounds
- PASS `draws_at_most_10pct_regular:veteran`: 0 of 100 rounds
- PASS `draws_at_most_10pct_recruit:regular`: 2 of 100 rounds
- PASS `health_retreats_recruit`: 11921 starts, max 0.5000 < 0.5, median 0.485 >= 0.40
- PASS `health_retreats_regular`: 16132 starts, max 0.4000 < 0.4, median 0.381 >= 0.30
- PASS `health_retreats_veteran`: 26673 starts, max 0.3300 < 0.33, median 0.289 >= 0.23
- PASS `health_retreats_elite`: 83558 starts, max 0.2800 < 0.28, median 0.233 >= 0.18

The one failed check does not gate. Recruit and Regular both lose nearly every round to Elite, so their win rates against Elite cannot be told apart. The structure lead against Elite and the direct pairing order them instead.

## Findings

- **The ladder holds.** Every direct pairing goes to the stronger level, with the weaker level's Wilson interval below 0.5: Recruit 0.173 against Regular, Regular 0.240 against Veteran, Veteran 0.168 against Elite. The structure lead against Elite falls at each step: Elite +0.05, Veteran −2.61, Regular −4.58, Recruit −6.00, with disjoint intervals.
- **Retreat lines.** Recruit's health retreats start at a median of 0.485 health (10th percentile 0.40), Regular's at 0.381, Veteran's at 0.288 and Elite's at 0.233. None starts at or above its level's line.
- **Tower dives.** Recruit took essentially no tower shots in its 300 rounds (under 0.0001 per bot-minute). Regular took 0.002 dives per bot-minute, while Veteran and Elite took about 0.026. Elite dives more against weaker teams (0.059 against Recruit).
- **Fights while outnumbered.** Recruit and Regular take under 1% of their hero fights while outnumbered, against 6–8% for Veteran and Elite.
- **Threat retreats** dominate the two lowest levels: 0.57 (Recruit) and 0.77 (Regular) per bot-minute, against 0.01–0.03 retaliations. A trade-gate refusal started 89% of Recruit's and 81% of Regular's threat retreats; the rest were attackers out of reach or out of range. In the screening runs no single Regular gate changed its result against Elite.
- **Draws.** At most 6% per pairing (Elite against Elite, 12 of 200). Few rounds reach Victory within 20 minutes: none between Elite teams, 35 of 200 when Elite plays Recruit.
- **Side bias.** Green won 58.5% of decisive Elite-against-Elite rounds, Wilson [0.514, 0.653], and 65.5% of Veteran-against-Elite rounds. Sides alternate, so the level comparisons are balanced. The cause is not investigated; candidates are the map and the bot update order (Green bots take the lower slots and update first each tick).
- **Cost.** Bots plan a route 166–299 times per bot-minute, mostly because an empty route plans again on the next step, and each plan rebuilds the walkable grid. Route planning is most of the simulator's CPU time, and the online server runs the same brain.

## Limits

These are bot-against-bot results. In practice the chosen level plays both teams, so a human's difficulty is the opposing team's level together with allies of the same level. Human results are for the dev contour, not this report.
