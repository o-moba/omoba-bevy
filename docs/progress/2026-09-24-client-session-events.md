# 2026-09-24 — Client session events and staged snapshot apply (roadmap step 15, slices 15a + 15b1)

## Goal
The first two slices of step 15 from
[plans/client-10-15.md](../plans/client-10-15.md) (sections 15.2 to 15.5):
- 15a: a `SessionEvent` message for the session's lifecycle edges, emitted
  at every site in the plan's table through one outbox and one flush system,
  with no consumers yet.
- 15b1: split `apply_server_snapshot` into chained stages without changing
  what it does: `Begin`, `Session`, `Resources`, one `Entities` stage
  holding the old remainder, and `Finish`, which announces
  `SnapshotApplied`.

No behaviour change and no wire change. Only `client/src/net/` changed
(`session.rs`, `apply.rs`, `ingest.rs`, `transport.rs`, `components.rs`,
`mod.rs`, `test_fixtures.rs`); `shared/` and `server/` are untouched.

## 15a: `SessionEvent`
`#[derive(Message, Clone, Debug, PartialEq)] pub enum SessionEvent` in
`net/session.rs`, registered by `NetworkingPlugin`, re-exported as
`crate::net::SessionEvent`.

| Variant | Emitted at |
| --- | --- |
| `TransportStarted { addr, offline }` | end of `spawn_network_transport` (startup, `StartOffline`, `ConnectTo`, `ConnectAllocated`, `LeaveMatch` back to the lobby, `Retry`, auto-reconnect) |
| `Connected` | `Session` stage, when the state becomes `Connected` |
| `Joined { your_id }` | `Session` stage, when `join_confirmed()` holds and `announced_join` is false; `clear_join_attempt` resets the flag |
| `Rejected(JoinRejection)` | `ClientSession::set_join_error`, when the error changes to a new `Some`: ingest's protocol mismatch, map-geometry mismatch and `join_error` from the snapshot; `send_join_attempt`'s denied avatar ticket; the transport's `ProtocolMismatch` signal |
| `JoinExhausted` | `ClientSession::exhaust_join` (edge only): `MAX_JOIN_ATTEMPTS` reached, or the Join could not be queued |
| `Disconnected { reason, reconnecting }` | end of `perform_network_teardown`; `reconnecting` is `reconnect.active` after the teardown |
| `Left { returning_to }` | `LeaveMatch`, after `abandon_join` |
| `ServerScopeReset` | where career and social are cleared: `StartOffline`, `ConnectTo` to a different address (checked before `spawn_network_transport` overwrites `server_addr_display`), `ConnectAllocated`, `LeaveMatch` with a lobby to return to |
| `RoundChanged { previous, current }` | `Resources` stage, from `NetworkState.last_round` |

- **Outbox.** Each site pushes into `ClientSession.outbox`
  (`pub(in crate::net)`). `flush_session_events` writes the queue with
  `write_batch`: it is the last system of `snapshot_apply_systems()` (end of
  `ApplySnapshot`, every frame, staged snapshot or not, which also delivers
  the `TransportStarted` queued at `Startup`), and it is chained after
  `retry_pending_join` in `SessionLifecycle`, which catches the lifecycle
  commands, the teardowns and the `send_join_attempt` calls from
  `SendCommands`. It returns early on an empty queue and drains through
  `bypass_change_detection`, so `ClientSession` change ticks are exactly as
  before (`help_overlay` checks `is_changed()`).
- **`RoundChanged`.** `NetworkState.last_round: Option<RoundId>` is never
  reset. `RoundId::from_meta` skips zero ids, the first round only records
  itself, and a reconnect to the same round finds the same id: the
  `CombatRoundIdentity` semantics.
- **Order within one frame.** `Connected` before `Joined`; `LeaveMatch`
  queues `Left`, then `ServerScopeReset`, then `TransportStarted`;
  `ConnectTo` queues `ServerScopeReset` before `TransportStarted`; the
  protocol-mismatch signal queues `Disconnected` then `Rejected`.
- **Protocol mismatch.** That branch cancels the reconnect right after the
  teardown. `ClientSession::stop_reconnecting` resets `reconnect` and
  rewrites the queued `Disconnected` to `reconnecting: false`, so the event
  does not promise a reconnect that will not happen.
- `TeardownReason` is `pub(crate)` and derives `PartialEq`.
- `SessionReactions` (`net/mod.rs`) is configured
  `.after(ClientNetPipeline::SessionLifecycle)` in
  `configure_network_pipeline` and has no members yet.
- Test apps: `admission_app` and the lifecycle-only test apps do not run
  the flush, so their outbox just grows (hazard 10). The apps that run
  `snapshot_apply_systems()` register `SessionEvent`, `SnapshotApplied` and
  `StagedSnapshot`.

## 15b1: `SnapshotApply` stages
`SnapshotApply::{Begin, Session, Resources, Entities, Finish}` is
configured `.chain().in_set(ClientNetPipeline::ApplySnapshot)` in
`configure_network_pipeline`. `apply::snapshot_apply_systems()` returns the
systems, chained and inside `ApplySnapshot`, followed by the flush:

| Stage | System | Runs | Does |
| --- | --- | --- | --- |
| `Begin` | `begin_snapshot_apply` | always | `StagedSnapshot.data = pending.frame.take()`, `gate = Full` |
| `Session` | `apply_snapshot_session` | `snapshot_staged` | state `Connected`, `waiting_since`, `last_qualifying_snapshot_wall`; `Connected`/`Joined` |
| `Resources` | `apply_snapshot_resources` | `snapshot_staged` | `network_state.local_id`, the 14 `GameStateSnapshot` fields (taken out of the staged frame), `RoundChanged`, prematch loadout into `TeamSelection` and `last_join`, `gate = Draft` in the Draft phase |
| `Entities` | `apply_snapshot_entities` | `snapshot_staged` | the old body from the Draft gate on, unchanged; the Draft branch reads the gate, the uncommitted-local-hero return sets `gate = LocalPending` |
| `Finish` | `finish_snapshot_apply` | `snapshot_staged` | writes `SnapshotApplied { meta, your_id, round, outcome }`, clears the staged frame |
| (after `Finish`) | `flush_session_events` | always | see 15a |

- The plugin, `net/test_fixtures.rs` (`snapshot_app`) and the offline
  lifecycle test in `net/session.rs` all use the helper.
- `SnapshotApplied` is `pub`, re-exported from `crate::net` and registered
  by the plugin. The struct carries
  `#[cfg_attr(not(test), expect(dead_code, reason = …))]` because nothing
  reads its fields outside the tests yet; the first reader (15f or a QA
  loop) turns the expectation into an error, and the attribute goes.
- **Hazard 1 (Commands flush).** Only `Entities` uses Commands, and it is one
  system, so nothing inside it sees its own spawns earlier than before. Bevy
  puts a sync point after it, before `Finish` and before every
  `.after(ApplySnapshot)` system, as before; the existing
  `admitted_snapshot_and_world_fallback_keep_one_local_root_in_the_same_frame`
  still passes.
- **Hazard 2 (early returns).** Two new tests pin them (see below).
- **Hazard 3 (resources before players).** `Session` and `Resources` run
  first and do not depend on the gate.
- **Hazards 4 and 5** belong to 15b2: the `ParamSet` and the `UtilityVfx`
  order stay inside the single `Entities` system.
- **Hazard 6.** `mirror_debug_flags_to_network_state` is still unordered.
- Parameters: `Entities` has 16 (`StagedSnapshot` replaced
  `PendingServerSnapshotFrame`, `Res<ClientSession>` replaced `ResMut`
  since it only reads `has_committed_join`). Without `SnapshotUiState` it
  would need 18, so the workaround stays with three resources
  (`CameraState`, `TeamSelection`, `PlayerVisualMode`); `GameStateSnapshot`
  moved to `Resources`.

## Deviations from the plan
- No separate `ApplyGate` type: `StagedSnapshot.gate` is an `ApplyOutcome`,
  which has the same three states, and `Finish` reports it as the outcome.
- `SnapshotApplied` has no `local: LocalHeroApply` field yet (as agreed for
  15b1).
- `SnapshotUiState` is not removed (see above); 15b2 removes it when it
  splits the entity stage.
- `flush_session_events` is part of `snapshot_apply_systems()` so the test
  apps flush exactly like production; they register the two messages and
  `StagedSnapshot`.
- `Rejected` fires on a change to a new rejection, not on every snapshot
  that repeats it; `JoinExhausted` fires on the edge only; the denied avatar
  ticket sets `join_exhausted` without a `JoinExhausted` event (the plan
  lists it under `Rejected` only).
- The stages move fields out of the staged frame (`mem::take`,
  `Option::take`) instead of destructuring it by value.

## Tests
Client lib 549 → 557 (529 → 537 with `--no-default-features`); every
existing test, including the `net/apply.rs` ones, is unchanged.
- `net/session.rs`: `first_admitted_snapshot_announces_connected_before_joined_once`,
  `teardown_of_a_committed_join_announces_a_reconnecting_disconnect`,
  `reconnect_to_the_same_round_is_no_round_change_but_a_new_match_is`,
  `map_geometry_mismatch_announces_one_rejection`,
  `protocol_mismatch_teardown_announces_no_reconnect_then_the_rejection`,
  `connect_and_leave_announce_scope_reset_transport_and_left`.
- `net/apply.rs`: `draft_snapshot_clears_heroes_and_leaves_world_entities_untouched`
  (structures, minions, projectiles and neutrals keep their entities,
  positions and HP while a Draft snapshot moves or drops them; the heroes go;
  `GameStateSnapshot` is still written; outcome `Draft`) and
  `listed_local_hero_without_team_or_join_spawns_no_remote_or_world_entity`
  (nothing spawns, the session and `GameStateSnapshot` are still updated;
  outcome `LocalPending`).
- `net/test_fixtures.rs`: `drain_session_events`, `drain_snapshot_applied`
  and `tear_down` (the production teardown through `run_system_once`).

## Verification
- `cargo fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets --no-deps -- -D warnings`: clean.
- `cargo clippy -p client --lib --no-deps --no-default-features -- -D warnings`: clean.
- `cargo test -p client --lib`: 557 passed.
- `cargo test -p client --lib --no-default-features`: 537 passed.
- `cargo test -p shared`: 78 passed.

## Next
15b2 (split `Entities` into `LocalPlayer`, `RemotePlayers`, `Projectiles`,
`Structures`, `Minions`, `Neutrals`; one local-hero bundle helper; drop
`SnapshotUiState`), then 15c (combat round reset and mobile clear read
`RoundChanged`), 15d (`ServerScopeReset` and `Left` consumers in
`SessionReactions`), 15e (`ClientSession` accessors). 15f and 15g are
optional.
