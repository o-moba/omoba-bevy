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
5. The leader presses **PLAY VS BOTS** (or **Casual PvP** / **Ranked** on a
   public lobby, or a queue button on Home). Every member is moved to hero select
   with the leader's queue. Members that are not leaders see "Waiting for … to
   start". A Ranked party must be one rated cohort (LR-08); otherwise its members
   are told to play Casual PvP together.
6. **Leave party** passes the lead to the next member; the leader can **Kick**.
   A party of one dissolves.

## Where it runs

| Server role | Party state | Same team via |
| --- | --- | --- |
| standalone / `practice` (no DB) | in the server process | joining human takes a seat on a seated mate's team (`party_team`), draft countdown held ≤60 s for missing members |
| `lobby` (public) | in the lobby process | queue units: a party is scheduled only when all present members queued with one preference; one manifest, one team (`split_teams`) |
| `match` worker | none (members show as *In match* in the lobby while it holds their allocation, *Away* otherwise) | frozen manifest |

Members are keyed by player id; a member with a career profile who leaves for
a public match is rebound by profile id when they return (45-minute grace;
guests: 60 s).

## Protocol (additive)

- `ClientPacket::Party { command: PartyCommand }` — `presence`, `invite`,
  `accept`, `decline`, `leave`, `kick`, `launch`. The client announces
  presence (nickname + showcase avatar) every 2 s while connected.
- `ServerPacket::Party { server_epoch, sequence, party: PartyView }` — framed
  like social views (tick namespace `3 << 62`), only to endpoints that
  announced presence. The periodic pass (every 500 ms) builds the online list
  once and derives every viewer's view from it; a presence heartbeat never
  triggers a send. A state-changing command (`invite`, `accept`, `decline`,
  `leave`, `kick`, `launch`) answers its issuer at once and a successful
  `invite` also reaches the invitee at once; everyone else sees the change on
  the next periodic pass.
- `PartyView.error` (additive, absent when there is none): why the command
  this view answers was refused, as a stable code the client localizes
  (`party.error.<code>`): `not_online`, `self_invite`, `already_member`,
  `party_full`, `no_invite` (also an expired one), `not_leader`, `no_party`,
  `invite_declined`, `invite_rate`. Only on the immediate reply to the
  issuer, never in the periodic broadcast.
- `PartyMember.in_match` on the public lobby: the member's profile holds a
  live allocation this lobby handed out (`MatchService::assigned_profiles`);
  such a member is not `away` even without presence. On a standalone or
  practice server it is a human seated in this process.
- Friends (`OnlinePlayer.friend`, listed first before the 16-row cap) are the
  viewer's accepted friends as known to the server's career view, which a
  Friends request fills; the client sends one on entering the party lobby
  (and the friends modal sends its own). The server re-reads the list every
  2 s per viewer.
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
party chat, not persisted across a server restart (a client that was in a
party when the lobby's epoch changes shows «Reconnected to the lobby. Your
party was disbanded.» in the party lobby). Inviting by friend code to another
server is out of scope.

Invite throttle: a target that declined is not re-invited by the same sender
for 60 s (`invite_declined`); a sender issues at most 10 invites per minute
(`invite_rate`). Refused invites do not count. Online list: at most 16 rows,
friends first, then players not in a party, then by nickname.
