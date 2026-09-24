# Measured combat balance and progression

Task: `TASK-COMBAT-BALANCE-2026-09-23`; baseline `75fff6e`; release `0.23.0-rc.2`.

The reported opening burst came from 100 HP, Q intervals of 0.25–0.6s alongside
basic attacks, and independent slot clocks. The baseline production-path matrix
measured a 1.67s level-one median. Level growth previously increased resources
without corresponding movement or offensive growth.

Added shared capped class curves, class-specific starting durability, slower
initial Q, a short cross-slot recovery, authoritative skill snapshots, buffered
client input, and identical human/bot/prediction movement scaling. The candidate
48-row matrix has a 10.53s early median and a 4.24s level-ten mean. Basic attacks
stay independent, mana remains finite, and skill identities/unlocks are retained.

Measured the durability side effect on towers and introduced a default-profile
hero-only damage multiplier. Ordinary waves keep their original 14-shot tower
clear cadence; absent custom-map fields retain multiplier 1. Ranked default-map
validation includes the new value. No minion/jungle stats or reward curves changed.

Evidence and independent verification belong in
`.agent/tasks/TASK-COMBAT-BALANCE-2026-09-23/`. Raw baseline/candidate/repeat matrices,
objective isolation, wave timing and suite logs are retained there. The
production test harness and matrix validator are checked in. See
[balance tuning](../balance-tuning.md) for research, formulas and reproduction.

This is the first measured mechanical balance, not proof of competitive fairness.
The stationary close-range target favors Warrior and omits range control, team
composition and human decisions. Level-ten burst, healer sustain and real match
length should receive playtest attention. Both the game server and native client
must be rebuilt together to pick up movement/cooldown prediction changes.
