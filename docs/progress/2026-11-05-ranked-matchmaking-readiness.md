# Ranked matchmaking readiness for the RU launch — 2026-11-05

Base: dev 2a38b2c (0.45.0). Goal: a Wild Rift–quality ranked PvP pipeline for
the Russian launch, server, client and portal, ready for ~1000 concurrent
players. Review method and the full list of findings: [the plan](../plans/ranked-launch-readiness-2026-11-05.md)
(eight reviewers, two-lens verification, a completeness critic; 68 confirmed
findings). Decision record: ADR-027.

## Changes

- **Lobby (server/src/match_service.rs, match_pool.rs, match_allocation.rs).**
  Parked queue entries per allocation and `requeue_parked`: an allocation that
  ends before its match starts returns the humans who had joined to the queue
  with their original order and age; the member who cancelled is not re-queued
  and, for Ranked, gets a 60 s `dodge_cooldown`; a no-show (not in the worker's
  `joined_sessions` receipt) is told `allocation_cancelled_or_expired`. Party
  cancel: the party searches on without the member (`party_member_left` notice,
  opt-out counted for 10 min); a member searching another queue is seated alone.
  Selection by seated humans first, then age. `OMOBA_LOBBY_MAX_WAITING`
  (1024, ≤ 4096), 4096 recent requests with `lobby_busy`, 30 s request
  retention. Worker: `saw_human` and a 60 s no-show abort; `cancel.json` names
  the leaving sessions; `status.json` names the joined sessions. Pool: a child
  with no receipt after 20 s or past its join deadline is killed; a spawn
  failure archives its directory and holds the roster 10 s.
- **Ranked window (career-store/src/matchmaking.rs, career_store.rs).**
  `CohortAllowance::after_wait`: 300 for 2 min, +100/min to 600, brackets
  merge at 5 min; `fits_cohort_with`/`same_cohort_with`; the store's rated-start
  re-check accepts the maximum window. `Manifest.cohort` records the window.
- **Parties (server/src/party.rs, shared/src/party.rs, client).** No broadcast
  on presence; one view context per 500 ms pass; issuer and invitee answered at
  once; `PartyView.error` codes localized; decline cooldown 60 s and 10
  invites/min; in-match from the lobby's allocations; friends requested on
  entering the lobby; lobby-restart notice.
- **Client (client/src/match_service.rs, frontend, i18n).** Pre-start worker
  loss or a rejected join returns to the lobby and re-sends the lock-in with a
  notice (20 s pre-start window); every lobby code, the prematch codes,
  `career.unrated.not_completed` and the party codes have ru/en/zh text; the
  developer connection line is replaced.
- **Transport and ops.** 2048 endpoints/career clients; per-IP refusal answers
  `server_full`; O(1) per-IP counting and a 250 ms sweep. RU kit: 16 arenas
  (41000–41015), `KillMode=process`, formula cgroup limits, NAT-sized
  `OMOBA_LIMIT_*`, nftables 4000/6000 pps, `max_connections ≥ capacity + 32`
  check, uplink budget documented. `make verify-public-mvp` and the CI step;
  the probe gains `--allow-large` and `--queue-only`.
- **Leaderboard.** Career schema 6 index, `GET /v1/leaderboard`, portal schema
  11 `leaderboard_listed` (default true), `/leaderboard` page, Settings toggle;
  portal copy: «Рейтинг» instead of MMR/XP, localized `unrated_reason`, newcomer
  n/20, RU landing rating section, 17-class filter, 30 s polling.

## Checks

The required gate (results below supersede the initial handoff claims): `make check`,
`scripts/postgres_tests.py` on a disposable PostgreSQL 18, `make
verify-public-mvp` (lifecycle and a mixed Ranked/Casual/vs Bots load on real
lobby + workers), portal `npm test`, typecheck and RU/global builds.

## Remaining risks and next steps

- No accept/ready-check step yet (LR-32): a no-show still costs the others up
  to 60 s of waiting before the re-queue; the dodge cooldown is in memory.
- Leaver/AFK penalty, remake and surrender (LR-33, LR-34), the Ranked entry
  gate (LR-37, owner default O-3), newcomer K and tiers (LR-53) are untouched.
- ~1000 concurrent players need more than one host: egress (~250 kB/s per
  player) and the one-host worker pool bound a VPS to tens of arenas (LR-57).
  The RU kit's 16 arenas and cgroup values are formula values pending the LR-09
  measurement on the host.
- The portal leaderboard, own rank, opt-out and newcomer line passed a real
  browser check against a loopback Account API and PostgreSQL (Russian,
  390 × 844 viewport). This is local browser evidence, not a physical-device
  test or a check of the RU production API.
- Protocol: no bump. New fields are additive (`Manifest.cohort`,
  `Status.joined_sessions`, `PartyView.error`); a protocol 11 production client
  is unaffected by the lobby changes but must not be mixed with dev's protocol 12
  servers (unchanged rule).

## Codex continuation — 2026-11-05

The inherited worktree was uncommitted. The first full gate found that a new
frontend test tried to initialize private matchmaking fields; it now uses the
public test-facing fields on a default value. Additional regression fixes:

- Automatic client returns preserve the retry across Searching/Draft/Loading
  until the shell reaches Home, and do not send a cancellation as if the
  player had deliberately dodged.
- `lobby_busy` retains the rejected search's session and request id so the
  client can actually display the correlated refusal, including on retries.
- A full Ranked roster retains an opted-in player when an older Casual roster
  could also seat them; human-count/age priority still applies to disjoint rosters.
- A worker that fails before its first receipt re-queues all parked humans with
  their original places, since it has no evidence of a player no-show.
- Explicit Ranked draft cancellation at the worker applies the same 60 s
  cooldown as cancellation at the lobby. Re-reading the failed slot does not
  extend the penalty; mere no-shows do not receive an explicit-dodge penalty.
- Existing account settings are read without an unconditional INSERT on every
  `/v1/me` poll; first-visit creation remains safe across replicas.
- The PostgreSQL cohort boundary test now checks 600/601 instead of the old
  300/301 rule, with a separate widened mixed-bracket allocation test.
- The Chinese display subset now includes 因/执/散/那 required by the new
  translations, preserving every previous glyph. Only the game font and its
  size manifest change; the sibling UI worktree is left untouched.
- Repeated live-UDP fixtures use test-only signup budgets, so previous load
  probes in the same disposable database cannot consume their daily quota.
- The portal browser test accepts explicit loopback origins and captures one
  Russian mobile viewport, allowing isolated services beside an existing stack.

No remote host, production configuration, secrets or deployment was changed.
The inherited RU kit and CI changes are reviewed as repository content only.
The remaining findings below and in the TODO are still open; this continuation
finishes the scoped readiness iteration, not the entire launch backlog.

### Verified evidence

- `make check` **PASS** in the CI-equivalent environment described below:
  formatting, workspace clippy with warnings denied, no-QA client clippy,
  **2476 Rust tests passed** (92 normal-gate ignores; database tests run
  separately), **625 script tests** (9 expected skips), **62 iOS-tooling
  tests**. In particular the transient-screen re-queue and complete Chinese
  dictionary glyph coverage tests pass.
- Portal branch `298f9f6` (feature `7c06633`): `npm test` **240 passed,
  3 skipped**; `npm run typecheck` **PASS**; builds with `OMOBA_MARKET=ru`
  and `OMOBA_MARKET=global` **PASS**.
- `npm run test:leaderboard` **PASS**, real loopback Account API + PostgreSQL
  18, Russian Chrome viewport 390 × 844. Explicit fixture origins:
  `OMOBA_E2E_PORTAL_ORIGIN=http://localhost:3010`,
  `OMOBA_E2E_ACCOUNT_API_URL=http://127.0.0.1:40551`. Screenshots and report:
  `../mm-web/.agent/browser/leaderboard/`. Not a physical phone or RU API test.
- `python3 scripts/postgres_tests.py` **PASS**: 549 tests across 18 result
  suites, zero failures and zero ignored tests, using real PostgreSQL 18.
  Includes widened-cohort allocation, the 600/601 boundary, ladder privacy
  and ordering, signed live-UDP gameplay/settlement and all new lobby
  regression tests.
- `make verify-public-mvp` **PASS**: lifecycle (including signed cancellation,
  queue expiry, stranger rejection, reconnect and Resume), then a mixed load
  with 10 Ranked + 5 Casual + 2 vs Bots players in four workers. Every player
  reached Running; worst time to Running was 40.66 s (Casual includes the
  real 30 s bot-fill wait). Raw reports:
  `.agent/codex-ranked-readiness/public-mvp/{lifecycle,mixed}/report.json`.
  The reported egress and timings are local Mac measurements, not RU capacity
  measurements or evidence for 1000 simultaneous players.
- `cargo test --locked -p harness --test admission -- --ignored
  --test-threads=1 --nocapture` **PASS**: 300 fresh profiles signed in in
  10.79 s; the same 300 reconnected after a lobby restart in 10.81 s, retaining
  their identities. This exercises admission, not 300 simultaneous matches.

All Rust checks use the existing primary checkout's cache B
(`/Users/wotori/git/ekza/omoba-bevy/target-b`, the existing Orico symlink).
The disposable database is `127.0.0.1:55581/omoba_test`.

The local shell originally selected Apple's Python 3.9 and a proxy intercepted
some loopback failure cases. Verification uses installed Homebrew Python 3.14
and both `NO_PROXY` and `no_proxy` set to `127.0.0.1,localhost,::1`. The sibling `omoba-ui/handoff`
has an older Noto Serif SC subset (455528 bytes). The game subset was
456860 bytes before this continuation; its rebuilt superset is 458764 bytes,
including every previous glyph and all new dictionary characters. Total
shipped UI assets remain 6900744 bytes, below the 6.6 MiB budget. Syncing the
sibling handoff would drop the newly required glyphs. CI-equivalent verification
sets `OMOBA_UI_HANDOFF=/tmp/omoba-mm-no-external-handoff`, which verifies the
game's committed token hashes, installed assets and size budget without that
unversioned sibling comparison. The normal sibling comparison remains a
reported local mismatch; it was not repaired or counted as passing.


P6 ongoing (#34), uncommitted source: connected/disconnected/AFK accounting,
90s grace, explicit/latching movement intent (protocol 14), individual leaver
loss/noXP, innocent teammate half-loss, durable exactly-once cooldowns, async
fresh queue access and atomic cross-owner admission denial. Player-caused
unattended shutdown penalizes proved abandons; host crashes/caps do not. Legacy
receipts cannot invent incidents or reverse an already recorded winner.

Current local PostgreSQL gate: **593 passed, zero failed/ignored**, 18 suites,
including 48 store and 440 server tests, signed live UDP, restricted-role grants,
rollback/concurrency/replay, schema 6→7 with rating 1234 and cross-owner cooldowns.
Only disposable 127.0.0.1:55581/omoba_test migrated. Shared 183, AFK runtime 15,
time-policy 10, tooling 642 (9skips), assets pass; native test targets type-check.
Workspace clippy and current client all-target/no-QA clippy pass; full client/current process gates pending.

Committed baseline 94311329: green CI 42024828738, all seven jobs/all 13 journey
stages,3194.98s. Standard-map Ranked victory in 2539.88 s, result
01ccac2e39690152af01c6cbbd3d8161; every fresh profile/history receives exact
+24/150 or-24/100. Private fixture removed. Artifact 13417265646 SHA256
fe5b706ca3d24b59f60631483f27cd3ed8f82bb2e23be1467a6c2be68ff66f4a verified.

No deployment or production schema/role changes. Full native builds are stopped
while root has 16 GiB (<AGENTS 20 GiB minimum); Mac still needs manual unlock for
native phone screenshot. Keep #34/P6 partial. Remake/surrender, unique picks,
eligibility, tiers/seasons and P7–P9 remain mandatory. See TODO and fair-play plan.

Personal outcome/forced-end regression is fixed and proven in the 593-test gate:
winning-team leavers and ranked unattended forfeits are individual losses in
history, filters and the rating graph; ordinary crash/cap interruptions do not
invent personal losses. Participation metadata remains visible after reconnect.
Portal 242 tests (3 skips), typecheck and real Chrome synthetic-API RU390x844
render pass; actual images inspected in mm-web/.agent/browser/fair-play/.
Full native execution/current journey CI still pending; owner workflow exception
requested because root16GiB violates the local heavy-build minimum. Frozen
reviewable AFK snapshots are .agent/codex-ranked-readiness/p6-afk-frozen.* in both
worktrees. Next P6 work is remake/surrender, without closing #34/P6 prematurely.
