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

## Durable gameplay / Ranked early-crash continuation

The server now distinguishes the allocation ACK from a committed gameplay
checkpoint. Both must arrive before combat or income. A cancelled unplayed
roster can still settle without credit if cancellation wins the activation-ACK
race; ordinary checkpoint/recovery paths cannot downgrade committed gameplay.

Finding #41 is verified and closed: ten signed Ranked humans were SIGKILLed
2.10 seconds after Running, before the first periodic ten-second checkpoint.
Recovery passed its actual >=120-second gate, kept the original result/epoch,
updated all ten profiles exactly once, exposed each player's own history, changed
no Elo/XP and allowed all ten to accept a fresh roster. Database inspection
confirmed initially rated=true, verdant-default-v1, committed gameplay and ten
humans/zero bots. Evidence is in
`.agent/codex-ranked-readiness/ranked-early-recovery/report.json`.

Current verification: 563 full PostgreSQL-backed API/store/server tests passed
with no ignores; server/store all-target clippy, formatting and Node syntax pass.
The full lifecycle process probe also passed after the new gameplay gate.
The added ordinary-checkpoint/Interrupted no-downgrade and pre-start-cancel
regressions passed against real PostgreSQL.

The unchanged client source now has completed CI evidence: the dev-contour job
on 548d152 passed 1553 tests (one ignored), including both new readiness
regressions. The main runner was externally shut down on its first attempt;
a rerun completed and every CI job for 548d152 is green. Current backend changes
have not required a new client/native build or any dependency/schema change.

P4 remains open for completed rated Victory (#14), public signed AI clients
(#40), loopback-only victory tooling (#42), and party/draft process evidence
(#43). The native RU screenshot and all other unresolved P5–P9 work remain
explicitly pending. No production deployment or configuration was changed.
