# 2026-09-24 — Client `net` module split

## Goal
Roadmap step 8, mechanical half: `client/src/net.rs` held the UDP thread,
the session controller, command encoding, snapshot ingest and apply,
interpolation, the connection panel and 40 inline tests (~3600 production
and ~2000 test lines). Split it into a `client/src/net/` module tree with
verbatim moves and no public-API change, so the follow-up (session events,
`SnapshotApplied`, splitting `apply_server_snapshot`) can be reviewed on its
own.

## Layout
| File | Contents |
| --- | --- |
| `net/mod.rs` | module list, `shared::wire`/`Lane` re-exports, `UPDATE_INTERVAL_SECONDS`, `NetworkingPlugin`, `ClientNetPipeline`, `configure_network_pipeline`, re-exports of the public surface |
| `net/components.rs` | `GameStateSnapshot`, `NetworkState`, replicated components (`RemotePlayer`, `NetworkPlayerId`, `PlayerUtility`, cooldowns, cosmetics, `PlayerEquipment`, `PlayerProgression`, projectiles, structures, minions, neutrals) and their `From<&…State>` conversions |
| `net/transport.rs` | size constants, `NetThreadSignal`, `SharedGameplaySigner`, `NetworkChannels`, `spawn_network_transport`, socket connect/resolve, `run_udp_client`, decode/forward, `send_packet`, payload guard, `udp_try_send` |
| `net/session.rs` | `ClientConnectionState`, `SessionUiCommand`, `NetIncomingDisconnected`, `CommittedJoin`, `ReconnectState`, `ClientSession`, `TeardownReason`, `start_networking`, join retry (`MAX_JOIN_ATTEMPTS`, `send_join_attempt`), teardown helpers, `update_session_lifecycle` |
| `net/commands.rs` | `NetworkCommand`, `LocalStateSendTimer`, `send_local_state`, `social_requires_signature`, `send_network_commands` |
| `net/ingest.rs` | `PendingServerSnapshotFrame`, `PendingSnapshotData`, `ingest_server_snapshot_packets` |
| `net/apply.rs` | `LOCAL_SNAP_DISTANCE`, store/sandbox model respawns, `accept_dash_ack`, `network_projectile`, `mirror_debug_flags_to_network_state`, `choose_authoritative_local_player`, `SnapshotUiState`, `apply_server_snapshot`, `*_state_to_*` converters |
| `net/interpolate.rs` | `MINION_RADIUS`, `NetworkGroundingSet`, `ground_networked_entities`, `age_utility_timers`, `NetEntityInterpolation`, `RemotePose`, `RemotePlayerInterpolation`, the two interpolation systems |
| `net/status_ui.rs` | connection panel components, `setup_connection_status_ui`, retry button, `sync_connection_status_ui` |
| `net/test_fixtures.rs` | `cfg(test)`: `admission_app`, `admission_snapshot`, `snapshot_app`, `team_vision_snapshot`, `network_hero_entity`, `exact_size_snapshot_fixture`, `assert_fixture_sentinel`, `populated_snapshot_fixture` |

`offline.rs` (practice simulation and banner) and `public_transport.rs`
stay where they were; both now import explicitly (`super::session`,
`super::transport`, `shared::…`) instead of `use super::*`.

## Visibility
Private items referenced from another submodule became
`pub(in crate::net)`: the systems registered by the plugin, `NetworkState`,
`NetworkChannels`, `PendingServerSnapshotFrame`, `PendingSnapshotData`,
`LocalStateSendTimer`, `SnapshotUiState`, `TeardownReason`,
`TeardownQueries`, `MAX_JOIN_ATTEMPTS`, `send_join_attempt`,
`spawn_network_transport`, `run_udp_client`, `decode_server_packet`,
`forward_complete_server_datagram`, `send_packet`, `SharedGameplaySigner`,
`NetEntityInterpolation`, `RemotePlayerInterpolation` (+ `new`, `push`,
`teleport`, `latest_translation`), the connection panel components,
`ClientSession::clear_join_attempt` and the private fields of
`ClientSession`, `NetworkState`, `NetworkChannels`, `PendingSnapshotData`,
`PendingServerSnapshotFrame`, `NetEntityInterpolation` and
`TeardownQueries`. Nothing became `pub`; `crate::net::NetThreadSignal` was
only named in a doc comment and is now linked as
`crate::net::transport::NetThreadSignal`.

## Tests
The 40 tests moved with their subject: apply 10, transport 12, session 7,
commands 3, ingest 3, interpolate 3, components 1, status_ui 1. Each test
module starts with `use super::*;` (plus `crate::net::test_fixtures::*`
where a shared app or fixture is used).

## Checks
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets
  --no-deps -- -D warnings` clean.
- `cargo test -p client --lib`: 536 tests (the IPv6 loopback test skips
  where the container has no IPv6 stack).
- `cargo test -p shared -p server` unaffected.

## Not done
- The event redesign (`SessionEvent`, `SnapshotApplied`, splitting
  `apply_server_snapshot`) is the next change; `apply.rs` still holds the
  760-line `apply_server_snapshot`.
- The practice banner stays in `offline.rs` next to the practice
  simulation rather than moving to `status_ui.rs`.
