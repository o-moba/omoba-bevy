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


C6 committed CI acceptance now PASS on888d16dc/run42083864979: all seven jobs
and17 public/PG stages pass (2535.42s). Artifact13445261847 SHA256
`f6bbf02f0461921ed1eaf98a0b2959707259f4c4db5540e5e66dbd3796711157`
verified before safe extraction; native-private fixture absent. CI independently
repeats wire denial0/9 -> real tenth standard-map completion (legal surrender,
600165ms) -> acceptance10, without VK; result263fdca0847a368acc5b0cbdd3528809.
Independent ordinary Ranked base victoryc9fdcca6ea0a945e987a5bc162f55ce4 takes
447.13s including setup/426274ms gameplay. All ten exact ±24 rating,150/100XP,
profile/counter/history increments and no false AFK independently rechecked.
Artifacts/provenance `.agent/codex-ranked-readiness/ci-888-postgres-harness/`.
C6 current native visual acceptance still awaits manual unlock; software/public
acceptance is now green. C7 full18-stage CI is separate run42090248772.

P7/#53 implementation checkpoint: an observed/reaped terminal child releases its
port immediately; a live child still holds it and an adopted process without exit
proof retains the conservative heartbeat hold. Ten pool runtime tests PASS,
including immediate reuse, adopted hold and live-child protection. Finding53
remains partial pending broader process/full acceptance. Finding54 caching follows;
no host/infrastructure/deployment change is made.


C7 local native acceptance now passes:13 focused draft tests, including the
852×393 actual Bevy layout, disabled teammate class/Lock and stale pressed-action
rechecks; final all-target client clippy PASS. These are executable UI tests, not
new screenshots. Current C7 CI run42090248772 has Android, headless harness,
dev-client, Python and bot-difficulty jobs green; full Rust and PG journey jobs
remain pending. Native visual acceptance still awaits manual Mac unlock.


P7/#53–54 targeted acceptance:112 relevant match/runtime tests PASS and all-target
server clippy PASS. Added shared250ms status/cancel snapshots for all career-view
recipients, with explicit fresh enqueue/cancel checks and monotonic observed end.
Between-polls settlement still refuses handoff at the real broadcast refresh
boundary. The view itself does no filesystem I/O. Tests cover ten cached views,
no revival by a stale Ready receipt, cached cancellation sessions, immediate exit
reuse and live/adopted holds. After101 consecutive completed arenas, startup now
adopts the retained receipts as one occupied physical bind endpoint instead of
refusing them as101 live arenas; the4096-entry scan/100occupied-port ceilings stay.

Review also found a terminal-cancel race: refreshing Finished/Failed before a
late Cancel must not invent a dodge. Finished and played Failed never requeue;
unplayed Failed returns only other parked humans with original FIFO. All three
regressions pass without changing cooldown rules. Full current PG and signed
lifecycle/recovery process acceptance remain pending; findings53/54 remain partial.
No RU/host/infrastructure changes or capacity claims are made.


P7 current full real PostgreSQL acceptance:623 tests PASS,0failed/0ignored,
18 suites. Relevant runtime112, all-target server clippy and Python harness5
checks pass. New server binary built from current P7 source; the next actual
signed process check uses capacity1 to prove post-crash durable cleanup and
reassignment within the new20-second deadline. The same capacity1 constraint
was added to the existing CI early-crash stage, without adding a workflow/stage.
P7 public lifecycle/recovery and committed full-CI acceptance are still pending.

Physical-device inventory (read-only): a USB Android device is connected and
authorized but screen-locked. Installed dev0.44.0/beta0.45.0 builds are from
October28/29 and predate the VIBRATE patch; they cannot prove current mobile
feedback. iPhone/iPad are unavailable. No app install, device unlock or physical
haptic/keep-awake PASS is claimed; existing manual Mac-unlock request remains.


P7 pool final local/process acceptance PASS: strict monotonic end-state guard,
112 match/runtime tests,623 real PG/API/store/server tests in18 suites (no skips),
all-target server clippy and existing Python harness checks. Final signed
capacity1 early-crash rehearsal reuses the same physical endpoint in2546.99ms
after durable cleanup (new20s deadline; terminal heartbeat age must be<35s).
Original result0e9957815ee8e7d9a55881e0eb98fa5b is Interrupted exactly once,
original epoch retained; profile/history checked, no Elo/XP, ordinary120s recovery
gate retained. Final lifecycle also passes queue expiry despite heartbeat, signed
cancel, stranger denial, stale-handoff isolation, retained identity/progress on
reconnect and leave/resume. Initial prior-candidate proofs remain separately
retained; only final artifacts prove the stricter20s and backward-status guard.

Final server SHA256f8cabb3071f6110e534aca9d5e4379c2086e09a2cb7471962a0db0a134b6e39f;
source/report hashes and exact assertions saved in
`.agent/tasks/RANKED-LAUNCH-READINESS-20261105/p7-pool-evidence.json`.
Findings53/54 remain partial until the new committed full-CI repeat passes;
the existing Ranked early-crash CI stage now uses capacity1 and the stricter20s
same-port assertion. No workflow YAML, dependencies, schema/protocol or production
infrastructure/deployment change. Next independent scope is bounded selector21;
season/cross-play choices still await the owner's O-5/O-18 response.


C7 committed final CI PASS on47ee63ae/run42090248772:all7 jobs and18 public/PG
stages pass in2609.62s. Native1572 PASS/1intentional ignore includes Taken/Lock
stale-press test. Artifact13446519867 SHA256
`e2e3b453bc610c40c9727840dd0b68ae6e274d098e31a6d8d3009a83d9f82069`
verified before safe extraction; private fixture absent. Independently rechecked
signed unique-draft allocation53bfa15be7cd702d6cf410d3f2934037 (five classes/team),
Ranked base victory0e369bd233d5a67582b3b67cbc3d8575 (544.82s setup-inclusive,
523474ms gameplay, all ten exact ±24/150/100XP/profile/history/participation), and
real tenth public completion6a8f091a936f62aec6c9755455254414 (surrender600100ms,
wire denial0/9→acceptance10 without VK). Full reports/provenance/summary in
`.agent/codex-ranked-readiness/ci-47-postgres-harness/`. C7 software/public
acceptance is green; current native visual still awaits manual unlock.

P7 selector21 core checkpoint:45 matching tests PASS including independent
subset oracles, mandatory anchors, due bits, parties/cohorts/wait boundaries and
integer limits. Actual optimized instrumented queue selection measures3.15ms
for1000 and12.94ms for4096 distinct-rating waiters; same-rating and awkward-party
cases also obey bounded-work assertions. These are algorithm checks, not a
1000-playing-client claim. Waiting-view nested scans and enqueue-time repeated
capacity-line construction are still being replaced; finding21 remains partial.

### P7 pool committed acceptance — 2026-11-06

Commit5c0583ee passes all seven CI jobs and all18 public/PG harness stages in
[run42097650577](https://github.com/o-moba/omoba-bevy/actions/runs/42097650577).
Artifact13451811044 SHA-256
`427648b70648e59cd0c2f90277a96f469184b2dfc95955e97c8bec1cc6693dd7`
was downloaded, hash/path/private-fixture checked and retained locally under
`.agent/codex-ranked-readiness/ci-5c-postgres-harness/`. Independent recheck
confirms all ten exact Ranked awards, profiles/history and no false AFK; actual
tenth public completion; and ten-human capacity1 crash recovery with the same
physical port reassigned in2452ms after ordinary durable recovery. Findings53
and54 are now done. No RU deployment or host-capacity claim follows from CI.

### P7 signed queue load and authorization correction — 2026-11-06

The first1000-target attempt stopped before load on a helper-status publication
race; the corrected10-client smoke passes. Actual1000-target v2 then ran the
full240s and FAILed: roughly600 endpoints lost authentication. Preserve both
failed reports under `.agent/codex-ranked-readiness/p7-queue-1000-{final,v2}/`.
Periodic access refresh had tried to send every client through a256-job link.

Bound refresh to16 oldest-due requests per100ms, retry missing replies after5s,
and preserve the original30s fail-closed deadline. Four new fair-load/congestion/
missing-reply/old-nonce tests and all643 real PG/API/server tests pass. A related
presence batch exceeded the store's512-entry bound; producer deduplication and
worker-side bounded calls are now implemented, with a fifth regression and
actual1000-presence process verification pending. No authentication bypass,
production load, dependency, schema or protocol change is introduced.

Actual queue v3 also FAILed at240s (373 Waiting); the old serial account read
stream cannot sustain500 FindMatch checks/s plus periodic refresh. No1000-client
acceptance is claimed. The next candidate batches at most4 consecutive
QueueAccess/RefreshAccess reads using the existing current-thread runtime and
4-connection lobby pool. It stops at all non-read barriers and drains older
parked operations first after DB reconnect. Strict server clippy passes; full
PG, live boundary regressions and repeated1000-client load are pending. Earlier
10-client v4 verifies both wire queue state and ten actual fresh presence rows;
its12s soak covers the normal10s presence cadence.

### P7 saved queue/runtime checkpoint — 2026-11-06

Current source passes649 real PG/API/server tests in18 suites (0 failures/skips),
including56 matchmaking cases, immediate near-deadline signed control handling,
read-batch barriers and real DB revocation. Strict server clippy, formatting and
three Node policy checks pass. Actual signed ready-check, party-leave/stale
handoff and unique-draft process journeys pass; reports are under
`.agent/codex-ranked-readiness/p7-selector-*-final/`. Fresh server SHA-256
`8d1192b9cfcbc3901d3ccbf44bb572709ee23cfe5141bdb37586c13485e2ce02`.

The ten-client v6 smoke also verifies ten fresh DB presence rows. Actual1000
v5 reaches all targets Waiting with no authentication loss, but FAILs final
account-error acceptance:266 profiles still report Busy. Preserve FAIL; this
checkpoint does not close finding50 or certify capacity. Continue account read
throughput work from the frozen supporter-read specification. Bounded queue
selection/view implementation21 still needs committed full CI.

Pool5c's separate release-build workflow42097650568 has Windows/Linux/Android
success, macOS cancelled and draft-release skipped. Its seven-job CI and all18
public/PG stages are fully green; the cancelled macOS job is not release proof.
No RU deployment, production infrastructure/credentials or dependency changes.

### P7 account-read/queue/admission/arena proof — 2026-11-06

Follow-up source reads supporter time, bounded active-first grants, stable points
and optional aura preference in one SQL statement. Independent legacy-snapshot
comparison covers no preference, explicit NULL, insufficient styles, future/
revoked and more-than-limit grants; career totals are unchanged. Local samples
of20 reads: empty422.22→40.13ms, bounded-active136.69→37.09ms. These are query
samples, not a universal speed guarantee. Strict store/server clippy and all650
real PG/API/server tests in18 suites PASS,0 failures/skips.

Fresh server SHA-256
`4ea743f96a5ae3d8c9a26da89134c5eba93f523423e0c7af0d83645dd3d1d74a`.
Actual queue v6 PASS: all1000 signed targets Waiting with no final account/socket
errors or ServerFull, current request correlation and no world/worker handoff.
All1000 DB presence rows are fresh under the same lobby owner. Response gap
p95=276.61ms/p99=294.61ms; maximum current age168.84ms. Sign-in-to-queue
p95=5.62s/max11.56s includes the probe's ordinary retry pacing. The server group
(lobby plus one disclosed ordinary capacity-holder, excluding Node/DB) sampled
43.39% interval-mean CPU and47.25MiB peak RSS. Per-source fixture allowances are
logged; this is not a production NAT or playing-capacity claim.

Raw boundary v2 PASS in5.15s:2048 admitted paths, endpoint2049 gets correlated
72-byte ServerFull for a495-byte probe. Global and per-source caps are both2048;
the wire proof does not independently distinguish them. No accounts or arenas
are created; the disposable DB only supports mandatory startup/heartbeat. Initial
4096 per-source configuration failed before clients and remains FAIL.

100-arena sample PASS:100 connected humans/900 server bots,101 server processes,
maximum time-to-Running30.16s. Snapshot gap p95=200.72ms/p99=413.20ms; downlink
per player p50=221481/p95=286318 bytes/s. Whole server-group CPU149.7% (about1.5
cores) and peak RSS1538.4MiB on Apple M4 Pro; no cgroup/RU constraint was applied.
The owned test PostgreSQL max_connections was temporarily100→200 with0 clients,
then restored to100 after owned workers stopped. This is not1000 connected humans.

Retained reports: `.agent/codex-ranked-readiness/p7-queue-1000-v6/`,
`p7-admission-boundary-2048-v2/`, `p7-hundred-arenas/`; hashes/specs/evidence
under `.agent/tasks/RANKED-LAUNCH-READINESS-20261105/p7-supporter-read-*`.
All earlier failed candidates remain retained.64e777be is pushed;
[CI42116356054](https://github.com/o-moba/omoba-bevy/actions/runs/42116356054)
has six successful jobs and the complete PG/gameplay gate still running. The
follow-up query source needs its own committed full gate.1000 playing, egress,
remote pools, RU-host proof, P8/P9 and pending native/mobile acceptance remain
mandatory. No production configuration, credential, dependency/schema/protocol
change or RU deployment occurred.

### P7 bounded queue committed acceptance — 2026-11-06

64e777be CI42116356054 passes all7 jobs and18 PG/public stages in2593.95s.
Artifact13458588706 SHA256
`2a494d6208d8efb6ac5c68cfb01a58786a251df12462b03210439b0e8eae33e5`
checked before safe extraction; private fixture absent. Independent exact award,
profile/history and participation checks PASS for ten-human Ranked base victory
599786f987e9722c70ea4f4589f4075a (421719ms gameplay), plus real tenth public
completion and0/9 denial→10 acceptance without VK. Capacity1 early-crash recovery
reuses the same port in1764.28ms after durable cleanup. Finding21 is done;
follow-up0c596fab CI42123329169 is still running.1000-playing/RU/P8/P9 remain.

P9 dependency review additionally finds four open rustls alerts (#21–24) against
locked0.23.25. Prepared compatible0.23.46 changes only its version/checksum;
no package identities, manifests, TLS validation, CI/configuration or deployment
changes. Frozen criteria and upstream advisory references are saved in
`.agent/tasks/RANKED-LAUNCH-READINESS-20261105/p9-rustls-spec.md`; fresh affected
PG/API/server tests are running. Do not call the new lock accepted before checks.

P9 local TLS lock acceptance PASS:650 real PG/API/server checks in18 suites,
zero failures/skips;58 passport/HTTP checks pass with3 pre-existing live
paired-account/Studio/registry-approval integrations ignored. Strict all-target
clippy across store/API/server/passport/arena-sync and formatting pass. Only
rustls version/checksum changed; full CI of the saved commit remains required.

P7 egress prototype (not applied):40 byte-exact standard-map running snapshots
from frozen0c596fab binary total695213 bytes. Draft bounded level1 zlib codec
produces145459 envelope bytes (-79.08%),614→141 UDP frames. Local encode median
51.58us/max64.42us, decode median33.85us/max42.53us; these are microbenchmarks,
not runtime capacity proof. Seven prototype transport/adversarial tests, exact
Node/Python cross-decode and120 Node truncation/trailing/concatenation refusals
PASS. Direct production use of already-locked flate2=1.1.0 awaits owner approval
under the root AGENTS dependency rule. No production transport/protocol/manifest
change has been applied. Specs/draft/corpus/results are retained under task proof.

### P7 connected playing clients — 2026-11-06

Added safe per-peer UDP counters (raw/blocking/drain, accepted complete snapshots,
JSON/framing errors), safe allocation telemetry and guarded native shard runner.
The runner rejects missing/stale/duplicate peers, bot-filled or inconsistent
allocations, counter regressions and non-current worker receipts. It preserves
Running receipts at acceptance and exact source/binary/manifest hashes before
cleanup.1 loopback UDP counter test,28 harness unit tests,5 Python policy/receipt
checks and all-target harness clippy PASS.

Ten-client final and100-client initial probes PASS before actual1000-client
acceptance:1000 signed native AI actors occupy100 ten-human Casual arenas,
zero server bots. All Running49.22s after startup;20s all-playing soak passes
continuous freshness/identity/error checks. Independent current hashes and100
Running ten-human receipts rechecked; final max peer age573ms, no final account,
JSON or framing errors. Fresh server SHA256
`ba9d40bccbe9d469eae9fccfdabf8bdafe4e44d56f6c8d45b89004bb34f59351`,
harness`c9a7c821e3ad60d6671cb9cb5a141bc440272c2d91f63520408eadf94f5347d0`.

Over18.81s resource samples:101 server processes mean526.08% CPU, peak1536.55MiB;
100 generator processes mean224.67% CPU, peak582.27MiB. DB CPU is excluded.
Downlink p50=200199/p95=258272/p99=278937 bytes/player/s; received complete
snapshots median11.72Hz/p05=10.97Hz, below nominal20Hz. This is short local
M4 Pro14-core48GiB evidence without RU/cgroup limits, not a sustained SLA,
1000 physical people or1000 rated players. Egress/performance and RU proof remain.
Owned PG max_connections100→200 was restored100 after all owned processes stopped.
Full raw reports: `.agent/codex-ranked-readiness/p7-playing-{10-final,100-v1,1000-v1}/`;
checks/specs/hash evidence under task `p7-playing-*`. New tool full CI is pending.

### P8 maintenance runtime and P9 CI correction — 2026-11-06

C2 runtime reads a bounded operator marker once per lobby scheduling tick; views
read cached state. Paused0/9/10-accepted rosters return to waiting without dodge
strikes, retaining original party/FIFO/age; live assignment Resume remains.
An additive capacity report publishes draining plus exact operation ID; invalid
markers stop allocations without acknowledging an operation.6 new server tests
and all656 current real PG/API/server tests in18 suites PASS,0 failures/ignores.
Strict server/API/harness clippy passes. Native maintenance retry/cancel and
RU/EN/ZH failure-text checks PASS in B; Mac still locked, fresh visuals pending.

Signed capacity2 drain v2 PASS:one running match keeps advancing while a free
arena stays unused, pending consent pauses beyond15s without penalty, delayed
decline is harmless and a fresh search gets correlated maintenance refusal.
After cancellation a new explicit check starts a second real match. A second
operation survives actual lobby restart; both workers remain adopted, existing
seat Resume works and new searches remain refused. Host ship/wait/cancel
controller and production approval remain required; no RU/config change.

P9:0c596fab full CI42123329169 FAILs only Ranked-victory,6/7 jobs and17/18
public/PG stages PASS. Verified failed artifact13462397903 SHA256
`86ba4dd956ba80df7fa19d6ecc2e1b25677c95a25ab0b896cce00fb93f4d418a`.
Natural base victory lasts955624ms; a connected Ranger idles >90s (unchanged
visible pose/action109.55s), validly forfeits -32/0XP and innocent losers lose12.
The ordinary no-AFK Elo proof correctly rejects this. Keep FAIL. Test actors
now send ordinary held movement/aim on every fresh alive decision, like native
active-input controls while waiting; dead/unjoined/stale actors remain inactive.
No production AFK/Elo change. Verifier saves all result rows before asserting
and explicitly refuses AFK/abandonment/disconnection.9 verifier tests PASS.

The first local P8 PG run also retained FAIL:shared Unicode queryНо matched
more old fixture names than the bounded search page, hiding the target. Unique
short Unicode rename/lowercase prefix fixes test isolation without deleting
data or changing the API; fresh full656-test repeat PASS. Current actual
standard-map ten-client Ranked victory is running; new committed full CI remains
required. All proof specs/raw failures/results remain in task artifacts.

P9 corrected current standard-map Ranked victory PASS in435.92s including
setup: result70227325cf99969e84347c8d4e7d3c97. All ten exact Elo/XP/profile/
history and participation rows independently rechecked; zero AFK/disconnection
or abandon, no map/rule/time change. Qualification uses the existing disclosed
loopback synthetic prior Casual receipts; actual match uses ten signed human
seats. Full committed CI remains pending on5aa462a2.


Owner O-5/O-18 and compression decisions2026-11-06 recorded in canonical
launch-readiness/TODO. Tier/season/input statistics are approved but not delivered
yet. Compression production integration is protocol16 with no new package
identities; bounded shared/Python/Node checks pass, actual current gameplay/load
and full CI remain required. Current native5aa Home10 pixels pass rating/
placement/footer checks; Home0 exposes clipped locked text, now corrected and
building for repeat capture. No physical-mobile or RU rollout claim.

TLS5da85959 full7-job/18-stage CI42126718754 independently rechecked PASS;
artifact13462799158 hash526d44735eb79bb3e5a66d455cd1d5e55b1851e1c4e182bce8c964b07dc446c0.
Actual Ranked a4c2b83b9034cc894ac3ca1bf454fbec (426382ms), exact all-ten
awards/history and zeroAFK/disconnection; real tenth-game/noVK acceptance PASS.
Current5aa and compression CI are separate checkpoints.


Compression local187 shared/656 real PG/API/server/28 harness unit/5 actual UDP
checks and strict affected-crate all-target clippy PASS. Signed Node15s ready
lifecycle v2 passes protocol16; first command omitted its qualification helper
and correctly failed Ranked eligibility (retained failure). Current native
Home0/10 captures visually PASS with corrected compact labels; C6 software now
complete. Full current Ranked, playing-load and compression CI remain pending.
5aa462a2 CI42132894282 independently rechecked all7 jobs/18 stages PASS,
artifact13466364097 SHA256c67161d2f13da078d98def557d205b16f5761a929f8c4cbaaba034368752c9bf.
Ranked a72dd61c9a1f635bb60da7092d3cb330,426600ms, exact all-ten awards/history
and zeroAFK/disconnection; tenth/noVK and early-crash proofs PASS.


Frozen ebf3a1fa protocol16 full Ranked PASS:3efdf9f470acaf248c024bc243185a98,
497.09s setup-inclusive/469460ms gameplay, all-ten exact awards/history and
zeroAFK/disconnection independently verified.1000-playing60s v1 live acceptance
PASS (p95 wire42014B/s, median received9.54Hz) but normal wrapper cleanup wait
raced connection closure; owned fixture restored100 after idle verification.
Repeat v2 restores100 normally but FAILS load: native actors retain expired
challenges/lost authority. Both failures retained, no overall scale acceptance.

Correction now passes30 harness tests (fresh nonce, epoch protection, paced
retry and authority reset),191 shared/14 matching and5 capacity-tool tests,
affected strict clippy; native test targets typecheck the alternate revocation
wording. Actual repeat/full CI remain pending. P6 pure rules preserved in local
e459dab8; seasons/archive/input persistence and UI remain open.


P7 warmup v4 FAIL retained; no Search released and owned test PG restored100.
The old world-dependent BOT_SAMPLE stopped updating for several unjoined lobby
shards; independent later PUBLIC_BOT messages show authenticated peers. Stale
samples cannot establish a permanent authentication failure. Authentication
telemetry now emits independently of world snapshots, with current matching-nonce
acknowledgement age; admission to the benchmark requires every acknowledgement
<3s old continuously for5s. Foreign nonce/epoch cannot refresh evidence.
32 harness unit/1 CLI process/16 Python telemetry-policy cases and strict harness
clippy PASS. Actual ten-client pilot PASS: all authenticated4.85s, Search9.86s,
all Running23.46s,15s soak, p95 wire58166B/player/s and median complete snapshots
17.77Hz. No qualification receipts or gameplay policy override. Fresh1000-client
repeat remains required; season/input persistence and UI remain open.


P6 declared controls persistence checkpoint: native movement carries the selected
control scheme as additive signed metadata. Authenticated human-seat evidence
is monotonic (unknown cannot erase touch/mouse; switching becomes mixed), survives
AFK takeover/reclaim and checkpoints, and does not itself count as activity.
The store merges prior checkpoint evidence before freezing intent, preserving
recovery and idempotent settlement without changing append-only participant rows.
Old receipts omit unknown metadata byte-for-byte. A bounded anonymous query and
read-only ranked-input-stats example expose four groups with wins, sample sizes,
scanned receipts and eligible completed Ranked games; bot/abandon seats excluded.
Details: docs/ranked-input-statistics.md. No queue or matching weight uses controls.
192 shared,17 fair-play and all55 real-PostgreSQL store/matching tests PASS;
33 harness tests PASS, including a silent Home path re-probe after25s using the
same account key. Native test-target check and strict native clippy PASS.
Strict all-target clippy for shared/common/store/server/harness/browser PASS.
Fresh server/actor binaries and full committed CI remain pending.
Season/archive/badge persistence and native/portal season UI remain open.

P7 independent-auth v5 FAIL retained: all1000 cached phases say authenticated,
but only976 have fresh acknowledgements;24 are stale >25s (maximum265s).
Search correctly remains held until the300s deadline; owned PG restored100.
The disposable actor now re-probes a silent held-Home path after25s, retaining
its key and never counting stale authority as fresh. Local unit proof PASS;
actual repeated1000-playing acceptance remains required. No server authority,
expiry, qualification or gameplay policy was relaxed.


P7 fresh source78070b27 repeated1000-playing v6 PASS: all1000 ordinary sign-ins
fresh/error-free at99.61s, stable5s Search release104.63s, all1000 Running146.16s;
60s uninterrupted all-playing soak,100 ten-human Casual arenas, zero server bots.
Independent source/binary/100 manifest hashes, exact1000 unique identities,
100 endpoints, current snapshots/error counters and owned process-group cleanup
recheck PASS. Owned test PG restored200->100, actual zero clients verified.
Total206.91s. Downlink received wire p50=34553,p95=39347,p99=43574B/player/s;
complete snapshot median8.642Hz,p05=8.090Hz. Server3.356 average cores/1546.125MiB
peak (101 processes); generator2.460 cores/574.266MiB (100),59.61s resource window,
DB excluded. M4Pro local native AI controllers, not physical humans or RU/WAN/
cgroup evidence. Fresh server/actor builds and six affected-crate strict clippy
PASS. Silent held-Home recovery resolves the observed v5 dead-path benchmark
failure; all failures retained. No new qualification receipts or gameplay override.
Received cadence/latency SLA, sustained load, real uplink and RU capacity remain
open; do not infer production1000-player acceptance from this bounded proof.
P6 input persistence/query is implemented; seasons/archive/badges and their
native/portal presentation remain the next product integration work.


P6 season schema/contract checkpoint (not runtime integration): SeasonProfile
separates current placements from lifetime totals; SeasonBadge supports a
provisional participation award.193 shared tests and strict five-crate all-target
clippy PASS. New migration010 is drafted with a90-day initial window, FK-checked
profile season snapshots/counters, immutable allocation mapping/archive/closure
receipts and deferred exactly-one-open-window protection. Registration takes a
shared season lock before stamping metadata. An owned local transaction applies
all migrations, verifies unchanged2300 rating/40 lifetime Ranked/55 played/50
public/12000XP, zero initial season counters, new registration and immutable
season deletion, then ROLLBACKs. Existing test DB remains schema9.
SCHEMA_VERSIONS remains9; profile reads still return no season summary. Migration
activation, allocation/settlement locks/counters, explicit close command, durable
season identity through manifests/results/recovery, queue expiry/cached profiles,
role-grant draft, full PG concurrency/rollback tests and UI remain required.
No production season or rating reset has been performed.


P6 seasonal storage/close engine implemented locally under schema10. Initial
migration preserves rating/lifetime/XP/public access while starting seasonal
placements at0. Immutable manifest/result season IDs and allocation mapping fence
late recovery; shared begin/settle lock precedes match/profile locks. Explicit
expired expected-ID close uses an exclusive lock and one transaction for archive,
badges, half-distance reset/new90-day window, with idempotent close receipts and
original-rated-intent blocking. Current-season K/cohort and Home placement count
use seasonal games. Expired queue/ready consent stops without dodge penalties.

Registration role proof found an invoker FOR SHARE privilege failure. A narrowly
scoped owner trigger, trusted-schema/pg_temp search path and revoked PUBLIC
execution fix it without granting runtime season UPDATE. Repeatable Read, actual
registration/close contention and caller temp-table shadowing proofs PASS.
193 shared,63 store/matching (real PG included),64 lobby and33 harness tests PASS;
strict affected-crate and native test-target clippy PASS. New Home regression is
typechecked, not yet executed.12 Python verifier cases use seasonal K/counters.
Actual operator CLI proof in an owned synthetic expired schema PASS: show and
invalid commands leave data unchanged,2300->1650 once, repeated receipt identical,
lifetime/XP/public totals preserved; owned schema removed. No real-game claim.

Role deployment patch is prepared/tested, awaiting owner response; actual
account-api/ops/grants.sql is unchanged. API/ladder/archive read presentation,
native/portal tiers/badges/promotions, expiry/cache UX, current full public/role
journeys and visual/CI gates remain open. Production migration requires old-worker
drain; close requires all-host drain including reservations and cache refresh.
Schema9 binary rollback after migration10 is unsupported and must be covered by
P8 deployment review. No production migration/reset/deploy has been performed.

Schema9 ddc59e1d full CI42153618597 independently rechecked all7 jobs/18 stages
PASS (2540.45s). Artifact13475806511 SHA256
5c83acdf6fef624de2e782ce9c61d06567b67c73a8363bbfccc14b702ecd8100
verified before safe extraction. Actual Ranked9cf904dd956939d5332f54fe1f8b2786,
418283ms gameplay, exact all-ten awards/profile/history and zeroAFK/disconnection;
real tenth public/noVK and early crash PASS. Private provider fixture removed.
This evidence precedes and does not cover the local schema10 engine.


Owner approved the prepared season-role repository patch, including the disclosed
portal archive read. Applied to account-api/ops/grants.sql: metadata SELECT,
portal archive SELECT, game SELECT/INSERT on immutable mapping, explicit season
sequence revoke. Runtime archive/close/reset writes remain ungranted. The exact
permission delta, registration/temp-shadow and post-reset runtime gameplay tests
PASS. Fresh full installation/role/public verification is starting in a new owned
local PostgreSQL fixture; the older test fixture remains for prior evidence.
No production role or deployment action was taken.
