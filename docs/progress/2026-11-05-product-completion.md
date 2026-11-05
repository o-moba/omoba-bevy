# Product-completion checkpoint — 2026-11-05

Owner scope is all 68 findings and all nine critic items. The authoritative
sequence and individual statuses are in [TODO](../plans/TODO-LOBBY-2026-11-05.md).
This checkpoint does not close that entire scope or authorize RU deployment.

## Implemented

- P1: named missing party members, bounded 60-second gathering and exact bounded
  two-team packing, including `[4,4,3,3,2,2]`.
- P2: unplayed allocations do not change career counters, history or analytics;
  Casual removes slow loaders and fills bots without resetting surviving picks;
  Ranked aborts for re-search. Join deadline and named loading waits are exposed.
- P3: every human explicitly accepts one immutable roster within exactly 15 s.
  No worker before unanimity. A timeout requeues accepted humans with original
  FIFO age; declined/missed confirmations carry persisted host-local escalating
  cooldowns (60/300/900/3600 s within 24 h). Stale answers cannot accept a new roster.
- C1: SIGTERM/SIGINT finalizes the round, confirms a generation-bound fsync
  barrier and forces final status publication before exit. An already completed
  Victory remains completed. Lobby termination preserves live worker processes.
- C9: monitor detects terminal receipts pending >=5 min, corrupt/oversized
  receipts and bounded-scan overflow; active checkpoints do not alert.
- Protocol 13 gates the new strict ready-check enums; client and server need a
  coordinated release. Existing JSON match results default to played for compatibility.

## Verification actually completed

- Full `make check`: 2486 Rust tests passed, 94 ignored; 628 script tests with
  nine skips; 62 iOS-tooling tests. This precedes the final ready-fixture ordering
  follow-up and its new UI regression. No physical-device installation occurred.
- Full disposable PostgreSQL 18 gate: 562 tests passed, none ignored. Covers
  unplayed settlement/recovery, API history/analytics, cooldown restoration,
  party opt-out regression and outbox barrier acknowledgement.
- Final `cargo clippy --locked --workspace --all-targets --no-deps -- -D warnings`
  passed after the last changes; formatting, diff whitespace and Node syntax pass.
- Current server ready-check process rehearsal PASS: nine accepts, 15 s timeout,
  no worker before unanimity, FIFO age retained, late answer ignored, one worker
  after a replacement roster unanimously accepts. Local evidence:
  `.agent/codex-ranked-readiness/product-ready-check-confirmed/report.json`.
- Current server termination process rehearsal PASS: SIGTERM to the owned worker,
  one saved Interrupted result, no Elo/XP changes; completed in 1.56 s. Evidence:
  `.agent/codex-ranked-readiness/product-termination-final/report.json`.
- Monitor: 24 isolated Python tests pass. No actual alerts were sent.

## Pending verification and product work

The Russian 852x393 native visual capture has no successful screenshot. The
computer-use service reports that the Mac is locked. The capture exposed fixture
scheduling that could hide the ready state; the source now explicitly orders it
and adds a renderer-free accept/decline regression. Its repeat was interrupted
while generating native code when system free space fell to ~17 GiB, as required
by the umbrella AGENTS.md. This last test is **not** claimed as passed locally.
The current source is statically checked; CI must execute that regression.
No further heavy client build is started. No cargo cache or user files were deleted.

Full Ranked victory/crash/party journeys, AFK/leaver/remake/surrender policy,
remaining client findings, multi-host/cross-host penalties, snapshot traffic,
1000-player evidence, drain, PITR and real RU capacity/deployment remain in TODO.
The first-ten-seconds crash case must durably distinguish gameplay from a draft.
No production host, database, secret, CI configuration or deployment was changed.
