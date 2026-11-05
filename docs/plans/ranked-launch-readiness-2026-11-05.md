# Ranked matchmaking readiness for the RU launch — 2026-11-05

Final review of the public matchmaking pipeline (Home → Find match → Searching →
Match found → Draft → Loading → Game → Result with rating → portal history) on
`dev` 2a38b2c (0.45.0), server + client + portal, against the owner's target: a
Wild Rift–quality ranked PvP product for the Russian market with ~1000
concurrent players distributed into matches. Eight independent reviewers (one per
dimension), two-lens adversarial verification of every finding (factual, material)
and a completeness critic; 68 confirmed findings. Raw findings:
`.agent/tasks/RANKED-LAUNCH-READINESS-20261105/raw/review-findings.json` (local).

## 1. Verdict

The core is production-grade and must not be rebuilt: the queue policy (Ranked /
Casual PvP / vs Bots, one cohort policy shared with the store, exhaustive
rating-balanced team split, FIFO order), the lobby → worker handoff (immutable
manifests, atomic receipts, coordinator lock, adoption after a lobby restart),
the durable start gate, in-match reconnect and takeover, team Elo (K = 32, team
average, clamp 0–5000) with idempotent settlement, and the portal's individual
rating story (number, per-match delta, history chart, rated badge, newcomer tag).

What is not ready, in the order it bites at launch:

1. **Pre-start resilience (LR-32).** One player's cancel, no-show or crash before
   the match starts aborts the match for all ten; the other nine see a raw code,
   lose their queue place and are not re-queued. A no-show holds nine players
   up to 180 s. One slow loader resets the whole draft.
2. **Scale constants and ops envelope.** The RU unit ships 2 arenas (20 PvP
   seats); the lobby queue is capped at 128 waiting and 512 recent requests
   (silent drop above); 512 transport endpoints and 512 career clients per
   lobby; 8 endpoints and 1 arena per source IP (carrier-grade NAT in Russia
   breaks that at a few hundred mobile players); every presence heartbeat
   rebuilds and broadcasts every party view (O(N·P) per 2 s per client).
3. **Ranked at low population (LR-35).** The 300-point / newcomer cohort never
   widens; ten Ranked humans across cohorts never form a match and the screen
   does not say why.
4. **Rating has no destination (LR-47).** No leaderboard anywhere; the portal
   mixes «Рейтинг» and «MMR», prints raw `unrated_reason` codes, and the RU
   landing never explains what the rating is.
5. **Localization leaks.** Six lobby failure codes, the prematch reset reasons
   and the unrated reason reach Russian players as English or snake_case.

A 1000-concurrent-player target is **not** reachable on the current 2 vCPU / 4 GB
RU VPS whatever the constants say: snapshot egress (~210–270 kB/s per player) and
the one-host worker pool bound a single host to roughly 20–40 arenas. This
iteration raises every software ceiling to or beyond 1000 and sizes the RU kit
to a defensible 16 arenas (160 PvP seats) with the formula to go further; a
bigger host or a second lobby host (game directory, LR-57) is an ops decision.

## 2. Scope of this iteration (PRs into `dev`)

### Game (`feat/ranked-matchmaking-readiness`)

Server, lobby (`server/src/match_service.rs`, `match_pool.rs`,
`match_allocation.rs`):

- **Re-queue instead of drop (LR-32 minimum).** When an allocation ends before
  its match started (a member cancelled, a no-show passed the join deadline,
  the worker failed or never heartbeated), every other human of that
  allocation returns to `waiting` with their original `order`, `queued` and
  `casual_since`, so they are seated next. The member who cancelled is not
  re-queued; after cancelling a *Ranked* allocation they get a 60 s in-memory
  `dodge_cooldown` (LR-32's escalating persisted cooldown stays later).
- **Party cancel** removes the whole party from the queue and tells the mates
  `party_member_left`. A party member already searching another queue is
  seated as a solo unit instead of deadlocking the party.
- **Arena priority.** Ready rosters take free arenas by seated humans first
  (10-human Ranked/Casual before a 1-human vs Bots), then by age; vs Bots is
  immediate only while arenas are free.
- **Queue caps.** `OMOBA_LOBBY_MAX_WAITING` (default 1024, max 4096) replaces
  the 128 constant; the recent-request map follows the career-client ceiling
  and a dropped request answers `Failed { lobby_busy }` instead of silence.
- **Pool hygiene.** A worker that never writes a status within 20 s of spawn,
  or that passed its join deadline without ever running, is killed and marked
  Failed; a worker with zero joined humans aborts 60 s after boot instead of
  waiting 180 s; a slot whose child exit the lobby observed frees its port at
  once; a spawn failure dequeues the roster and archives the directory; the
  career view reads the cached status instead of the disk four times a second.
- **Ranked window (LR-35).** The cohort allowance grows with the oldest unit's
  wait: rating spread 300 → +100 every 60 s after 2 min, capped at 600;
  the newcomer bracket merges with veterans after 5 min. The allowance is
  recorded in the manifest (validated within the caps); the store's rated-start
  re-check accepts any roster within the maximum window, so the lobby still
  never forms a roster the store rejects and the wire/receipt stay unchanged.
  Parties stay one base cohort.
- **Transport ceilings.** `MAX_ENDPOINTS` and `MAX_CLIENTS` 512 → 2048; a per-IP
  refusal answers `server_full` instead of a silent drop.
- **Codes, not sentences.** `unrated_reason` is `not_completed`; prematch reset
  reasons carry a stable `code` next to the English `error`.

Server, parties (`server/src/party.rs`, `shared/src/party.rs`):

- A presence heartbeat no longer forces an immediate broadcast to every
  subscriber; the periodic 500 ms broadcast builds the online list once per
  pass; state-changing commands answer the issuer (and the invitee) at once.
- Party command errors reach the issuer as a stable code (`PartyView.error`),
  localized in the client; re-invites after a decline are refused for 60 s and
  a sender is limited to 10 invites per minute.
- A mate who is in a live allocation shows «В матче», not «Отошёл».

Client (`client/src/match_service.rs`, `client/src/frontend/mod.rs`,
`client/i18n/*`):

- When the worker rejects the join or the connection is lost before the match
  started, the client returns to the lobby, re-sends its last lock-in and shows
  «Один из игроков не подключился · ищем матч снова» instead of a 3-minute
  reconnect wait or a dead allocation.
- Russian, English and Chinese text for every lobby code
  (`queue_full`, `already_queued`, `allocation_cancelled_or_expired`,
  `profile_has_another_session`, `worker_spawn_failed`, `lobby_busy`,
  `dodge_cooldown`, `party_member_left`, `capacity_busy`), the prematch codes,
  `career.unrated.not_completed`, the party error codes, and a player-facing
  «Сервер не отвечает» line instead of the GAME_SERVER_ADDR developer text.
- Friends are requested on entering the party lobby so friends are marked and
  listed first on the first visit.

Ops (`ops/vps/ru/*`, docs):

- RU kit: 16 arenas (41000–41015), `KillMode=process` so workers survive a
  lobby restart (the adoption path already exists), cgroup limits from the
  per-arena formula, `OMOBA_LIMIT_ENDPOINTS_PER_IP=256`,
  `OMOBA_LIMIT_ARENAS_PER_IP=8`, raised probe budgets, nftables rates sized for
  carrier-grade NAT, PostgreSQL `max_connections` ≥ capacity + 32, the uplink
  budget (arenas × 10 × 250 kB/s ≤ 60 % of the port) recorded in the README.
- `make verify-public-mvp` runs the public-protocol probe (lifecycle + a mixed
  Ranked/Casual/vs Bots load) against a disposable database; the `postgres` CI
  job runs it too.

### Account API + portal (`feat/ranked-matchmaking-readiness` in omoba-web)

- `GET /v1/leaderboard` (career schema 6 index): top 100 by rating among
  profiles past the newcomer bracket that have not opted out
  (`portal.profile_settings.leaderboard_listed`, default true, portal schema
  11), ties by rated matches then profile id, plus the caller's own rank.
  Response: `{"entries":[{"rank","profile_id","nickname","rating",
  "rated_matches","wins","losses"}],"me":{"rank":N|null,"rating",
  "rated_matches","listed"},"min_rated_matches":20,"data_as_of"}`.
  `PATCH me/settings` gains `leaderboard_listed`; `GET me` returns it.
- Portal `/leaderboard` page (nav, mobile nav, RU/EN/ZH), «Рейтинг» instead of
  «MMR»/«XP» literals, localized `unrated_reason` codes, a «Новичок · n/20»
  progress line, a rating paragraph on the RU landing, the 17-class filter,
  the corrected empty-history copy, and `me` polling at 30 s when visible.

## 3. Deferred, with the reason

| Item | Why not now | Where |
| --- | --- | --- |
| Accept/ready-check step before the worker spawns | Needs a new lobby view, client screen and its own tests; the re-queue above removes the player-visible damage of a no-show | LR-32 |
| Leaver penalty, AFK detection, remake, surrender | Owner sets thresholds (O-13); needs connected-time tracking | LR-33, LR-34 |
| Ranked entry gate, same-IP seating, void-match tooling | Owner update 2026-11-06: Ranked requires 10 completed public matches, no mandatory VK link. C6 implementation/acceptance pending; other LR-37 integrity items remain open. | LR-37 |
| Newcomer K, tiers, placement, seasons | Rating model decisions for the owner | LR-53 |
| Casual backfill (humans replace bots) | Roster freeze is load-bearing for settlement | LR-36 |
| Second game host / game directory | Architecture change (D0–D4) | LR-57 |
| WAL archiving / PITR, drain mode, unsettled-result alert | Ops work on the host | LR-26, LR-27, LR-17 |
| Mobile keep-awake while searching | Platform code (Android/iOS) | LR-59 |

## 4. Acceptance

- AC1 `cargo test -p server match_service` covers: cancel after assignment
  re-queues the other nine in order and no worker is handed out to them; a
  Failed slot re-queues; party cancel yields `party_member_left`; priority
  seats a 10-human roster before a solo vs Bots when one arena is free;
  `lobby_busy` and the configurable waiting cap.
- AC2 `cargo test -p omoba-career-store` covers the widening allowance: the
  base policy rejects a 500-point spread; the widened policy and the store
  accept it, including mixed newcomer/veteran rosters. The store re-checks
  against the documented maximum: 600 points accepted, 601 rejected.
- AC3 Party unit tests: a presence does not trigger an immediate broadcast to
  other subscribers; errors reach the issuer; invite throttle.
- AC4 Client tests: a pre-start worker loss re-queues with a localized notice;
  every server code constant has a `queue.failed.*` key in en/ru/zh.
- AC5 `make check` green; `scripts/postgres_tests.py` green on a disposable
  PostgreSQL 18; `make verify-public-mvp` reaches Running for a mixed
  Ranked/Casual/vs Bots load on real lobby + workers.
- AC6 Account API PostgreSQL test for `/v1/leaderboard` ordering, opt-out and
  own rank; portal `npm test`, typecheck and RU build green.
- AC7 `CHANGELOG.md`, `docs/features.md`, `docs/match-progression.md`,
  `docs/public-mvp.md`, `docs/party.md`, `ops/vps/ru/README.md`, an ADR and a
  `docs/progress/` record are updated in the same PRs.
