# 2026-07-03 — Pre-join flow fixes (0.7.1)

## Goal

Fix three related pre-join bugs that deadlocked a fresh server + fresh client:
ghost players created by heartbeat packets, the lobby overlay covering (and
click-blocking) the character-select screen, and team clicks being silently
swallowed while the transport was dead.

## Changes

- `server/src/main.rs`, `server/src/session.rs`: added `ConnectedPlayer.joined`
  (false until a `Join` packet). Pre-join endpoints keep receiving snapshots
  but are excluded from the replicated `players` list
  (`build_players_snapshot`) and from all gameplay: movement/cast guards,
  skill upgrades, rematch/god-mode/speed-boost, ECS sync, minion/tower/neutral
  targeting, team-buff regen, and minion kill-reward splits. Pre-join
  connection log renamed to "Endpoint ... connected (pre-join)".
- `client/src/game_state.rs`: `GameStateOverlay` root and label are
  `Pickable::IGNORE`; the Lobby "Waiting for match to start..." state only
  shows after `join_flow_committed`.
- `client/src/team.rs`: pressing a team button while
  `ClientConnectionState::Disconnected` no longer commits/despawns; it writes
  `SessionUiCommand::Retry` and keeps the select overlay up.
- Version bumped to 0.7.1; `CHANGELOG.md` updated.

## Checks

- `cargo build --workspace`, `cargo test --workspace` (new tests:
  `pre_join_endpoint_is_hidden_and_inert_until_join` on the server;
  `overlay_never_blocks_pointer_picking` and
  `lobby_overlay_stays_hidden_until_join_is_committed` on the client),
  `cargo clippy --workspace --all-targets -- -D warnings` — all PASS.
- Live smoke: server + no-autojoin client + ping-only UDP probe for 10 s —
  no join, no match start, snapshots stayed `lobby` with `players=0`. Then an
  `OMOBA_AUTOJOIN=mage:agnes:green` client joined: match started and the
  probe's snapshots showed exactly one player (id 3, green mage).

## Remaining risks

- Pre-join endpoints still consume player ids and a map slot until the 5 s
  timeout; harmless but ids are not reused.
- The team-press Retry path relies on the existing transport respawn; if the
  server stays down, the user keeps the select screen plus the Disconnected
  status banner, which is the documented recovery UX.
