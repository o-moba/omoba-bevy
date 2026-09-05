# 2026-07-06 — Session Reconnect UX (TASK-25)

## Goal

The character-select screen sometimes popped up mid-game "out of nowhere",
letting the player re-join. Root cause: every client-side connection
teardown (3 s qualifying-snapshot staleness, transport-failure signal from
the UDP thread, 45 s server-wait timeout, closed channel) despawned the
local player and respawned the team-select overlay — even for a joined
player. Any 3 s+ snapshot gap (packet-loss burst, background-throttled
server, machine hiccup) silently kicked the player back to select, although
the server keeps sessions reclaimable for 30 s.

## Changes (`client/src/net.rs`)

- `TeardownReason` enum, passed into `perform_network_teardown` and logged
  (`warn!`) — surprise disconnects are now diagnosable from the client log.
- `ClientSession.last_join: Option<CommittedJoin>` — the loadout actually
  sent in the Join packet (team/character/class/avatar), recorded at the
  commit point.
- Teardown with a committed join no longer spawns the select overlay and
  keeps the team selection; it arms `ReconnectState` instead. The select
  screen still appears for never-joined players.
- Auto-reconnect: while disconnected with an armed reconnect, the transport
  respawns on the shared `T_RETRY` (2 s) cadence — same code path as the
  manual Retry button, which keeps working.
- Auto-rejoin: when snapshots flow again, the client resends Join with the
  remembered loadout + persistent session id; the server reclaims the
  session (same player id/team/position within 30 s) or admits the player
  through matchmaking afresh.
- Connection panel shows "Connection lost — reconnecting (attempt N)..."
  while the loop runs.
- `TeardownQueries` SystemParam bundle (Bevy's 16-parameter system cap).

## Checks

- 3 new unit tests: teardown-reason display uniqueness,
  select-screen gating, reconnect cadence gate (state/active/cooldown).
- Live smoke: dev server + autojoined client → server killed → client
  logged `Network teardown: transport failure...`, `Auto-reconnect attempt
  1` → server restarted on the same port → `Auto-rejoined after reconnect
  (attempt 1)`, player re-joined the match, and the select overlay never
  respawned (raw logs in the task evidence).
- `cargo check/clippy --workspace` clean; `cargo test --workspace` green
  (137 tests).

## Remaining notes

- Frozen TASK-14 timing constants unchanged (staleness stays 3 s; retry
  cadence 2 s; reconnect keeps retrying indefinitely until success or a
  deliberate pause-menu restart).
- Across a server *process* restart the session store is empty, so the
  auto-rejoin lands as a fresh join (matchmaking rules apply); within one
  server process the session is reclaimed with hero/team/position intact.
