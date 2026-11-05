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

See the PR descriptions for the exact commands and results: `make check`,
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
- The portal leaderboard and settings toggle were not browser-verified against
  a live Account API in this iteration; the API is covered by PostgreSQL tests.
- Protocol: no bump. New fields are additive (`Manifest.cohort`,
  `Status.joined_sessions`, `PartyView.error`); a protocol 11 production client
  is unaffected by the lobby changes but must not be mixed with dev's protocol 12
  servers (unchanged rule).
