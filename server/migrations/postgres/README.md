# PostgreSQL career adapter

`career_store.rs` uses these PostgreSQL migrations with SQLx dynamic queries. It
does not use the older SQLite prototype. `connect` applies career schema versions
1–3 transactionally under a migration advisory lock, upgrades supported earlier
versions, and rejects unknown versions. Since 0.20.0-rc.1, use `migrate-career` (or Account API `migrate`)
explicitly as the migration owner. The game worker uses `connect_runtime` and
requires no DDL permissions; startup rejects missing/unknown versions. See
[portal roles and grants](../../../account-api/README.md). Use a dedicated
database/schema; no extension is needed. Version 2 requires PostgreSQL built with
ICU and the `"und-x-icu"` collation for Unicode case-insensitive player handles.
CareerStore runtime SQLx pools are bounded to one connection when
`OMOBA_SERVER_ROLE=match`, and four connections for lobby/standalone game roles.
They configure statement and lock timeouts plus `synchronous_commit = on`.
The optional Account API pool and administrative connections need their own budget. Infrastructure backups,
replication, credentials, TLS policy and disaster recovery remain deployment work.

## Accounts, profiles and friends

The trusted worker must verify device signatures before calling `authenticate`
or account operations. PostgreSQL maps each public key to a separately generated,
immutable 64-character random profile ID. Repeated/concurrent authentication keeps
the existing nickname; only explicit authorized `rename` changes it. This mapping
supports the device-key table introduced by schema version 3. Browser-authorized
device enrollment/recovery is provided by the separate Account API; the persistence
adapter does not itself approve new devices or export private device keys.

Friends use one canonical unordered pair of profile IDs. A reverse pending
request never accepts itself; only the recipient may accept or reject. Requests,
acceptances and removals are idempotent for the same relationship state. The
worker must reject replays of old signed action sequences so an old removal
cannot act on a later recreated friendship. Self requests are forbidden, profiles
must exist, and each account has at most 64 friends plus pending requests.
Mutation transactions lock both profiles in sorted ID order. Authenticated exact `nickname#1234` lookup returns only the immutable ID and
current name, allowing invitations even when partial discovery is disabled.
Friend operations use that resolved ID. See [player handles](../../../docs/player-handles.md). Full profile lookup permits self, accepted friends,
and incoming/outgoing friend-request contacts, matching the profile data already
included in those lists. Removing, rejecting or canceling the relationship revokes
that access. Match detail still requires actual participant membership.

Presence lives in PostgreSQL and expires after 30 seconds without refresh.
`touch_presences` requires that each supplied result ID matches that profile's
live allocation owned by this worker. Friends are Playing only while a presence refresh and live active
assignment both exist; an expired presence is Offline. A restart does not fabricate
Online status from stored friendships. The worker uses `touch_presences` for
up to 512 refreshes in one atomic SQL statement. Duplicate account refreshes are
coalesced; a missing profile or foreign allocation rejects the whole batch.

## Allocation and result lifecycle

`CareerBackend::new_result_id` allocates a globally random identifier. Numeric server epoch and
round ID remain full-range decimal `u64` metadata; they are intentionally not
globally unique, since independent hosts can have identical clock/round values.
Retries must reuse the same result ID and immutable allocation identity.

Call `start` and wait for durable acknowledgment before Running. It freezes
profile/player/team/name/loadout identity, ruleset, map and start time, and reserves
each authenticated profile uniquely until settlement. Legacy non-public unrated
allocations allow append-only late participants through `checkpoint` or `stage`;
existing identities cannot change or disappear. Rated and `public-casual-v1`
allocations both have an exact frozen roster.
Unique seat numbers from 0 to 31 bound each roster without a concurrent COUNT race.

`checkpoint` saves current statistics without rewards. Older-duration retries
cannot overwrite newer checkpoints. `stage` durably freezes terminal intent in a
separate transaction; `settle` calls it before crediting profiles. In settlement,
the match row is locked, then affected profile rows are locked in sorted profile-ID
order. Every profile update and the immutable finalized result commit together.
Identical retries return the existing receipt; conflicting identities, outcomes
or statistics fail. Deadlock/serialization failures retry the entire transaction
up to three times; other errors retain pending work for the worker to retry.
`saved` is exposed only after successful commit or reading an already committed
receipt. Foreign keys, immutable-result triggers and participant guards provide
additional defenses; they do not replace worker authorization.

The worker owns human/release/configuration eligibility. The adapter additionally
requires the `verdant-default-v1` ruleset, equal authenticated teams and no unrated
reason for rated play. Before a new rated allocation commits, its locked current
profiles must share newcomer status and have a full-roster rating spread at most
300. A stale queue selection is rejected without reserving seats; the worker must
refresh profiles before a new attempt. This final check does not recompute team
assignment from changed ratings. Elo uses the common K=32 policy with per-player 0–5000 clamps.
Rated completed matches grant 150 XP for a win or 100 for a loss. Approved public
casual completions grant 50/25 win/loss XP without any Elo/rated-match change.
The adapter requires `public-casual-v1`, reason `allocated_bots`, default map metadata,
ten balanced seats, at least one human and one bot, authenticated human profiles,
and no profile identities on bots. Runtime additionally verifies actual map and
gameplay modifiers before selecting this policy. Other unrated modes grant no XP.
Every persisted authenticated participation increments matches played;
completed outcomes count a win/loss even when unrated. Interrupted/abandoned
outcomes grant no rating, win/loss or XP. Guests have no profile credit. Match-time
names and loadouts remain unchanged after later profile renames.

## Ownership, recovery and pagination

Each connected worker has a unique process owner token. Call `heartbeat` every
10 seconds to renew its 90-second leases. Heartbeats never revive expired leases
or renew another owner. `adopt_expired` explicitly takes only expired unfinished
allocations and increments generation; a still-live different owner is rejected.
Old owners then fail checkpoint and settlement ownership checks.

On worker restart, replay its durable local outbox before sweeping expired games.
If an old lease is still live, wait for expiry before adopting its unfinished
allocation. `ensure_allocation` reconciles either preserved original Start metadata
or a compatible checkpoint roster (append-only only for legacy non-public unrated
games), creates a missing allocation, or
adopts an expired one under lock. It never compares checkpoint statistics or
rewrites a settled result; subsequent `stage` still checks immutable terminal
intent. Preserve original Start metadata when coalescing the local spool so a
terminal receipt can recover even if the initial allocation write failed.
`recover_expired` processes at most 32 expired allocations per call,
rechecking ownership under row lock. Public workers call the epoch-filtered
`recover_expired_for_epoch` using their original worker epoch, including after a
recovery-only process restart. The public lobby never runs the global sweep;
standalone servers retain the legacy global recovery method. Persisted terminal intents are settled as
recorded; live checkpoints become Interrupted with database-derived end time and
no ranked rewards. It never interrupts a live remote owner. If a sweep settles an
allocation Interrupted before an unstaged local victory is replayed, that immutable
result cannot be rewritten. The worker's durable spool/order is therefore part of
the recovery contract. Statistics checkpoints do not resume full gameplay.

History uses participant membership and descending immutable BIGINT sequence with
an exclusive `before` cursor, fetching 11 rows to return pages of 10. Sequence
allocation is not completion/commit order. Refresh the first page to discover new
results or an older allocation finalized after pagination passed its sequence;
this is not a frozen multi-page snapshot.

Real integration tests are in `career_store_tests.rs`, ignored by default. Set
`OMOBA_TEST_DATABASE_URL` to a dedicated PostgreSQL test database and run
`cargo test -p server career_store -- --ignored`. Each test creates its own schema
and removes that schema after success. No SQLite or mock adapter substitutes for
these checks; database availability, actual run results and deployment durability
must be reported separately.
