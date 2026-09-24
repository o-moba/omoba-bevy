# Client plan: roadmap steps 10 and 15

Read-only analysis of `main` after PR #33 (line numbers refer to that tree and drift as slices land). Slices are tracked in [REFACTORING.md](../REFACTORING.md).

# Step 15: client session events and staged snapshot application

## 15.1 Who reads or writes session state outside `client/src/net/`

**A. Production code that writes `ClientSession` (only two places, both through `abandon_join()`)**
- `client/src/frontend/mod.rs:181,215-226` (`drive_screen_from_session`): on the Searching screen, when `join_blocked()` is true, it sets the notice text, calls `session.abandon_join()` and goes back to HeroSelect. This reacts to a rejected or exhausted join.
- `client/src/match_service.rs:78,98-104` (`update_match_service`): on Victory with a saved result for the same round, it calls `session.abandon_join()`. It also reads `server_addr_display` (lines 89, 106) to scope identity and allocation. This reacts to the round ending.
- Every other outside write is a test fixture: `help_overlay.rs:383`, `pause_menu.rs:1313-1314`, `game_state.rs:260-275,327-330,376,420`, `player.rs:2338`, `frontend/searching.rs:171-187`, `frontend/home.rs:476-477`, `frontend/mod.rs:515-699`, `career.rs:3193-3198`, `world.rs:591-592,614`, `career_identity.rs:844-845`.
- Joins themselves are committed through `NetworkCommand::Join`/`JoinPrematch`, inside `net/commands.rs:335-376`. `CancelQueue` clears `last_join` inside net at `commands.rs:255-258`.

**B. Outside code that reacts to lifecycle changes by polling**
| Site | Reads | Why |
|---|---|---|
| `frontend/mod.rs:178-260` | `join_confirmed`, `has_committed_join`, `join_blocked`, `join_rejection` | Screen state machine: joined, rejected, left |
| `frontend/mod.rs:336-351` | `state == Disconnected && !has_committed_join` | Retries the connection from the menus every 5 s |
| `team.rs:1597-1608,1682` | `join_flow_committed`, `state == Disconnected` | Defers a Join and sends Retry |
| `pause_menu.rs:561-577` | `state != Disconnected` | Closes the in-match menu on disconnect |
| `social.rs:526-529` | `!is_connected()` | `social.clear()` on disconnect |
| `world.rs:424-440` | `last_join.prematch`, `join_confirmed` | Gate for the fallback local-hero spawn |
| `career_identity.rs:554-620` | `is_offline`, `is_connected`, `server_addr_display` + epoch | Signing scope per endpoint |
| `career.rs:2125` | `join_flow_committed && !join_confirmed` | Join-in-flight gate |
| `mobile_ui.rs:260-265,360-362,406,663` | `join_flow_committed`, `last_join`, `server_addr_display`, `is_choosing_loadout` | Server entry panel |

These are all level-based and robust: they re-derive the state every frame. I recommend they keep polling, because turning them into one-shot events could miss a transition, for example entering Searching after the rejection already fired.

**C. Round-identity (`meta.server_epoch`, `meta.match_id`) change detectors, i.e. round reset / match id change**
- `combat.rs:86-135` `reset_round_input_state` with `CombatRoundIdentity`. It ignores (0,0), fires only when a previous value existed and differs, and does not fire on reconnect. Pinned by the test at `combat.rs:3728`.
- `mobile_controls.rs:647-659` (`round_identity`): exactly the same semantics.
- These compare against the raw value, so they also fire on teardown (0,0) and again on reconnect:
  - `shop.rs:762-768`
  - `edge_hud.rs:372-376`
  - `sandbox/mod.rs:251-261`, keyed with `your_id`
  - `frontend/draft.rs:148-156`, keyed with `draft.generation`
- Cursors that deduplicate events (they need the id per event, so leave them alone): `combat_feedback.rs:121-130` (`HitCursor`), `game_audio.rs:227-235` / `game_audio/policy.rs:221`, `game_vfx.rs:1028-1033,1229-1234`, `team_vision.rs:389`, `player.rs:290,930-936,1553`, `presentation3d.rs:100,169-173`.

**D. Reverse direction: `net` writing other modules' resources on lifecycle changes (the main target of this step)**
- `update_session_lifecycle`, `net/session.rs:536-889`, has 15 parameters.
  - `StartOffline` (564-608) clears `team_selection.team` and the avatar, despawns the world, resets `GameStateSnapshot` and `PendingServerSnapshotFrame`, calls `match_service.take_return_to_lobby()`, `career.clear_account()` and `social.clear()`.
  - `ConnectTo` (609-634) calls `social.clear()` and `career.clear_account()` only when the address changes.
  - `ConnectAllocated` (635-664) calls `career.clear_account()` and `social.clear()`, and sets `cam_state.locked = false`.
  - `LeaveMatch` (665-724) signs `CancelQueue` with `career_identity`, calls `match_service.take_return_to_lobby()`, clears `team_selection.team`, unlocks the camera, and sets `frontend::PendingScreen(Home)`. When it returns to the lobby it also clears career and social.
- `perform_network_teardown`, `session.rs:472-522`, resets `GameStateSnapshot`, unlocks the camera, and clears `team_selection.team` if nothing was committed.
- `ingest_server_snapshot_packets`, `net/ingest.rs:50-204`, routes Social and Career packets into `SocialClient.apply_view/bind`, `CareerIdentity.observe_server_auth`, `CareerClient.apply_view` and `local_player_id` (lines 86-122, 158-167).
- `apply_server_snapshot` writes `TeamSelection` (prematch loadout at `apply.rs:230-248`, assigned team at 373-379), `CameraState.locked` and the `MainCamera` transform (500-513).

**E. QA harnesses (poll `is_connected`/`join_confirmed` for their autopilots; leave as is)**
`combat_qa.rs:167-201,339-358`, `map_qa.rs:152-172,448-468`, `navigation_qa.rs:216-230,344-392`, `targeting_qa.rs:254-271,418-480`, `team_vision_qa.rs:94-126,194-215`, `forest_pickup_qa.rs:117-148,208-224`, `visual_qa.rs:337-385,544,719,773`, `beta_ui_qa.rs:156-191,408-555,924`, `social_qa.rs:124-166,360-361`, `audio_qa.rs:133-157`, `offline_qa.rs:220-226,250`, `frontend_flow_qa.rs:155-366`, `sandbox/ui/qa.rs:97-402`.

**F. Plain `join_confirmed()`/`is_connected()` gates (no change)**
`edge_hud.rs:373,518`, `game_audio.rs:172,217`, `help_overlay.rs:55-62,221-228`, `input_context.rs:72-93`, `match_hud.rs:310-317`, `minimap.rs:777-781`, `minimap_route.rs:109`, `practice_sandbox.rs:224-269`, `sandbox/ui.rs:375-378`, `shop.rs:600-612,756-801`, `god_mode.rs:139-215`, `game_state.rs:109-153`, `frontend/home.rs:106-121,398-439`, `frontend/searching.rs:34-144`, `frontend/draft.rs:261-270`.

## 15.2 What `apply_server_snapshot` does today (`net/apply.rs:150-910`)

**Size.** The system is about 760 lines out of a 1341-line file; lines 969-1341 are 10 tests. It already has the maximum 16 parameters, using `SnapshotUiState` (lines 140-148) as a workaround.

**Order of work inside the system**
1. **Session** (207-211): state becomes Connected, `waiting_since = None`, `last_qualifying_snapshot_wall` is set.
2. **Resources** (213-227): `network_state.local_id` and 14 `GameStateSnapshot` fields (`your_id`, `prematch`, `match_mode`, `geometry_id`, `map_profile`, `meta`, `state`, `rematch_in_secs`, `team_buffs`, `combat_events`, `scoreboard`, `sandbox`, `forest_pickups`, `vision`).
3. **Prematch loadout** (230-248): copied into `TeamSelection` and `last_join`.
4. **Draft gate** (250-268): despawns the local hero and all remote players, then **returns**. Projectiles, structures, minions and neutrals are not touched during Draft.
5. **Local hero** (269-516):
   - It keeps exactly one `Player` (`choose_authoritative_local_player`, 126-138) and despawns duplicates.
   - It inserts 13 replicated components and `PlayerCosmeticAction` only when it changed.
   - Dash acknowledgement (`accept_dash_ack`, 81-95) removes `MovementTarget`/`MovementRoute` and writes `UtilityVfx::Dash`.
   - It snaps the position at `LOCAL_SNAP_DISTANCE` (widened by `speed_boost_active`).
   - If the hero doesn't exist, it spawns one of three bundles (2D sprite / 3D scene with `ModelScaleSource` / mesh fallback), sets `local_team`, and snaps and locks the camera.
   - **Hidden early return at 369-371:** if the local hero is listed but no team is selected and no join is committed, it returns and skips everything after it (remote players and all world entities).
6. **Remote players** (518-652): update the interpolation buffer (push, or teleport on dash, with `UtilityVfx::Dash`) and components, or spawn (3D adds `NormalizeModelScale`, `ModelScaleSource`, and a `SceneRoot` or mesh child); despawn stale ones recursively.
7. **Projectiles** (654-694), **structures** (696-747): transform, `StructureKind`, `Team`, `NetworkStructureId`, `NetworkStructureProtected`, `NetworkMapStructure`, `CombatStats`.
8. **Minions** (749-804, stale removal at 894-909): `NetEntityInterpolation`, brain, kind, action; 3D adds `NormalizeModelScale::scaled_by(0.9)`.
9. **Neutrals** (806-892): boss neutrals get `BossVisual`; stale removal.

**Resources it writes:** `ClientSession` (`state`, `waiting_since`, `last_qualifying_snapshot_wall`, `last_join`), `NetworkState` (`local_id`, `local_team`, `local_dash_ack`, 5 id→entity maps), `GameStateSnapshot`, `TeamSelection`, `CameraState`, the `MainCamera` transform, `PendingServerSnapshotFrame` (takes the frame), `Messages<UtilityVfx>`.

**Systems that depend on its side effects in the same frame (all `.after(ClientNetPipeline::ApplySnapshot)`)**
- `world.rs:204` `spawn_local_player_on_team`: checks `existing_players`, so it depends on the Commands flush. Pinned by `apply.rs:1219`.
- `combat.rs:188` round reset, `frontend/mod.rs:93` (`in_world` = a local `Player` exists).
- `game_state.rs:22,219`, `match_hud.rs:34`, `career.rs:420`, `career_identity.rs:517`, `match_service.rs:70`, `social.rs:400`, `sandbox/mod.rs:204`, `frontend/draft.rs:48`.
- `supporter.rs:289` (`NetworkSupporterAura`), `combat_feedback.rs:27` (`combat_events`), `game_audio.rs:38`.
- `AgeUtilityTimers`, `InterpolateNetEntities` (reads the `NetEntityInterpolation` inserted by Commands), `InterpolateRemotePlayers`, `InputContextSet::Modal`.
- In PostUpdate: `NetworkGroundingSet`, the presentation2d attach chain, and the combat bars (`combat.rs:1915` uses `try_insert` because apply may despawn a duplicate in the same frame).
- Before apply: `respawn_players_with_new_store_models` and `respawn_sandbox_models` (`net/mod.rs:122-129`).
- Test harnesses register apply directly: `net/test_fixtures.rs:120-128` and `net/session.rs:1043-1052` (in a `.chain()` with `update_session_lifecycle`, `offline::step`, `ingest`).

## 15.3 Design

**`RoundId`** (from step 10a, `client/src/domain/round.rs`): `struct RoundId { server_epoch: u64, match_id: u64 }` with `from_meta(&SnapshotMeta) -> Option<RoundId>`, returning `None` when either part is 0.

**`SessionEvent`** (`net/session.rs`, `#[derive(Message, Clone, Debug, PartialEq)]`, registered by `NetworkingPlugin`)
| Variant | Emitted at |
|---|---|
| `TransportStarted { addr: String, offline: bool }` | `transport.rs:61-110` (`spawn_network_transport`) |
| `Connected` | Apply `Session` stage, the edge at `apply.rs:207-210` |
| `Joined { your_id }` | Apply `Session` stage, edge of `join_confirmed()`. Tracked by a new `announced_join` flag reset in `clear_join_attempt()` (`session.rs:282-289`) |
| `Rejected(JoinRejection)` | `ingest.rs:146,150,168` (when the error changes to `Some`), `session.rs:376,813` |
| `JoinExhausted` | `session.rs:361,396` |
| `Disconnected { reason: TeardownReason, reconnecting: bool }` | `perform_network_teardown` (`session.rs:472-522`); `TeardownReason` becomes `pub(crate)` |
| `Left { returning_to: Option<String> }` | `LeaveMatch` (`session.rs:665-724`) |
| `ServerScopeReset` (career/social views are stale) | `session.rs:595-600`, `616-627` (only when the address changes; evaluated before `spawn_network_transport` overwrites `server_addr_display`), `652-657`, `712-717` |
| `RoundChanged { previous: RoundId, current: RoundId }` | Apply `Resources` stage |

- **`RoundChanged` tracking:** the last round goes into a new field `NetworkState.last_round: Option<RoundId>`. It is never reset (`despawn_tracked_net_entities` at `session.rs:459-460` only resets `local_id`/`local_team`). This reproduces the `CombatRoundIdentity` semantics: skip zero ids, ignore reconnects.
- **How events get out (outbox).** Emission goes through a buffer, `ClientSession.outbox: Vec<SessionEvent>` (`pub(in crate::net)`), not a `MessageWriter` in each system:
  - `apply_server_snapshot` and `update_session_lifecycle` are at or near the 16-parameter limit.
  - `perform_network_teardown` and `spawn_network_transport` are plain functions (the tests call them via `run_system_once`, `apply.rs:1153`).
  - One `flush_session_events` system drains the buffer. It runs unconditionally at the end of `ClientNetPipeline::ApplySnapshot` and chained after `retry_pending_join` in `SessionLifecycle`, which also catches `send_join_attempt` from `SendCommands`.
  - Struct literals `ClientSession { .., ..default() }` exist only inside `crate::net`, so the new private field doesn't break them.
- **`SessionReactions` system set,** configured `.after(ClientNetPipeline::SessionLifecycle)` in `configure_network_pipeline` (`net/mod.rs:64-84`). Consumers of lifecycle events must run in the same frame, before the next frame's `IngestSnapshot`.

**`SnapshotApplied`** (`net/apply.rs`):
```rust
#[derive(Message, Clone, Debug)]
pub struct SnapshotApplied {
    pub meta: SnapshotMeta,
    pub your_id: u64,
    pub round: Option<RoundId>,
    pub outcome: ApplyOutcome,   // Full | Draft | LocalPending (the 369-371 early return)
    pub local: LocalHeroApply,   // Unchanged | Updated { entity, corrected, dashed } | Spawned { entity, position, team } | Cleared
}
```
It is emitted by the `Finish` stage. Its first real consumers are the optional camera slice (15f) and QA frame loops that currently diff `meta.snapshot_tick`. HUD readers stay level-based. Gating `sync_draft` or `collect_hits` on it would change behaviour: teardown resets `GameStateSnapshot` without applying a snapshot, and `collect_hits` ages its effects every frame.

**Staged systems.**
- A new set `SnapshotApply::{Begin, Session, Resources, LocalPlayer, RemotePlayers, Projectiles, Structures, Minions, Neutrals, Finish}`, configured `.chain()` and each `.in_set(ClientNetPipeline::ApplySnapshot)`. Because the stages are members of `ApplySnapshot`, all 20 existing `.after(ClientNetPipeline::ApplySnapshot)` sites keep working.
- `Begin` moves `pending.frame.take()` into `StagedSnapshot { data: Option<PendingSnapshotData>, gate: ApplyGate }`. Each later stage runs `run_if(snapshot_staged)`.
- `Resources` sets `gate = Draft` when the prematch phase is Draft.
- `LocalPlayer` either runs the Draft despawn (both local and remote, as at 255-266) or sets `gate = LocalPending` for the 369-371 return.
- `RemotePlayers` through `Neutrals` run only when `gate == Full`.
- `Finish` emits `SnapshotApplied` and clears `StagedSnapshot`.
- A helper `pub(in crate::net) fn snapshot_apply_systems()` returns the chained configs, used by the plugin, `test_fixtures.rs:124` and `session.rs:1049`.
- The three duplicate spawn bundles at 393-495 become one `local_hero_components(state, your_id)` helper producing exactly the same component set.

**Consumers that switch to events**
- `combat.rs:86-135` and `mobile_controls.rs:647-659` read `RoundChanged`; `CombatRoundIdentity` and `MobileControls.round_identity` are deleted.
- `career.rs` (`clear_account`) and `social.rs` (`clear`) read `ServerScopeReset` in `SessionReactions`.
- `frontend` sets `PendingScreen(Home)` on `Left`.
- These keep polling: `shop.rs:762`, `edge_hud.rs:372`, `sandbox/mod.rs:251` and `frontend/draft.rs:148`. Their semantics differ from `RoundChanged` because they also fire on teardown to (0,0), so switching them would change behaviour.

## 15.4 Ordering hazards

1. **Commands flush between stages.** Bevy inserts sync points between chained systems that use Commands. Later stages will now see entities spawned by earlier stages, which the single system never did. The stale-despawn logic uses the `NetworkState` maps, not queries, so it's unaffected. The `.get(entity).is_ok()` checks (645, 687, 740, 885, 902) only look at entities from earlier frames. `world.rs:204` still runs after the whole set; the test at `apply.rs:1219` pins that.
2. **Both early returns must stay exact.** Draft skips all world entities. The uncommitted-local-hero case (369-371) skips remote players and all world entities. Add a test for each: no structure or minion update during Draft, and no remote spawn while the local hero isn't committed.
3. **Resources before players.** `GameStateSnapshot`, the prematch loadout and `last_qualifying_snapshot_wall` are written even when the entity stages are skipped, so `Session` and `Resources` come first, not last.
4. **Transform query conflicts.** The single `ParamSet` (155-158) splits into per-stage queries. The local stage needs `Query<&mut Transform, (With<Player>, Without<MainCamera>)>` plus a camera query that excludes `Player`. The entity stages use `With<NetworkProjectile>`, `With<NetworkStructure>` and so on.
5. **Message order.** `UtilityVfx` is written local first, then remote; the chain keeps that order.
6. **Unordered speed-boost mirror.** `mirror_debug_flags_to_network_state` (`net/mod.rs:120`) is unordered relative to apply today. Leave it unordered in the mechanical slice. Ordering it `.before(ApplySnapshot)` changes timing by at most one frame; put that in a separate PR.
7. **Event consumers must be ordered after the emitter.** A message written in frame N is seen next frame by any reader that ran earlier in frame N. `ServerScopeReset` consumers therefore have to be in `SessionReactions`. If they run in the next frame instead, `ingest` can apply a fresh Career view before `clear_account()` wipes it.
8. **Keep `TeamSelection.team = None` inside `net`** (`session.rs:505,575,694`). The comment at 692-693 explains that a snapshot already in flight would otherwise respawn the hero. It is join intent, not UI.
9. **`LeaveMatch` needs `match_service.take_return_to_lobby()` synchronously** (681-685) to choose the address, and `career_identity` to sign `CancelQueue`. Keep both reads in `net` as a documented exception.
10. **Test apps must register the message** (`add_message::<SessionEvent>()`) or leave out the flush system. The flush lives outside `update_session_lifecycle`, so tests at `session.rs:1227,1355` keep working. Their outbox just isn't drained.

## 15.5 Step 15 slices (in order; 543 tests unchanged unless noted)

| # | Slice | Size | Notes |
|---|---|---|---|
| 15a | Add `SessionEvent`, the outbox, `flush_session_events`, `NetworkState.last_round`, and emit at every site in the table; no consumers | S | New net tests: Connected before Joined; teardown gives `Disconnected{reconnecting:true}`; same-round reconnect gives no `RoundChanged`; new round gives `RoundChanged`; geometry mismatch gives `Rejected` |
| 15b1 | Pure extraction: `Begin`/`Session`/`Resources`/`Finish` stages plus one `Entities` system holding 250-909 unchanged; `SnapshotApplied`; `snapshot_apply_systems()` wired into the plugin, `test_fixtures.rs:124` and `session.rs:1049` | M | Removes `SnapshotUiState`; add the two early-return gate tests |
| 15b2 | Split `Entities` into `LocalPlayer`/`RemotePlayers`/`Projectiles`/`Structures`/`Minions`/`Neutrals`; deduplicate the spawn bundle | M | Hazards 1, 4, 5; `apply.rs:1007-1313` tests must pass unchanged |
| 15c | Round reset and mobile clear read `RoundChanged` | S | Rewrite `combat.rs:3728` to feed messages; move the teardown-gap assertion to a net apply test; update `mobile_controls.rs:1929-1938` |
| 15d | `ServerScopeReset` consumers (career, social) and `Left` → `PendingScreen` in `SessionReactions`; `update_session_lifecycle` drops the `CareerClient`/`SocialClient` params | M | Hazard 7; test pins same-frame clearing |
| 15e | Encapsulate `ClientSession`: make its public fields crate-private and add accessors (`state()`, `join_in_flight()`, `server_addr()`, `joined_prematch()`, `last_join()`) plus `#[cfg(test)]` builders | M | Touches about 12 test files listed in A; `abandon_join()` stays public |
| 15f (optional) | Camera spawn snap and unlock through `SnapshotApplied`/`SessionEvent`; removes `CameraState`/`MainCamera` from `net` | S | Behaviour note: `StartOffline` does not unlock the camera today (only `ConnectAllocated` 658, `LeaveMatch` 696 and teardown 499 do), so its own PR. `world.rs:456-470` uses `locked_camera_offset` while apply uses `locked_camera_offset_for_team`; keep both |
| 15g (optional) | Career and Social packets from `ingest` become messages | M | They need a `before(ApplySnapshot)`-equivalent consumer order |

---

# Step 10: domain module, combat/player split, run_if backends, plugin groups, QA feature

## 10.1 Current plugin list (`client/src/lib.rs:135-214`) and build-order constraints

In registration order:
1. `DefaultPlugins` (135-142).
2. `CameraPlugin`, `PlayerPlugin` (movement chain, gravity, respawn UI, animation), `SpriteVisualsPlugin` (**inserts `PlayerVisualMode::from_environment()`**, `sprite.rs:123`), `Presentation2dPlugin`, `MapsPlugin`, `ClientPersistencePlugin`, `SetupPlugin` (world scene, camera, lighting, fallback local spawn `world.rs:201-206`), `World2dPlugin`, `NetworkingPlugin`, `BossesPlugin`, `MinimapPlugin` (nests `MinimapRoutePlugin`), `CombatPlugin`, `MatchHudPlugin`, `TeamSelectPlugin`, `GameStateUiPlugin` (143-159).
3. `SandboxPlugin` (installs `sandbox/ui/qa.rs` via `sandbox/ui.rs:278`), `EdgeHudPlugin`, `MatchServicePlugin`, `FrontendPlugin` (nests 9 screen plugins, `frontend/mod.rs:96-106`), `FrontendQaPlugin` (160-163).
4. `InputContextPlugin`, `UiKitPlugin`, `ShopPlugin`, `HelpOverlayPlugin`, `DebugConsolePlugin`, `PauseMenuPlugin`, `PracticeSandboxPlugin`, `GodModePlugin`, `ModelScalePlugin`, `MinionVisualsPlugin` (164-175).
5. `DecorPlugin`, `Presentation3dPlugin`, `JungleVisualsPlugin`, `Verdant3dPlugin`, `VisualQaPlugin` (177-182).
6. `MobileControlsPlugin`, `MobileUiPlugin` (183-186).
7. `CombatVisualsPlugin`, `ProjectileVisualsPlugin`, `CombatFeedbackPlugin` (187-191).
8. `SocialPlugin`, `ReactionVisualsPlugin`, `SocialQaPlugin` (192-196).
9. `CareerPlugin`, `SupporterPlugin`, `SupporterStoreKitPlugin`, `GameAudioPlugin`, `GameVfxPlugin`, `BattlefieldAtmospherePlugin`, `TeamVisionPlugin`, `TeamVisionQaPlugin`, `AudioQaPlugin`, `OfflineQaPlugin`, `CareerIdentityPlugin`, `CareerVisualQaPlugin`, `MapVisualsPlugin`, `MapQaPlugin`, `CombatQaPlugin`, `ForestPickupQaPlugin`, `TargetingQaPlugin` (197-213).
10. Also in `main` before the app: `animation_qa::run` (106-109) and `model_scale::run_model_measurement_analyzer` (116-119).

Build-time dependencies (the only reads of the world inside `build`):
- `UiKitPlugin` must build before `MobileControlsPlugin` (`mobile_controls.rs:579` reads `UiPlatform`).
- `MobileControlsPlugin` must build before `MobileUiPlugin` (`mobile_ui.rs:20` does `resource::<MobileControls>()` and panics if missing).
- `VisualQaPlugin` nests `BetaUiQaPlugin`/`NavigationQaPlugin` (`visual_qa.rs:62-69`); `FrontendQaPlugin` nests `AvatarQaPlugin`/`FrontendFlowQaPlugin` (`frontend_qa.rs:68-75`).
- `FrontendQaPlugin` inserts `ScreenDriverPaused(true)`. It is correct in either order relative to `FrontendPlugin`'s `init_resource`.
- No two production plugins `insert_resource` the same type at build time. QA plugins only add `WinitSettings::continuous()`, which overrides `platform.rs:14`.

## 10.2 Plugin groups (`client/src/plugins.rs`; `lib.rs` becomes the module list plus `main`)

- **`NetPlugins`:** `ClientPersistencePlugin`, `NetworkingPlugin`, `MatchServicePlugin`, `CareerIdentityPlugin`.
- **`GameplayPlugins`:** `MapsPlugin`, `InputContextPlugin`, `PlayerPlugin`, `CombatPlugin`, `SandboxPlugin`, `PracticeSandboxPlugin`, `GodModePlugin`, `DebugConsolePlugin` (the last four move to a debug group in step 11).
- **`PresentationPlugins`:**
  - Shared: `CameraPlugin`, `SetupPlugin`, `ModelScalePlugin`, `CombatVisualsPlugin`, `CombatFeedbackPlugin`, `GameVfxPlugin`, `ReactionVisualsPlugin`, `TeamVisionPlugin`, `GameAudioPlugin`, `MapVisualsPlugin`.
  - 2D: `SpriteVisualsPlugin`, `Presentation2dPlugin`, `World2dPlugin`.
  - 3D: `Presentation3dPlugin`, `Verdant3dPlugin`, `DecorPlugin`, `JungleVisualsPlugin`, `MinionVisualsPlugin`, `BossesPlugin`, `ProjectileVisualsPlugin`, `BattlefieldAtmospherePlugin`.
  - All are always added and gated by run_if; see 10.4 for why.
- **`UiPlugins`,** in this order: `UiKitPlugin`, `MobileControlsPlugin`, `MobileUiPlugin`, `FrontendPlugin`, `TeamSelectPlugin`, `GameStateUiPlugin`, `MatchHudPlugin`, `EdgeHudPlugin`, `MinimapPlugin`, `ShopPlugin`, `HelpOverlayPlugin`, `PauseMenuPlugin`, `SocialPlugin`, `CareerPlugin`, `SupporterPlugin`, `SupporterStoreKitPlugin`.
- **`QaPlugins`** (`#[cfg(feature = "qa")]`): `FrontendQaPlugin`, `VisualQaPlugin`, `SocialQaPlugin`, `TeamVisionQaPlugin`, `AudioQaPlugin`, `OfflineQaPlugin`, `CareerVisualQaPlugin`, `MapQaPlugin`, `CombatQaPlugin`, `ForestPickupQaPlugin`, `TargetingQaPlugin`.
- `main`: `app.insert_resource(PlayerVisualMode::from_environment())` before the groups, removed from `SpriteVisualsPlugin::build`, so the choice is made once and visibly. Then `.add_plugins((NetPlugins, UiPlugins, GameplayPlugins, PresentationPlugins))` and, under the feature, `QaPlugins`.
- This removes the 15-element tuple workaround (`lib.rs:176`).

## 10.3 Splitting `combat.rs` (3866 lines; tests 2478-3863) and `player.rs` (3155 lines; tests 1882-3146)

**What `combat.rs` does:** round reset, cooldown mirror, action feedback line, skill hotbar UI, target picking, casting with pending approach, mobile cast/utility, world HP/mana bars, target marker. It has no VFX (that lives in `combat_feedback.rs`, `game_vfx.rs`, `presentation*`).

**`client/src/combat/`**
| File | Contents (current `combat.rs` lines) |
|---|---|
| `mod.rs` | `CombatPlugin` (167-238), `configure_target_presentation` (243-256), re-exports. The sets `CombatPointerInputSet`/`WorldMovementInputSet` (169-173) move to `input_context.rs` with re-exports |
| `cooldown.rs` | `LocalCastCooldown` (27-48), `tick_local_cast_cooldown` (437-456), `sync_authoritative_cooldown_durations` (457-518), `effective_cast_duration` (519-535), `local_hero_class` (536-545) |
| `feedback.rs` | `ActionFeedback`, `ActionFeedbackText`, `update_action_feedback` (50-84), `adapt_mobile_combat_feedback` (1779-1794) |
| `round_reset.rs` | 86-135 |
| `selection.rs` | pick radii (145-149), `TargetState` (295-300), `WorldPointerState` (302-308), `TargetCandidates` (350-400), `select_target_system` (990-1091), `find_nearest_enemy_target`/`find_target_near_screen`/`consider_screen_target`/`screen_pick_distance` (2143-2477) |
| `cast.rs` | `PendingCastRequest`/`PendingCast` (310-348), `try_cast_slot` (1092-1174), `queue_cast_request` (1175-1207), `cast_spell_system` (1208-1254), `resolve_pending_cast_system` (1312-1499), `within_cast_range` (1795-1798) |
| `mobile.rs` | 1500-1778 |
| `hotbar.rs` | skill constants (158-165), slot components (411-435), `setup_combat_ui` (644-815), `update_skill_bar_system` (816-954), `skill_upgrade_input_system` (955-989), `skill_button_system` (1255-1311) |
| `bars.rs` | bar constants (137-144), `CombatVisualAssets` (283-293), `CombatBars`, `CombatBarRoot`, `CombatBarAnchor` (402-406, 546-554), `setup_combat_visual_assets` (555-643), bars systems (1799-2069) |
| `marker.rs` | `TargetMarker`, marker constants (150-157), `update_target_marker_system` (2070-2142) |
| `targeting.rs` | Today's `client/src/targeting.rs` moved verbatim (basic attack, aim UI, locked target); `lib.rs` keeps `pub(crate) use combat::targeting;` so `crate::targeting::…` still resolves |
| `tests.rs`, `target_presentation_tests.rs` | The 22 + 4 tests, unchanged |

`CombatStats` (258-281) moves to `domain`.

**What `player.rs` does:** input, local prediction/motion, a large 3D animation pipeline, respawn UI. Server reconciliation of the local hero lives in `net/apply.rs:324-362`; that is the seam to step 15b2.

**`client/src/player/`**
| File | Contents (current `player.rs` lines) |
|---|---|
| `mod.rs` | `PlayerPlugin` (65-92), `DebugSpeedBoost`, constants, `ground_origin_y` (30-63) |
| `input.rs` | `MovementTarget`/`MovementRoute` types (144-155, which go to domain), `handle_player_input` (1076-1179), `move_player_mobile` (1180-1277), `mobile_screen_direction` (1278-1293), `secondary_move_pressed` (1294-1303), `plan_movement_routes` (1304-1393), `should_issue_ground_move`, `viewport_to_simulation_world` (1394-1420) |
| `motion.rs` | `Jumping` (157-162), `move_player` (1421-1541), `SandboxVisualClock` (1542-1573), `animate_jump` (1574-1604), `apply_gravity` (1605-1655), collisions (1761-1881), `hero_movement_multiplier` (3147-3155) |
| `respawn_ui.rs` | 124-141, 1656-1760 |
| `animation.rs` | `register_hero_animation_systems` (95-112), 164-1075 (library, playback, binding, sandbox seek, `sync_player_animation_state`) |
| `tests.rs`, `animation_tests.rs` | 1882-2379, 2380-3146 |

Markers `Player`, `PlayerBody`, `VerticalVelocity` (114-121) move to `domain`.

> Status (10b, 10c): done as tabled (`refactor/client-combat-split`), items re-located by name. `CombatStats`, the two input sets, the markers and `MovementTarget`/`MovementRoute` had already moved in 10a, so `combat/mod.rs` and `player/mod.rs` re-export them (the movement intents stay in `domain`, not in `input.rs`). `animation.rs` is 951 lines and `combat/tests.rs` 1395; both stay single files. `sync_jump_fallback_mode` is in `animation.rs` (inside the tabled range) although `PlayerPlugin` registers it at the head of the movement chain. Nothing was widened to `pub(crate)`; besides items and fields used across files or by the test modules, six component/resource types became `pub(super)` because `pub(super)` systems name them in their signatures (`CombatVisualAssets`, `CombatBars`, `CombatBarRoot`, `SkillBarSlot`, `SkillUpgradeButton`, `SkillNameLabel`).

## 10.4 2D vs 3D

**Why run_if and not conditional plugins.** Every plugin must still be added, because 2D resources are read in both modes:
- `SpriteVisualAssets` in `team.rs:284,354`, `career.rs:2076`, `minimap.rs:442`.
- `MapVisualRegistry` in `world2d.rs:627`.

The mode is never mutated in production; `sprite.rs:123` is the only non-test writer.

**Conditions.** Add `sprite::in_models3d()` and `in_sprite2d()`, both `resource_exists_and_equals(PlayerVisualMode::…)`, which is safe when the resource is missing. Keep every internal `if *mode != …` guard: tests register systems directly and flip modes, for example `presentation3d.rs:385-419`.

**Gate with `in_sprite2d()`**
- `sprite.rs:130-138` (and the Startup at 125-129)
- `presentation2d.rs:202-225`
- `world2d.rs:193-194`

**Gate with `in_models3d()`**
- `presentation3d.rs:24`
- `verdant3d.rs:17-21`
- `jungle.rs:15-19`
- `minions.rs:17-29`, including creature asset setup
- `bosses.rs:95-110`
- `decor.rs:11`
- `map_visuals.rs:452-462` (prop chain only) and `map_visuals/river.rs` `repair_river`
- `projectile_visuals.rs:25-31` (`setup_assets`, attach, update, orbits)
- `world.rs:192-193` (`force_vrm_models_double_sided`, `apply_lighting_settings_system`) and `sync_selected_player_assets` (guard at 403)
- `battlefield_atmosphere.rs:13-14`, but check its `sync_visibility` first (I didn't see a mode guard)

> Status (10d): gated as listed, except `battlefield_atmosphere.rs`, which stays ungated. Its mist is a screen-space UI node with no mode guard, shown in both modes while the game is Running, and `overlay_is_single_noninteractive_full_viewport_and_match_scoped` adds the plugin without a `PlayerVisualMode`. `river.rs` gates its whole registration (`repair_river` + `remove_orphaned_replacements`, a no-op without 3D replacements); `world.rs` gates the three systems as one set.

**Must run in both modes (they branch internally)**
- `camera.rs` systems
- `net/apply` (spawn bundles)
- combat bars (`CombatVisualAssets.is_2d`), targeting draw (`targeting.rs:398,798`)
- `game_vfx.rs` (348-1153), `combat_feedback.rs` (141-308), `supporter.rs:535,591` auras
- `minimap.rs:514,856`, `team_vision.rs:139,288-300`, `world.rs` `setup_scene`/`setup_main_camera`/fallback spawn
- player movement (`mobile_screen_direction`, `viewport_to_simulation_world`)
- `projectile_visuals.rs:472` `draw_trails`
- `map_visuals` `request_config`/`apply_config` (used by `world2d.rs:627`)
- **Do not gate `register_hero_animation_systems`:** `animation_qa.rs:103` uses it without inserting `PlayerVisualMode`. It is a no-op in 2D.

The existing opposite-mode tests already pin "nothing attaches": `jungle.rs:190`, `minions.rs:315`, `bosses.rs:529`, `verdant3d.rs:294`, `presentation2d.rs:2373`.

## 10.5 QA behind a cargo feature

**QA modules and what gates them today (env vars only; nothing is cfg-gated).**
- `animation_qa` (`OMOBA_ANIMATION_QA`, `lib.rs:106`).
- `visual_qa` (`OMOBA_VISUAL_QA_DIR`, and it dispatches to `beta_ui_qa`, which pulls in `edge_hud_qa.rs` via `#[path]` at `beta_ui_qa.rs:26`, and to `navigation_qa`).
- `targeting_qa`, `combat_qa`, `map_qa`, `forest_pickup_qa`, `team_vision_qa`: `OMOBA_VISUAL_QA_SCENARIO` plus `OMOBA_VISUAL_QA_DIR`.
- `frontend_qa` (`OMOBA_FRONTEND_QA_OUTPUT`), which dispatches to `frontend_qa/avatar.rs` (`OMOBA_AVATAR_QA`) and `frontend_flow_qa` (`OMOBA_FRONTEND_QA_FLOW`).
- `social_qa` (`OMOBA_SOCIAL_QA_OUTPUT`), `audio_qa` (`OMOBA_AUDIO_QA_OUTPUT`), `offline_qa` (`OMOBA_OFFLINE_SMOKE_DIR`), `career_visual_qa` (`OMOBA_CAREER_QA_OUTPUT`), `sandbox/ui/qa.rs` (`OMOBA_SANDBOX_QA_OUTPUT`).
- QA code inside production modules:
  - `supporter.rs:253-266` (`OMOBA_SUPPORTER_QA_DIR`)
  - `career_identity.rs:524,561`
  - `frontend/mod.rs:125-134` (`bypass_for`, which is automation, so keep it)
  - `minimap.rs:92-116` (`MinimapQaScene`, used only by QA and the test at 1315)

**How the client is built today.**
- `Makefile:82-103` uses `cargo run -p client`.
- CI (`.github/workflows/ci.yml`) runs clippy/test on the workspace.
- `scripts/package_native.py:38-43` builds the workspace with `--profile dev|release`.
- `scripts/capture_verdant.py:356,390-394` runs visual QA **against packaged builds**. `capture_*.py` take `--client-bin`. `combat_test.py:53` runs `cargo build`.
- Mobile: `mobile/android/build.py:122`, `mobile/ios/xcode_build.py:86`, `build_device.py:343`, `build_simulator.py:64`. None of them use QA env vars.

**Plan**
1. `client/Cargo.toml`: `[features] default = ["qa"]` and `qa = []`. A cargo feature can't depend on the profile, so "on for dev" means on by default. Nothing in `make`, CI, packaging or `scripts/` changes.
2. Move the 14 QA files (plus `frontend_qa/`, `edge_hud_qa.rs`) into `client/src/qa/`, with a `qa/mod.rs` that owns `QaPlugins`. Only historical `docs/progress` notes reference the file paths; scripts use env vars only.
3. `#[cfg(feature = "qa")]` on `mod qa`, on `animation_qa::run` in `main`, on `sandbox/ui.rs:1,278`, on the supporter QA block (or move it to `qa/supporter.rs`), and `#[cfg(any(test, feature = "qa"))]` on `MinimapQaScene`.
4. CI and a Makefile `check-no-qa` target: `cargo clippy -p client --lib --no-deps --no-default-features -- -D warnings`. `--no-default-features` touches only client's own features; Bevy's features come from the dependency entries. This catches `dead_code` in production items that only QA uses; expect a few beyond `MinimapQaScene`. Tests with the feature off would be 523.
5. Optional last slice: mobile store builds add `--no-default-features` (`xcode_build.py:86` for the release/TestFlight profile, `android/build.py:122`). Check that `mobile/ios/test_xcode_build.py:89` only asserts `--locked`.

## 10.6 Domain module: worth doing, but keep it small

Models are split across modules, and today `net` depends on `combat`, `player` and `team` only for these types:
- `CombatStats` (`combat.rs:258-281`, used in 34 files; `net/apply.rs:13`).
- `Team` with its `shared::map::Team` conversions (`team.rs:40-96`, 43 files, inside an 1887-line UI file).
- `Player`, `PlayerBody`, `VerticalVelocity` (`player.rs:114-121`), `RemotePlayer` (`net/components.rs:51`).
- `MovementTarget`, `MovementRoute` (`player.rs:144-155`).
- About 12 ad-hoc `(u64, u64)` round tuples (15.1 C).

Proposed `client/src/domain/`: `mod.rs`, `round.rs` (`RoundId`), `team.rs`, `stats.rs` (`CombatStats` + `MAX_HP`/`MAX_MANA`), `actors.rs` (markers and movement intents). Re-export from the old paths (`pub use crate::domain::Team;` and so on) so the slice has no import churn.

Leave in place:
- The replicated hero components in `net/components.rs` (`PlayerUtility`, cooldowns, `PlayerProgression`, `PlayerEquipment`, `PlayerCosmeticAction`); they already have one home.
- Input-intent state (`TargetState`, `PendingCast`, `BasicAttackState`); it belongs to the combat submodules.
- `TeamSelection` (lobby intent).

## 10.7 Step 10 slices

> Status: 10a and 10d landed together in #35 (`refactor/client-domain`, progress note `docs/progress/2026-09-24-client-domain.md`); 10b and 10c are one PR (`refactor/client-combat-split`, progress note `docs/progress/2026-09-24-client-combat-player-split.md`); 10e onwards are open.

| # | Slice | Size | Risk |
|---|---|---|---|
| 10a | `domain/` with `RoundId`, `Team`, `CombatStats`, actor markers; re-exports; input sets moved to `input_context.rs` | S | Low; mechanical |
| 10b | `combat.rs` → `combat/` (and `targeting.rs` → `combat/targeting.rs`), verbatim moves; private items become `pub(super)`/`pub(in crate::combat)` | L (mechanical) | Visibility churn; keep it a move-only diff like step 8 |
| 10c | `player.rs` → `player/` | M/L (mechanical) | Same as 10b |
| 10d | `in_models3d()`/`in_sprite2d()` gates at the registration sites in 10.4 | S | Low, if you use `resource_exists_and_equals` and keep the internal guards |
| 10e | `plugins.rs` groups; mode insertion moved to `main` | M | Build-order constraints (UiKit → MobileControls → MobileUi); the relative order of unordered systems can change under the multi-threaded executor. Run the capture scripts as a smoke test |
| 10f | Move QA to `qa/`, add the `qa` feature (default on), cfg the hooks | M | Dead code with the feature off; the Python tests must not change |
| 10g | CI/Makefile `--no-default-features` clippy | S | CI time: one more client lib compile |
| 10h (optional) | Mobile store builds without QA | S | `mobile/ios/test_xcode_build.py` |
| 10i (optional) | Migrate imports off the re-export shims | M | Churn only |

### Critical Files for Implementation
- /home/user/omoba-bevy/client/src/net/apply.rs
- /home/user/omoba-bevy/client/src/net/session.rs
- /home/user/omoba-bevy/client/src/net/mod.rs
- /home/user/omoba-bevy/client/src/lib.rs
- /home/user/omoba-bevy/client/src/combat.rs