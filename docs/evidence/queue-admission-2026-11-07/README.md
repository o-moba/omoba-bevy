# Shared queue access and durable admission

Fresh new-search authorization now reads the shared queue cooldown on both
serial/parked and batched worker paths. Ordinary authenticated access remains
separate so a queue penalty does not revoke account access or existing Resume.
The durable new-allocation transaction checks queue discipline after participant
profile locks, shared with event recording. A failed check rolls back matches,
participants, active assignments and ranked reservations. Existing immutable
admissions/recovery stay usable. Blocked Start receipts are permanently rejected
through the existing durable tombstone path; queued/late terminal jobs cannot
resurrect that attempt after penalty expiry.

Actual PostgreSQL18.6: [all72 store tests](store-all.log), including [9 queue
cases](store-targeted.log), all PASS with0 ignores/failures. New cases verify
other-owner fresh access/expiry/revocation/missing-schema failure, transactional
rollback and existing allocation recovery, and an actual in-flight recording
race. The race pauses the real record method only via a disposable schema's
trigger/advisory lock, observes each exact backend PID waiting on its production
statement, then confirms start waits for the profile lock and refuses after the
other owner's commit. No production race/time hook is introduced.

[All63 native backend tests](backend-all.log) PASS,0 ignores/failures. Two new
actual PG cases exercise serial/batched access plus normal refresh/expiry, and
permanent Start refusal with terminal replay after expiry. One native state
machine case proves no queue dispatch on the shared denial. [Targeted result](backend-targeted.log).
Store/backend strict Clippy and formatting pass; existing workspace allowances
are retained. [Source hashes and limits](summary.json), [cleanup](store-source-and-cleanup.json).

The first full store attempt71/1/0 is rejected: the restricted-role test fixture
lacked the already-existing source queue grants. Copy the exact two source DML
grants into that disposable fixture, remove only its verified schema/two NOLOGIN
roles, and rerun72 PASS. [Exact failed-fixture cleanup](rejected-fixture-cleanup.json).
No production role or credential was modified. Archived logs trim trailing blank
lines only; raw logs and tiny native artifacts remain under the existing ignored
.agent/codex-ranked-readiness/queue-discipline and queue-delivery directories.

These checks compile actual current shared/store/native backend sources against
coherent read-only cached third-party libraries. The backend harness copies only
the engine-coupled transport capacity constant and allows dead code for omitted
callers. [Store inputs](store-compile.json), [backend inputs](backend-compile.json).
Reproducers are retained in those two .agent directories and archived in adjacent
queue-discipline-store/queue-dodge-delivery evidence. No local Bevy build or new
Cargo cache was used; the system volume remains below20GiB free.

Engine/runtime and client changes require fresh standard CI. The Russian client
uses the existing dodge translation for both raw stable code and error_code; its
regression is added but not locally executed. No current pixels are claimed.
No device/language matrix, current RU/WAN or physical proof is implied.

This is not full multi-host acceptance. Shared ready ownership/deadlines must
fence the interval before an old host's event reaches SQL. Legacy local penalty
floor reconciliation, actual signed two-lobby/party/timeout/Resume proof, schema11
recovery/receipt replay policy and typed-journal monitoring remain required.
C2/C4 host-tool proposals remain unapplied pending existing owner questions.
