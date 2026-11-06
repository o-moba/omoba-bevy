# TODO: LOBBY — matchmaking review artifacts (2026-11-05)

Dump of the 2026-11-05 ranked matchmaking review so nothing is lost between sessions. Source of truth for the plan: [ranked-launch-readiness-2026-11-05.md](ranked-launch-readiness-2026-11-05.md); decisions: ADR-027 in [../architecture/decisions.md](../architecture/decisions.md); progress: [../progress/2026-11-05-ranked-matchmaking-readiness.md](../progress/2026-11-05-ranked-matchmaking-readiness.md). Raw reviewer output (JSON, local only): `.agent/tasks/RANKED-LAUNCH-READINESS-20261105/raw/review-findings.json`.

Branches: game `feat/ranked-matchmaking-readiness` (this repo, PR into `dev`), portal `feat/ranked-matchmaking-readiness` → o-moba/omoba-web#33. Disposable current-schema PostgreSQL: Docker `omoba-mm-seasons-pg` on 127.0.0.1:55582 (`omoba_test`, career10, portal11). Legacy prototype fixture `omoba-mm-test-pg` on55581 is retained separately and is not clean-installation proof. Cargo cache B: `/Volumes/Orico/omoba-build-cache/target-b`.

## Method

Eight independent read-only reviewers (lobby queue, allocation/start, rating/settlement, client flow, admission/scale for ~1000 players, parties, portal/Account API, tests/harness), two-lens adversarial verification of each finding (factual, material), one completeness critic. 77 raw → 68 confirmed findings (3 blocker, 23 major, 42 minor; 11 bugs, 24 gaps, 20 scale, 13 ux) plus 9 critic items. Status below as of the end of 2026-11-05.

## What works and must not be rebuilt

- [LOBBY QUEUE AND ROSTER SELECTION] Queue policy is correct and tested: Ranked = exactly 10 humans of one rated cohort (same newcomer bracket, <=300 spread from the anchor's lowest rating), never bot-filled; Casual = every human regardless of cohort, bot fill once any member has searched Casual >= 30 s; vs Bots allocates at once (server/src/match_service.rs:869-930, tests 1146-1200, 1370-1414). Rated flag comes only from the queue (`MatchPreference::rated`, shared/src/match_service.rs:29-33), so 10 humans in Casual stay unrated.
- [LOBBY QUEUE AND ROSTER SELECTION] The lobby and the store share one cohort policy (`fits_cohort`/`same_cohort`, career-store/src/matchmaking.rs:19-34; `validate_current_cohort`, career-store/src/career_store.rs:1199-1211), and the test `a_mixed_party_never_yields_a_rated_roster_the_store_rejects` (match_service.rs:1201-1264) proves the lobby never forms a Ranked roster the store would reject.
- [LOBBY QUEUE AND ROSTER SELECTION] Team split is exhaustive and deterministic: parties stay together, head counts as even as possible, then minimum rating-sum difference over all 2^units masks (`split_teams`, match_service.rs:801-862). The career-store Matchmaker uses an exact subset-sum DP for the same purpose (matchmaking.rs:324-358).
- [LOBBY QUEUE AND ROSTER SELECTION] Ordering is FIFO by a monotonic `order` and never changes compatibility; a Ranked roster is always taken before a Casual one, so a Ranked player who took the Casual offer cannot be stolen by Casual while a full Ranked roster exists (match_service.rs:877-928, test 1266-1346). The 300-spread never widens (matchmaking.rs:19-23; test `newcomer_and_huge_skill_gaps_never_widen_with_wait`, matchmaking.rs:461-494).
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
| 5 | major | bug | One player's Leave/Cancel in the draft aborts the match for all 10 (even in Casual/vs Bots where bots could fill the seat) | `server/src/career_runtime.rs:1096` | M: in the worker's `take_cancelled` path, for non-Ranked manifests drop the cancelling human from the required set (a `released: HashSet<session>` consulted by `allocated_humans_ready` and `authorize_allocated_join`) instead of writing cancel.json, so `fill_practice_bots` seats a bot; write cancel.json only when the manifest is Ranked or when no human remains. | done minimum — others re-queued with their place; bot-fill of the vacated seat (worker side) not done |
| 6 | major | bug | A live worker that never writes a heartbeat is never killed; its 10 players are stuck on 'подготовка арены…' until lobby restart | `server/src/match_pool.rs:289` | S: in `Pool::poll`, when `child.is_some()` and `status.is_none()` for more than e.g. 20 s after spawn (track `spawned_at` in `Slot`), or when `now > manifest.join_deadline_ms` with a non-terminal status, call `child.kill()`, reap it and write a `Failed` status so the normal error path (`allocation_cancelled_or_expired`) runs. | done — lobby kills a worker with no receipt after 20 s / past its join deadline |
| 7 | major | bug | One player's Cancel (search or draft) aborts the allocated match for all 10, others see a raw code | `server/src/match_service.rs:459` | Minimum: (1) in the lobby, on cancel of an Assigned/Allocating profile remove only that seat and re-queue the remaining humans at the front with their original `order` (match_service.rs cancel + tick) instead of pool.cancel; (2) add ru/en `queue.failed.allocation_cancelled_or_expired` ('Один из игроков вышел. Ищем матч снова…') and `queue.failed.profile_has_another_session`, `already_queued`, `queue_full`; (3) draft  | done minimum — re-queue + localized codes; explicit acceptance implemented; native visual evidence verified in #11 |
| 8 | major | bug | Party member's Cancel leaves the rest of the party searching forever with no message | `server/src/match_service.rs:756` | In cancel(): when the cancelled profile has `party: Some((id, n))`, remove every waiting entry with that party id and set `errors[member] = "party_member_left"` for them; add ru/en `queue.failed.party_member_left` ('{name} вышел из поиска. Начните поиск снова.'); alternatively recompute `n` from present members on the next FindMatch so the party searches as 4. | done — party searches on without the member; party_member_left notice |
| 9 | major | bug | arenas_per_ip=1 refuses the second vs-Bots player behind the same NAT with 'Из вашей сети уже идёт матч' | `server/src/match_service.rs:227` | S (env): OMOBA_LIMIT_ARENAS_PER_IP=50 (max 100, admission.rs:185-189) in lobby.env. M (code): key the anti-abuse limit on device key / profile id instead of source IP, keeping IP only as a secondary signal. | done — OMOBA_LIMIT_ARENAS_PER_IP=8 in the RU kit (code keyed on IP unchanged) |
| 10 | major | bug | Launching or accepting an invite while a member is already Searching another queue deadlocks the party | `server/src/match_service.rs:395` | In party_tag (party.rs:627-632) or enqueue, exclude from the party tag a member whose existing Waiting preference differs from the party's launch preference (treat it as solo), or refuse Launch/Accept with a PartyError when a member is currently queued and surface it. Add a match_service test: party {Casual-queued, Ranked-queued} still seats each as before. | done — member in another queue is seated alone |
| 11 | major | gap | No 'Match found / Accept' step: the client jumps straight from Searching to the worker | `client/src/match_service.rs:528` | Add MatchServiceView::ReadyCheck{deadline_secs, accepted, needed} on the lobby before spawning a worker; client searching screen shows 'Матч найден' with a countdown and Принять/Отклонить; declines/timeouts leave the queue, the rest keep their queue age. Keys: searching.ready.title/accept/decline/waiting in ru+en. (LR-32) | done — explicit signed unanimous 15-second check before worker spawn; native RU 852×393 capture verified with countdown and both controls (product-ready-ui-db-confirmed). Mobile feedback is tracked separately in C3. |
| 12 | major | gap | No horizontal path: one baked lobby address, workers only on the lobby's host, directory plan unscheduled | `docs/plans/game-directory.md:3` | M-L minimum viable path: (1) D0 bake a DNS name (play.omoba.ru) instead of the IP; (2) D1 signed directory JSON on the portal listing N lobbies (host, port, weight, status) that all point OMOBA_DATABASE_URL at the same RU PostgreSQL (the career store settles rating per result, so ratings stay consistent); client picks one at start and falls back to the baked name. Each lobby keeps its own match root and coordinator l | open — game directory / second host (LR-57); DNS name for the lobby |
| 13 | major | gap | Arena capacity never measured on the RU host; probe cannot simulate more than 100 clients | `ops/vps/ru/README.md:533` | M: run the probe on the RU host (or an identical VPS) with --limits-from omoba-lobby-ru.service for --preference=bot_practice and a --mix, fill README:545-548, set capacity/cgroup/nftables from it. S: lift the 100-client assert to 1000 and allow --capacity up to 100 with `--mix` so a 100-arena run is possible on a bigger box. | open — LR-09 measurement on the RU host; probe now allows --allow-large up to 1000 clients |
| 14 | major | gap | No end-to-end Ranked match: terminal → settle → rating → history is never exercised on the real lobby+worker binary with PostgreSQL | `scripts/test_public_mvp.mjs:544` | Add `harness/tests/ranked_journey.rs` (#[ignore], OMOBA_HARNESS_DATABASE_URL): (1) migrate with `harness::server::prebuilt("migrate-career")` (admission.rs:238-249); (2) `ServerProcess::spawn_with_env` with the lobby env from registration_journey.rs:221-235 (OMOBA_SERVER_ROLE=lobby, OMOBA_MATCH_MODE=release, OMOBA_MATCH_CAPACITY=1, OMOBA_MATCH_FIRST_PORT, OMOBA_MATCH_BIND_IP/PUBLIC_HOST=127.0.0.1, raised OMOBA_LIMIT_ | done — real ten-client signed AI match completed in 406.115 s on canonical rules; all ten saved results, fresh profiles, ±16 rating, 150/100 XP and exactly one history row verified. Reusable `public_bots_rehearsal.py --require-victory`; no map/clock/damage override. |
| 15 | major | gap | One player's cancel during handoff/draft aborts the whole 10-player match; the other 9 get a raw code and lose their queue place | `server/src/match_service.rs:459` | On cancel of an assigned profile, remove only that human from the manifest (bot-fill the seat for Casual/vs Bots); for Ranked abort the allocation but re-insert the other nine into `waiting` with their original `queued`/`order` and `casual_since`, give the leaver a short requeue cooldown, and add the ru/en/zh `queue.failed.allocation_cancelled_or_expired` text ('Один из игроков покинул матч, ищем снова…'). | done minimum — see #7 |
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
| 28 | minor | bug | Interrupted Ranked match is labelled «Не рейтинговые» in history; single-match tag uses the plural filter word | `/Users/wotori/git/ekza/_workspace/worktrees/mm-web/src/components/portal.tsx:108` | Row: `m.rated ? (m.rating ? delta : '—') : w.unratedShort`; tag: add singular keys `ratedOne:'Рейтинговый'`, `unratedOne:'Без рейтинга'` (the game's career.result.rated/unrated wording) and use them at portal.tsx:128. Add a presentation test for rated:true/rating:null. | mostly refuted — singular tag fixed on the portal; interrupted ranked rows show — |
| 29 | minor | bug | Greedy unit packing misses feasible party combinations | `server/src/match_service.rs:893` | Within a cohort window, search subsets of units (a cohort has at most ~20 units; DFS over sizes with the `splits` pruning, preferring the lexicographically oldest `order` sequence) for a combination totalling 10 before falling back to the partial roster; add a test with [4,4,3,3,2,2]. | done — exact bounded two-team packing, [4,4,3,3,2,2] Ranked/Casual tests |
| 30 | minor | bug | Spawn failure leaves the roster flapping between Failed and Waiting and leaves a dead allocation directory on disk | `server/src/match_service.rs:653` | S: on allocate error, remove the selected entries from `waiting` (so the error is sticky until the player cancels/retries explicitly) and call `archive()` on the failed directory; add RU text for `worker_spawn_failed`. | done — spawn failure dequeues the roster, archives the directory, 10 s hold |
| 31 | minor | gap | Harness PostgreSQL tests (300-client admission, registration journey) are never run by any automated gate | `.github/workflows/ci.yml:120` | In ci.yml `postgres` job add after the Python step: `cargo build --locked -p server` then `OMOBA_HARNESS_DATABASE_URL=$OMOBA_TEST_DATABASE_URL cargo test --locked -p harness --test admission -- --ignored --test-threads=1` (and the new ranked_journey test). Make admission.rs/registration_journey.rs fall back to OMOBA_TEST_DATABASE_URL when OMOBA_HARNESS_DATABASE_URL is unset, and add a `make test-postgres-harness` target. | done — owner-approved CI gate passes all 13 stages (1097.16 s, e3b0942/run 42005385554), including admission/registration and full Ranked Victory. Artifact inspected: fixture removed and no private-key file uploaded. |
| 32 | minor | gap | No leaderboard / ranking surface anywhere (portal, API, DB) — rating has no social meaning | `/Users/wotori/git/ekza/_workspace/worktrees/mm-game/account-api/src/lib.rs:554` | Minimal LR-47 slice. (1) career-store migration 006_ladder.sql: `CREATE INDEX career_profiles_ladder ON career_profiles(rating DESC, rated_matches DESC, profile_id)`. (2) account-api/src/read.rs: `pub async fn leaderboard(app,&Option<Session>,query)` → top 100: `SELECT p.profile_id,p.nickname,p.rating,p.rated_matches,p.wins,p.losses FROM career_profiles p JOIN portal.profile_settings s USING(profile_id) WHERE s.publi | done — GET /v1/leaderboard, portal page, opt-out (portal schema 11, career schema 6) |
| 33 | minor | gap | No leaderboard: no index on career_profiles(rating), no API/portal endpoint (LR-47) | `/Users/wotori/git/ekza/_workspace/worktrees/mm-game/career-store/migrations/postgres/001_career.sql:5` | Migration 006: `CREATE INDEX career_ladder ON career_profiles(rating DESC, profile_id) WHERE rated_matches >= 20` (plus SCHEMA_VERSIONS bump); account-api GET /leaderboard returning top-100 nickname/rating/rated_matches honouring portal.profile_settings.public_profile_enabled (or nickname-only), and a portal page with RU copy; optionally the client Home shows 'Ваше место #N'. | done — see #32 |
| 34 | minor | gap | Leaver on the winning team gains full rating and 150 XP (LR-33) | `/Users/wotori/git/ekza/_workspace/worktrees/mm-game/career-store/src/career_store.rs:731` | Minimal: in finish_inner, when `participant.disconnected && won` set progression_xp_gained = 0 and clamp the rating delta to <= 0 (keep the loser's normal loss), and surface this in the receipt (client already has career.player.progress_disconnected). Add a career_store_tests case. Full LR-33 (connected-time tracking, abandon counter + cooldown) is a later L. | done — 90s participation/AFK policy, individual leaver loss/no XP, teammate half-loss and exactly-once PostgreSQL cooldowns. 593 PostgreSQL/API/store/server tests and current native tests pass; commit 0db576fc CI 42061098787 is green in all seven jobs and all13 real public journeys. Artifact inspected/hash verified. No production deployment; remake/surrender and remaining P6 stay separate. |
| 35 | minor | gap | Fixed K=32 with no placement/newcomer K (LR-53) | `/Users/wotori/git/ekza/_workspace/worktrees/mm-game/career-store/src/matchmaking.rs:410` | Pass `rated_matches` into rating_changes_for_players and use K=48 while `rated_matches < NEWCOMER_MATCHES` (per player, winner/loser clamped independently as today), update the test expectations and the number in docs/match-progression.md. Tiers/seasons remain LR-53. | partial — K=48 for pre-match rated_matches<20, then K=32 from locked profiles; boundary/mixed-K/bounds and real PostgreSQL 20th→21st/idempotence tests pass. all 42 store tests (real PostgreSQL included) and all-target clippy pass. Current complete-gameplay CI 42024828738 passed: standard-map ten-human victory, exact +24/150 and -24/100 awards for fresh profiles; tiers/seasons remain in P6. |
| 36 | minor | gap | RU guest home explains the modes but never explains the rating itself | `/Users/wotori/git/ekza/_workspace/worktrees/mm-web/src/components/guest-home.tsx:62` | Add a fourth landing section «Рейтинг» (two sentences from match-progression.md:186-195: старт 1000, меняется только в рейтинговых матчах, первые 20 матчей — группа новичков, обычный PvP и боты дают опыт) with ru/zh entries in landing-translations.ts, and assert it in tests/ru-landing.e2e.mjs. | done — RU landing rating section |
| 37 | minor | gap | Newcomer/calibration state is a bare tag with no progress or explanation | `/Users/wotori/git/ekza/_workspace/worktrees/mm-web/src/components/portal.tsx:200` | Render `Новичок · {rated_matches}/{newcomer_matches} рейтинговых матчей` with a title/tooltip «Первые 20 рейтинговых матчей играются среди новичков; рейтинг пока калибруется» on /me and as a one-line notice above the chart on /me/stats; keys in ru/en/zh. No tiers, per spec. | done — Новичок · n/20 with tooltip on /me and /me/stats |
| 38 | minor | gap | The public-lobby probe is manual-only: no Makefile target, no CI job, RU host capacity never measured | `Makefile:56` | Add `make verify-public-mvp` (requires OMOBA_TEST_DATABASE_URL, builds the server, exports HARNESS_SERVER_BIN, runs `--scenario=lifecycle` and `--mix=bot_practice:5,quick:5,humans_only:10 --max-wait-secs=60 --seconds=10`) and a CI step in the `postgres` job that runs the same two with `--seconds=5`. Before launch, run ops/vps/ru/README.md 'Arena capacity' steps 1-4 on the RU host and record CAPACITY_RESULT lines in d | done — make verify-public-mvp + CI step |
| 39 | minor | gap | Rating is not shown on Home before queueing; no tier, rank or leaderboard anywhere in client or portal | `client/src/frontend/home.rs:355` | S: render `profile.rating` and `rated_matches` (with 'Новичок · {n}/20' when < NEWCOMER_MATCHES) on the Home profile plate next to wins/losses (data already in HomeSignature) and on the Ranked button subtitle. M: add a /leaderboard page to the portal from `career_store` ratings (top 100 + 'your place') and a client button opening it via career_web; tiers/seasons remain LR-53. | partial — Home phone rating/placement, Ranked subtitle and trusted regional ladder link implemented; baseline client gate passes; native Home capture remains pending. Tier/season policy remains explicit P6 (LR-53). |
| 40 | minor | gap | Fill bots cannot join the public signed lobby, so a developer cannot form a 10-human Ranked match on the dev/RU lobby by hand | `harness/src/bot.rs:46` | Give `harness --bin bots` a `--public` mode built on the shared public_client helper (finding 1): generate N throwaway keys, probe/authenticate, FindMatch `humans_only`/`quick`, follow the handoff, auto Lock/Loaded, then hand the running socket to the existing `BotBrain` lane AI. Document `make bots BOTS=9 BOTS_SERVER=127.0.0.1:4000 BOTS_PUBLIC=1`. | done — `bots --public` signs admission/account/gameplay requests, accepts the exact ready roster, follows handoff, locks/loads and plays. Ten distinct authenticated clients in one rated worker verified on loopback/PostgreSQL; reusable rehearsal added. |
| 41 | minor | gap | Worker-crash recovery is proved only for vs Bots; a rated match that loses its worker has no process-level test | `scripts/test_public_mvp.mjs:36` | Relax line 36 to allow `--scenario=recovery --preference=humans_only --clients=10` (each client is already independent) and extend the recovery check loop (505-515) to iterate over all clients' profiles; reuse the existing `careerProgress` snapshot (220). Also allow `--preference=quick` so the 30 s bot fill path is covered. | done — ten signed Ranked humans, worker killed 2.10 s after Running; durable played interruption, all ten profile/history checks and fresh requeue pass. Initial allocation rated=true, canonical ruleset, zero bots confirmed in PostgreSQL. |
| 42 | minor | gap | Allocated workers cannot be brought to a legitimate Victory in a test: no sanctioned accelerator survives the worker env scrub | `server/src/match_pool.rs:418` | Add a loopback-only passthrough: when `OMOBA_MATCH_BIND_IP` is loopback (match_pool.rs already distinguishes exposed vs loopback lobbies, test `exposed_lobby_refuses_a_loopback_or_unset_public_worker_host` 608), forward `OMOBA_MATCH_TEST_MAP_CONFIG` to the worker as `OMOBA_MAP_CONFIG`; an exposed lobby ignores it and logs a warning. Add a unit test next to 517 asserting an exposed lobby still strips it. The harness t | done — explicit test-map forwarding requires both lobby and worker loopback binds; IPv4/IPv6/exposed guard tests pass. Ten signed humans completed a custom-map Victory in 33.629 s with zero Elo/XP and saved history; full probe wall time 50.87 s. |
| 43 | minor | gap | Party Ranked and leave-during-draft have unit tests only; no process-level path exists and the probe cannot form a party | `server/src/match_service.rs:1348` | In the shared `public_client` helper from finding 1 add `invite/accept` via the existing social CareerActions so a 5-stack can queue; add a `--party=5` option to the probe; add one harness scenario where peer #3 stops sending during Draft and assert the other 9 see `PrematchPhase::Draft` restart (today) or the LR-32 re-queue once it lands. | done — signed five-stack plus solos, explicit Leave and silent disconnect, all nine receive the return code, FIFO/group retained, all ten career/history unchanged, only explicit leaver cooled once, fresh unanimous Ranked roster reaches Running. Process evidence below. |
| 44 | minor | gap | No invite throttle, block or cooldown; a stranger can keep an invite toast on Home | `server/src/party.rs:254` | In PartyState keep `declined: HashMap<(from,to), Instant>` and refuse re-invites for 60 s after a decline, plus at most 10 invites per minute per sender (new PartyError codes surfaced via the error field above). Full block list stays LR-39. | done — decline cooldown 60 s, 10 invites per minute |
| 45 | minor | gap | Parties and queue vanish silently on every lobby restart/deploy | `server/src/party.rs:5` | Show a one-line notice ('Соединение с лобби было восстановлено, группа распущена') when PartyClient clears a non-empty party on epoch change; persistence of parties (LR-55) stays out of this iteration. | partial — global one-shot restart toast implemented for party and solo search, preserves party-loss evidence through stale-view expiry; functional regression and baseline full client gate pass. |
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
| 65 | minor | ux | Hero is picked twice: HeroSelect before Searching, then again in Draft | `client/src/frontend/home.rs:1318` | For public-lobby queues skip HeroSelect: build the JoinPrematch from the profile card's `main_class`/current avatar (ProfileCard already on Home, home.rs:1131-1184) and go straight to Searching; keep HeroSelect for Offline practice. | partial — public Home and party launch skip the first picker; Home action regression passes. Home/party functional regressions and baseline full client gate pass; native Home0/10 pixels PASS; complete Home/party-to-single-Draft visual flow remains pending. |
| 66 | minor | ux | Worker dies during handoff/draft: player sees only 'Нет связи · Переподключение…' for up to 3 minutes | `client/src/net/session.rs:850` | While `flow.allocation.is_some()` and the match has not started, keep polling the lobby view in parallel (FindMatch with the same request_id is already idempotent) and, on `Failed{allocation_cancelled_or_expired}` or a terminal slot, run LeaveMatch + show 'Сервер матча недоступен. Ищем матч снова…' and re-queue automatically; or shorten the prematch reconnect window to ~20 s. | done (client agent) — 20 s pre-start window, then re-queue |
| 67 | minor | ux | Party command errors are only logged on the server; the player sees nothing | `server/src/party.rs:584` | Add `#[serde(default)] error: Option<String>` (stable code) to PartyView, set it for the issuing viewer on the immediate reply, and add `party.error.<code>` strings in ru/en/zh (e.g. 'Игрок уже не в сети', 'Группа заполнена', 'Приглашение истекло'); render it under the online list for ~5 s. | done — PartyView.error codes localized |
| 68 | minor | ux | Mate in a match shows 'Отошёл' on the public lobby instead of 'В матче' | `server/src/party.rs:652` | In send_party_views on a lobby, build `in_match` from the match service: profiles with a live allocation (lobby.assignments) mapped to member profile_ids; keep `away` for members with no presence and no allocation. | done — in_match from the lobby allocations |

## Critic items (missed by the eight reviewers)

- [major/bug/M] **Lobby crash or deploy SIGTERMs every live match worker; workers have no signal handler, so all running (incl. Ranked) matches die and settle as unrated 'server_interrupted'** — `ops/vps/ru/omoba-lobby-ru.service:42`. Fix: Set `KillMode=process` (and `SendSIGHUP=no`) in omoba-lobby-ru.service and the ru_host.py unit rules so only the lobby PID is stopped and workers are adopted on restart (the code path already exists); add a SIGTERM handler in the worker that writes a checkpoint and settles Interrupted gracefully for the real host-shutdown case; add a release test that restarts the lobby unit under a running bot ma — **done in kit — KillMode=process preserves live workers; SIGTERM/SIGINT handler fsyncs terminal records, forces final status publication, process rehearsal passes (Interrupted, no Elo/XP, one played match). RU rollout remains pending.**
- [major/gap/M] **No drain/maintenance mode: deploy is refused while any match runs, so at ~1000 concurrent players the server can never be updated or hot-fixed** — `ops/vps/ru/ru_host.py:1562`. Fix: Add a lobby drain flag (e.g. `OMOBA_MATCH_ROOT/drain` file or SIGUSR1) that refuses new allocations with a localized `searching.match.draining` ('Сервер готовится к обновлению · новые матчи скоро возобновятся') while live matches finish; add `ru_host.py ship --drain` that sets the flag, waits for `occupied == 0` (bounded by OMOBA_MATCH_MAX_DURATION_SECS), then restarts; keep `guard_no_matches` as  — **partial — runtime pauses new ready/allocation, preserves queued parties/FIFO without dodges and publishes operation acknowledgement.6 new server/656 real PG checks, native retry/localization and signed live/restart/adoption/Resume proof PASS. Host ship/wait/cancel integration, fresh full CI and RU rollout remain required.**
- [major/ux/M] **Mobile player who locks the screen while Searching silently loses the queue place after 15 s and is re-queued at the back on resume; no keep-awake and no match-found sound/vibration** — `server/src/match_service.rs:668`. Fix: Keep the screen awake while in Searching/Draft on Android (`ANativeActivity_setWindowFlags(AWINDOW_FLAG_KEEP_SCREEN_ON)` via the ndk crate) and iOS (`isIdleTimerDisabled`); raise the lobby expiry to 60 s and have the client, on resume with a gap >15 s, show `searching.status.resumed` ('Поиск был прерван · продолжаем'); play a short SFX + vibration when the view becomes allocated/assigned. — **partial — FIFO intent60s, foreground keep-awake/resume refresh, ready sound and approved iOS/Android haptics implemented. Android compile and all7-job CI42097650577 PASS with the approved VIBRATE/JNI patch; current native/mobile visual and physical keep-awake/haptic verification remain pending.**
- [major/scale/M] **Daily pg_dump only: a host failure rolls every player's rating and history back up to 24 h; no WAL archiving or point-in-time recovery** — `ops/vps/ru/omoba-backup-ru.timer:7`. Fix: Enable `archive_mode=on` with an `archive_command` that ships WAL segments to the existing off-host backup target (pgBackRest or wal-g; or rsync as a stopgap), keep the nightly base dump, and extend `ru_backup.py check`/the restore drill to replay WAL to the latest point; as a one-line stopgap raise the dump cadence to hourly. — **open — WAL archiving / PITR (daily pg_dump only)**
- [major/gap/L] **No remake or surrender** — original finding at `server/src/session_recovery.rs:200`. Server-owned remake before180s after90s absence; strict majority of remaining humans, surrender from600s with4/5 (all when fewer remain),30s votes/120s failed cooldown. Innocent remakes are neutral; proved ranked leavers retain their individual penalty. — **partial — implemented/pushed7fd41139+b5a6b9ba; 608 real PG tests, both ten-human real-time process proofs and1571 native CI tests pass. Full15-stage CI42074945255 passes; native visual proof still pending.**
- [major/gap/S] **Ranked has no eligibility gate** — original finding at `server/src/match_service.rs:892`. Owner decision2026-11-06: ten completed public matches, including Casual/vs Bots; no mandatory VK. Separate authoritative `completed_public_matches` excludes interruptions, remakes, custom/offline practice and own proved leaves/AFK. Fresh search and locked allocation admission; Home progress; existing seats still resume. — **done in software — ten-public-match/noVK gate passes locked admission and real tenth-completion proofs, including current5aa all7-job/18-stage CI42132894282. Fresh protocol16 native RU852×393 Home0 shows disabled Ranked/progress; Home10 shows enabled Ranked/rating. Initial clipped caption was corrected and both actual pixel captures rechecked. Production rollout remains separate.**
- [minor/ux/S] **Draft allows two players on one team to lock the same hero class; no per-team uniqueness or 'taken' feedback** — `server/src/prematch.rs:235`. Fix: In `handle_prematch` refuse `Select`/`Lock` whose `hero_class` is already locked by a joined teammate, setting `player.draft.error = Some("draft.error.hero_taken")` resolved client-side to 'Этого героя уже выбрал союзник'; grey the class card in the client draft when a teammate has locked it (draft snapshot already carries teammates' picks). — **partial — server Select/Lock/async/timeout/opt-out enforcement and localized native Taken/disabled/stale-action guards implemented; signed ten-human draft-to-play proof and618 PG/API/server tests PASS. 13 focused native tests, final client clippy and full7-job/18-stage CI47ee63ae PASS; current native Taken visual remains pending; manual unlock was confirmed and the current native build is running.**
- [minor/scale/S] **nftables lobby rule is 400 pkt/s per source IP: ~250 players behind one carrier-grade NAT saturate it and the kernel drops FindMatch/presence for all of them** — `ops/vps/ru/nftables-omoba.conf:56`. Fix: Raise the lobby limit to e.g. 4000/s burst 8000 (it is a flood guard, not a fairness tool) or key the dynamic set by `ip saddr . udp sport` so each NAT'd client gets its own bucket; document the new figure in README 'Admission limits'. — **done — nftables lobby rule 4000/s burst 8000**
- [minor/gap/S] **No alert when match results pile up unsettled (PostgreSQL slow/down): players sit on 'Сохранение результата…' and nobody is paged** — `ops/vps/ru/ru_monitor.py:435`. Fix: In `ru_monitor.py` add `settlement_problems`: count `*.json` receipts older than N minutes under `/var/lib/omoba-ru/matches/*/` worker outboxes and the lobby outbox, alert when > 0 for 5 min; optionally have the lobby write `pending_settlements` into capacity.json so the existing report path carries it. — **done in kit — bounded terminal-outbox scan, >=5 min threshold, corrupt/oversized/limited-scan alerts, no active-checkpoint false positives; 24 monitor tests pass. RU installation remains part of deployment.**

## Next steps, in order

1. Finish the game PR into `dev` after `make check` + `scripts/postgres_tests.py` + `make verify-public-mvp`. Use the actual continuation results in [the progress record](../progress/2026-11-05-ranked-matchmaking-readiness.md); the initial full-client-gate claim was not reproducible (a private-field test initializer failed to compile and was fixed).
2. Deploy order on RU: `omoba-account-api migrate` (career 9 after C6, portal 11) → Account API → portal (omoba-web#33) → lobby kit (`ru_host.py ship`; the unit now has 16 arenas, KillMode=process, new limits; bootstrap appends the new `OMOBA_LIMIT_*` lines).
3. LR-09: run `scripts/test_public_mvp.mjs` on the RU host (`--limits-from omoba-lobby-ru.service`, `--mix`), fill the README table, set capacity/cgroup from the measurement.
4. LR-32: unanimous 15-second confirmation and persistent host-local cooldown are implemented and network-tested. Native RU ready-screen visual acceptance is verified; cross-host penalty sharing remains P7.
5. LR-33 leaver policy (owner thresholds), LR-34 remake/surrender, LR-37 Ranked gate (owner O-3), LR-53 newcomer K / tiers.
6. Scale beyond one host: LR-57 game directory; LR-28 snapshot egress (~250 kB/s per player today).
7. Harness: end-to-end Ranked journey with a rating delta asserted (#14), harness bots on the signed public lobby (#40), party at process level (#43).


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
