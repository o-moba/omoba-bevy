# Durable ready ownership and refusal-correlation checkpoint

Schema12 adds separate ready checks, immutable seat/session receipts and unique
profile claims. Reserve is unpublished; Publish records opened_at_ms and exactly
15000ms to the deadline. Repeated requests preserve deadline/event identities.
Only the owning store identity may publish, answer, heartbeat or cancel. All
profile locks are sorted before check rows; no transaction spans the consent
window and no database-wide queue mutex is introduced.

Any new-search host can resolve an expired claim and atomically commit the missed
seat's stable queue event, penalty and claim release. A server heartbeat surviving
to the deadline is required to blame a missed answer; forming candidates, lost
owners and failed approved handoffs release without a fabricated dodge. Explicit
maintenance cancellation differs from a player decline. Ordinary account access
stays available. New allocation consumes a known approved check only under
profile locks; existing immutable admission/recovery is preserved.

[All81 store cases](store-all.log) PASS,0 failures/ignores, actual PostgreSQL18.6.
[Nine new ready cases](store-targeted.log) cover immutable/idempotent ownership,
overlapping concurrent hosts, exact15000ms/unanimity and allocation rollback,
decline/deduplication, another host resolving a missed answer with no outbox gap,
failed/unpublished owner without penalty, maintenance/old replies, concurrent
last accept/decline and schema11→12 validation. Timeout states are positioned by
SQL only in disposable test schemas; exact window proof uses recorded database
timestamps, without a flaky wall-clock scheduling assertion or production clock
flag. [Current source hashes and scope](summary.json).

The restricted-role case executes the ready lifecycle using the narrow source
DML grants and confirms the portal cannot read these tables. Temporary NOLOGIN
roles and per-test schemas are removed. [Actual cleanup/source report](source-and-cleanup.json).
The owned reusable fixture is now career12/portal11. While schema12 was still
uncommitted, its local fixture received the opened_at_ms column as the contract
matured; [exact local-only adjustment](owned-fixture-adjustment.json). No deployed
schema12, production role or credential was changed by this task.

[All64 native backend cases](backend-all.log) PASS,0 failures/ignores. The new real
worker case rejects a start with foreign unconfirmed consent and discards late
terminal replay after cancellation. The native refusal regression also verifies
current request ID, no queue dispatch, successful retry clearing and stale reply
rejection. Strict store/backend Clippy passes with existing workspace allowances;
formatting/diff checks pass. Actual shared/store/native backend sources compile
against coherent read-only cached dependencies. The wrapper supplies only the
verbatim engine-coupled transport capacity constant and allows dead code for
omitted callers. [Compiler inputs](store-compile.json), [backend inputs](backend-compile.json).
No engine or rendered-client certification follows from that wrapper.

Parent1e587a39 CI is rejected as a full gate: store72/server534 cases and six
other jobs passed, but the unchanged party-leave public journey failed. Its
checksum-verified artifact shows nine correctly retained survivors and a returning
dodger with a shared error but local Idle/null request ID. Keep the backend's typed
Failed/current request response when composing the lobby view; clear on retry,
cancel or success. [Exact rejected parent evidence](rejected-parent-ci.json).
This repair still needs the unchanged public journey to pass in fresh CI.

Activation is NOT complete: the current lobby has not yet wired reserve/publish/
answer/poll jobs and acknowledgements, or bound the worker result ID to the check.
Do that before exposing ready UI or launching a worker. Apply the committed SQL
penalty snapshot without also creating a second local event. Then prove two real
signed lobbies, party/FIFO behavior,15-second timeout, host loss and Resume.
Legacy penalty floors, schema12 real recovery, ready/journal retention/health,
release trust configuration and every physical/capacity/RU gate remain required.
C2/C4 host-tool proposals are still unapplied pending the existing owner questions.

Only tiny native test artifacts remain under the existing ignored .agent paths;
no Bevy build, cache deletion, third Cargo target or new dependency occurred.
Archived logs trim trailing blank lines only. Reproduction with enough disk uses
ordinary Cargo server/store tests against a disposable migrated PostgreSQL DB;
never use production credentials. The archived drivers are workspace/cache
specific, not portable release tooling.
