# Party, party lobby and co-op vs bots (0.24.0)

A **party** is up to five players connected to the same server who see each
other in the party lobby, start together and are always seated on **one team**.
It is the minimum social loop for playtesting with a friend.

## Player flow

1. Home → **Party & friends** opens the party lobby (`AppScreen::Lobby`).
2. *Online on this server* lists every other client that announced presence
   on this server (friends first and marked ★ when career storage is on).
   **Invite** sends an invite; a party is created on the first invite.
3. The invitee sees a toast on Home and an entry in the lobby: **Accept** or
   **Decline**. Invites lapse after 60 seconds. Accepting leaves any previous
   party.
4. The lobby shows a 3D line-up: every member's avatar on a pedestal (the
   leader's is gold), with name plates (Ready / In match / Away).
5. The leader presses **PLAY VS BOTS** (or **Quick match** on a public lobby,
   or PLAY on Home). Every member is moved to hero select with the leader's
   queue preference. Members that are not leaders see "Waiting for … to start".
6. **Leave party** passes the lead to the next member; the leader can **Kick**.
   A party of one dissolves.

## Where it runs

| Server role | Party state | Same team via |
| --- | --- | --- |
| standalone / `practice` (no DB) | in the server process | joining human takes a seat on a seated mate's team (`party_team`), draft countdown held ≤60 s for missing members |
| `lobby` (public) | in the lobby process | queue units: a party is scheduled only when all present members queued with one preference; one manifest, one team (`split_teams`) |
| `match` worker | none (members show as *Away* in the lobby) | frozen manifest |

Members are keyed by player id; a member with a career profile who leaves for
a public match is rebound by profile id when they return (45-minute grace;
guests: 60 s).

## Protocol (additive)

- `ClientPacket::Party { command: PartyCommand }` — `presence`, `invite`,
  `accept`, `decline`, `leave`, `kick`, `launch`. The client announces
  presence (nickname + showcase avatar) every 2 s while connected.
- `ServerPacket::Party { server_epoch, sequence, party: PartyView }` — framed
  like social views (tick namespace `3 << 62`), every 500 ms or right after a
  command, only to endpoints that announced presence.
- Avatars shown to others are restricted to roster slugs; nicknames are
  trimmed/bounded; authenticated players show their profile nickname.
- A launch carries an increasing `sequence`; a client follows each launch of
  its current party once and never replays one it saw on joining.

## Code

- `shared/src/party.rs` — wire types.
- `server/src/party.rs` — `PartyState` rules and the runtime glue;
  `server/src/match_service.rs` — party-aware `select` / `split_teams`;
  `server/src/runtime/handlers/join.rs` — party seat; `server/src/prematch.rs`
  — launch gather gate.
- `client/src/party.rs` — `PartyClient`, presence, launch follow.
- `client/src/frontend/lobby.rs`, `client/src/frontend/party_stage.rs` — lobby
  screen and 3D line-up (render layer 27).

## Verification

- Unit tests: `cargo test -p shared party`, `cargo test -p server party`,
  `cargo test -p server match_service`, `cargo test -p client party lobby`.
- End to end against the real server binary:
  `python3 .agent/tasks/TASK-PARTY-LOBBY-2026-09-25/raw/party_e2e.py target/debug/server`.
- Screenshot: `OMOBA_FRONTEND_QA_OUTPUT=<dir> OMOBA_FRONTEND_QA_ACCEPT_PARTY=1`
  with a scripted friend (`raw/party_friend_bot.py`) captures `14-party-lobby.png`.

## Limits

Parties are per server process and in memory: no cross-server invites, no
party chat, not persisted across a server restart. Inviting by friend code to
another server is out of scope.
