# Persistent careers and basic friends — 2026-09-14

Version: `0.19.0-rc.6`. Replays were explicitly deferred by the user.

The server now records lifetime accepted damage and combat totals, freezes the
first final outcome and persists immutable match receipts and profile progress in
PostgreSQL. A separate device signing key authenticates each installation. The
UI exposes profile/history/results and incoming/outgoing/accepted friendships on
both desktop and mobile layouts. Friend names support the bundled Inter and
licensed Noto CJK font coverage.

The release queue uses saved MMR and newcomer status, freezes equal-team rosters,
waits for a durable account allocation and requires explicit play-again. Database
allocation rechecks current skill/experience so another server cannot admit a
stale newcomer selection. Ineligible development/custom/guest rounds are unranked.

Persistence runs off the tick through a bounded worker and SQLx pool. The local
outbox preserves Start metadata and terminal receipts; PostgreSQL enforces atomic
idempotent settlement, account reservations and owner fencing. Replies never block
the worker's async heartbeat; critical acknowledgements retain their durable
receipt under backpressure. Rejected allocations carry durable tombstones, and
saved ACKs carry refreshed ratings before the next queue entry.

Career transport uses the existing small datagram framing with a separate
assembly namespace. It preserves complete result/history/friend payloads and
alternates oversized combinations rather than dropping participants. Friends
are bounded to 64 total accepted/pending relationships per account.

## Verification record

Task evidence is under `.agent/tasks/MATCH-PROGRESSION-2026-09-14/`. The final
`evidence.md` and `evidence.json` link exact commands, outputs and capture paths.
Checks include real PostgreSQL transactions/recovery, real signed UDP admission
and combat victory, desktop/phone native renderer screenshots, and the workspace
regression suite. Screenshots are explicitly labeled synthetic presentation
fixtures; real persistence is proved separately with database/UDP tests.

See [operator setup and remaining release limits](../match-progression.md).
Production deployment, global load validation, physical mobile device acceptance,
party matchmaking/chat and account recovery/linking are separate work. The new
code does not implement or record replays.
