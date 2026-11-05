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


P6 AFK/#34 is committed and verified in `18389e72` plus the stable QA identifier
fix `0db576fc`. Human participation, 90s AFK/reconnect grace, individual leaver
loss/no XP, teammate half-loss, exactly-once durable cooldowns and fresh/atomic
queue admission are covered by 593 real PostgreSQL/API/store/server tests.
Host failures/caps remain exempt and legacy results retain their historical meaning.

Current CI `42061098787` passes all seven jobs and all 13 complete public/PG
journeys in 1136.32s. Native tests pass after CI caught and the patch fixed the
missing TestId on the AFK notice. Standard-map Ranked victory: 445.59s including
setup, 428402ms gameplay, result `f04d399b61e8d74b2efbe280b9f26128`. Ten fresh
profiles/history/receipts independently verify exact +24/150 or -24/100 and no
false AFK incidents. Artifact13433365824 SHA256
8ea9b8407f23a8d90a2925518937bdbe2da13ce5ec197a5c73621435d9494091 verified;
no private fixture retained. Prior 94311329 green CI remains historical evidence.

The Mac was unlocked and a real RU852×393 Home preview captured rating/placement
and direct queues. It exposed the build-label/nav overlap; the source adjustment
is committed, but its fresh native capture remains pending. System free space
recovered above20GiB, so guarded local native verification resumed. Both caches
are assigned within the allowed limit: B for native, A for headless server checks,
on Orico. No cache was cleaned or third cache created.

C5 remake/surrender is working-tree integration: protocol15/core8, authenticated
team votes, terminal persistence/history and native menu/prompt. Twelve targeted
runtime/policy tests and server/store/API all-target typecheck pass; real PG,
native and public-process checks are underway. Portal presentation is pushed in
8f6314b: 243 tests pass (3 expected skips), typecheck/production build and focused
RU390×844 Chrome fixture scenarios pass, with remake/surrender images inspected.

Owner update 2026-11-06 for C6/O-3: Ranked requires ten completed public matches,
without mandatory VK linking. Counter/server/Home/proof implementation remains.
C7, tier/season rules and all P7–P9 stay required. No RU deployment or production
schema/roles/infrastructure changes have been made.


C5 local completion checkpoint:608 real PG,183 shared and12 runtime/policy tests
pass, plus native all-target clippy. Actual signed remake and surrender process
runs passed after real96.209968s/600.183473s eligibility. All ten profile/history/
receipt records and exact awards verified; remake also proves persistent leaver
cooldown admission. Result IDs a66bc505b1e3b6695585a508e639f226 and
dce7261dce0ae4666c1f490676804993. No clocks or server throttle weakened.
Full committed CI (15 stages), native kit execution and native vote visual proof
remain pending. The frozen0db576fc local full gate also passes:2527Rust/642Python/
62iOS-tooling tests, with104Rust ignores and9Python skips explicitly retained.
C6 implementation follows after saving C5; owner decision recorded above.


C6 progress: core9 counter/backfill and server+Home gate are implemented in the
working tree. Three real PG eligibility tests pass; native clippy and13 Python
checks pass. Full PG, current native build and public tenth-completion proof
are underway. Synthetic qualification uses a separate loopback-only store example
and is disclosed in process reports; real Ranked victory remains an ordinary game.
C5 CI first found two server Clippy single-match errors; b5a6b9ba fixes them and
repeat CI42074945255 is running. Both source commits are pushed. Native screenshot
still awaits a usable unlocked screen; the owner was already asked once again.


C6 native client build passes in771.07s, root free minimum50.26GiB. All1111
tracked client/shared/common/passport files stayed identical to the build source
manifest. Binary SHA25666f1e272e29377e9d8dee5641bfefdc13f7a5153aa1dafc39c6949e16eca5ac3,
kept in the existing Orico cacheB. Fresh native capture still needs manual unlock.
The counter-only progress QA input affects only the existing render fixture; it
cannot authorize a real account. No new production dependencies or CI YAML edits.


C6 complete targeted verification: fresh `postgres_tests.py` passes all612 cases
(store53/API105/server454),18 suites/no ignores; shared184/harness27, Python13,
headless all-target clippy and native all-target clippy pass. Installed UI assets
pass with the known external-handoff override; CJK463876bytes retains old glyphs.
The native build snapshot precedes the final Searching refusal-format refinement;
its Home implementation is unchanged, but it is not claimed as the exact latest
complete client. Final screen/CI proof remains pending. The eligibility probe
supports ordinary surrender only when the worker offers it after ten minutes;
standard Ranked base destruction remains an independent stage.
C5 full CI42074945255 is now green across all seven jobs; native1571 tests pass.


C6 public v2 PASS616.87s, resultf2ba22712fd1e68d01f237c7e3f37dbc: denial0/9,
legal standard-map surrender after600140ms, saved counter9->10, fresh Ranked
acceptance10 without VK. Nine prior receipts explicitlysynthetic; the real game
is one completed public loss,25XP. Original20min natural-victory attempt retained
FAIL, not accepted as proof. Current888d16dc full CI42083864979 runs17 stages;
obsolete own a733CI42082344781 canceled after888started, a733Release had passed.
C5CI42074945255 all7/all15green with verified artifact. Native visual stillpending.
C7 pre-implementation contract frozen; code work now follows C6 local acceptance.


### C7 implementation checkpoint — 2026-11-06

The pre-implementation contract is saved at
`.agent/tasks/RANKED-LAUNCH-READINESS-20261105/c7-spec.md`. Server Select/Lock and
async ownership completion reject a locked human teammate's class; the opposing
team may mirror it and unlock frees it. Deadline fallback preserves valid human
locks, then resolves idle humans before bots with deterministic free canonical
classes. Loading replacements invalidate only changed assets. The first public
allocation boundary also checks opt-out clients; existing durable/played rosters
are not reinterpreted. No schema/protocol/dependency/deployment change.

Native class tiles and Lock now show localized Taken/disabled state and recheck
stale presses. Headless/probe clients plan available classes from authoritative
rosters, including a fifth distinct fill class. Twenty-five prematch tests pass
(including opt-out boundary, bots yielding, delayed ownership race and survivor
readiness); Node policy tests pass. Actual signed unique-draft rehearsal and
current native/all-regression checks are still pending. This checkpoint does not
close C7. The existing CI harness now includes the signed unique-draft journey.


C7 public v2 proof now PASS: actual signed ten-human Ranked draft rejected same-team
Select and Lock, allowed opposing Mage mirrors, freed the class on unlock, then
started with five distinct classes per team while preserving valid locks.
Allocation `55db1284051ccd289e2c83d2e82ff6e7`; green Warrior/Mage/Ranger/Cleric/Warden,
blue Mage/Warrior/Ranger/Cleric/Warden. Ten disclosed synthetic prior Casual
fixtures satisfy admission; gameplay/validation is ordinary. The first attempt
is FAIL: synchronous fixture helpers blocked the probe's UDP processing and caused
expired sessions. Helpers now serialize asynchronously while heartbeats continue;
no server timeout/authentication policy was relaxed. Both artifacts are retained.
Current full real PG/API/store/server regression:618 passed/0failed/0ignored in18
suites. Harness28, Node policy11cases/Python7tests, all-target server/harness clippy,
format/diff and installed UI assets pass. Native draft tests/final clippy and
committed18-stage full CI remain pending. Mac is still manually locked; no new
screenshot is claimed. Evidence `.agent/tasks/RANKED-LAUNCH-READINESS-20261105/c7-evidence.json`.
