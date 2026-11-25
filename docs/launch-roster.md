# Launch roster playtest

This is the candidate for the first controlled human 5v5 playtest on Verdant
Confluence. It provides two choices per duty under the existing draft, which
allows the opposing team to mirror a class. Approval and public enforcement
are tracked in [LR-45](plans/launch-readiness.md#lr-45--launch-hero-roster).

| Duty | Candidate pair | Choice to verify |
| --- | --- | --- |
| Solo | Warrior, Cinderforge | Frontline dueling or opening a fight by changing a route |
| Jungle | Warden, Stormfist | Stable clears or early lane pressure |
| Mid | Mage, Orbitwright | Straightforward ranged damage or control of a contested position through the orb |
| Carry | Ranger, Wildspark | Range and single-target pressure or weapon switching and group pressure |
| Support | Chainkeeper, Frostguard | Catch and rescue or absorb incoming fire and protect teammates |

Keep Cleric in the playtest rotation as the first additional support once its
ally healing and barrier have been tried with both carries. Edgeweaver is the
next solo option. Try Dawnweaver as an alternative mid, Riftshot after validating
seal and blink decisions, and Emberveil, Veilstalker and Adventurer once their
mobility, concealment and openings have been tested. Keep their current models
and kits accessible in Combat Test throughout the iteration.

## Balance changes in this iteration

- Warden keeps fast clears and extra camp gold. Camp experience comes from
  the common economy.
- Wildspark commits to a visible 0.45 s Last Spark windup. The rocket reaches
  60 m and its missing-health bonus applies to the hero hit directly.
- Cleric can heal an injured teammate and use a nearby team barrier.
- Adventurer earns a backstab from actual position. Bluff holds the enemy's
  facing, and Backstab resolves without a random near-lethal result.
- Riftshot can blink away freely. The follow-up shot requires a visible enemy
  already carrying Riftshot's own mark.

In the repeatable live-camp opening probe, Warden cleared the same three camps
0.90 s faster than Stormfist on both sides. Both ended at level 2 with 195 XP
and full health. Warden earned 171 camp gold and Stormfist earned 122. This
route leaves out enemy pressure and passive income.

## Simulator comparisons

Run the launch matrix from the game repository. It uses the same in-process
combat simulation as practice, with ten Elite bots starting at level one on
Verdant Confluence.

```sh
OMOBA_LAUNCH_SIM_REPORT="$PWD/launch-roster-sim.json" \
  cargo test --release --locked -p common --features test-support \
  --test bot_difficulty_sim -- --ignored --exact --nocapture launch_roster::launch_roster_matrix
```

Each comparison changes one duty and gives the two teams the same four allied
classes. Each lineup plays once on each side with the same tick schedule. The
default 16 pairs per duty cover every combination of those four allies. Use
`OMOBA_LAUNCH_SIM_PAIRS`, `OMOBA_LAUNCH_SIM_CAP_MIN` (default 45 minutes), and
`OMOBA_LAUNCH_SIM_SEED` to repeat the comparison at a different sample size,
round length, or tick schedule.

The report records actual base wins and results at the time cap separately.
At the cap, compare the fractions of enemy structure health removed, counting
each destroyed structure as one. A lead under 0.05 is a draw. A pair favors
the class that wins more of its two games, and a pair with equal wins is tied.
The 95% Wilson bands describe the first class's share of decisive pairs, once
using base wins alone and once including the results at the cap. Keep the
number of tied pairs, actual base wins and the structure score beside the
bands. The bands describe the tested lineups and tick schedules. The bots
clear ordinary camps and gank opportunistically. Use their results to
prioritize human scenarios, including deliberate boss rotations.

### First two runs

Each run used the full 16 paired lineups per duty and a 45-minute cap. Seeds 1
and 2 produced 320 games in total. Seven ended with a base destroyed and 313
reached the cap. Both runs favored Green in the raw counts, which makes the
side swap material. The table counts only decided pairs, in class order.

| Pair | Seed 1 | Seed 2 | Base wins across both runs |
| --- | ---: | ---: | ---: |
| Warrior / Cinderforge | 1–6 | 2–4 | 1–1 |
| Warden / Stormfist | 2–3 | 3–4 | 2–0 |
| Mage / Orbitwright | 8–1 | 0–2 | 1–1 |
| Ranger / Wildspark | 3–7 | 2–3 | 0–0 |
| Chainkeeper / Frostguard | 0–5 | 4–3 | 1–0 |

Cinderforge and Wildspark led on structure pressure in both runs. Compare
their lane pressure and the defender's response in the first human matches.
Decided pairs for mid and support reversed across schedules. Mage can damage
exposed structures with targeted spells, while coordinating Orbitwright's orb
and using Chainkeeper's lantern rely on decisions the current bots exercise
only partly. Test those decisions with human teammates before using those
comparisons to tune damage or shields. The Warden's repeatable camp gold lead
is a separate early-game question from the whole-match comparison.

## Scenarios to record

Record the build, side, player count, actual roles, input device and whether
bots took over any seat. Use the same players and exchange sides between
paired matches. Use nonmirror results when comparing class win rates.

| Scenario | Capture | Review question |
| --- | --- | --- |
| Warden vs Stormfist jungle start | First comparable clear time, HP, gold, XP, first purchase after the starter and first opportunity to enter a lane | Can Stormfist find a useful early gank while Warden farms? |
| Ranger and Wildspark vs both supports, with sides exchanged | First damaging cast, response opportunity, saves, ally interaction with Guiding Lantern and pressure after missed abilities | Can an opponent dodge Last Spark or change the angle around Northwall, and can a threatened ally recognize and use the lantern in time? |
| Rotate both solos and mids through an objective fight | Position before the fight, pillar and orb placement, time from orb arrival to W or R, longest forced control sequence and time until the target can act | Can players coordinate around the orb and change their route through contested space after the first hit? |
| Cleric with each carry | Useful healing and barrier saves, failed casts, visibility of casts and recipients | Can players deliberately reach allies to protect them and understand the resulting heal or barrier? |
| Adventurer and Riftshot in Combat Test | Front versus rear hit, visible sealed target versus empty escape | Do earned position and a prepared mark change the outcome predictably? |

Playtest one desktop or mobile view first and record the exact viewport. Look
for repeatable failures of readability or response before interpreting pick or
win rates from a small batch. The automated [legacy combat matrix](balance-tuning.md)
uses a stationary target, no items and only five early classes. Treat it as a
regression check. Scenario tests prove particular interactions. The release
decision still needs observations of both players in a live exchange and of
the candidate lineup in human 5v5.
