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


### P6 checkpoint — season profile/archive API and seasonal ladder (2026-11-06)

Profile reads now select season metadata and a bounded latest10 archive in the
same MVCC statement as privacy flags. Hidden statistics expose neither season,
archive nor badge; self/contact access follows existing rules. Self reads preserve
lifetime/XP/public eligibility fields. Ladder calibration and tie-breaks now use
season_rated_matches and the schema10 partial index, while lifetime fields remain
in the response. Ladder listing alone never exposes historical badges.

All11 Account API PostgreSQL tests PASS, including restricted portal reads,
archive ordering/bounds, public/private transitions and a1000-lifetime/19-season
newcomer excluded from the ladder. Strict store/API all-target clippy PASS.
Logs: .agent/tasks/RANKED-LAUNCH-READINESS-20261105/p6-season-api-postgres-v2.log
and p6-season-api-clippy-v1.log. The first API test attempt failed on its synthetic
privacy fixture setting public=false/stats=true; corrected both flags together,
without weakening the database constraint.

Fresh schema10 full-run v1 installed the actual approved grants, then stopped on
App::connect503 in the device-migration case. No backend SQL error identified;
unchanged focused migration and all8 device cases subsequently PASS. Root cause
is unconfirmed and v1 remains failed. Full current-source PostgreSQL/public and
CI verification are still required. Portal/native presentation, promotion/expiry
UX and release/ops gates remain open; overall TODO counts are unchanged.

Current schema10 CI42165588927: PostgreSQL Rust tests, Android compile, Python,
headless harness and both client clippy configurations PASS. Two client-test jobs
fail the same font coverage check: the CJK display subset lacks休/季/闲 from new
translations (1572 other client tests PASS). Full public journeys still running.
Repair the font in its own checkpoint; this CI is not green.


### P6 CI repair — Chinese display font (2026-11-06)

Rebuild the CJK display subset from the existing cached Noto Serif SC600 upstream
face with the handoff FontTools subset settings, preserving every old codepoint.
Exactly休/季/闲 were added (465556 bytes, +1680). Installed manifest/font package
sizes updated; asset-budget/license consistency check PASS. No upstream UI files
or production dependencies changed. FontTools4.59.0 was installed only in the
ignored task tooling directory. Source/before/after hashes and cmap verification:
.agent/tasks/RANKED-LAUNCH-READINESS-20261105/p6-cjk-font-rebuild.json.
All supported current dictionary characters and every old cmap entry are present.
This repairs the specific CI42165588927 missing-glyph failure; native test
execution must still be confirmed by the next full CI, not inferred from cmap.


### P6 checkpoint — native seasonal presentation (2026-11-06)

Native Home now shows the calibrated tier or the current-season placement count.
Own/visited/friend profiles show a tier/placement pill; profile cards add the
current window and the already earned old-season badge/final rating. Lifetime
statistics remain separate. Post-match notices use the latched round, the local
immutable award and matching current season/profile rating; repeated snapshots,
stale seasons and historical/unscoped receipts cannot announce a rank change.
The first calibrated rank, promotion and demotion have distinct copy. A personal
penalty may warn of a demotion after interruption; an abandonment/unrated result
never triggers a positive celebration. No extra result-strip height was added.

Strict native test-target clippy PASS (100.09s, minimum root40.88GiB), compiling
four new helper/system tests and the extended RU Home calibration/tier test.
Execution of these tests is pending current CI, not claimed from compilation.
Formatting PASS. Added eight CJK glyphs, preserved the old cmap, updated installed
asset bytes; dictionary coverage and asset-budget/license checks PASS.
Proofs: .agent/codex-ranked-readiness/p6-native-season-ui-clippy-v1-report.json and
.agent/tasks/RANKED-LAUNCH-READINESS-20261105/p6-native-season-cjk-font-rebuild.json.
Native main/screenshots are still required; previous ebf captures do not prove
this new interface. Portal fa4dc7f is pushed:247 tests+3 expected skips, typecheck,
production build and six RU852x393 real Chrome/Next fixture scenarios PASS.
Portal browser data is synthetic; actual API privacy/grants are tested separately.

Current core236d1c13 local full PostgreSQL run PASS:669 tests across18 suites,
zero failed/ignored, actual approved role installation. The public pipeline v2
then stopped during ready-check fixture setup (7/10 qualification writes completed
before its20s consent wait expired, only6 had searched, no ready check started).
The fixed targeted scenario now prepares all qualified accounts before Search;
its strict15s timeout, nine accepts, no premature worker, FIFO preservation,
late-accept rejection and replacement allocation PASS. This harness fix is the
next separate checkpoint. Complete public/core+new-native CI and visual gates
remain required. Overall68-finding status counts remain unchanged.


### P6 verification repair — prepare ready-check fixtures before Search

The local full public v2 failure was before any ready check: serial disposable
PostgreSQL qualification was counted inside the20s wait for nine accepts. Move
initial ten-player and replacement preparation behind a bounded90s setup phase,
verify no initial Search began, then enable Search. The actual game retains its
15s consent window; no worker before unanimity, nine-accept timeout, preserved
FIFO age, late-accept refusal and exactly one replacement allocation are unchanged.

Targeted signed local ready scenario PASS:
.agent/codex-ranked-readiness/p6-ready-setup-v3/report.json.
Initial qualification setup15.598s; replacement allocation
c40dd273b659d468ef54bcdcd917569b. JS syntax, queue policy and seven isolated
queue-only boundary cases PASS. Server/API binaries remain core236d1c13; native
13abace6 changes are separate and not proved by this wire scenario. Public v2
remains failed; complete fresh public verification must pass after this repair.


### P6 checkpoint — full schema10 core proof and targeted native QA

Schema10 core236d1c13 full CI42168782869 PASS:all7 jobs/18 public stages. Downloaded
artifact13481319080, verified SHA256
8c0338d3886cfeb2dc1d2ce101845d832aa307e9a8cee477842fcfdec04f657c before bounded,
path/duplicate/symlink-safe extraction (139 entries,10.95MB expanded). Independent
award/profile/history/season verifier PASS for all10 signed AI controllers in
human seats. Actual Rankedf7a85df6eadb8fd9c071bf1a6bdbea1f,454307ms gameplay;
season1 counters0->1, exact K/XP, noAFK/disconnection. Real tenth-public/noVK and
early crash PASS; private provider fixture absent. Backend source is unchanged
through56b86dec; later native UI remains a distinct gate.
Summary: .agent/tasks/RANKED-LAUNCH-READINESS-20261105/ci-236-independent-summary.json.

Fresh local full PostgreSQL:669 PASS across18 suites, zero failures/ignored.
Full public v3 on56b86dec:all18 stages PASS in2346.68s after separating ready
fixture preparation from Search; raw v1/v2 failures are retained. Reports:
p6-season-full-v2-verify-report.json and p6-season-public-v3-verify-report.json in
the task directory. Native changes do not alter the server/API core proved here.

Native RU build on shared cacheB PASS (982.92s, minimum root38.94GiB), binary
SHA25651ba584e1b48c089102c34c1c1d5ccea5a9d3f2f313d109de6ff4ebca5dbf868.
Six inspected RU852x393 pixels PASS: Home Gold, Home0/20 with100 lifetime games,
own/visited profile season2 and retained season1 Platinum2300 badge, friend pill
and scoped Gold promotion. Owned synthetic returning-user settings and capture
power assertions are disclosed; these prove neither legal acceptance, actual
accounts/gameplay, physical phones nor the game's keep-awake feature.

The existing Result QA deliberately owns its snapshot/profile: a helper server
in the first capture attempt overwrote those fixtures and hid the notification.
The capture driver was corrected to use no server; invalid images were preserved.
The initial demotion image showed a negative award on a winning team without
abandonment evidence. It is not accepted as a coherent example. The fixture now
models a proved returning leaver, -32 rating, zero XP and no honest completed-game
increment; only that state needs recapture. No captured pixels were edited.
Exact first-source patch/helper and inspection records are retained in the task
directory (native-season-qawip-v1.*,native-ranked-ui-inspection-v1.json).

CI56b86dec executed the four new rank-change helper/system tests successfully;
1576 client tests PASS, one fails only on the stale pre-tier Home string
(1199/20 games now correctly displays Silver). The expectation is repaired here.
Strict RU native test-target clippy PASS (45.05s,min35.87GiB), including the new
QA realism test. Formatting PASS. New final native CI and coherent demotion pixels
remain required; overall68-finding counts are unchanged.


### P6 CI checkpoint — native tests pass; register the shared QA source

CI e8b25cc7 run42181087939 executes all1578 client tests successfully (one
pre-existing ignored case). This includes seasonal rank notices, the repaired
Home tier expectation and the coherent returning-leaver QA fixture. Android,
headless gameplay, difficulty and fmt/clippy/tests jobs also PASS. PostgreSQL
public journeys are still running at this checkpoint.

The Python tooling job rejects the newly added ranked_fixture.rs because the
screen registry does not name it. Register that supporting source under the two
existing frontend/career runs that use it; Result QA is already listed among
other harnesses. Keep the source-coverage assertion intact and avoid unrelated
JSON reformatting. Targeted screen-map tests:5 PASS. The full Python tooling rerun
is in progress; its terminal result remains a separate verification gate.

Native RU build-v2 is exact e8b25cc7, SHA256
a5a4f91849a64b857c7439fd00934097cfe16d7f98d3376579e8ebb9387362ac. The v3
demotion capture is fully black while Quartz confirms the session is locked;
it is retained as failed evidence and does not satisfy the visual gate. Reuse
this binary for the next capture rather than rebuilding it. No release or
production action has occurred; overall68-finding counts are unchanged.


### P6 checkpoint — connected profiles follow the new season

The existing bounded access query now reads the canonical own-profile snapshot
in the same database statement as key status/sanctions/cooldown. Only an active
own key receives it. Periodic signed-session refresh and fresh QueueAccess
adopt a strictly newer season before Find is dispatched. No extra query, poller,
role grant, schema change or dependency is introduced. This removes the stale
pre-reset rating/placement/badge cache for already connected players.

Own-profile action and settlement replies also refuse an older season or another
identity. Same-season background reads cannot overwrite a newer settled rating.
The lifetime completed-public counter is monotonic too: a slow pre-qualification
reply cannot relock Ranked after the tenth game. Nonce correlation, revocation,
bounded pacing and fail-closed authorization remain covered. This is not an automatic season close or production reset.

Local verification on isolated PostgreSQL55582:all63 store/matching cases PASS
(including real close/access snapshot equality and missing-key/profile reads);
all497 server unit cases PASS, four DB-specific cases ignored by that default
invocation. The affected concurrent PostgreSQL access case is separately run
and PASS. Strict store/server all-target clippy and formatting PASS. Report:
.agent/tasks/RANKED-LAUNCH-READINESS-20261105/p6-season-refresh-v1-verify-report.json and p6-season-refresh-v2-verify-report.json
(final monotonic public-counter guard). Source hashes are recorded in
p6-season-refresh-tested-sources.json. A new full current-source CI/public run remains required.

Registry repair df0547fa passes the new CI42184537405 Python tooling job. Local
complete tooling also PASS:658 cases/9 skips, iOS62 and Android7. The first local
full attempt failed because loopback went through the configured proxy and the
asset test found the unrelated dirty sibling handoff. The unchanged rerun uses
NO_PROXY and the already established isolated OMOBA_UI_HANDOFF; it passes. Raw
v2 failures and v3 results are retained, not relabelled.

Demotion v4 actual RU852x393 pixels prove personal defeat despite the winning
team,1478(-32),zero XP and saved progress. The transient demotion toast is absent
from that image:the40-frame QA wait outlasted its lifetime at low frame rate.
Do not accept this as notification proof or extend production toast lifetime.
Next native capture must wait for the expected visible settled toast, then shoot.
Inspection: native-ranked-ui-inspection-v2.json in the task directory. Earlier
black v3 and incoherent fixtures remain rejected. Overall68-finding counts are
unchanged; expiry display, remaining visuals, P7/P8/P9 and RU rollout remain open.


### P6 checkpoint — server-clock season availability and transient capture

CareerView now carries optional ranked_season_open, evaluated for the signed-in
profile by the server UTC clock. Missing stays backward-compatible for older
peers/guests. Home renders a localized two-line interseason state, disables
Ranked and refuses an activation retained across a rebuild; Casual remains
available. Changes to this hint alone rebuild Home. Allocation/store checks
remain the final authority; device wall time never decides admission.

The seasonal Result QA now waits for the expected localized notice to be visible
and its toast at rest, rather than waiting40 frames past a short notification
at low FPS. Missing notices time out. Production toast lifetime is unchanged.
An explicit expired Home fixture keeps an expired90-day window and returning
player's earned badge; no real account, season close or production state changes.

Verification:193 shared and498 server cases PASS (four DB-specific cases remain
ignored by the default server unit invocation). Strict shared/server all-target
clippy and final RU native test-target clippy PASS; screen-map5 cases and JSON/
format checks PASS. Native clippy v1 found a test-only Activated field typo;
source was corrected to source and v2/v3 PASS. Reports in the task/codex dirs:
p6-season-hint-v1-verify-report.json and p6-native-season-hint-ru-clippy-v3-report.json.
New Home interaction test execution and actual expired/notification pixels await
the committed CI/native build. Overall68-finding counts stay unchanged.


### P6 current native gate and full-CI port-collision repair

Native c8260b31 CI42187649841 executes all1578 client tests PASS (one ignored),
including phone Home expiry/reopen/legacy hints, stale activation refusal and
Casual availability. RU build-v3 PASS:493.09s,minimum root36.35GiB,binary SHA256
366d088f72db1455fcab8d6a3071784f3be582d7ae8c4eb36f7ec966394a7d01.
The Mac is currently locked (Quartz verified); expired Home and visible demotion
are not captured or accepted. The ready binary is retained; no rebuild is needed.
Portal ccee8fa adds own-account calibration/promotion/demotion banners:250 tests,
3 expected skips,typecheck/build and seven RU852x393 Chrome/Next checks PASS;
two actual images inspected. Loopback API fixtures are not settlement/hardware
proof. Account/season changes, late/replayed counts and dishonest completions
cannot invent a celebration.

Earlier a877c1ce full CI42186198928 failed before its party-disconnect injection:
22 workers all failed AddrInUse on127.0.0.1:45600, which a live test client had
bound as its ephemeral source port. No client joined a worker; the party recovery
assertions were never reached. Artifact13488342913 verified SHA256
bd590225467ee9c9941e015b29cd925178c4a323ba6c325ea884542cbe2b6e00; bounded
path/duplicate/symlink/size checks passed. Original FAIL and compact diagnosis
ci-a877-party-port-diagnosis.json are retained in the task directory.

Every loopback probe bind now excludes the lobby and the entire future worker
range, waits for an accepted bind before sending, and has bounded resampling.
Diagnostics include local source port/rebind count. Two actual UDP regressions
prove rejection/release/data delivery and exhaustion without leaked sockets;
five Python-invoked policy/socket cases PASS. No production queue/worker policy,
consent deadline, workflow or server-port configuration is changed.

Fresh actual signed ten-seat party-disconnect rehearsal on c826 server + current
probe PASS in92.27s:one held Ranked Draft, nine survivors requeued with retained
age, unchanged profiles/empty histories, return codes, stale consent correlation
and replacement ten-human/no-bot roster Running. All source ports are outside
the server ranges. Report:.agent/codex-ranked-readiness/p6-party-port-v1/report.json.
Guarded build/fixture/rehearsal report:p6-party-port-v1-verify-report.json. Full
current-source18-stage CI remains required; all68 status counts stay unchanged.


### P6 scoped visual acceptance and original findings35/39

Exact c826 RU Home interseason/disabled Ranked with Casual available and personal
leaver demotion (Silver,-32,zeroXP,saved progress) PASS. Three EN native profile/
friend/visited states PASS on the same binary. Six earlier valid RU states are
reused. Portal ccee8fa seven RU notice cases and six EN season/privacy/legacy
cases PASS; images inspected. Accepted unmodified PNGs plus hashes/source scopes
are now in docs/evidence/ranked-2026-11-06 instead of only ignored worktree files.
First v5 post-unlock black frame remains rejected; system reported unlocked and
awake, so its renderer failure is not attributed to the owner. v6/v7 succeed.
EN driver path typo was corrected before its actual three-state run. No rebuild
or OS preference change was used. Findings35/39 now done;59 done,6 partial,2 open,
1 mostly refuted. Other P5/P6 and full release/P7/P8/P9 gates remain required.


### P7 signed directory contract and offline operator

D1 now has bounded schema/envelope, exact domain+key-ID+UTF8 signing bytes,
region/contour/time/sequence checks, unique concrete IP targets and compatible
weighted selection with explicit maintenance/update/full results. Offline
signer/verifier validates a private seed file without printing it; production
keys/publication are not created. All197 shared/2 operator cases and strict
checks PASS;9 real command/independent Node signature cases PASS. No runtime
client/router, deployment, dependency or protocol activation occurs yet.
The first shared run caught its new enum missing the explicit non-UDP registry;
that classification was added and final v3 passes. Synthetic private fixtures
were removed. Logs/report:p7-directory-v3-* in the task directory. Exact pending
client/cache/fallback/Resume/party/multi-host gates remain in game-directory.md.
Finding12 partial; original list now59 done,7 partial,1 open,1 mostly refuted.


### Full pre-directory CI independently verified — 2026-11-06

Commit93a769ab, [run42191648464](https://github.com/o-moba/omoba-bevy/actions/runs/42191648464):
all7 jobs and18 public/PostgreSQL stages PASS in2561.11s. Downloaded artifact
13492089050 matches GitHub SHA256
`00bcb20e9885731713d2b9bf9ba8f0aacd4979a52baec505335ca9b1f2a7980c`;
139 entries pass bounded path/duplicate/symlink/size/CRC checks. Private fixture
is absent and its removal is recorded. Independently recalculated all-ten
rating/XP/lifetime/current-season increments and matched each own history entry
for actual Ranked03f40b9047209635c7cb5e9d653bfa9d,463497ms,standard map,zero
AFK/disconnection,ten signed AI-controlled human seats and no server bots.

The ready journey proves15s/nine acceptances/no worker before unanimity/FIFO
retention/stale rejection; party-disconnect reaches replacement Running with
unchanged cancelled-match profiles/history. Eligibility proves the actual10th
public completion without VK; early crash, unique picks, remake and surrender
pass. This supersedes the prior full-CI pending note for93a769ab and its
unchanged runtime through the documentation/directory checkpoint. It is not
physical-phone, real-provider, WAN or RU-host proof. The new directory sources
at2a45600c have a separate current CI run42201671041 in progress.

Sanitized [committed summary](../evidence/ranked-2026-11-06/ci-93-summary.json).
No original-finding status changes:59 done,7 partial,1 open,1 mostly refuted;
remaining critic/UI/scale/operations/release gates stay required.


### Focused remaining native presentation proof — in progress

The opt-in fair_play_qa harness selects one of8 explicit states in one language/
viewport: Taken, party/search epoch toasts, remake menu, remake/surrender prompt
and both result presentations. It removes real transport before holding a
synthetic session, preserves production rendering/kit activation/epoch reaction,
and checks visible copy, fitting controls, vote targets and actual color data.
Remake uses the authoritative remake reason,95s window and an absent teammate;
the innocent local player receives zero XP/no rating change. Its scope excludes
actual admission/gameplay/accounts/phones; those have separate acceptance.

Strict RU client/lib/test-target clippy PASS; all5 map cases and static64-screen/
13-gap registry check PASS. Guarded v3 cacheB build is still live; no actual new
capture or completed finding is claimed. Earlier owned builds v1/v2 were
intentionally interrupted during fixture review (transport isolation, then
receipt coherence); their negative exits/logs are retained. No third cache,
cleanup, dependency, workflow or production change. Original59/7/1/1 statuses,
C5/C7 and all other outstanding gates remain unchanged until actual acceptance.


### Native fair-play checkpoint — C7 feedback accepted; C5 failure retained

f2214c66 RU build v3 PASS:1047.03s,min root35.08GiB,binary69dc09cc… . Five
accepted exact PNGs/readback/source hashes are preserved in
[the evidence pack](../evidence/fair-play-2026-11-06/README.md). C7 Taken feedback
is now done in software. The Search warning and returning-user vote menu/both
prompts are readable; vote targets are44px. No main-list count changes:59 done,
7 partial,1 open,1 mostly refuted.

Automatic green checks were insufficient for the first remake-menu frame: the
first-match guide occluded it. Default-guide prompt initialization twice panicked
with an invalid/despawned entity; original logs/backtrace/timeout/manual-stop
results are retained. Home warning over an empty backdrop and the blocked result
attempt are not accepted. Result QA omitted its synthetic player, now corrected.
Render ordering now flushes replacement vote controls before decoration; a new
real kit-decorator/modal-cycle regression is running, along with cacheB native
v4. Capture also requires its real root and no obstructing help/modal. C5/#45,
remaining P5/P6 and all scale/operations/release requirements stay open.

Directory2a45600c CI42201671041 has now completed successfully (all7 jobs),
separate from current f2214c66 CI42206582628 and this newer uncommitted fix.
No production deployment, dependencies, workflow or owner preferences changed.
