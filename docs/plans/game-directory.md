# Game directory and server routing

Status: **P7 in progress** (2026-11-06). D1's bounded contract, routing policy
and offline signer/verifier plus opt-in Account API/portal HTTP delivery are
implemented and locally verified. No directory
is published or used by the running client yet. Fetch/cache/fallback, trusted
key provisioning, operator publication, cross-host penalties/party/Resume
behavior and actual second-host/capacity proof remain required. D0/D1 are now
scheduled in P7 before multi-host launch acceptance; they are not deferred away
from the owner's complete-finding scope.

## Idea

Production clients should not decide which game server they talk to. They know one
fixed entry point. A backend behind it routes players to the right lobby and match
servers. Operators then move, add, drain or upgrade servers without shipping a new
client.

## Where it meets the current game

| Area | Today | Why it matters |
| --- | --- | --- |
| Client default | Account-region startup uses RU `lobby.omoba.ru:4000`; Global still uses `77.246.105.57:4000` ([region.rs](../../shared/src/region.rs), [session_config.rs](../../client/src/session_config.rs)) | Global D0/DNS provisioning remains open; signed routing must preserve account-region isolation |
| Lobby | One coordinator, one region; it redirects to match workers on `OMOBA_MATCH_PUBLIC_HOST` and ports from `OMOBA_MATCH_FIRST_PORT` with `OMOBA_MATCH_CAPACITY` ([match_pool.rs](../../server/src/match_pool.rs), [public MVP](../public-mvp.md)) | The lobby already hands out match servers, but only on its own host |
| Compatibility | Every transport runs a JSON preflight: handshake, release, protocol, catalog, geometry ([release compatibility](../release-compatibility.md), [compatibility.rs](../../shared/src/compatibility.rs)) | Routing can pick a lobby whose contract matches the client instead of failing after connect |
| Web layer | Next/HTTPS front and regional Account API | It can serve the signed document; UDP still goes directly to game hosts, not through the web front |
| Contours | Dev builds bake the dev lobby and registry; prod builds will hide the server picker (connection profiles work) | A directory URL per contour can replace hard-coded addresses in both |

## Phases

### D0 — Names instead of IPs (S)

Bake a hostname (for example `play.omoba.io:4000`) instead of the IP. Moving the
lobby then becomes a DNS change. Check that every client path resolves hostnames,
including mobile and party/reconnect. The dev lobby already uses `lobby.dev.omoba.io`.

### D1 — Directory document (S–M)

Clients fetch one HTTPS document at startup:
`https://app.omoba.io/api/game/directory` (dev: `app.dev.omoba.io/...`).

The document carries:
- a schema version, an issue time and an expiry;
- a list of lobbies: `id`, locality, `host`, signed concrete IP `addresses`, `port`, the compatibility contract
  (protocol, catalog, geometry), `status` (open, maintenance, full) and a `weight`;
- a client policy: minimum release, update URL and a message of the day.

The client:
1. picks a lobby whose contract matches its own, then by region or measured
   latency (the existing preflight can time it);
2. keeps the last good document and falls back to a baked hostname when the
   directory is unreachable;
3. never mixes contours: a prod client only accepts the prod directory.

Sign the document with an ed25519 key whose public half is baked into the client,
so a hosting compromise cannot replace its routing policy. Clients must use the
signed concrete IP targets, not a fresh DNS answer for the label; baked fallback
also needs pinned addresses to meet the DNS-compromise requirement. These client
and fallback rules are still pending. The current unauthenticated UDP preflight
is not server identity proof. Operators publish the reviewed signed artifact with
a CLI from a reviewed file. Production builds then hard-code only the directory URL
and the fallback name.

### D2 — Rolling upgrades and maintenance (M)

- Route by contract: old clients keep reaching the old lobby until it drains,
  while new clients reach the new one. Upgrades no longer need the
  "deploy the server first" choreography.
- Maintenance mode and "update required" screens come from the document, not from
  a failed handshake.
- Canary: send a share of compatible clients to a new lobby build.

### D3 — Matchmaker and match tickets (L)

- A stateful matchmaker service. It is not serverless; it runs next to the lobbies,
  for example with the Account API on the game VPS. app.omoba.io stays its public
  HTTPS front.
- Flow: the client asks for a match over HTTPS, the matchmaker queues it (across
  regions if needed) and returns a short-lived signed ticket `{worker host:port,
  match id, seat, expiry}`. The worker admits only valid tickets.
- Parties, reconnect to the same worker, bot fill (see the bot personas plan) and
  rating bands move from the single lobby into the matchmaker.

### D4 — Fleet (L–XL)

Several game hosts and regions report health and capacity to the matchmaker.
Optionally autoscale by starting and stopping VPS workers. Add observability:
queue times, fill rate, match start latency.

## Open questions

1. Delivery implementation: the fixed regional portal route proxies a signed file
   served by the regional Account API. Reviewed live publication/key provisioning
   remains pending; no live endpoint has been activated.
2. Regions worth having for the first players. Beijing and Moscow latency differ a lot.
3. Whether the matchmaker in D3 belongs to the Account API or is a separate service.
4. What happens to LAN and self-hosted servers in production builds. Today the plan
   is a desktop-only environment override.

## TODO when scheduled

- [ ] **D0** — hostname default, a resolver test on every platform, DNS record, release note.
- [ ] **D1** — directory schema and signer CLI, client fetch/cache/fallback/selection,
      contour separation, tests (unreachable, expired, bad signature, no compatible
      lobby), a dev directory on the dev contour first.
- [ ] **D2** — contract routing, maintenance and update screens, canary weights,
      upgrade runbook.
- [ ] **D3** — matchmaker service and tickets, worker ticket check, party and
      reconnect, migration away from the direct lobby queue.
- [ ] **D4** — multi-host health, capacity, autoscaling, metrics.


## Implemented D1 contract/operator checkpoint

Sources: [contract/policy](../../shared/src/game_directory.rs),
[contract tests](../../shared/src/game_directory_tests.rs),
[offline operator](../../server/examples/game-directory.rs),
[process/interoperability proof](../../scripts/test_game_directory.mjs).
No new dependency, live key, DNS record, workflow or production service is added.

The signed envelope carries `key_id`, exact UTF-8 JSON `payload` and a lowercase
hex Ed25519 signature. Signing bytes are
`b"omoba.game-directory.v1\0" || u8(key_id_bytes.len) || key_id_bytes || payload_bytes`.
The pinned public key must come from the client/operator trust configuration,
never from the document. Envelope/payload limits are64/48KiB, at most32 lobbies
and8 concrete addresses per lobby. Version1 is separate from UDP protocol16.

Payload fields: `version`, `region` (`ru`/`global`), exact `contour` (`production`
or the build contour), positive `sequence`, `issued_at_ms`, `expires_at_ms`,
`policy` and `lobbies`. TTL is at most7 days; future issue skew is at most30s.
Reject expired/wrong-scope/older-sequence documents, malformed or duplicate
lobby IDs/targets, invalid destinations and incompatible contracts. A client
cache must additionally refuse different payloads reusing the same sequence.
Every publication/expiry/status change must advance the sequence.

Policy has a semantic `min_release`, `maintenance`, optional HTTPS `update_url`
and bounded plain `message`. Lobby fields are `id`, `location`, `host`, `addresses`,
`port`, `contract`, `status` (`open`/`full`/`maintenance`) and weight1..10000.
`host` is a display/contour label; `network_endpoints()` returns the exact signed
IP destinations. Selection uses compatible open entries and caller entropy;
valid maintenance/update/full outcomes must not silently bypass policy through
fallback. Existing live-worker Resume must retain its owning lobby/worker and
must not go through new-admission selection.

Build and prove locally from the game checkout with shared cacheA:

```sh
export CARGO_TARGET_DIR="$(git rev-parse --path-format=absolute --git-common-dir)/../target"
cargo build --locked -p server --example game-directory
node scripts/test_game_directory.mjs "$CARGO_TARGET_DIR/debug/examples/game-directory"
```

The offline command interface (UTC values are Unix milliseconds):

```text
game-directory sign INPUT_JSON KEY_FILE KEY_ID REGION CONTOUR NOW_MS
game-directory verify SIGNED_JSON PINNED_PUBLIC_KEY_HEX KEY_ID REGION CONTOUR NOW_MS MIN_SEQUENCE
```

`sign` accepts a separate regular file containing the32-byte hexadecimal seed,
with no group/other permissions on Unix. It validates input, signs and verifies
its own output before printing the public envelope. `verify` prints only an
authenticated, currently valid typed document; failure has no usable stdout.
No networking, migration, key generation or automatic publication occurs.
Live key creation/rotation/baking and publication require the owner's separate
production approval; the publicly known fixture key must never be provisioned.

Verification:all197 shared cases and2 operator cases PASS; strict shared/example
clippy PASS. Nine actual-process/independent Node Ed25519 checks PASS, including
exact Unicode payload, wrong key/region, expiry, rollback, tamper, key alias and
unsafe key permissions. Synthetic private fixture removed. Reports:
`.agent/tasks/RANKED-LAUNCH-READINESS-20261105/p7-directory-v3-*`.
Initial v1 shared gate correctly required classifying the new `LobbyStatus` as
non-UDP; classification was added without a gameplay protocol bump, and final
v3 passes. This checkpoint does not prove runtime routing or multi-host capacity.

Remaining D1 integration gates:

- Client anonymous bounded HTTPS read, signature verification and scoped durable
  last-good cache; equal-sequence immutability and rollback fencing.
- Pinned fallback destinations and region/contour checks on both labels and IPs;
  worker handoff and saved Resume must retain the authorized destination scope.
- Maintenance/update/capacity UI, compatible alternate/fallback tests, key rotation
  and a reviewed publication kit; preserve party grouping when routing changes.
- Cross-host ready/dodge discipline and real two-lobby/worker process proof, then
  measured RU/WAN operation. No item is closed by these primitives alone.


## Implemented D1 HTTP delivery checkpoint — 2026-11-06

The regional portal exposes anonymous `GET`/`HEAD /api/game/directory`. It
fetches only its configured Account API's `GET /v1/game-directory`; browser
cookies, authorization and forwarding headers never select the upstream or
become upstream account credentials. Existing hosted ingress authentication is
applied from server configuration only. Query-selected region/contour is rejected.
The route checks its configured Host, region, contour, freshness, strict UTF-8,
envelope shape and bounded streamed body before forwarding the original bytes.
It does not reserialize JSON or round the directory's u64 sequence. The portal
is a transport guard; cryptographic authority remains the pinned Rust verifier,
and native independent verification is still required before using destinations.

Account API configuration is optional and all-or-none:

```text
OMOBA_GAME_DIRECTORY_FILE=/absolute/path/to/reviewed-signed-directory.json
OMOBA_GAME_DIRECTORY_KEY_ID=<reviewed public key identifier>
OMOBA_GAME_DIRECTORY_PUBLIC_KEY=<32-byte lowercase public key hex>
```

Unset configuration leaves the route unavailable and existing account behavior
unchanged. Partial/invalid configuration fails startup. Region and contour come
from the API build, never a request. The source must be a bounded regular file;
it is signature/schema/scope/time checked on each read. Responses preserve exact
signed wire bytes and use JSON, `no-store` and `nosniff`; missing, expired,
tampered or wrong-scope input returns generic503 without reflecting paths/data.
A reviewed regular-file atomic replacement becomes visible without restarting
the API or portal. This server does not keep a sequence high-water mark; native
durable rollback/equal-sequence fencing and monotonic operator publication remain
required. No signer or private signing key runs in either service.

Verification:4 API cases PASS, including the explicitly executed real migrated
PostgreSQL/router case and real loopback HTTP replacement; strict all-target API
clippy/build PASS. Portal257 tests PASS/3 pre-existing expected skips; typecheck
and RU production build PASS. Seven real Rust API/PostgreSQL/production Next
checks PASS, including native offline verification of the portal-delivered file,
HEAD/method/query boundaries, corruption503 and hot signed replacement. The
publicly known fixture private file was removed before services started. Summary:
[HTTP proof](../evidence/fair-play-2026-11-06/directory-http-summary.json).

Reproduce with an already migrated loopback `omoba_test`, prebuilt RU API and
signer, and an RU/production portal build; from the portal checkout run
`npm run test:game-directory` with `OMOBA_DIRECTORY_API_BIN`,
`OMOBA_DIRECTORY_SIGNER_BIN`, `OMOBA_TEST_DATABASE_URL` and a fresh
`OMOBA_DIRECTORY_EVIDENCE_DIR`. This starts only owned loopback processes and
uses a publicly known test pin. It does not establish live HTTPS, native routing,
key rotation, any second host, WAN capacity or release readiness. No live key,
service settings, deployment, workflow, dependency or migration was changed.


## Native trust/cache integration checkpoint — targeted checks PASS

Native trust and storage now have an explicit reusable API in
`client/src/game_directory.rs`; anonymous fixed-origin fetch is in
`passport/src/game_directory.rs`. This is not yet wired to live UDP admission.
No live pin was generated/baked/provisioned, and the existing ready native UI
binary is preserved for the outstanding captures.

- Trust comes from a separate caller-supplied set of up to4 public pins (unique
  key IDs). Strict Ed25519 verification authenticates the exact signing domain,
  key identity and UTF-8 payload before returning an immutable typed view.
- Every directory and cached document is bound to the exact account region and
  contour. Known foreign regional labels and IPs are checked independently;
  production also refuses the documented dev namespaces/VPS IP and mapped IPv6
  aliases. A valid signature never bypasses these destination guards.
- The regular-file cache contains the original signed envelope, retaining its
  u64 sequence after expiry. Expired data yields no admission destination, but
  remains an authenticated rollback/equal-sequence fence across restart. Same
  sequence with different signed payload is refused; outer JSON whitespace is
  irrelevant. Corrupt/unreadable/nonregular state is not silently reset to zero.
  A backwards clock returns an error instead of pretending known policy is absent.
- A separate stable OS file-lock inode serializes compare-and-publish across
  independently opened cache instances/processes. Contention is bounded/fails
  busy; process death releases the lock. New bytes are written/synced to a unique
  sibling, renamed and (on Unix) the directory is synced before exposure. Failure
  must not permit falling back to an older open policy after a newer closed one.
- Pin rotation requires an overlap and an advanced publication sequence before
  retiring the cached document's old verifier. This disk cache is not hardware
  monotonic storage and does not defend against an owner deleting/replacing the
  entire application state or an untrusted process owning its storage directory.
- Public transport derives its HTTPS URL from region/contour, ignores ambient
  service overrides, has no account token/cookie jar, refuses redirects/non-JSON
  or unsuccessful replies, and enforces a5s total/3s connect limit and64KiB
  streamed cap. Only private test construction permits a loopback HTTP fixture.

Verification:all9 native signature/cache cases and4 actual HTTP transport cases
PASS, including an actual independently launched process blocked by the OS cache
lock. All9 endpoint guard and18 post-match cases PASS; strict client lib/test-
target and RU passport all-target clippy PASS. The first cacheA client test
compile took1941.09s; subsequent test execution took0.12s and no new cargo cache
was created. [Source hashes/proof](../evidence/fair-play-2026-11-06/directory-native-summary.json).
Runtime pin configuration,
fetch/cache/fallback lifecycle, policy UI, party/Resume ownership and actual
multi-host/WAN remain required D1/P7 work.
