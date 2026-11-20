# Shared queue-dodge store foundation

Actual current shared/store source compiles and passes69 store tests against
PostgreSQL18.6, with no ignored cases. Six new real-DB scenarios cover independent
owners, same/different concurrent events,60/300/900/3600s escalation, conflicting
receipts, unchanged career/history/AFK state, expired delivery,24h quiet reset,
clock drift, out-of-order arrival and schema10→11 upgrade. Missing schema returns
an error. [Full source hashes and cleanup](source-and-cleanup.json),
[six cases](six-new-cases.log), [all69](all-store-tests.log), [summary](summary.json).

The exact native current source is compiled directly with coherent cached third-
party dependencies read-only. Current shared is rebuilt too, rather than using a
stale shared library. This renderer-free check creates only34MiB of owned test
artifacts in .agent/codex-ranked-readiness/queue-discipline; no Cargo cache or Bevy
build occurs. [Compiler inputs](compile.json), [reproducer](verify_store.py).
[Strict test-target clippy](clippy.log) includes the existing workspace allow
levels. The first driver attempt omitted these levels and is rejected; no
unrelated source was changed to silence it. Standard Cargo/PG16/full CI remains
required, so this proof does not certify the complete game or release.

The full suite includes existing supporter fixtures that open/migrate the task's
public test database. Its current schema is career11/portal11; the final run
observes11 before and after. All per-process temporary schemas are verified
absent afterward. No production database or RU service is touched. Do not use this
updated fixture as a schema10 compatibility proof for an older frozen binary.

Normal reproduction on an isolated PostgreSQL database with Account API
migrations and enough build space:

```sh
cargo test --locked -p omoba-career-store queue_discipline_ -- --ignored
cargo test --locked -p omoba-career-store -- --include-ignored --test-threads=2
```

Supply the isolated OMOBA_TEST_DATABASE_URL in the environment, never a real
server connection or a credential on the command line. Existing PostgreSQL
fixtures create synthetic records and may migrate the supplied owned database.

This is a foundation, not a closed cross-host bypass. Lobbies still use their
local penalty path. Runtime grants, durable event production/delivery, fresh
access/final-start checks, shared ready ownership/deadline fences, legacy-floor
reconciliation and actual two-lobby fault tests are required before activation.
The earlier C4 recovery proofs use source/schema10 and remain historical;
repeat the real recovery drill on schema11 before release acceptance.
