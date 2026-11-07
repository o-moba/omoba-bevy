# Legacy local queue penalties survive shared admission

A lobby now captures persisted aggregate penalty snapshots on startup and pauses
new admissions/selections until their exact SQL acknowledgements arrive. Existing
allocations/Resume use the existing path. Known pending event IDs drain first;
retries and conflicting/old acknowledgements cannot erase the retained snapshot.
If an adopted draft adds a penalty during startup, its newer snapshot supersedes
the older pending import. Storage failure keeps new search closed.

The trusted store API holds the existing profile lock and merges counter/deadline
floors by maximum. It never adds the legacy counter to known shared events or
manufactures missing historical event receipts. Expired deadlines stay expired;
strike history older than24h is not revived. A subsequent actual dodge advances
the shared counter normally. Invalid identity/counter/time, missing profiles and
clock skew beyond5s fail rather than becoming an empty success. The bound accepts
existing5s observation tolerance plus whole-second cooldown-view rounding; repeated
cache import does not accumulate time grace. No schema/grant/dependency is added.

## Evidence

[All87 store cases](store-all.log), actual isolated PostgreSQL18.6, PASS with zero
failures/ignores. Five new cases cover idempotence, shared refusal/no career credit,
expired/stale counters, invalid/future/unknown input, concurrent overlapping owners,
known receipt replay and clock/second-rounding cache roundtrip. Floors create no
queue event rows; only actual new events do. [Failed fixture cleanup](rejected-fixture-cleanup.json)
records the rejected first run's wrong device-key fixture and exact owned cleanup.

[All555 actual server cases](server-all.log), including ten real-PG cases, PASS
with zero failures/ignores. Two new port cases prove bounded work plus undrained
ACKs and full payload matching; two actual lobby cases prove the startup gate,
preserved queue age, known event ordering and rejection of an older snapshot ACK
after an adopted draft's new penalty. Strict actual full-server test/production
and store Clippy PASS; formatting, four Node policy self-checks and5 harness tooling
tests PASS. [Source/build scope](summary.json).

The [live regression](live-report.json) PASS uses two real signed UDP lobbies with separate epochs, roots
and outboxes. It imports one active and one expired versionless snapshot, checks
both hosts, then makes one real decline and restarts the importing lobby. It
requires no career changes, source-file rewrite, invented producer events or
second-host worker. It is part of the ordinary PostgreSQL harness. The final
report is archived with this checkpoint; the [rejected timing attempt](rejected-live-attempt.json)
records why single-shot profile reads were retried without changing product assertions.

## Rollout and remaining scope

A floor is a lower bound from an aggregate, not recoverable exact event history.
Overlapping old hosts' unknown histories are merged by maximum, not guessed sums;
known typed event IDs retain their existing exact SQL deduplication. Every old
host's authoritative penalty snapshot must be present/imported before global
admission opens during rollout. An offline/lost snapshot cannot be inferred from
another host. The actual C2 host-tool proposal is still awaiting the existing
owner decision; no RU host rollout is performed here.

Career remains schema12/portal11. Native current common/passport/shared/store and
the complete real server target compile against the coherent read-only pinned
cache graph from the previous [runtime checkpoint](../ready-runtime-2026-11-07/README.md).
The archived drivers are workspace/cache specific, not release tooling. Root disk
remains below20GB; there is no graphics/third-party rebuild, Cargo cache mutation,
new cache or dependency. Owned small artifacts stay under the ignored
`.agent/codex-ranked-readiness/ready-runtime` path.

Fresh complete CI/client validation is still required. Actual multi-host/WAN fault
and Resume, ready/journal retention/health, schema12 real recovery, trust packaging/
provisioning, physical Android/UI/capacity/RU gates and C2/C4 host-tool decisions
remain open. The original68 finding ledger is still62 done,5 partial,1 open.
