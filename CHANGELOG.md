# Changelog

All notable changes to this repository should be documented in this file.

The canonical repository version lives in `Cargo.toml` under `[workspace.package].version` and follows SemVer.

## [Unreleased]

### Client debug module and session accessors
- New `client/src/debug/` module (roadmap slice 11d). `god_mode.rs` moved to `debug/hud.rs`, `debug_console.rs` to `debug/console.rs` and `practice_sandbox.rs` to `debug/tools_page.rs`; entity `Name`s and test ids are unchanged, and no re-exports are left at the old paths. `DebugToggles { god_mode, speed_boost }` replaces the three client copies: `DebugToggleState` (HUD god mode), `PracticeSandboxState.god_mode` (the practice page's line) and `player::DebugSpeedBoost` (HUD speed boost, read by local movement). The HUD hotkeys and buttons, the practice page, local movement and the local-player snapshot stage read it; `mirror_debug_flags_to_network_state` and `NetworkState.speed_boost_active` are gone because `apply_snapshot_local_player` now reads `DebugToggles` directly (16 parameters). The 0.5 s re-send is `debug::resend_debug_toggles`, still only while the HUD is enabled (`OMOBA_DEBUG_UI` and not in Combat Test).
- `NetworkCommand::Debug(DebugCommand)` replaces `SetGodMode`, `SetSpeedBoost` and `Practice`; `send_network_commands` sends `command.to_packet()` behind the same `join_confirmed()` gate, so the packets are byte-identical. The offline simulation routes every debug packet through `DebugCommand::from_packet` into one `Simulation::debug`, where the speed boost is an explicit no-op (offline accepts any finite client transform). The practice page's visibility is `DebugAccess::for_match_mode(match_mode).practice` instead of the `"practice" | "offline_practice"` literal (same result).
- New `DebugPlugins` group (`SandboxPlugin`, `PracticeSandboxPlugin`, `GodModePlugin`, `DebugConsolePlugin`), added right after `GameplayPlugins`, where the four used to be last, so the build order is unchanged.
- `ClientSession` is encapsulated (roadmap slice 15e, which completes step 15): every field is private to `net`. Outside code reads `state()`, `join_in_flight()` (the old `join_flow_committed`), `server_addr()` (the old `server_addr_display`) and `joined_prematch()`, next to the existing `is_connected()`, `is_offline()`, `join_confirmed()`, `has_committed_join()`, `join_blocked()`, `join_rejection()` and `is_choosing_loadout()`; `abandon_join()` stays the only public write. Tests use new `#[cfg(test)]` setters (`set_state_for_test`, `set_join_in_flight_for_test`, `set_server_addr_for_test`, `set_joined_prematch_for_test`, `clear_last_join_for_test`) next to the existing constructors.
- No wire change. Behaviour is unchanged except where the three copies used to disagree, which only shows with `OMOBA_DEBUG_UI`: the practice page's god mode line now also shows a god mode turned on with F2 or the HUD button, and leaving a practice match now also clears the HUD's god mode (before, the HUD kept showing and re-sending it into the next match). The clear happens on the edge out of a practice match, so a HUD toggle in a dev match is kept. Tests: client lib 561 → 562 (541 → 542 without `qa`), for the test that pins the practice reset and the dev toggle surviving; shared unchanged.

### Debug command family
- New `shared::debug`: `DebugCommand { GodMode(bool), SpeedBoost(bool), Practice(PracticeCommand) }` is one in-process type for the development toggles and the practice sandbox. It has no serde derives: `to_packet()` and `from_packet(&ClientPacket)` map it onto the existing `SetGodMode`, `SetSpeedBoost` and `Practice` packets, which stay the wire encoding (no new `ClientPacket` variant, golden strings and `CLIENT_PACKET_TAGS` unchanged; a test pins every command's encoding to them). `DebugAccess { toggles, practice }` with `for_match_mode(&str)` (`dev`: toggles; `practice` and `offline_practice`: both; anything else: neither) and `allows(DebugCommand)`. The constants `DUMMY_MAX_HP`, `DUMMY_DISTANCE` and `OFFLINE_PRACTICE_MODE` moved here from `server/src/bots.rs` and `client/src/net/offline.rs`. `shared::practice` stays and is re-exported from `shared::debug`.
- `PracticeCommand` gained `#[serde(other)] Unsupported`: a practice command with an unknown `kind` now decodes and is ignored by the server and the offline simulation instead of failing the whole datagram, so future practice commands are additive. The datagram now counts as traffic from the endpoint like a refused practice command did (it refreshes the endpoint's last-seen time) and still runs nothing.
- New `server/src/debug/` (`mod.rs`, `toggles.rs`, `practice.rs`): `ServerRuntime::debug_access()` (the toggles with `rules.debug_commands`, practice with `rules.fills_with_bots`, each only without a worker allocation; a test pins it to `DebugAccess::for_match_mode(rules.mode_id())` for every mode) and `ServerRuntime::handle_debug(addr, DebugCommand, now)`, the one entry point. The practice orchestration (the command handler, the dummy anchor, the duelist set-up) moved out of `bots.rs`; the roster primitives (`spawn_bot`, `remove_bot`, `remove_all_bots`, `place_dummy`) and the bot AI stay there. `runtime/handlers/debug.rs` is now `runtime/handlers/tools.rs` with only the Combat Test `Sandbox` packet. The dispatcher arms keep their positions, clocks and control flow (`Practice` before the paused-sandbox gate on the wall clock, always skipping the post-command tail; the toggles after it on the simulation clock, running the tail once applied) and convert the packet with `DebugCommand::from_packet`.
- New pure planners: `shared::progression::skill_upgrade_order(class, level, ranks, points)` (the ultimate first once unlocked, then Q, W, E) and `shared::shop::plan_purchases(class, gold, owned)` (greedy in the class's recommended order). Server `auto_rank_skills`/`auto_shop` (lane bots and the practice duelist, still one ordinary upgrade or purchase per step), the harness bots' `choose_shop_item` and the offline duel (its `ranks_for_level` and `shop_with` are gone) use them; the catalog's starter-budget test uses `plan_purchases` instead of a local copy. A temporary equivalence test compared the old and new code over every class × level 1-10 × budget 0-1000 (server: every rank vector and point count, every owned-item subset; harness: every owned subset and eligibility; offline: the real `StartDuel` path) and passed before the old code was deleted.
- No other behaviour change and no wire-visible change. Tests: shared 88 → 94 (command encodings, unknown practice kind, access table, two upgrade-order tests, purchase plans), server 283 → 285 (+3 ignored; access parity, unknown practice kind over a datagram), client lib 549 and harness 22 + 24 unchanged.

### Client snapshot stages and session reactions
- The `SnapshotApply::Entities` stage is split into `LocalPlayer`, `RemotePlayers`, `Projectiles`, `Structures`, `Minions` and `Neutrals`, chained between `Resources` and `Finish` inside `ClientNetPipeline::ApplySnapshot`. `LocalPlayer` runs the Draft despawn (every hero, local and remote) or closes the gate with `LocalPending` when the listed local hero has no team and no committed join; the other five run only while the gate is `Full`. The `Transform` `ParamSet` gave way to per-stage filtered queries (hero `With<Player>, Without<MainCamera>`, camera `With<MainCamera>, Without<Player>`, `With<NetworkProjectile>`, `With<NetworkStructure>`, `With<NetworkMinion>`, `With<NetworkNeutral>`), and the dash VFX are still written local first, then remote. The three local-hero spawn bundles (2D sprite, 3D scene, mesh fallback) share one `local_hero_components` helper with the same component set. `SnapshotUiState` is removed. `SnapshotApplied` gains `local: LocalHeroApply` (`Unchanged`, `Updated { entity, corrected, dashed }`, `Spawned { entity, position, team }`, `Cleared`); nothing outside the tests reads `SnapshotApplied` yet.
- The combat round reset (`combat/round_reset.rs`) and the mobile-controls round clear (`read_mobile_controls`) react to `SessionEvent::RoundChanged` instead of tracking the round themselves; `CombatRoundIdentity` and `MobileControls.round_identity` are deleted. Both still run in the frame the new round is applied (after `ApplySnapshot`, whose flush writes the event) and before input. The polling round checks in the shop, the edge HUD, the Combat Test sandbox and the draft screen are unchanged, because they also react to the zero ids a teardown leaves.
- `CareerClient::clear_account()` and `SocialClient::clear()` run on `SessionEvent::ServerScopeReset` (`career::clear_account_on_scope_reset`, `social::clear_on_scope_reset`), and the front end requests the Home screen on `SessionEvent::Left` (`frontend::return_home_on_leave`), all in `SessionReactions`, the same frame as before. `update_session_lifecycle` no longer takes `CareerClient` or `SocialClient` and no longer inserts `PendingScreen`; it keeps clearing `TeamSelection.team`, taking the lobby address from `MatchServiceClient` and signing `CancelQueue` with `CareerIdentity`.
- No behaviour or wire change. The one scheduling difference: each entity stage's Commands are applied at their own sync point, so later stages see earlier spawns and a stale minion is despawned before the neutrals are reconciled instead of after; no stage looks up entities spawned in the same frame by another stage. Tests: client lib 557 → 561 (537 → 541 without `qa`): what `LocalHeroApply` reports (spawn, update, correction, dash, Draft clear) together with the local-then-remote dash VFX order, the teardown gap keeping the last round (moved here from the combat test), same-frame career and social clearing on a scope reset with the next frame's career view surviving, and `Left` requesting Home. The combat round-reset and mobile round-clear tests now feed `RoundChanged`, and the offline leave test runs the front end's reaction. Shared 88 unchanged.

### Client session events
- New `SessionEvent` message (`crate::net::SessionEvent`, registered by `NetworkingPlugin`) for the session's lifecycle edges: `TransportStarted { addr, offline }` (every `spawn_network_transport`), `Connected` (the first applied snapshot of a connect attempt), `Joined { your_id }` (once per join attempt, when `join_confirmed()` becomes true), `Rejected(JoinRejection)` (when the join error changes to a new rejection: protocol or map-geometry mismatch and server rejections in ingest, a denied avatar ticket, the transport's protocol-mismatch signal), `JoinExhausted` (retry budget spent or the Join could not be queued), `Disconnected { reason, reconnecting }` (every teardown; `reconnecting` is false when the protocol-mismatch branch cancels the reconnect), `Left { returning_to }` (`LeaveMatch`), `ServerScopeReset` (wherever career and social are cleared: `StartOffline`, `ConnectTo` to a new address, `ConnectAllocated`, `LeaveMatch` back to the lobby) and `RoundChanged { previous, current }` (a snapshot of another round; zero ids are skipped and a reconnect to the same round is not a change, as `CombatRoundIdentity` does it).
- Emission goes through `ClientSession.outbox` (crate-private to `net`); `flush_session_events` writes the queue as messages at the end of `ClientNetPipeline::ApplySnapshot` and after `retry_pending_join` in `SessionLifecycle` (it bypasses change detection, so `ClientSession` change ticks are as before). New `NetworkState.last_round` and `ClientSession.announced_join`; `TeardownReason` is `pub(crate)`. New empty `SessionReactions` set, ordered after `SessionLifecycle`, for same-frame consumers. Nothing reads the events yet.
- `apply_server_snapshot` is split into the chained `SnapshotApply::{Begin, Session, Resources, Entities, Finish}` stages, all inside `ClientNetPipeline::ApplySnapshot`: `Begin` moves the pending frame into the new `StagedSnapshot`, `Session` and `Resources` are the old first part (connection state, `GameStateSnapshot`, prematch loadout, Draft gate), `Entities` is the old remainder unchanged (both early returns now record the gate), and `Finish` writes the new `SnapshotApplied { meta, your_id, round, outcome }` message (`ApplyOutcome::{Full, Draft, LocalPending}`, no reader yet) and clears the staged frame. `snapshot_apply_systems()` builds the chain for the plugin and the net test apps. `SnapshotUiState` lost `GameStateSnapshot` and stays until the entity stage is split (15b2).
- No behaviour or wire change. Tests: client lib 549 → 557 (529 → 537 without `qa`): Connected before Joined, a reconnecting `Disconnected` on teardown, no `RoundChanged` on a same-round reconnect but one for a new match, one `Rejected` for a geometry mismatch, the protocol-mismatch teardown, the scope-reset/transport/leave order, and the two early-return gates (Draft leaves world entities alone; a listed local hero without team or join spawns nothing). Shared 78 unchanged.

### Data-driven catalogs
- New `shared/assets/catalog/heroes.json` and `shared/assets/catalog/items.json` (`schema_version` 1) hold everything that differs per class or per item: display name, tagline, draft role, base HP, growth caps, basic attack, projectile style, the Q/W/E/R kit (id, name, description, targeting, mana cost, cooldown, cast range and one of projectile damage, self heal or self mana restore) and the recommended item order; item name, description, cost and bonuses. Uniform rules stay in code: `HeroClass`, `ItemId`, `SkillSlot`, rank scaling, unlock levels, the global values in `shared::hero_balance`, `STARTING_GOLD`, `INVENTORY_CAPACITY`, `SHOP_RADIUS` and the Warden's jungle passive.
- New `shared::catalog`: both files are embedded with `include_str!` and parsed once into `LazyLock` tables through private `deny_unknown_fields` structs, then validated (classes and items match their enums one to one and in order, four abilities with exactly one effect each and targeting consistent with it, unique ability ids, positive HP, costs and cooldowns, multipliers of at least 1, flat bonuses of at least 0, known projectile style and bonus fields, every item recommended exactly once per class). Bad data panics with the file and the entry. `catalog::ensure_loaded()` runs first in the server's `runtime::run` and the client's `main`. There is no runtime override; client and server must agree.
- The accessors keep their names and signatures and read the catalog: `basic_attack_for_class`, `HeroClass::{display_name, tagline, primary_role, abilities, ability}`, `ability_for_class_slot`, `hero_balance::{base_hp, basic_damage_multiplier, attack_rate_multiplier}`, `shop::{item, recommended_items}`, `ProjectileStyle::for_class`. They are no longer `const fn`. `AbilityDefinition` and `ItemDefinition` are unchanged and still `Copy` (their strings are leaked once per process). The `WARRIOR/MAGE/RANGER/CLERIC/WARDEN_ABILITIES` constants and `shop::ITEMS` are removed; the new `shop::items()` and `ItemId::ALL` replace them. `ItemId::from_id` searches the enum, so wire decoding never touches the catalog. `hero_balance::DEFAULT_MAX_HP` is the literal `220.0`, pinned to `base_hp(Warrior)` by a test.
- The item count is no longer tied to `INVENTORY_CAPACITY`: the late-game preset and the server tests that fill an inventory take at most `INVENTORY_CAPACITY` items, and the practice comment "the cap buys the full inventory" is now a test.
- Python: new `scripts/catalog.py` reads the same JSON. `combat_test.py --hero` choices, `verify_beta_match.py` item costs and offensive slots (a slot is offensive when its ability deals projectile damage) and `capture_combat.py` projectile styles come from it.
- No gameplay or wire change: a migration test compared every catalog field with the old Rust tables bit for bit (`f32::to_bits`) before they were deleted, and the snapshot byte pins and golden JSON tests are unchanged. Tests: shared 78 → 88 (eight catalog tests, `DEFAULT_MAX_HP` against the Warrior base HP, the duel gold cap), server 283 (+3 ignored), client lib 549 and harness 22 + 24 unchanged; Python script tests 86 → 93.

### Client plugin groups and qa feature
- New `client/src/plugins.rs` with four `PluginGroup`s: `NetPlugins` (persistence, networking, match service, career identity), `UiPlugins` (UI kit, mobile controls, mobile UI, frontend, team select, game-state overlay, match HUD, edge HUD, minimap, shop, help, pause menu, social, career, supporter, StoreKit), `GameplayPlugins` (maps, input context, player, combat, and the debug tooling: Combat Test sandbox, practice sandbox, god mode, debug console) and `PresentationPlugins` (camera, scene setup, model scale, combat visuals and feedback, VFX, reactions, team vision, audio, map visuals, the 2D sprite backend and the 3D model backend). `main` is now `DefaultPlugins`, one `PlayerVisualMode::from_environment()` insertion, `.add_plugins((NetPlugins, UiPlugins, GameplayPlugins, PresentationPlugins))` and `QaPlugins`; the tuple-size workarounds are gone. `SpriteVisualsPlugin` no longer reads the environment and only `init_resource`s the mode (the default, `Models3d`, when added alone). The build-time chain `UiKitPlugin` → `MobileControlsPlugin` → `MobileUiPlugin` is kept at the head of `UiPlugins`; the QA plugins build last. Registration inside every plugin is unchanged; because plugins now build in a different order, the multi-threaded executor may run mutually unordered systems in a different relative order than before (no ordering constraint was added or removed).
- New `qa` cargo feature on `client` (`default = ["qa"]`). The QA harnesses moved to `client/src/qa/` (`animation_qa`, `visual_qa` with `beta_ui_qa`, `edge_hud_qa` and `navigation_qa`, `targeting_qa`, `combat_qa`, `map_qa`, `forest_pickup_qa`, `team_vision_qa`, `frontend_qa` with `frontend_qa/avatar.rs` and `frontend_flow_qa`, `social_qa`, `audio_qa`, `offline_qa`, `career_visual_qa`), and `qa/mod.rs` owns `QaPlugins`. The supporter screenshot capture (`OMOBA_SUPPORTER_QA_DIR`) moved from `SupporterPlugin` to `qa/supporter.rs`. The animation audit entry in `main` and the Combat Test panel harness (`sandbox/ui/qa.rs`, kept in place) are `cfg(feature = "qa")`; `OMOBA_QA_SUPPORTER` and `OMOBA_CAREER_QA_OUTPUT` only take effect with the feature. Scripts, `make` targets, packaging and mobile builds are unchanged because the feature is on by default.
- `cargo clippy -p client --lib --no-deps --no-default-features -- -D warnings` runs in CI and as `make check-no-qa` (part of `make check`). Production items only the harnesses read are `#[cfg(feature = "qa")]` and items the harnesses and tests read are `#[cfg(any(test, feature = "qa"))]` (`MinimapQaScene`, `MapPropInstance.role`, `DamageNumber.event_id`, `SocialClient`'s QA hooks, `CareerClient::present_visual_fixture` and a few accessors). No behaviour change and no wire change; client lib tests 549 (529 with `--no-default-features`).

### Client combat and player modules
- `client/src/combat.rs` (3836 lines) is split into `client/src/combat/`: `mod.rs` (`CombatPlugin`, `configure_target_presentation`, re-exports), `cooldown.rs` (`LocalCastCooldown` and its sync systems, `effective_cast_duration`, `local_hero_class`), `feedback.rs` (`ActionFeedback` and its label), `round_reset.rs`, `selection.rs` (`TargetState`, `WorldPointerState`, `TargetCandidates`, pick radii, `select_target_system` and the nearest/screen pickers), `cast.rs` (`PendingCast`, `try_cast_slot`, `queue_cast_request`, the cast and pending-cast systems, `within_cast_range`), `mobile.rs` (mobile cast and utility, assisted targeting), `hotbar.rs` (skill bar UI and upgrade input), `bars.rs` (world HP/mana bars and `CombatVisualAssets`) and `marker.rs` (target ring). `client/src/targeting.rs` moved unchanged to `combat/targeting.rs`; `lib.rs` re-exports it as `crate::targeting`. The 22 inline tests are `combat/tests.rs`, and `target_presentation_tests.rs` moved next to them.
- `client/src/player.rs` (3135 lines) is split into `client/src/player/`: `mod.rs` (`PlayerPlugin`, `DebugSpeedBoost`, the constants, `ground_origin_y`, re-exports), `input.rs` (desktop and mobile movement input, route planning, viewport picking), `motion.rs` (`Jumping`, `move_player`, the sandbox visual clock, jump, gravity, collisions, `hero_movement_multiplier`), `respawn_ui.rs` and `animation.rs` (`register_hero_animation_systems` and the hero animation pipeline); the two inline test modules are `tests.rs` and `animation_tests.rs`.
- Moves only: bodies, comments and test names are unchanged, system registration and ordering are unchanged, and `mod.rs` re-exports every item other modules import, so no file outside the two directories changed its imports. Items, fields and methods shared between the new files or used by the test modules became `pub(super)`; nothing became `pub(crate)`. No behaviour change and no wire change; client lib tests stay at 549 (only `targeting::tests::…` gained the `combat::` prefix).

### Client domain and render gates
- New `client/src/domain/` holds the client-side model types that `net`, gameplay and presentation share: the new `RoundId` (`round.rs`: `server_epoch` + `match_id`; `RoundId::from_meta(&SnapshotMeta)` is `None` when either part is zero) and, moved verbatim, `Team` with its `shared::map::Team` conversions and `PartialEq` bridges and `as_str` (`team.rs`), `CombatStats` with `MAX_HP`/`MAX_MANA` (`stats.rs`), and the markers `Player`, `PlayerBody`, `VerticalVelocity`, `RemotePlayer` plus the movement intents `MovementTarget`/`MovementRoute` (`actors.rs`). The old paths (`crate::team::Team`, `crate::combat::{CombatStats, MAX_HP}`, `crate::player::{Player, PlayerBody, VerticalVelocity, MovementTarget, MovementRoute}`, `crate::net::RemotePlayer`) re-export them, so no caller changed its imports. `Team::ui_color`/`ui_hover_color` stay with the team-select UI in `team.rs`. `CombatPointerInputSet` and `WorldMovementInputSet` moved to `input_context.rs` (re-exported from `combat`). The combat round reset and the mobile-controls round clear keep `Option<RoundId>` instead of a `(u64, u64)` tuple, with the same semantics.
- New run conditions `sprite::in_models3d()` and `sprite::in_sprite2d()` (`resource_exists_and_equals`, so a missing `PlayerVisualMode` runs neither backend instead of panicking). The 2D backend (sprite proxies, `presentation2d`, `world2d`) and the 3D backend (`presentation3d`, Verdant, jungle, minions and creature assets, bosses, decor, the map-visual prop chain and river repair, projectile models, and `world.rs` model sync, VRM double-siding and lighting) now register under them. Every plugin is still added, because 2D resources are read in both modes. The systems keep their internal mode checks. Systems that branch on the mode (camera, snapshot apply, combat bars, targeting, VFX, minimap, team vision, projectile trails, movement, hero animation) and the battlefield mist overlay are not gated. No behaviour change: the gated systems were already no-ops in the other mode. The one difference is resources that go unused: in 2D the client no longer creates the projectile model meshes and materials or an empty boss asset cache. No wire change; client lib tests 543 → 549 (`RoundId`, the two conditions, opposite-mode and missing-mode backend apps).

### Packet handlers
- New `server/src/runtime/handlers/`: one handler per `ClientPacket` variant, grouped by concern (`join.rs`, `movement.rs`, `combat.rs`, `utility.rs`, `shop.rs`, `debug.rs`, `session.rs`), each a `ServerRuntime::handle_<variant>(addr, <fields>, now) -> ControlFlow<()>` holding the former `match` arm verbatim. `Break` replaces the arm's early `return` and skips the post-command tail (endpoint touch, sandbox roster, practice bots, prematch, round start, career registration); `Continue` runs it. `runtime/dispatch.rs::handle_packet_authorized` keeps its ordered pre-checks and is now one `match`: `Leave`, the career-flow `RequestRematch`, `Practice`, `Sandbox` and the paused-sandbox gate are its first arms, in the old order, followed by one line per handler; the `unreachable!` arms for `Leave`, `Practice` and `Sandbox` are gone.
- Explicit imports: `server/src/main.rs` is the module list and `fn main`. The 24 crate-root glob re-exports (`use balance::*`, `pub(crate) use entities::*`, `pub(crate) use sim::{cast::*, …, *}`, …) and the root `use shared::…`/`use std::…` lines that fed them are removed; every module imports what it uses (`use crate::entities::ConnectedPlayer;`, `use shared::wire::GameState;`), and `use crate::*;` is gone (26 sites, plus 20 file-level `use super::*;` in direct children of the crate root). Test modules keep `use super::*;` for their parent module only.
- No behaviour change and no wire-visible change; test counts unchanged (server 282 + 3 ignored, shared 78).

### Server ports
- New `server/src/career_port.rs`: `trait CareerPort` is everything the simulation asks of the account and result store (`enabled`, `new_result_id`, `profile`, `supporter_aura`, `authenticated_session`, `gameplay_principal`, `view`, `forget`, `touch`, `set_playing`, `recovery_confirmed_since`, `handle`, `poll`, `take_match_requests`, `take_cancelled`, `take_social`, `take_settled`, `start`, `checkpoint`, `settle`, `started`, `start_rejected`, `start_error`, `forget_start`, plus the `test_*` hooks under `cfg(test)`). `CareerRuntime.backend` is a `Box<dyn CareerPort>`. `career_backend.rs` is restructured around a `JobLink`: `CareerBackend<L>` is the signed-account state machine (challenges, sessions, replay protection, pending records), `WorkerLink` is the bounded channel pair to the PostgreSQL worker thread (the production `CareerBackend`), and the test-only `MemoryLink` runs the identical logic on the tick thread. `MemoryCareer::test_backend(epoch)` is the old fixture (enabled, nothing acknowledged until a `test_ack_*` hook, jobs kept in `link.jobs`); `MemoryCareer::immediate(epoch)` acknowledges every job at once the way a healthy worker would (login creates a profile, a start is started, a settle is saved, an access refresh keeps the key active); `MemoryCareer::disabled(epoch)` is the guest-only server without a database URL.
- New `server/src/runtime/ports.rs`: `trait Transport { recv, send_to, local_addr }` (non-blocking, `WouldBlock` ends the receive loop) with `UdpTransport(UdpSocket)` and the test-only `MemoryTransport` (shared inbound queue and captured outbound datagrams); `trait Clock { now }` with `SystemClock` and the test-only `ManualClock` (advanced by the test). `ServerRuntime { transport: Box<dyn Transport>, clock: Box<dyn Clock>, .. }` replaces the `socket` field; `prepare_tick`, the receive loop, the passport-admission completions, the sandbox's snapshot and roster clocks and the constructor read `clock.now()` instead of `Instant::now()`; leaf helpers keep taking `now`. `ServerRuntime::with_ports(transport, clock, server_epoch, career, config, map)` is the one constructor; `new_with_map` and the test-only `new(socket, config)` wrap it with the UDP transport, the system clock and the environment's career backend, so existing fixtures are unchanged; `ServerRuntime::for_test(MemoryTransport, ManualClock, MemoryCareer, config)` is the socket-free fixture. Three tests run on it (a practice start, a release-mode roster check, a new whole-runtime session timeout); a new career test pins the immediate acknowledgements. `run()` and the 10 ms loop are unchanged. No behaviour change and no wire-visible change.

### Match rules
- New `server/src/match_rules.rs`: `MatchRules` is the one place that turns the match mode into decisions. `MatchRules::for_mode(mode, team_size)` fixes `team_assignment` (`Balanced` in release, `ClientChoice` in dev, `PracticeSeat` in practice), `start` (`FullRoster` in release, `FirstJoin` otherwise), `prematch_roster` (`Present` in dev, `Full` otherwise), `fills_with_bots` (practice), `debug_commands` (dev and practice), `combat_sandbox_allowed` (dev), `career_credit` (release), `local_results` (practice) and `career_flow` (not practice). `ServerRuntime.rules` replaces `match_config`; the dispatcher, formation, bots, prematch, sandbox and career code read the field they need instead of comparing the mode, and the runtime conditions they combined with (a worker allocation, the sandbox being enabled, a prematch-capable join) stay where they were. `MatchMode`, `MatchConfig` and the env parsers moved from `formation.rs` into the new module unchanged. No behaviour change and no wire-visible change; a unit test pins the per-mode table.

### Hero stats and redaction
- New `server/src/hero_stats.rs`: `StatModifiers` (`ConnectedPlayer.modifiers`) is the one overlay on top of class, level and gear: `damage_mult`, `attack_speed_mult`, `move_speed_mult`, `armor`, `resistance`, `base_max_hp` and the rule flags `god_mode`, `infinite_hp`, `infinite_resource`, `no_cooldowns`, `unlock_all`, `bypass_vision`, `grant_xp`, `respawns`; `Default` is normal play. It replaces `ConnectedPlayer.sandbox` (the stored `ActorConfig`), `sandbox_infinite_hp`, `god_mode` and `speed_mult`, and every `sandbox.is_some()` gate in the simulation (vision bypass, XP skip, respawn skip, cooldown and resource checks) now reads the flag it needs. The effective-stat formulas live there once: `combat_bonuses`, `basic_attack_damage`, `basic_attack_cooldown`, `ability_cooldown`, `skill_recovery`, `move_speed`, `movement_envelope`, `max_hp`, `max_mana`, `mitigate`; `sandbox::effective_*` and the duplicated movement, attack-speed and max-pool arithmetic in `session.rs`, `bots.rs` and `sandbox.rs` are gone.
- `sandbox::SandboxRuntime::apply_actor` converts an `ActorConfig` into modifiers plus the hero's loadout once and keeps the config (by hero id) only to echo it in the telemetry, with the values the hero has owned since (ranks, inventory, the god-mode and speed toggles) read back from the hero; the hand-copies from `apply_skill_upgrade`, the debug toggles and the shop's re-apply on purchase are gone. The practice dummy's flat HP is `base_max_hp`. No sandbox telemetry or owner-view change; a sandbox purchase no longer resets the actor's unspent skill points.
- Wire-visible: `public_view` now blanks the private economy for everyone but the owner. Non-owners, teammates included, receive `gold`, `earned_gold`, `inventory`, `item_bonuses`, `last_purchase`, `basic_attack_request_id` and `utility.last_request_id` at their serde defaults; level, XP, ranks, HP, mana and the cooldown copies stay public. `snapshot::build_players_snapshot(world, recipient, now)` builds the recipient's own entry through `owner_view` and the rest through `public_view`, in the sandbox too. No protocol version bump (every blanked field is `#[serde(default)]`); the client reads another player's economy nowhere (equipment is attached to the local player only, the scoreboard's earned gold comes from the ledger).

### Hero state
- New `server/src/hero.rs`: the authoritative hero lives in server-owned structs. `Hero` holds `identity: HeroIdentity` (id, `is_bot`, team, class, character, avatar, sprite, supporter aura; set at join or reconnect, never by the simulation), position, yaw, HP and mana, `progress: HeroProgress` (XP, level, next-level XP, skill points, ranks), `utility: HeroUtility` (`last_request_id`, `dash_sequence`) and `last_action: HeroAction` (sequence, kind, slot). `HeroEconomy` holds gold, earned gold, inventory, item bonuses, the last purchase receipt, the basic-attack request mark and the former loose `purchase_sequence` and `gold_income_remainder`. `ConnectedPlayer { hero, economy, timers, .. }` replaces `ConnectedPlayer.state`; the wire `PlayerState` is no longer stored anywhere on the server.
- `PlayerState` is built only by the views: `ConnectedPlayer::owner_view` maps `hero` and `economy` onto the wire struct and fills the derived cooldown, utility-clock and shop fields; `public_view` still equals it. `shop_is_available` and `neutral_horizontal_distance_sq_from_anchor` take `&Hero`; `apply_level_up` and `grant_player_xp` take `&mut Hero`; `Hero::new(id, spawn)` is the pre-join placeholder the join packet then fills.
- No wire-visible change: snapshot bytes are identical for every recipient. `server/src/tests/player_view.rs` now compares the view against a hand-written `PlayerState` literal built from `hero` and `economy` plus the hand-computed clocks, so the field mapping is pinned independently of `owner_view`.

### Hero views
- The replicated `PlayerState` is a view: `ConnectedPlayer::owner_view(now, map, phase)` fills the cooldown copies (`basic_attack_cooldown_secs`, `basic_attack_remaining_secs`, `skill_cooldown_remaining_secs`, `skill_recovery_remaining_secs`), the utility clocks (`dash_remaining_secs`, `haste_remaining_secs`, `haste_active_secs`) and `shop_available` from the authoritative timers at the tick's `now`; `public_view` is the redaction hook and equals the owner view for now. `snapshot::build_players_snapshot(world, now)` builds every replicated player through it. The stored `ConnectedPlayer.state` keeps those wire fields at their defaults; the per-tick refreshers (`refresh_basic_attack_cooldowns`, `refresh_skill_cooldowns`, `refresh_utilities`) and every direct write of those fields are gone.
- New `server/src/hero_timers.rs`: `HeroTimers` (`player.timers`: last movement, per-slot casts, last basic strike, dash/haste readiness, haste expiry, respawn) replaces seven loose `ConnectedPlayer` fields; pure reads (`basic_attack_remaining`, `skill_cooldown_remaining`, `skill_recovery_remaining`, `dash_remaining`, `haste_remaining`, `haste_active`) are the gates for dash/haste requests, the cast recovery check and the sandbox telemetry; `normalize_hero_timers` holds the side effects the refreshers used to hide (clearing the basic-strike instant when dead or `no_cooldowns`, the utility cooldowns when `no_cooldowns`, the haste expiry when dead) and runs once per tick after respawns.
- No wire-visible change: snapshot bytes are identical for every recipient, pinned by `server/src/tests/player_view.rs` through join, purchase, skill upgrade, cast, basic attack, dash, haste, death, respawn and sandbox `apply_actor`.

### Server tick
- The server no longer runs a Bevy `App`: `runtime::run` is a fixed-step loop calling `ServerRuntime::prepare_tick` and the new `ServerRuntime::tick(now, dt)` (formerly `simulate_after_mana`, now also covering mana regeneration and the minion projectile pass). `ecs.rs`, `gameplay/`, the `Player`/`Transform3D`/`Health`/`Mana`/`TeamMarker` mirror components, `EcsPlayerEntities`, `EcsMinionEntities`, `DamageEvent` and the sync systems are deleted; `GameWorld` is the only copy of the state.
- One mana regeneration: `sim::regenerate_mana` (joined, living heroes, `dt > 0`, clamped to the pool) replaces the ECS system and the test-only copy in `session.rs`.
- One projectile path: `sim::projectiles::simulate_projectiles_filtered` flies every target kind through one `step_homing` helper and applies minion hits with `apply_minion_damage`; the tick runs the minion-targeted pass where the ECS systems used to run and the rest where `simulate_projectiles` ran before, so timing is unchanged. Minion impacts are now applied in projectile id order like every other kind, instead of hash order.
- No wire-visible change. Test loops that drive `ServerRuntime::tick` now also resolve minion-targeted projectiles (they used to skip the ECS pass); no test expectation changed.

### UI kit
- New `client/src/ui/` (roadmap step 9 pilot, `docs/ui-kit.md`): `UiKitPlugin`, a `UiPlatform` resource read where systems used to call `platform::ui_profile()`, one theme (`ui/theme.rs` merges `ui_theme` and the `frontend::widgets` palette, both kept as shims), one tap recognizer (`Pressable`, `TapTracker`, `recognize_presses`, `GestureEpoch`, `SyntheticPress`) replacing the pause menu's and career's copies, typed button actions (`UiAction<T>` → `Activated<T>` via `add_ui_action::<T>()`), `ButtonStyle` painting and `TestId` (mirrored into `Name`).
- The pause menu and the practice sandbox are rebuilt on the kit: `PauseAction`/`PracticeAction` enums and four handlers replace 19 marker components and 10 per-button systems; every QA-visible `Name` is unchanged. Career buttons use `Pressable` and bump the gesture epoch on modal changes. The audio QA harness presses kit buttons through `SyntheticPress`. Behaviour is unchanged except: career taps share the unified gate (landscape + focus) and get mouse emulation in mobile preview builds; the pause header `×` keeps its tile colour on hover instead of turning primary green after the first hover.

### Client network module
- `client/src/net.rs` (5600 lines) is split into `client/src/net/{mod,components,transport,session,commands,ingest,apply,interpolate,status_ui}.rs` plus a `cfg(test)` `test_fixtures.rs`; the 40 inline tests moved next to the code they cover. `mod.rs` keeps `NetworkingPlugin`, `ClientNetPipeline`, the wire re-exports and re-exports the public types, so `crate::net::X` paths used by the rest of the client are unchanged. Items shared between the submodules gained `pub(in crate::net)`; `offline.rs` and `public_transport.rs` import what they need explicitly instead of `use super::*`. No behavior change; the session-event redesign (roadmap step 8, second half) is still pending.

### Server runtime structure
- `server/src/main.rs` (6000 lines) is split into `runtime/{mod,dispatch,tick}.rs`, `snapshot.rs`, `formation.rs`, `entities.rs`, `ecs.rs`, `game_world.rs` and `sim/{mod,cast,projectiles,minions,neutrals,towers}.rs`; its 50 inline tests moved to `server/src/tests/`. `main.rs` keeps the module list and `fn main`.
- New `GameWorld` groups the entity maps, team buffs, forest pickups, game state, map layout/config, id allocators and the wave clock; `ServerRuntime.world` replaces 15 loose fields. Simulation (`simulate_projectiles/minions/tower_attacks/neutrals`, `spawn_minion_waves_if_due`, `handle_respawns`, regeneration), request handlers (`handle_cast_request`, `handle_basic_attack_request`), vision (`target_visible`, `filter_snapshot`, `sources`) and formation take `&mut GameWorld` with a `TickCtx { now, dt }` instead of 5-12 map parameters; `ensure_player_connected`, `ensure_player_for_join` and `reset_match_with_map` became `GameWorld::{ensure_connected, ensure_player_for_join, reset_round}`. The dispatcher and the tick no longer destructure `self`; the snapshot broadcast is `ServerRuntime::broadcast_snapshots`. No behavior change; the wire format and every test are unchanged.

### Crate hygiene
- New `career-store` crate holds the Postgres career store, its migrations and the queue policy; the game server and the account API link it directly, so the account API no longer depends on the whole game server (Bevy, the Ekza SDK). The server package is binary-only again.
- Remove the orphan `skills` crate (nothing depended on it and its numbers contradicted `shared`), the unused `PlayerAbilitySnapshot`, 64 redundant clippy allows already covered by the workspace policy, and two references to a documentation page that never shipped.

### Shared balance and facing
- Hero speed, debug speed multiplier, mana pool and regeneration, projectile speed, respawn delay, level HP/mana growth, XP thresholds and bot engage range now live once in `shared::hero_balance`; the server, client prediction, offline practice, minimap and harness import them. Offline practice used to regenerate mana at 12/s (server 8/s), fly projectiles at 30 (server 19) and default the first level threshold to 120 (server 90).
- `shared::math` defines the two model facing conventions (heroes look along -Z, minions and neutrals along +Z); every yaw in the server, client and offline simulation goes through it, and the Combat Test actor no longer runs backwards.

### Shared wire protocol

- Move the gameplay UDP/JSON packet types (`ClientPacket`, `ServerPacket`, `PlayerState`, `ProjectileState`, `StructureState`, `MinionState`, `NeutralState`, `GameState` and their enums) into `shared::wire`; server, client and harness use that one definition. Bytes on the wire are unchanged and pinned by golden tests taken from the previous server.
- Replace the server and client `Team`/`Lane` copies with `shared::map::{Team, Lane}` and delete the identity converters; the client keeps `team::Team` and `net::StructureKind` as Bevy components with `From` conversions at the network boundary.
- Drop the client-only `career` field from `Snapshot` (the server never sent it; career data keeps its own datagram).
- Harness: the protocol mirror is gone (it lacked `warden`, `passport_ticket` and the social/career envelopes); bots log the first undecodable packet instead of dropping it silently.
- Add `wire` golden and round-trip tests to `shared`, and the review rule that wire types are never copied.

## [0.23.0-rc.6] - 2026-09-24

- Add the fifth class, Warden (wire id `warden`): a melee jungler with Feral Swipe, Barkskin, Hunter's Mark and Primal Maul, 210 base HP and its own growth, claw projectile style and item order.
- Add the server-authoritative Warden passive Forest Tracker: +35% damage to jungle camps (+15% to bosses), +40% gold and +25% XP per ordinary camp kill.
- Give each class a primary draft role (Warrior Solo, Mage Mid, Ranger Carry, Cleric Support, Warden Jungle); picking a class in the draft proposes its role without overriding a deliberate choice.
- Practice bots fill teams as a five-role composition around human picks; the bot Warden clears camps on its own half before joining mid lane.
- Add a fifth Warden row to the skill icon atlas (original rows unchanged; provenance in `client/assets/ui/skills/PROVENANCE.md`).
- Clients older than this release decode `warden` as Warrior; ship client and server together.
- Keep the Xcode-reformatted project and scheme without a committed team ID (set it in the ignored `mobile/ios/Omoba.local.xcconfig`), and add `make iphone-box` for timestamped unsigned iPhone packages.

## [0.23.0-rc.5] - 2026-09-23

- Add Offline practice from Home: choose a bundled hero/avatar and test 3D movement, class attacks, all four skills and dash/haste without starting or reaching a server.
- Practice starts at level six with recovering targets and mana; it never awards online progress, match history or rating. Leaving restores the saved online endpoint.
- Keep a touch-sized close button and navigation footer visible in Game menu; scroll settings and main actions in short windows and prevent drag gestures from activating Controls guide.
- Validate actual menu layout and touch scrolling at phone/iPad viewport and DPI sizes, plus native offline rendering.

## [0.23.0-rc.4] - 2026-09-23

- Add server-authoritative shared team vision and ten symmetric, walkable brush patches in the 3D battlefield.
- Conceal enemy heroes in brush until an allied observer enters the same patch or the enemy performs an accepted hostile targeted action; the latter reveals them for two seconds within ordinary team sight.
- Filter hidden actors, projectiles, combat events and pickup receipts per recipient, reject fresh hidden-target attacks, and apply visibility to bot, minion and tower targeting.
- Preserve visible killing-hit effects for dead minions, neutrals and structures while still filtering hidden impacts.
- Add soft live battlefield/minimap fog, swaying gameplay grass and a local concealed/revealed indicator, with native desktop/mobile preview verification.
- Document sight distances, already-launched homing behavior and bounded rendering in `docs/team-vision.md`.

## [0.23.0-rc.3] - 2026-09-23

- Add readable magic orbs, orbiting sparks, bounded projectile particle trails and larger confirmed-hit bursts; replace the Warrior's segmented yellow projectile with a steel blade.
- Add six shared healing butterfly flocks: server-authoritative 5% maximum-HP collection, single contested winner, 30-second respawn and replicated availability.
- Add a noninteractive soft oval battlefield vignette, with glowing animated butterfly presentation in both 3D and Sprite2d.
- Add native pickup lifecycle captures and document rendering budgets and gameplay rules in `docs/forest-combat-vfx.md`.

## [0.23.0-rc.2] - 2026-09-23

- Rebalance starting health and Q cadence across all four classes, add bounded level-based movement, basic damage/rate and skill growth, and prevent same-tick cross-slot skill volleys.
- Reconcile authoritative skill cooldowns on the client, buffer the next skill through recovery, and match player/bot movement to the shared progression curve.
- Preserve early tower safety with a built-in hero-target damage multiplier while retaining minion siege cadence and custom-map defaults.
- Add repeatable before/after combat matrices, finite-mana/sustain samples, objective safety measurements and research-backed tuning notes in `docs/balance-tuning.md`.

## [0.23.0-rc.1] - 2026-09-23

- Add an opt-in local Combat Test launcher and unified developer panel with direct hero selection, actor progression/stat/equipment controls and reset/teleport.
- Add authoritative training dummy measurements, configurable enemy AI, direct two-client duels, reusable test presets, real minion controls and simulation pause/speed/frame stepping.
- Add actual animation graph inspection and combat geometry/state overlays while keeping network clocks live. Sandbox commands are isolated from practice, release and rated matches.
- See `docs/combat-test.md` for launch commands, controls, damage semantics and developer extension instructions.

## [Unreleased]

### Quality gate
- Add GitHub Actions CI (format, clippy with warnings as errors, workspace tests, the headless harness, Python tooling tests), pin the toolchain in `rust-toolchain.toml`, and add `make check` / `fmt` / `lint` / `test` / `test-scripts`.
- Fix the clippy findings the pinned toolchain reports and let the asset-gate script test skip when its historical fixture is absent.

### Offline practice
- Offline practice heroes now face the way they run (their yaw pointed the model backwards), and the round keeps a live scoreboard so K/D/A counts kills and deaths.
- The pause menu's "Practice sandbox" page works offline too: god mode, target dummies, clearing or restoring the circling heroes, and a 1v1 opponent of your class that walks mid, strikes and casts its unlocked kit at the chosen level with the chosen gold spent on items. Dying offline respawns you at base after five seconds.

### Practice sandbox
- The pause menu gains a "Practice sandbox" page in local bot practice: god mode for yourself, a stationary target dummy in front of you (up to four, respawning on their spot), "Clear all bots", "Standard bots", and a 1v1 opponent on mid with a chosen level (skills ranked for that level) and gold (spent on items at base).
- Practice bots now buy their class's recommended items whenever they stand in their base shop, rank every unlocked skill, and cast their whole hostile-target kit instead of only Q.
- God mode and speed boost are accepted in practice matches (already unrated), not only in development mode.
- Round statistics register anyone who joins after the roster was drafted before the first hit, and the server logs a hero hit whose participant has no scoreboard row instead of dropping it silently.

### Practice bots, utility effects and targeting polish
- Practice bots now face the way they run and the unit they strike; the server used the +Z yaw convention while hero models face -Z, so every bot ran backwards.
- Dash shows a departure ring, staggered cyan afterimages along the travelled path and an arrival flash with sparks, for the local hero and for remote heroes and bots, which now teleport instead of sliding to the destination.
- Haste leaves an amber double speed trail behind the hero and pulses under it while the buff lasts, so the speed boost is readable even standing still.
- Settings gain a Camera > Distance row (55%–225%, 10% steps) that is remembered across restarts and stays in sync with wheel zoom; Reset graphics restores it.
- A hero in melee reach turns to face its target when it strikes instead of swinging with its back to the enemy; the stick and an explicit move order keep facing authority.
- On phones, dragging ATTACK onto a unit only locks it as the target; approaching and striking still require tapping or holding ATTACK.
- Refresh the staged Rust executable timestamp so incremental Xcode device builds re-sign the current executable.

## [0.22.0-rc.5] - 2026-09-23

- Keep avatar collection controls stable during account/catalogue updates and expose wallet approval status and retry actions.
- Open Ekza account and wallet approval pages through the native iOS browser bridge.
- Add explicit Debug-only local Studio configuration and a LAN rehearsal launcher using the existing isolated test database.
- Pin the Ekza SDK fix for opt-in private-host HTTP development; release SDK builds retain HTTPS requirements.

## [0.22.0-rc.4] - 2026-09-23

- Add server-authoritative own-base healing at 12% maximum HP per second inside the shop zone.
- Repair mobile settings/results drag scrolling, preserve nonshrinking result cards, and keep settings Back outside the scroll body.
- Make mobile career actions activate on short release, cancel them on a drag, and verify live scoreboard delivery through UDP and client ingest.

## [0.22.0-rc.3] - 2026-09-23

- Align practice bot routes with authoritative base footprints so every lane advances after spawn and respawn.
- Align local structure movement with server sweeps and retain route clearance while sliding around buildings to prevent prediction drift.

## [0.22.0-rc.2] - 2026-09-22

- Add a versioned Xcode iOS application project and shared Archive scheme for the existing Rust game, including bundled assets, icons, privacy manifest and matching debug symbols.
- Keep signing and TestFlight distribution in Xcode; provide ignored local team/build/server settings without storing personal credentials.

## [0.22.0-rc.1] - 2026-09-22

### Public multiplayer MVP
- Add an authenticated lobby with quick bot fallback, humans-only waiting and solo bot matches, backed by a bounded pool of isolated arena processes.
- Freeze each allocated roster, wait for team draft/loading and durable start, preserve returning participants, and reject fresh mid-match joins.
- Save approved bot-match history and progression (50/25 win/loss XP) without competitive rating; keep eligible full-human rating authoritative and settlement idempotent.
- Add return-path UDP admission, signed replay-resistant gameplay commands, bounded traffic work and private worker manifests/outboxes.
- Preserve lobby preferences during worker handoff/reconnect, allow leaving draft during outages, refresh saved results on the result screen, and package public service launch instructions.
- Add real PostgreSQL, public-protocol lifecycle and 100-client capacity probes. Public hosting and platform distribution remain operator release gates.

### Persistent Ekza account connection
- Restore a connected Ekza library after restart with the SDK's private, atomic,
  backend/project-scoped credential file and fresh Registry validation.
- Background refresh retains sessions through outages and clears revoked sessions.
  Hero selection and Collection expose local sign out and retain catalogue/scroll updates.
- Pin Ekza Bevy SDK 0.7.0 at `42c39e4`; preserve dynamic catalogue entitlement changes.

## [0.21.0-rc.5] - 2026-09-22

### Shared run animation and VRM humanoid foundation
- All 15 shipped 3D heroes use a real shared sprint motion during normal local or remote movement. Walking remains a separate motion reserved for future debuffs; existing 2D Run sequences remain unchanged.
- VRM0/VRM1 humanoid metadata drives runtime retargeting from one engine-owned motion library. Per-instance skin-index binding supports validated clipless models without baking animation into each skin or changing approved model bytes.
- Added a bounded offline VRM compatibility validator and explicit unsupported-rig diagnostics. Existing Studio profile, integrity and ownership admission checks remain intact; external clipless profile rollout is documented separately.
- Added all-roster motion audits, malformed-rig/instance lifecycle regressions and a labeled native capture harness.

## [0.21.0-rc.4] - 2026-09-22

### Coordinated team draft and avatar presentation
- Mobile attack now sits at the exact center of a shared circle: four skills, minion targeting and tower targeting use one radius, with separate dash/haste controls. Desktop keeps its centered bottom skill bar. Compact phones retain safe touch areas and a smaller upper-left map.
- Find match assigns the team and starting side automatically. A shared draft shows teammates, avatars, classes, intended roles and lock status, with composition warnings. Teammates see accepted changes before locking in.
- A common countdown leads to a shared loading roster. Gameplay waits for required clients' actual map/avatar readiness and the existing durable match-start acknowledgment. Dropout, reconnect and loading timeout preserve an explicit retry path.
- Collection previews open facing the viewer, stationary, and rotate with the drag. Validated SDK downloads replace preview and match fallback models when ready.
- Default avatars precede a persistent Ekza Studio/library section with asynchronous refresh, thumbnails and truthful loading, empty, cached and unavailable states. Temporary catalogue failures retain the cached roster; existing authorization and model validation remain in place.
- New protocol fields are opt-in for legacy clients and harnesses. Physical-device testing and the previously observed renderer-teardown issue remain release follow-ups.

## [0.21.0-rc.3] - 2026-09-22

### Compact mobile MOBA controls and live match information
- Both profiles place the minimap at upper left with money and two recommended quick purchases underneath. Money opens the shop; purchases retain server confirmation and base/inventory rules. Equipment is inspectable in the shop.
- Mobile keeps movement at lower left and groups attack, minion/structure targeting, four skill circles and utilities at lower right. Skill rank rings and a single full-size upgrade mode replace detached upgrade buttons. The center and lower center stay open; desktop keeps its center-bottom skill cards.
- A compact top strip displays team kills and personal K/D/A beside chat, reactions and menu icons. The score opens both teams' live scoreboard with identity, K/D/A, level and earned gold. Income excludes the starting wallet grant and never decreases when shopping.
- Selected targets have a compact top-center health bar with exact HP, supporting heroes, minions, structures and neutrals. Local health/mana/level remain in the left edge module; detailed objective guidance is available in the scoreboard and controls guide.
- Mobile dash travels up to 5 units with a 20-second cooldown; haste grants 40% movement speed for 3 seconds with a 25-second cooldown. Both are server-authoritative, bound to the current round, and validated against cooldown/life state. Dash respects obstacles and explicitly reconciles even when blocked short of its full distance.
- Native QA covers the revised resting layout, category controls, rank mode, target health, live purchase receipts and scoreboard states. Phone captures are development previews; physical-device verification remains a release gate.

## [0.21.0-rc.2] - 2026-09-21

### Art-directed interface and clear combat sightlines
- A compact desktop tactical dock groups the minimap, hero resources, abilities, and
  equipment along the lower edge. Match and target information sits beside that dock;
  the upper-middle battlefield stays visible when opponents approach from the north.
- Landscape phone HUD groups status beside the minimap and keeps the middle approach
  clear while retaining the movement stick, attack fan, upgrades, chat, and shop.
- Verdant menus share a restrained palette, consistent borders, typography, spacing,
  and clear primary actions. Home gives the selected hero a composed showcase and
  groups player identity and matchmaking in a single framed layout.
- Phone shell controls retain full touch targets and readable type instead of shrinking
  with the desktop composition. Modals use their actual phone layout scale.
- Help is shorter and easier to scan. Game-menu controls and settings rows align;
  career navigation highlights the current tab and shares menu hover feedback.
  Settings remain usable while the menu is offline, and phone utility controls
  yield to the shop so its Close action remains exposed. Explicit Help requests
  also work before joining; closing the guide restores the fitted phone home.
- Native screenshot checks now measure the protected north sightline, HUD text fit,
  and panel overlap, and include essential frontend actions, game-menu, server form, and scrolled
  settings captures. Longer class-specific ability labels have their own reserved space.


### Phone attack button finds and chases a target
- With nothing locked, the attack button now looks beyond its own reach (at least 12
  units, attack range + 6 for ranged heroes) instead of only inside the attack range, so
  a Warrior finds the enemy standing a few steps away.
- It always takes an enemy hero when one is in reach, then the nearest minion or jungle
  camp, then a structure. Previously heroes only had a distance bonus, so a minion at
  your feet could win over a hero nearby.
- On a phone the hero now walks up to an out-of-range target and attacks when it
  arrives, as on desktop. It used to stop with "Target out of attack range". Moving the
  stick still takes over at once and drops the walk.

### New app icon
- The iOS app icon is the new Omoba logo. The source art ships with pre-rounded corners
  on black; the icon is a full-bleed 1024x1024 square with the panel's own edge colours
  extended into the corners, so the iOS mask never shows black edges.

## [0.21.0-rc.1] - 2026-09-21

### Front end: home screen, avatar collection and a hero pick before the match
- The client now opens on a home screen instead of dropping the player into the live
  map with a picker floating over it. Screens are an explicit state machine
  (`Home -> HeroSelect -> Searching -> Loading -> InMatch -> PostMatch`), and every
  menu screen blocks gameplay input through the existing input context.
- PLAY opens hero select. The join packet - which is also the matchmaking queue entry -
  is only sent when the player locks in a class, an avatar and a side, so the map now
  appears after the hero is chosen instead of before.
- Matchmaking has its own screen: the server's queue view, the formation counters, the
  chosen hero and a cancel that returns to the home screen. A match found leads to a
  loading screen and only then into the world.
- Avatar collection with a live 3D preview: every shipped, owned and community avatar
  can be inspected on its own render layer, turned by dragging, and played through any
  animation clip the model actually declares. The preview never touches the match
  world. An avatar can be put on the profile card or selected for the next match.
- Profile card: nickname, level, rating and W/L from the career profile, plus a chosen
  main class, showcase avatar, accent colour and a title unlocked by wins. The card is
  stored locally in `profile_card.json` next to the client preferences.
- Match history and friends open from the home screen through the existing career
  modals; the result screen after a match offers "Play again" and "Back to menu", the
  latter leaving the session cleanly instead of holding the seat for a reconnect.
- `OMOBA_FRONTEND_QA_OUTPUT` captures the shell screen by screen and fails the run if
  a screen leaves a 1280x720 viewport; adding `OMOBA_FRONTEND_QA_FLOW=1` instead presses
  the real buttons against a live server and records the screen sequence it produced.
  Headless evidence runs (`OMOBA_AUTOJOIN`) and the screenshot harnesses bypass the
  shell and still reach a match directly.
- Beta polish pass on the shell: the home screen shows the card's hero in 3D next to a
  right-hand action rail and the build version; hero select gains a live panel with the
  chosen avatar in 3D, the class line and that class's Q/W/E/R kit, and its wallet and
  account controls collapse into one strip under the grids; the connection panel and the
  in-match career bar keep out of the menus, which print their own status line. Layouts
  are checked at 1280x720 and 1024x640 and hold at both.
- Front-end failure paths, found in review and fixed:
  - New `Leave` client packet. The server releases the seat or the queue entry at once
    and keeps nothing for a session reclaim, while the endpoint stays connected. "Back to
    menu" and the search screen's Cancel both use it, so leaving works the same on a
    practice, dev or ranked server, and the same client can lock in a different hero
    straight away. Previously leaving only swapped the local transport: the next join was
    refused as `SessionActive` or handed back the old seat with the old hero.
  - A rejected or exhausted join now returns to a working picker that says why. The dead
    join is dropped, so the lock-in works again, and the reason is shown in the picker
    header until the next attempt.
  - A transport teardown in the middle of a match no longer throws the player onto the
    home screen: the shell tells a reconnect from a leave by the committed join, not by a
    flag the teardown clears.
  - A teardown with nothing committed no longer pulls the player out of the home, card or
    collection screen into hero select. Instead the menus retry the connection themselves
    every five seconds.
  - The lock-in is ordered before the network send, so the search screen never opens on a
    join that is not committed yet.
  - The 3D preview tags its model onto the preview layer until the scene reports ready,
    however slow the load, lives far below the arena, and is released when no screen
    shows it, so a preview avatar can never stand in a match. A model without animation
    clips now says so instead of loading forever.
  - A showcase avatar from the store or the community list survives a restart: the card
    no longer drops a slug the catalogue simply has not delivered yet.
  - The collection grid scrolls (wheel, Page Up/Down); avatars below the fold were
    unreachable.
  - Smaller: the accent swatch follows the choice, every `*_QA_OUTPUT` world harness
    bypasses the shell (decided once, not per frame), and the home footer no longer
    advertises an F1 overlay that only exists in a match.
- Phone and tablet: the menus scale to the window height (an iPhone in landscape is about
  400 logical pixels tall; the screens are laid out for 640) and return to full size for
  the match, whose HUD and touch controls are laid out for the real screen. The phone
  picker keeps its own tuned layout; its Back button sits under the class column, and the
  desktop-only parts (3D side panel, wallet strip) stay hidden there. The phone bar
  (?, MENU, SERVER) now shows on the home screen and the picker instead of hiding under
  them, so a phone can reach settings and the server address from the menus. Checked
  with the touch UI at 874x402.
- Fix eight pre-existing clippy findings so `cargo clippy --workspace --all-targets -D
  warnings` passes again under clippy 1.93 (`account-api`, `server/passport_admission`,
  `client/career_devices`, `client/map_visuals/river`). Behaviour is unchanged.

### Ekza account: your own library in the picker, no wallet
- "Connect Ekza account" in the avatar picker: the game shows a short code and opens
  Ekza Studio, the player signs in (email, later Google) and confirms, and the picker
  gains a "My Ekza library" group with the free avatars that account saved or created
  and that are approved for Omoba. The library is re-read every 15 seconds while
  connected, so an avatar saved in the browser appears without restarting the game.
- The connection grants nothing. The server still admits a free avatar from its own
  registry read, so a connected account and a guest can wear exactly the same avatars;
  the account only decides what the picker lists first.
- Update `ekza-bevy-sdk` to 0.6.0 (`account` module). Wallet pairing is unchanged.

### Ekza community avatars: free, no wallet
- Avatars a creator published through Ekza Studio, prepared for Omoba and approved by
  an Omoba project owner now reach the game with no wallet and no ticket. The picker
  shows them as "Ekza community avatars"; the model installs on first use through the
  same verified SDK store as a purchased avatar.
- The server decides from its own read of the registry's unified catalogue
  (`/v2/avatars`, `access: "free"`), off the game tick. A client can never declare an
  avatar free: an owned template, an unknown slug or a forged entry is still refused.
  A known avatar is re-checked after five minutes, so a withdrawn approval stops
  working; an unknown slug may trigger a new read at most every ten seconds; a registry
  outage keeps what was already approved.
- Update `ekza-bevy-sdk` to 0.5.0: unified catalogue, `ekza:avatar:<uuid>` identities,
  `StoreAvatar::free`. Purchased avatars and their tickets are unchanged.

### Ekza avatars: the game owns its rendition builder
- Add `scripts/ekza_build_rendition.py`, the builder behind the Ekza rendition profile
  `desktop / humanoid-glb-v1`: a VRM goes in, a GLB with the five retargeted clips
  comes out, named by its SHA-256. The Ekza registry runs it as an external command,
  so Omoba's requirements and the code that satisfies them stay in this repository.
  One JSON document on stdout; exit 0 on success, 2 with typed issues when the avatar
  cannot become a rendition. No network, standard library only, idempotent.

### Developer workflow
- `scripts/ekza_demo.py serve` also starts Avatar Studio from `../ekza-registry/web`
  when it is built, points the storefront's old `/studio` links at it, and enables
  the registry's Studio API when Supabase settings are present in the environment.

### Repository hygiene
- Make this checkout the single place every platform is built from. The separate
  local copies used for the iPhone, mobile beta and passport work were retired;
  their task evidence and the first physical iPhone build log were moved here.
- Stop versioning `.agent/` task evidence (248 log and report files that predated
  the ignore rule). The files stay on disk; only the index entries are removed.
- Extend `.gitignore` with signing material, Xcode/Gradle/IDE state and packaged
  artifacts so platform tooling cannot leak them outside `/target` and `/builds`.

### Ekza avatar loop, demo ready
- The demo runner uses the registry from its own `ekza-registry` repository
  (falls back to `ekza-mirror/backend`).
- "Connect Ekza wallet" button in the avatar picker: pairing runs through the
  SDK `PairingFlow` without blocking the game, the approval page opens in the
  browser, the code and link stay on screen, and the picker rebuilds with the
  purchased avatars once the wallet is approved. The `OMOBA_PASSPORT_CONNECT=1`
  terminal flow still works.
- `scripts/ekza_publish.py`: operator tool that turns an on-chain avatar
  template into a registry catalogue entry: reads the minter program, verifies
  the source VRM, bakes the five Omoba clips into a `desktop/humanoid-glb-v1`
  rendition, writes content-addressed assets and an explicit approval record.
  It never signs, spends or deploys.
- `scripts/ekza_demo.py`: local demo of the whole loop on devnet
  (`publish`, `serve` = registry + storefront/passport + game server, `client`).

### Ekza avatars through the SDK
- Purchased Ekza avatars now reach a running game through `ekza-bevy-sdk` 0.4.0.
  The client reads the public registry catalogue in the background, lists the
  store avatars the paired wallet owns, installs a model on first use into the
  private settings directory (`ekza://` asset source) and only after the SDK
  size/SHA-256/GLB checks and Omoba's humanoid-clip check pass.
- The avatar picker has two groups: "Default avatars" (the 15 shipped CC0
  skins, free for everyone, unchanged) and "Your Ekza avatars" (not in the box:
  bought on Ekza and delivered through the SDK), with an explanatory hint when
  the group is empty.
- Another player's store avatar is downloaded and verified on demand; the
  legacy model stands in until it is installed, instead of rendering nothing.
- The server admits an `ekza-<sha256>` slug from the consumed passport ticket
  alone: the slug is the hash of the granted identity and rendition, so no
  pre-synced manifest or asset copy is required. Forged, mismatched, replayed
  and ticketless joins stay denied.
- `omoba-passport` no longer carries its own passport HTTP transport; it wraps
  the SDK client and keeps the Omoba-specific checks and the staged importer.
  The passport origin defaults to the public Ekza storefront
  (`OMOBA_PASSPORT_URL` / `EKZA_PASSPORT_URL` override it); the registry origin
  is overridable with `OMOBA_REGISTRY_URL`.

### Build reproducibility
- Repin the SDK to its content-identical rewritten Passport revision after Git
  attribution cleanup, preserving the locked dependency graph and platform features.

### Visual effects
- Add animated forest butterflies and pooled confirmed-hit particles: melee slash
  arcs and sparks, expanding magic rings and motes, and ranged impact flashes.
  Reuse skin impact colors and fixed budgets in both 3D and sprite rendering.
- Upgrade live Supporter auras with three inclined electron-like orbits, bright
  cores, soft camera-facing glow sprites, tapered light trails and rising glints. Keep a fixed
  35-element budget per hero, shared materials, and hidden/dead actor suppression
  in both 3D and sprite rendering. No combat or entitlement changes.

### Documentation
- Reframe the README and mission around a collaborative open world of avatars;
  add verified Ekza Space/SDK links, a developer integration path and concrete
  entry points for artists, modelers, animators, players and contributors.
- Refresh native practice, phone/PC connection and iPhone/TestFlight instructions;
  distinguish source-beta features from public distribution and provider readiness.
- Normalize mission and licensing-guide Markdown paragraphs/list spacing; preserve
  license terms and remove an accidental editing suffix from the mission.

### Developer workflow
- Make bare `make` show `make help`; derive the command list from target comments
  without starting game processes or checking the iPhone toolchain.
- Prepare Xcode archives from existing signed physical iPhone apps for later
  TestFlight distribution; include an original beta icon, required-reason API
  manifest, explicit build numbers and an Apple account/signing handoff guide.
- Add an enrolled-iPhone install/launch helper and a home practice-server launcher;
  prepared kits keep signed app/IPA and the native Mac server outside compiler caches.
- Consolidate physical iPhone build/signing tools and tests in the main repository;
  add `make iphone-check` / `make iphone` and separate retained `builds/` packages
  from disposable Cargo caches. Refresh the reviewed prop asset inventory.
- Add `make play` / `make play-bots` to build locked current sources and launch
  supervised local 3D practice with nine bots, with owned-process cleanup.

## [0.20.0-rc.5] - 2026-09-16

### Added
- Shared native-account enrollment with explicit portal approval, one-time recovery
  codes, device revocation and preserved local identity backups.
- Server-authorized Solar, Lunar and Verdant Supporter auras, isolated native
  preview, account status and cosmetic selection without gameplay advantages.
- PostgreSQL provider events and expiring payment periods with replay, ownership,
  cancellation, refund and reconciliation handling.
- Account-bound Solana Pay quotes, finalized USDC transfer validation and recent
  orders that remain recoverable after a browser reload.
- Native StoreKit purchase/restore bridge compiled by the regular iOS Cargo build;
  unfinished purchases await committed server verification.

### Changed
- Career and portal migrations advance to version 3. Runtime roles gain narrowly
  scoped device/cosmetic permissions; revoked game keys cannot be reused.
- Payment providers remain unavailable without explicit verified configuration.
  Live Apple/Solana purchases, App Store products and deployment are operator
  rollout steps, separate from local synthetic payment verification.

## [0.20.0-rc.4] - 2026-09-16

### Fixed
- Reduce practice-bot stop/turn jitter with stable targets, immediate route
  continuation and bounded collision-aware hero separation. Smooth remote
  players with a short snapshot buffer, including reconnect/teleport resets.
- Anchor BOT names above normalized models using current rendered hero/camera
  poses, independent of animation bounds and model yaw.
- Replace overlapping Verdant river planes with a continuous joined surface
  below roads; close bank seams and clip water cleanly at map corners.
- Keep the phone chat input, Send and Close in the top safe-area row; preserve
  drafts when hiding the keyboard and accept the native iOS Return key.

### Added
- Original class-specific artwork for all sixteen abilities on mobile and desktop,
  with readable availability/cooldown overlays and asset provenance.
- Mobile stationary skill inspection on a 450 ms hold. Inspection never casts
  on release; taps and deliberate directional drags retain their combat behavior.

### Changed
- Mobile basic targeting follows a forward aim ray independently of thumb reach.
  The visible handle stays short; actual attack range and server validation remain
  unchanged. Release commits only the same eligible target shown in the preview.
- iPhone builds retain debug symbols; TestFlight archives require a matching dSYM
  and validate its UUID against the packaged game executable before archiving.

## [0.20.0-rc.3] - 2026-09-15

### Fixed
- Update target selection visuals after interpolation and grounding using current
  target/camera poses before UI layout, eliminating mixed-frame screen projection.
- Replace the rotating, bobbing bounding-box marker with a stable terrain-anchored
  ring in 3D and 2D; animation bounds and model rotation no longer move it.
- Hide both indicators immediately for invalid, dead or despawned targets.

## [0.20.0-rc.2] - 2026-09-15

### Fixed
- Keep native friend/profile input widgets alive while typing, preserving scroll,
  focus and IME state on desktop and mobile layouts.

### Added
- Editable public `nickname#1234` addresses in the game and player portal, random
  initial names and tags, exact signed friend lookup and collision feedback.
- Atomic career v2 migration preserving internal account IDs, friendships, ratings
  and historical match receipts; Unicode case-insensitive uniqueness requires ICU.

## [0.20.0-rc.1] - 2026-09-15

### Added
- Rust Account API and separate portal schema for game-confirmed browser sessions,
  private match reports/history, statistics projection, friends and preferences.
- Native desktop/mobile profile action to confirm a website code with the existing
  game identity; HTTPS worker is bounded and approval remains explicit.
- Atomic browser operation receipts and friendship version preconditions, preserving
  recreated relationships when an old removal request is retried.
- OpenAPI contract, PostgreSQL role grants, local deployment/restore guidance and
  real database, browser and load-test support for the standalone Next.js portal.

### Changed
- Enable Bevy std for standalone server builds, using OS sleep instead of the
  no-std spin fallback that consumed a full core between ticks.
- Game database migrations run explicitly with `migrate-career` or Account API
  `migrate`; game and API runtime startup only verify the existing schema version.
- Authoritative career/matchmaking persistence is shared through the server library.

## [0.19.0-rc.8] - 2026-09-14

### Added
- Bundled CC0 instrumental music loop, distinct combat-style impacts, local
  hero/match notifications and interface sounds; exact attribution and asset
  provenance accompany the compact Ogg Vorbis pack.
- Local audio presentation driven by accepted server combat and match state,
  with round/reconnect baselines, distance attenuation, bounded voices and rate
  limits. Missing sounds are dropped without delaying gameplay or replaying later.
- Smooth music gain changes, focus/mute handling and separate persistent master,
  music, effects and interface settings on desktop and mobile. Preferences migrate
  without changing older graphics or player selections.
- Versioned cue manifest using safe packaged paths, an original sound-synthesis
  builder and native audio/settings diagnostics.

### Scope
- Audio uses the existing Bevy/Vorbis stack; no new production dependency or
  network protocol is introduced. Native phone-sized previews do not establish
  physical Android/iOS output or browser autoplay acceptance.

## [0.19.0-rc.7] - 2026-09-14

### Added
- Explicit native bot practice: immediate solo start, labelled bot heroes, late
  human replacement, safe reconnect and bounded round rollover. Bots follow map
  navigation and use authoritative attacks, skills, cooldowns and damage.
- `make practice` and `make practice-server`; packaged practice uses native
  server bots. Existing `make play` retains its legacy local-release workflow.
- Opt-in match/team chat and short-lived picture reactions through a separately
  framed social stream. Server-owned identity, audience filtering, deduplication,
  signed profile requests and shared rate limits protect the message boundary.
- Desktop/mobile chat, local mute controls and a reaction wheel opened by holding
  the local hero, a visible button or T. The UI cancels gameplay orders and blocks
  closing-frame click-through. Four original starter reaction pictures are bundled.
- Versioned reaction catalog and local presentation manifest, with a fail-closed
  entitlement boundary for future approved cosmetic/NFT packs.

### Compatibility and scope
- Human result JSON remains compatible with rc.6 immutable receipts. Rated storage
  rejects bots; an existing participant cannot change its human/bot identity.
- Practice results are explicitly local and unranked, including with PostgreSQL
  configured. No permanent career XP/MMR/history credit is claimed for bot practice.
- Runtime reaction access currently includes the free starter pack only. Generic
  NFT ownership verification, minting/marketplace, party queue and voice remain
  future work. Physical-device and global-load validation remain separate checks.

## [0.19.0-rc.6] - 2026-09-14

### Added
- Server-owned match totals and persistent PostgreSQL careers: historical nicknames,
  heroes/skins, outcome, K/D/A, damage by target category, last hits, levels,
  progression and outcome-based rating. Atomic immutable settlement prevents double credit.
- Device-key profile authentication and signed account operations, with explicit
  guest/unranked compatibility when career storage is not configured.
- Separate desktop/mobile result, history, profile and friends panels. Persistent
  friend requests, accept/reject/cancel/remove, friend profiles and presence.
- Licensed bundled CJK font fallback for Chinese/Japanese/Korean names, with
  existing Latin/Cyrillic typography preserved.
- Authenticated release queues that use saved rating and newcomer experience;
  a durable account allocation gates match start and prevents simultaneous seats.
- Bounded background PostgreSQL work, durable local result outbox, owner leases,
  expired-owner recovery, and explicit play-again after viewing results.

### Fixed
- Preserve actual accepted round totals through disconnect and cosmetic event expiry.
- Freeze the first terminal winner before further same-tick projectile/minion impacts.

### Release scope
- PostgreSQL configuration and deployment remain operator tasks. This beta has one
  arena/queue per process; global routing, party invitations, chat, account recovery
  UI and replays are not implemented. Device keys currently identify installations.

## [0.19.0-rc.5] - 2026-09-12

### Added
- Validated map-object profiles with stable IDs, configurable lane tower count,
  placement, HP and attack parameters, loaded once at server startup.
- Ordered lane siege tiers, configured rematch reconstruction and replicated
  map/structure presentation identity.
- Reusable prop archetypes, per-instance visual overrides and a shared palette
  for live tower/base models and 2D sprites, with asset/collision fallbacks.
- Contributor map-tuning documentation and native/UDP verification artifacts.

### Changed
- Share Verdant layout coordinates between client and server and reject a
  mismatched map geometry instead of discarding static collision.
- Preserve the existing eight-structure layout and gameplay numbers by default.
- Clip player movement against live structures and measure minion siege reach
  on the ground plane so valid lateral tower adjustments remain attackable.

## [0.19.0-rc.4] - 2026-09-12

### Added
- Class-specific arrow, arcane, holy and crescent projectiles, with a versioned
  local cosmetic registry for class/action and avatar/sprite overrides, packaged
  models/sprite animations, trails, impacts and avatar clip aliases.
- Floating damage numbers and impact bursts sourced from bounded, deduplicated
  server damage receipts with actual HP removed and typed actor identities.
- Two melee fighters and one ranged caster per existing three-minion wave.
  Casters launch traveling projectiles; both roles have distinct silhouettes
  and sequence-driven attack poses. Minions are visually 50% taller.

### Fixed
- Mark the unfinished Orchard 2D animation pack as art pending, disable new
  selection, and use an explicit render fallback for legacy identities. Retain
  its stable ID/portrait without requesting absent files or claiming finished art.

### Compatibility
- Gameplay stays authoritative. Cosmetic configuration cannot change damage,
  reach or projectile travel. Additive protocol-2 fields keep legacy defaults;
  rebuild both peers to see the full presentation and mixed-minion behavior.

## [0.19.0-rc.3] - 2026-09-12

### Added
- Standard reciprocal open-source licenses: AGPL-3.0-only server and MPL-2.0
  client/shared source, with separate content and third-party scope.
- Open Moba mission, contributor rights, brand and exact-source distribution
  guidance. Packagers include the project license texts and notices.

## [0.19.0-rc.2] - 2026-09-11

### Added
- Six point-symmetric ordinary jungle camps, three types on each side, using
  shared reachable positions. Anyone can farm them for last-hit gold and XP;
  ordinary monsters return with full HP 40 seconds after death. The living
  last hitter also recovers 20% maximum HP, capped at their maximum.
- Distinct original procedural jungle creatures with grounded models and
  persistent living/depleted minimap camp markers on desktop and phone.

### Fixed
- Reset stranded jungle monsters when their target dies, disconnects or leaves
  the leash. Ordinary chase respects static forest obstacles.

## [0.19.0-rc.1] - 2026-09-11

### Added
- Pair the native client to a browser wallet through the Ekza passport device
  flow. Keep the free roster and show only owned, explicitly approved Omoba
  renditions among protected choices. Tokens remain in process memory.
- Stage purchased GLBs through a bounded SHA-256 importer and versioned public
  roster; client and server restart against the same manifest. Preserve
  canonical template identity separately from the buyer's NFT mint.
- Consume a project/session-bound one-use ticket before authoritative paid
  admission, asynchronously and with bounded concurrency. Reject forged slugs,
  missing or rejected proofs, mismatched renditions and paid-session downgrade
  bypasses. Replicate the exact protected slug through the normal game path.

### Verification
- Native build, SDK/parser/importer tests and real UDP admission with a local
  passport fixture. Live devnet and renderer results are recorded separately in
  [the integration note](docs/progress/2026-09-11-avatar-passport-roundtrip.md).

## [0.18.0-rc.10] - 2026-09-11

### Fixed
- Remove the dead-end corner detour from side-lane minion routes. Dragon/bottom
  waves now leave the green base directly toward their lane; the mirrored top
  route is corrected for blue, including the opposite far-base approaches.
  Preserve road artwork, tower locations, spawn formation and wave timing.

## [0.18.0-rc.9] - 2026-09-11

### Added
- Separate no-mana basic attacks with class/equipment stats, independent server
  cooldowns, hostile/range/protection checks and round-bound replay protection.
- Phone ATTACK button surrounded by Q/W/E/R, stationary hold repeat and growing
  drag reticle with explicit candidate preview, target lock and cancel control.

### Changed
- Desktop left click selects; right click attacks hostiles or moves to ground.
  Movement cancels attack orders; S/Backspace stop movement and attacks and clear target.
- Phone skills use an explicit locked target without silently choosing another
  when the selected foe is out of range. Phone attacks do not chase.
- Wire protocol is now 2; old clients/servers fail compatibility checks clearly.
  Existing Q/W/E/R values and Q attack-speed item behavior are preserved.


## [0.18.0-rc.8] - 2026-09-11

### Changed
- Move the desktop minimap to the upper-left corner and reserve objective-panel
  space beside it. Keep the desktop inventory inside narrower windows and retain
  the separate phone HUD and computed-coordinate minimap navigation.
- Increase the normalized hero height from 1.45 to 2.1 world units and move the
  default 3D follow camera 15% closer at the same angle. Creature sizes, combat
  ranges, navigation clearance, 2D projection and zoom controls are unchanged.
- Halve decorative reed height and reduce grass-fan height by 35% in the
  deterministic runtime art derivative. Preserve ground contact, XZ placement,
  trees, rocks, materials and all 236 solid collision polygons.
- Migrate the previous 1.45 default once through preference schema 4 while
  preserving custom scales and the older default migration rules.

### Verification
- Add actual-vertex plant-height/grounding checks, narrow desktop inventory and
  saved-preference regressions, and native upper-left minimap placement evidence.
  See the dated [readability record](docs/progress/2026-09-11-hero-readability.md).

## [0.18.0-rc.7] - 2026-09-11

### Added
- Add native phone controls: an independent left joystick, right attack/ability
  cluster, drag targeting/cancellation, upgrades and focus/modal/rotation cleanup.
- Add a landscape phone HUD, compact minimap, reflowed entry/shop/help/results,
  touch-accessible menu and in-game server address entry for hosted playtests.
- Prepare Android NativeActivity packaging and an iOS Simulator bundle scaffold,
  shared native entry, packaged asset handling and private mobile preferences.
- Extend the existing actual-renderer capture command with phone dimensions and
  an explicit touch-control mode; retain desktop input and the shared UDP protocol.

### Changed
- Select desktop/mobile UI once from the compiled target OS. Android/iOS use
  mobile UI; Windows/macOS/Linux use desktop UI. Restrict mobile preview on a
  desktop to development builds and exclude phone forms/IME handlers there.
- Refine the phone HUD against the supplied Wild Rift reference: a tighter
  right-thumb skill fan, HP/mana/progression at upper right, minimap at upper
  left and shop below it. Reserve separate space for the objective and menu.
- Include objective-panel overlap checks and an explicitly labelled opt-in
  skill-upgrade layout fixture in native HUD capture verification.

### Release status
- Mobile implementation candidate, with Android/iOS toolchain, installable-build,
  real-phone and public-server gates still open. No phone download is published.
  See `mobile/README.md` and the dated mobile beta progress record for evidence.

## [0.18.0-rc.6] - 2026-09-08

### Changed
- Move the local hero's remaining route and destination to the minimap; remove
  movement-path gizmos from the world. Show the shared forest collision mask
  beneath tactical markers and the camera footprint.
- Derive 236 solid footprints from shipped Verdant geometry: 113 tree trunks,
  boulders, rock outcrops and ruin walls. Canopies, grass and walkable surfaces
  remain passable; authored lanes, spawns and objective approaches stay open.
- Share a cached spatial index, bounded A* search and swept collision between
  the native client, server movement authority and fill bots. Routes respect
  hero clearance, smooth safe segments and replan when live structures change.
- Enforce tree/stone collision on movement packets, including long steps;
  preserve normal movement speed and the existing input/modal controls.

## [0.18.0-rc.5] - 2026-09-08

### Added
- Right-click the battlefield or minimap to order a persistent hero route. Local
  golden waypoints and a destination ring show the intended movement; another
  order replaces it and cancels a pending approach/cast.
- Route around solid towers and bases using their existing collision radii;
  project obstructed destinations to reachable clearance and stop on arrival.
  This uses the current collision model, not a terrain navmesh.
- Native input-driven navigation capture with independent UDP confirmation of
  obstacle detouring, destination arrival, menu/orbit isolation and camera pan.

### Preserved
- Left-click/touch movement and attacks, left-click/touch minimap camera pan,
  Alt+right-click orbit, speed limits and server movement authority.

## [0.18.0-rc.4] - 2026-09-07

### Added
- Add a server-authoritative six-item base shop, class recommendations, starter
  gold and passive match income. Equipment affects damage, attack/spell rate,
  movement and resources; paid purchases survive death/reconnect and reset on rematch.
- Add circular hero portraits, a gold local-player halo, lane/river markings
  and the actual yellow camera ground-footprint outline to the bottom-right minimap.
- Add shared radial detection for enemy hero minimap markers. This is minimap
  detection only; world rendering and network snapshots still expose full positions.

### Changed
- Increase default hero presentation height by 26% with a one-time saved-default
  migration; preserve custom hero settings and existing creature sizes.
- Give both teams a fixed camera orientation with a diagonal midlane matching
  the minimap; retain deliberate orbit, zoom, follow and recenter controls.
- Rebuild the playable HUD with compact resource bars, ability states,
  inventory/gold and a modal shop using bundled OFL-licensed Inter typography.
- Extend native UI capture to real receipt-confirmed purchases, shop dismissal,
  both team spawns and 720p/1080p; extend normal-rules match acceptance to equipment.

## [0.18.0-rc.3] - 2026-09-07

### Fixed
- Keep dead heroes at zero HP when shared XP grants a level, until the normal
  authoritative respawn restores them.
- Remove the pause-menu local restart that stranded an admitted player. Add
  clickable help dismissal and make Escape close help before opening Pause.
- Hide healthy connection status over the minimap, improve HUD contrast and
  bound the result panel with an accurate automatic-rematch countdown.
- Face both home spawns from the lane side so the sanctuary cannot hide a
  newly admitted hero behind its arches in the default camera view.

### Changed
- Tune team-shared lane XP for the full five-player roster; fill bots use legal
  unlocked offensive abilities, distribute classes across both factions and
  rally through an open lane after level 6 to finish the base push.
- Add a bounded real-UDP, normal-rules two-round acceptance runner with roster,
  objective protection, reset, progression and snapshot-health evidence.
- Add source-independent practice, host and remote-join launchers with validated
  player counts/addresses, session logs and cleanup of their owned processes.
- Add native 720p UI capture coverage and a tester-first beta guide for September 8.

### Release status
- Controlled native 3D beta candidate for the tested macOS ARM64 package.
  See the dated beta readiness record for measured match/UI evidence and
  remaining human, hardware, platform/network and dependency-review limits.

## [0.18.0-rc.2] - 2026-09-05

### Changed
- Bring the original Verdant Confluence Blender arena into native 3D: jade/sage
  forests, pale stone routes, turquoise river, ruins, sanctuaries and watchtowers.
  Warm sun/cool fill and a wider play camera support the new architectural scale.
- Derive runtime assets deterministically from the preserved scene; normalize
  walk surfaces and reconcile mitered base ramps without changing server routes.
- Attach faction structures to authoritative owners, including destruction and
  rematch; share environment/actor assets and preserve optional 2D behavior.
- Remove El Bueno, both imported slimes and Wendigo Hollow plus their previews.
  Keep 15 heroes and King Mutatio; original animated stone/crystal minions and
  guardian retain the removed creatures' gameplay roles.
- Replace the unconditional internal-review package guard with a content check
  of actual files, denied hashes, approved provenance and embedded permissions.
- Add opt-in native GPU screenshot evidence and an isolated capture runner.
- Order local-player fallback spawning after snapshot admission to prevent a
  duplicate entity and startup panic. Hide development controls/hotkeys when
  debug UI is disabled; use supported ASCII separators in gameplay text.

### Release status
- The four-model asset conflict is addressed by exclusion and original
  replacements. The package remains a controlled playtest candidate; human
  sessions, long performance runs, platform/network coverage and remaining
  dependency dispositions are separate gates. See the dated Verdant test guide.

## [0.18.0-rc.1] - 2026-09-05

### Changed
- Prepare the native Models3d candidate for controlled playtests, with a dated
  implementation ledger and reproducible package containing binaries and assets.
- Join is idempotent, reconnect restores complete authoritative state, reserved
  seats count toward capacity, and abandoned matches reset after a bounded grace.
- Rematches rebuild all gameplay state and repeat release formation/countdown;
  release servers reject development god/speed commands.
- Protocol 1 negotiates snapshots in datagrams of at most 1200 bytes, with bounded
  reassembly and epoch/round/tick ordering; legacy JSON scripts remain supported.
- Join retries until authoritative admission or visible rejection, with recovery.
- Models3d camera orbit uses held Alt+right mouse; shared modal/debug input rules
  isolate gameplay. Roster layout excludes inactive content and scrolls at 720p.
- Shipped heroes play attack/cast/death clips, with distinct allegiance shapes,
  bounded combat effects, visible action feedback and hotbar availability.
- Models3d startup uses shipped assets without optional 2D loads or SDK downloads.
- Bases require a protecting lane tower to fall first. Scenario bots spend skill
  points, use applicable self-sustain and require wave support for tower dives.
- Match logs report progression, deaths, objectives, disconnects and victory.
- Update existing crossbeam-channel to 0.5.15 to address GHSA-pg9f-39pc-qf8g
  in the client networking dependency, without adding packages or changing the SDK revision.

### Release status
- Asset provenance conflicts in four models restrict the current package to
  internal review. Public distribution needs rights-holder evidence or replacements.
- This is a candidate with outstanding human, visual, performance, platform and
  network certification gates. Automated tests do not certify a public release.
- XP rewards remain unchanged pending measured full-roster pacing; gold has no
  spending path. Optional 2D asset failures remain outside this 3D milestone.

## [0.17.1] - 2026-09-05

### Fixed
- Clean-checkout locked builds now retain the existing git SDK revision in
  Cargo.lock, without dependency upgrades or a sibling checkout override.
- Include the two existing CC0 slime minion models and their provenance
  manifest in version control so the default 3D renderer can load them.
- Derive the large UDP regression fixture from the shipped avatar roster and
  assert the replicated identity, avoiding dependence on local Arena downloads.
- Keep the shipped avatar roster to the 16 committed offline models; remove
  references to local-only Arena downloads while preserving those local files.

### Documentation
- Add the current 3D readiness audit, prioritized gameplay/UI/network roadmap,
  reproducible verification results and a mandatory full-match/rematch test gate.
  This patch is a development delivery fix, not a playtest-ready release.

## [0.17.0] - 2026-07-29

### Added
- Desktop and mobile now share direct screen-space combat input: click or tap a
  living hostile hero, minion, neutral, tower, or base to select it and request
  the Q attack. Touch-sized logical-pixel hit areas remain stable across 2D
  camera zoom, while Tab and middle-click selection remain available.
- Out-of-range unit casts now queue a bounded approach to the moving selected
  actor and emit exactly one cast after entering the shared scaled ability
  range. Manual ground movement, target death, or target removal cancels it.
- Ground taps provide the same 2D click-to-move path as desktop primary clicks;
  target, minimap, and UI presses are consumed so one gesture has one action.

### Changed
- Keyboard Q/W/E/R and the 64-pixel on-screen skill buttons now queue through
  the same target-aware cast path. Self-target skills still cast without an
  enemy. Missing-target, cooldown, mana, and approach states have explicit
  diagnostic feedback.

### Fixed
- Local cooldowns no longer begin for out-of-range requests that the server
  would reject without producing an attack. Direct 2D target selection no
  longer depends on a middle mouse button or a zoom-sensitive ground radius.

## [0.16.1] - 2026-07-28

### Changed
- `Y` now toggles hero camera follow in both visual modes. If the minimap is
  currently supplying a focus override, `Y` clears it and immediately restores
  hero follow; `Space` remains the force-recenter shortcut.
- 2D player nameplates now scale against hero height and estimated label width
  instead of using a fixed oversized transform.

### Fixed
- 2D ground movement no longer stops accepting commands when camera follow is
  unlocked. Right-click and Alt also no longer accidentally toggle follow or
  capture the cursor in `sprite2d`; their legacy 3D behavior is preserved.

## [0.16.0] - 2026-07-28

### Added
- **TASK-2D-PRODUCTION-READINESS-01 — readable match actors.** Genuine 2D
  mode now presents the six authoritative lane towers with Green-square or
  Blue-diamond team badges and explicit TOP/MID/BOT labels. Both teams' lane
  minions use the same color-plus-shape language. Proxy reconciliation runs
  after snapshot/interpolation updates, remains one-to-one with authoritative
  owners, and recursively removes its bounded cues when an owner disappears.
- Added occupied-alpha bounds to every non-player 2D actor definition and an
  offline screen-space readability validator. Focused ECS tests cover the
  exact six-lane-tower plus two-base plus 18-minion render world, idempotent
  24→23 lane-tower/minion cleanup, sustained teardown/recreate bounds,
  team/lane cues, authoritative projection, and `models3d` isolation.
- Added a real-server 5v5 UDP regression which receives and verifies a full
  runtime-dependent JSON datagram in the asserted `8192 < bytes <= 65507`
  range containing ten players, eight structures, and 18 minions, then proves
  the server recovers after malformed and oversized client datagrams.

### Changed
- The genuine 2D camera is twice as close and render-only actor sizes now use
  occupied pixels to preserve the tower > hero > minion hierarchy at default
  zoom and maximum zoom-out. The selection portrait atlas now covers all ten
  manifest characters while preserving the original five cells byte-for-byte.
- Client and harness receive storage is now 65,536 bytes, covering the legal
  IPv4 UDP application-payload ceiling of 65,507 bytes. The server checks a
  complete serialized snapshot against that ceiling before sending, rejects
  oversized client requests whole, and rate-limits malformed/send diagnostics.

### Fixed
- Removed the exact-8-KiB receive truncation that produced repeated Serde EOF
  failures at column 8192 and intermittently hid trailing structures/minions.
- Lane towers and minions no longer collapse to near-subpixel silhouettes at
  supported 2D camera zooms.

## [0.15.0] - 2026-07-28

### Added
- **TASK-FULL-2D-WORLD — genuine orthographic 2D game mode.** `sprite2d` now
  creates a planar XY render world with one `Camera2d`, deterministic 55×55
  tiled terrain, paths, a traversable diagonal river, forest belts, both
  bases, all lane structures, camps, and boss pits. Heroes, minions,
  structures, neutrals, bosses, projectiles, markers, bars, labels, and
  bounded combat VFX use cached Bevy 2D sprites; the mode no longer renders
  the old 3D arena, GLBs, mesh billboards, or directional-light scene.
- Added a tested XZ↔XY projection/picking layer, orthographic follow/free-pan
  camera with clamped zoom and minimap focus, stable foot-Y layer sorting,
  deterministic prop placement, a 4,096 static-entity ceiling, and a 256 VFX
  cap with two-second cleanup.
- Added original CC0 world art produced through Higgsfield Recraft V4.1,
  `client/assets/world2d/manifest.json`, a topology-aware offline validator,
  negative fixtures, and final world/contact-board proof artifacts.
- **TASK-2D-RELEASE-VERTICAL-SLICE — networked 2D combat presentation.** All
  five sprite heroes now have manifest-driven attack, cast, hit, and death
  one-shots in addition to idle/run. Accepted server casts replicate a safe
  action sequence/kind/slot so local and remote clients play each action once;
  HP deltas drive hit/death/respawn transitions with deterministic priority.
- Added an art-directed `presentation2d` layer for the arena, towers, bases,
  team minions, neutrals, both raid bosses, projectiles, cast/hit/heal/death
  VFX, portraits, and UI framing. Sprite mode replaces primitive/GLB combat
  actors with cached billboard art while preserving their gameplay roots;
  the default 3D path remains available.
- Added a frozen style bible/gap matrix, expanded asset validation for the
  sprite manifest v2 and presentation atlas, and a real-UDP two-client combat
  action test covering accepted, rejected, sequential, legacy, and unknown
  action fields.
- Added `make game2d` as the direct single-client launcher for the illustrated
  presentation; it preserves `GAME_SERVER_ADDR` behavior from `make game`.
- **TASK-2D-SPRITE-PROTOTYPE — optional animated 2D player visuals.** The
  pre-join screen now keeps independent 3D-avatar and 2D-sprite selections
  and offers five original characters: Mossback Teapot, Neon Axolotl Courier,
  Origami Storm Heron, Clockwork Turnip Oracle, and Void Jelly Astronaut.
  `models3d` remains the default; set `OMOBA_PLAYER_VISUAL_MODE=sprite2d` for
  billboarded, unlit sprite-sheet visuals in the existing 3D arena. Local and
  remote sprites select idle/run loops from owner movement, and the optional
  cosmetic id is validated, reconnect-retained, and snapshot-replicated.
- Added `client/assets/sprites/manifest.json` as the runtime source of truth,
  CC0 provenance documentation, and `scripts/validate_sprite_assets.py` for
  structural PNG/manifest validation (including negative contract tests).

### Changed
- **`ekza-bevy-sdk` is now a git dependency.** `client` and `server` depend on
  https://github.com/ekza-space/ekza-bevy-sdk (branch `main`) instead of a
  relative `../../ekza-bevy-sdk` path, so a fresh clone of this repo builds
  without any sibling checkout. Local development still uses the on-disk SDK
  via a gitignored `.cargo/config.toml` `[patch]` override.

## [0.12.1] - 2026-07-06

### Fixed
- **Character select popped up mid-game (TASK-25).** Any client-side
  connection teardown (3 s snapshot staleness, transport failure, wait
  timeout) respawned the team-select overlay even for a joined player — a
  transient hiccup silently kicked you back to "pick a character" and let
  you re-enter. Now every teardown logs its reason, and a session with a
  committed join never shows the select screen again: the client
  auto-reconnects on the 2 s retry cadence and auto-rejoins with the
  remembered loadout + persistent session id (the server reclaims the
  session for up to 30 s, so hero/team/position survive short outages).
  The connection panel shows "Connection lost — reconnecting (attempt
  N)..." meanwhile; the select screen still appears for players who never
  joined, and the manual Retry button keeps working.

## [0.12.0] - 2026-07-06

### Added
- **TASK-24 — slime minion models.** Lane minions render as team-colored
  CC0 "Mimic Slime" creeps (Halloween Rising, Polygonal-Mind; green
  "Classic" / blue "Water") staged from the Open Source Avatars collection
  with retargeted UAL clips, replacing the placeholder spheres. Minions go
  through the shared model-scale pipeline (0.6× hero height, tweakable via
  `model_scale_overrides.json` keys `slime-green`/`slime-blue`), stand on
  their measured foot offset, get the VRM double-sided material fix, and
  animate from the replicated AI state (marching/chasing → walk,
  attacking → attack). Headless analyzer now measures `minions/` too
  (36 models).

### Fixed
- **Camps and raid bosses were entombed in decorative boxes.** The three
  neutral camps and both boss pits sit exactly at jungle-block centers, so
  the 12×4×12 decorative boxes fully enclosed the creatures. Blocks within
  10 units of a camp/boss anchor are no longer spawned (5 of 10 removed):
  camps and bosses now stand in open clearings and are visible from the
  battlefield.

## [0.11.0] - 2026-07-06

### Added
- **TASK-23 — bot lane-push AI.** Fill bots now actually play instead of
  wandering at spawn: each bot takes a lane (Mid/Top/Bot round-robin),
  walks its lane waypoints toward the enemy base (geometry mirrors the
  server's lane control points, oriented by the server-assigned team),
  fights enemy players and minions it meets (approach into Q cast range,
  hold, cast — ranges from the shared per-class ability kits), sieges
  enemy towers in reach, and rejoins its lane from the nearest waypoint
  after a respawn. The server stays fully authoritative: bot movement
  steps fit the speed budget and the server enforces cast range/cooldown/
  damage. New `harness::bot_ai` module (pure, unit-tested brain), harness
  snapshot mirror now models minions and structures for targeting, and a
  live integration test proves a brain-driven bot pushes ≥20 units along
  its lane on a real server with server-accepted positions.

## [0.10.0] - 2026-07-06

### Added
- **TASK-22 — matchmaking and gated match start.** The server now has two
  explicit match modes (`OMOBA_MATCH_MODE`): `release` (default) queues
  joining players, forms the match to a full 5v5 roster
  (`2 × OMOBA_TEAM_SIZE`, default 5, clamped 1–16), assigns balanced teams
  server-side (client team choice becomes a preference), runs a 3-second
  countdown, and only then starts the match; `dev` preserves the historical
  instant start on first join for local development. New replicated match
  phases `forming { ready, needed }` and `starting { countdown_ms }`;
  countdown rolls back to forming if a player drops, and an empty queue
  returns to lobby. Joins beyond a full roster are rejected and logged.
- **Client matchmaking UX.** The lobby overlay now walks through the search
  states: "Searching for match...", "Waiting for players — X/10",
  "Match found! Starting in N...". The client adopts the server-assigned
  team on spawn (release-mode balancing) instead of waiting for its
  requested team to be acked.
- **Fill bots for solo testing.** `cargo run -p harness --bin bots`
  (`make bots`, `make play-bots`) joins N dummy UDP clients (round-robin
  classes/avatars, join-resend until acked, ping keepalive, light wander)
  so one developer can fill a 5v5 queue and walk the full matchmaking flow.
- **Makefile/dev-flow split.** `make server` (release), `make server-dev`,
  `make start` (dev quick-start, unchanged UX), `make start-release`,
  `make play-bots`, `make bots BOTS=<n>`, extended `make stop`; RUNBOOK and
  README document modes, env vars, and the solo bot flow, and state
  explicitly that instant start is dev-only.
- **Tests.** 10 new server unit tests (mode parsing, formation gating at
  9/10, countdown + rollback, 5v5 balancing, full-match rejection, dev
  instant start), a client overlay-state test, and a release-mode harness
  integration test (`OMOBA_TEAM_SIZE=1`: solo waits, second player triggers
  countdown → running, teams balance 1v1). Existing harness gameplay tests
  now run the server explicitly in dev mode.

## [0.9.1] - 2026-07-06

### Fixed
- **Characters walked backwards.** Movement code aligned the entity's +Z axis
  with the walk direction, but character models face -Z (Bevy forward), so
  every model rendered 180° from its heading. Local yaw now points -Z along
  the movement direction (`(-dx).atan2(-dz)`); the flipped yaw replicates
  as-is, so remote players match. Raid-boss models (same VRM-staged facing)
  keep the server's +Z yaw convention and get a 180° model-child rotation
  instead.

## [0.9.0] - 2026-07-06

### Added
- **TASK-21 — world-relative character size.** Characters were normalized to
  0.26 world units in a world tuned for `PLAYER_SIZE = 1.0` (46-unit base
  pads, 4-unit jungle blocks, camera at ~19 units) — barely visible.
  `DEFAULT_MODEL_TARGET_HEIGHT` is now 1.15 with range [0.3, 3.0]; persisted
  target heights saved under the legacy scale (below the new minimum) are
  migrated to the new default on load instead of being clamped.
- **TASK-21 — spawn platform traversal (League-style).** The base pad is now
  walkable: `MapLayout::terrain_height(x, z)` describes the pad top and a
  6-unit linear ramp band around it, matching four new visible ramp slabs
  spawned per pad (corner-overlapping, team-colored). Local player gravity,
  the jump-fallback hop, remote players, and minions all ground onto that
  surface client-side; the server keeps its flat ground plane (no protocol
  change). Normalized models now also expose a measured foot offset, so
  character feet rest on the surface instead of the entity origin floating
  at cube half-height (models used to sink ~0.2 into the 0.7-tall pad).

## [0.8.0] - 2026-07-05

### Added
- **TASK-20 — character scale normalization module.** New
  `client/src/model_scale.rs` owns all model-size logic. Every character and
  boss GLB is measured once in bind pose straight from the loaded glTF data
  (node transforms × mesh bounds — independent of animation state or spawn
  timing; raw heights ranged 0.64 m..2.41 m across legacy models, roster
  avatars, and bosses) and its root is rescaled absolutely to the shared
  target height, so all characters render at exactly the same size by
  default. Per-model multipliers live in
  `client/assets/config/model_scale_overrides.json` (slug → multiplier,
  missing = 1.0), hot-reloaded ~1 s while the game runs. A headless analyzer
  mode (`OMOBA_MEASURE_MODELS=1 cargo run -p client`) prints the measured
  height table using the same code path the game uses.

### Fixed
- **Model rescaling no longer compounds.** The old normalization sampled the
  world-space AABB after spawn (timing/animation dependent) and re-applied
  relative factors on top of the already-scaled transform when the target
  height changed. Scales are now always derived from the raw measured height;
  the AABB fallback for primitive stand-ins remembers its first raw
  measurement and stays absolute too.

## [0.7.1] - 2026-07-03

### Fixed
- **Server: pre-join ghost players.** Any packet (including the transport's
  immediate `Ping` heartbeat) used to create a fully joined default player
  (Green, default character) that appeared in everyone's snapshots and could
  start the match. Endpoints are now tracked as `joined = false` until their
  `Join` packet arrives: they still receive snapshots (for addressing) but are
  excluded from the replicated player list and from all gameplay (movement,
  casting, skill upgrades, rematch/god-mode/speed-boost requests, minion and
  tower and neutral targeting, buff regen, and kill-reward splits).
- **Client: lobby overlay blocked the character-select screen.** The
  full-screen "Waiting for match to start..." overlay is now `Pickable::IGNORE`
  and only shows once the local join is committed, so the pre-join class/
  avatar/team select UI stays visible and clickable on a fresh server.
- **Client: team click while disconnected silently lost the join.** Picking a
  team with a dead transport no longer commits the selection and despawns the
  select overlay (which stranded the player); it now triggers the same
  reconnect flow as the Retry button and keeps the select screen up.

## [0.7.0] - 2026-07-03

### Added
- **TASK-19 — raid bosses: epic neutral objectives with team buffs.**
  - **Two raid bosses on the neutrals system**: Wendigo (bottom pit, spawns at
    60 s match time, 900 HP) and King Mutatio (top pit, spawns at 180 s,
    1500 HP), placed at 180°-rotationally-symmetric pits derived with the same
    map formula as the jungle camps. Bosses aggro when attacked, use a larger
    leash (full-HP reset at the pit), and respawn 180 s after death while
    camps keep their 40 s cooldown. All tuning is named constants in
    `server/src/balance.rs`.
  - **Team buffs on boss kill**, replicated via a new additive
    `team_buffs` snapshot field (`serde(default)`): Wendigo's Favor (+15%
    ability damage, 90 s) and Mutatio's Might (+25% ability damage plus
    2 HP/s team regen, 90 s). Re-kills refresh the timer; both buffs combine
    multiplicatively; the server applies the damage multiplier and the regen
    authoritatively. Rematch (`reset_match`) clears buffs and restarts the
    boss spawn schedule.
  - **Client presentation**: bosses render their staged CC0 GLB models
    (`client/assets/bosses/`, staged/retargeted/validated by the existing
    avatar pipeline with a dedicated manifest) scaled to ~3x player height,
    with HP bar, floating nameplate, idle/walk animation driven by the
    replicated AI state, and a match-HUD indicator listing the local team's
    active buffs with remaining seconds. Boss slugs live outside the player
    roster manifest, so they are never selectable as player avatars
    (covered by a shared-crate test).
  - **Tests**: server unit coverage for the spawn schedule, boss stats/pits,
    per-type respawn, buff apply/expiry/refresh/team scoping and authoritative
    buffed damage/regen; harness integration coverage for live boss spawn
    timing and stats over the wire; HUD text unit tests; asset validation for
    the boss directory (`--roster-min/--roster-max`).


## [0.6.0] - 2026-07-03

### Added
- **Ekza Arena avatar sync (`arena-sync` crate).** New workspace tool that
  pulls Avatar cards from the on-chain Ekza Arena registry (raw JSON-RPC
  `getProgramAccounts` + minimal borsh parsing, no anchor client), fetches
  each card's metadata, enforces the model-format classifier (only `vrm` /
  `glb` — what Bevy's glTF loader handles; mirrors the on-chain
  `ProjectProfile "omoba"` in solana-stellar), downloads the model +
  thumbnail into `client/assets/avatars/`, and idempotently merges the
  entries into `manifest.json` under collection "Ekza Arena".
  Usage: `cargo run -p arena-sync -- [--rpc …] [--dry-run]`.

### Changed
- **Avatar roster is now file-first.** `shared::avatar_roster()` reads
  `client/assets/avatars/manifest.json` at runtime (override with
  `OMOBA_AVATAR_MANIFEST`); the compile-time embedded manifest is only the
  fallback. Avatars synced from the chain appear in the team-select grid
  after a client restart — no rebuild. Client and server must share the same
  manifest file, otherwise the server's slug validation rejects runtime-added
  avatars.
- Roster unit test now checks invariants (lower bound, unique slugs,
  non-empty license/source) instead of a hard 10–20 size window.

## [0.5.0] - 2026-07-03

### Added
- **TASK-18 — environment decoration: procedural vegetation from primitives.**
  - New client-only `DecorPlugin` (`client/src/decor.rs`) that dresses the
    arena with stylized low-poly props assembled purely from Bevy primitives
    (Cuboid, Sphere, Cylinder, Cone, Capsule3d): 3 tree variants, 2 bush
    variants, grass tufts, 4 flower variants, and 2 rock variants — no
    external art assets.
  - Deterministic seeded scatter (`generate_layout`, inline splitmix64 PRNG,
    no new dependencies): forest belts along the arena edges, trees/boulders
    ringing the jungle blocks, grass/flowers/bushes across the open meadow.
    Exclusion zones derived from the real map constants keep lanes, base
    pads, towers, neutral camp clearings, the river, and the jungle blocks
    completely clear; covered by unit tests.
  - Purely cosmetic: no collision, no server/shared changes, no networking.
    Fixed layout: 396 props = 970 entities (budget ceiling 1200), spawned
    once at `Startup` under a single `DecorRoot`, reusing 5 shared mesh and
    12 shared material handles so Bevy batches instances.
  - Client-local F4 debug toggle hides/shows the whole decoration layer
    (Visibility flip on `DecorRoot`, logged).
  - `MapLayout` now exposes the lane/river/jungle-block/camp geometry as
    shared methods consumed by both the map renderer and the decor layout,
    so the exclusion math cannot drift from the rendered map.

## [0.4.0] - 2026-07-03

### Added
- **TASK-17 — playable demo: hero classes + VRM avatar roster.**
  - **Four hero classes with distinct Q/W/E/R kits** (Warrior, Mage, Ranger,
    Cleric; 16 distinct ability definitions) defined in the `shared` crate and
    resolved **authoritatively on the server** per player. Kits reuse the
    projectile-damage / self-heal / self-mana-restore primitives with per-class
    numbers; rank mechanics unchanged (max rank 3, `rank_effect_scale`,
    cooldown/range scaling) and slot unlock levels preserved (Q@1/W@2/E@4/R@6,
    now enforced server-side per cast). Casts carry a slot index and cool down
    per slot; skill upgrades cap at the shared max rank.
  - **CC0 avatar roster (16 VRM avatars)** staged as GLB with embedded
    retargeted clips (`idle`/`walk`/`attack`/`cast`/`death`, Quaternius UAL,
    CC0) under `client/assets/avatars/` with a provenance manifest. The
    manifest is embedded in the `shared` crate so client and server agree on
    the shipped set; unknown slugs fall back to the default model (and unknown
    class ids decode as Warrior) without breaking packets.
  - **Pre-join selection flow**: class buttons (name + kit summary), a
    16-avatar thumbnail grid, then team; the join packet carries
    `{team, character, hero_class, avatar, session_id}` and the server
    replicates class + avatar to every client. `OMOBA_AUTOJOIN=<class>:<slug>:<team>`
    joins without UI for automation/evidence runs.
  - **Runtime avatar animation**: roster avatars load lazily, spawn for local
    and remote players, and drive the idle/walk locomotion graph from movement
    state (with a short idle-grace hysteresis so snapshot interpolation does
    not flap the animation). The VRM double-sided material fix now covers the
    whole roster (previously Paco-only).
  - **Class-aware HUD**: the hotbar shows the selected class's ability names
    and per-slot rank; the match HUD lists each slot's ability with effect
    numbers, per-slot cooldown, and lock level.
  - **New tests**: shared kit/unlock/rank/roster unit tests; server tests for
    per-class cast resolution, self-target heals, unlock gating, rank caps, and
    avatar-slug normalization; harness end-to-end scenarios for two clients
    joining with different class+avatar (replication + distinct kit costs),
    locked-slot rejection, and hostile class/avatar values falling back safely.

### Fixed
- Roster thumbnails that were actually JPEG data under a `.png` name are now
  staged with their real extension (Bevy picks the image decoder by extension);
  the client enables the `jpeg` Bevy feature. `scripts/stage_avatars.py` sniffs
  the magic bytes when staging.

### Earlier unreleased work shipped with this release

### Added
- **VRM avatar support + one CC0 humanoid (`Paco`):** the engine now loads VRM 0.x
  avatars through the existing glTF model catalog. VRM 0.x files are glTF 2.0
  binary containers whose VRM-specific data (`VRM`, spring bones, blendshapes) is
  listed under `extensionsUsed` only — never `extensionsRequired` — so Bevy's
  standard `GltfLoader` ignores it and still loads the mesh + skeleton. The
  avatar is staged as `.glb` (a byte-identical glTF 2.0 container) so the asset
  server selects the glTF loader by extension; the validate-and-copy step lives
  in `scripts/convert_vrm_to_glb.py`. Added a new `EkzaCharacter::Paco` variant
  (SDK enum + `ALL` + `BUILTIN_MODEL_MANIFEST` → `downloaded/paco.glb`), so it
  appears in the character-select UI and spawns/normalizes like other models.
  Avatar: *Paco* (Avatar 211) from ToxSam's **100Avatars R3**, **CC0** — see
  `ATTRIBUTION.md`. The avatar ships **no animation clips**, so it renders as a
  static skinned mesh via the existing anim-less fallback (no idle/walk
  locomotion); see the progress note for how to add clips later.
- **Headless gameplay test harness (`harness/` crate, `publish = false`):** a new
  workspace member that spins up the *real* UDP server on a unique loopback port
  per test and drives it with typed bot clients over the JSON wire protocol — no
  GPU, no renderer, no human. Layered into `protocol` (a documented test mirror of
  the server wire format), `server` (`ServerProcess`, RAII: kills the child on
  drop), and `bot` (`Bot`, typed packet senders + freshest-snapshot polling).
  Integration scenarios in `harness/tests/gameplay.rs` assert: join → snapshot at
  full HP; god mode prevents all damage (with a no-god-mode control proving damage
  is detectable); speed boost widens the movement-authority clamp; and an
  `upgrade_skill` with zero points is a server-side no-op. Run via
  `make verify-gameplay` (builds the server first, then `cargo test -p harness
  -- --test-threads=1`). No server/client source was modified.
- **TASK03 — upgradable skills:** the primary ability (Q) now scales with an
  authoritative per-slot rank. Server tracks `ranks: [u8;4]` in `PlayerState`,
  handles a new `UpgradeSkill { slot }` packet (spends one skill point, capped at
  `MAX_SKILL_RANK`), and the projectile damage is `PRIMARY_ABILITY_DAMAGE_BY_RANK`
  by Q rank (now an increasing table 20→52). Client shows each slot's `Lv N` and an
  upgrade ↑ button (lit when a point is spendable); the `U` key upgrades Q.
- **TASK04 — God Mode (debug):** a left-side toggle button makes the local player
  invulnerable for gameplay debugging. Authoritative: `ClientPacket::SetGodMode`
  sets a server-side `god_mode` flag that skips all player damage (projectile,
  neutral, and minion attacks). Not networked back; the requesting client owns it.

### Added
- **TASK05 — Debug Speed Boost:** a button next to God Mode (bottom-left) toggles an
  authoritative movement multiplier (`DEBUG_SPEED_MULTIPLIER`). The server widens the
  movement-authority clamp so the boosted client is not rubber-banded; the client
  moves faster locally. Re-asserted on (re)connect.

### Fixed
- Minions now always prioritize enemy minions over players: while any enemy minion
  is within vision a minion never targets a player (overrides sticky player aggro);
  players are only chosen when no enemy minion is in range. Covered by
  `minion_prefers_enemy_minion_over_closer_player`.
- Debug toggles (God Mode / Speed Boost) now take effect reliably: a single
  edge-triggered send could be lost (UDP, connection races, fresh server session),
  leaving the server flag unset — so god mode "did nothing" and speed boost
  rubber-banded. The client now re-asserts the current toggle state to the server
  ~2x/sec (idempotent; server logs only on change), and the snapshot reconcile
  widens its local snap threshold while boosting so the boosted player is not
  snapped back. Verified end-to-end against a live server (`set_god_mode` /
  `set_speed_boost` received and applied).
- Debug toggles can now be driven by keyboard (**F2** god mode, **F3** speed boost)
  as a reliable fallback if the on-screen buttons do not receive clicks; toggling
  logs `[debug] god_mode/speed_boost -> <bool>` client-side, and the server logs
  receipt, to diagnose the command path end to end.
- God Mode now reliably keeps the player alive: besides skipping damage at every
  player-damage site, the server restores god-mode players to full HP **and full
  mana** each tick (after damage, before respawn) and on toggle, so no missed path
  can kill them and abilities can be cast freely.
- Stopped per-frame `"idle/walk animations were not found"` log spam for models
  without locomotion clips: the animation library now skips a GLTF once evaluated
  (gating on `evaluated_characters`) instead of re-checking every frame.

### Changed
- Skill upgrade arrows now appear **only when a point can actually be spent** on that
  slot (hidden otherwise) instead of always showing dimmed.
- God Mode debug button moved to the bottom-left, on the same line as the skill bar,
  and is re-asserted to the server after a (re)connect (the server resets the flag for
  a fresh session).
- First level is reachable in ~3 minion kills (`LEVEL_XP_THRESHOLDS[0]` 120 → 90) so the
  skill-upgrade flow is easy to exercise during playtests.

### Fixed
- Target selection (`Tab` nearest-enemy and middle-click) no longer picks **friendly minions**: minion candidates now skip same-team units like players and structures do, so the enemy base tower can be selected near friendly minion waves (`client/src/combat.rs`).
- Head HP/Mana bar for player models is now anchored to a deterministic normalized head height (`NormalizeModelScale.head_local_y`) instead of unstable per-frame AABB sampling, so the bar sits above the head for every character regardless of GLB pivot (previously drifted to mid-body for `wang`/`toka`).

### Added
- **UI/UX iteration:** team-select buttons now show their `Green`/`Blue` labels (previously bare colored squares) plus a flow hint ("Pick a character, then a team to join the match."); the in-match HUD gains color-coded HP and Mana bars (HP tints green/amber/red by ratio) shown only while the match is `Running`.
- Production authority hardening: stable optional client session ids in join packets, client-side persistence for the id, and server-side reclaim of timed-out player slots from a new UDP endpoint.
- Standalone sibling `ekza-bevy-sdk` repository with stable Ekza-Stellar character ids, built-in 3D model manifest metadata, GLB validation helpers, and a Bevy-gated model catalog loader for local/remote GLB assets.
- SDK model validation module and examples: typed GLB validation reports, configurable rules, issue enums, a `model_check` CLI example, a headless `model_cache` built-in source verifier, and an interactive Bevy `model_viewer` viewport.
- TASK-12: centralized server gameplay tuning in `server/src/balance.rs` with `docs/balance-tuning.md`; release gate checklist, manual QA matrix + run log, and release readiness report under `docs/`.
- `scripts/verify_task_12_qa_matrix_live_udp.py` and `make verify-task-12` for recorded two-client UDP join (M1) and cast/mana/damage smoke (M3).
- `PRIMARY_ABILITY_DAMAGE_BY_RANK` / `SKILL_SLOT_COUNT` in `balance.rs` for documented per-rank and four-slot progression hooks.
- Server regression test tying jungle camp templates to `balance` constants; `cast_drains_mana_respects_cooldown_and_blocks_empty_mana` for cast/mana/cooldown invariants.
- **TASK-13 (client):** Match HUD column (level, XP, skill points, upgrade key hint, HP/mana, target summary, objective line, F1 reminder); bottom skill bar with four labeled slots (`Q`–`R`) wired to the same cast action until the server exposes distinct skills; centralized key labels in `input_bindings`; F1 help overlay with control and objective copy; clearer victory/defeat next-step text; help panel only renders during `Running` so lobby/victory overlays stay readable on reconnect/snapshot transitions; bracketed skill slot line and display strings derived from `SKILL_SLOT_KEY_LABELS`; unit tests for binding/display consistency.
- **TASK-14**: Client connection lifecycle (`Connecting` / `WaitingForServer` / `Connected` / `Disconnected`), bounded wait and stale-snapshot handling with named thresholds in `session_config`, transport failure signaling from the UDP thread, teardown that clears replicated entities and re-opens team select, manual **Retry** after disconnect, ingest/apply snapshot pipeline ordering for Bevy 0.18, snapshot-channel disconnect detection (`NetIncomingDisconnected`), pause menu closes on **Disconnected**, minimap visible only while **Connected**, preferences save when the resolved server address resource updates, settings panel shows current server address text, and `docs/network-client-session.md` for constants and manual QA cross-reference.
- Client preferences file: graphics (lighting, model scale), character selection, and optional `game_server_addr` with load precedence `GAME_SERVER_ADDR` → saved file → default; pause menu **Reset graphics to defaults**; paths documented in `client/src/persistence.rs` and `RUNBOOK.md`.
- **TASK-15:** Playtest and ops documentation — root `README.md`, `docs/playtest-script.md` (10–20 minute session), `docs/bug-report-template.md`, `docs/mvp-scope-and-limitations.md`, `tasks/MVP-CHECKLIST.md` (MVP vs deferrable), expanded `RUNBOOK.md` troubleshooting with recovery steps, cross-links from `docs/features.md`, and `.gitignore` whitelists so these paths stay versioned (previously blanket-ignored).
- Added a reproducible multiplayer session verification harness at `scripts/verify_task_02_multiplayer_session_flow.py` that exercises sequential and simultaneous joins, repeated joins, timeout cleanup, reconnect-as-new-player, server restart recovery, and four-client snapshot consistency against the live UDP server.
- Added focused client coverage for authoritative local-player selection so duplicate local `Player` entities cannot silently break gameplay systems that rely on `Query::single()`.
- Added multiplayer session policy documentation and a `TASK-02` progress log with the recorded session matrix.
- Jungle neutral camps: three server-simulated camp types (Skirmisher, Bruiser, Spitter) with distinct HP, damage, attack range, and kill rewards; placement mirrors client jungle layout (off-lane).
- Server-authoritative neutral AI (idle, proximity/damage aggro, chase, attack, leash reset, respawn), snapshot sync, and `TargetKind::Neutral` for player casts.
- Client rendering for neutrals (sphere mesh) and HP bars consistent with other units; TAB and middle-click target selection includes neutrals.
- Level-based player progression driven by server-authoritative XP thresholds and stat scaling.
- Snapshot propagation of progression fields (`level`, `xp`, `next_level_xp`, `skill_points`) for synchronized client state.
- Local HUD progression readout for level, XP progress, and available skill points.

### Fixed
- Neutral kill XP now uses `grant_player_xp` so jungle rewards level the same way as other XP sources.

### Changed
- Server movement/cast authority: transform packets are clamped by server speed/time/map bounds instead of being trusted as teleports, and cast requests now require authoritative range checks against live player, minion, structure, and neutral target positions.
- Client/server character identity now uses the shared `ekza-bevy-sdk::EkzaCharacter` type while preserving existing snake_case packet values.
- Server: allow `clippy::items_after_test_module` on the binary and `clippy::too_many_arguments` on `simulate_projectiles`; allow `clippy::assertions_on_constants` in `balance` unit tests (keeps `-D warnings` clean for `cargo clippy -p server`).
- Server refactor: split monolithic `server/src/main.rs` logic into focused modules (`progression`, `neutrals`, `world`, `session`) while preserving runtime behavior and test coverage.
- Server runtime loop now runs under a headless Bevy `App` + `ScheduleRunnerPlugin`; mana regeneration was moved to ECS (`Player`/`Health`/`Mana` components and systems) with a sync bridge to the existing authoritative state maps.
- Server ECS combat slice: introduced `server/src/gameplay/combat.rs` and `GameplayPlugin` for message-driven `projectile -> minion` collision/damage resolution (`DamageEvent`), with minion ECS mirroring and legacy minion-hit handling removed from `simulate_projectiles`.

## [0.2.0] - 2026-04-01

### Added
- Implemented TASK-05 player leveling and stat progression, including XP thresholds, level-up scaling for HP/mana, and respawn compatibility with upgraded stats.
- Added progression-oriented server tests covering multi-level XP transitions and respawn behavior after scaling.
- Added client progression ingestion and HUD presentation of progression state.
- **TASK-03**: Full match lifecycle — `Lobby → Running → Victory → (rematch) → Running` state machine on both server and client.
  - Server starts in `Lobby`; match begins when the first player sends a `Join` packet.
  - Victory state blocks all movement and cast input from clients.
  - Auto-rematch after 10 s or immediately on `RequestRematch` packet; resets structures, minions, projectiles, and players without restarting binaries.
  - Client UI: Lobby overlay ("Waiting for match to start..."), Victory overlay with winner text and rematch countdown, no overlay during Running.
  - Added lifecycle state diagram in `.agent/tasks/TASK-03/spec.md`.
- Synchronized agent workflow guidance for Cursor and Claude, including task reuse, repo task proof loop continuation, and mandatory `git worktree` usage for non-trivial isolated work.
- Repository-level documentation rules for changelog maintenance, feature inventory updates, progress logging, and SemVer-based version handling.
- Initial `docs/features.md` and `docs/progress/` structure for ongoing release tracking.
- Added `docs/agents/README.md` with copy-paste prompts for single-task and parallel task execution.
- Standardized agent prompts and repo-facing coordination docs on English for this international project.
- Added project-scoped Cursor subagents in `.cursor/agents/` (`verifier`, `code-reviewer`, `search-agent`, `reasoning-agent`) to improve verification, review, search, and architecture support workflows.

## [0.1.0] - 2026-04-01

### Added
- Initial repository version baseline from `[workspace.package].version`.
