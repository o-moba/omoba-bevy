# TODO: LOBBY — matchmaking review artifacts (2026-11-05)

Dump of the 2026-11-05 ranked matchmaking review so nothing is lost between sessions. Source of truth for the plan: [ranked-launch-readiness-2026-11-05.md](ranked-launch-readiness-2026-11-05.md); decisions: ADR-027 in [../architecture/decisions.md](../architecture/decisions.md); progress: [../progress/2026-11-05-ranked-matchmaking-readiness.md](../progress/2026-11-05-ranked-matchmaking-readiness.md). Raw reviewer output (JSON, local only): `.agent/tasks/RANKED-LAUNCH-READINESS-20261105/raw/review-findings.json`.

Branches: game `feat/ranked-matchmaking-readiness` (this repo, PR into `dev`), portal `feat/ranked-matchmaking-readiness` → o-moba/omoba-web#33. Disposable current-schema PostgreSQL: Docker `omoba-mm-seasons-pg` on 127.0.0.1:55582 (`omoba_test`, career13, portal11; owned fixture only). Legacy prototype fixture `omoba-mm-test-pg` on55581 is retained separately and is not clean-installation proof. Cargo cache B: `/Volumes/Orico/omoba-build-cache/target-b`.

## Current release checkpoint — 2026-11-07

Original68: **62 done,5 partial (#1,#2,#12,#20,#50),1 open (#13)**.
Critic/physical/UI and deployment requirements remain additional acceptance gates.
Current local proof: **575 actual whole-server cases (16 real-PG) and116 current
monitor/host-tool cases PASS**. Store code is unchanged from its101 actual-PG18.6
case checkpoint. Bounded Ready/Flush completions retain exact work/generation/
result data while a live receiver is full. Intake pauses before the256-slot
budget can overflow, with SQL/lease/probe/health work continuing. The reproduced
lease starvation fails before and passes after; a seeded full-budget stress case
retains256 completions, keeps the independent active-match lease above60s after
12s with no pending Start, and delivers every original sequence plus the barrier.
[Current critical-reply proof](../evidence/critical-replies-2026-11-07/README.md).

Health report v2 exposes critical backlog/known closed recipients; paired monitor
validates the new format. Required schema13/permission/storage health, reconnect
recovery, bounded ready retention, mandatory SQL consent and restore evidence
remain verified. Current signed15s/FIFO/replacement through ten-human Running and
fresh v2 health, plus real worker SIGTERM/durable shutdown, PASS. Full current CI
and actual host/physical/recovery/load acceptance remain required.
Current-schema13 physical/WAL and restored runtime rights now PASS in isolated
PG16.15/18.6 containers,20 checks each. Exact named/latest consent/claims,
queue discipline and rating/history are preserved; the C4 helper remains unapplied.
[Current local recovery proof](../evidence/schema13-recovery-2026-11-07/README.md).
Current PG18.6 raised-limit proof now adds actual signed public-worker physical
restore, same worker/profile/epoch Resume, lobby adoption and223.47s real crash
recovery:22 stages PASS, once-only Interrupted credit with no Elo/XP. The reviewed
C4 helper now preserves all five recovery dimensions; original hardcoded20 fails
when the source is80. Revised candidate is still unapplied,176 unit pass/1 skip.
[Current worker and recovery-limit proof](../evidence/worker-physical-recovery-2026-11-07/README.md).
PG16.15 now passes the same22 raised-limit/current-worker stages, including
221.63s ordinary crash recovery and a fresh post-restore authenticated probe.
The runner waits for writable promotion after connection readiness; the rejected
immediate assertion is recorded.
PG16 actual terminal-file/duplicate physical replay now also passes22 stages:
SIGTERM with SQL offline, exact original Settle absent from source SQL, automatic
recovery and a second public-worker redelivery, one credit and no Elo/XP. The
complete restored result differs only by saved=false→true.
[Terminal proof](../evidence/terminal-spool-replay-2026-11-07/README.md).
Next: independent off-host/RU acceptance; UI-2/UI-3 exact current pixels and physical C3; actual multi-host/WAN
and sustained host capacity/cadence; trust packaging/provisioning; current full CI.
C2/C4 host-tool proposals still await their existing owner decisions. Original
review coordinates and dated checkpoints below preserve history; their older
schema numbers, test counts and superseded limitations are not rollout commands.
**No RU migration, season reset or deployment has been performed.**

## Method

Eight independent read-only reviewers (lobby queue, allocation/start, rating/settlement, client flow, admission/scale for ~1000 players, parties, portal/Account API, tests/harness), two-lens adversarial verification of each finding (factual, material), one completeness critic. 77 raw → 68 confirmed findings (3 blocker, 23 major, 42 minor; 11 bugs, 24 gaps, 20 scale, 13 ux) plus 9 critic items. Finding coordinates retain the2026-11-05 review baseline; statuses follow verified continuation checkpoints (latest2026-11-07).

## Review-baseline invariants and preserved behavior

- [LOBBY QUEUE AND ROSTER SELECTION] Queue policy is correct and tested: Ranked = exactly 10 humans of one rated cohort (same newcomer bracket, <=300 spread from the anchor's lowest rating), never bot-filled; Casual = every human regardless of cohort, bot fill once any member has searched Casual >= 30 s; vs Bots allocates at once (server/src/match_service.rs:869-930, tests 1146-1200, 1370-1414). Rated flag comes only from the queue (`MatchPreference::rated`, shared/src/match_service.rs:29-33), so 10 humans in Casual stay unrated.
- [LOBBY QUEUE AND ROSTER SELECTION] The lobby and the store share one cohort policy (`fits_cohort`/`same_cohort`, career-store/src/matchmaking.rs:19-34; `validate_current_cohort`, career-store/src/career_store.rs:1199-1211), and the test `a_mixed_party_never_yields_a_rated_roster_the_store_rejects` (match_service.rs:1201-1264) proves the lobby never forms a Ranked roster the store would reject.
- [LOBBY QUEUE AND ROSTER SELECTION] Team split is exhaustive and deterministic: parties stay together, head counts as even as possible, then minimum rating-sum difference over all 2^units masks (`split_teams`, match_service.rs:801-862). The career-store Matchmaker uses an exact subset-sum DP for the same purpose (matchmaking.rs:324-358).
- [LOBBY QUEUE AND ROSTER SELECTION] Ordering is FIFO by a monotonic `order` and never changes compatibility; a Ranked roster is always taken before a Casual one, so a Ranked player who took the Casual offer cannot be stolen by Casual while a full Ranked roster exists (match_service.rs:877-928, test 1266-1346). The original300-spread baseline is superseded by the verified300→600 age allowance/cohort merging in finding16; preserve the current shared policy.
- [LOBBY QUEUE AND ROSTER SELECTION] Queue intent is renewed only by explicit signed FindMatch retries (client every 2 s, client/src/match_service.rs:386-399) and expires 60 s after the last one (C3 continuation; originally 15 s) (match_service.rs:668-670), so dead clients leave the queue quickly without extra heartbeat state; identical re-requests refresh `touched`/party/also_casual in place without changing the queue position (match_service.rs:386-402).
- [LOBBY QUEUE AND ROSTER SELECTION] Re-queue races are handled: a FindMatch bound to a terminal or settled allocation is pruned and never rebinds (match_service.rs:365-384, test 1606-1668); a running match the profile left is shown as Resume instead of a fresh handoff (LR-13, view 479-537).
- [LOBBY QUEUE AND ROSTER SELECTION] The searching view carries humans/needed, elapsed, bot-fill countdown, the whole-queue `searching` count, the Casual offer after 300 s and the arena line with ETA (`waiting_view` 683-754, shared/src/match_service.rs:52-78), all localized in Russian (client/i18n/ru/searching.json, ru/queue.json); the Ranked mixed-party dead end is reported immediately as `ranked_party_cohort` instead of waiting forever (match_service.rs:694-700).
- [ALLOCATION, WORKER SPAWN AND MATCH START (lobby → worker handoff, join deadline, draft/lock/load, durable start, cancel, crash/recovery, port reuse, throughput)] Handoff contract: the client learns the worker address only from the lobby's career view (`MatchServiceView::Assigned { allocation.endpoint }`, server/src/match_service.rs:503-516) once the worker's own disk receipt is fresh (`Slot::ready_status`, server/src/match_pool.rs:365-373); the client validates the address before dialing (client/src/match_service.rs:374-379). A settling or terminal worker is never handed out again (match_service.rs:366-387, test at 1606).
- [ALLOCATION, WORKER SPAWN AND MATCH START (lobby → worker handoff, join deadline, draft/lock/load, durable start, cancel, crash/recovery, port reuse, throughput)] Manifest validation is strict and tested: Ranked = exactly 10 humans, never bot-filled; ≤5 per team; vs Bots is one party on one team; duplicate profiles/sessions rejected (server/src/match_allocation.rs:42-73). The worker mode comes from the queue, not the head count (LR-08, match_allocation.rs:78-84), and the lobby reports `rated` from the queue.
- [ALLOCATION, WORKER SPAWN AND MATCH START (lobby → worker handoff, join deadline, draft/lock/load, durable start, cancel, crash/recovery, port reuse, throughput)] Receipts are atomic (tmp + fsync + rename + dir fsync, match_allocation.rs:131-149); one coordinator per match root via an advisory lock released even on crash (match_pool.rs:424-439, Drop at 317-326); a restarted lobby re-adopts live workers and rebuilds `assignments` from their manifests (match_pool.rs:129-161, match_service.rs:293-307).
- [ALLOCATION, WORKER SPAWN AND MATCH START (lobby → worker handoff, join deadline, draft/lock/load, durable start, cancel, crash/recovery, port reuse, throughput)] Exposed lobby refuses a loopback/unset `OMOBA_MATCH_PUBLIC_HOST` instead of failing every match (match_pool.rs:55-85, tested 608-660). Worker env hygiene: local overrides (map file, targeting QA, bot level) are stripped, team size 5 and mode are forced from the manifest (match_pool.rs:403-422); online bots are always Elite (match_allocation.rs test 541-565).
- [ALLOCATION, WORKER SPAWN AND MATCH START (lobby → worker handoff, join deadline, draft/lock/load, durable start, cancel, crash/recovery, port reuse, throughput)] Durable start gate: no gameplay, income or spawns until PostgreSQL acknowledges the Start record (server/src/career_runtime.rs:589-722, prematch.rs:176-180); a rejected start (conflicting active assignment / changed ratings) rolls the roster back with an error and blocks a silent retry (career_runtime.rs:1134-1169). The backend retries records every 2 s through a fsynced outbox (career_backend.rs:1682-1790).
- [ALLOCATION, WORKER SPAWN AND MATCH START (lobby → worker handoff, join deadline, draft/lock/load, durable start, cancel, crash/recovery, port reuse, throughput)] Recovery probe never duplicates a live worker socket on stale disk state (match_pool.rs:288-300); every match ends (abandoned reaping after 180 s + 30 s, 45 min cap) and settles `Interrupted` exactly once, then the worker retires and frees its port (server/src/match_lifecycle.rs:77-176).
- [ALLOCATION, WORKER SPAWN AND MATCH START (lobby → worker handoff, join deadline, draft/lock/load, durable start, cancel, crash/recovery, port reuse, throughput)] In-match reconnect/takeover is solid and covered: a timed-out or leaving human's hero moves to an internal bot-controlled seat, the same session reclaims it with full state (gold, hp, recall cancelled), teammates can majority-vote Idle, a late vote cannot idle a later disconnection, and reclaim resets the reaping (server/src/session_recovery.rs; tests match_allocation.rs:597-1191; match_lifecycle.rs:359-380). LR-13 Resume for a player who left a running match is in both lobby and client (match_service.rs:494-516, client/src/match_service.rs:365-381).
- [ALLOCATION, WORKER SPAWN AND MATCH START (lobby → worker handoff, join deadline, draft/lock/load, durable start, cancel, crash/recovery, port reuse, throughput)] Allocation identity, not the endpoint, is the client's boundary, so worker port reuse is safe (client/src/match_service.rs test 905-924); a worker that refuses the seat sends the player back to the lobby (client test 1056).
- [ALLOCATION, WORKER SPAWN AND MATCH START (lobby → worker handoff, join deadline, draft/lock/load, durable start, cancel, crash/recovery, port reuse, throughput)] Per-source abuse limits (one arena per IP for bot-filled matches, practice-abandon cooldown, LR-16) and the capacity line with ETA when every arena is busy (LR-09) are implemented and tested (match_service.rs:175-261, 2052-2177).
- [RATING, ELIGIBILITY AND SETTLEMENT] Elo is correct and tested: team-average expected score, per-player K=48 for the first 20 rated completions then K=32, .round(), per-player clamp 0..5000 with the actual persisted delta reported (career-store/src/matchmaking.rs:375-430); symmetry/upset/bounds covered by tests at matchmaking.rs:687-758 (equal new profiles +24/-24; equal established profiles +16/-16; mixed K is intentionally non-zero-sum).
- [RATING, ELIGIBILITY AND SETTLEMENT] Rated flag is double-guarded. Runtime: rated only when the worker manifest preference is Ranked, no bots, full authenticated roster, balanced teams, approved default map, no dev modifiers (server/src/career_runtime.rs:536-585, 648). Store: validate_rated (ruleset verdant-default-v1, no unrated_reason, all profiles, equal teams, no bots) at start, stage and settle (career-store/src/career_store.rs:553-554, 686, 1341-1370) and the newcomer/300-spread cohort is re-read from the DB under FOR UPDATE at start (career_store.rs:559-562, 1199-1211; test career_store_tests.rs:217-277).
- [RATING, ELIGIBILITY AND SETTLEMENT] Interrupted/Abandoned never rate: runtime strips rated+winner (career_runtime.rs:819-825), store refuses a non-Completed result with winner or rated (career_store.rs:1335-1339); server-recovered matches become Interrupted/unrated/server_interrupted (career_store.rs:661-666; career_backend.rs:1366-1372).
- [RATING, ELIGIBILITY AND SETTLEMENT] Double-credit protection: settle re-reads status under SELECT ... FOR UPDATE and returns the stored receipt if already settled (career_store.rs:636-645); DB trigger makes settled rows and intent immutable (001_career.sql:73-92); outbox drops a second Settle for the same id (career_backend.rs:1586-1589); concurrent tokio::join settle test asserts one credit (career_store_tests.rs:402-477).
- [RATING, ELIGIBILITY AND SETTLEMENT] One active match per profile: career_active_profiles PRIMARY KEY (001_career.sql:52-55) turns a double seat into 23505, which the worker converts to a durable Rejected tombstone and bounces the roster (career_backend.rs:1753-1790).
- [RATING, ELIGIBILITY AND SETTLEMENT] Settlement concurrency: profiles are locked in sorted, deduplicated order in every path (career_store.rs:1150-1172), match row is locked first, serialization/deadlock errors 40001/40P01 are retried 3x (career_store.rs:104-124); lock_timeout 5s / statement_timeout 10s per connection (career_store.rs:234-246).
- [RATING, ELIGIBILITY AND SETTLEMENT] Receipt to clients carries before/after/delta per participant plus refreshed ProfileSummary rows (career_store.rs:736-737; career_backend.rs:1291-1302, 1127-1143); client renders 'after (+delta)' and has RU strings for rated/unrated/my_progress (client/src/career.rs:1172-1177; client/i18n/ru/career.json:11-28).
- [RATING, ELIGIBILITY AND SETTLEMENT] XP values match docs/match-progression.md:144-146,186-195: 150/100 rated, 50/25 public-casual-v1, 0 otherwise, only on Completed (career_store.rs:743-754); rated_matches increments only on rated Completed (career_store.rs:738-742).
- [RATING, ELIGIBILITY AND SETTLEMENT] Outbox replay is sound: Start/Checkpoint left live become an Interrupted settle carrying the allocation, Settle records replay as-is, ensure_allocation adopts an expired lease, recover_expired is scoped by epoch so a new worker never settles another live worker's match (career_backend.rs:1344-1400, 1701-1730, 1800-1827; career_store.rs:812-855).
- [RATING, ELIGIBILITY AND SETTLEMENT] Per-profile rating history exists: portal.player_match_facts stores rating_before/after/delta with index (profile_id, ended_at DESC, result_id DESC) and the Analytics endpoint returns the last 1000 changes (account-api/migrations/001_portal.sql:67-89; account-api/src/read.rs:277-296); the portal has RU copy 'Изменение рейтинга' (mm-web/src/lib/i18n.ts:19).
- [RATING, ELIGIBILITY AND SETTLEMENT] Schema invariants: rating CHECK 0..5000, counters bounded, rated_matches <= matches_played and wins+losses <= matches_played (001_career.sql:5-11).
- [CLIENT PLAYER FLOW] Three queues are clearly labelled in Russian on Home (Обычный PvP / Рейтинговый / Против ботов, home.rs:894-895, i18n/ru/queue.json) and the searching screen carries a queue badge 'Рейтинговый · С рейтингом' / '… · Без рейтинга' (client/src/match_service.rs:281-306, searching.rs refresh_queue).
- [CLIENT PLAYER FLOW] Searching status is rich and localized: humans/needed, elapsed seconds, players searching this queue, bot-fill countdown for Casual, 'без ботов' for Ranked, 'Матч найден · подготовка арены…' / '… подключение к команде…' (client/src/match_service.rs:470-532, i18n/ru/searching.json). en/ru key parity holds for queue, searching, draft, loading, postmatch, career, rejoin, capacity, home, lobby, net, error, moderation (diff empty).
- [CLIENT PLAYER FLOW] Low-population helpers exist and are localized: the Casual-PvP offer on a long Ranked search (CasualOffer, match_service.rs:188-230), capacity line with position/ETA when arenas are busy (capacity_text, capacity.json), one-tap Offline practice while waiting, and moderation refusals with their end time (client/src/moderation.rs:48-58).
- [CLIENT PLAYER FLOW] Draft has a 30 s server timer with auto-lock on expiry (server/src/prematch.rs:124-146; client draft.rs:1224-1238 'ВЫБОР · {seconds}с'); loading shows 'N / 10 игроков готовы · осталось не более X с' (loading.rs:1522-1530) and a 30 s slow hint with Cancel.
- [CLIENT PLAYER FLOW] Result screen shows the local player's rating after the match with signed delta plus XP ('Ваш рейтинг: 1216 (+16) · +150 очков опыта', postmatch.rs:377-389, career.json) and localized reasons when a match is unrated (career.unrated.*). The portal shows a rating history chart with before → after per match (mm-web/src/components/portal.tsx:174-184).
- [CLIENT PLAYER FLOW] Reconnect/resume is solid: a torn-down committed join auto-reconnects quietly for SESSION_RECLAIM_WINDOW=180 s (session.rs:769-781, common/src/balance.rs:80); a running seat can be resumed from Searching or Home ('Вернуться в матч', LR-13) and a durable resume hint survives app restart (net/recovery.rs). Play again re-queues the same lock-in (match_service.rs:124-140, resume_requeue).
- [CLIENT PLAYER FLOW] Server failure codes that are known get full Russian copy (ranked_party_cohort, arena_limit, practice_cooldown; queue.json:14-16), and a refused new profile / ban is explained instead of queued (searching.rs refresh_status).
- [Admission, transport and capacity for ~1000 concurrent players (server + RU ops)] Return-path admission is sound and non-amplifying: probe -> challenge (reply never larger than the probe, server/src/public_transport.rs:184-188) -> proof -> Ed25519-signed commands bound to epoch/match/path nonce/session with a replay counter (public_transport.rs:236-288); bounded tables (512 endpoints, 5 s unproven TTL, 30 s validated TTL, 120 pkt/s per endpoint).
- [Admission, transport and capacity for ~1000 concurrent players (server + RU ops)] At the global endpoint ceiling and the career-client ceiling the lobby answers ServerFull instead of going silent (public_transport.rs:126-137, career_backend.rs:665-668); the client shows the Russian text 'Сервер заполнен. Повторяем попытку…' (client/i18n/ru/capacity.json) and keeps probing with a 10 s hold (client/src/net/session.rs:152,439-445).
- [Admission, transport and capacity for ~1000 concurrent players (server + RU ops)] The receive loop is budgeted (128 packets / 2 ms per pass, server/src/runtime/dispatch.rs:61-66) and the loop runs ~100 passes/s (runtime/mod.rs:34), so one lobby socket nominally takes thousands of datagrams/s. 512 lobby clients at ~1-2 pkt/s each (FindMatch resent every 2 s, client/src/match_service.rs:388; career requests rate-limited to 10/s per client, career_backend.rs:777-782) are far inside the budget; outbound lobby traffic is 1 Hz lobby snapshots (match_service.rs:934) + 4 Hz career views (career_runtime.rs:20). The single UDP socket is NOT the 1000-player bottleneck; the constants below are.
- [Admission, transport and capacity for ~1000 concurrent players (server + RU ops)] Every admission limit is runtime-tunable through OMOBA_LIMIT_* in lobby.env without a rebuild, validated at start, logged once as 'Admission limits: ...' (server/src/admission.rs:117-215; ops/vps/ru/README.md:505-517).
- [Admission, transport and capacity for ~1000 concurrent players (server + RU ops)] Capacity line when arenas are busy: position + ETA + one-tap offline practice in Russian (client/i18n/ru/capacity.json), capacity.json rewritten every 5 s, CAPACITY_METRIC journal lines, monitor alert at a 120 s wait (ops/vps/ru/README.md:338,561-567).
- [Admission, transport and capacity for ~1000 concurrent players (server + RU ops)] Match pool: capacity validated 1-100 and ports to fit u16, port held 35 s after retirement, coordinator file lock prevents two lobbies on one match root, allocation waits for a worker readiness receipt (server/src/match_pool.rs:96-103,170-172,186-198,424). Workers are separate processes (crash isolation).

## Findings and status (#1–#68)

| # | Sev | Cat | Finding | Where | Smallest fix (as reviewed) | Status 2026-11-05 |
|---|---|---|---|---|---|---|
| 1 | blocker | scale | RU host ships 2 arenas: 20 PvP players (or 2 vs-Bots players) is the whole server | `ops/vps/ru/omoba-lobby-ru.service:33` | Short term: raise OMOBA_MATCH_CAPACITY to the measured value (start 24-32) in omoba-lobby-ru.service, widen OMOBA_ARENA_PORTS in nftables-omoba.conf and the ufw rule, lift ru_host.py's 64 cap to 100. Long term: see the horizontal-path finding. | partial — RU kit set to 16 arenas (formula values); LR-09 measurement on the host still open |
| 2 | blocker | scale | RU host capacity is 2 arenas, the pool is hard-capped at 100 and single-host: ~1000 concurrent players cannot be seated | `server/src/match_pool.rs:98` | M: run the existing capacity probe (README 'Arena capacity') on the real host, raise `OMOBA_MATCH_CAPACITY`/`OMOBA_ARENA_PORTS`/MemoryMax/TasksMax accordingly (ru_host.py allows up to 64) and move the lobby to a host sized for the target; document the resulting seat ceiling on the status page. L: a remote worker pool (lobby allocates to agents on other hosts, manifest/status over HTTP instead of the shared directory) | partial — software ceilings raised; one host stays the hard bound (egress, one worker pool); second host = LR-57 |
| 3 | blocker | scale | Every presence heartbeat triggers a full O(N·P) party broadcast to all lobby subscribers | `server/src/party.rs:588` | In handle_party reset `last_sent` only for state-changing commands (not Presence) and even then send the immediate view only to the issuing endpoint (and the invitee for Invite); keep the periodic 500 ms broadcast but build the online list and a `player_id -> party_id` HashMap once per broadcast and reuse them for all viewers; replace the CareerView clone with a cached `HashSet<profile_id>` of friends per client. Add | done — no broadcast on presence; one view context per 500 ms pass |
| 4 | major | bug | After a rejected worker join the client keeps the dead allocation; the next lock-in bypasses the lobby queue | `client/src/frontend/mod.rs:291` | In the join_blocked branch also `session_ui.write(SessionUiCommand::LeaveMatch)` (or call `flow.take_return_to_lobby()` and reconnect to `lobby_addr`) so the shell returns to the lobby transport with a clean MatchServiceClient before showing the notice. | done (client agent) — rejected join returns to the lobby and re-queues |
| 5 | major | bug | One player's Leave/Cancel in the draft aborts the match for all 10 (even in Casual/vs Bots where bots could fill the seat) | `server/src/career_runtime.rs:1096` | M: in the worker's `take_cancelled` path, for non-Ranked manifests drop the cancelling human from the required set (a `released: HashSet<session>` consulted by `allocated_humans_ready` and `authorize_allocated_join`) instead of writing cancel.json, so `fill_practice_bots` seats a bot; write cancel.json only when the manifest is Ranked or when no human remains. | done — exact durable lobby release intent and owning-worker ACK preserve other Casual/vs-Bots seats without rewriting the manifest; frozen/running roster keeps admission and Resume. Ranked FIFO/dodge behavior retains full baseline CI.4 new regressions,502 server tests/strict clippy and real10-human loopback handoff→9 humans+1 bot Running pass. Fresh branch-wide release CI remains required. |
| 6 | major | bug | A live worker that never writes a heartbeat is never killed; its 10 players are stuck on 'подготовка арены…' until lobby restart | `server/src/match_pool.rs:289` | S: in `Pool::poll`, when `child.is_some()` and `status.is_none()` for more than e.g. 20 s after spawn (track `spawned_at` in `Slot`), or when `now > manifest.join_deadline_ms` with a non-terminal status, call `child.kill()`, reap it and write a `Failed` status so the normal error path (`allocation_cancelled_or_expired`) runs. | done — lobby kills a worker with no receipt after 20 s / past its join deadline |
| 7 | major | bug | One player's Cancel (search or draft) aborts the allocated match for all 10, others see a raw code | `server/src/match_service.rs:459` | Minimum: (1) in the lobby, on cancel of an Assigned/Allocating profile remove only that seat and re-queue the remaining humans at the front with their original `order` (match_service.rs cancel + tick) instead of pool.cancel; (2) add ru/en `queue.failed.allocation_cancelled_or_expired` ('Один из игроков вышел. Ищем матч снова…') and `queue.failed.profile_has_another_session`, `already_queued`, `queue_full`; (3) draft  | done — exact durable lobby release intent and owning-worker ACK preserve other Casual/vs-Bots seats without rewriting the manifest; frozen/running roster keeps admission and Resume. Ranked FIFO/dodge behavior retains full baseline CI.4 new regressions,502 server tests/strict clippy and real10-human loopback handoff→9 humans+1 bot Running pass. Fresh branch-wide release CI remains required. |
| 8 | major | bug | Party member's Cancel leaves the rest of the party searching forever with no message | `server/src/match_service.rs:756` | In cancel(): when the cancelled profile has `party: Some((id, n))`, remove every waiting entry with that party id and set `errors[member] = "party_member_left"` for them; add ru/en `queue.failed.party_member_left` ('{name} вышел из поиска. Начните поиск снова.'); alternatively recompute `n` from present members on the next FindMatch so the party searches as 4. | done — party searches on without the member; party_member_left notice |
| 9 | major | bug | arenas_per_ip=1 refuses the second vs-Bots player behind the same NAT with 'Из вашей сети уже идёт матч' | `server/src/match_service.rs:227` | S (env): OMOBA_LIMIT_ARENAS_PER_IP=50 (max 100, admission.rs:185-189) in lobby.env. M (code): key the anti-abuse limit on device key / profile id instead of source IP, keeping IP only as a secondary signal. | done — OMOBA_LIMIT_ARENAS_PER_IP=8 in the RU kit (code keyed on IP unchanged) |
| 10 | major | bug | Launching or accepting an invite while a member is already Searching another queue deadlocks the party | `server/src/match_service.rs:395` | In party_tag (party.rs:627-632) or enqueue, exclude from the party tag a member whose existing Waiting preference differs from the party's launch preference (treat it as solo), or refuse Launch/Accept with a PartyError when a member is currently queued and surface it. Add a match_service test: party {Casual-queued, Ranked-queued} still seats each as before. | done — member in another queue is seated alone |
| 11 | major | gap | No 'Match found / Accept' step: the client jumps straight from Searching to the worker | `client/src/match_service.rs:528` | Add MatchServiceView::ReadyCheck{deadline_secs, accepted, needed} on the lobby before spawning a worker; client searching screen shows 'Матч найден' with a countdown and Принять/Отклонить; declines/timeouts leave the queue, the rest keep their queue age. Keys: searching.ready.title/accept/decline/waiting in ru+en. (LR-32) | done — explicit signed unanimous 15-second check before worker spawn; native RU 852×393 capture verified with countdown and both controls (product-ready-ui-db-confirmed). Mobile feedback is tracked separately in C3. |
| 12 | major | gap | No horizontal path: one baked lobby address, workers only on the lobby's host, directory plan unscheduled | `docs/plans/game-directory.md:3` | M-L minimum viable path: (1) D0 bake a DNS name (play.omoba.ru) instead of the IP; (2) D1 signed directory JSON on the portal listing N lobbies (host, port, weight, status) that all point OMOBA_DATABASE_URL at the same RU PostgreSQL (the career store settles rating per result, so ratings stay consistent); client picks one at start and falls back to the baked name. Each lobby keeps its own match root and coordinator l | partial — P7 scheduled; bounded signed D1 contract, concrete IP targets, compatibility/maintenance/weight policy and offline signer/verifier implemented.197 shared/2 operator and9 independent process/Node checks PASS. Opt-in fixed-scope Account API/portal HTTP delivery also implemented:4 API and7 actual-stack checks PASS; portal257 PASS/3 expected skips. Native anonymous transport/trust/cache also verified:4 HTTP/9 native/9 endpoint cases. Admission coordinator9 new cases and full dev-client1601/0/1, strict clippy/Android checks PASS. Native startup/queue/policy-UI/network binding and compiled trust/fallback configuration are implemented; all11 runtime regressions/full7-job715a8117 CI pass. Explicit public package input, live publishing/key provisioning, shared cross-host discipline, actual cross-host party/Resume and real second-host proof remain open. |
| 13 | major | gap | Arena capacity never measured on the RU host; probe cannot simulate more than 100 clients | `ops/vps/ru/README.md:533` | M: run the probe on the RU host (or an identical VPS) with --limits-from omoba-lobby-ru.service for --preference=bot_practice and a --mix, fill README:545-548, set capacity/cgroup/nftables from it. S: lift the 100-client assert to 1000 and allow --capacity up to 100 with `--mix` so a 100-arena run is possible on a bigger box. | open — LR-09 measurement on the RU host; probe now allows --allow-large up to 1000 clients |
| 14 | major | gap | No end-to-end Ranked match: terminal → settle → rating → history is never exercised on the real lobby+worker binary with PostgreSQL | `scripts/test_public_mvp.mjs:544` | Add `harness/tests/ranked_journey.rs` (#[ignore], OMOBA_HARNESS_DATABASE_URL): (1) migrate with `harness::server::prebuilt("migrate-career")` (admission.rs:238-249); (2) `ServerProcess::spawn_with_env` with the lobby env from registration_journey.rs:221-235 (OMOBA_SERVER_ROLE=lobby, OMOBA_MATCH_MODE=release, OMOBA_MATCH_CAPACITY=1, OMOBA_MATCH_FIRST_PORT, OMOBA_MATCH_BIND_IP/PUBLIC_HOST=127.0.0.1, raised OMOBA_LIMIT_ | done — real ten-client signed AI match completed in 406.115 s on canonical rules; all ten saved results, fresh profiles, ±16 rating, 150/100 XP and exactly one history row verified. Reusable `public_bots_rehearsal.py --require-victory`; no map/clock/damage override. |
| 15 | major | gap | One player's cancel during handoff/draft aborts the whole 10-player match; the other 9 get a raw code and lose their queue place | `server/src/match_service.rs:459` | On cancel of an assigned profile, remove only that human from the manifest (bot-fill the seat for Casual/vs Bots); for Ranked abort the allocation but re-insert the other nine into `waiting` with their original `queued`/`order` and `casual_since`, give the leaver a short requeue cooldown, and add the ru/en/zh `queue.failed.allocation_cancelled_or_expired` text ('Один из игроков покинул матч, ищем снова…'). | done — exact durable lobby release intent and owning-worker ACK preserve other Casual/vs-Bots seats without rewriting the manifest; frozen/running roster keeps admission and Resume. Ranked FIFO/dodge behavior retains full baseline CI.4 new regressions,502 server tests/strict clippy and real10-human loopback handoff→9 humans+1 bot Running pass. Fresh branch-wide release CI remains required. |
| 16 | major | gap | Ranked cohorts never merge or widen: 10 Ranked humans across cohorts never form a match (LR-35) | `server/src/match_service.rs:884` | Add a time-based allowance to the policy: `fits_cohort_with(low, candidate, spread, merge_newcomers)` where spread grows e.g. 300→600 over 3 min and the newcomer bracket merges after 5 min; carry the allowance in the Manifest so the store's validate_current_cohort accepts exactly that roster; and show 'в вашем диапазоне рейтинга: {humans}' on the searching screen. | done — CohortAllowance::after_wait, store accepts the max window; parties stay one base cohort |
| 17 | major | gap | No accept/ready step: one no-show holds 9 players up to 180 s, then all are dropped with no localized reason and no re-queue | `server/src/match_service.rs:632` | Minimal (M): (1) client — when a pre-start allocation ends (worker Failed/transport lost before `match_started_at`), automatically return to the lobby and re-send the last lock-in with the original search (reuse `request_requeue`/`resume_requeue`, client/src/match_service.rs:121-135, 406-435) and show a localized notice 'Игрок не подключился — ищем снова'; (2) server — shorten `join_deadline_ms` to 60 s and show it i | done — re-queue/client auto-search, unanimous 15 s confirmation before spawn, no-show abort/join deadline 60 s |
| 18 | major | gap | Party never forms a unit while a member idles; no 'waiting for X', no timeout, Casual bot-fill promise never fires | `server/src/match_service.rs:771` | Extend MatchServiceView::Waiting with `party_pending: Vec<String>` (nicknames of party members not yet queued, from party_tag) and show 'Ожидание {names}…' on Searching (RU/EN/ZH); after 60 s of pending, drop the missing members from present_size (or auto-cancel the unit with a new Failed code `party_member_missing`, localized) so bot fill/solo search resumes. Unit test: party of 2 with one unqueued member shows the  | done — named missing members, 60 s gathering, FIFO preserved; regression tests |
| 19 | major | gap | A teammate who never loads is never removed: 30 s loop back to draft until everyone is dropped at 180 s | `server/src/prematch.rs:166` | On loading timeout keep the loaded players' locks, detach only the non-loaded humans (Casual: replace with a bot via the existing fill path; Ranked: abort and re-queue the others at the front), and add a localized stage line 'Ожидаем: {names}' built from `players.iter().filter(\|p\| !p.loaded)` (data already in the snapshot). (Part of LR-32/LR-33.) | done — Casual removes only slow loaders and fills bots; Ranked aborts/requeues without full draft reset; 19 prematch tests pass |
| 20 | major | scale | Egress: ~210 kB/s per player in 5v5 means 1000 players need ~1.7 Gbit/s; a 100 Mbit VPS port carries ~60 players | `docs/public-mvp.md:134` | S: record the host's uplink in README and set OMOBA_MATCH_CAPACITY so that arenas x 10 x 250 kB/s stays under 60% of it. L: shrink snapshots (binary/delta encoding, interest management already exists via vision) or lower SNAPSHOT_INTERVAL to 100 ms for mobile builds; measure bytes/s per player (LR-28 input) on the RU host. | partial — protocol16 bounded lossless compression and full7-job CI42139546925 PASS; frozen78070b27 actual1000-playing60s repeat independently verified, p95 received wire39347B/player/s, median complete snapshots8.64Hz. Cadence/latency SLA, sustained WAN/egress and RU uplink proof remain open. |
| 21 | major | scale | Lobby queue hard-capped at 128 waiting; 129th player is refused with a raw code | `server/src/match_service.rs:404` | Make MAX_WAITING configurable (env, default >=1024 on RU) and rewrite select to one pass per queue: Casual = oldest units packed FIFO until 10 (no per-anchor loop); Ranked = sort fitting units by rating once, slide a 300-wide window and take the oldest 10 in it; call refresh_line only from tick. Add a ru/en/zh `queue.failed.queue_full` string that says the queue is full and the search keeps retrying. | done — bounded selection/views/exact line cache and immediate signed controls;64e777be full CI42116356054 passes all7 jobs/18 PG and public stages. Independent artifact/Ranked awards/eligibility/crash recheck PASS. Follow-up actual1000 queue/presence PASS;1000-playing and RU proof remain #50. |
| 22 | major | scale | vs Bots has absolute priority for arenas; with 2 arenas two solo bot players starve all PvP | `server/src/match_service.rs:870` | Order ready rosters by (humans seated desc, oldest order) when the pool is at or near full, or reserve capacity-1 arenas for PvP: allocate vs Bots only if at least one arena would remain free after all ready Ranked/Casual rosters. Update refresh_line ordering and the `a_full_lobby_shows_each_ready_roster_its_place_in_the_line` test accordingly. | done — seated humans first, then age |
| 23 | major | scale | 8 endpoints / 8 career clients per source IP silently lock out the 9th player behind a carrier-grade NAT | `server/src/public_transport.rs:140` | S (env only): set OMOBA_LIMIT_ENDPOINTS_PER_IP=256 in /opt/omoba-ru/lobby.env (max 512, admission.rs:177-181) and document the CGNAT reasoning. S (code): answer the per-IP refusal with the same non-amplifying ServerFull datagram instead of Drop so the client shows 'Сервер заполнен' rather than the GAME_SERVER_ADDR text; replace that net.status.waiting text in production builds with a player-facing line. | done — 256 per IP in the RU kit; per-IP refusal answers server_full |
| 24 | major | scale | A no-show (or a cancel before Ready) holds an arena about 4 minutes; a worker with zero joins never aborts early | `server/src/session.rs:76` | S: in the worker, abort (write Failed) when `booted_at + 45 s` has passed and no allocated human has ever joined (phase still Ready); keep the 180 s deadline only once at least one human is connected. Optionally lower `join_deadline_ms` to 60-90 s. | done — worker nobody reached aborts 60 s after boot |
| 25 | major | ux | Friends are only marked/sorted first after the Friends modal was opened; 16-row cap hides them | `server/src/party.rs:670` | Client: send CareerRequest::Friends once on authenticated connect / OnEnter(Lobby). Server: always include online friends before applying the 16 cap, and add a nickname filter field or 'Пригласить по коду друга' (lookup by handle, already supported by LookupPlayer) to the lobby. | done (party agent) — Friends requested on entering the lobby; friends before the cap |
| 26 | major | ux | Draft is reset for everyone by one slow loader or any roster change; the reasons are English server strings and the join deadline is never shown | `server/src/prematch.rs:168` | M: add an additive `error_code: Option<&'static str>` (e.g. `roster_changed`, `loading_timeout`, `start_timeout`, `selection_frozen`, `durable_start_pending`, `reserved_roster`) next to the free-text `error` in `PrematchSnapshot`/`CareerView`, map codes in client/i18n/{en,ru,zh}/draft.json; on loading timeout keep locks and re-enter Loading (only the unloaded player must re-send `Loaded`) instead of a full draft rese | done — worker join countdown, named loading wait, stable localized reasons and preserved loaded locks |
| 27 | minor | bug | Prematch dodge/loading timeout after the durable Start is persisted as a played 'Abandoned' match for all 10 players | `/Users/wotori/git/ekza/_workspace/worktrees/mm-game/career-store/src/career_store.rs:714` | In finish_career_round, when `self.match_started_at.is_none()` and outcome != Completed set unrated_reason = Some("prematch_abandoned"); in finish_inner skip the matches_played increment for that reason, and exclude such rows from history()/account-api history (`m.result->>'unrated_reason' <> 'prematch_abandoned'`). Add a store test. | done — explicit durable gameplay_started flag; no profile/history/analytics changes for unplayed allocations; real PostgreSQL store/API regressions pass |
| 28 | minor | bug | Interrupted Ranked match is labelled «Не рейтинговые» in history; single-match tag uses the plural filter word | `/Users/wotori/git/ekza/_workspace/worktrees/mm-web/src/components/portal.tsx:108` | Row: `m.rated ? (m.rating ? delta : '—') : w.unratedShort`; tag: add singular keys `ratedOne:'Рейтинговый'`, `unratedOne:'Без рейтинга'` (the game's career.result.rated/unrated wording) and use them at portal.tsx:128. Add a presentation test for rated:true/rating:null. | done — original interrupted-history claim refuted by actual RU Next/Chrome render; history and personal award remain —, detail retains singular Рейтинговый. Focused3-check browser regression and257 portal units PASS; exact proof below. |
| 29 | minor | bug | Greedy unit packing misses feasible party combinations | `server/src/match_service.rs:893` | Within a cohort window, search subsets of units (a cohort has at most ~20 units; DFS over sizes with the `splits` pruning, preferring the lexicographically oldest `order` sequence) for a combination totalling 10 before falling back to the partial roster; add a test with [4,4,3,3,2,2]. | done — exact bounded two-team packing, [4,4,3,3,2,2] Ranked/Casual tests |
| 30 | minor | bug | Spawn failure leaves the roster flapping between Failed and Waiting and leaves a dead allocation directory on disk | `server/src/match_service.rs:653` | S: on allocate error, remove the selected entries from `waiting` (so the error is sticky until the player cancels/retries explicitly) and call `archive()` on the failed directory; add RU text for `worker_spawn_failed`. | done — spawn failure dequeues the roster, archives the directory, 10 s hold |
| 31 | minor | gap | Harness PostgreSQL tests (300-client admission, registration journey) are never run by any automated gate | `.github/workflows/ci.yml:120` | In ci.yml `postgres` job add after the Python step: `cargo build --locked -p server` then `OMOBA_HARNESS_DATABASE_URL=$OMOBA_TEST_DATABASE_URL cargo test --locked -p harness --test admission -- --ignored --test-threads=1` (and the new ranked_journey test). Make admission.rs/registration_journey.rs fall back to OMOBA_TEST_DATABASE_URL when OMOBA_HARNESS_DATABASE_URL is unset, and add a `make test-postgres-harness` target. | done — owner-approved CI gate passes all 13 stages (1097.16 s, e3b0942/run 42005385554), including admission/registration and full Ranked Victory. Artifact inspected: fixture removed and no private-key file uploaded. |
| 32 | minor | gap | No leaderboard / ranking surface anywhere (portal, API, DB) — rating has no social meaning | `/Users/wotori/git/ekza/_workspace/worktrees/mm-game/account-api/src/lib.rs:554` | Minimal LR-47 slice. (1) career-store migration 006_ladder.sql: `CREATE INDEX career_profiles_ladder ON career_profiles(rating DESC, rated_matches DESC, profile_id)`. (2) account-api/src/read.rs: `pub async fn leaderboard(app,&Option<Session>,query)` → top 100: `SELECT p.profile_id,p.nickname,p.rating,p.rated_matches,p.wins,p.losses FROM career_profiles p JOIN portal.profile_settings s USING(profile_id) WHERE s.publi | done — GET /v1/leaderboard, portal page, opt-out (portal schema 11, career schema 6) |
| 33 | minor | gap | No leaderboard: no index on career_profiles(rating), no API/portal endpoint (LR-47) | `/Users/wotori/git/ekza/_workspace/worktrees/mm-game/career-store/migrations/postgres/001_career.sql:5` | Migration 006: `CREATE INDEX career_ladder ON career_profiles(rating DESC, profile_id) WHERE rated_matches >= 20` (plus SCHEMA_VERSIONS bump); account-api GET /leaderboard returning top-100 nickname/rating/rated_matches honouring portal.profile_settings.public_profile_enabled (or nickname-only), and a portal page with RU copy; optionally the client Home shows 'Ваше место #N'. | done — see #32 |
| 34 | minor | gap | Leaver on the winning team gains full rating and 150 XP (LR-33) | `/Users/wotori/git/ekza/_workspace/worktrees/mm-game/career-store/src/career_store.rs:731` | Minimal: in finish_inner, when `participant.disconnected && won` set progression_xp_gained = 0 and clamp the rating delta to <= 0 (keep the loser's normal loss), and surface this in the receipt (client already has career.player.progress_disconnected). Add a career_store_tests case. Full LR-33 (connected-time tracking, abandon counter + cooldown) is a later L. | done — 90s participation/AFK policy, individual leaver loss/no XP, teammate half-loss and exactly-once PostgreSQL cooldowns. 593 PostgreSQL/API/store/server tests and current native tests pass; commit 0db576fc CI 42061098787 is green in all seven jobs and all13 real public journeys. Artifact inspected/hash verified. No production deployment; remake/surrender and remaining P6 stay separate. |
| 35 | minor | gap | Fixed K=32 with no placement/newcomer K (LR-53) | `/Users/wotori/git/ekza/_workspace/worktrees/mm-game/career-store/src/matchmaking.rs:410` | Pass `rated_matches` into rating_changes_for_players and use K=48 while `rated_matches < NEWCOMER_MATCHES` (per player, winner/loser clamped independently as today), update the test expectations and the number in docs/match-progression.md. Tiers/seasons remain LR-53. | done — seasonal K48 for the first20 Ranked awards then32; owner-set tiers,90-day archive/badge/reset operator and shared-input statistics implemented and tested. Schema10 core full7-job/18-stage proof, real PostgreSQL/operator/role checks, c826 native1578 tests and RU/EN badge/notification pixels satisfy LR-53 software acceptance. Production season close remains a separate owner action. |
| 36 | minor | gap | RU guest home explains the modes but never explains the rating itself | `/Users/wotori/git/ekza/_workspace/worktrees/mm-web/src/components/guest-home.tsx:62` | Add a fourth landing section «Рейтинг» (two sentences from match-progression.md:186-195: старт 1000, меняется только в рейтинговых матчах, первые 20 матчей — группа новичков, обычный PvP и боты дают опыт) with ru/zh entries in landing-translations.ts, and assert it in tests/ru-landing.e2e.mjs. | done — RU landing rating section |
| 37 | minor | gap | Newcomer/calibration state is a bare tag with no progress or explanation | `/Users/wotori/git/ekza/_workspace/worktrees/mm-web/src/components/portal.tsx:200` | Render `Новичок · {rated_matches}/{newcomer_matches} рейтинговых матчей` with a title/tooltip «Первые 20 рейтинговых матчей играются среди новичков; рейтинг пока калибруется» on /me and as a one-line notice above the chart on /me/stats; keys in ru/en/zh. No tiers, per spec. | done — Новичок · n/20 with tooltip on /me and /me/stats |
| 38 | minor | gap | The public-lobby probe is manual-only: no Makefile target, no CI job, RU host capacity never measured | `Makefile:56` | Add `make verify-public-mvp` (requires OMOBA_TEST_DATABASE_URL, builds the server, exports HARNESS_SERVER_BIN, runs `--scenario=lifecycle` and `--mix=bot_practice:5,quick:5,humans_only:10 --max-wait-secs=60 --seconds=10`) and a CI step in the `postgres` job that runs the same two with `--seconds=5`. Before launch, run ops/vps/ru/README.md 'Arena capacity' steps 1-4 on the RU host and record CAPACITY_RESULT lines in d | done — make verify-public-mvp + CI step |
| 39 | minor | gap | Rating is not shown on Home before queueing; no tier, rank or leaderboard anywhere in client or portal | `client/src/frontend/home.rs:355` | S: render `profile.rating` and `rated_matches` (with 'Новичок · {n}/20' when < NEWCOMER_MATCHES) on the Home profile plate next to wins/losses (data already in HomeSignature) and on the Ranked button subtitle. M: add a /leaderboard page to the portal from `career_store` ratings (top 100 + 'your place') and a client button opening it via career_web; tiers/seasons remain LR-53. | done — Home rating/current-season calibration/tier, Ranked subtitle and trusted regional ladder link; own/friend/visited badges and seasonal portal ladder/archive are implemented. RU/EN native/profile and browser acceptance PASS; expired Home and coherent personal demotion warning verified. See ../evidence/ranked-2026-11-06/README.md. Final release/P5/P6 hardware and other findings remain separate. |
| 40 | minor | gap | Fill bots cannot join the public signed lobby, so a developer cannot form a 10-human Ranked match on the dev/RU lobby by hand | `harness/src/bot.rs:46` | Give `harness --bin bots` a `--public` mode built on the shared public_client helper (finding 1): generate N throwaway keys, probe/authenticate, FindMatch `humans_only`/`quick`, follow the handoff, auto Lock/Loaded, then hand the running socket to the existing `BotBrain` lane AI. Document `make bots BOTS=9 BOTS_SERVER=127.0.0.1:4000 BOTS_PUBLIC=1`. | done — `bots --public` signs admission/account/gameplay requests, accepts the exact ready roster, follows handoff, locks/loads and plays. Ten distinct authenticated clients in one rated worker verified on loopback/PostgreSQL; reusable rehearsal added. |
| 41 | minor | gap | Worker-crash recovery is proved only for vs Bots; a rated match that loses its worker has no process-level test | `scripts/test_public_mvp.mjs:36` | Relax line 36 to allow `--scenario=recovery --preference=humans_only --clients=10` (each client is already independent) and extend the recovery check loop (505-515) to iterate over all clients' profiles; reuse the existing `careerProgress` snapshot (220). Also allow `--preference=quick` so the 30 s bot fill path is covered. | done — ten signed Ranked humans, worker killed 2.10 s after Running; durable played interruption, all ten profile/history checks and fresh requeue pass. Initial allocation rated=true, canonical ruleset, zero bots confirmed in PostgreSQL. |
| 42 | minor | gap | Allocated workers cannot be brought to a legitimate Victory in a test: no sanctioned accelerator survives the worker env scrub | `server/src/match_pool.rs:418` | Add a loopback-only passthrough: when `OMOBA_MATCH_BIND_IP` is loopback (match_pool.rs already distinguishes exposed vs loopback lobbies, test `exposed_lobby_refuses_a_loopback_or_unset_public_worker_host` 608), forward `OMOBA_MATCH_TEST_MAP_CONFIG` to the worker as `OMOBA_MAP_CONFIG`; an exposed lobby ignores it and logs a warning. Add a unit test next to 517 asserting an exposed lobby still strips it. The harness t | done — explicit test-map forwarding requires both lobby and worker loopback binds; IPv4/IPv6/exposed guard tests pass. Ten signed humans completed a custom-map Victory in 33.629 s with zero Elo/XP and saved history; full probe wall time 50.87 s. |
| 43 | minor | gap | Party Ranked and leave-during-draft have unit tests only; no process-level path exists and the probe cannot form a party | `server/src/match_service.rs:1348` | In the shared `public_client` helper from finding 1 add `invite/accept` via the existing social CareerActions so a 5-stack can queue; add a `--party=5` option to the probe; add one harness scenario where peer #3 stops sending during Draft and assert the other 9 see `PrematchPhase::Draft` restart (today) or the LR-32 re-queue once it lands. | done — signed five-stack plus solos, explicit Leave and silent disconnect, all nine receive the return code, FIFO/group retained, all ten career/history unchanged, only explicit leaver cooled once, fresh unanimous Ranked roster reaches Running. Process evidence below. |
| 44 | minor | gap | No invite throttle, block or cooldown; a stranger can keep an invite toast on Home | `server/src/party.rs:254` | In PartyState keep `declined: HashMap<(from,to), Instant>` and refuse re-invites for 60 s after a decline, plus at most 10 invites per minute per sender (new PartyError codes surfaced via the error field above). Full block list stays LR-39. | done — decline cooldown 60 s, 10 invites per minute |
| 45 | minor | gap | Parties and queue vanish silently on every lobby restart/deploy | `server/src/party.rs:5` | Show a one-line notice ('Соединение с лобби было восстановлено, группа распущена') when PartyClient clears a non-empty party on epoch change; persistence of parties (LR-55) stays out of this iteration. | done — global one-shot warning verified on actual loopback lobby restarts: both native party clients on Home see group dissolved; solo Searching sees distinct search restarted wording. Real party/profile/toast observations and inspected RU852×393 images committed; current full CI remains an independent release gate. |
| 46 | minor | scale | Settlement needs one Postgres connection per arena; 100 arenas exceed PostgreSQL's default max_connections | `/Users/wotori/git/ekza/_workspace/worktrees/mm-game/career-store/src/career_store.rs:1213` | In ops/vps/ru/ru_host.py bootstrap write `max_connections = 256` (and `shared_buffers` accordingly) to a conf.d drop-in, document it in the README 'Arena capacity' table as capacity + 32, and make ru_host.py refuse a kit whose OMOBA_MATCH_CAPACITY + 32 exceeds the configured max_connections. | done — ru_host.py checks max_connections ≥ capacity + 32 (not written by the kit) |
| 47 | minor | scale | Portal polls /v1/me every 5 s unconditionally and /v1/me/matches every 5 s; each call writes to Postgres | `/Users/wotori/git/ekza/_workspace/worktrees/mm-web/src/components/portal.tsx:278` | Change portal.tsx:278 to 30 s and gate on `document.visibilityState==='visible'` (as useData already does); keep 5 s for `me/matches` only while `pending_results>0` or for 60 s after a focus event; in accounts::settings replace the unconditional INSERT with SELECT-then-INSERT-on-miss. Optionally serve history from portal.player_match_facts (facts_profile_date index) once avatar is added to the facts table. | done — 30 s visible-only polling |
| 48 | minor | scale | nftables arena rule is 1200 pkt/s per source across the whole arena range: ~48 in-match players behind one NAT IP, then kernel drops for all of them | `ops/vps/ru/nftables-omoba.conf:58` | S: key the meter on `ip saddr . udp dport` (per source per arena) or raise the arena rate to 6000/s burst 12000; keep the per-endpoint 120 pkt/s game limit as the real anti-flood bound. | done — arena rule 6000/s burst 12000 (saddr.dport keying left as the next step) |
| 49 | minor | scale | Lobby cgroup (700 MB / 150% CPU / TasksMax 128) caps the host at ~40 arenas by memory and ~62 by threads | `ops/vps/ru/omoba-lobby-ru.service:45` | After the LR-09 measurement set MemoryMax to ~(16 MiB x arenas + 200 MiB), TasksMax >= 3 x arenas + 16, CPUQuota=190% on the 2-vCPU host; document the formula in README 'Arena capacity'. For >40 arenas buy a bigger host or add hosts. | done — formula values 450M/500M/190%/96 for 16 arenas, pending measurement |
| 50 | minor | scale | Nothing tests beyond 100 players / 20 arenas; the 1000-player target has no load evidence at all | `scripts/test_public_mvp.mjs:60` | Lift the probe caps to 1000 clients / 128 arenas behind an explicit `--allow-large` flag, add a `--queue-only` mode (sign in + FindMatch, no handoff) so 1000 lobby clients can be driven from one Mac without 100 workers, and record one run at 512+1 clients that asserts the documented `server_full` reply; run `--clients=100 --preference=bot_practice --capacity=100` on the dev Mac to get a per-arena CPU/RSS figure for t | partial — queue1000/presence and2048+1 refusal PASS. Frozen78070b27 repeat:1000 unique signed native AI peers,100 ten-human Casual arenas, zero server bots and60s all-playing PASS; hashes/receipts/cleanup independently verified. Server3.36 cores/1546MiB, received median8.64Hz/p95 wire39347B/s. Full current CI, sustained cadence/latency and RU-host proof remain required. |
| 51 | minor | scale | Per-IP admission defaults (8 endpoints, 1 arena) will refuse players behind RU carrier-grade NAT | `server/src/admission.rs:104` | Set OMOBA_LIMIT_ENDPOINTS_PER_IP=64 and OMOBA_LIMIT_ARENAS_PER_IP=4 (and OMOBA_LIMIT_NEW_PROFILES_PER_IP higher) in ops/vps/ru/omoba-lobby-ru.service, document the NAT rationale in ops/vps/ru/README.md, and key the arena limit on profile/device rather than IP alone in a later step. | done — RU lobby.env limits |
| 52 | minor | scale | Probe budgets (8/s per IP, 32/s per /24, 64/s global) stretch mass reconnects to tens of seconds | `server/src/admission.rs:99` | S (env): OMOBA_LIMIT_PROBES_GLOBAL=512, OMOBA_LIMIT_PROBES_PER_NETWORK=256, OMOBA_LIMIT_PROBES_PER_IP=64, OMOBA_LIMIT_PROBES_NEW_RESERVE=128 in lobby.env. | done — probe budgets in the RU kit |
| 53 | minor | scale | Port stays reserved ~65 s after a match ends even when the lobby itself reaped the worker | `server/src/match_pool.rs:346` | S: when `poll()` observed the child exit (`exited`) and the slot is terminal, treat the port as free at once (keep the 35 s age rule only for slots with `child == None` that were adopted at startup). | done — observed-exited terminal ports release immediately; live/adopted safeguards,101 rounds/restart and623 real PG tests pass. Full7-job/18-stage CI42097650577 PASS; ten-human capacity1 crash reuses the same port in2.45s. |
| 54 | minor | scale | Every career view re-reads status.json and stats cancel.json from disk for each assigned player (4 Hz per player) | `server/src/match_pool.rs:369` | S: have `view()` use the cached `slot.status` (refreshed by `poll()` and explicitly by `enqueue`), and cache `cancelled` in `Slot` (set when `cancel()` writes the file, refreshed in `refresh_status`). | done — shared worker status/cancellation snapshots, no per-view file I/O and monotonic observed settlement.112 runtime tests, signed lifecycle and full7-job/18-stage CI42097650577 PASS. |
| 55 | minor | scale | After 512 recent FindMatch profiles new searches are dropped silently; player sees 'Подключение…' forever | `server/src/match_service.rs:357` | Drop the 512 cap or raise it with MAX_CLIENTS (career_backend.rs:28), and when it is hit insert an error code (`lobby_busy`) so the view becomes Failed instead of Idle; prune `requests` entries that are neither waiting nor assigned after 30 s instead of 120 s. | done — lobby_busy, 4096 recent requests, 30 s retention |
| 56 | minor | scale | Lobby hard caps (128 waiting, 512 requests) are far below 1000 concurrent and fail silently or with a raw code | `server/src/match_service.rs:17` | Raise MAX_WAITING and the requests cap to configurable values sized for the target (e.g. 2048 / 4096, env OMOBA_LOBBY_MAX_WAITING), make a dropped request answer `Failed{"lobby_busy"}`, and add ru/en `queue.failed.queue_full` / `queue.failed.lobby_busy` ('Очередь переполнена. Повторяем попытку…'). Capacity of 2 arenas is a separate (server) dimension. | done — see #21/#55 |
| 57 | minor | scale | 512 endpoints and 512 career clients per lobby: a lobby restart with 1000 online players puts 488 of them on 'Сервер заполнен' | `server/src/public_transport.rs:16` | S: raise MAX_ENDPOINTS and MAX_CLIENTS to 2048 (or read OMOBA_LIMIT_ENDPOINTS from env like the other limits), and update the two numbers in docs/public-mvp.md:148,173. | done — 2048 endpoints and career clients |
| 58 | minor | ux | Unrated reasons and rejected-start errors reach Russian players as English prose | `/Users/wotori/git/ekza/_workspace/worktrees/mm-game/server/src/career_runtime.rs:823` | Emit snake_case codes (`not_completed`, `roster_incomplete`, `teams_unbalanced`, `custom_map`, `dev_modifiers`, `casual_only`) from career_runtime.rs instead of sentences, add `career.unrated.<code>` entries to client/i18n/{ru,en,zh}/career.json and a `queue.error.cohort_changed` code for the rejected start; keep the English sentence only as the `error` fallback. | done — not_completed code; client text |
| 59 | minor | ux | Match page prints raw unrated_reason codes (casual_queue, allocated_bots, server_interrupted) to Russian players | `/Users/wotori/git/ekza/_workspace/worktrees/mm-web/src/components/portal.tsx:130` | In portal.tsx add `unratedReason(code,w)` mapping casual_queue→w.unratedCasual («Матчи в обычном PvP не влияют на рейтинг»), allocated_bots→w.unratedBots («На свободные места были добавлены боты»), server_interrupted→w.unratedInterrupted («Сервер прервал матч до завершения»), anything else→w.unratedOther («Матч завершился нештатно»); never print the raw string. Add the three keys to ru/en/zh in i18n.ts (copy from cli | done — portal maps the codes to sentences |
| 60 | minor | ux | Portal says «MMR» and «XP» where the game and the dictionary say «Рейтинг» and «очков опыта» | `/Users/wotori/git/ekza/_workspace/worktrees/mm-web/src/components/portal.tsx:108` | Replace the hard-coded 'MMR'/'XP' literals with dictionary keys: `w.rating` («Рейтинг») for the kicker and Metric label, `w.ratingDelta` for rows (`+12 рейтинга` or just `+12` with the «Рейтинг» column caption), and `w.xp` («опыт»/«XP» per locale, ru «очков опыта»). Extend tests/portal-presentation.test.mjs 'new copy is complete in every supported language' to the new keys. | done — Рейтинг / опыт instead of MMR / XP |
| 61 | minor | ux | Class filter and class presentation know only 4 of 17 hero classes | `/Users/wotori/git/ekza/_workspace/worktrees/mm-web/src/components/portal.tsx:84` | API: validate hero_class against a lowercase list generated from HeroClass::ALL (shared) instead of the literal match (read.rs:74). Portal: build the filter list from a shared JSON/const of the 17 class ids with ru/zh names (client/i18n already has them), and extend CLASS_ART with the missing portraits or a per-class fallback glyph; update tests/portal-presentation.test.mjs:54 allowlist test. | done — 17 classes in the API filter and the portal |
| 62 | minor | ux | Russian empty-history copy claims bot matches are not recorded, but vs Bots is | `/Users/wotori/git/ekza/_workspace/worktrees/mm-web/src/lib/i18n.ts:6` | ru: «Сохранённых матчей пока нет. Офлайн-тренировка не записывается в карьеру; матчи на сервере появятся здесь после завершения.» | done — empty-history copy |
| 63 | minor | ux | Production RU client shows a developer message when the server does not answer | `client/i18n/ru/net.json:4` | S: replace with a player-facing line ('Сервер не отвечает. Проверьте интернет; повторяем попытку…') and keep the technical line only when the build has the developer server picker. | done (client agent) — player-facing connection line |
| 64 | minor | ux | Lobby failure codes without Russian text are shown raw with a misleading instruction | `client/i18n/ru/queue.json:14` | Add queue.failed.{queue_full, already_queued, allocation_cancelled_or_expired, profile_has_another_session, capacity_busy, worker_spawn_failed} to ru/en/zh-Hans with accurate wording (retrying vs. needs action), and extend the `status_text_follows_the_language` test to assert every server code constant has a key. | done (client agent) — every lobby code has ru/en/zh text + test |
| 65 | minor | ux | Hero is picked twice: HeroSelect before Searching, then again in Draft | `client/src/frontend/home.rs:1318` | For public-lobby queues skip HeroSelect: build the JoinPrematch from the profile card's `main_class`/current avatar (ProfileCard already on Home, home.rs:1131-1184) and go straight to Searching; keep HeroSelect for Offline practice. | done — real RU native Home→Searching→15s unanimous ready→one Draft verified for solo and two-client party through actual loopback lobby/worker/PostgreSQL; ten human seats, party kept on one side. Functional/source regressions pass; full branch release gates and separately recorded layout defects remain open. |
| 66 | minor | ux | Worker dies during handoff/draft: player sees only 'Нет связи · Переподключение…' for up to 3 minutes | `client/src/net/session.rs:850` | While `flow.allocation.is_some()` and the match has not started, keep polling the lobby view in parallel (FindMatch with the same request_id is already idempotent) and, on `Failed{allocation_cancelled_or_expired}` or a terminal slot, run LeaveMatch + show 'Сервер матча недоступен. Ищем матч снова…' and re-queue automatically; or shorten the prematch reconnect window to ~20 s. | done (client agent) — 20 s pre-start window, then re-queue |
| 67 | minor | ux | Party command errors are only logged on the server; the player sees nothing | `server/src/party.rs:584` | Add `#[serde(default)] error: Option<String>` (stable code) to PartyView, set it for the issuing viewer on the immediate reply, and add `party.error.<code>` strings in ru/en/zh (e.g. 'Игрок уже не в сети', 'Группа заполнена', 'Приглашение истекло'); render it under the online list for ~5 s. | done — PartyView.error codes localized |
| 68 | minor | ux | Mate in a match shows 'Отошёл' on the public lobby instead of 'В матче' | `server/src/party.rs:652` | In send_party_views on a lobby, build `in_match` from the match service: profiles with a live allocation (lobby.assignments) mapped to member profile_ids; keep `away` for members with no presence and no allocation. | done — in_match from the lobby allocations |

## Critic items (missed by the eight reviewers)

- [major/bug/M] **Lobby crash or deploy SIGTERMs every live match worker; workers have no signal handler, so all running (incl. Ranked) matches die and settle as unrated 'server_interrupted'** — `ops/vps/ru/omoba-lobby-ru.service:42`. Fix: Set `KillMode=process` (and `SendSIGHUP=no`) in omoba-lobby-ru.service and the ru_host.py unit rules so only the lobby PID is stopped and workers are adopted on restart (the code path already exists); add a SIGTERM handler in the worker that writes a checkpoint and settles Interrupted gracefully for the real host-shutdown case; add a release test that restarts the lobby unit under a running bot ma — **done in kit — KillMode=process preserves live workers; SIGTERM/SIGINT handler fsyncs terminal records, forces final status publication, process rehearsal passes (Interrupted, no Elo/XP, one played match). RU rollout remains pending.**
- [major/gap/M] **No drain/maintenance mode: deploy is refused while any match runs, so at ~1000 concurrent players the server can never be updated or hot-fixed** — `ops/vps/ru/ru_host.py:1562`. Fix: Add a lobby drain flag (e.g. `OMOBA_MATCH_ROOT/drain` file or SIGUSR1) that refuses new allocations with a localized `searching.match.draining` ('Сервер готовится к обновлению · новые матчи скоро возобновятся') while live matches finish; add `ru_host.py ship --drain` that sets the flag, waits for `occupied == 0` (bounded by OMOBA_MATCH_MAX_DURATION_SECS), then restarts; keep `guard_no_matches` as  — **partial — runtime pauses new ready/allocation, preserves queued parties/FIFO without dodges and publishes operation acknowledgement.6 new server/656 real PG checks, native retry/localization and signed live/restart/adoption/Resume proof PASS. Host ship/wait/cancel and rollback now have a tested review-only candidate (28 new/184 combined tests,1 expected skip), composed with unapplied C4. Owner approval, integration/full CI and actual RU rollout remain required.**
- [major/ux/M] **Mobile player who locks the screen while Searching silently loses the queue place after 15 s and is re-queued at the back on resume; no keep-awake and no match-found sound/vibration** — `server/src/match_service.rs:668`. Fix: Keep the screen awake while in Searching/Draft on Android (`ANativeActivity_setWindowFlags(AWINDOW_FLAG_KEEP_SCREEN_ON)` via the ndk crate) and iOS (`isIdleTimerDisabled`); raise the lobby expiry to 60 s and have the client, on resume with a gap >15 s, show `searching.status.resumed` ('Поиск был прерван · продолжаем'); play a short SFX + vibration when the view becomes allocated/assigned. — **partial — FIFO intent60s, foreground keep-awake/resume refresh, ready sound and approved iOS/Android haptics implemented. Android compile and all7-job CI42097650577 PASS with the approved VIBRATE/JNI patch; current native/mobile visual and physical keep-awake/haptic verification remain pending.**
- [major/scale/M] **Daily pg_dump only: a host failure rolls every player's rating and history back up to 24 h; no WAL archiving or point-in-time recovery** — `ops/vps/ru/omoba-backup-ru.timer:7`. Fix: Enable `archive_mode=on` with an `archive_command` that ships WAL segments to the existing off-host backup target (pgBackRest or wal-g; or rsync as a stopgap), keep the nightly base dump, and extend `ru_backup.py check`/the restore drill to replay WAL to the latest point; as a one-line stopgap raise the dump cadence to hourly. — **open — production kit still uses daily pg_dump. Isolated PG18 native physical-base/WAL proof passes5 checks; real encrypted pgBackRest/SFTP named-point/latest, missing/corrupt-WAL, wrong-pin and outage proof passes7 checks at career10/portal11 in two local containers. Current-schema13/portal11 combined candidate rehearsal now passes20 checks on each of PG16.15/18.6, including exact consent/claims/discipline and restored actual grants. The tested C4 integration patch remains review-only, awaiting its existing decision. Raised-limit PG18 actual-worker recovery now passes22 stages; a defect in the original hardcoded20-connection drill is corrected in the revised review-only C4 parameter proposal. Current composed candidate176 pass/1 skip. PG16 raised-limit/current-worker now also passes22 stages. PG16 actual terminal/duplicate replay now also passes22 stages; only saved changes in the entire terminal payload. Approved independent-storage/RU rollout and integration remain required.**
- [major/gap/L] **No remake or surrender** — original finding at `server/src/session_recovery.rs:200`. Server-owned remake before180s after90s absence; strict majority of remaining humans, surrender from600s with4/5 (all when fewer remain),30s votes/120s failed cooldown. Innocent remakes are neutral; proved ranked leavers retain their individual penalty. — **done in software — server/store/ten-human product-controller checks and inspected RU852×393 menu,44px vote prompts, both saved results and real first-guide dismissal are verified. Corrected full CI42243761726 at6a61a7ac is SUCCESS across all7 jobs, including PostgreSQL/public journeys, full client, core, Android and gameplay harness. Exact native frame/source evidence remains in ../evidence/fair-play-2026-11-06/offscreen-manifest.json. Physical-phone and final current-revision release gates remain separate.**
- [major/gap/S] **Ranked has no eligibility gate** — original finding at `server/src/match_service.rs:892`. Owner decision2026-11-06: ten completed public matches, including Casual/vs Bots; no mandatory VK. Separate authoritative `completed_public_matches` excludes interruptions, remakes, custom/offline practice and own proved leaves/AFK. Fresh search and locked allocation admission; Home progress; existing seats still resume. — **done in software — ten-public-match/noVK gate passes locked admission and real tenth-completion proofs, including current5aa all7-job/18-stage CI42132894282. Fresh protocol16 native RU852×393 Home0 shows disabled Ranked/progress; Home10 shows enabled Ranked/rating. Initial clipped caption was corrected and both actual pixel captures rechecked. Production rollout remains separate.**
- [minor/ux/S] **Draft allows two players on one team to lock the same hero class; no per-team uniqueness or 'taken' feedback** — `server/src/prematch.rs:235`. Fix: In `handle_prematch` refuse `Select`/`Lock` whose `hero_class` is already locked by a joined teammate, setting `player.draft.error = Some("draft.error.hero_taken")` resolved client-side to 'Этого героя уже выбрал союзник'; grey the class card in the client draft when a teammate has locked it (draft snapshot already carries teammates' picks). — **done in software — authoritative Select/Lock/async/timeout enforcement, signed ten-human draft-to-play,13 native regressions and full CI are verified. Native f2214c66 RU852×393 now visibly shows Warrior Taken/disabled90×44 and available132×44 lock-in; exact PNG/hash/readback preserved in ../evidence/fair-play-2026-11-06. Class feedback only; roster loading/physical devices and other findings remain separate.**
- [minor/scale/S] **nftables lobby rule is 400 pkt/s per source IP: ~250 players behind one carrier-grade NAT saturate it and the kernel drops FindMatch/presence for all of them** — `ops/vps/ru/nftables-omoba.conf:56`. Fix: Raise the lobby limit to e.g. 4000/s burst 8000 (it is a flood guard, not a fairness tool) or key the dynamic set by `ip saddr . udp sport` so each NAT'd client gets its own bucket; document the new figure in README 'Admission limits'. — **done — nftables lobby rule 4000/s burst 8000**
- [minor/gap/S] **No alert when match results pile up unsettled (PostgreSQL slow/down): players sit on 'Сохранение результата…' and nobody is paged** — `ops/vps/ru/ru_monitor.py:435`. Fix: In `ru_monitor.py` add `settlement_problems`: count `*.json` receipts older than N minutes under `/var/lib/omoba-ru/matches/*/` worker outboxes and the lobby outbox, alert when > 0 for 5 min; optionally have the lobby write `pending_settlements` into capacity.json so the existing report path carries it. — **done in kit — bounded terminal-outbox scan, >=5 min threshold anchored to original result time; identical retries preserve file age. Typed queue-dodge journal checks cover stale/corrupt/capacity-limited delivery with no player IDs. All42 monitor cases, actual native report interoperability and real-PG schema/IO failure/recovery PASS. Required anonymous process health covers pre-persistence SQL/role/journal/floor/result blockers. Controlled-load latency and RU installation remain release gates.**

## Next steps, in order

1. Complete the current game/portal PR gates into `dev`: `make check`,
   `scripts/postgres_tests.py`, full public journeys and portal checks. The owner
   permits targeted local commits with the full gate in CI while root disk is
   below20GB; a prior head's green job does not certify the current source.
2. Verify controlled SQL/heartbeat/backpressure/cadence under the full target load
   and paired health-v2 rollout. Direct blocking Ready/Flush sends are removed;
   bounded full-receiver/budget, unchanged generations, actual lease/health
   liveness and late-completion/canonical-admission checks PASS. This does not
   establish1000-player sustained latency or RU capacity. Preserve authoritative
   recovery records and never treat missing ready rows as standalone admission.
3. Prove independent off-host/RU recovery and production RPO/RTO with the approved
   final host kit. Local PG16/PG18 physical replacement/Resume/SQL-checkpoint and
   PG16 actual terminal-file plus real duplicate redelivery now pass separately.
   The latter preserves every payload field except saved and awards one credit.
   Preserve matching admission authority and missing-authority evidence. These
   tests use one local engine, not independent failure domains/RU storage. C4's
   revised numeric-dimension package and C2 integration still await their existing
   owner decisions; older schema10/20-connection evidence stays historical.
4. Finish UI-2 widest-name/UI-3 compatibility current-source captures and C3
   physical keep-awake/haptics. Use the repository UI skill's locked-Mac capture
   workflow and an exact matching native binary; preserve rejected visual evidence.
5. Complete #1/#2/#12/#20/#50 and #13 host acceptance: actual multi-host/WAN,
   party/Resume faults, public trust packaging/provisioning and sustained cadence,
   latency/egress/SQL-heartbeat load. Measure the RU host or an identical approved
   host with the existing probe and derive real capacity/cgroup/firewall limits.
6. Refresh/integrate the concrete C2/C4 host-tool review candidates after their
   existing owner decisions, then perform the owner-controlled RU rollout/recovery
   checks. Match **career12/portal11 and the exact paired protocol16 binaries**;
   do not follow historical career9/10 rollback or deployment snippets below.
7. Audit every original finding, critic, UI/physical and release criterion against
   current authoritative evidence, update the ledger/changelog and obtain the
   owner's release/production action. `main`, releases and production deployment
   remain owner-controlled.


## Codex continuation — 2026-11-05

The previous readiness checkpoint is locally verified from the inherited worktree.
The product-completion checklist below defines the remaining acceptance scope.
Regression fixes and exact evidence are recorded in
[the progress record](../progress/2026-11-05-ranked-matchmaking-readiness.md#codex-continuation--2026-11-05).
In particular: `lobby_busy` request correlation, Ranked priority for shared
players, first-receipt worker-failure re-queue, worker-side draft dodge cooldown,
read-only polling of existing settings, and repeatable database fixtures.
The portal ladder and opt-out passed a Russian 390 × 844 browser check against
a real loopback API; this removes the earlier browser-verification limitation.
RU deployment and capacity measurement remain pending owner authorization.
No open/deferred feature is marked done merely because its supporting
prerequisites passed.


## Additional native UI follow-ups

These are additional to the original68-row ledger. Their scope is retained
separately rather than silently treating the inspected frames as complete UI QA.

| ID | Status | Acceptance / remaining work |
|---|---|---|
| UI-1 | done | Home nickname stays one readable line with intact ellipsis at actual glyph width; rating is contained. Ordinary and maximum-width20-W names inspected at RU852×393. |
| UI-2 | partial | Ordinary Draft names/class/roles no longer overlap in the inspected team. Draft/Lobby still need actual-width fitting or verified widest-name bounds; the initial character estimate undercounts W. |
| UI-3 | open | Compatibility-unavailable error pushes offline-practice progress hint into bottom navigation at RU852×393. Fix this failure-state layout and repeat only this Home state. |


## Product completion — owner instruction 2026-11-05

The owner requires closing the complete 68-finding list and nine critic items,
including items formerly deferred. A prior partial fix is not completion of
the original finding. Work proceeds in this order; each completed item needs
implementation, relevant verification, an updated finding status and CHANGELOG.

- [x] P1 Party collection: pending-member names, bounded gathering (#18), exact party packing (#29).
- [x] P2 Pre-start failure: no career credit/history for unplayed allocations (#27), bounded loading and preserved locks/countdown (#19, #26), Casual replacement seats (#5).
- [x] P3 Ready-check: all players explicitly accept within 15 seconds before allocation (#11, #17), persistent escalating dodge cooldown (LR-32).
- [x] P4 Ranked journey: #14 and #40–43 gameplay proofs plus #31 automation; all 13 complete-gate stages pass in Linux CI.
- [ ] P5 Client: Home rating/placement and ladder link (#39), one hero selection (#65), global reconnect notice (#45), mobile keep-awake and match-found feedback (critic C3).
- [ ] P6 Fair play: AFK/leaver accounting and reward exclusion (#34), remake/surrender (C5), eligibility (C6), unique locked team picks (C7), newcomer K and explicit tier/placement/season rules (#35, #39, LR-53).
- [ ] P7 Scale: bounded queue selection and views (#21), worker retirement/status I/O (#53–54), 1000-client evidence (#50), snapshot traffic (#20), multi-lobby directory/worker capacity (#2, #12).
- [ ] P8 Operations: graceful termination (C1), drain (C2), unsettled-result alert (C9), backup/PITR recovery kit (C4), host capacity proof (#1, #13).
- [ ] P9 Final gates, update every original/critic status, preserve commits/PRs, product acceptance review.

Runtime changes, local test fixtures and repository tooling are authorized.
Actual RU deployment, infrastructure/credentials changes and host load remain
subject to the owner's standing production-approval rule. They cannot be
marked verified based on local tests or formula capacities.

Ready-check rule confirmed by the owner: all members must accept within 15 s.
No worker starts before unanimous acceptance. A refusal/timeout cancels that
ready-check; accepted players resume search with original FIFO age, while
non-accepting players leave the queue.

### Verified continuation evidence

- P1/P2: 415 server tests and 182 shared tests passed (3 PostgreSQL server tests
  remain for the database gate), plus client test-target type checking. Separate
  real-PostgreSQL regressions passed for unplayed store settlement/recovery and
  API history/analytics exclusion.
- P3 network rehearsal: `test_public_mvp.mjs --scenario=ready-check --seconds=2
  --input-hz=0` passed on loopback: nine accepts, one 15 s timeout, zero worker
  spawns before unanimous acceptance, FIFO age retained, stale late accept
  rejected, replacement roster starts exactly one worker. Evidence is in
  `.agent/codex-ranked-readiness/product-ready-check/report.json` (local output).
- Ready/dodge cooldowns are persisted in the locked host's match root. Sharing
  them across multiple lobby hosts remains an explicit P7 acceptance condition.
- Critic C9 monitor implementation: 24 isolated Python tests pass; no alerts
  were sent and no production files were changed.

### Verification limits on this checkpoint

- Full local `make check`: 2486 Rust tests passed (94 deliberately ignored),
  628 Python tests (9 skipped), 62 iOS tooling tests. No device installation.
- Full disposable PostgreSQL gate: 562 tests passed, none ignored; this includes
  the persisted ready cooldown, party opt-out and outbox-barrier regressions.
- The RU 852×393 visual capture has **not passed**. The computer-use service
  reports that the Mac is locked; the capture also exposed nondeterministic
  fixture scheduling, which is being replaced with an explicitly ordered fixture
  and a renderer-free accept/decline regression. No screenshot is claimed.
- C1 signal handling and fsync barrier are implemented and database-tested; the
  process-level SIGTERM rehearsal passed on the current server binary (1.56 s, one Interrupted result, no Elo/XP changes).
- New heavy native builds are stopped while system free space is below 20 GiB;
  cargo caches are retained as required by the umbrella AGENTS.md.

P4 recovery must additionally cover a crash in the first ten seconds after
`Running`, before the first periodic checkpoint: the new gameplay flag must
reach durable recovery state promptly, while aborted drafts retain no credit.
The repeat of the final client ready-screen test was interrupted during native
code generation at ~17 GiB system free space, per AGENTS.md. Its source is kept;
that last test is not recorded as passed. The earlier complete gate above remains
valid for its recorded revision, before the fixture-ordering follow-up.

Final server process verification: the current binary passed ready timeout /
replacement again (`product-ready-check-confirmed/report.json`). The SIGTERM
rehearsal passed (`product-termination-final/report.json`), including a fix for
the final status heartbeat: a successful settlement must publish Failed before
exit rather than leaving a stale Settling receipt that spawns a recovery child.

### P4 continuation: committed gameplay and full Ranked early-crash recovery

- Added a separate acknowledged gameplay checkpoint before combat/income can
  begin; allocation ACK alone cannot enter Running. Reuses the existing durable
  checkpoint format and requires no schema or client-protocol change.
- Explicit unplayed Abandoned/prematch_abandoned cancellation can win an
  activation-ACK race. Ordinary checkpoints and Interrupted recovery cannot
  erase a committed gameplay marker; real PostgreSQL regressions cover both.
- `test_public_mvp.mjs --scenario=early-recovery --preference=humans_only
  --clients=10 --seconds=2 --input-hz=0` passed on the actual server process.
  SIGKILL occurred 2.10 s after Running, before the 10 s periodic checkpoint.
  Every participant has exactly one played/history row, unchanged Elo/XP,
  and can accept a fresh roster after the real >=120 s recovery gate.
  Artifact: `.agent/codex-ranked-readiness/ranked-early-recovery/report.json`.
- Direct PostgreSQL inspection confirms result `05f7cbc06ddf185408b613b685105ebc`
  had initially rated=true, verdant-default-v1, a committed gameplay checkpoint,
  ten authenticated humans and zero bots. Result is Interrupted, never rated.
- The `recovery` probe now accepts ten Ranked humans or a single Casual roster;
  harness clients accept an explicitly requested ready-check independently of
  automatic queue retries, fixing post-recovery/manual-search stalls.
- Current-source server/store tests: 420 + 40 passed with PostgreSQL, plus the
  strengthened activation/cancellation regression. Lifecycle process probe and
  server/store all-target clippy pass. Full API/store/server PostgreSQL gate:
  563 tests passed, none ignored.
- The ready-button regression that could not finish locally passed in the dev
  client CI job for 548d152: 1553 tests passed (one ignored). The unchanged client
  source retains that evidence. Native RU screenshot is still not claimed.
- The main CI runner for 548d152 was shut down externally (exit 143), not a test
  assertion failure; its retry was requested after the first run became terminal.
  The retry completed successfully: every required CI job is green for 548d152.

P4 remains open for the party/draft process scenarios (#43) and the remaining
CI automation in #31. Signed AI clients (#40), real rated Victory (#14), Ranked
early-crash recovery (#41), and loopback terminal tooling (#42) have passing evidence.
P5–P9 and the other open findings retain their full original scope.


### P4 continuation: signed public AI clients

- `harness::Bot::connect_public` is nonblocking across roster creation. The
  client signs only its own auth challenge, signs scoped account/game commands,
  explicitly accepts each ready-check, follows the correlated allocation and
  re-authenticates before draft Lock/Loaded. No debug commands or credentials.
- `make bots BOTS_PUBLIC=1 BOTS=9` fills a public Ranked lobby alongside a human;
  `BOTS_PREFERENCE=quick|bot_practice` selects other queues. Existing local/raw
  bot behavior remains available. Public mode uses standard default cosmetics.
- Ten throwaway identities reached one rated worker with zero server bots,
  selected/loaded, bought items and used normal movement/casts. Initial smoke
  evidence: `.agent/codex-ranked-readiness/signed-ai-bots/report.json`.
- `scripts/public_bots_rehearsal.py` owns its lobby/workers, requires a loopback
  `/omoba_test` database without connection options, rejects reused output roots,
  and records all peer identities despite fog of war. `--require-victory` uses
  the standard map/rules and asserts saved results, XP, rating and history for
  every client. It does not manufacture or accelerate a rated win.
- #14 terminal rehearsal PASS: result `99e131dbd302451ee593eac48cba0ee4`,
  canonical `verdant-default-v1`, ten signed humans, real Victory after 406.115 s.
  All ten fresh profiles/history verified, winners +16/150 XP, losers -16/100 XP,
  exactly one played/rated match each. Total wall time 422.50 s; every AI process
  and owned lobby/worker was stopped. No map/clock/damage/debug override.
  Evidence: `.agent/codex-ranked-readiness/ranked-ai-victory/report.json`.


### P4 continuation: local terminal accelerator (#42)

- The pool forwards `OMOBA_MATCH_TEST_MAP_CONFIG` only after checking both actual
  lobby and worker binds are loopback. Exposed listeners ignore the setting;
  workers still strip ordinary map, QA and bot-difficulty overrides. Worker
  launch also rechecks its own bind. Eight pool tests pass, including the
  IPv4/IPv6/exposed matrix and negative forwarding.
- Explicit `--test-map` copies a fixture into the owned loopback root and selects
  `bots --rush-objectives`: normal signed movement/casts focus structures instead
  of waiting for the strategic AI's midgame rally. Remote CLI use is rejected.
  The first custom run without this policy reached the 300 s test deadline;
  the current objective rehearsal passes in 50.87 s total (33.629 s gameplay).
- Result `fd3dd7e0aaeed5a9d798d95690cf447a` is saved Completed,
  `custom-unrated-v1`, ten humans, zero rating/XP and one history entry per
  profile. PostgreSQL confirms the durable allocation/result are unrated even
  though the lobby queue was Ranked. No fake settlement or server debug input.
  Evidence: `.agent/codex-ranked-readiness/custom-unrated-objective-victory/report.json`.
- Full local gate: 2490 Rust passed /95 deliberately ignored; 634 Python tests
  /9 skips; 62 iOS-tooling tests. Includes the new pool guard and both readiness
  regressions. Full headless harness: 55 passed /2 opt-in ignores. Six rehearsal
  validator tests pass. Final objective CLI follow-up additionally passes
  server/harness all-target clippy and the real signed terminal rehearsal.
- Previous saved head b027ba7 has all CI jobs green. The next pushed head needs
  its own CI; no native RU screenshot or production deployment is claimed.


Final full PostgreSQL validation for this continuation: 564 tests passed, zero
ignores/failures. Final server/harness all-target clippy, formatting and whitespace
checks pass. All 68 findings and all nine critic items remain the owner scope;
#43, the remaining #31 automation and P5–P9 are not closed by these test results.


Final rebuilt objective binary repeat: PASS, 49.05 s total /33.741 s gameplay,
result `e577592f609e17ac2fece91beb6649fc`. Evidence:
`.agent/codex-ranked-readiness/custom-unrated-objective-confirmed/report.json`.
Final workspace/all-target clippy also passes after the CLI follow-up.

Next #43 uses actual signed `ClientPacket::Party` Presence/Invite/Accept commands
and `ServerPacket::Party` views, not `CareerAction::Social` (which is chat and
reactions). Form a five-stack plus five solo peers, hold Draft, make a solo
leave, verify no unplayed career credit, preserve the other nine searches, and
accept a fresh full roster with a replacement. Keep #43 open until this real
lobby/worker process path passes; #31 automation remains separately tracked.


### P4 continuation: real parties and both draft-departure paths (#43)

- `test_public_mvp.mjs --scenario=party-draft --clients=10
  --preference=humans_only --seconds=3` forms a signed five-stack plus five
  solos, holds nine searches for six seconds, accepts the tenth and verifies
  one rated ten-human Draft with the stack together.
- The process test exposed two production-path bugs: gameplay Leave did not
  write a Ranked cancellation (only account CancelQueue did), and aborted
  workers stopped sending the draft error that triggers native automatic
  return. Both are fixed. The deadline path now sets the retirement flag too.
  Three new server regressions cover exact identity/session, idempotent Leave,
  late-Join refusal, running-worker preservation and repeated retirement views
  with no simulation/income.
- Explicit Leave PASS: all nine receive allocation_cancelled_or_expired,
  original ages `[6,6,6,6,6,6,6,6,6,0]` are retained (survivor views 15/9 s),
  all ten fresh profiles/history unchanged, the leaver has exactly one strike
  and no survivor is penalized. The five-stack is restored after lobby return;
  a replacement ten-human roster unanimously accepts and reaches Running.
  Evidence: `.agent/codex-ranked-readiness/party-draft-return-confirmed/report.json`.
- `--draft-exit=disconnect` PASS: close the peer without Leave, exercise the
  actual join deadline, keep survivor ages 70/64 s, deliver all nine return
  signals, leave every penalty map empty, then allow the reconnected player
  into a fresh unanimously accepted Ranked match. All profiles/history remain
  unchanged for the unplayed first worker.
  Evidence: `.agent/codex-ranked-readiness/party-disconnect-return-confirmed/report.json`.
- Full local gate: 2493 Rust passed /95 intentional ignores; 634 Python tests
  /9 skips; 62 iOS-tooling tests. Full PostgreSQL gate: 567 passed, no ignores.
  Current workspace/all-target clippy, formatting and Node syntax pass.

CI head 67683d6 passed unit/platform/client/harness jobs, but its mixed public
probe stalled ten Ranked synthetic peers. CI did not retain client artifacts.
A deterministic old-socket callback test reproduced authentication replacement
in the Node probe (red). Socket-identity guards now reject those callbacks;
17-peer mixed load with `--exercise-stale-handoff` passes (17 injected stale
callbacks, every peer Running, max 41.94 s). Failure logs now carry bounded
client/draft/worker details. This proves the fixed race locally; the next head
still needs its own Linux CI. No unsupported claim about the old CI root cause.
Evidence: `.agent/codex-ranked-readiness/stale-handoff-red/report.json` and
`.agent/codex-ranked-readiness/mixed-stale-handoff-confirmed/report.json`.

#31 automation, P3 native visual evidence and P5–P9 remain in scope and open.


Final combined probe checks also pass with stale callbacks deliberately injected:
`.agent/codex-ranked-readiness/party-final-stale-handoff/report.json` verifies
20 guarded handoffs across the original and replacement party rosters;
`.agent/codex-ranked-readiness/lifecycle-party-followup/report.json` verifies
three guarded handoffs, same-seat reconnect and leave/resume for a running match.
No owned probe processes remain. These are runtime/probe changes; no native
client source, production dependency, schema, protocol or CI configuration changed.

### P6 continuation: participation and durable discipline (in progress)

Contract: `docs/plans/fair-play-2026-11-05.md`. Optional participation preserves
legacy receipt bytes. Protocol 14 distinguishes deliberate movement from AI pose
echoes; short taps remain latched until transmission. AFK takeover keeps the same
authenticated actor and snapshots; meaningful input reclaims it. Grace is 90 s,
with sticky abandon and final aggregate absence accounting.

Schema7 adds incidents, profile abandon totals and rolling 24 h cooldowns of
5min/30min/24h. Settlement and admission share profile locks: repeat results
cannot duplicate punishment, and an old lobby/accepted ready view cannot race
an active cooldown into a new allocation. Queue admission refreshes access
asynchronously, is correlated to sign-in/search sequence, and fails closed.

Proved ranked abandons get no XP and an individual loss, including winners;
innocent losing teammates get half loss. A player-caused unattended shutdown
still penalizes proved abandons; server failure and duration caps do not. Legacy
receipts suppress new disconnected win rewards without inventing incidents or
rewriting an old winner. Native RU/EN/zh-Hans copy explains AFK and cooldowns.

Current P6 verification:

- All 593 PostgreSQL/API/store/server tests passed across 18 suites, zero failed
  or ignored, using only disposable `127.0.0.1:55581/omoba_test`. Includes48
  store tests,440 server tests, real signed UDP gameplay/history, restricted
  runtime-role grants, injected terminal-TX rollback, concurrent/replayed
  settlement, saved rating 1234 through schema 6→7, cross-owner cooldown denial
  and no leaked allocation on refusal. Only this disposable DB was migrated.
- All 183 shared tests, production time-policy 10/10 and tooling 642 (9expected
  skips) pass. Native client/server test targets type-check; full current client
  execution and complete current-process journeys remain pending.
- All 15 AFK runtime cases pass: exact grace, death/respawn, same actor reclaim,
  AI pose echoes, real peer timeout, all-AFK worker reaping and old-round reset.
  Queue refresh correlation/fail-closed and all three queue cooldowns pass.
- CJK glyph subset 462616 bytes retains all old glyphs and adds挂罚; asset budget
  and hashes pass. Full workspace clippy and subsequent client all-target/no-QA clippy pass.

The committed baseline 94311329 has green CI 42024828738: all seven jobs and all 13
complete-journey stages (3194.98s). Standard-map Ranked victory in 2539.88 s,
result 01ccac2e39690152af01c6cbbd3d8161, all ten profiles/history verify +24/150
or-24/100 for fresh profiles. Private fixture removed. Artifact 13417265646,
SHA256`fe5b706ca3d24b59f60631483f27cd3ed8f82bb2e23be1467a6c2be68ff66f4a`
verified; reports `.agent/codex-ranked-readiness/ci-943-postgres-harness/`.
This proves newcomer K 48; tiers/seasons still belong to P6.

#34/P6 remain partial until current native-client execution and complete current journeys pass.
Reviewable local backup: `.agent/codex-ranked-readiness/p6-fair-play-wip.patch`.
No RU deployment or production roles/schema changed. Native debug binaries
built successfully, but locked Mac prevents accepting Home phone screenshots;
the earlier black release capture remains rejected. Root 16GiB is below the
AGENTS 20 GiB heavy-build threshold, so full Bevy/client builds are stopped.
Remake/surrender, unique team picks, eligibility and tier/season rules remain
mandatory, as do P7–P9. Nothing here closes all 68 findings or all 9 critic items.

Additional AFK checkpoint: all 593 PostgreSQL-backed tests pass after fixing
personal result filtering and the forced-end handoff. A proved ranked penalty
in an `abandoned_reaped` interruption is a personal loss/rated history entry;
other players retain no invented winner or reward. API history, rating graph,
profile counters, native history/details/postmatch and portal presentation share
that interpretation. Participation durations are decimal strings in the Account
API; native receipts retain their numeric protocol fields. Server crash recovery
and duration caps remain exempt. The forced reason reaches settlement, including
Casual, without being replaced by a generic interruption or bot-fill label.

Portal tests: 242 passed / 3 expected skips; typecheck passed. Focused real
Next/Chrome render with a synthetic API: RU 390x844 profile and returned-leaver
match report passed. Inspected images show the profile counter, personal loss
with a winning team, and a readable full-width AFK notice. These are browser
fixture images, not physical mobile or real-provider evidence. Files:
`../mm-web/.agent/browser/fair-play/`.

Frozen AFK patches and hashes are kept under each worktree's
`.agent/codex-ranked-readiness/p6-afk-frozen.*`. The owner was asked whether full
CI may follow intermediate commits while root is below the local heavy-build
threshold; that permission is pending. No feature commit/push bypassed that gate.
Next independent P6 implementation is remake/surrender; all P7–P9 still required.

### Saved AFK package and C5 continuation

- Game AFK package: `18389e72`, pushed to PR148; portal: `6d987c2`, pushed to PR33.
  The owner explicitly approved intermediate commits after targeted local checks
  with the complete gate in existing CI. CI run `42056989638` is in progress;
  it is not recorded as passed. The full local gate also runs against an isolated
  source-identical AFK/compact-Home checkout; current C5 changes cannot enter it.
- Unlocked Mac capture: `.agent/codex-ranked-readiness/product-home-native-input14-unlocked/`.
  Actual RU 852×393 Home render shows rating/placement and the direct queues.
  It exposed a version-label/nav overlap. Layout adjusted; fresh changed-source
  screenshot is pending. This is a desktop phone preview with seeded QA data,
  not a physical mobile or authenticated account acceptance result.
- Portal AFK production build passed in addition to 242 tests, typecheck and
  the focused browser scenario. It is saved, not deployed.
- C5 source integration has begun: authenticated per-team vote slots, generation
  and round checks, 30s/120s timing, remake/surrender terminal paths, native menu,
  and settlement/history policy. Protocol 15/core 8 are only working-tree code.
  Runtime/PostgreSQL/native/public-process verification is pending. C5 remains
  open; C6/C7, tiers/seasons and every P7–P9 acceptance condition remain required.

Owner decision 2026-11-06 for C6/O-3: Ranked opens after 10 completed public
matches; linking VK is not required. This explicitly supersedes the old launch
default. Server admission, authoritative completed-match counter, Home progress
and public/PG proof are still required; the decision alone does not close C6.

C5 targeted runtime/policy tests: 12 passed, no failures or ignores. Server/store/
Account API all-target typecheck passed. Portal production build, focused RU
390×844 browser check and 10 presentation tests passed; remake penalty/neutral
and surrender screenshots inspected. C5 real PostgreSQL/public-process/native
checks still pending. AFK notice fix `0db576fc` is pushed; repeat CI `42061098787`
is running after the earlier run found the missing stable QA identifier.

### Current AFK acceptance — 2026-11-06

`0db576fc`: CI `42061098787` passed all seven jobs and all 13 complete public/PG
stages (1136.32 s). Standard-map Ranked base victory took 445.59 s including
setup, with gameplay duration 428402 ms. Result `f04d399b61e8d74b2efbe280b9f26128`: all
ten profiles/history/receipts independently checked, exact +24/150 or -24/100,
participation present with no false AFK/abandons. Artifact `13433365824` SHA256
`8ea9b8407f23a8d90a2925518937bdbe2da13ce5ec197a5c73621435d9494091` verified;
private fixture absent. Reports retained in `.agent/codex-ranked-readiness/ci-0db-postgres-harness/`.
The new AFK notice test passes after its missing TestId was fixed. Finding #34
is closed by current code and checks; this does not close the rest of P6 or
physical-device/host acceptance. Current portal C5 presentation is saved in
`8f6314b`, with 243 tests/3 expected skips, typecheck, production build and
focused RU mobile browser proof passing. Game C5 PG/native/public checks remain.


### C5 public-process proof — 2026-11-06

The working implementation passed 608 real PostgreSQL tests (store50/API105/
server453), 183 shared tests, 12 runtime/policy tests and all-target native clippy.
Both real-time, signed ten-human Ranked process scenarios pass on protocol15/core8:

- Remake: `a66bc505b1e3b6695585a508e639f226`, eligibility after96.209968s.
  All ten profile/history/detail records checked; innocent neutral totals and
  individual -32/no-XP/durable cooldown checked. Stale/opponent ballots rejected.
- Surrender: `dce7261dce0ae4666c1f490676804993`, eligibility after600.183473s.
  Explicit No, exact quorum, stale/opponent rejection and all ten ordinary
  result/profile/history awards checked, without accelerated game clocks.

The first remake probe settled correctly but failed its own unpaced account
reads; phased reads respect the unchanged server throttle and the rerun passes.
Raw artifacts: `.agent/codex-ranked-readiness/p6-public-remake-v2/` and
`p6-public-surrender/`. Portal8f6314b is saved and verified separately.
C5 remains partial until native kit/full committed CI and native visual proof.
The existing PostgreSQL harness now includes15 stages; ordinary base victory
remains an independent stage. C6 is next: owner-approved ten completed public
matches, without VK requirement. All later P6–P9 work remains mandatory.


### C6 implementation checkpoint — 2026-11-06

Owner-approved gate is working-tree code: `completed_public_matches`, core9
historical backfill, exactly-once settlement increment, fresh QueueAccess and
locked new-allocation admission, localized refusal and Home0/9/10 progress.
No mandatory VK requirement or server bypass. Existing allocation replay is exempt.
Three real PG eligibility cases pass (0/9/10, exclusions, upgrade/recovery/replay);
expanded concurrent/backfill coverage and the complete regression gate are running.
Native all-target clippy and13 Python tool cases pass. New guarded native build
runs on cacheB; general headless checks useA, both on Orico.

Public test setup explicitly distinguishes synthetic prior Casual receipts from
real gameplay. A loopback-omoba_test-only store example prepares prior results;
no server admission override is introduced. A new signed gameplay rehearsal is
being verified: denial0, nine disclosed prior receipts, denial9, real standard-map
vs-Bots completion10 and acceptance10 on a fresh lobby connection. Until it passes,
C6 remains partial. No release/deployment claim is made.

C5 is pushed in7fd41139; CI found two Clippy single-match findings, fixed separately
inb5a6b9ba after passing local server clippy. Current repeat full CI42074945255 is
running. Do not treat7fd41139's initial CI as green.


C5 native kit acceptance now passes in CI42074945255/b5a6b9ba:1571client tests
pass (one intentional ignore), including both actual vote-control press tests,
stable control identity while timer/counts change,44px touch targets, original
round/generation and unavailable-action guards. Both normal/dev client jobs pass;
PG/process CI and native visual proof still pending.

C6 admission review moved the public search gate to the queue's actual new-entry
boundary, after existing live allocations are recognized. Thus a legacy zero-count
profile can resume its own allocated seat while a new Ranked search still fails;
new database allocation remains independently locked. Regression coverage added.
The first broad regression found an old schema-upgrade fixture without actual
qualifying receipts; it now asserts that aggregate stats do not fabricate maturity
and separately sets its explicit cooldown-test snapshot. A C5 test also raced an
unauthorized foreign live owner; it now proves fencing first, races valid retries
from the actual owner and then checks cross-owner terminal replay. Production
ownership fencing was preserved.


C5 final CI evidence: b5a6b9ba/run42074945255, all seven jobs and15 public/PG
stages PASS1852.27s. Remake result b7aff3e3f97755a1b0fb90e29ce68902;
surrender d9dc14289adfad8f31ecc76564910454. Ordinary Ranked base victory
30b5e2413afa56633fe7c31e081376c3 took427.58s; all ten exact rating/XP awards,
history and no-false-AFK participation independently rechecked. Artifact13441500118
SHA25655cd68ea379433a178c1d3a483780a2b5759bd12feee835de9f3741c2b75b8a8
verified before safe extraction, private fixture absent. Reports/provenance retained
in `.agent/codex-ranked-readiness/ci-b5-postgres-harness/`. C5 native visual
acceptance remains; C6 full17-stage committed CI and tenth-game proof are pending.


### C6 public acceptance — 2026-11-06

Saved/pushed a73368c6 +888d16dc. Actual public v2 PASS616.87s:
Ranked denial at0 -> nine disclosed synthetic prior Casual receipts -> denial9
-> ordinary standard-map vs-Bots play -> legal surrender after600140ms gameplay
-> saved completed counter10 -> fresh lobby Ranked acceptance10. No VK identity,
custom map or clock/rules override. Result `f2ba22712fd1e68d01f237c7e3f37dbc`;
rating1000/rated0, played10/XP475 (the real tenth game is a completed loss,+25XP).
Artifacts `.agent/codex-ranked-readiness/p6-public-eligibility-v2/`; frozen evidence
`.agent/tasks/RANKED-LAUNCH-READINESS-20261105/c6-evidence.json`.
The first natural-victory attempt timed out while Running and remains FAIL;
it is not counted. The independent full Ranked base-victory proof is green.
All612 real PG/API/server tests, shared184/harness27/Python13 and native/headless
clippy pass. Current full17-stage CI888d16dc/42083864979 and current native visual
acceptance remain pending. C6 remains partial until those finish. C7 is next.


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

P7 actual load checkpoint (not acceptance):1000-client queue v2 FAIL. Periodic
authorization refresh submitted all clients to a256-job link; dropped tail
refreshes caused authorization loss. Pacing/fairness correction is in progress
with4 new bounded-link/timeout/nonce tests passing and repeat PG/load pending.
A separate discovered presence ceiling also needs closure: the worker submits
more than512 entries while CareerStore correctly refuses such a batch; prepare
deduplication and bounded512-entry calls, then verify actual presence for all
1000 identities. Neither issue is hidden by the raised2048 connection ceiling.

P7 second load correction: queue1000 v3 FAIL retained (373 Waiting); serial
account reads remain saturated. Bounded four-read batches are implemented using
the existing pool, with non-read barriers and parked-write order preserved.
Fresh full PG/process verification is pending. Ten-client v4 queue + actual
presence PASS; this does not certify1000 clients or RU capacity.


P9 added release dependency check: current lock rustls0.23.25 matches four open
Dependabot ranges (#21–24). Compatible0.23.46 lock-only update is prepared, with
no added/removed package or manifest change. Current-lock650 PG/API/server tests,
58 passport/HTTP tests (3 pre-existing live-approval skips), affected all-target
clippy and formatting PASS. Fresh committed full CI remains required. References
and exact proof contract:
`.agent/tasks/RANKED-LAUNCH-READINESS-20261105/p9-rustls-spec.md`.


P7 actual connected-playing proof: guarded `scripts/public_playing_capacity.py`
uses ten-client native AI shards and separate server/generator metrics.1 UDP
counter test,28 harness unit checks,5 Python policy/receipt checks and strict
harness clippy PASS.10-client final,100-client trial and1000-client current-source
process probes PASS.100 independent Running receipts/1000 unique fresh peers
rechecked; zero server bots or final account/JSON/framing errors. All peers
Running after49.22s,20s soak, max final peer age573ms. Received snapshots
p05=10.97Hz/median11.72Hz; downlink p95=258272 bytes/player/s. Server group
5.26 cores/1536.55MiB; generator2.25 cores/582.27MiB on14-core M4 Pro. Owned
test DB restored200→100 and all owned processes stopped. This is short local
Casual AI-driver evidence, not1000 physical people, a rated journey, sustained
SLA or RU-host capacity. Egress/performance and committed full CI remain open.


P9 full0c596fab CI42123329169 FAIL:6/7 jobs,17/18 public stages PASS. A natural
Ranked victory ends with one idle native actor correctly forfeited for AFK;
the ordinary all-active Elo oracle rejects teammate loss reduction. Raw failed
artifact13462397903/hash is retained. The test driver now sends deliberate
ordinary held movement/aim on fresh alive decisions; dead/unjoined/stale peers
still send no active input. The verifier explicitly requires no AFK/abandon/
disconnection and saves result rows before asserting. No production AFK/Elo
change. A separate repeated-PG fixture uses a unique Unicode prefix instead of
a common name hidden by the bounded search page. Current656 PG/API/server and
9 Ranked verifier checks PASS; fresh full Ranked victory is running.

P8/C2 runtime and signed process checkpoint PASS: a marker pauses all queued
ready checks (0/9/10 confirmations tested) without strikes, preserves groups/
FIFO and permits active seat Resume. The capacity report acknowledges a unique
operation ID; malformed markers fail closed. Existing match keeps advancing
while a free arena stays unused; old decline/15s expiry do not invent a dodge.
After cancellation a new check and actual second match start. A second marker
survives actual lobby restart, both workers are adopted and Resume still works.
Native maintenance retry/cancel and RU/EN/ZH strings pass; current visual still
awaits manual unlock. Host ship/wait/cancel integration and production approval
remain open. Full current CI is mandatory; no C2/RU completion is claimed.

P9 corrected actual full Ranked victory PASS:5aa462a2 source, result
70227325cf99969e84347c8d4e7d3c97,435.92s including setup. Independent exact
all-ten Elo/XP/profile/history and zero AFK/disconnection checks PASS. Standard
map and ordinary rules/timing retained; ten qualification profiles use disclosed
synthetic prior Casual receipts. Fresh committed full CI remains required.


Owner O-5 decision2026-11-06:20 placement Ranked games; Bronze<1000,
Silver1000–1499,Gold1500–1999,Platinum2000–2499,Diamond2500–2999,Master3000+.
Season90 days; soft reset1000+(old_rating−1000)/2, archived results and seasonal
badge. The owner accepted this proposal; application of any production reset
remains a separate authorized action. P6 tier/season implementation and full
acceptance are now unblocked; nothing has reset live ratings.

Owner O-18 decision2026-11-06: touch and mouse players share one Ranked queue.
Persist the participant's input type for result comparison; do not split the
queue or change matching weights by input. P6 implementation and statistics
verification remain required.

Owner P7 decision2026-11-06: approved the direct pinned flate2=1.1.0 dependency
and applying lossless world-snapshot compression. The prototype's79% reduction
is corpus evidence only. Production protocol/client/helper compatibility,
bounded malformed-stream tests, actual gameplay and current load measurements
remain required before closing egress/performance findings.


P9 TLS checkpoint5da85959 CI42126718754 PASS:all7 jobs and18 real public/PG
stages (2570.21s). Artifact13462799158 SHA256
`526d44735eb79bb3e5a66d455cd1d5e55b1851e1c4e182bce8c964b07dc446c0`
verified before bounded safe extraction; private fixture removed. Independently
rechecked Ranked result a4c2b83b9034cc894ac3ca1bf454fbec,426382ms gameplay,
all ten exact Elo/XP/profile/history awards and zero AFK/disconnection. Real tenth
public completion and denied0/9→accepted10 withoutVK also PASS. This green run
precedes the maintenance/current-driver/compression checkpoints;5aa/current CI
is tracked separately and is not assumed green from this result.

P5 current native5aa462a2 RU852×393 Mobile preview: both Home states captured
from actual Bevy pixels, with exact binary/source/image hashes. Home10 shows
rating1234/placement19of20 and enabled Ranked; version/footer no longer overlaps
navigation. Home0 FAIL: long locked-progress text wraps outside the half-row
button. Corrected short captions and14px phone queue labels are building for
repeat captures. Render fixtures do not prove real eligibility/physical devices.
C6/#39/P5 are not closed by the initial QA summary's automatic passed flag.

P7 production compression checkpoint: protocol16, pinned approved flate2 direct
dependency with no added/removed package identities. World-only selective zlib
envelope; existing control encoders and cadence unchanged.187 shared tests,
Node323 malformed/bounded refusals, byte-exact40-frame Python/Node corpus,
7 contract tests and10 navigation observer tests PASS. Tooling full654 tests
PASS with9 intended skips in isolated Python3.13 settings; first system-Python/
proxy/handoff failures retained as failed evidence. Current PG/server/harness/
clippy build and actual gameplay/load checks are running.


P7 compression local gate PASS:187 shared,656 real PostgreSQL/API/server,
28 harness unit and5 actual UDP checks, strict all-target clippy for five affected
crates. Real populated release5v5 frames use OMZ1 and preserve byte-complete
ordered schema/fog under reorder/duplication; raw/packet counters pass. Both
current protocol16 native Home captures visually PASS after the locked-caption
fix (RU852×393 Mobile preview, render-only account). C6 software acceptance is
now complete; tier/season, mobile physical feedback and C5/C7 visuals remain.
Signed Node ready-check v2 PASS with real15s expiry/no premature spawn/FIFO/
stale answer/replacement. v1 correctly failed because the command omitted the
qualification helper; its fresh accounts were denied Ranked, not bypassed.
A verification wrapper also initially named a nonexistent qualification example;
corrected build PASS, both first failures retained. Complete current Ranked and
post-compression playing-load measurements remain pending.

P9 current5aa462a2 CI42132894282 PASS all7 jobs/18 stages (2523.50s). Artifact
13466364097 SHA256c67161d2f13da078d98def557d205b16f5761a929f8c4cbaaba034368752c9bf
verified before safe extraction. Independently rechecked actual Ranked result
a72dd61c9a1f635bb60da7092d3cb330,426600ms gameplay, exact all-ten Elo/XP/
profile/history and zeroAFK/disconnection; real tenth/noVK gate and early-crash
recovery PASS. This closes the saved maintenance/test-driver full-CI checkpoint;
new protocol16 compression still needs its own full CI.


Protocol16 frozen ebf3a1fa actual standard-map Ranked victory PASS: result
3efdf9f470acaf248c024bc243185a98,497.09s including setup/469460ms gameplay.
Independent all-ten exact Elo/XP/profile/history and zeroAFK/disconnection PASS.
Synthetic prior qualification receipts disclosed; real gameplay unchanged.

Post-compression1000-playing v1: live acceptance PASS (100 arenas,1000 unique
connected native AI peers, zero server bots,60s all-playing); p95 wire42014B/s,
median received complete snapshots9.54Hz. This is below cadence acceptance and
not a controlled A/B versus the shorter prior run. First wrapper missed the
normal wait for asynchronous PostgreSQL connection closure; all processes
exited, zero DB clients verified and temporary200 restored100. Failure retained.
Repeat v2 correctly waits/restores100 but gameplay acceptance FAIL:707 actors
retain rejected auth challenges,165 retain lost account authority, only~12
arenas start. No repeated-load/overall acceptance claim. Correct the synthetic
client's fresh-challenge/explicit-authority-loss recovery, audit the native
client's existing scoped recovery/25s timeout, then repeat. Production signature/
challenge TTL/ban policy remains unchanged.

P6 approved-rule primitives: six tier boundaries,20 seasonal placements,90-day
duration, bounded integer half-distance reset toward1000, and monotonic declared
touch/mouse/mixed/unknown evidence.191 shared tests PASS after registering the
new tolerant types in the enforced unknown-value policy. Persistence, season
rollover/archive/badges, input recording/query and native/portal display remain
open. Helpers are uncommitted WIP while the repeat-load recovery is corrected.


P7 repeat-load recovery correction: synthetic public client now requests a
fresh challenge after rejection instead of re-signing the expired nonce. An
explicit loss of account authority starts a new transport replay scope with
the same key; empty views/foreign epochs cannot trigger it, repeated old views
cannot flood retries. Native already implements scoped authority-loss recovery
and bounded25s attempts; add its missing alias for the server's QueueAccess
revocation wording. No server TTL/signature/refusal relaxation.191 shared,14
matching,30 harness and5 capacity-policy tests, strict affected-crate all-target
clippy and current native test-target checking PASS. New actual1000-playing
repeat/full CI remains required. Load provenance now includes public-client
source/transport hashes and dirty-path disclosure. P6 primitives are preserved
in local e459dab8 with the integration plan; persistence/UI remain open.


Protocol16 ebf3a1fa full CI42139546925 PASS all7 jobs/18 stages (2549.47s).
Artifact13469856606 SHA2566d83ff4a37ad62b7ba014315c44e2f3b26fb436694a8b76bb250fa429c652c75
verified before safe extraction; independent exact all-ten Ranked proof
bc9062cb291051251448828a595b5fc5,466788ms gameplay, zeroAFK/disconnection.
Real tenth public game/noVK and early crash PASS. Later auth/helper changes
still need their own full CI.

P7 repeat v3 FAIL retained and owned test PG restored100:95 full rosters plus
a valid five-human/five-bot Casual fallback consume the finite pool, so1000
humans cannot all fit. Most expired-auth clients now recover; this finding
does not justify changing Casual's30s fallback. The playing-capacity tool now
waits for every distinct real sign-in to be fresh/error-free/unallocated for5s
before releasing Search, like the native Home flow. Loopback-only actor gate
keeps ordinary signed authentication, consent/draft and gameplay.31 harness
unit cases,3 CLI refusals in1 process test,6 Python policy cases, fresh binaries
and strict harness clippy PASS. Ten-client actual pilot PASS: all authenticated
9.14s, Search released14.17s, all Running28.00s,15s soak; p95 wire58056B/s,
complete received median17.69Hz.1000 repeat and full current CI remain required.


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


### P6 acceptance — findings35/39 complete in software

Exact c826 native RU expired Home and coherent personal demotion pixels PASS;
English own/friend/visited badge pixels PASS on the same retained binary. Earlier
six scoped RU states are reused. Portal ccee8fa adds six English seasonal/privacy/
legacy checks to the seven RU notice checks. All inspected images are preserved
unchanged with source identities and SHA256 in
[the evidence pack](../evidence/ranked-2026-11-06/README.md). Native1578 cases,
season-core full CI, real PG/operator/permission checks and portal250 cases are
recorded separately; fixtures are not live-account/hardware proof.

Rows35 and39 are now done:59 done,6 partial,2 open,1 mostly refuted. Other P5/P6
items and all P7/P8/P9 release requirements stay open. First post-unlock v5 Home
was black despite Quartz reporting unlocked/awake; v6 and v7 then produced valid
pixels. Keep v5 rejected, no assumption that the owner failed to unlock. An EN
driver path typo failed before launch, was corrected, and the real three-state
run passed; raw failure/success logs are retained. No new native build occurred.


### P7 directory foundation — not activated

The shared D1 contract/policy and offline Ed25519 signer/verifier are implemented;
197 shared/2 operator cases, strict checks and9 independent Node/process cases
PASS. Signed concrete IPs are part of the payload so upcoming routing can avoid
DNS substitution; pinned fallback/client integration remain required. No live
key, URL, infrastructure, dependency or UDP protocol change was performed.
Full scope and remaining gates:[game-directory.md](game-directory.md).
Finding12 moves from open to partial. Current original list:59 done,7 partial,
1 open,1 mostly refuted. Critic items remain separately tracked; P7/P8/P9 and
other P5/P6 requirements are not closed by this checkpoint.


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


### Vote replacement regression/fix checkpoint — verification in progress

Production vote rendering now follows current pause navigation and explicitly
flushes before UiSet::Paint. This removes the overlap with queued slab decoration
of replaced prompt/menu children. The new actual decorator/modal-cycle regression
checks immediate skins,20 Help modal open/close cycles, removed entities and a
subsequent correctly scoped vote request. Strict RU client/lib/test-target clippy
PASS (64.24s); filtered execution is still compiling on cacheA, native v4 on B.
No passing execution or repaired native-guide result is claimed yet.

Focused QA additionally requires an actual visible named root and appropriate
modal/help state; initial states are re-entered, and result scenarios create the
same synthetic local Player that the existing Result QA uses. No new dependency,
protocol, workflow or production action. All remaining gates retain their scope.


### Current native build and independently checked directory CI

e85875ba source hashes match all6 bound client/QA/regression files. Native RU
v4 build PASS in612.1s,minimum root35.08GiB,binary SHA256
`a54fdaa13b27540825ea342d2a04ced0b039d4870a3763168b047dcae9519d61`.
Fresh capture v4 stopped before launching a client because the current Quartz
session reports locked. The prior unlock did work and produced the5 committed
accepted images. No rebuild is needed for the remaining fixed-state captures.
The filtered actual decorator/modal regression is still compiling on cacheA;
CI42211135390 for the fix is active. C5/#45/result acceptance is still pending.

Directory2a45600c/run42201671041 independently rechecked:all7 jobs/18 stages PASS,
2699.35s. Artifact13497606620 matches SHA256
`4d5b0973b4e1a7b3a67b659a0ef818898f6538b5c6f84be4c352de1518a104e3`;
139 bounded/unique/path-safe/non-symlink entries and CRC pass. Real standard-map
Rankede2de026ae59553a668818e861b2dbe8f lasts546050ms with zeroAFK/disconnection;
all-ten Elo/XP/history/lifetime/season increments were recalculated.15s unanimity,
FIFO/party recovery, actual10th completion/noVK, crash, unique picks, remake and
surrender proofs pass. Sanitized summary:../evidence/fair-play-2026-11-06/ci-2a-summary.json.
This predates the new modal fix and is not WAN/RU-host/hardware/provider evidence.
All remaining original/critic/operations/release requirements retain full scope.


### D1 HTTP delivery and modal regression checkpoint — 2026-11-06

The opt-in anonymous signed-directory file publisher and exact-byte regional
portal transport are implemented, without a live publication/activation. All4
API tests PASS, including explicitly executed migrated-PostgreSQL/router proof;
strict API all-target clippy and RU executable build PASS. Portal257 cases PASS,
3 existing expected skips, typecheck and RU production build PASS. Seven actual
Rust API/PostgreSQL/production Next checks PASS: browser account material is not
forwarded, native verifier accepts exact delivered bytes, HEAD/method/query
boundaries hold, corruption returns503, and atomic signed replacement is visible
without a restart. Known fixture private file removed before services started.
[Report](../evidence/fair-play-2026-11-06/directory-http-summary.json) and
[configuration/limits](game-directory.md#implemented-d1-http-delivery-checkpoint--2026-11-06).
#12 remains partial: native trust/cache/selection/pinned fallback, publication,
party/Resume/cross-host discipline and actual multi-host/WAN proof stay required.

The actual kit-decorator/20-modal-cycle regression now PASS in CI42211135390,
job127869761950, on e85875ba:1579 client cases PASS,1 ignored. All6 bound client
source hashes match. The redundant local Global compilation was deliberately
stopped after that execution proof, not reported as a local pass. Sanitized
[CI evidence](../evidence/fair-play-2026-11-06/vote-modal-ci-e858.json) is preserved.
Native RU binary remains ready; fresh Quartz check after the owner's unlock
reply again reports locked, so no new client/image was launched or accepted.
C5/#45/result pixels remain pending; no rebuild is needed for them. Original
finding counts stay59 done/7 partial/1 open/1 mostly refuted. No remaining
P5/P6/P7/P8/P9 gate was waived, and no live deployment was performed.


### Native anonymous directory transport — verified, not activated

`passport/src/game_directory.rs` derives its endpoint from the exact account
region/contour, with no runtime service override, browser/session material or
cookie jar. Redirect/non-success/non-JSON/interrupted responses are rejected;
streaming is capped at64KiB with5s total/3s connect deadlines. Four actual local
TCP/HTTP cases PASS: fixed origins/negative URL inputs, exact Unicode/u64 wire
and absent account headers, redirects/errors/HTML/truncation, and declared/
missing/chunked oversize bodies plus the exact64KiB limit. Strict RU passport
all-target clippy PASS. Reports:p7-directory-transport-v1-* under the task proof
directory. This fetch returns untrusted bytes; native signature/cache/selection,
pinned fallback/ownership and full P7 requirements remain mandatory. #12 stays
partial; no URL, key or player routing was activated.


### Native signed-directory trust/cache checkpoint — verified, not activated

All9 native cases PASS: exact Unicode/u64 signing domain and independent pins,
scope/time and separately checked known foreign/dev labels/IPs, expired-cache
rollback/equal-sequence fencing after restart, pin overlap, corrupt/nonregular/
symlink/oversize state, actual second-process OS-lock contention, immutable old
cache after invalid input, concrete authenticated destinations and backwards
clock rejection. All9 existing endpoint guard cases and strict client lib/test-
target clippy PASS. Source hashes and guard timing/disk proof are preserved in
[directory-native-summary.json](../evidence/fair-play-2026-11-06/directory-native-summary.json).
A cached expired signature retains its sequence floor but yields no admission
address; corrupt state never silently resets that floor. No signer/private key
or new dependency was introduced. No live key or UDP player route was activated.
Runtime pin baking/fetch/cache/pinned fallback/selection, maintenance/update UI,
party/Resume/cross-host discipline, actual multi-host/WAN and all remaining gates
stay required;#12 remains partial. Full current-source CI is still required.


### Fair-play fixture correction and no-window render feasibility

v6 remade result was rejected: it still inherited the saved-win fixture's Green
Victory while its receipt was Remade/winner-less. Production PostMatch correctly
kept the scoped victory. The QA fixture now uses Running until the terminal
remake receipt; existing victory-conflict coverage includes Remade. All18
post-match cases and strict client lib/test-target clippy PASS. A new native
build is required before corrected result pixels can be accepted.

v7 locked-window prompt was also rejected (44px geometry/text present but black
readback). No new accepted game image exists. An independent native no-window
render-to-texture diagnostic succeeded in12.29s with a visually inspected
852x393 frame while Mac reported locked after capture. It changes no lock/OS
preferences, reads no desktop and adds no dependency/cargo cache. Integration
into actual opt-in QA and game-screen acceptance remains required; this is not
C5 completion. [Failure/source summary](../evidence/fair-play-2026-11-06/rejected-v6-v7-summary.json).
The earlier full e85875ba CI is now all7 jobs green; latest directory/runtime
work still needs its own full current-source CI. All remaining requirements
retain their original scope.


### Current native CI correction — directory protocol literals

1ab44e5a CI42224273521/dev-client job127913696738 failed the key-shaped-literal
source scan: DNS fixture strings beginning with the shipped `lobby` namespace
were mistaken for translation keys. The other1587 executed client cases pass,
with1 existing ignored case. The routing fixtures now use raw literals, the
scanner's documented non-key convention (`i18n/tests.rs`, Span::Skipped). Wire
host values are unchanged and the scanner/translation tables are untouched.
The exact unchanged compiled source-scan test passes against current sources
(.agent/codex-ranked-readiness/p7-directory-i18n-source-v1.log); full corrected
branch CI is still required. No failed job is reported as a green full gate.


### Native rendering without an OS window — actual game frames verified

Explicit desktop `OMOBA_QA_OFFSCREEN=1` disables Winit before DefaultPlugins,
supplies a virtual primary window and redirects the production main camera/UI
to a screenshot image. Existing avatar/party preview image targets are retained;
Creator Loop uses the selected virtual dimensions. Normal unflagged launches
retain their window path. Only the selected fair-play/Creator Loop harness can
activate it; phones refuse the mode. No OS focus/unlock/power settings changed.

Strict RU client lib/test-target clippy PASS74.24s; native build PASS981.32s,
binaryd8d0bf61… ; minimum root32.71GiB. Two actual opt-in/camera regressions PASS,
including late cameras, distinct preserved preview target and DefaultUiCamera.
All5 map cases/64-screen registry checks PASS. Initialv1/v2 compile negatives
(RenderTarget equality then fallible SystemState API) were corrected before the
single successful native build; no failed run is counted as pass.

Fresh real production-rendered RU852×393 fixture frames are inspected: coherent
Remade/no ordinary reward/zero XP; played surrender result; remake entry and
surrender availability; both44px vote prompts; actual first-match guide followed
by a real Creator Loop dismiss and the restored remake prompt, without the old
entity-despawn panic. These are synthetic presentation proof, not real admission,
physical phones or performance. Current full corrected branch CI remains required.
The party-restart/Home attempt still has no underlying Home content and is
rejected;#45 remains partial. No empty backdrop or false green was accepted.


Owner-requested reusable method saved in the repository skill
`.claude/skills/omoba-ui-verification/SKILL.md`, with
`references/offscreen-native.md` and `scripts/capture_offscreen.py`. The runner
needs no temporary-session code, creates only fresh ignored fixture output,
clears inherited service overrides, uses closed loopback endpoints and records
binary/source/runner hashes. Skill validator and Python syntax pass; actual
locked-Mac surrender, guide/dismiss/prompt, menu and surrender-prompt calls pass
and their images are inspected. The method does not grant legal consent,
production access, extra UI matrix, physical-device or OS-window acceptance.


Six accepted offscreen game PNGs, build/source hashes, per-image scope and actual
locked-launch metadata are now preserved in
[the evidence pack](../evidence/fair-play-2026-11-06/offscreen-manifest.json).
C5 native visual acceptance is now complete for entry, vote prompts, actual
first-guide coexistence/dismissal and both saved result states. Final C5 closure
still awaits corrected current full CI. The separate empty Home fixture is
retained as a failed attempt;#45 and all physical/multi-host/RU release gates
remain open. No diagnostic/empty/black image was promoted to acceptance.


### Home native acceptance — correct the Fair Play shell classification

`spawn_home` intentionally skips world-only automation. Fair Play exercises Home
as well as match states, but was absent from `SHELL_HARNESSES`; its root constructor
therefore returned immediately. Add it to that allowlist and the existing
classification regression. No ordinary gameplay behavior or screen-lock setting
changes. Existing regression PASS1; RU strict client/test-target clippy PASS74.23s,
native build PASS605.95s, minimum root28.51GiB. Binaryafa5e8de… .

The new real native RU852×393 Home image is inspected: menu, character, actions
and the restart/dissolved-party notice are visible. Preserve exact PNG/build/source
hashes in [Home acceptance](../evidence/fair-play-2026-11-06/offscreen-home-accepted.json).
This closes the missing-Home visual failure, not the actual lobby-restart/public
party journey. Row45 remains partial and row65 still needs real admission proof.
The old rejected Home attempt remains rejected. A first real local solo runner
attempt was also rejected: its gate file was nonempty and its button inventory
included hidden controls. No ready-check/draft acceptance is claimed for that run.

Currentbf359465 CI42233107055: format/core, gameplay harness, Android, Python and
bot jobs PASS; dev-client job127943697882 FAILED1589 passed/1 failed/1 ignored
with `CacheBusy` in expired-cache restart regression. PostgreSQL public journeys
are still running. Investigate the lock lifecycle; do not report this as full green.


### D1 CI repair — inherited cache-lock descriptors

The bf359465 dev-client failure is a real lock-lifecycle race. Unix locks belong
to the open-file description: a concurrent fork can retain it after the parent
closes its descriptor, until the child execs/exits. Explicitly unlock in a small
private RAII guard before closing. The stable lock inode, fail-closed cache,
nonblocking contention and cross-process exclusion remain intact.

A bounded actual subprocess retains a duplicate descriptor and proves both
contention while owned and successful reacquisition after parent release. The
exact production guard module compiled through a std-only wrapper passes30/30;
the close-only negative control fails with `child retained a released lock`.
Strict RU client/test-target clippy PASS90.30s. Full integrated directory tests
are still compiling; Linux/full branch CI is required. Preserve [the scoped
regression proof](../evidence/fair-play-2026-11-06/directory-cache-unlock-summary.json).
No test is weakened/retried to mask a failure, and no CI workflow is changed.


### P5 native public journeys — row65 functional acceptance

Real RU852×393 native client, owned loopback lobby/worker and PostgreSQL55582:
solo Home→Searching→15s Ready→one Draft with9 authenticated AI-controlled human
seats. Two additional native instances formed a party through real invite/accept,
returned Home, and the leader launched the group. Both went directly to Searching,
confirmed separately and entered one Draft each with8 AI-controlled human peers.
All10 actually joined the same worker; both native profiles are on the same side.
No synthetic party/match-service views, database friend seed, rank maturity fixture
or account binding was used. No HeroSelect was observed on either route; the
existing Home/party functional regressions cover the direct transition. Row65 is
done in software. Ledger:60 done (including3 minimum),6 partial,1 open,1 mostly
refuted. Current full CI and P5/P6/release gates remain independently required.

Preserve exact PNGs, scoped transition/allocation evidence and matching UI-source/
binary hashes in [the native public UI pack](../evidence/native-public-ui-2026-11-06/summary.json).
These are actual admission/flow checks, not a new completed-match/settlement,
physical-phone, performance, WAN or RU-host certification. Nativeworker snapshots
use the internal practice mode; the authoritative allocation is public `quick`
with10 signed human seats. The solo frame is captured during join; party draft
frames wait for all10 actual worker joins.

New defects discovered on supported long nicknames remain explicitly OPEN:
- **UI-1:** mobile Home identity nickname wraps and pushes rating outside its chip.
  Reproduce with authenticated `QA-Party-leader#7964`; fix bounded name/rating layout
  and verify the same native state. Existing row39 minimum/short-name evidence does
  not excuse this overflow.
- **UI-2:** five-player mobile Draft nameplates clip the beginning of nicknames
  and crowd labels. Fit/truncate names within each slot and inspect a full team.
  AI avatar assets are not loaded in these frames; investigate actual source before
  claiming team-stage visual completeness.
Functional row65 acceptance does not promote these images to full layout acceptance.


D1 cache-lock follow-up: the actual integrated client suite finished:10/10
`game_directory::` tests PASS, no ignored/failed,0.30s after885.13s compile. This
includes cryptographic scope, expired rollback/equivocation fences, corruption,
independent-process contention and the inherited-descriptor regression. Minimum
root space stayed above20GiB. Linux/current full branch CI remains required.

Actual local restart attempts are retained under ignored
`.agent/ui-proof/p5-native-public-restarts-v1` and `...-v2`. The first captured
reconnecting clients and pressed Play before it was enabled; rejected. The
second restored real clients after both owned lobby restarts, but Home captures
missed the group-loss toast and the later Searching capture still showed the
group-loss wording. Do not count these as the intended Home/solo-search warning
acceptance or close45. Add observable actual party/profile/toast state to the
Creator Loop readout, then wait for the correct event and inspect the matching
frame. No production/dev service was restarted.

UI-1/UI-2 fixes are prepared: one-line bounded Home nickname plus enough chip
height for the rating, and per-slot name/class/role ellipsis in Draft at the
actual readable phone font minimum. Verification/build is in progress. These
new layout defects remain open until native long-name images are inspected.


D1 cache-lock Linux confirmation: CI42238965581 at8a02d375, dev-client job
127963924108 SUCCESS:1591 passed/0 failed/1 ignored,12.61s. Both the previously
failing expired-cache restart test and inherited-descriptor regression pass.
Android, Python, gameplay harness and bot jobs also pass; core/PostgreSQL public
journeys remain active. This is a full client gate, not a full branch/release gate.


Minimum audit checkpoint for5/7/15: current `career_runtime::leave_match` already
releases the exact authenticated non-Ranked prestart session; the worker tracks
`released` and aborts only when all human seats are released. Shared allocation
regression covers exclusion of that exact profile/session and preservation of
others. The unclosed gap is `MatchService::cancel` on an assigned lobby profile: it
still calls `pool.cancel`, aborting the worker and re-queuing others. Any fix must
request a durable worker release and respect its acknowledgement / race at actual
start; do not simply rewrite a running manifest or recycle the profile's active
seat. Minimum statuses stay unchanged.


UI-1/UI-2 candidate passes strict RU client lib/test-target clippy26.11s. The
first compile attempt exposed an ambiguous float and Bevy system parameter limit
in the new QA readout; both were corrected (typed font size, grouped optional
resources, both gated and ordinary return paths updated) before the successful
check. Native build is still running; neither layout defect is marked closed.


Creator Loop QA now reports actual public party/profile state, current party
notice and active toast texts; its visible-button inventory checks display and
computed size as well as inherited visibility. Add `UiToastText` name to the
existing toast text; no toast delivery/timing policy changes. The reusable skill
now explains waiting for real new-epoch events and an empty bot search gate, and
its helper records Home/draft/party/toast sources too. Skill validation and helper
syntax PASS. Current native build/pixel verification is still required.


### P5 native restart acceptance — row45 done; nickname follow-up remains

Native build PASS746.61s, RU strict clippy PASS26.11s, binaryb2609614… , minimum
root22.01GiB. Both actual native clients form a party using real UI invitation.
After returning Home, restart only the owned loopback lobby. Wait for the actual
new-epoch party view and current group-loss toast, then capture: both Home
perspectives show the readable dissolved-party warning. Start a real solo public
search, restart that owned lobby again, wait for the distinct actual search
restart toast, then capture. Search resumes1/10 and the correct wording is visible.
All processes stop through their owned groups; no production/dev service changes.
Row45 is done in software. Ledger:61 done (including3 minimum),5 partial,1 open,
1 mostly refuted. Full corrected branch CI remains required for release/C5.

The changed UI revision also repeats the actual two-native party public journey:
Home→Searching→15s ready→one Draft each, all10 actual worker joins, party on the
same side. Long Draft identities/class/roles/status now fit their own slots with
ellipses; no adjacent text overlap in either perspective. **UI-2 done** for this
RU852×393 case. Empty AI plinths reflect missing/unrevealed fixture avatar choices;
these frames are not a complete five-avatar asset/render certificate.

**UI-1 remains partial:** the long nickname is one line and the rating now fits
inside its taller chip; however, the leader name end/ellipsis can still be clipped.
Use the actual glyph/layout bounds or correct the constrained text width, then
repeat only this failed Home check. Do not mark it done based only on fitting
rating geometry or the member name's successful ellipsis. Preserve all exact
images/source/build/event evidence in [the follow-up pack](../evidence/native-public-ui-2026-11-06/restart-and-names-summary.json).
The earlier restartv1/v2 attempts remain rejected for their intended warning
acceptance; the event-drivenv3 is the actual successful proof.


### P5 handoff cancellation completed —5/7/15 leave minimum status

An immutable manifest is retained. For unstarted Casual/vs-Bots, the lobby writes
a bounded durable exact profile/session/team release request; the owning worker
validates it, removes only that unused human and publishes `released_sessions`.
The lobby retains the assignment until that ACK. An explicit cancelled search
is removed from parked intent immediately after durable request publication so
a later worker failure does not silently requeue the canceller. Direct worker
Leave ACKs also prune parked entries. Worker scans are bounded to4Hz before start
plus the final formation boundary; gameplay does no release-file scanning.

Durable roster freeze/start wins a racing request: no participant/admission is
freed, and the original seat remains resumable. A terminal receipt observed
between the lobby's cached check and request write takes the terminal cleanup
path, excludes the canceller and restores only survivors. The last released
human aborts rather than starting a bots-only credited match. Ranked retains
its existing abort/FIFO/penalty path; no bot is inserted into Ranked.

Four targeted regressions PASS. Strict server all-target clippy PASS; full normal
server suite502 passed/0 failed/4 ignored (7.76s), native server build PASS. The
new bounded real public scenario, against isolated PostgreSQL55582, forms10
signed human seats. One cancels from the lobby without ever joining the worker;
its exact release is ACKed and9 original humans reach Running with1 bot on the
same allocation/PID. Fresh profile read proves no cancelled match/XP/rating
credit. A running survivor signs Cancel through the lobby and keeps its original
resumable assignment; no new release or whole-match cancellation is written.
The final source revision repeats all four process assertions PASS.

Preserve source/binary/gate/process evidence in [the handoff pack](../evidence/casual-handoff-2026-11-07/summary.json).
The public scenario is separately callable with `--scenario=handoff-release` and
also follows the existing `unique-draft` PostgreSQL probe, so CI exercises it
without workflow edits. Rows5/7/15 are done in software. Ledger:61 done with no
remaining minimum-only items,5 partial,1 open,1 mostly refuted. New full branch
CI is required after this candidate, and physical/RU/scale/operations gates remain.

### C5 corrected full baseline gate accepted

CI42243761726, source6a61a7ac, completed SUCCESS across all7 jobs. This satisfies
C5's outstanding corrected full gate after source-scan/cache-lock repairs. C5 is
done in software; its server/controller/process and actual native pixel evidence
retain their original scopes. The separate Release workflow was cancelled; it
is not a deployment. All new subsequent source changes need their fresh full
release gate and do not inherit this result automatically.


### UI-1 accepted with production glyph bounds; corrected visual diagnosis

The previously suspected `QA-Party-leade…` truncation was a false negative: the
original committed image contains all three ellipsis dots. Exact unchanged
source-pixel connected components and shaped layout confirm it. Do not retain
that mistaken interpretation as a product bug. A genuine supported counterexample
is20 W characters plus tag: initial heuristic emits12 W+ellipsis, but the actual
Inter20px layout is258px inside a210px node. Its ellipsis is clipped.

Keep the readable font and original profile value. A Home-only marker consults
production `TextLayoutInfo`/glyph ink bounds and node width before each new UI
layout, shortening just the displayed prefix until it fits with an ellipsis.
Actual final wide layout197px/210px,20px font; normal name165px/210px unchanged.
New recorder geometry reports bounded selected-label metrics/last8 glyphs and
inherited clip. No new dependency, account data mutation or OS setting change.

RU strict client/test-target clippy PASS48.09s, native build PASS547.43s, binary
e70b6657… , minimum root31.85GiB. Both native Home identity frames are inspected.
The repository skill gains an optional validated synthetic `--nickname` for its
Home fixture; its actual bundled helper reproduces the wide name with intact
ellipsis and passes existing native/offscreen gates. Skill validation and Python
syntax PASS. Preserve original failure, actual bounds, unchanged-original-pixel
check and fixed exact PNGs in [the identity pack](../evidence/home-identity-2026-11-07/summary.json).

UI-1 is done. Existing numeric-rating proof remains in the real public-party
pack; these new closed-loopback identity fixtures do not prove account/server
admission or physical phones. UI-2 widest-name acceptance and UI-3 compatibility
failure-state hint/navigation overlap remain explicitly open/partial above; no
full Home/full-device certification is claimed.


Combined CI-entrypoint follow-up: actual `--scenario=unique-draft`, capacity2,
owned PostgreSQL55582 passes its3 existing Ranked unique-pick assertions followed
by all4 new Casual handoff-release assertions in the same lobby. This verifies
the newly connected path, not only the standalone new scenario. Ranked maturity
uses the disclosed existing synthetic fixture; no synthetic history is claimed
as played. Source1310e564 is pushed; fresh CI42267312369 is running. Code is
saved separately as84894318 (server handoff) and1310e564 (Home glyph fitting).


### Fresh CI failure and peer-source correction — 2026-11-07

Run42268366799 at84fa2344 passes6 of7 jobs (full native client, Android,
core/clippy, gameplay, Python and bots), but PostgreSQL public `party-leave`
times out before the planned departure: only9 of10 seats reached Draft. All506
server Rust cases passed; this does not override the failed public scenario.
The artifact SHA is verified against GitHub. Client6's new worker endpoint59898
reused client7's former lobby endpoint; its final snapshot is still Lobby despite
140 signed Join attempts. The probe ignored packet source and could apply late
lobby epoch/auth to its new socket. Controlled source replay confirms the fault;
the native client already connects its UDP socket to its selected peer.

Filter the loopback probe's current peer before any decoding, retain retired-
socket fencing, and inject old lobby views into both the retired callback and
the *new* worker socket on each flagged handoff.3 real-socket cases PASS; removing
the source filter makes the new regression fail.5 existing policy wrappers PASS.
Actual ten-player `party-draft` with stale handoff exercises PASS: five-stack kept,
nine survivors retain FIFO/party/career, only the voluntary leaver receives one
strike, and the fresh10-human roster enters Running.20 callback and20 old-source
replays execute. Ranked maturity prerequisites are disclosed synthetic receipts.
[Exact compact proof](../evidence/probe-peer-fence-2026-11-07/summary.json).

Fresh complete CI after this fix is still required. UI-2/UI-3 follow-up remains
unverified WIP: no heavy client test/build launched while root disk is below
20GiB. No additional original finding is closed by this harness repair, and no
production deployment or physical/RU/scale acceptance is implied.


### UI-2/UI-3 follow-up candidate saved — verification pending

Generalize the previously accepted Home glyph fitter into an explicit shared
single-line label marker, and apply it to actual Lobby and Draft name/status
plates. Add bounded label geometry to the existing native recorder. This retains
readable font sizes and the original full nickname; widest-name frames are still
required before accepting UI-2. The repository capture helper now hashes the
shared widgets source as part of its source provenance.

Move phone Home required-update/compatibility copy into a left-hand notice slot,
reserve identity-chip space and keep it out of the right-hand Offline practice
column. Preserve desktop presentation. A separate reviewed functional gap was
found: a compatible server plus a newer catalog minimum disabled the primary
Play button but left side Ranked/vs-Bots entries active. Disable all new queue
entries and ignore stale activation events after either requirement appears;
offline practice stays available. Add a connected/catalog-only regression for
all three stale queue activations and actual offline entry. This test has not yet
executed, so the candidate is not certified.

`cargo fmt --all -- --check`, source review and diff whitespace checks PASS.
The guarded targeted Rust test and RU clippy/native build both refused to launch
before any Cargo child because root free space was below20GiB. Approved A/B
caches already point to Orico. No cache deletion, foreign-process shutdown or OS
settings change was performed. Save this as an intermediate source candidate for
existing CI; UI-2 remains partial and UI-3 remains open until tests/native pixels
pass. This does not add a production dependency or alter the CI workflow.


### Finding28 adjudicated and closed — 2026-11-07

An actual owned production Next/Chrome run with synthetic Account API values
`outcome:interrupted,rated:true,rating:null` confirms neutral «Прерван» and `—`
in history, singular «Рейтинговый» in the detail, no winning team and `—` for
personal rating. The original history-label claim is refuted; the genuine
singular-tag part was already fixed. Preserve both inspected unmodified images,
source/report/build identifiers and scope in [the finding28 pack](../evidence/interrupted-ranked-2026-11-07/summary.json).
New reusable portal `tests/interrupted-ranked.e2e.mjs` passes3 focused render
checks; complete portal units257 PASS,0 FAIL,3 expected skips. Reused the existing
RU production build, without claiming a fresh source-to-bundle/release build.
Initial bad-selector test attempt was rejected and corrected before acceptance.

Original68 ledger now62 done,5 partial (#1,#2,#12,#20,#50),1 open (#13).
Separate critic/physical requirements and UI-2/UI-3 native acceptance remain.
Current game candidatef144c8f7 is pushed, full CI42273662478 is in progress:
Android/Python/bots already PASS; remaining jobs are not yet certified.
The previous84fa full gate remains failed, not retroactively relabelled green.


### Current client gate and D1 admission candidate — 2026-11-07

Sourcef144c8f7 dev-client job128084354047 in CI42273662478 passes1592 cases,
0 failures,1 ignored. The new required-catalog guard regression actually executes
and passes; both strict dev client clippy variants pass. Core/fmt/clippy, Android,
Python, bots and headless gameplay also pass. PostgreSQL full public journeys
are still running at this observation, so no full green gate is claimed.
UI-2/UI-3 exact native pixels remain unaccepted. Root free space fell to11GiB;
approved A/B caches are already on Orico. Release macOS jobs are terminal
cancelled without steps/artifacts; no Android device is currently attached.
A resource question is pending while independent work continues.

Prepare the real D1 admission coordinator (bounded anonymous worker/cache,
concrete signed destinations, validated pinned fallback, current policy outcomes,
party affinity and retained owned route). Preserve the observed-newer-policy
in-memory floor through a failed write so later unavailability cannot revive old
open cache. New9 cases are written but unexecuted; formatting/source review pass.
See [scope and remaining integration](game-directory.md#native-admission-coordinator-candidate--2026-11-07).
No live trust or player-routing activation occurs. Compile/tests, build pin config,
legal/region/network/UI integration, actual ownership/two-lobby/shared-discipline
and RU/WAN evidence are still required. #12/P7 remain partial; original ledger
stays62 done,5 partial,1 open. No production dependency or infrastructure change.


### D1 coordinator cases executed; C4 local recovery mechanism verified

Source8c9f9351 dev-client CI42277199327/job128096469173 actually executes all9
new admission cases: PASS. Full client1601 PASS,0 FAIL,1 ignored; strict dev
client clippy variants and Android compilation PASS. [Exact source/job scope](../evidence/directory-admission-2026-11-07/summary.json).
Core/PostgreSQL complete gate remains pending. This verifies the coordinator,
not startup/queue/policy-UI integration or any live signed directory deployment.

C4 now has a real isolated PostgreSQL18.3 physical-base + archived-WAL rehearsal
with actual career10/portal11 tables. Explicit direct-SQL synthetic receipt/profile
fixtures are disclosed as unplayed. From a base rating1000/history0, point recovery
returns1024/history1 and latest-archive recovery1048/history2; required post-base
WAL is absent from the base itself. Removing it makes recovery terminate because
the named target was not reached. All5 checks PASS; all owned Postgres processes
stop and temporary database files are removed.1MiB WAL is a fixture bound, not
production sizing/RPO. [Runnable exact source/report/logs](../evidence/pitr-2026-11-07/summary.json).

Initial v1 incorrectly treated read-only hot-standby readiness as completed
promotion and was rejected. Corrected proof waits for the writable end state;
the missing-WAL case waits for the actual terminal recovery failure. Production
backup/WAL code/configuration is unchanged: encrypted signed off-host base/WAL
sets, archive/check/restore integration, retention/alerts, actual off-host drills
and approved installation remain required. C4 stays open; C2 host-controller
integration and all P7–P9 requirements stay in scope. Original ledger62/5/1.


### Corrected full current pre-directory gate; build-trust candidate

Sourcef144c8f7 CI42273662478 completed SUCCESS across all7 jobs and18 actual
PostgreSQL/public stages. Verified498521-byte artifact against GitHub SHA256,
checked all report states and all10 profile awards independently: natural default-
map Ranked victory466.798s, ±24 rating, expected XP, completed-public and seasonal
credits. Readiness15s/late acceptance, party departure/replacement, crash, remake,
surrender and the new Casual handoff continuation all pass. [Exact compact proof](../evidence/full-ci-2026-11-07-f144/summary.json).
This closes the previously fresh-f144 gate;84fa remains recorded failed. Later
8c9/b4/configuration code does not inherit this result automatically.

Prepare bounded compile-only public trust profiles and required concrete fallback
config, plus desktop `--check-directory-trust` audit before all game/account
initialization. Runtime environment cannot substitute pins; configured missing
regions and malformed data fail instead of becoming legacy. Cache scopes include
region/contour; weak Ed25519 points are rejected. New5 cases include an actual
subprocess runtime-override attempt; they remain unexecuted at this checkpoint.
Formatting/source review pass; root disk remains below20GiB and heavy local
compilation is not launched. No public key/configuration is actually provisioned.

[Configuration format and remaining binding](game-directory.md#build-trust-configuration-candidate--2026-11-07).
The parser/audit alone does not activate D1. Actual legal/startup/queue/policy UI,
exact-IP transport, stored owning-lobby Resume, party scope and shared cross-host
discipline still need end-to-end integration/proof. Existing saved Resume's static
startup-address check must be preserved/replaced with authenticated ownership
before any new lobby selection. #12/P7 remain partial; original ledger62/5/1,
C2/C3/C4 and all RU/physical/capacity/final release requirements stay in scope.


The same candidate adds current-authority saved-owner lookup before session
binding: keep the original lobby port and worker IP under that owner's current
signed/pinned destinations, canonicalize mapped addresses/known legacy labels
without DNS, reject foreign/removed/expired-secondary/cache-failed authority.
New-search policy/weights do not select a different lobby for Resume; actual
worker compatibility and saved allocation/epoch checks remain mandatory. Four
additional regressions (9 total in this candidate) await execution. Retained
address values alone are explicitly not authorization for a fresh transport.
The running network/Resume/policy UI is still not wired to this candidate.


### Build trust packaging boundary correction — 2026-11-07

Source57a93ad2 CI42284439321 fails one Python gate: the new compile-time key was
not in the release tool's scrubbed namespace. The existing test correctly refuses
ambient build settings that could leak between releases/contours. Rename the
client/test setting to `OMOBA_DEFAULT_GAME_DIRECTORY_TRUST`, covered by the
existing OMOBA_DEFAULT_ scrub rule; do not weaken or edit the packaging guard.
All29 DevContour checks PASS using Python3.13, including the actual failing
source-scan/scrub case. The initial system-Python3.9 run cannot execute one existing
PEP604 test annotation and is rejected; no source workaround is added for it.
Formatting/diff checks pass. Rust/new9-case CI still needs its current outcome.

This also corrects the earlier packaging assumption: direct Cargo may receive a
reviewed public bundle explicitly, while release packaging deliberately clears
ambient values. A separately prepared/reviewed explicit package option is still
required before D1 can ship configured. No release/workflow file, live key or
service is changed. The full57a gate remains failed until corrected source passes.


### Compiled trust and owning-worker regressions executed; formatter correction

Sourcea7116c00 CI42285913875 dev-client job128126563753 completes SUCCESS:
1610 PASS,0 FAIL,1 ignored. Logs explicitly execute all5 config and4 new Resume
cases, including subprocess runtime-environment isolation, exact owner/worker
validation, removal/expiry/cache failure, and DNS-free multihomed canonicalization.
Both strict dev-client clippy variants, Android compilation, Python packaging,
bot and headless jobs PASS. PostgreSQL/public journeys are still running at this
checkpoint. [Current run](https://github.com/o-moba/omoba-bevy/actions/runs/42285913875).

The overall gate is not accepted: job128126563519 fails its formatter before
core clippy/tests because the renamed environment-setting test now exceeds the
formatter's line width. The preceding local-formatting claim did not establish
that the final renamed line was formatted. Apply the exact rustfmt layout and
rerun the whole formatter/diff checks: PASS locally, without any compilation.
Fresh corrected-source CI remains required; no change to workflows or guards.

Directory startup/refresh transport binding is being implemented locally; it is
not yet connected or claimed to compile. In particular, the new networking
resource draft is not a release candidate. Original68 ledger remains62 done,
5 partial,1 open; C2/C3/C4, UI-2/UI-3, capacity/RU/physical requirements remain.
Root free space is now about7GiB, so no heavy local build is started under the
20GiB rule. No production service, key, deployment or CI configuration changed.


### Native directory runtime connected — candidate, verification pending

Implement the configured directory path in NetworkingPlugin/session lifecycle,
recovery persistence, outbound command dispatch, update selection and Home/loading
UI. Start/fetch only after legal/region acceptance, dial concrete trusted IPs,
prefer the saved original owner before new selection, retain ownership for party
returns and reconnect, and wait on current authority without losing allocations.
Retry no longer drops an allocated seat while a worker handoff is deferred.
Guard new admission before search interception while preserving an active FIFO
request/cancel and existing worker recovery. Offline boot/return stays usable;
invalid configured trust does not become legacy. [Runtime details and exact test
scope](game-directory.md#native-runtime-integration-candidate--2026-11-07).

Nine networking plus two Home/update cases are added, not yet accepted as run.
Initial library check PASS; all-target clippy first found a private test import,
then two test initializer-style issues and an invalid status/weight fixture,
all corrected. Final-source all-target strict client clippy and no-default-
features strict library clippy PASS. The11 test bodies compile; execution and
current full CI still need proof. Formatting/diff and all29
DevContour packaging checks PASS locally; changed EN/RU/ZH dictionaries have
matching key sets. Local root briefly21GiB then19.5GiB: stop further heavy local
builds under the20GiB rule. No new dependency or CI/deployment/key change.

Corrected108d147d CI42289126392 is live: dev-client, Android, Python, bot and
headless and core jobs pass; actual PostgreSQL/public journeys still run
at this checkpoint. Its formatter passes, so the previous formatting failure is
corrected, but the full run is not yet accepted. The runtime candidate is later
source and needs a fresh gate of its own. Original ledger remains62 done,
5 partial,1 open; C2/C3/C4, UI-2/UI-3, physical, scaling and RU release requirements
remain. No release/deployment readiness is claimed from this candidate.

[Exact source fingerprint and validation scope](../evidence/directory-runtime-2026-11-07/summary.json).


### Native runtime regressions executed; shipped CJK subset corrected

Source97732182 CI42292829206 dev-client job128150400206 actually executes all11
new networking/Home/update cases: PASS. The overall dev-client run is rejected:
1620 PASS,1 FAIL,1 ignored, with the existing complete-dictionary CJK guard finding
missing兼(U+517C) and维(U+7EF4). This is a packaged font defect, not a passing gate.
Rebuild only the installed Serif SC subset from the same pinned upstream face,
retain every prior codepoint, add the two glyphs (+844bytes), and update its byte
counts. Actual production Rust cmap parser over all shipped dictionary values
with exact is_cjk ranges: PASS; whole installed asset size/manifest check: PASS.
Do not synchronize unrelated dirty omoba-ui assets or change Chinese wording to
avoid the guard. [Exact source/job and font proof](../evidence/directory-runtime-2026-11-07/summary.json),
[font hashes/coverage](../evidence/directory-runtime-2026-11-07/font-fix.json).

Fresh corrected-source CI is required. Native pixels/physical acceptance and all
remaining original/critic/RU requirements are unchanged; original ledger62/5/1.


### C4 encrypted pgBackRest/SFTP recovery mechanism verified — still open

PG18.6 + pgBackRest2.58.0 in two fresh local containers actually stores an
AES-256-CBC physical backup through SFTP with a pinned host key. Actual career10/
portal11 schemas and explicitly direct-SQL synthetic fixtures recover1000/0 →
1024/history1 at a named restore point and1048/history2 at the latest archive.
Missing and corrupted post-base encrypted WAL both refuse target recovery.
A wrong SFTP host pin produces structured repository status99; an unreachable
archive fails the live check. Seven cases PASS; all owned clusters/containers/
network are removed and absence is checked. The143MB development image is kept
for upcoming C4 integration checks. [Exact runnable proof and logs](../evidence/pgbackrest-2026-11-07/README.md).

Earlier failed/incomplete attempts are rejected; v4 strengthens the first six-case
checkpoint with actual cipher/structured pin status, corruption and cleanup.
This is two containers on one Docker engine, not independent failure domains or
RU storage/RPO evidence.1MiB WAL is only a fixture bound. The repository uses
pgBackRest native encryption/checksums; no new GnuPG-signature claim is made and
existing signed logical backup tooling is unchanged.

C4 remains OPEN: prepare the actual host-kit configuration, backup/retention,
archive, health/alert and isolated restore-drill integration; then obtain approval
for the concrete new production package/kit patch under AGENTS and perform the
separate approved independent-storage/RU rollout. No production dependency,
secret, service, host configuration or workflow is changed by this proof.
Original ledger62 done,5 partial,1 open and every remaining physical/RU/final
requirement stay in scope.


### C4 full host-kit review candidate and current full CI

The complete corrected715a8117 CI42295758507 passes all7 jobs, including the
actual18-stage PostgreSQL/public journeys and1621/0/1 dev-client results. The11
directory/Home/update regressions and full CJK coverage pass. Downloaded artifact
SHA/size checked; all10 AI-played Ranked profile receipts independently checked
for rating±24 and public/rated/season increments. Fifteen-second unanimous ready
timeout/FIFO replacement passes. [Exact current gate and limitations](../evidence/full-ci-2026-11-07-715a/README.md).

C4 now has a concrete [15-file review patch](reviews/C4-PITR-2026-11-07.md),
not applied to tracked host tooling: combined signed logical and encrypted
physical/WAL backups, escrow, retention, minute checks/alerts, isolated drill,
required readiness and SFTP unknown-link-count correction.156 unit tests
(155 pass,1 optional E2E skip); actual PG16.15 andPG18.6 complete combined
rehearsals each18 PASS. Missing/corrupt WAL, wrong host pin, outage, repaired
health, independent logical preservation and notification-state isolation are
actually exercised. [Runnable exact proof](../evidence/pitr-kit-2026-11-07/README.md).

C4 remains OPEN: new production package/kit integration requires owner approval
under the user-provided AGENTS. No host installation, secrets, archive settings,
service restart, workflow or RU deployment occurred. Actual independent-storage/
RU recovery, C2/C3, UI-2/UI-3, scaling and final release acceptance remain. The
original68 ledger remains62 done,5 partial,1 open; no remaining requirement waived.


### C2 complete host-drain review and combined C4 proof

The [C2 host patch](reviews/C2-DRAIN-2026-11-07.md) composes on the exact unapplied
C4 candidate:28 new cases PASS;184 combined tests (183 pass,1 expected optional
E2E skip). Real host methods use temporary files and simulated OS commands, not
actual RU services. [Exact source hashes and logs](../evidence/drain-host-2026-11-07/README.md).

Fix two discovered draft defects: equal-timestamp old reports cannot acknowledge
a restarted lobby; cancellation failures record the operation and observed marker
state. Post-removal sync failure returns resume unconfirmed/nonzero rather than
falsely claiming drain held. Ship/server rollback support bounded --drain; failed
backup/migration/start/health/timeout keep admission paused. Dry run/preflight are
read-only; unexpected restarted workers hold admission.

C2 stays PARTIAL: owner approval under the supplied AGENTS is required for host
deployment-tool changes. The pending C4 question does not authorize C2 or real
RU operations. Tracked integration/full CI and actual service acceptance remain.
Original68 ledger62/5/1 and other physical/capacity/RU gates are unchanged.


### D1 public input audit implemented; execution pending CI

Add an operator-only --check-directory-trust-input production|dev over bounded
stdin, using the exact existing native parser/curve/destination guards before
account/network/settings/cache/UI initialization. Input success explicitly says
it has not checked compiled binary trust; the separate compiled audit is still
required. Strict audit arguments cannot silently launch the game or certify an
ignored input file. Four regressions added; formatting/diff PASS, execution/full
current CI pending. [Source fingerprints and limits](../evidence/directory-input-audit-2026-11-07/summary.json).

This is a prerequisite for safe explicit packaging, not completion of #12/D1.
No release.py/workflow/mobile packaging edits, public key provisioning, published
directory or service changes. Both C4/C2 review patches remain unapplied pending
owner answers. Original68 ledger62/5/1 and RU/physical/capacity gates unchanged.


### Shared dodge storage verified; cross-host wiring remains open

Career schema11 adds separate shared queue-dodge receipts/penalties. Six new
actual PG18.6 scenarios and all69 store tests pass with no ignored cases. Strict
test-target clippy and formatting pass. Duplicate/concurrent/conflicting events,
clock drift, old delivery,24h quiet reset and schema upgrade are verified; career/
history and played-match penalties are preserved. [Exact proof](../evidence/queue-discipline-store-2026-11-07/README.md).

Runtime event/outbox/access/start wiring, shared ready ownership/deadlines, legacy
floor reconciliation and actual signed two-lobby fault proof remain required.
The current lobby still uses local dodge storage: do not mark D1/#12 complete.
The task's public test fixture is now career11/portal11 (supporter suite migrates
it); final verification sees11 before/after and zero owned scratch schemas.
Repeat C4 real recovery on11; older schema10 proofs are historical. Standard full
current CI/PG16 and every RU/physical/capacity requirement stay open.

The prior1f39528a input-audit dev-client CI128196318370 executes all4 new cases:
PASS,1625 total pass,0 failures,1 ignored; its full7-job run is still pending at
this checkpoint and cannot certify this later schema/API change.


### Durable dodge delivery verified; global admission still open — 2026-11-07

Wire stable local dodge events through a separate typed, bounded, locked durable
journal into schema11 SQL. Retain producer copies until exact payload commit ACK;
include this journal in shutdown flush without creating Interrupted match history.
Corrupt local penalty state now closes new search and stays preserved; storage
failure pauses ready checks and preserves other players/parties/FIFO.

All60 renderer-free native backend tests PASS,0 failed/ignored, including all7
actual PG cases. New coverage:16 journal units,5 backend ACK units and3 actual PG
worker outage/restart/full-channel/corruption cases. Strict native Clippy,
formatting and diff checks PASS. Source DML grants pass a rolled-back temporary
NOLOGIN ACL proof, with zero role/profile/event leftovers. [Scope and evidence](../evidence/queue-dodge-delivery-2026-11-07/README.md).

MatchService/engine tests and full current CI remain required. Parent1d1deb79
CI42309434476 now reports all7 jobs SUCCESS, including the public-journey job;
that earlier result does not certify this newer implementation. No fresh journey
artifact inspection is claimed here. Shared cooldown reads/final durable-start,
ready ownership/deadlines, legacy floor reconciliation, signed two-lobby fault
proof, schema11 recovery and typed-journal health remain open. Original68 ledger
stays62 done,5 partial,1 open. C2/C4 questions remain pending; no RU rollout.


### Shared new-search and durable-start checks verified — 2026-11-07

New search now uses shared queue discipline on both serial/parked and batched
backend access paths. Normal account access/Resume remains separate. New durable
allocation checks queue penalties after participant profile locks and rolls back
all match/participant/active/season rows on refusal. Existing immutable allocation
retry/adoption is preserved. A refused Start is retired through the durable
rejection path so late terminal replay cannot start it after cooldown expiry.
The client maps its stable dodge code to the existing Russian localized notice.

All72 actual-PG18.6 store tests and all63 native backend tests PASS,0 failures/
ignores; strict store/backend test-target Clippy and formatting/diff PASS. New
proof includes three store cases (fresh cross-owner read, rollback/Resume and an
observed real in-flight event/profile-lock race), two actual-worker PG cases and
one backend dispatch unit. First71/1 store attempt exposed missing source grants
in the restricted-role fixture; corrected, cleaned only its exact disposable
schema/two NOLOGIN roles and reran72 PASS. [Exact scope/proof](../evidence/queue-admission-2026-11-07/README.md).

Fresh full engine/client CI and rendered current Russian pixels remain required.
Previousef9c865b CI42316037246 was still running at this checkpoint, with Android,
Python, harness and bot jobs passed; it does not certify these newer changes.
Shared ready ownership/deadlines BEFORE cross-host re-search, legacy floors and
actual signed two-lobby/party/FIFO/Resume fault proof remain open. So do C2/C4
approval/actual RU recovery, typed-journal health and physical/capacity gates.
Original68 ledger stays62 done,5 partial,1 open; no acceptance criterion waived.


### Durable ready ownership contract; correlation repair — 2026-11-07

Schema12 now stores check ownership, immutable profile/session membership,
responses and unique profile claims. Reserve precedes Publish; the stored window
is exactly15000ms and retries cannot extend it. An expired healthy check writes
only missed seats' stable events/penalties and releases claims in one transaction;
failed/unpublished owners and failed handoffs free players without invented
strikes. Sorted profile/check locks guard new admission; existing allocation
recovery remains separate. [Exact scope/proof](../evidence/ready-ownership-2026-11-07/README.md).

Nine new actual-PG scenarios and all81 store tests PASS,0 failures/ignores; all64
native backend cases and strict store/backend Clippy PASS. Restricted game-role
lifecycle and portal refusal are exercised. The owned fixture is career12/portal11;
no per-test schema/role remains. Additive migration/strict runtime schema guard
requires the matching migrated schema and binary for release; no rollout occurs.
Older schema10/11 recovery proofs remain historical; real schema12 recovery is open.

Parent1e587a39 CI42317119362 has6 successful jobs, but PG/public job128232347108
FAILED at party-leave after store72/server534 PASS. Exact artifact13551717666,
68440bytes SHA256a440a26df733050702b06e65001e2603ba5523e8363dc92c298b6892dba59b01
shows nine retained survivors and a correctly denied dodger whose Failed/current
request was overwritten with local Idle/null. Preserve the correlated store
refusal; clear it on new search/cancel/success and ignore stale replies. Native
regression PASS; unchanged public-journey rerun/engine/client current CI pending.

This contract is NOT yet active lobby consent: wire bounded off-tick reserve,
publish/answer/poll/cancel jobs and ACKs, expose consent only after SQL publication,
require committed unanimity before spawn and bind worker result ID to its check.
Use the SQL penalty snapshot without a second local event. Actual signed two-
lobby15s/party/FIFO/restart/Resume proof, legacy floor reconciliation, ready/journal
health/retention and every other physical/capacity/release/RU gate remain open.
C2/C4 questions remain pending; original68 ledger stays62 done,5 partial,1 open.

### Committed ready consent active in lobby; local cross-lobby proof — 2026-11-07

Production lobby now runs bounded off-tick reserve/publish/answer/poll/cancel
jobs with exact SQL acknowledgements. No ready UI before publication; no worker
before committed unanimity; immutable account/session/search binding, sticky
explicit decline and15s SQL deadline. Timeout discipline updates the local cache
without another event. Worker result ID equals the consent allocation ID.

A real draft journey exposed old Approved claims blocking the next roster. Hold
claims until pending draft-dodge SQL ACK, then release them. The unchanged five-
stack journey now reaches a replacement Ranked match Running with nine retained
peers, FIFO age and no false career/history credit. Two independent live lobbies
share ownership/cooldown correctly across19 profiles/one duplicate endpoint; no
worker before unanimity and no foreign penalty-file copy. Maintenance/restart/
Resume also PASS on an actual two-arena BotPractice process pair. Ranked setup
uses disclosed synthetic qualification receipts, not ten real played games.

All551 actual whole-server tests and82 actual-PG18.6 store tests PASS, zero
failures/ignores. Full native server test/production and store strict Clippy,
formatting, four Node policy self-checks and5 tooling tests PASS. The actual full
server target is linked without copied engine modules, using the read-only pinned
cache graph; no heavy graphics rebuild under the disk floor.
[Exact proof/scope](../evidence/ready-runtime-2026-11-07/README.md).

Parent39d2fd53 PG/public CI now PASS; core/dev-client failed E0716 in a translation
test, repaired separately in ba4bba35. Current full CI/client execution is pending.
Actual multi-host/WAN failure/Resume, legacy floors, ready/journal retention/health,
schema12 recovery, trust packaging/provisioning, physical/capacity/RU gates and
existing C2/C4 owner decisions remain open. Original68 stays62 done,5 partial,
1 open; this checkpoint waives no acceptance criterion and performs no RU rollout.

### Legacy penalty floors now import before new search — 2026-11-07

Capture each persisted local aggregate at lobby startup; drain its known typed
pending events before importing the immutable snapshot through a bounded worker.
New admissions/selections wait for exact SQL ACKs; existing allocations keep their
Resume path. An adopted draft's newer snapshot cannot be cleared by an old ACK.
The store merges strikes/deadlines by maximum under the shared profile lock,
never creates fictional event history and never reanchors an expired cooldown.
A subsequent real dodge escalates normally. Allow the existing5s source-clock
skew and cooldown-view second rounding without accumulating grace on replay.

All87 real-PG18.6 store cases and555 actual whole-server cases PASS, zero
failures/ignores; strict store/full-server test+production Clippy and targeted
script checks PASS. Five new SQL and four native behavioral cases cover the
migration/retry/error/concurrency paths. The live signed two-lobby regression
covers active+expired versionless snapshots, a new decline and restart, and is
included in the existing full PostgreSQL harness.
[Exact scope and reports](../evidence/legacy-penalty-floors-2026-11-07/README.md).

Aggregate floors are lower bounds, not a reconstruction of unknown historical
events: merge overlapping old sources by maximum, never invented sums. Actual
rollout must import every authoritative old host snapshot before opening global
admission. C2 tooling approval/application and this real RU rollout gate remain
open, as do fresh full CI, multi-host/WAN failure/Resume, ready/journal health and
retention, schema12 recovery, trust provisioning and physical/capacity gates.
Original68 remains62 done,5 partial,1 open; no remaining criterion is waived.

### Public admission rejects missing consent; restore evidence is retained — 2026-11-07

Both public roles now select mandatory consent from their server recovery scope;
client/receipt fields cannot choose unmanaged standalone admission. Public Start
and unknown-allocation recovery require Approved SQL consent. Rated starts need
Ranked consent, all10 Ranked profile seats and current eligibility; fresh recovered
admission also rechecks discipline under profile locks. Failed transactions leave
no match/participant/consumed-consent or career credit. Existing immutable SQL
admissions still Resume without another vote, including after ready-row removal.

Fresh unacknowledged starts and queued terminal retries retire on definitive
refusal. All records loaded from disk retain restore evidence if matching SQL
admission is missing, including Start, Checkpoint and old terminal receipts whose
newer flags are absent/false. The replay cannot invent an admission; restoring the
exact previously committed SQL admission permits replay with preserved statistics.
Explicit unmanaged standalone APIs retain their existing separate behavior.

All92 actual-PG18.6 store cases and559 complete actual-server cases PASS, zero
failures/ignores; strict store/server test+production Clippy and formatting PASS.
Five new SQL cases, three actual worker cases and a public-scope unit cover these
boundaries. Worker restore cases use isolated disposable schemas; partial-restore
simulation toggles only their own guards inside one transaction. Failed fixture
preparation attempts are rejected and documented. No production guard changes.
[Scope, exact evidence and live journeys](../evidence/public-consent-admission-2026-11-07/README.md).

An intermediate parallel native run under combined SQL/build load also failed the
unchanged full-reply heartbeat timing assertion. Its diagnostic target and current
complete serial run PASS with the original assertion; no threshold is relaxed.
The full run matches scripts/postgres_tests.py's test-threads1. Controlled host/SQL
load cadence remains part of the open capacity acceptance, not proved by serial
regression. Safe ready-row retention is still to implement; the new mandatory
admission guard is its prerequisite. Real schema12 PITR/receipt-restore operation,
C2/C4 owner decisions and RU rollout, current full CI, multi-host/WAN/Resume,
trust provisioning, typed-journal health and physical/UI/capacity gates remain
open. Original68 remains62 done,5 partial,1 open; no criterion is waived.


## 2026-11-07 — receipt health and journal ownership

Repeated identical settlement delivery refreshed mtime, which could indefinitely
hide an unsaved result from the five-minute monitor. Preserve exact durable bytes
and mtime on retry; terminal alerts also use original ended_at_ms. Typed queue
journals use observation-anchored ages, bounded regular-file reads, exact native
record validation, corruption/capacity/incomplete-scan alerts and anonymous counts.
The real Rust producer and read-only Python monitor pass an interoperability case;
no external alert is sent by tests.

CI42330926635 at031833d7 exposed Busy while reopening a retired queue journal.
A deterministic duplicated-descriptor case reproduces the original failure. An
explicit lock guard releases ownership on retirement and failed open/scan, while
a second active writer still fails; closing an old duplicate cannot release its
replacement's lock. All564 actual whole-server tests (13 actual-PG cases),108
monitor/host-tool tests and strict native test/production Clippy PASS.
[Exact proof and limitations](../evidence/outbox-health-2026-11-07/README.md).

Parent6f6a9544 CI42327346048 is fully green in7 jobs; its downloaded PostgreSQL
artifact passes SHA256/ZIP/CRC checks and all20 stages, including actual cross-lobby
ready discipline, legacy import and full Ranked victory. This is historical parent
evidence, not certification of the new source. The current full CI must pass.

This closes the persisted-receipt monitoring defects, not ready/floor availability
health, safe retention, pre-persistence failures or sustained heartbeat/capacity
acceptance. Career12/portal11/protocol16 are unchanged. Current-schema PITR,
UI/physical, trust/WAN/multi-host/RU gates and existing C2/C4 decisions remain open.
Original68 stays62 done,5 partial,1 open. No RU changes or deployment.


## 2026-11-07 — bounded ready maintenance (schema13)

New partial indexes bound expired-check and closed-receipt lookups. Maintenance
reconciles at most128 expired checks under existing sorted profile/check locks;
healthy missed answers charge exactly one existing discipline event, failed
ownership/handoff cancels without a dodge. Busy locks defer. A narrow pinned
owner function removes at most128 closed receipts older than7 days, excluding
claims and unfinished matches; members cascade, event deduplication/history and
immutable SQL admissions/results remain. Runtime has function EXECUTE, no direct
consent DELETE; PUBLIC/portal are denied and temp-table shadowing is tested.

Lobby/standalone starts one async32-check/minute task after300s warmup; each run
has a5s deadline and skips missed intervals. Match workers do not schedule it.
All100 actual-PG18.6 store cases and564 actual whole-server cases (13 real-PG),
strict native test/production/store checks and formatting PASS. The fresh signed
15s timeout/FIFO/replacement journey PASS uses schema13.
[Exact source, recovery invariants and runtime proof](../evidence/ready-retention-2026-11-07/README.md).

Owned public fixture migrated12→13; isolated upgrade preserves live membership
and penalty state, with no leaked per-test schemas. Paired013 migration, runtime
function grants and binaries are required; a schema12 binary is not rollback.
Schema13 physical/WAL/off-host recovery and restored rights still need rehearsal;
older schema10/12 evidence is historical. Ready/floor SQL availability health,
pre-persistence failures, controlled heartbeat/load cadence, trust/WAN/multi-host,
UI/physical/RU and existing C2/C4 decisions remain open. Original68 remains
62 done,5 partial,1 open. No RU migration, grant application or deployment.


## 2026-11-07 — pre-persistence health and startup reconnect recovery

Career workers publish atomic disposable health/status.json every5s, outside the
receipt format, with anonymous authenticated SQL preflight/freshness, journal
availability and staged-dodge/legacy-floor/unpersisted-result counts. Schema13,
writable primary and actual required table/function privileges are checked by a
10s background probe with2s deadline. No profile/event IDs, DSNs or raw errors.
The monitor requires live reports after startup grace, matches worker epochs,
alerts on sustained120s blockers, stale/missing/corrupt data or bounded-scan limits,
and uses existing repeat/resolution state. Finished/failed workers are skipped.
Ship producer/monitor through the reviewed drained rollout; old live workers need
to finish before required reports become mandatory. No new unit or recipient.

A real startup/schema/IO failure test exposed shared connection/result retry-clock
starvation: delivery ticks kept postponing reconnect after schema restoration.
Separate clocks fix it. Actual worker now recovers schema, persists and ACKs its
previously in-memory dodge once, then detects/reverses a second connected schema
failure. The two rejected fixture schemas are verified/removed; no other schema
is changed. All101 real-PG18.6 store,570 actual server (14 real-PG),42 monitor/74
host-tool cases and strict native checks PASS.
[Evidence and exact limits](../evidence/storage-health-2026-11-07/README.md).

The ready-check harness now requires the replacement ten-human roster actually
Running, then fresh authenticated health from both real processes with exactu64
epoch matching. The accepted rerun PASS uses the lossless native packet parser;
a rejected initial harness used lossy JSON Number parsing and is not acceptance.
Prior reports only proved Assigned for that replacement; current proof closes that
validation gap without changing the15s consent/FIFO rule.

Parentaa2e4cb4 CI42334461242 is fully green in7 jobs. Downloaded artifact13559679363
matches SHA256/size and safe ZIP/CRC checks; all20 public stages PASS with private
fixture removed. This historical schema12 source is not current acceptance. New
full CI, schema13 real PITR/off-host/permissions/replay, generic blocking-reply and
controlled heartbeat/cadence/load, public trust, WAN/multi-host, UI/physical and RU
gates remain open. C2/C4 decisions remain pending; C4's patch still applies cleanly.
Career13/portal11/protocol16 unchanged; original68 stays62 done,5 partial,1 open.
No RU migration, grant/settings/credential change, alert send or deployment.


## 2026-11-07 — bounded critical completions and full-receiver liveness

Replace the two remaining blocking Ready/Flushed sends with an exact FIFO of at
most256 critical completions. Reserve room for a whole current Ready batch before
intake; pause intake with10ms async timer yielding when the budget is tight. SQL,
leases, pending record/journal retries and operational reports continue. Keep
work/sequence, result errors, flush generation and success unchanged. Known closed
recipients abandon undeliverable callbacks; SQL/outboxes retain durable authority.
Shutdown still polls before accepting a flush barrier. Input/output channel caps
are unchanged. No new pool/thread, dependency, DB schema or game protocol.

The real red test leaves only2.987s of a15s active-match lease after12s while output
is full. The fixed version passes the unchanged>60s threshold and delivers both
Ready and Flush. The stronger seeded stress first ACKs Start/removes its spool,
then retains256 critical completions and bounds extra input. Its only lease renewal
comes from heartbeat work;82.964s remain after the same12s hold. All256 original
work sequences and exact barrier arrive after reading resumes; expired unpublished
consent stays cancelled, cannot start a public match and gives no game/penalty.
The unseeded fixture's legitimate ownership-retry refusal is rejected as setup;
only the seeded pass is acceptance, with both rejected schemas verified/removed.
[Exact code/proofs/limits](../evidence/critical-replies-2026-11-07/README.md).

All575 actual server cases (16 real-PG), strict native test/production checks and
42 monitor/74 host-tool cases PASS. Unchanged store retains its101-case proof.
Anonymous health v2 adds critical backlog and known closed receivers; the monitor
checks256 capacity and includes aggregate counts. Native report/parser checks and
current signed15s/FIFO/ten-human Running with fresh v2 health PASS. Actual worker
SIGTERM flushes durably and records Interrupted, with no ranked rewards.

Parent1e7607b7 CI42336326091 is fully green across7 jobs; artifact13561237197 matches
SHA256/size/safe ZIP/CRC and all20 public stages with private fixture removed.
This is historical schema13 retention proof, not current backpressure/v2 acceptance.
Fresh full CI, schema13 physical/WAL/off-host/rights/replay, sustained host cadence
and RU/WAN capacity, public trust packaging/provisioning, actual multi-host faults,
current UI/physical and C2/C4 decisions remain open. Original68 stays62 done,
5 partial,1 open. No RU migration/settings/credential/grant change or deployment.


## 2026-11-07 — current-schema13 physical recovery and restored rights

The unchanged review-only C4 helper executes actual combined encrypted logical
and physical/WAL backup, named/latest restoration and fault checks against current
career13/portal11 migrations on both PG16.15 and18.6. All20 checks per version
PASS. Post-base synthetic SQL transitions preserve exact ready membership/claims,
consent approval/consumption, queue event/cooldown, full migration histories and
rating/history at both points. Source/restored game and portal roles execute the
actual current grants; retention works through its definer function, while direct
consent/event deletion, portal retention and season closure are refused. The
verification transaction rolls back. Both own container/network sets are removed.
[Exact runner, hashes, reports and limitations](../evidence/schema13-recovery-2026-11-07/README.md).

An initial evidence-packaging assumption incorrectly expected portal version8,
which no migration declares; declaration parsing and exact recovered histories
were checked, without changing/rerunning any integration assertion. Current
schema recovery is now evidenced locally, rather than borrowed from schema10.
These are direct SQL fixtures on one local engine, not played-game, runtime
authentication, actual worker replay/Resume, independent storage or RU RPO proof.
C4 stays open and unapplied; existing C2/C4 decisions and approved rollout remain
required. Full current CI, load/WAN/trust/UI/physical and original68 ledger remain
unchanged. No RU setting, migration, credentials, grants, alert or deployment.

Parentbc08eb52 CI42339705987 completed all7 jobs successfully. Its521888-byte
artifact13562612775 matches SHA256/safe ZIP/CRC and all20 public stages, with
private fixture removed. It is historical startup-health proof; a654 critical
reply/v2 full CI42343372154 is still running, not yet accepted.


## 2026-11-07 — actual worker physical restoration and C4 limit regression

The original proposed C4 drill hardcodes20 connections. A source with80 cannot
recover using20; native physical reproduction fails, and inspection confirms the
same constant in the original helper. The revised15-file review package preserves
max_connections, max_worker_processes, max_wal_senders, max_prepared_transactions
and max_locks_per_transaction using the greater validated current-primary/base-
control value. Unknown/missing/duplicate/malformed/injected values are refused.
An isolated current-source application passes176 unit cases with1 optional skip;
C2's patch also applies after it, without claiming combined behavior or approval.
[Revised proposal](reviews/C4-PITR-PARAMETERS-2026-11-07.md).

PG18.6 source dimensions80/12/14/4/128 now pass the actual combined C4 helper
drill and all backup/fault stages. A separate real signed public bot-practice
game uses the current native binary, actual game grants and SCRAM login. After
Running/durable gameplay checkpoint, a physical backup/WAL restore replaces the
owned SQL source behind a loopback proxy. Source identity and known admission
survive; the same authenticated player resumes the same worker/profile/epoch
with advancing snapshots and a SQL probe strictly after the switch. Lobby restart
adopts the worker. Actual SIGKILL then passes ordinary recovery deadlines in
223.47s, records one Interrupted match without Elo/XP and allows new assignment.
The final restored-SQL read agrees. All22 stages PASS and all three own containers/
network are removed. No graphics build, production code/dependency or RU change.
[Exact runner, manifests, reports and limitations](../evidence/worker-physical-recovery-2026-11-07/README.md).

Rejected preparation remains explicit: raised source versus hardcoded20, Docker
internal-network port publication, and canonical true gameplay serialization being
omitted. The immutable draft allocation is not the gameplay marker; the current
checkpoint follows shared MatchResult's true default. Those runs are not native
acceptance. The accepted new SQL probe must postdate the actual restore.

This proves PG18 SQL/checkpoint recovery, not terminal-spool replay (the pre-crash
JSON outbox was empty), PG16 raised-limit/native behavior, independent off-host/RU
RPO/RTO, physical devices, Ranked party/WAN or sustained capacity. Those, current
CI, public trust and existing C2/C4 decisions remain required. Original68 is still
62 done,5 partial,1 open; the host proposal is unapplied and no RU setting, grant,
secret, migration, external alert or deployment has been changed.


## 2026-11-07 — PG16 physical worker acceptance and agent handoff

PG16.15 with80/12/14/4/128 dimensions passes the same22 stages asPG18: actual
combined backup/PITR helper, current grants, physical SQL replacement, same signed
human/worker/epoch Resume, lobby adoption and221.63s actual SIGKILL recovery.
One Interrupted history/profile credit, no Elo/XP, final restored-SQL agreement
and all own container/network cleanup are verified. The first PG16 run accepted
read-only connection readiness too early; the current bounded loop waits for
actual promotion before routing writable traffic. Earlier assertions and scope
remain unchanged. Both exact runner revisions and the rejected run are preserved.
[Reports](../evidence/worker-physical-recovery-2026-11-07/README.md).

The next gate remains actual durable terminal-file replay through restored SQL,
including exact payload preservation and a real duplicate redelivery. A draft
protocol probe is saved as explicitly UNVERIFIED review data, not acceptance.
The complete next-agent entry point is
[HANDOFF-LOBBY-2026-11-07.md](HANDOFF-LOBBY-2026-11-07.md), including all six open
OMOBA PRs, approved decisions, existing C2/C4 questions, local helpers and commands.
Full CI and original68/critic/UI/physical/WAN/host/trust/RU gates retain scope.


## 2026-11-07 — actual durable terminal spool and duplicate physical replay

A real public bot-practice match reaches Running through current grants/SCRAM.
After its physical backup/WAL archive the owned proxy disconnects SQL; actual
SIGTERM emits SHUTDOWN_METRIC terminal_outbox=durable and a Settle JSON. Source
SQL remains running with no terminal intent. Physical restoration preserves known
admission; automatic recovery ACKs the file and saves Interrupted. A second actual
public recovery process receives the identical original bytes, ACKs them and
returns a correlated SQL Profile still at one played credit with unchanged
Elo/XP/wins/losses/rated count. Both recovery deadlines remain ordinary120s; a new
assignment is accepted only after retirement. Total SIGTERM→new assignment248.22s.

Direct restored-SQL comparison matches identity/end timestamps/duration/outcome
and all ten participants' stats. The separate whole-payload verifier proves
**every serialized field** equal except saved=false→true. This closes the local
terminal-file gap that earlier empty-outbox SQL/checkpoint runs did not cover.
All22 PG16 stages PASS; source/binary/probe/runner/receipt hashes and cleanup are
verified. No new Rust/dependency/graphics build or RU change.
[Actual data/runner/verifier](../evidence/terminal-spool-replay-2026-11-07/README.md).

The saved prototype remains historical review data; its missing duplicate delivery
was completed in the actual probe, not reinterpreted as a passing Profile reread.
C4/C2 remain unapplied proposals. Current full CI, independent off-host/RU RPO/RTO,
current UI/physical, public trust/WAN/Ranked parties and sustained host capacity
retain their gates. Original68 stays62 done/5 partial/1 open. Handoff updated.


### Current-source load diagnostic and rejected1000 acceptance — 2026-11-07

[Exact reports and reproduction](../evidence/load-cadence-2026-11-07/README.md).
Current production source/binary are unchanged from efe32101 (263 hashes checked).
Native AI10/100 controls pass30s with received medians17.96/18.01Hz. A new
restricted-runtime PG18.6/current13/11 fixture passes10-client30s:17.7648 accepted
snapshots/s versus18.7442 producer-tick span/s (fraction0.94775). This is local
unoptimized direct-rustc source, not Cargo dev/release performance qualification.

The same fixture's1000-client120s request FAILS before a full Running population:
ordinary sign-in9.47s, stable Search release14.53s, owned process exit109.69s.
No soak/cadence result exists. Mixed cached phases and busy/auth/cooldown/pending
errors are retained. Native Connection-refused panics were read after teardown;
the original runner did not record the first exiting PID, so neither these
secondary observations nor their last sample times identify the first cause.
This rejection supersedes any assumption that frozen78070b27 acceptance proves
current1000 operation. It does not retrospectively falsify that old exact scope.

Expose last accepted producer metadata per native peer and require unchanged
protocol/epoch/match plus consistent tick/count deltas. Retain initial samples
and first observed owned-process exit codes before cleanup. Seven Python cases,
one real UDP transport regression including a deliberately missed tick, three
strict native Clippy targets, formatting/diff checks and archived-data verifier
PASS. Final early-validation/baseline/first-exit edits postdate the live run;
no fresh live acceptance is claimed for them. Both owned DB containers/volumes
were removed; no existing DB settings, production code or RU infrastructure changed.

Next reproduce with pre-cleanup exit/worker health/status observations, identify
admission/auth/SQL/timer versus generator failure, then measure an explicitly
optimized current binary. Sustained1000/cadence/latency and RU uplink/host remain
open. Original68 stays62 done,5 partial,1 open; critic/UI/physical/WAN/RU retain
full scope. Existing e70b6657 native UI build predates UI-2/UI-3; no matching
current pixels or heavy build on root13GiB. Full CI for efe32101 remains live;
this source packet requires its own subsequent CI.


Follow-up in the same packet: the original rejection's finally block itself
failed with PermissionError on killpg, leaving owned processes alive after DB
cleanup. Exact traceback and manual-cleanup inventory are now archived. Revalidate
UID/executable/owned groups, signal86 actual owned processes individually, and
verify the final owned-group inventory is empty. No foreign process was touched.
Fix stop_owned so one group denial cannot skip other children; individually stop
its live Popen child, bound final waiting and explicitly report unverified group
cleanup after all cleanup attempts.14 rehearsal cases PASS, including two real
child lifecycle regressions with injected group-signal denial. This is a separate
cleanup defect; it does not establish the first1000-client admission failure.
