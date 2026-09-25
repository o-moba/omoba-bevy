# 2026-09-25 — Party, party lobby and co-op vs bots

## Goal
Let two players group up, see each other's avatars in a lobby and play on the
same team against bots (TASK-PARTY-LOBBY-2026-09-25). Before this, two humans
were always split across teams and there was no party or invite.

## Changes
- `shared/src/party.rs`: wire types, additive `Party` packets.
- `server/src/party.rs`: `PartyState` (presence, invites, membership, leader,
  launch, prune/rebind) and runtime glue; party seat in `join.rs`; launch gate
  in `prematch.rs`; party-aware queue units and team split in
  `match_service.rs`.
- Client: `party.rs` (view, presence, launch follow), `frontend/lobby.rs`,
  `frontend/party_stage.rs`, Home toast/button/PLAY label, frontend QA stage.
- Docs: README "Play with a friend", `docs/party.md`, features, changelog;
  version 0.24.0-rc.1.

## Checks
- `make check`: fmt, clippy (with and without `qa`), 1074 tests passed, 0 failed.
- `party_e2e.py` against the real practice server: invite → accept → member
  launch refused → leader launch → both joined on one team, 2 humans + 3 bots
  vs 5 bots. PASS.
- Frontend QA capture with a scripted friend: Home invite toast and a
  two-avatar lobby line-up.

## Remaining risks
- Public-lobby party flow is covered by unit tests only (no Postgres E2E run).
- In memory per server: a server restart drops parties; no cross-server invite.
- A party member who cancels the public queue holds the rest of the party in
  the queue until their retries lapse (15 s).
- Release blockers R5/R6 (WAN soak, public distribution/ops) are unchanged.
