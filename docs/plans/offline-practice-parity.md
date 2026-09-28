# Offline practice gameplay track

Owner feedback: 2026-09-28 iPad playtest. Keep this separate from UI polish.

## Goal

Offline practice should offer a recognisable match with lanes, towers and minions,
plus an explicit optional skill-testing mode. Circular moving heroes must not be
the only/default experience presented as a normal match.

## Tasks

- [ ] Audit `net::offline` simulation versus the server's authoritative match:
      tower spawn/targeting/damage, protection, lane minion waves and aggro,
      bases, match victory/reset and resource rewards.
- [ ] Reuse shared simulation rules where practical; do not copy server combat
      or economy rules into a second independently maintained implementation.
- [ ] Add practice presets: normal lane match, stationary targets, moving targets.
      Moving targets explicitly opt in; keep the choice in offline-practice settings.
- [ ] Add scoped debug controls: target team/class, movement on/off and speed,
      target health, respawn/reset, cooldown/resource reset and player level/gold.
      Clearly distinguish local practice from online games and respect debug access.
- [ ] Support resetting a practice session without returning through the main menu.
- [ ] Verify tower destruction, minion progression, skills and full reset in headless
      simulation, then test touch controls and performance on iPad.

## Source implementation in 0.28.3

The follow-up TASK-IPAD-FRAME-OFFLINE-LANES-2026-09-28 adds authored structures,
protection ordering, both-team three-lane waves, autonomous minion/tower combat,
and player targeting of those entities. It uses shared structure stats and lane
routes; offline minion tuning currently mirrors server constants locally.
Character testing remains available alongside the lane simulation.

This is not yet full match parity: practice stays open after base destruction,
there are no career rewards, and rejoining is still the full reset path. The
regression tests were written but not executed under the low-disk build stop.
The unchecked verification and broader preset/economy tasks above remain open.
