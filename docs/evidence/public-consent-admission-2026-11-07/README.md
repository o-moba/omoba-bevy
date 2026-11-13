# Public match admission always requires committed consent

Public lobby and match roles now select mandatory consent by their server recovery
scope. No client flag or receipt metadata can switch them to unmanaged standalone
admission. The dedicated `start_public` / `ensure_public_allocation` methods reject
a missing/closed/deleted ready receipt, require Approved consent, reject a rated
start authorized by another queue and require all10 Ranked human profiles. A new
allocation recovered from spool also rechecks current discipline/eligibility under
profile locks. Its failed transaction creates neither a match nor consumed consent.

An existing immutable SQL allocation remains the authority for retries and Resume,
including recovery after its ready receipt is removed or a new queue cooldown
appears. Fresh unacknowledged starts and already queued terminal messages are
retired on definitive refusal. Every recovered receipt stays durable for restore
review if the matching SQL admission is missing: Start, Checkpoint and old terminal
records are covered independently of newer JSON flags. No spool alone invents an
admission; restoring the exact previously committed SQL allocation permits normal
replay afterward. Progress/terminal statistics survive that wait.

Unmanaged standalone storage keeps its explicit existing APIs. Both production RU
roles use the mandatory path: the service sets `lobby`, the allocator sets `match`;
the role-to-scope mapping and all public scopes are inspected/tested. No schema,
wire protocol, grant, dependency or live RU setting is changed by this checkpoint.

## Verification

- [All92 actual PostgreSQL18.6 store cases](store-all.log) PASS,0 failures/ignores.
  Five new cases cover absent consent/no false career credit, committed allocation
  recovery without a second vote, closed/deleted receipts, fresh discipline on
  unknown recovery and Ranked queue/ten-profile/current eligibility boundaries.
- [All559 actual whole-server cases](server-all.log) PASS,0 failures/ignores,
  including13 real-PG cases. Three new actual worker cases cover queued terminal
  rejection, durable Start/Checkpoint/old-terminal restore evidence and existing
  unplayed allocation recovery. The scope policy has a separate native unit case.
- Strict actual server test/production and store Clippy PASS with existing
  workspace allowances; formatting/diff checks PASS.
- [Signed five-stack journey](party-draft.json) PASS: actual ten-human Ranked draft,
  one leave, nine retained with unchanged profiles/history/FIFO age, fresh request
  consent and a replacement Ranked match Running. Qualification setup remains
  explicitly synthetic prior public receipts, not ten real played games.

- [Casual handoff release](handoff-release.json) PASS with the new mandatory
  consent path: only the cancelled seat is replaced, nine original humans reach
  Running in the same worker, the canceller gets no career credit, and a late
  cancel after start keeps admission/Resume without aborting others.

- [Maintenance/restart/Resume](drain.json) PASS: no worker on a free arena while
  draining, existing play advances, old decline causes no strike, reopening needs
  fresh consent, and restarted lobby adopts live workers while allowing Resume.

- [Fresh15s confirmation](ready-check.json) PASS: nine accepted players retain
  FIFO age after one misses the deadline; no worker is launched for that roster;
  a late accept cannot revive it; a new unanimous roster launches exactly once.

The new worker restore tests own disposable schemas. Only their partial-restore
simulation temporarily toggles those schemas' append-only guards inside a single
transaction, and all guards are restored atomically. Production guards remain
strict. [Rejected preparation/parallel attempts](rejected-attempts.json) distinguish
fixture failures from acceptance; an initial combined-load run failed the unchanged
heartbeat timing assertion. Its intermediate [diagnostic target retry](heartbeat-targeted.log) and the
current full server run PASS with the same heartbeat assertion. The full run uses `test-threads=1`, matching
`scripts/postgres_tests.py`; host/SQL-load cadence remains a capacity acceptance
concern, not certified by that serial regression.

## Build/release scope

Actual current common/passport/shared/store and complete server sources are linked
against the coherent pinned cached graph, without copied engine modules. Production
common excludes test-support. [Exact compiler arguments](../legacy-penalty-floors-2026-11-07/server-compile.json)
are byte-identical to the prior archived graph; [current hashes/binary proof](summary.json)
bind them to this source. Root disk remains below20GB; no graphics/third-party
compilation, Cargo-cache mutation, new cache or dependency occurs. Native artifacts
remain under the ignored `.agent/codex-ranked-readiness/ready-runtime` path.

Fresh complete CI, ready/journal health/retention, legacy snapshot rollout, actual
multi-host/WAN failure/Resume, schema12 real PITR/receipt-replay policy, public trust
provisioning, physical UI/Android and RU capacity acceptance remain required.
Existing C2/C4 host-tool questions remain pending; there is no RU rollout here.
Original68 ledger remains62 done,5 partial,1 open. Removing unsafe absent-consent
fallback is also a prerequisite to safe ready-receipt retention, not completion
of that retention or the physical disaster-recovery gate.
