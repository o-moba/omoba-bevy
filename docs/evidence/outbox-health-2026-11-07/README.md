# Durable receipt age and journal ownership

Identical result retries previously rewrote the receipt every delivery attempt.
The five-minute monitor used mtime, so repeated failure could keep an unsaved result
apparently fresh indefinitely. The actual writer now verifies identical regular
file bytes with a bounded read, fsyncs the file/directory and preserves mtime.
Changed checkpoints retain atomic replacement. The terminal monitor also anchors
age to original ended_at_ms, covering receipts whose mtime was already refreshed.
Missing/zero legacy end time falls back to mtime; invalid/future source time is
actionable. Active Start/Checkpoint records do not trigger stale-result alerts.

Typed queue-dodge journals are inspected separately from match results. The monitor
checks exact native v1 fields, filename identity, observation time, bounded regular
files, per-journal native capacity and global scan limits. Leaf links/FIFOs cannot
make a read escape or block. Old pending events cannot appear fresh merely because
the file was recreated. Reports/alerts contain counts, not profile or event IDs.
Existing alert repetition/resolution behavior is reused. No alert is sent by these
tests; the interoperability helper calls only two read-only scanner methods.

## Journal reopen regression

At031833d7, CI42330926635's core job failed reopening a retired journal with Busy.
The deterministic [before-fix case](journal-lock-before.log) retains a duplicated
descriptor across Journal drop and reproduces the same error without sleep/retry.
Rust documents that automatic lock release waits for duplicated/inherited handles
to close; Linux flock likewise associates the lock with the open file description.
[Rust File::unlock](https://doc.rust-lang.org/std/fs/struct.File.html#method.unlock),
[Linux flock](https://man7.org/linux/man-pages/man2/flock.2.html).
Concurrent process spawning is a plausible source of such transient aliases in
the parallel CI process; the original run did not trace descriptor inheritance.

A lock guard now explicitly unlocks when its owner retires, including failed
open/scan paths after acquisition. A failed competing acquisition never constructs
the guard. The [after-fix suite](journal-lock-after.log) checks reopening with an
old duplicate still live, continued exclusion of another writer, and continued
exclusion after that old duplicate closes. The guard never deletes the lock file,
breaks a live lease or retries through another writer's ownership.

## Current local verification

- [564 complete actual server cases](server-all.log) PASS,0 failures/ignores,
  including13 real PostgreSQL18.6 cases; serial policy matches the official PG
  test runner. Five new cases cover retry age, changed checkpoints, static leaf
  symlinks, actual Rust-to-Python receipts and duplicated-descriptor ownership.
- [34 monitor cases](monitor.log) and [74 host-tool cases](host-tools.log) PASS.
  Fake filesystem/systemd/clock/Telegram isolate the Python suite from live ops.
  Corruption, oversize, source age/future times, scan/capacity bounds, symlink/FIFO,
  active checkpoints and resolution are covered.
- Strict actual full-server test and production [Clippy](clippy.log), formatting
  and diff checks PASS. [Original native](age-before-native.log) and
  [monitor](age-before-monitor.log) age regressions fail before the fix.
- [Signed maintenance/restart/Resume journey](drain.json) PASS with the current
  production binary. Live play advances, draining does not allocate, consent must
  be fresh on reopening and lobby restart adopts workers/preserves Resume. This
  is loopback, one AI-controlled human seat with bots, not physical or load proof.

The first attempt to invoke the cached linker used the system Python lacking
tomllib; use the installed Homebrew Python3.13 for those local helper scripts.
An intermediate compile used an unavailable logging crate, corrected to the
existing server stderr convention. A prematurely launched run used the older
red-test binary and was stopped; it is not acceptance evidence. The archived
complete564-case run uses the successfully relinked final source.

Actual full server/common/passport/shared/store sources use the coherent pinned
read-only third-party cache, without copied engine modules or graphics builds.
Production common excludes test-support. [Source and binary hashes](summary.json)
bind this proof to the current code; compiler arguments match the archived
legacy-penalty-floors graph byte-for-byte. Root disk remains below20GB, so full
platform/graphics checks stay in CI. No new dependency, cache, schema or protocol.

## CI and remaining release scope

Parent6f6a9544's [CI42327346048](https://github.com/o-moba/omoba-bevy/actions/runs/42327346048)
is fully green in7 jobs. The downloaded PostgreSQL artifact13557149866 matches
the expected SHA256/size and passes ZIP path/size/CRC checks; [all20 stages](parent-ci-stages.json)
PASS, including full Ranked victory, [two-lobby ready discipline](parent-ci-cross-lobby-ready.json)
and [legacy floor/restart](parent-ci-legacy-penalty.json).
[Artifact inspection](parent-ci-inspection.json). The report confirms fixture
removal and contains no private-key path. These are historical parent results.
The031833d7 core failure is not accepted as green; fresh full CI must validate
the ownership fix and current source, including client/platform jobs.

This packet covers persisted-receipt monitoring, not health for an unavailable
ready/floor SQL path or a failure before any receipt is durable. Bounded safe
retention is still to implement; no receipts are deleted here. Current-schema
physical/WAL/off-host recovery, controlled heartbeat/load cadence, public trust
packaging/provisioning, actual multi-host/WAN, UI/physical Android and RU host
acceptance remain open. Career12/portal11/protocol16 are unchanged. The unchanged
store's previous92-case PG proof remains separate. C4's review patch still applies
cleanly; C2/C4 decisions remain pending. No unit/config/credential/live alert target,
RU migration, season reset or deployment is changed. Original68 remains62 done,
5 partial,1 open, with additional critic/UI/physical/release gates.
