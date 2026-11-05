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

- Full local `make check`: 2493 Rust tests passed, 95 intentional ignores;
  634 Python tests with nine skips; 62 iOS-tooling tests. Both readiness
  regressions pass locally. No physical-device installation occurred.
- Full disposable PostgreSQL 18 gate: 567 tests passed, none ignored. Covers
  allocation/gameplay commitment, unplayed recovery/history/analytics,
  cooldown restoration, party opt-outs and the outbox barrier.
- Final workspace/all-target clippy with warnings denied, formatting, diff
  whitespace and six rehearsal-validator tests pass. Headless harness:
  55 passed, two opt-in scenarios ignored.
- Ready-check process rehearsal PASS: nine accepts, 15 s timeout, no early
  worker, retained FIFO, rejected stale answer and one unanimous replacement.
  `.agent/codex-ranked-readiness/product-ready-check-confirmed/report.json`.
- Termination process rehearsal PASS: one saved Interrupted result, no Elo/XP,
  1.56 s. `.agent/codex-ranked-readiness/product-termination-final/report.json`.
  The later full Ranked proof also preserves its already completed/rated Victory
  after owned-worker shutdown, verified directly in PostgreSQL.
- Ten-human standard Ranked Victory and early crash, plus explicit custom
  unrated Victory, pass as detailed below. All owned probe processes were stopped.
- Monitor: 24 isolated Python tests pass. No actual alerts were sent.

## Pending verification and product work

The Russian 852x393 native capture is still pending: the computer-use service
reports the Mac is locked. The fixture ordering fix and both readiness
regressions passed in CI; no native screenshot is claimed. No further heavy
native client build is needed for the current harness work.

Remaining CI automation, AFK/leaver/remake/surrender policy,
remaining client findings, multi-host/cross-host penalties, snapshot traffic,
1000-player evidence, drain, PITR and real RU capacity/deployment remain in TODO.
The first-ten-seconds crash case is now closed below.
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

Durable-gameplay checkpoint verification: 563 full PostgreSQL-backed API/store/server tests passed
with no ignores; server/store all-target clippy, formatting and Node syntax pass.
The full lifecycle process probe also passed after the new gameplay gate.
The added ordinary-checkpoint/Interrupted no-downgrade and pre-start-cancel
regressions passed against real PostgreSQL.

The unchanged client source now has completed CI evidence: the dev-contour job
on 548d152 passed 1553 tests (one ignored), including both new readiness
regressions. The main runner was externally shut down on its first attempt;
a rerun completed and every CI job for 548d152 is green. Current backend changes
have not required a new client/native build or any dependency/schema change.

P4 retains only remaining CI automation (#31); party/draft process evidence (#43) is closed below. The native RU screenshot and all other unresolved P5–P9 work remain
explicitly pending. No production deployment or configuration was changed.


## Public signed AI continuation

Finding #40 is implemented. Ten independent ephemeral-key AI clients authenticated
at the public lobby, explicitly accepted its ready roster, followed the one
rated allocation, re-authenticated in the worker, locked/loaded and entered live
5v5 Ranked play. Normal movement, purchases and casts were observed. Keys never
enter diagnostics or evidence files. The initial process smoke passed; see
`.agent/codex-ranked-readiness/signed-ai-bots/report.json`.

A reusable loopback-only rehearsal now verifies identity/liveness under fog of
war. Its full terminal mode checks a saved completed result and each player's
profile, rating/XP and exactly one history row. The standard-rules run in
`.agent/codex-ranked-readiness/ranked-ai-victory/report.json` PASSED: canonical
verdant-default-v1, result `99e131dbd302451ee593eac48cba0ee4`, real Victory after
406.115 s, all ten profiles credited once. Winners +16 rating /150 XP, losers
-16 /100 XP, exactly one history entry each. Total wall time 422.50 s; all owned
processes were stopped. No custom map, shortened cooldown, damage override or
direct settlement was used. Finding #14 is closed.

Current verification: full local `make check` passed 2490 Rust tests (95
intentional ignores), 634 Python tests (nine skips) and 62 iOS-tooling tests.
Both readiness regressions pass locally as well as in CI. The headless harness
passed 55 tests (two opt-in ignores); six rehearsal validator tests pass.
The final objective CLI follow-up passes server/harness all-target clippy and
the real signed terminal rehearsal below.


## Loopback terminal accelerator

Finding #42 is closed. Explicit map forwarding checks both actual bind addresses;
workers independently reject the override for an exposed bind and remove the
test setting from their inherited environment. Eight pool tests cover the guard,
IPv4/IPv6 and ordinary override removal. Public client `--rush-objectives` is
loopback-only and uses ordinary signed movement/casts, with all gameplay checks
left authoritative. The standard Ranked proof above does not use it.

The original custom-map run with strategic lane AI hit its 300 s deadline. The
explicit objective strategy completed the replacement proof in 50.87 s total,
33.629 s played: saved Completed `fd3dd7e0aaeed5a9d798d95690cf447a`, ten humans,
custom-unrated-v1, zero Elo/XP for every fresh profile and one matching history
entry each. PostgreSQL confirms both durable allocation and result unrated.
Evidence: `.agent/codex-ranked-readiness/custom-unrated-objective-victory/report.json`.
All probe processes were stopped; the old timed-out evidence is retained.

Previous pushed head b027ba7 is fully green in CI. The next head has not yet
completed CI. No production host, service configuration, secrets, deployment or
CI workflow files were changed in this continuation. Native RU visual evidence
still needs the Mac unlocked; other product findings retain their full scope.


Final full PostgreSQL gate for the public-AI/loopback-map continuation passed
564 API/store/server tests, zero ignores and zero failures. Final formatting,
whitespace and server/harness all-target clippy pass. The latest headless unit
rerun passed 25 tests. Full local build stayed above the 20 GiB system-disk
threshold and used only the existing shared cache B; no cargo caches were removed.


The final rebuilt objective binary was rechecked with a fresh owned fixture:
`.agent/codex-ranked-readiness/custom-unrated-objective-confirmed/report.json`,
saved result `e577592f609e17ac2fece91beb6649fc`, 49.05 s wall /33.741 s played,
all ten profiles/history checked with zero rating/XP. Final workspace/all-target
clippy passes after that CLI follow-up.


## Party/draft continuation and public-probe handoff isolation

Finding #43 is closed with real signed five-stack/solo scenarios. Explicit Leave
and silent disconnect both preserve nine survivors' FIFO ages and their restored
five-stack membership, change none of the ten profiles/history, deliver the
native return code to all nine, then form a fresh unanimous Ranked worker.
The explicit leaver has exactly one persisted strike; a disconnect penalizes
nobody and permits the original identity to join the replacement roster.

These process tests exposed two runtime gaps: raw signed gameplay Leave could
remove a Ranked draft participant without cancelling the allocation, and retired
unplayed workers stopped publishing the native return signal. One scoped
cancellation helper now serves account cancel and gameplay Leave; both abort and
deadline paths stop simulation and repeat return views during retirement. Three
new regressions cover scope, repeated Leave, late Join, started-match preservation,
return publication and no income/simulation. No schema/protocol change.

Evidence:
- `.agent/codex-ranked-readiness/party-draft-return-confirmed/report.json`:
  explicit departure, six-second initial waiting ages preserved, no collateral
  penalties and fresh Ranked Running (29.16 s from search).
- `.agent/codex-ranked-readiness/party-disconnect-return-confirmed/report.json`:
  real deadline, retained 70/64-second queue ages, no penalties, full rejoin
  and Running (82.26 s from search).

Current full local validation: 2493 Rust passed /95 intentional ignores,
634 script tests /9 skips, 62 iOS-tooling tests; 567 full PostgreSQL-backed
API/store/server tests, none ignored. Workspace/all-target clippy passes.

CI 67683d6 passed its unit/platform/client/harness jobs but failed its mixed
public probe with ten Ranked peers stalled at Joining; that job had no client
artifact upload. Investigation found and deterministically reproduced a Node
probe race: a queued callback from the old lobby socket could overwrite the
new worker's authentication. Socket-identity guards fix it. A 17-peer mixed
run with 17 deliberate old-callback deliveries passes (max 41.94 s to Running).
Bounded failure diagnostics now make any new CI stall inspectable. The old CI
root cause remains unproven without its lost artifacts; the next head needs CI.

Evidence: `.agent/codex-ranked-readiness/stale-handoff-red/report.json` and
`.agent/codex-ranked-readiness/mixed-stale-handoff-confirmed/report.json`.
All 68 findings and all nine critic items retain their full scope. #31, native
visual capture and P5–P9 remain open. No production deployment or infrastructure,
secret or CI workflow configuration was changed.


Final combined probe checks also pass with stale callbacks deliberately injected:
`.agent/codex-ranked-readiness/party-final-stale-handoff/report.json` verifies
20 guarded handoffs across the original and replacement party rosters;
`.agent/codex-ranked-readiness/lifecycle-party-followup/report.json` verifies
three guarded handoffs, same-seat reconnect and leave/resume for a running match.
No owned probe processes remain. These are runtime/probe changes; no native
client source, production dependency, schema, protocol or CI configuration changed.
