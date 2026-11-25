# Bot personas: S3 difficulty simulator and calibration — 2026-10-14

The S3 package of [docs/plans/bot-personas.md](../plans/bot-personas.md): a headless simulator of full bot-only 5v5 rounds, and the bot-level calibration it supports. The raw report is [2026-10-14-bot-difficulty-report.json](2026-10-14-bot-difficulty-report.json). This is the acceptance matrix, run after the review fixes; it replaces the S3 calibration commit's matrix (base seed 0, 100 rounds per direct pairing), whose numbers are in that commit.

## Setup

- Code: `feat/bot-levels` at `f1c3a6f`: the S3 calibration (Veteran without the outnumbered hold and threat response), the `resume_hp` hold yielding to the defend exception, and the tower tie-break between equally near minions. Later commits change only docs, comments and an `#[ignore]` reason.
- Host: the owner's Mac (M4 Pro, 14 cores, macOS), release build, 14 threads at nice 10. Wall time 2,078 s (35 minutes); a gate build shared the cores for the first few minutes.
- Rounds: the standard practice fill on both sides, sides alternating, 50 ms steps with a seeded ±20% jitter (base seed 1; the calibration sweeps used base seed 0, so no round here picked a knob value). A round ends at Victory or at 20 game minutes; a capped round goes to the side that removed more enemy structure health, and a lead under 0.05 structures is a draw.
- Pairings: Elite against Elite, each lower level against Elite, Regular against Veteran and Recruit against Regular, 200 rounds each.

Command as run (the prebuilt release test binary, from `common/`):

```sh
cargo test --release --locked -p common --features test-support --test bot_difficulty_sim --no-run
OMOBA_BOT_SIM_PAIRINGS='elite:elite@200,veteran:elite@200,regular:elite@200,recruit:elite@200,regular:veteran@200,recruit:regular@200' \
  OMOBA_BOT_SIM_SEED=1 OMOBA_BOT_SIM_THREADS=14 OMOBA_BOT_SIM_REPORT=report.json \
  nice -n 10 <target>/release/deps/bot_difficulty_sim-<hash> --ignored --exact --nocapture full_matrix > /dev/null
```

`OMOBA_BOT_SIM_MATCHES=200 OMOBA_BOT_SIM_SEED=1 OMOBA_BOT_SIM_REPORT=report.json cargo test --release --locked -p common --features test-support --test bot_difficulty_sim -- --ignored --exact --nocapture full_matrix > /dev/null` plays the same pairings and seeds. The thread count does not change the result. Another `libm` plays different, equally valid rounds.

## Outcomes

"Subject win rate" counts decisive rounds only. The structure lead is the subject's mean lead in structures removed, with a 95% normal interval. "Green share" is the share of decisive rounds Green won.

| Pairing (subject : opponent) | Rounds | Subject W–L–D | Subject win rate (decisive) | 95% Wilson | Victories | Structure lead (subject) | Green share |
| --- | --- | --- | --- | --- | --- | --- | --- |
| elite : elite | 200 | 101–87–12 | 0.537 | [0.466, 0.607] | 1 | +0.17 ± 0.31 | 0.65 |
| veteran : elite | 200 | 43–154–3 | 0.218 | [0.166, 0.281] | 4 | -2.44 ± 0.38 | 0.70 |
| regular : elite | 200 | 3–197–0 | 0.015 | [0.005, 0.043] | 20 | -4.73 ± 0.29 | 0.51 |
| recruit : elite | 200 | 0–200–0 | 0.000 | [0.000, 0.019] | 32 | -5.87 ± 0.23 | 0.50 |
| regular : veteran | 200 | 53–146–1 | 0.266 | [0.210, 0.332] | 0 | -1.77 ± 0.34 | 0.67 |
| recruit : regular | 200 | 39–160–1 | 0.196 | [0.147, 0.257] | 0 | -1.71 ± 0.28 | 0.55 |

## Behaviour per pairing side

Rates are per bot-minute on the field. Health retreats are the health-triggered starts only; re-triggers during a retreat are not counted. A tower dive is a hero's first tower shot after at least 3 s without one.

| Pairing, side | Kills / bot-min | Deaths / bot-min | Health retreats: n, median HP [95% CI], max | Gate retreats / bot-min (outnumbered, threat) | Threat retaliations / bot-min | Tower shots / bot-min | Tower dives / bot-min | Outnumbered share of hero fights |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| elite:elite, elite | 0.827 | 0.830 | 18488, 0.228 [0.227, 0.229], 0.280 | 0.000, 0.000 | 0.000 | 0.017 | 0.010 | 0.063 |
| elite:elite, elite | 0.822 | 0.837 | 18687, 0.228 [0.228, 0.229], 0.280 | 0.000, 0.000 | 0.000 | 0.020 | 0.009 | 0.062 |
| veteran:elite, veteran | 0.791 | 0.734 | 18900, 0.285 [0.284, 0.286], 0.330 | 0.000, 0.000 | 0.000 | 0.035 | 0.019 | 0.083 |
| veteran:elite, elite | 0.730 | 0.802 | 17725, 0.229 [0.228, 0.231], 0.280 | 0.000, 0.000 | 0.000 | 0.034 | 0.018 | 0.057 |
| regular:elite, regular | 0.554 | 0.528 | 7839, 0.376 [0.376, 0.376], 0.400 | 0.000, 0.878 | 0.014 | 0.010 | 0.001 | 0.009 |
| regular:elite, elite | 0.522 | 0.582 | 15060, 0.241 [0.240, 0.242], 0.280 | 0.000, 0.000 | 0.000 | 0.074 | 0.032 | 0.043 |
| recruit:elite, recruit | 0.489 | 0.419 | 6880, 0.482 [0.481, 0.483], 0.500 | 0.444, 0.681 | 0.030 | 0.000 | 0.000 | 0.011 |
| recruit:elite, elite | 0.405 | 0.523 | 13468, 0.247 [0.246, 0.248], 0.280 | 0.000, 0.000 | 0.000 | 0.208 | 0.056 | 0.054 |
| regular:veteran, regular | 0.394 | 0.476 | 8284, 0.381 [0.380, 0.382], 0.400 | 0.001, 0.771 | 0.010 | 0.048 | 0.003 | 0.007 |
| regular:veteran, veteran | 0.468 | 0.411 | 15667, 0.299 [0.298, 0.300], 0.330 | 0.000, 0.000 | 0.000 | 0.126 | 0.048 | 0.054 |
| recruit:regular, recruit | 0.137 | 0.211 | 9271, 0.486 [0.486, 0.487], 0.500 | 0.224, 0.350 | 0.001 | 0.005 | 0.000 | 0.005 |
| recruit:regular, regular | 0.187 | 0.153 | 8888, 0.387 [0.386, 0.387], 0.400 | 0.001, 0.485 | 0.000 | 0.047 | 0.006 | 0.003 |

The highest health at a retreat start prints as the level's line after rounding. Every start was strictly below it, as the `health_retreats_*` checks confirm.

## Checks

- PASS `elite_vs_elite_contains_half`: Wilson95 [0.466, 0.607]
- PASS `vs_elite_recruit_below_regular`: 0.000 < 0.015, one-sided p = 0.0411; both lose nearly every round to Elite (Wilson upper < 0.10), so this is not gating
- PASS `vs_elite_regular_below_veteran`: 0.015 < 0.218, one-sided p = 0.0000
- PASS `vs_elite_veteran_below_elite`: 0.218 < 0.537, one-sided p = 0.0000
- PASS `vs_elite_lead_recruit_below_regular`: -5.87 ± 0.23 < -4.73 ± 0.29 structures
- PASS `vs_elite_lead_regular_below_veteran`: -4.73 ± 0.29 < -2.44 ± 0.38 structures
- PASS `vs_elite_lead_veteran_below_elite`: -2.44 ± 0.38 < 0.17 ± 0.31 structures
- PASS `recruit_loses_to_regular`: subject Wilson95 [0.147, 0.257]
- PASS `regular_loses_to_veteran`: subject Wilson95 [0.210, 0.332]
- PASS `veteran_loses_to_elite`: subject Wilson95 [0.166, 0.281]
- PASS `draws_at_most_10pct_elite:elite`: 12 of 200 rounds
- PASS `draws_at_most_10pct_veteran:elite`: 3 of 200 rounds
- PASS `draws_at_most_10pct_regular:elite`: 0 of 200 rounds
- PASS `draws_at_most_10pct_recruit:elite`: 0 of 200 rounds
- PASS `draws_at_most_10pct_regular:veteran`: 1 of 200 rounds
- PASS `draws_at_most_10pct_recruit:regular`: 1 of 200 rounds
- PASS `health_retreats_recruit`: 16151 starts, max 0.5000 < 0.5, median 0.485 >= 0.40
- PASS `health_retreats_regular`: 25011 starts, max 0.4000 < 0.4, median 0.383 >= 0.30
- PASS `health_retreats_veteran`: 34567 starts, max 0.3300 < 0.33, median 0.291 >= 0.23
- PASS `health_retreats_elite`: 83428 starts, max 0.2800 < 0.28, median 0.233 >= 0.18

`vs_elite_recruit_below_regular` passes the z-test this time but does not gate. It rests on 3 wins in 400 rounds, where the normal approximation is weak: Fisher's exact one-sided p is 0.12. Recruit and Regular both lose nearly every round to Elite, so their win rates against Elite cannot be told apart reliably. The structure lead against Elite and the direct pairing order them instead. Using that fallback awaits owner sign-off (plan, S3 acceptance).

## Findings

- **The ladder holds.** Every direct pairing goes to the stronger level, with the weaker level's Wilson interval below 0.5: Recruit 0.196 against Regular, Regular 0.266 against Veteran, Veteran 0.218 against Elite. Against Elite the win rates fall Elite 0.537, Veteran 0.218, Regular 0.015, Recruit 0.000; the Wilson intervals are disjoint at every step except Recruit to Regular. The structure lead against Elite falls at each step: Elite +0.17, Veteran −2.44, Regular −4.73, Recruit −5.87, with disjoint intervals.
- **Retreat lines.** Recruit's health retreats start at a median of 0.485 health (10th percentile 0.41) against its 0.50 line, Regular's at 0.383 (0.40), Veteran's at 0.291 (0.33) and Elite's at 0.233 (0.28). None starts at or above its level's line.
- **Tower dives.** Recruit took almost no tower shots (0.003 per bot-minute, no dives). Regular took 0.004 dives per bot-minute, while Veteran and Elite took 0.025–0.034. Elite dives more against weaker teams (0.056 against Recruit).
- **Fights while outnumbered.** Recruit and Regular take under 1% of their hero fights while outnumbered, against 6–7% for Veteran and Elite.
- **Threat retreats** dominate the two lowest levels: 0.51 (Recruit) and 0.71 (Regular) per bot-minute, against 0.01–0.03 retaliations. A trade-gate refusal started 92% of Recruit's and 83% of Regular's threat retreats; the rest were attackers out of reach or out of range.
- **Draws.** At most 6% per pairing (Elite against Elite, 12 of 200). Few rounds reach Victory within 20 minutes: 1 between Elite teams, 32 of 200 when Elite plays Recruit.
- **Side bias.** Green won 64.9% of decisive Elite-against-Elite rounds, Wilson [0.578, 0.714], 70.1% of Veteran-against-Elite rounds and 67.3% of Regular-against-Veteran rounds. Sides alternate, so the level comparisons are balanced. The cause is not investigated; candidates are the map and the bot update order (Green bots take the lower slots and update first each tick).
- **Cost.** Bots plan a route 166–299 times per bot-minute, mostly because an empty route plans again on the next step, and each plan rebuilds the walkable grid. Route planning is most of the simulator's CPU time, and the online server runs the same brain. On the Mac a round of Elite against Elite took about 0.8 s of wall time on 14 threads, and Recruit against Elite 3.0 s.

## Limits

These are bot-against-bot results. In practice the chosen level plays both teams, so a human's difficulty is the opposing team's level together with allies of the same level. Human results are for the dev contour, not this report.
