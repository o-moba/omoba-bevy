# Bounded ready maintenance — schema13

The schema12 ready ownership model retained every closed check/member receipt and
relied on future searches to reconcile orphaned claims. Schema13 adds indexed,
bounded maintenance without weakening consent, Resume or discipline identity.

At most128 expired forming/open/approved checks are reconciled per API call,
using the existing sorted participant/check locks and exact close transaction.
A healthy missed-answer deadline charges only missing answers once. Failed owner,
unpublished candidate or handoff expiry cancels without a player dodge. New
closure starts a fresh retention window. Busy profile locks defer after50ms;
maintenance statements have500ms limits.

A narrow owner-pinned function removes at most128 cancelled/expired/consumed
receipts older than7 days, with neither ready claims nor an unfinished SQL match.
Members cascade; immutable match admission/results, profile totals and queue-dodge
deduplication receipts are untouched. PUBLIC and portal have no EXECUTE; the game
has function EXECUTE but no direct DELETE on consent tables. A caller-owned temp
table cannot redirect the function. Missing consent still rejects every unknown
public admission; existing immutable admission remains the authority for Resume
and result retries. Random check identities are never reused for another roster.

One lobby/standalone async task starts after the default300-second reconnect
warmup and processes32 checks/minute, with a5-second overall deadline and skipped
missed intervals. Match workers do not start this task. It does not synchronously
run the batch ahead of a ready job or delete local restore spools. This does not
fix generic blocking reply/heartbeat behavior, which retains its separate load
acceptance requirement.

## Verified current behavior

- [All100 actual-PG18.6 store cases](store-all.log) PASS,0 failures/ignores.
  The log first confirms successful linking of the current test target before
  execution. Eight new cases cover bounded batches/freshness, unfinished games
  and settled retry authority, exact missed-answer discipline, failed ownership,
  locked/claimed receipts, busy profiles, schema12 upgrade with a real preserved
  penalty, and actual indexed query plans over10000 recent records. The extended
  real runtime-role case positively prunes through the game function, refuses
  direct DELETE/portal execution and invalid batches, and proves temp-shadow safety.
  [Current store provenance](store-proof.json); zero per-test schemas remain.
- [All564 complete actual-server cases](server-all.log) PASS,0 failures/ignores,
  including13 actual-PG cases and the existing known/unknown public admission,
  partial-restore evidence and backpressure/heartbeat regressions. Serial policy
  matches scripts/postgres_tests.py; this does not certify sustained host cadence.
- Strict actual [store](store-clippy.log) and [server test/production](server-clippy.log)
  Clippy, formatting/diff and C4 review-patch application checks PASS.
- [Signed15s ready journey](ready-check.json) PASS using actual current schema13
  lobby/workers. Nine accepted players retain FIFO age, no worker starts before
  unanimity, late acceptance cannot revive the expired check, and a replacement
  roster reaches Running. Synthetic prior eligibility receipts and AI controllers
  are disclosed; these are not physical testers or ten real qualification games.
- [Actual production scheduler](background.json) PASS with the unmodified300s
  warmup: a real API-created closed receipt in an isolated PG18.6 schema is pruned
  after304.71s. [Lobby output](background-lobby.log) reports one prune and durable
  shutdown. No member/claim remains; profile still exists with zero played games,
  no match and no penalty event. The process exits and its own schema is removed.
  [Fixture source](prepare_fixture.rs), [local runner](run_background.py).
  This verifies idle scheduler wiring, not multiplayer load/WAN or live RU roles.

The [owned public fixture transition](owned-fixture-transition.json) records12→13;
the final run independently uses13. [Rejected preparation attempts](rejected-attempts.json)
are excluded: an incorrect public-schema expectation in a targeted harness, a
test-only Option assertion compile error followed by an old-binary run, and a
manual probe quoting error. The final current linked100-case run and actual
scheduler proof are the accepted artifacts.

Actual current full server/common/passport/shared/store sources are compiled
against the coherent pinned read-only cache. Production common excludes
test-support. [Source/binary provenance](summary.json) binds acceptance to this
source; server compiler flags match the archived legacy-penalty-floors graph.
No graphics/third-party compilation, Cargo-cache mutation, dependency or new
cache occurs. Root disk remains below20GB; full graphics/platform checks use CI.

## Release scope

Paired013 migration, function grants and runtime binaries are required. Runtime
expects all13 career versions; a schema12 binary cannot be rolled back onto13.
Portal stays11 and wire protocol16. No RU migration, runtime grant application,
configuration, credential, season reset or deployment occurs in this checkpoint.

Historical aa2e4cb4 CI42334461242 has six successful jobs, including the fixed
Linux core journal regression and current client; PostgreSQL/public journeys were
still running at this checkpoint. New full CI must certify schema13 source.
Ready/floor SQL availability and failures before durable receipts remain health
work. Schema13 physical/WAL/off-host recovery and restored permissions, controlled
heartbeat/load/capacity, trust packaging/live provisioning, actual multi-host/WAN,
current UI/physical Android and reviewed RU rollout remain open. C2/C4 decisions
remain pending. Original68 stays62 done,5 partial,1 open; no gate is waived.
