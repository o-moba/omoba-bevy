# 2026-07-06 — Matchmaking And Gated Match Start (TASK-22)

## Goal

Pre-release match formation: today the first `Join` flips the server straight
to Running — one player "starts" a match alone with client-chosen teams.
Needed: a queue that forms real 5v5 matches with balanced teams and a visible
search state, while keeping the instant-start flow for development, plus a way
for one developer to fill a match with bots.

## Changes

- **Server (`server/src/main.rs`):** `MatchMode::{Release, Dev}` +
  `MatchConfig` from `OMOBA_MATCH_MODE` (default `release`, safe by default)
  and `OMOBA_TEAM_SIZE` (default 5, clamped 1–16). `GameState` gained
  `Forming { ready, needed }` and `Starting { countdown_ms }` phases.
  Release: `Lobby → Forming` on first join, `Forming → Starting` at a full
  `2 × team_size` roster, 3 s countdown → `Running` (boss schedule armed
  there), countdown rolls back to `Forming` on a drop, empty queue returns to
  `Lobby`; joins beyond a full roster are rejected. Teams are server-assigned
  to the smaller side (rejoins keep their team). Dev: first join → `Running`,
  requested team honored (legacy). All gameplay systems already gate on
  `Running`, so pre-match phases idle for free; joined players can walk on
  the base pad as a warm-up.
- **Client:** `GameState` mirror extended; the lobby overlay shows the search
  progression (Searching / Waiting X/Y / Match found + countdown); the local
  spawn no longer waits for the requested team to echo back — it spawns when
  its own joined player appears in a snapshot and adopts the server-assigned
  team (`TeamSelection` updated for camera/HUD).
- **Harness:** protocol mirror gained `GameState` + snapshot `game_state`
  accessor; `ServerProcess::spawn()` now runs the server in dev mode (keeps
  the instant-start assumptions of existing scenarios) with
  `spawn_with_env(...)` for explicit modes; new `bots` binary
  (`--count`, `--server`) with join-resend, ping keepalive, and a small
  wander; new integration test `release_mode_forms_and_starts_a_full_match`
  (`OMOBA_TEAM_SIZE=1`).
- **Makefile:** `server` (release), `server-dev`, `start` (dev quick-start,
  unchanged UX), `start-release`, `play-bots`, `bots` (`BOTS`,
  `BOTS_SERVER`), `stop` also kills bots.
- **Docs:** RUNBOOK match-modes section, command/env tables, solo bot-flow
  scenario, explicit "instant start is dev-only" note; README quick-start
  updated; features.md; version 0.9.1 → 0.10.0.

## Checks

- `cargo check/clippy --workspace` clean; `cargo test --workspace` green
  (123 tests total, incl. 10 new server matchmaking tests, client overlay
  test, harness matchmaking integration test).
- Live smoke (release server + real `bots` binary over UDP): 9 bots hold at
  `forming 9/10`; the 10th join logs `match found (10/10) - starting in 3s`
  then `Match running`, with exactly 5 green / 5 blue joins. An interrupted
  run also live-demonstrated the rollback path (`Starting/Forming → Lobby`
  after all bots dropped).
- `make -n` verified for all new targets; `make stop` cleans bots too.

## Remaining limitations

- One match per server process; "100 players" means ~10 concurrent server
  processes behind manual port assignment — no orchestrator/lobby service
  yet (documented as out of scope in the spec).
- Bots are matchmaking/UX dummies (wander + keepalive), not combat AI.
- No party/premade support, no skill rating, no reconnect-into-queue UX
  beyond the existing session reclaim.
- Client team *preference* is silently overridden in release mode (server
  logs and the client adopts the assigned team; the select screen still
  shows a team picker).
