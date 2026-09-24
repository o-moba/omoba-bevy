# 2026-09-24 — Client snapshot stages and session reactions (roadmap step 15, slices 15b2 + 15c + 15d)

## Goal
The next three slices of step 15 from
[plans/client-10-15.md](../plans/client-10-15.md) (sections 15.3 to 15.5),
after 15a and 15b1 ([client-session-events](2026-09-24-client-session-events.md), #39):
- 15b2: split the `Entities` stage into one stage per entity kind, with one
  local-hero spawn bundle, without changing what it does.
- 15c: the combat round reset and the mobile-controls round clear react to
  `SessionEvent::RoundChanged` instead of tracking the round themselves.
- 15d: career, social and the front end react to `ServerScopeReset` and
  `Left` in `SessionReactions`; `update_session_lifecycle` stops writing
  their resources.

No behaviour change and no wire change. Only `client/` changed (`net/apply.rs`,
`net/mod.rs`, `net/session.rs`, `combat/round_reset.rs`, `combat/mod.rs`,
`combat/tests.rs`, `mobile_controls.rs`, `career.rs`, `social.rs`,
`frontend/mod.rs`); `shared/` and `server/` are untouched.

## 15b2: one stage per entity kind
`SnapshotApply::{Begin, Session, Resources, LocalPlayer, RemotePlayers,
Projectiles, Structures, Minions, Neutrals, Finish}`, configured `.chain()`
inside `ClientNetPipeline::ApplySnapshot`; `snapshot_apply_systems()` builds
the same chain for the plugin and the net test apps, with
`flush_session_events` last.

| Stage | System | Runs | Queries and resources |
| --- | --- | --- | --- |
| `LocalPlayer` | `apply_snapshot_local_player` | `snapshot_staged` | `Query<(Entity, Option<&NetworkPlayerId>), With<Player>>`, `Query<&mut Transform, (With<Player>, Without<MainCamera>)>`, `Query<&mut Transform, (With<MainCamera>, Without<Player>)>`, `Query<Option<&PlayerCosmeticAction>>`; `ClientSession`, `NetworkState`, `PlayerAssets`, `PlayerModelResolver`, `CameraState`, `TeamSelection`, `PlayerVisualMode`, `UtilityVfx` writer (14 parameters) |
| `RemotePlayers` | `apply_snapshot_remote_players` | `world_stages_open` | `Query<(&mut RemotePlayerInterpolation, Option<&PlayerUtility>), With<RemotePlayer>>`, `Query<Option<&PlayerCosmeticAction>>`; `NetworkState`, `PlayerAssets`, `PlayerModelResolver`, `PlayerVisualMode`, `UtilityVfx` writer |
| `Projectiles` | `apply_snapshot_projectiles` | `world_stages_open` | `Query<&mut Transform, With<NetworkProjectile>>`, `Query<&NetworkProjectile>` |
| `Structures` | `apply_snapshot_structures` | `world_stages_open` | `Query<&mut Transform, With<NetworkStructure>>`, `Query<&NetworkStructure>` |
| `Minions` | `apply_snapshot_minions` | `world_stages_open` | `Query<&Transform, With<NetworkMinion>>`, `Query<&NetworkMinion>`, `PlayerVisualMode` |
| `Neutrals` | `apply_snapshot_neutrals` | `world_stages_open` | `Query<&Transform, With<NetworkNeutral>>`, `Query<&NetworkNeutral>` |

- `world_stages_open` is `data.is_some() && gate == ApplyOutcome::Full`. Bevy
  evaluates a run condition when the stage is reached, so it sees the gate
  `Resources` (Draft) or `LocalPlayer` (`LocalPending`) set.
- `LocalPlayer` holds the old local-hero code: the Draft branch (despawn
  every local `Player` and every remote hero, return), the duplicate
  cleanup, the component update with the dash acknowledgement and the
  snap, the spawn with the camera lock, and the uncommitted-local-hero
  return, which now sets `gate = LocalPending` and leaves the rest closed.
- `local_hero_components(state, your_id) -> impl Bundle` is the shared part
  of the three spawn bundles (`Player`, `PlayerBody`, `VerticalVelocity`,
  `Team`, the seven identity components, stats, progression, equipment, the
  three cooldown/utility components, `Name("Player")`). The 2D spawn adds
  `Transform` + `Visibility`, the scene spawn `SceneRoot`, `Transform`,
  `GlobalTransform`, `Visibility`, `NormalizeModelScale` (+ `ModelScaleSource`),
  the mesh fallback `Mesh3d`, `MeshMaterial3d`, `Transform`, exactly as before.
- `SnapshotUiState` is gone: the largest stage has 14 parameters.
- `StagedSnapshot.local: LocalHeroApply`, reset by `Begin` and reported by
  `Finish` as `SnapshotApplied.local`:
  - `Unchanged`: the snapshot does not list the local hero (a live hero is
    only re-tagged with its id), or the gate stopped before the spawn.
  - `Updated { entity, corrected, dashed }`: the existing hero took the
    server state; `corrected` when its transform snapped, `dashed` for a new
    dash (which always snaps).
  - `Spawned { entity, position, team }`: server position and assigned team.
  - `Cleared`: the Draft branch despawned at least one local `Player`.
- The world stages read the entity lists through `Res<StagedSnapshot>`;
  `Resources` still moves the resource fields out.

Hazards (plan 15.4):
1. **Commands flush.** Every entity stage uses Commands, so Bevy puts a sync
   point after each. No stage looks an entity up that another stage spawned
   in the same frame: the lookups go through the stage's own `NetworkState`
   map, and the `.get(entity).is_ok()` stale checks only see entities from
   earlier frames. Two orders changed, neither observable: the local hero's
   components are applied before the remote players are reconciled, and the
   stale minions are despawned in `Minions`, before `Neutrals`, instead of
   after the stale neutrals. A later stage's spawn may reuse an entity index
   an earlier stage's despawn freed in the same frame (with a new
   generation, so no handle aliases).
   `admitted_snapshot_and_world_fallback_keep_one_local_root_in_the_same_frame`
   still passes: `world.rs` runs after the whole set.
2. **Early returns.** The two gate tests from 15b1 pass unchanged.
3. **Resources before players.** Unchanged.
4. **Transform conflicts.** The `ParamSet` is gone; the filters are in the
   table. The hero and camera queries exclude each other; the minion and
   neutral stages only read transforms (the old code used `get_mut` without
   writing, so no change ticks move).
5. **Message order.** `LocalPlayer` writes the local `UtilityVfx::Dash`
   before `RemotePlayers` writes the remote ones; the new test pins it.
6. **Speed-boost mirror.** Still unordered.

## 15c: round resets on `RoundChanged`
- `combat::round_reset::reset_round_input_state` reads
  `MessageReader<SessionEvent>` and resets when any `RoundChanged` arrived;
  it keeps its place (after `ApplySnapshot`, before `InputContextSet::Modal`).
  `CombatRoundIdentity` is deleted.
- `mobile_controls::read_mobile_controls` does the same at the top of the
  input frame (`MobileControlsSet::Input`, after `InputContextSet::Resolve`,
  which is after `ApplySnapshot`); `MobileControls.round_identity` and the
  `GameStateSnapshot` parameter are deleted.
- Why not `SessionReactions`: `RoundChanged` is written by the flush at the
  end of `ApplySnapshot`, in the frame the new round is applied, and both
  readers must clear the old round before this frame's input and
  `SendCommands`. `SessionReactions` runs after `SendCommands`.
- Same semantics as the private trackers: `NetworkState.last_round` skips
  zero ids and is never reset, so a teardown (zero ids in
  `GameStateSnapshot`) and a reconnect to the same round are not a change.
- Kept polling (plan 15.1 C): `shop.rs`, `edge_hud.rs`, `sandbox/mod.rs`,
  `frontend/draft.rs`.

## 15d: `SessionReactions`
| Event | System | Does |
| --- | --- | --- |
| `ServerScopeReset` | `career::clear_account_on_scope_reset` | `CareerClient::clear_account()` |
| `ServerScopeReset` | `social::clear_on_scope_reset` | `SocialClient::clear()` |
| `Left` | `frontend::return_home_on_leave` | `PendingScreen.0 = Some(Home)` |

- Each reads every message of the frame (a loop, not `any`, so a second
  event in the same frame is not left for the next frame).
- `update_session_lifecycle` dropped the `CareerClient` and `SocialClient`
  parameters (13 left) and the `PendingScreen` insertion. `ServerScopeReset`
  is still queued at the same four places; `ConnectTo` still checks the
  address before `spawn_network_transport` overwrites it.
- Kept in `net` (hazards 8 and 9, documented on the system):
  `TeamSelection.team = None`, `MatchServiceClient::take_return_to_lobby()`
  and the `CareerIdentity` signature on `CancelQueue`.
- Timing: the old writes happened in `SessionLifecycle` (`PendingScreen`
  through Commands, applied at the sync point after it); the reactions run
  right after `SessionLifecycle` in the same frame, and `apply_pending_screen`
  still picks the screen up at the start of the next frame's `FrontendSet`.
- Hazard 7: `SessionReactions` is after `SessionLifecycle`, so the clear
  happens before the next frame's ingest can apply the new server's view.

## Deviations from the plan
- `SnapshotApplied` keeps its `cfg_attr(not(test), expect(dead_code))`:
  `SessionEvent` now has readers (it never carried the attribute), but
  nothing outside the tests reads `SnapshotApplied` yet. Removing the
  attribute gives "fields `meta`, `your_id`, `round`, `outcome` and `local`
  are never read". 15f (the camera) is its first reader. `LocalHeroApply`
  needs no attribute (rustc does not report its variant fields).
- The combat and mobile round readers run after `ApplySnapshot`, not in
  `SessionReactions` (see 15c).
- The combat test stands for a reconnect with `TransportStarted`,
  `Connected` and `Joined`: `TeardownReason` (inside `Disconnected`) is not
  reachable outside `net`.

## Tests
Client lib 557 → 561 (537 → 541 with `--no-default-features`). The
`net/apply.rs` tests from before, including the two gate tests, are
unchanged.
- New `net::apply::tests::local_hero_apply_reports_spawn_update_correction_dash_and_draft_clear`
  (`Spawned` with position and team, `Updated` without and with a
  correction, a dash with the local VFX before the remote one, `Unchanged`
  when the snapshot drops the local hero, `Cleared` on Draft, `Unchanged`
  on a second Draft).
- New `net::apply::tests::teardown_gap_keeps_the_last_round_so_only_the_next_round_is_a_change`
  (the assertion moved out of the combat test: during the gap
  `GameStateSnapshot` has zero ids, `last_round` is kept and no
  `RoundChanged` is written; the reconnect into the next round writes
  `Connected`, `Joined`, `RoundChanged { 1 → 2 }`).
- New `net::session::tests::scope_reset_clears_career_and_social_in_its_frame_so_the_next_view_survives`
  (production sets: ingest in `IngestSnapshot`, lifecycle + flush in
  `SessionLifecycle`, the two reactions in `SessionReactions`; `ConnectTo`
  clears the old account and closes the chat in its frame, and the career
  view ingested in the next frame survives).
- New `frontend::tests::leaving_requests_home_and_other_session_events_do_not`.
- Rewritten to feed messages: `combat::tests::round_change_event_clears_old_intents_cooldowns_and_queued_casts_but_reconnect_events_do_not`
  (was `round_identity_clears_…_but_reconnect_does_not`) and gate 4 of
  `mobile_controls::tests::ecs_gates_flush_held_fingers_on_modal_death_focus_rotation_and_round_change`.
- `offline_leave_restores_saved_endpoint_and_next_join_uses_real_udp_transport`
  runs `frontend::return_home_on_leave` after the flush and keeps its
  `PendingScreen(Home)` assertion.

## Verification
- `cargo fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets --no-deps -- -D warnings`: clean.
- `cargo clippy -p client --lib --no-deps --no-default-features -- -D warnings`: clean.
- `cargo test -p client --lib`: 561 passed.
- `cargo test -p client --lib --no-default-features`: 541 passed.
- `cargo test -p shared`: 88 passed.

## Next
15e (`ClientSession` accessors and `#[cfg(test)]` builders). 15f (camera
snap and unlock through `SnapshotApplied`/`SessionEvent`, which also drops
`SnapshotApplied`'s `expect(dead_code)`) and 15g (career and social packets
as messages) stay optional.
