# Omoba Account API

Rust 0.21.0-rc.1 HTTP adapter for the existing game career store. Axum 0.8.9,
SQLx PostgreSQL, ring 0.17.14. No HTTP endpoint can settle matches or change rewards.
The adapter reuses the game friendship transaction and nickname validation rules.
[OpenAPI 3.1](docs/openapi.json) describes the wire contract. Game core schema is
version 5 (3 = revocable device keys, 4 = admission limits and profile retention, LR-16,
5 = sanctions, moderation audit and chat switch, LR-15);
portal schema has its own versions 1–7 and 9 (8 is reserved for LR-20 feedback) and never
runs Prisma/Drizzle push.

## Database and migration

Create a database and separate migration-owner, game-runtime and portal-runtime
roles using your normal administration process. Credentials belong in secret storage,
not source control. Run **once as the migration owner**:

```sh
OMOBA_DATABASE_URL="$MIGRATION_DATABASE_URL" cargo run -p omoba-account-api --locked -- migrate
psql "$MIGRATION_DATABASE_URL" -v portal_role=omoba_portal -v game_role=omoba_game -f account-api/ops/grants.sql
```

The migration command initializes the game’s existing v1 migration when needed,
then applies portal v1–v7 and v9 under an advisory lock. Repeated execution is safe. Existing
unknown versions fail closed. Runtime startup verifies versions and performs no DDL.
For game-only initialization, `cargo run -p server --bin migrate-career --locked` uses
`OMOBA_DATABASE_URL`. This replaces implicit DDL at game-worker startup.

`ops/grants.sql` assumes roles already exist. Inherited owner/superuser memberships
would defeat least privilege; use unprivileged runtime roles without memberships.
No role has ownership or CREATE on public/portal. On databases predating PostgreSQL
15, remove public CREATE permission as part of administration. The portal can update
only the nickname column in career_profiles, read core career tables, mutate friend
relations, and maintain portal tables. It can enroll a proved new device key into an explicitly approved existing profile, and revoke keys. It cannot create game profiles or alter XP,
rating, match outcomes, allocation identities or core migration versions. The game role
counts new profiles in `career_profile_admissions` and may run only the owner-defined
`career_prune_unplayed_profiles` to delete profiles that never played (LR-16); it still
cannot delete a device key itself. The game role reads sanctions and the chat switch; the
portal role may also apply and lift sanctions (only the lift columns change), append to
`career_moderation_events` and switch chat, so the operator commands below run with it.
No runtime role may delete a sanction or an audit row (LR-15).

## Moderation commands (LR-15)

The same binary is the operator's moderation tool. With `OMOBA_DATABASE_URL` set (the
portal role is enough):

```sh
omoba-account-api player <nickname#1234 | profile-id>
omoba-account-api sanction add <player> <kind> (--for <duration> | --permanent) --reason <text> --by <operator>
omoba-account-api sanction list <player> [--all]
omoba-account-api sanction lift <sanction-id> --reason <text> --by <operator>
omoba-account-api rename <player> (--for <duration> | --permanent) --reason <text> --by <operator>
omoba-account-api chat [status | on | off --reason <text> --by <operator>]
```

Kinds: `chat_mute`, `matchmaking_ban`, `ranked_ban`, `account_ban`, `name_reset`
(`rename` applies it: the name becomes `Player#NNNN`). Durations: `30m`, `12h`, `7d`,
`2w`. Each change writes a `career_moderation_events` row in its transaction; game
servers apply it within about 10 seconds without a restart. While an account ban is
active, every session-authenticated route, pairing approval and completion and VK
sign-in answer `403 account_banned` (sessions are not revoked, so they work again once
the ban ends); `PATCH /v1/me/profile` answers `403 name_locked` during a name reset.
Procedure, effects and the RU host invocation: [RUNBOOK.md › Moderation](../RUNBOOK.md#moderation-lr-15).

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
`/v1/auth/devices/create`, `/status`, `/recover`, `/complete`, `/recovery-codes`,
`/v1/supporter/native` and, with VK ID enabled, `/v1/auth/social/link`. Apple notifications use only the separately configured
`/v1/supporter/apple/notifications` route. In a YooKassa market (RU) the ingress
also passes YooKassa's webhook, `POST /v1/supporter/yookassa/notifications`, as its
own public route. Next’s BFF uses the internal API.
Do not broadly expose all API routes as an alternative browser backend.
`OMOBA_ALLOW_INSECURE_LOCAL=1` permits the exact localhost/127.0.0.1 portal on port
3010 for isolated development; never enable it for a remote HTTP site.

For a private loopback ingress that overwrites `X-Real-IP`, both services can set
`OMOBA_TRUST_LOOPBACK_PROXY=1`. On one host the portal then calls this API directly
on loopback and forwards the `X-Real-IP` its own ingress wrote
([ops/vps/ru/README.md › Topology](../ops/vps/ru/README.md#topology)). Without this flag IP headers are ignored and limits
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

In-game recovery codes (LR-18): `POST /v1/auth/devices/recovery-codes`, signed by
the game's active device key (`omoba.recovery.native.v1`, at most 60 s, single-use
nonce), reads the account's protection (`status`: unused codes, linked sign-ins,
active devices) or issues a new set of codes (`rotate`, the same table and the
same per-profile limit as `POST /v1/me/recovery-codes`). It needs no portal
session and no schema change. Tests: `tests/recovery_native_postgres.rs`.

## Sign in with VK ID (portal schema 7)

Markets with the `vk_sign_in` feature (`docs/markets.md`; the RU build) sign
players in with VK ID: VK, OK and Mail.ru are one VK ID web app (OAuth 2.1 with
PKCE, public client). Set `OMOBA_VKID_CLIENT_ID` (the numeric app id) and
`OMOBA_SOCIAL_SECRET` (64 lowercase hex, generated once and never rotated: it keys
every stored identity). Neither set disables the routes (404). A build without
`vk_sign_in` refuses to start with them set. `OMOBA_VKID_LOCAL_FIXTURE_ORIGIN`
(loopback http only) points the exchange at a local stand-in for tests.

The portal BFF forwards the VK callback to `POST /v1/auth/social/vk`; this
service exchanges the code at `id.vk.ru` (with the callback's `device_id` and
`state`), reads the user, and never stores its token, name or e-mail: an identity
is `HMAC(OMOBA_SOCIAL_SECRET, vk user id)`. A linked identity receives a normal
seven-day browser session (label "VK ID", "OK" or "Mail.ru", the same 20-session
cap). An unlinked one receives a 30-minute ticket. `POST /v1/auth/social/ticket`
describes a ticket; `POST /v1/auth/social/attach` offers it to the pending game
enrollment whose code the game opened the browser with; the enrollment's status
then reads `social_pending` with a short display name; the game links it with
`POST /v1/auth/social/link`, signed by its active key over the enrollment
(`shared::device_account::SocialLink`). A website sign-in never creates a game
profile; one VK account links to one profile; `DELETE /v1/me/social/vk` (fresh
session) unlinks. `vk`, `ticket` and `attach` are BFF routes: keep them off the
public ingress (behind the portal ingress key, or loopback only as on RU). Expired tickets and unlinked identities idle for 30 days
are cleaned up. Tests: `tests/social_postgres.rs` (loopback VK stand-in).

Free markets (`supporter` off) serve no `supporter` route and refuse Apple,
Solana or YooKassa settings; without `ekza` there is no `me/ekza-library` route.
A market with `supporter` on sells only through its own providers
(`shared::market::payment_providers`): a global build refuses YooKassa settings,
an RU build refuses Apple and Solana settings (until LR-69), both with
`payment_provider_not_in_market` at startup, and routes of an unlisted provider
answer 404 (`supporter::provider_route_allowed`).

## Hero tiers and YooKassa (portal schema 9, LR-67)

`009_yookassa_supporter.sql` adds provider `yookassa` to the supporter CHECKs,
`amount_minor` (kopecks, YooKassa only) and `tier_points` (default 0, existing rows
keep 0) on `portal.supporter_grants`, the seven tier aura ids, and two functions:
`portal.supporter_points(profile, at)` (the tier sum of the grants covering `at`)
and `portal.supporter_months(profile)` (credited months: YooKassa donations count
distinct UTC calendar months). `ops/grants.sql` gives both runtime roles EXECUTE on
`supporter_points` and the portal role on `supporter_months`.

Tier points are set only here, in `apply_event_tx`, from the provider rule
(YooKassa: whole rubles after refunds; Apple and Solana: 0 in M0). The dashboard
(`GET /v1/me/supporter`) adds `providers`, a `catalog` entry per style with
`tier_points` (0 = any active grant) and `unlocked`, and `supporter.tier_points` /
`supporter.tier_until`. `PATCH /v1/me/supporter/aura` refuses a tier style without
its points (403 `tier_required`).

YooKassa settings (all-or-none): `OMOBA_YOOKASSA_SHOP_ID` (digits),
`OMOBA_YOOKASSA_SECRET_KEY`, `OMOBA_YOOKASSA_VAT_CODE` (1–12; 1 = без НДС) and
`OMOBA_YOOKASSA_NEW_CHARGES` (exactly `1` allows new charges; reconciliation always
runs). Only a build whose market sells through YooKassa (RU) accepts them.
`OMOBA_YOOKASSA_TEST_API_ORIGIN` (a loopback `http://` origin) points the client at
a local stand-in for tests; a release build refuses to start with it. The secret
key and provider bodies (card and customer data) are never logged: a failed call
logs its operation, the HTTP status and YooKassa's `x-request-id` only. Amounts
are integer kopecks. The migration also creates the durable attempt, refund,
refund-scan cursor and issue tables; the portal role may read, insert and update
them but delete none.

### YooKassa provider flow (LR-67b)

`src/supporter/yookassa/` (`api.rs` the REST client, `checkout.rs`,
`notifications.rs`, `reconcile.rs`). The rules come from the owner's production
YooKassa integration; see [supporter.md](../docs/supporter.md#yookassa-donations)
for the full list.

- `POST /v1/me/supporter/yookassa/checkout` (session, `Idempotency-Key` header,
  16–80 of `[A-Za-z0-9-]`) with `{amount_rub, receipt_email, quote, offer_version}`
  answers `{confirmation_url, attempt_id}`: the HTTPS redirect to YooKassa's
  payment page (never one with credentials). `amount_rub` is a tier (100, 250,
  500, 1000, 2500, 5000, 10000) or any whole amount from 100 to 100 000.
  `quote` must equal the server's `{amount_minor, currency: "RUB",
  duration_days: 30, tier}` for that amount, else `409 reload`. The attempt, its
  Idempotence-Key and the frozen «Чеки от ЮKassa» receipt (the e-mail, one
  `service` item paid with `full_payment`, `vat_code` from the env, «OMOBA —
  поддержка проекта (аура на 30 дней)») are committed before the POST; the
  return URL is `<OMOBA_PORTAL_ORIGIN>/supporter?paid=1`. The same key repeats the
  same attempt (re-reading its payment, never creating another); the same key with
  another amount, e-mail or offer is `409 idempotency_conflict`. Other codes:
  `400 amount_not_allowed`, `400 receipt_email_invalid`, `409 payment_succeeded`,
  `409 payment_canceled`, `409 payment_in_progress`, `409 payment_manual_review`,
  `409 too_many_open_payments` (10 open attempts per day), `503 new_charges_paused`
  (the switch is off, or the refund scan has not succeeded for 30 minutes),
  `503 payment_provider_unavailable`, `502 payment_provider_rejected`.
- `GET /v1/me/supporter/yookassa/payments`: the owner's last 20 attempts of the
  year (`pending`, `succeeded`, `canceled` or `review`, with `refunded_minor`). It
  first re-reads the owner's newest submitted payment of the last hour (at most
  every 5 s), so the return page sees a payment before its webhook.
- `POST /v1/supporter/yookassa/notifications` (public; limits per IP 120/min and
  600/min in total): the body is only a hint. A payment or refund this API did
  not create answers `200 {"status":"ignored"}` without a call to YooKassa. A
  payment is known when it is bound to an attempt or its metadata names a
  posted, still unbound attempt (a lost create reply, also one in manual
  review); a refund only when its payment is bound (an unbound attempt never
  has a paid payment: the player gets the redirect after the bind). A known one
  is re-read with the shop credentials and settled exactly once
  (`200 {"status":"accepted"}`); a named object YooKassa answers 404 for is
  `200 {"status":"ignored"}` too; any other processing error answers 5xx so
  YooKassa delivers it again.
- The dashboard (`GET /v1/me/supporter`) adds `yookassa`: `available`,
  `new_charges`, `currency`, `duration_days`, `min_rub`, `max_rub`, `tiers_rub`.
- Reconciliation (`reconcile`, every 60 s next to the Apple loop): closes
  never-posted attempts after an hour; polls 8 open attempts per pass, oldest
  polled first; replays a lost create with the same key and body while the first
  provider attempt is younger than 23 hours, after that marks it
  `provider_replay_window_expired` with an issue for manual review and never posts
  it again; scans refunds in a fixed window with YooKassa's cursor stored in
  `supporter_yookassa_refund_scan` (2 pages of 32 per pass), queueing every refund
  it cannot settle as an issue before the cursor moves. Each eventful pass logs a
  `yookassa_reconciliation` JSON line with counts only.

Tests: `tests/yookassa_settlement_postgres.rs` (settlement with synthetic provider
objects, restricted portal role) and `tests/yookassa_provider_postgres.rs` (the
whole flow against a loopback YooKassa stand-in: the key persisted before the POST,
replays under and over 23 hours, webhooks for unknown objects without a call,
duplicate and late webhooks, refunds before payments, partial and full refunds,
the durable refund cursor, the restricted portal role, and the routes per market).

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
operation receipts live eight days, audit entries thirty days. Moderation audit rows
(`career_moderation_events`, LR-15) are a separate career table that this cleanup
never touches; they are kept at least a year. Core match receipts
are retained. Expired sessions are removed after a one-day grace period once no
operation receipt refers to them. No replay recording or player-presence fabrication is introduced.

## Downloads

Optional `OMOBA_RELEASE_CATALOG` points to a curated JSON file:
`{"releases":[]}` is a valid honest empty catalog. Entries follow the OpenAPI Release
schema: platform, architecture, version, HTTPS URL, SHA-256, decimal size_bytes,
RFC3339 published_at, installation instructions, test_status. Optional per entry
(LR-14, LR-68 rules): `channel` (`[a-z0-9_-]{1,32}`, `direct` when absent),
`min_release` (the oldest release of that channel and platform that may still play
online, never above `version`) and `update_url` (the HTTPS page the game's update button
opens, at most 256 bytes, a query allowed; `shared::compatibility::valid_update_url`).
Optional `region` and `mirror_url` (LR-23: the market, an IPFS copy of the same bytes)
are passed through unchanged; `scripts/release.py catalog` writes them and only HTTPS
links. Populate only after independently fetching/verifying the exact public
artifact (`release.py verify-downloads`). Never list an unsigned
or privately provisioned IPA as publicly installable. No upload or shell launcher
exists in the browser. Missing configuration shows no downloads.

The game reads the same `GET /v1/releases` at boot and after an update-required
mismatch (LR-14, `passport/src/release_catalog.rs`): the newest entry of its channel for
its platform and architecture (`universal` matches any) drives the **Update available**
banner, its `min_release` can require the update, and its `update_url` is the button
target. An empty catalog offers no update.

## Backup, restore, rollback

Back up the full database (core + portal) with `pg_dump -Fc`; also retain configuration
and the portal secret securely, separately from the dump. Restore to a **new isolated
database**, apply grants for existing runtime roles, verify both schema versions,
row counts and immutable receipt checksums, then start the services against it.
Never restore over the running production database as a test. On the RU host
`ops/vps/ru/ru_backup.py` does all of this on a schedule: nightly and pre-migration
sets off-host, a monthly restore drill and the secrets escrow
([ops/vps/ru/README.md](../ops/vps/ru/README.md#backups-escrow-and-restore-drill)).

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

## Private class workshop prototype

Portal migration 004 adds owner-scoped private class preferences; 005 adds the
separate encrypted Ekza library connection. Startup requires exactly the portal
schema versions this build serves (`PORTAL_SCHEMA`, 1–7 and 9) and runs no DDL, so run
`migrate` first. No new browser authorization scopes are
added: private named builds use the existing personal-settings consent.

`GET/POST /v1/me/class-builds` lists/creates up to 50 documents per profile.
`GET/PATCH/DELETE /v1/me/class-builds/{id}` reads, replaces or deletes an owned
64-hex ID. POST accepts `{document}`, PATCH accepts `{expected_version,document}`,
and DELETE requires `?version=N` and an empty body. Versions are decimal strings.
Mutations require `Idempotency-Key` and commit the owner-scoped change, replay
receipt and audit event in one transaction. Reusing a key with another method,
path or payload conflicts. Foreign and absent IDs both return 404.

Every saved `omoba.class-build.v1` document is validated by
`shared::workshop::ClassBuildDocument` and the game recipe resolver. This stores
private preferences only; it does not publish a class or admit it to ranked play.
The current 8 KiB body bound, session expiry/revocation and API rate limits apply.

For the isolated community demo, compile the API binary and `seed-local-portal`
example, then use `python3 account-api/ops/workshop_fixture.py init`. The helper
creates only its dedicated local PostgreSQL cluster on 127.0.0.1:55581, explicit
`workshop_test`/`workshop_demo` databases and unprivileged runtime roles. It never
uses an ambient database URL or changes a production service. Run `serve` to
start Account API on 127.0.0.1:40560 for portal origin 127.0.0.1:3010. The separate
labelled Registry fixture must listen on 40561. Synthetic game key seed 7 is
registered for the demo's real signed pairing flow.

```sh
OMOBA_PORTAL_TEST_DATABASE_URL=postgres://workshop_owner@127.0.0.1:55581/workshop_test \
OMOBA_PORTAL_ROLE_TEST_URL=postgres://workshop_portal@127.0.0.1:55581/workshop_test \
cargo test -p omoba-account-api --test class_builds_postgres --test ekza_library_postgres --locked -- --ignored --test-threads=1
```

The fixture helper does not remove clusters or databases. Stop the API with Ctrl-C
and stop the dedicated cluster explicitly with PostgreSQL's `pg_ctl` when the
demo no longer needs it. Keep generated fixture secrets and DB files untracked.
