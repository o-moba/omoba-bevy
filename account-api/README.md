# Omoba Account API

Rust 0.20.0-rc.5 HTTP adapter for the existing game career store. Axum 0.8.9,
SQLx PostgreSQL, ring 0.17.14. No HTTP endpoint can settle matches or change rewards.
The adapter reuses the game friendship transaction and nickname validation rules.
[OpenAPI 3.1](docs/openapi.json) describes the wire contract. Game core schema is
version 3 (revocable device keys); portal schema has its own version 3 and never runs Prisma/Drizzle push.

## Database and migration

Create a database and separate migration-owner, game-runtime and portal-runtime
roles using your normal administration process. Credentials belong in secret storage,
not source control. Run **once as the migration owner**:

```sh
OMOBA_DATABASE_URL="$MIGRATION_DATABASE_URL" cargo run -p omoba-account-api --locked -- migrate
psql "$MIGRATION_DATABASE_URL" -v portal_role=omoba_portal -v game_role=omoba_game -f account-api/ops/grants.sql
```

The migration command initializes the game’s existing v1 migration when needed,
then applies portal v1–v3 under an advisory lock. Repeated execution is safe. Existing
unknown versions fail closed. Runtime startup verifies versions and performs no DDL.
For game-only initialization, `cargo run -p server --bin migrate-career --locked` uses
`OMOBA_DATABASE_URL`. This replaces implicit DDL at game-worker startup.

`ops/grants.sql` assumes roles already exist. Inherited owner/superuser memberships
would defeat least privilege; use unprivileged runtime roles without memberships.
No role has ownership or CREATE on public/portal. On databases predating PostgreSQL
15, remove public CREATE permission as part of administration. The portal can update
only the nickname column in career_profiles, read core career tables, mutate friend
relations, and maintain portal tables. It can enroll a proved new device key into an explicitly approved existing profile, and revoke keys. It cannot create game profiles or alter XP,
rating, match outcomes, allocation identities or core migration versions.

## Runtime

```sh
cargo build -p omoba-account-api -p server --bins --locked
export OMOBA_DATABASE_URL="$PORTAL_RUNTIME_DATABASE_URL"
export OMOBA_PORTAL_ORIGIN=https://YOUR-PLAYER-HOST
export OMOBA_PORTAL_SECRET="$PRIVATE_RANDOM_32_BYTE_HEX_SECRET"
target/debug/omoba-account-api
```

`OMOBA_PORTAL_SECRET` must be an independently generated 32-byte random value,
represented as 64 lowercase hex characters; retain it across restarts. It keys
credential hashes and the 60-second authenticated session-delivery envelope.
Changing it intentionally invalidates outstanding pairings and browser tokens.
Do not log it or expose it in `NEXT_PUBLIC_` variables.

Default `OMOBA_ACCOUNT_BIND=127.0.0.1:40550`; non-loopback binds fail. TLS is terminated
by your existing trusted ingress. Native clients need ingress routes for POST
`/v1/auth/pairings/lookup`, `/approve`, `/deny`,
`/v1/auth/devices/create`, `/status`, `/recover`, `/complete`, and
`/v1/supporter/native`. Apple notifications use only the separately configured
`/v1/supporter/apple/notifications` route. Next’s BFF uses the internal API.
Do not broadly expose all API routes as an alternative browser backend.
`OMOBA_ALLOW_INSECURE_LOCAL=1` permits the exact localhost/127.0.0.1 portal on port
3010 for isolated development; never enable it for a remote HTTP site.

For a private loopback ingress that overwrites `X-Real-IP`, both services can set
`OMOBA_TRUST_LOOPBACK_PROXY=1`. Without this flag IP headers are ignored and limits
apply to socket peers. With it a valid header is required. Never enable it on an
unprotected listener or pass an inbound header through unchanged. Rate limits live
in PostgreSQL and are shared by API replicas. Apply edge per-IP/body/time limits too.

Health: `/health/live`, `/health/ready`. Logs report sanitized operational codes;
each request has an `X-Request-ID` and JSON log with status, duration and pool counts.
Paths, queries, request bodies, proof signatures, cookies, tokens and DSNs are never
logged. Aggregate these events in the deployment monitoring system.
Only original-browser completion can issue a session; unknown game keys never
create a career profile. Sessions have a seven-day absolute and one-day idle expiry.
At most twenty active sessions are allowed per account; further completion returns
`session_limit` until an existing session is revoked or expires. The account lock
serializes this limit across simultaneous approvals.
Revoking other sessions requires a new game confirmation within ten minutes.

## Projection and retention

The background worker scans settled results lacking a projection marker every five
seconds (128 results/batch). Facts and marker commit together. No sequence watermark
can miss an older allocation that settles late. Raw immutable match JSON remains the
source of truth. Replaying unprojected results is idempotent; a future projection
version needs an explicit migration/backfill procedure, not deletion of game data.
A single PostgreSQL advisory lock elects a writer across API replicas. Analytics
report pending work, data freshness and a 1000-point rating-series cap. A transaction
advisory lock limits analytics to one concurrent request per account across replicas;
competing requests receive 429 with Retry-After.

Pair codes live five minutes. Encrypted completion delivery lives sixty seconds.
Expired pairings/rate buckets/receipts/audit entries are cleaned in bounded batches;
operation receipts live eight days, audit entries thirty days. Core match receipts
are retained. Expired sessions are removed after a one-day grace period once no
operation receipt refers to them. No replay recording or player-presence fabrication is introduced.

## Downloads

Optional `OMOBA_RELEASE_CATALOG` points to a curated JSON file:
`{"releases":[]}` is a valid honest empty catalog. Entries follow the OpenAPI Release
schema: platform, architecture, version, HTTPS URL, SHA-256, decimal size_bytes,
RFC3339 published_at, installation instructions, test_status. Populate only after
independently fetching/verifying the exact public artifact. Never list an unsigned
or privately provisioned IPA as publicly installable. No upload or shell launcher
exists in the browser. Missing configuration shows no downloads.

## Backup, restore, rollback

Back up the full database (core + portal) with `pg_dump -Fc`; also retain configuration
and the portal secret securely, separately from the dump. Restore to a **new isolated
database**, apply grants for existing runtime roles, verify both schema versions,
row counts and immutable receipt checksums, then start the services against it.
Never restore over the running production database as a test.

Rollback of the application does not require destructive SQL. Stop the portal and
use a game binary compatible with career v3. Pre-v3 binaries fail schema checks;
there is no automatic downgrade of player handles. Keep portal schema and
immutable game data; rotate/revoke portal sessions deliberately if retiring access.
Do not delete tables or lower migration versions. Public cutover, secrets, ingress,
backups and deployment remain operator-approved infrastructure actions.

## Player handles

Profiles expose an editable `nickname#1234` in the existing `nickname` field.
Core migration 002 requires PostgreSQL ICU collation `"und-x-icu"`. Run owner
migrations before starting matching client/server/API releases. Exact authenticated
`GET /v1/players?query=Name%231234` returns only ID and name, including private
accounts; prefix discovery still requires opt-in. Friend mutations bind the resolved
immutable ID. See [handle rules and migration](../docs/player-handles.md).

## Shared devices and Supporter

See [Supporter operations](../docs/supporter.md) for enrollment/recovery, billing
provider configuration, test boundaries and schema/runtime grants. Web sessions
now explicitly authorize device/recovery management; native confirmations display
these capabilities. Old limited sessions are invalidated by the account upgrade.
