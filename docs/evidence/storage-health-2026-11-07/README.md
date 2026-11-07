# Career storage health before persistence

Required operational reports now expose failure even before the first receipt
can reach an outbox. Actual career workers publish atomic disposable
`<outbox>/health/status.json` every5s: authenticated SQL probe/time, journal
availability, staged-dodge/legacy-floor/unpersisted-result counts and the beginning
of a continuous blocked interval. No player/event IDs, DSNs or raw errors. These
files add no fsync and are separate from match results and recovery authority.

The asynchronous10s probe has a2s deadline. It checks schema13, writable primary
and the required ready/discipline/admission table/function privilege footprint.
Role, replica/read-only and schema faults cannot look healthy just because a TCP
port accepts connections. This is a preflight, not proof of every business rule
or transaction. Real transaction ACKs still authorize gameplay and event retirement.

The monitor requires the lobby report after120s startup grace and live worker
reports bound to their exact epoch, with missing-report grace through the worker
join deadline plus120s. Finished/failed workers are skipped. Persistent120s
blockers, reports older than60s, probes older than30s, corrupt/oversized/linked
files and incomplete bounded scans are actionable. Counts-only alerts reuse the
existing repetition/resolution state. Tests send no external notification.

## Reproduced reconnect failure

The [before-fix test](startup-before.log) restores an initially absent SQL version
but the worker remains disconnected, with no completed probe. Connection retries
shared `last_retry` with result delivery; even an empty delivery pass could reset
that clock before the next connection check. Separate `last_connect` scheduling
fixes the starvation without a restart or weaker consent/durability rules.

The same [after-fix actual worker test](startup-after.log) verifies six stages:
startup unavailable; a dodge retained only in memory while its journal directory
is blocked; schema restored; journal restored with one exact SQL ACK; connected
schema lost again; connected schema restored again. Before journal recovery there
is zero durable event; afterward exactly one, with all original identity intact.
The anonymous report contains neither profile nor event ID. The new test's isolated
schema is removed; [both rejected fixture schemas](rejected-fixture-cleanup.json)
are also verified/removed without changing another schema.

## Current verification

- [570 complete actual-server cases](server-all.log) PASS,0 failures/ignores,
  including14 real-PG cases. New units cover report age/recovery, unavailable
  journal/unpersisted result visibility, real failed-result-write recovery and
  atomic producer/reader interoperability. The full suite uses the official
  serial PG policy; it is not sustained latency/heartbeat-load acceptance.
- [101 actual-PG18.6 store cases](store-all.log) PASS,0 failures/ignores. The new
  preflight case uses lost/restored schema and a real read-only connection before
  player requests, with zero profiles/events created. The actual game/portal-role
  case verifies the required capability difference. [Store provenance](store-proof.json)
  records zero per-test schemas remaining.
- [42 monitor cases](monitor.log) and [74 host-tool cases](host-tools.log) PASS.
  New cases cover every blocker, missing/stale data, exact worker epoch, startup
  grace, terminal workers, invalid/future/inconsistent contracts, symlink/FIFO,
  bounded scans, identifier exclusion and resolution with a fake Telegram.
  [Final Rust-to-current-Python contract check](native-monitor-interop.log) PASS;
  the helper calls only read-only scanners, not systemd/network/alerts.
- Strict actual [store](store-clippy.log) and [server test/production](server-clippy.log)
  Clippy, formatting/diff, JS syntax and C4 review-patch application checks PASS.
- [Strengthened signed ready journey](ready-check.json) PASS: nine accepts, exact
  15s expiry without a worker, FIFO retention, late-accept rejection, a new
  unanimous roster and **all10 replacement humans actually Running**. Then the
  actual lobby and Ranked worker expose fresh authenticated healthy reports with
  exact epoch matching, no injected status. [Retained anonymous report audit](live-reports.json).
  Controllers are AI in human seats and qualification is synthetic prior public
  receipts; this is loopback gameplay, not physical players or WAN/capacity proof.

Earlier ready-check reports stopped the replacement at Assigned; the baseline bot
Running was a separate path. The prior retention README is corrected. A rejected
first strengthened audit used plain JSON Number and lost epoch precision; reusing
the existing lossless packet parser passes unchanged exact-u64 assertions.
[Rejected attempts](rejected-attempts.json) remain separate from acceptance.

Actual current full server/common/passport/shared/store sources use the coherent
pinned read-only cached graph; production common excludes test-support. The
[source/binary provenance](summary.json) binds the compiled Rust and current
separately verified monitor/harness to this checkpoint. Server compiler arguments
match the archived legacy-penalty-floors graph. No graphics/third-party compilation,
Cargo-cache mutation, dependency, new cache, schema, protocol or runtime-grant
addition occurs. Root disk remains below20GB; full platform/graphics CI is required.

## CI and remaining release scope

Historical aa2e4cb4 [CI42334461242](https://github.com/o-moba/omoba-bevy/actions/runs/42334461242)
is fully green in7 jobs. Downloaded artifact13559679363 matches the expected
SHA256/size, safe ZIP paths/size and CRC; all20 public stages PASS and private
fixture removal is reported, with no private-key path. [Exact parent inspection](parent-ci.json).
That schema12 parent does not certify current source; fresh full CI remains needed.

Ship report producer/monitor together through the reviewed drained rollout; old
live workers without the contract must finish first. Reports are disposable and
never authorize a start, reward or replay. Generic blocking Ready/Flush reply
sends and controlled heartbeat/backpressure/cadence still need work/acceptance;
stale reporting detects a stall but does not remove it. Schema13 physical/WAL/
off-host recovery with restored rights/replay, current UI/physical Android, public
trust packaging/provisioning, actual multi-host/WAN and sustained RU capacity
remain open. C2/C4 decisions remain pending. Career13/portal11/protocol16 unchanged;
original68 stays62 done,5 partial,1 open. No RU migration, grant/settings/credential
change, live alert send, season reset or deployment is performed.
