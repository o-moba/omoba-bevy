# Architecture

This page is the map of the repository for people changing it: which crate
owns what, how a frame flows through the client, how a tick flows through the
server, the rules that keep the two in step, and the refactoring roadmap the
maintainers are working through. Feature-level documentation lives in
[features.md](features.md); operations in [RUNBOOK.md](../RUNBOOK.md).

## Crate map

| Crate | Role | Depends on |
| --- | --- | --- |
| `shared` (MPL) | The gameplay model both sides agree on: hero classes and ability kits, hero growth, items, map geometry and navigation, the wire protocol, prematch/draft, social/career/account contracts, the debug command family (`shared::debug`) and the Combat Test sandbox protocol, and the pure bot planners (`shared::progression`, `shop::plan_purchases`). Per-class and per-item data is JSON in `shared/assets/catalog/`, embedded and validated at startup (`shared::catalog`). No Bevy, no I/O in the model itself. | serde |
| `server` (AGPL) | The authoritative simulation and UDP endpoint: match lifecycle, bots, combat, shop, career settlement, public transport signing, allocation workers. Binary only. No Bevy since step 6a; `sqlx` is a dev-dependency (the PostgreSQL fixtures), production reaches the database through career-store. | shared, passport, career-store, ekza-bevy-sdk (no default features), tokio, ed25519-dalek |
| `career-store` (AGPL) | Trusted career persistence (Postgres, migrations) and the bounded queue policy, linked by the server and the account API without the engine. | shared, sqlx |
| `client` (MPL) | The Bevy game: networking, prediction, presentation (2D sprites and 3D models), UI, mobile input, offline practice, QA harnesses (`qa` feature, on by default). | shared, passport, bevy, ekza-bevy-sdk |
| `harness` | Black-box UDP players and gameplay/matchmaking checks that launch the server binary. | shared |
| `passport` | Ekza passport contract: tickets, device and web accounts, store admission. | shared, ekza-bevy-sdk (`http`), reqwest |
| `account-api` | Axum/Postgres HTTP service over the career store (portal, devices, supporter billing). | shared, career-store, axum, sqlx |
| `arena-sync` | CLI that pulls Ekza Arena avatars and merges the avatar manifest. | reqwest |

Rules that follow from the map:

- Anything both client and server must agree on lives in `shared`. That
  includes every wire type; never copy a wire struct into another crate.
- `shared` must stay free of Bevy so the server and tools compile without an
  engine. Client-side ECS components wrap shared types instead.
- Tuning numbers have one home. What differs per class or item (base HP,
  growth caps, basic attack, ability kit, item costs and bonuses) is in
  `shared/assets/catalog/*.json`; the uniform growth curve, speed, mana,
  respawn, XP thresholds and projectile speed are in `shared::hero_balance`;
  facing conventions in `shared::math`; server-only simulation numbers
  (minions, towers, neutrals) in `server/src/balance.rs`; tower stats in the
  map JSON.

## Client frame

`client/src/lib.rs` is the module list and `main`. `main` adds
`DefaultPlugins`, inserts `PlayerVisualMode` once (from
`OMOBA_PLAYER_VISUAL_MODE`), then adds the plugin groups from
`client/src/plugins.rs` in this order:

| Group | Plugins |
| --- | --- |
| `NetPlugins` | `ClientPersistencePlugin`, `NetworkingPlugin`, `MatchServicePlugin`, `CareerIdentityPlugin` |
| `UiPlugins` | `UiKitPlugin`, `MobileControlsPlugin`, `MobileUiPlugin`, `FrontendPlugin` (+ its nine screen and widget plugins), `TeamSelectPlugin`, `GameStateUiPlugin`, `MatchHudPlugin`, `EdgeHudPlugin`, `MinimapPlugin` (+ `MinimapRoutePlugin`), `ShopPlugin`, `HelpOverlayPlugin`, `PauseMenuPlugin`, `SocialPlugin`, `CareerPlugin`, `SupporterPlugin`, `SupporterStoreKitPlugin` |
| `GameplayPlugins` | `MapsPlugin`, `InputContextPlugin`, `PlayerPlugin`, `CombatPlugin` |
| `DebugPlugins` | `SandboxPlugin` (Combat Test panel), `PracticeSandboxPlugin` (pause-menu practice page), `GodModePlugin` (debug HUD toggles and their re-send), `DebugConsolePlugin` |
| `PresentationPlugins` | shared: `CameraPlugin`, `SetupPlugin`, `ModelScalePlugin`, `CombatVisualsPlugin`, `CombatFeedbackPlugin`, `GameVfxPlugin`, `ReactionVisualsPlugin`, `TeamVisionPlugin`, `GameAudioPlugin`, `MapVisualsPlugin`; 2D: `SpriteVisualsPlugin`, `Presentation2dPlugin`, `World2dPlugin`; 3D: `Presentation3dPlugin`, `Verdant3dPlugin`, `DecorPlugin`, `JungleVisualsPlugin`, `MinionVisualsPlugin`, `BossesPlugin`, `ProjectileVisualsPlugin`, `BattlefieldAtmospherePlugin` |
| `QaPlugins` (`qa` feature) | `FrontendQaPlugin` (+ avatar and flow), `VisualQaPlugin` (+ beta UI and navigation), `SocialQaPlugin`, `SupporterQaPlugin`, `TeamVisionQaPlugin`, `AudioQaPlugin`, `OfflineQaPlugin`, `CareerVisualQaPlugin`, `MapQaPlugin`, `CombatQaPlugin`, `ForestPickupQaPlugin`, `TargetingQaPlugin` |

The group order only fixes the order of `Plugin::build`. Three builds read
what an earlier build inserted, and the order keeps them satisfied:
`UiKitPlugin` inserts `UiPlatform`, `MobileControlsPlugin` reads it to set
`MobileControls::enabled`, and `MobileUiPlugin` reads that to decide whether
to register at all; the QA plugins come last, so their
`WinitSettings::continuous()` and `ScreenDriverPaused(true)` overwrite the
production values. Frame order comes from the system sets below, never from
the group order.

QA harnesses live in `client/src/qa/` behind the `qa` cargo feature. It is on
by default, so `cargo run -p client`, packaging, the mobile builds and the
capture scripts compile them as before. Each harness is dormant unless its
environment variable is set: `OMOBA_ANIMATION_QA` (runs instead of the game),
`OMOBA_VISUAL_QA_DIR` (with `OMOBA_VISUAL_QA_SCENARIO` = `beta-ui`,
`navigation`, `targeting`, `combat`, `map`, `forest-pickups` or
`team-vision`), `OMOBA_FRONTEND_QA_OUTPUT` (with `OMOBA_AVATAR_QA` or
`OMOBA_FRONTEND_QA_FLOW`), `OMOBA_SOCIAL_QA_OUTPUT`, `OMOBA_AUDIO_QA_OUTPUT`,
`OMOBA_OFFLINE_SMOKE_DIR`, `OMOBA_CAREER_QA_OUTPUT`,
`OMOBA_SUPPORTER_QA_DIR`, and `OMOBA_SANDBOX_QA_OUTPUT` (the Combat Test
panel harness, which stays in `sandbox/ui/qa.rs` because it drives the
panel's private types). `cargo clippy -p client --lib --no-default-features`
(`make check-no-qa`, also in CI) keeps the build without them free of
warnings. Production items that only the harnesses and tests read are
`#[cfg(any(test, feature = "qa"))]`, and the ones only the harnesses read
are `#[cfg(feature = "qa")]`.

The `Update` schedule is ordered by a small number of system sets; read
these together to understand frame order:

1. `ClientNetPipeline` (`client/src/net/mod.rs`): ingest UDP snapshot
   fragments, apply the authoritative snapshot to entities, age utility
   timers, interpolate remote entities and players. The `net` module is
   split by stage: `transport` (UDP thread, framing, decode, channels),
   `session` (`ClientSession`, join retries, teardown, reconnect),
   `commands` (`NetworkCommand` to packets), `ingest` (drain the channel,
   stage one snapshot per frame), `apply` (snapshot to ECS), `interpolate`
   (remote poses, grounding), `components` (replicated components and
   `GameStateSnapshot`), `status_ui` (connection panel), `offline`
   (practice simulation) and `public_transport` (signed public datagrams).
   `mod.rs` re-exports the public surface, so other modules keep importing
   `crate::net::X`.
2. `InputContextSet::{Social, Modal, Resolve, Actions}`
   (`client/src/input_context.rs`): modal UI (pause menu, shop, career,
   social, help) runs first and decides whether gameplay input is allowed;
   `Actions` holds movement (`WorldMovementInputSet`), targeting and casting
   (the combat chain in `client/src/combat/`, pointer picking in
   `CombatPointerInputSet`) and mobile controls. Both input sets are
   defined in `input_context.rs`.
3. `ClientNetPipeline::SendLocalState` and `SendCommands` after `Actions`:
   the local transform and the queued `NetworkCommand`s become packets.
4. `PostUpdate`: grounding, target presentation, UI layout adjustments.

Snapshot application is a chain of `SnapshotApply` stages
(`client/src/net/mod.rs`, systems from `apply::snapshot_apply_systems()`),
each a member of `ClientNetPipeline::ApplySnapshot`, so every
`.after(ClientNetPipeline::ApplySnapshot)` reader still sees the whole
application, including the Commands of every stage (Bevy applies them at a
sync point after each stage, so a later stage sees what an earlier one
spawned):
- `Begin` moves the frame `ingest` staged into `StagedSnapshot`.
- `Session` marks the session connected and records the snapshot time.
- `Resources` writes `GameStateSnapshot`, the round and the prematch
  loadout, and sets the Draft gate. It runs even when the entity stages
  are skipped.
- `LocalPlayer` (queries: `Player` ids, the hero `Transform` `With<Player>,
  Without<MainCamera>`, the camera `Transform` `With<MainCamera>,
  Without<Player>`): during Draft it despawns every hero, local and remote;
  otherwise it keeps one local `Player`, applies the server's state (dash
  acknowledgement, position snap) or spawns the hero and locks the camera.
  A listed local hero without a team or a committed join closes the gate
  (`LocalPending`) instead.
- `RemotePlayers`, `Projectiles`, `Structures`, `Minions`, `Neutrals` run only
  while the gate is open (`Full`), each with its own filtered queries
  (`With<RemotePlayer>`, `With<NetworkProjectile>`, ...). Dash VFX are written
  local first, then remote.
- `Finish` writes `SnapshotApplied { meta, your_id, round, outcome, local }`
  (`outcome`: `Full`, `Draft` or `LocalPending`; `local`: `Unchanged`,
  `Updated { entity, corrected, dashed }`, `Spawned { entity, position,
  team }` or `Cleared`) and clears the staged frame. Nothing outside the
  tests reads it yet.

Session lifecycle edges are `SessionEvent` messages: `TransportStarted`,
`Connected`, `Joined`, `Rejected`, `JoinExhausted`, `Disconnected`, `Left`,
`ServerScopeReset` and `RoundChanged`. `net` queues each one in
`ClientSession`'s outbox where the edge happens (in plain functions and in
the systems at the 16-parameter limit alike). `flush_session_events` writes
the queue as messages at the end of `ApplySnapshot` and again at the end of
`SessionLifecycle`, after `retry_pending_join`. Readers:

| Event | Reader | Where it runs |
| --- | --- | --- |
| `RoundChanged` | `combat::round_reset::reset_round_input_state` (drops targets, pending casts, cooldowns, queued gameplay commands, move orders) | after `ApplySnapshot`, before `InputContextSet::Modal` |
| `RoundChanged` | `mobile_controls::read_mobile_controls` (releases held fingers) | `MobileControlsSet::Input`, after `InputContextSet::Resolve` |
| `ServerScopeReset` | `career::clear_account_on_scope_reset`, `social::clear_on_scope_reset` | `SessionReactions` |
| `Left` | `frontend::return_home_on_leave` (`PendingScreen(Home)`) | `SessionReactions` |

`SessionReactions` runs after `SessionLifecycle`, so a reaction sees the
event in the frame it is written and finishes before the next frame's ingest
(a career view that arrives right after a scope reset lands on a cleared
client). `RoundChanged` is flushed at the end of `ApplySnapshot`, so its
readers sit right after that set and clear the old round before this
frame's input and `SendCommands`. `net` keeps three outside writes in
`update_session_lifecycle` on purpose: `TeamSelection.team = None` (join
intent), and for `LeaveMatch` the lobby address from
`MatchServiceClient::take_return_to_lobby` and the `CareerIdentity` signature
on `CancelQueue`. Code that needs the current state keeps polling
`ClientSession`. Its fields are private to `net`; outside code reads
`state()`, `is_connected()`, `is_offline()`, `join_confirmed()`,
`join_in_flight()` (a join packet was sent), `has_committed_join()`,
`joined_prematch()`, `join_blocked()`, `join_rejection()`,
`is_choosing_loadout()` and `server_addr()`, and writes only through
`abandon_join()`. Tests build sessions with the `*_for_test` constructors
and setters. The round pollers in `shop.rs`, `edge_hud.rs`, `sandbox/mod.rs` and
`frontend/draft.rs` keep comparing `GameStateSnapshot.meta`, because they
also react to the zero ids a teardown leaves.

Gameplay input and local prediction live in two module trees that follow
the `net` pattern: `mod.rs` holds the plugin and re-exports what other
modules import, so callers keep `crate::combat::X` and `crate::player::X`.
- `client/src/combat/`: `cooldown` (local cast cooldown mirror), `feedback` (action line), `round_reset` (clear intents on `SessionEvent::RoundChanged`), `selection` (`TargetState`, pointer and nearest-enemy picking), `cast` (`PendingCast`, slot casts, approach), `mobile` (mobile cast and utility), `hotbar` (skill bar UI), `bars` (world HP/mana bars), `marker` (target ring), `targeting` (basic attacks, aim UI, locked target; still reachable as `crate::targeting`).
- `client/src/player/`: `input` (desktop and mobile movement input, route planning, viewport picking), `motion` (local motion, jump, gravity, collisions), `animation` (hero animation library, binding, playback, sandbox seek; `register_hero_animation_systems`), `respawn_ui` (respawn countdown).

Presentation is chosen once per run by `PlayerVisualMode` (3D models by
default, 2D sprites with `OMOBA_PLAYER_VISUAL_MODE=sprite2d`). Simulation
positions are always XZ on the ground plane; 2D rendering maps them to XY.
Every backend plugin is always added, because some 2D resources
(`SpriteVisualAssets`, `MapVisualRegistry`) are read in both modes; the
presentation backends run under `sprite::in_models3d()` /
`sprite::in_sprite2d()` (`resource_exists_and_equals`, so a missing mode
runs neither). 2D: sprite proxies, `presentation2d`, `world2d`. 3D:
`presentation3d`, Verdant, jungle, minions, bosses, decor, the map-visual
prop chain and river repair, projectile models, and the model/lighting
systems in `world.rs`. Systems that branch on the mode internally (camera,
snapshot apply, combat bars, targeting, VFX, minimap, team vision,
projectile trails, player movement) and the screen-space battlefield mist
run in both modes. The gated systems keep their own mode checks because
tests register them directly and flip the mode.

Client-side model types that `net`, gameplay and presentation share live
in `client/src/domain/`: `RoundId` (`server_epoch` + `match_id`, `None`
for zero), `Team` with its `shared::map::Team` bridges, `CombatStats` with
`MAX_HP`, the hero markers `Player`, `PlayerBody`, `VerticalVelocity`,
`RemotePlayer`, and the movement intents `MovementTarget`/`MovementRoute`.
The old paths (`crate::team::Team`, `crate::combat::CombatStats`,
`crate::player::Player`, `crate::net::RemotePlayer`, ...) re-export them.

The offline practice playground (`client/src/net/offline.rs`) is a
socket-free simulation that speaks the same packets through the same channels
as the UDP transport, so the rest of the client does not know it is offline.

## Server tick

`ServerRuntime` (`server/src/runtime/mod.rs`) owns its three ports, the
sub-runtimes (bots, prematch, career, social, sandbox) and one `GameWorld`
(`server/src/game_world.rs`): the entity maps (players, disconnected
sessions, projectiles, structures, minions, neutrals), team buffs, forest
pickups, the game state, the map layout and config, the id allocators and
the wave clock. Simulation functions and request handlers take
`&mut GameWorld` plus a `TickCtx { now, dt }` (or just `now`) instead of a
parameter per map; only the leaf damage helpers (`apply_*_damage`) still take
individual maps because they run inside loops that hold other fields.
`main.rs` is the module list plus `fn main`; it re-exports nothing.
Modules import what they use (`use crate::entities::ConnectedPlayer;`,
`use shared::wire::GameState;`, `use std::time::Instant;`); there are no
crate-root glob re-exports and no `use crate::*;`, and only test modules
keep `use super::*;` for their parent module. `runtime::run` is a plain
paced loop (`SIMULATION_STEP_SLEEP`, 10 ms): `prepare_tick`, then
`tick`, then sleep the remainder of the step. It is not a fixed timestep:
`prepare_tick` passes the real elapsed time as `dt` (capped at 100 ms), so a slow step makes
the next `dt` longer (the Combat Sandbox can scale or pause it). There is no Bevy `App` and no
ECS mirror on the server; the `GameWorld` maps are the only copy of the
state.

The ports (`server/src/runtime/ports.rs`, `server/src/career_port.rs`) are
the runtime's only I/O: `transport: Box<dyn Transport>` (`recv`, `send_to`,
`local_addr`; non-blocking, `WouldBlock` ends the receive loop) carries
every datagram in and out, `clock: Box<dyn Clock>` (`now`) is the tick
path's time source, and `career.backend: Box<dyn CareerPort>` is the
account and result store. The process runs `UdpTransport`, `SystemClock`
and `career_backend::CareerBackend` (the signed-account state machine over
a bounded job channel to the PostgreSQL worker thread); tests run
`MemoryTransport` (queued inbound and captured outbound datagrams),
`ManualClock` (advanced by the test) and `career_backend::MemoryCareer`
(the same state machine over an in-memory job link that acknowledges
immediately, on request through the `test_*` hooks, or not at all when
built disabled). `ServerRuntime::with_ports` is the one constructor;
`new_with_map`/`new(socket, config)` wrap it with the process ports and
`for_test` with the memory ones. `prepare_tick`, the receive loop, the
admission completions, the sandbox's roster and snapshot throttles and the
constructor read `clock.now()`; the leaf helpers take `now` as a parameter
and the Combat Sandbox's virtual clock starts from the injected clock and
advances by its own scaled `dt`. A tick is:

1. `ServerRuntime::prepare_tick`: `runtime::dispatch::receive_packets` decodes
   datagrams; public roles verify signed commands first (`public_transport`).
   `handle_packet` admits identity (career, social and prematch requests are
   routed there), then `handle_packet_authorized` runs the ordered
   pre-checks (allocated roster, join normalisation, the career session
   gate, career and practice join admission) and one `match` that is the
   whole dispatcher: `Leave`, the career-flow `RequestRematch`, `Practice`
   and `Sandbox` first, then a paused sandbox swallowing movement and
   combat, then one handler per variant in `runtime/handlers/` (`join.rs`,
   `movement.rs`, `combat.rs`, `utility.rs`, `shop.rs`, `session.rs`, and
   `tools.rs` for the Combat Test `Sandbox`; each
   `ServerRuntime::handle_<variant>`). The debug packets (`Practice`,
   `SetGodMode`, `SetSpeedBoost`) keep their arms and become a
   `DebugCommand` for `ServerRuntime::handle_debug` (see "Debug commands"
   below). A handler returns `ControlFlow<()>`: `Continue` runs the
   dispatcher's post-command tail (endpoint touch, sandbox roster, practice
   bots, prematch, round start, career registration), `Break` skips it.
   The wall-clock `dt` is clamped to 100 ms and, in the Combat Sandbox,
   replaced by the sandbox's virtual clock.
2. `ServerRuntime::tick(now, dt)` (`runtime/tick.rs`): mana regeneration
   (`sim::regenerate_mana`), the minion-targeted projectile pass, then
   formation (`formation`), bots, and the `sim` modules (`minions`, `towers`,
   the remaining `projectiles`, `neutrals`, plus regeneration and respawns in
   `sim/mod.rs` and `session`); it records combat receipts into the round
   ledger, and `snapshot::broadcast_snapshots` builds one snapshot per
   recipient (vision filtered) and sends it. Projectile flight is one
   function, `sim::projectiles::simulate_projectiles_filtered`, run twice with
   a target-kind filter only to keep the minion pass at the point in the
   frame where the old ECS systems ran; folding it into one pass is the
   next slice.

Hero state and views: the server never stores the wire `PlayerState`.
`ConnectedPlayer` (`entities.rs`) holds `hero: Hero` (`server/src/hero.rs`:
`HeroIdentity` for id, bot flag, team, class, character, avatar, sprite and
supporter aura, set at join and never by the simulation; position, yaw, HP
and mana; `HeroProgress` for XP, level, skill points and ranks;
`HeroUtility` for the utility request marks; `HeroAction`, the last accepted
cosmetic action), `economy: HeroEconomy` (gold, earned gold, inventory, item
bonuses, last purchase receipt, the basic-attack and purchase request marks
and the passive-income remainder) and `timers: HeroTimers`
(`server/src/hero_timers.rs`: last movement, per-slot casts, last basic
strike, dash and haste readiness, haste expiry, respawn), next to the
transport and admission fields. `hero_timers` also holds the pure reads over
the instants (`basic_attack_remaining`, `skill_cooldown_remaining`,
`skill_recovery_remaining`, `dash_remaining`, `haste_remaining`,
`haste_active`), used as gates by the request handlers and by the sandbox
telemetry, and `normalize_hero_timers`, the one place that clears instants
from derived conditions (death, `no_cooldowns`), run once per tick after
respawns. `modifiers: StatModifiers` (`server/src/hero_stats.rs`) is the
one overlay on top of class, level and gear: multipliers (damage, attack
speed, move speed), flat armor and resistance, an optional base-HP override,
and the rule flags (`god_mode`, `infinite_hp`, `infinite_resource`,
`no_cooldowns`, `unlock_all`, `bypass_vision`, `grant_xp`, `respawns`);
`Default` is normal play. The development toggles write `god_mode`
(with `infinite_resource`) and `move_speed_mult`; the Combat Sandbox's
`apply_actor` converts an `ActorConfig` into modifiers plus the hero's
loadout once and keeps the config only to echo it in the telemetry. The
simulation never asks whether a hero is a sandbox actor; it reads the flag
it needs. `hero_stats` also holds the effective-stat formulas
(`combat_bonuses`, `basic_attack_damage`, `basic_attack_cooldown`,
`ability_cooldown`, `skill_recovery`, `move_speed`, `movement_envelope`,
`max_hp`, `max_mana`, `mitigate`), used by the request handlers, the timers,
the bots and the sandbox telemetry. `PlayerState` is built only by the
views: `ConnectedPlayer::owner_view(now, map, phase)` maps `hero` and
`economy` onto the wire struct and fills the cooldown, utility-clock and
shop fields at the tick's `now`; `public_view` is the owner view with the
private economy blanked (gold, earned gold, inventory, item bonuses, the
purchase receipt) and the owner's request marks zeroed (basic-attack and
utility request ids). `snapshot::build_players_snapshot(world, recipient,
now)` builds the recipient's own entry through `owner_view` and every other
player, teammates included, through `public_view`; the broadcast calls it
per recipient, in the sandbox too. `server/src/tests/player_view.rs` pins
the mapping and both views' bytes through a hero lifetime;
`vision/tests.rs` pins the per-recipient redaction.

Supporting modules: `entities` (the server-side records), `sim/cast.rs`
(ability casts), `vision` (server-owned sight, takes `&GameWorld`). Unit
tests for these live under `server/src/tests/`.

Bots are ordinary `ConnectedPlayer`s on unspecified IPv6 addresses; their
addresses never accept network commands. Practice, development and release
modes differ in team assignment, match start, bot filling, debug commands
and career credit; `match_rules::MatchRules` is the one place that turns the
mode into decisions. `MatchConfig` is the parsed input (`OMOBA_MATCH_MODE`,
`OMOBA_TEAM_SIZE` or the worker manifest); `MatchRules::for_mode` derives
one plain value per decision (`team_assignment`, `start`, `prematch_roster`,
`fills_with_bots`, `debug_commands`, `combat_sandbox_allowed`,
`career_credit`, `local_results`, `career_flow`) and `ServerRuntime.rules`
holds it. The dispatcher, formation, bots, prematch, sandbox and career
code read the field they need; a rule that combines with a runtime
condition (a worker allocation, the sandbox being enabled, a
prematch-capable join) keeps that condition at the site. The table of
values per mode is pinned by `match_rules::tests::rules_table_per_mode`.

## Debug commands

Developer and practice commands form two families, kept apart on purpose.

- **The debug command family** (`shared/src/debug.rs`):
  `DebugCommand { GodMode(bool), SpeedBoost(bool), Practice(PracticeCommand) }`
  is an in-process type without serde derives. `to_packet` and
  `from_packet` map it onto the `SetGodMode`, `SetSpeedBoost` and `Practice`
  packets, which stay its wire encoding forever: `ClientPacket` rejects
  unknown tags, so a new `{"type":"debug"}` would be dropped by an old
  server. `PracticeCommand` decodes an unknown `kind` as `Unsupported`,
  which every host ignores, so new practice commands are additive.
  `DebugAccess { toggles, practice }` says what a match accepts;
  `DebugAccess::for_match_mode` derives it from the snapshot's `match_mode`
  (`dev`: the toggles; `practice` and `offline_practice`: both; anything
  else: neither). `DUMMY_MAX_HP`, `DUMMY_DISTANCE` and
  `OFFLINE_PRACTICE_MODE` live here too; `shared::practice` keeps the
  command type and the duel limits and is re-exported from `shared::debug`.
- **Server** (`server/src/debug/`): `ServerRuntime::debug_access()` is
  `toggles: rules.debug_commands`, `practice: rules.fills_with_bots`, both
  only without a worker allocation (a worker round fixes its durable
  ruleset at round start). A test pins it to
  `DebugAccess::for_match_mode(rules.mode_id())` for every mode.
  `ServerRuntime::handle_debug(addr, command, now)` is the one entry point:
  `toggles.rs` writes `god_mode` (plus `infinite_resource` outside Combat
  Test) and `move_speed_mult`; `practice.rs` restores the roster, clears the
  bots, spawns a dummy or starts a duel, on top of the roster primitives in
  `bots.rs` (`spawn_bot`, `remove_bot`, `remove_all_bots`, `place_dummy`).
  The dispatcher converts the packet with `DebugCommand::from_packet` in the
  arms the packets always had: `Practice` before the paused-sandbox gate on
  the wall clock (always `Break`, and it touches the endpoint even when
  refused), the toggles after it on the simulation clock (`Break` when
  refused or not joined, `Continue` once applied).
- **Offline** (`client/src/net/offline.rs`) implements the same commands in
  its own simulation, through one `Simulation::debug(DebugCommand)` fed by
  `DebugCommand::from_packet`: a ring roster instead of lane bots, god mode
  as a local flag, the speed boost an explicit no-op (offline accepts any
  finite client transform, so there is no clamp to raise).
- **Bot planners** for every host that levels or equips a hero without a
  player: `shared::progression::skill_upgrade_order` (the ultimate first
  once unlocked, then Q, W, E) and `shared::shop::plan_purchases` (greedy
  in the class's recommended order). Server lane bots and the practice
  duelist apply them through the ordinary upgrade and purchase paths
  (`bots::auto_rank_skills`, `bots::auto_shop`); the offline duel and the
  harness bots (`bot_ai::choose_shop_item`) use the same plans.
- **Combat Test** (`shared::sandbox`, `server/src/sandbox.rs`,
  `runtime/handlers/tools.rs`) is its own protocol: acknowledged,
  sequenced and scoped to an epoch, applying a whole `SandboxConfig` to
  fixed actors on a virtual clock, dev and loopback only. Offline cannot
  host it.
- **Client** (`client/src/debug/`, the `DebugPlugins` group):
  `DebugToggles { god_mode, speed_boost }` is the one client copy of the
  toggles. The debug HUD (`hud.rs`: F2/F3 and the two buttons), the
  pause-menu practice page (`tools_page.rs`), local movement and the
  local-player snapshot stage (snap threshold while boosting) read it.
  Every debug command is `NetworkCommand::Debug(DebugCommand)`, encoded
  with `to_packet()` once the join is confirmed. `resend_debug_toggles`
  re-sends both toggles every 0.5 s, only while the HUD is enabled
  (`OMOBA_DEBUG_UI`, not in Combat Test). The practice page shows when
  `DebugAccess::for_match_mode(match_mode).practice`, and leaving a
  practice match clears `god_mode`. `console.rs` is the on-screen log.
  One tools page driven by `DebugAccess` everywhere toggles are allowed
  (dev without the env var, practice re-send) is slice 11e, a behaviour
  change that waits for the owner.

## Protocol rules

- `PROTOCOL_VERSION` (`shared::protocol`) is bumped only for incompatible
  changes; peers with another version are rejected at `Hello`.
- Compatible changes are additive: new fields carry `#[serde(default)]`,
  new enum values are only added where the decoder tolerates unknown values.
- Gameplay commands that must not replay (`BasicAttack`, `Utility`,
  `BuyItem`) carry `server_epoch`, `match_id` and a monotonic `request_id`.
  `Cast` carries none: a replayed cast is refused by the slot's cooldown,
  skill recovery and mana. Transforms carry `dash_sequence`, and the distance
  a transform may cover is a time budget (speed × elapsed time plus at most
  one `MOVEMENT_POSITION_TOLERANCE` of unspent slack,
  `HeroTimers::movement_slack`), not a per-packet allowance, so sending
  transforms faster does not move a hero faster.
- Snapshots are trimmed to the UDP payload limit by dropping the oldest
  cosmetic combat events, never gameplay state.
- A player's private economy (gold, earned gold, inventory, item bonuses,
  purchase receipt) and request marks (basic-attack and utility request ids)
  are replicated only to that player; every other recipient, teammates
  included, gets them at their serde defaults. Level, XP, ranks, HP, mana
  and the cooldown copies are public. The scoreboard's earned gold comes
  from the round ledger, not from `PlayerState`.

## Adding content

Hero and item data lives in two JSON files, `shared/assets/catalog/heroes.json`
and `shared/assets/catalog/items.json`. `shared::catalog` embeds them in both
binaries, parses them once and validates them; `catalog::ensure_loaded()` runs
at the top of the server's `runtime::run` and the client's `main`, so a bad
file stops the process at startup with the file and the entry that failed.
There is no runtime override: client and server must agree. The rule of thumb
is that what differs per class or per item is data, and what is the same for
everyone (rank scaling, slot unlock levels, the growth curve and the global
values in `shared::hero_balance`, starting gold, inventory capacity, shop
radius) is code.

- **Tuning a class or an item:** edit the JSON only. `heroes.json` lists the
  classes in `HeroClass::ALL` order, each with `id`, `display_name`,
  `tagline`, `role`, `base_hp`, `growth` (`basic_damage_cap`,
  `attack_rate_cap`), `basic_attack` (`range`, `damage`, `cooldown_secs`),
  `projectile_style`, four `abilities` in Q/W/E/R order (`id`, `name`,
  `description`, `targeting`, `mana_cost`, `cooldown_secs`, `cast_range` and
  exactly one of `projectile_damage`, `self_heal`, `self_mana_restore`) and
  `recommended_items` (every item exactly once, in buying order).
  `items.json` lists the items in `ItemId::ALL` order with `id`, `name`,
  `description`, `cost` and `bonuses` (a partial `ItemBonuses`; missing
  fields are neutral). Unknown fields are rejected. `cargo test -p shared`
  runs the catalog checks (enum coverage and order, kit shape, value ranges,
  starter budget). The Python tools read the same files through
  `scripts/catalog.py`.
- **Hero class** still needs code: a `HeroClass` variant, its place in
  `HeroClass::ALL` and its `id` in `shared/src/lib.rs`, plus its
  `heroes.json` entry at the same position; bot composition and bot avatars
  in `server/src/bots.rs`; the client art: the skill atlas and its order in
  `client/src/skill_icons.rs`, a class entry in
  `client/assets/config/combat_visuals.json`, the minimap letter in
  `client/src/minimap.rs`, and audio cues. A new projectile look needs a
  `ProjectileStyle` variant and its visuals. Behaviour beyond the three
  ability effects (like the Warden's jungle passive in `shared/src/jungle.rs`)
  is code. Client and server ship together; an old client decodes an unknown
  class as Warrior.
- **Item** still needs code: an `ItemId` variant, its place in `ItemId::ALL`
  and its `id` in `shared/src/shop.rs`, plus its `items.json` entry at the
  same position, a place in every class's `recommended_items`, and its short
  code in `client/src/shop.rs` (`item_code`). The number of items is
  independent of `INVENTORY_CAPACITY`.
- **Map:** structure placement and stats in a map JSON
  (`docs/map-customization.md`); arena geometry and collision are fixed.

## Roadmap

Ordered by value over cost. Each step is a separate change with the full
`make check` gate green.

1. CI gate, pinned toolchain, `make check` (done).
2. One wire protocol in `shared`, golden JSON tests, harness on shared types
   (done).
3. Balance constants and the hero facing convention in one place (done).
4. Crate hygiene: retire the orphan `skills` crate, move the career store out
   of the server package (done); keep I/O out of the shared model (the avatar
   and sprite rosters still embed client manifests and read env vars; open).
5. Server `GameWorld` + tick context instead of many-map parameters; split
   `main.rs` into runtime (dispatch, tick), snapshot, formation, entities,
   ECS and simulation modules (done); per-variant packet handlers in
   `runtime/handlers/` and explicit imports instead of the crate-root glob
   re-exports (done, step 14). Step complete.
6. One server tick (done: the Bevy `App`, the ECS mirror of players and
   minions, the duplicate mana regeneration and the ECS-only minion
   projectile path are gone; `ServerRuntime::tick` is the whole step);
   replicated `PlayerState` as a view over authoritative state and
   `HeroTimers` (done: `hero_timers.rs`, `owner_view`/`public_view`, no
   per-tick copies); authoritative hero state in server-owned structs (done:
   `hero.rs` with `Hero`, `HeroIdentity`, `HeroProgress`, `HeroUtility`,
   `HeroAction` and `HeroEconomy`; `ConnectedPlayer.state` is gone and the
   views are the only place that builds a `PlayerState`); `StatModifiers`
   and the effective-stat formulas in `hero_stats.rs` (done: the sandbox
   overlay, the debug toggles and the three max-pool computations are one
   overlay and one set of formulas); snapshot redaction through
   `public_view` (done: non-owners receive the private economy and request
   marks blanked). Step complete.
7. Match rules as one policy object (done: `match_rules.rs`, `MatchRules`
   derived once from the mode, no `mode ==` outside the startup banner);
   career, transport and clock behind traits (done: `CareerPort`,
   `Transport`, `Clock` with the process and the in-memory implementations,
   `ServerRuntime::with_ports`/`for_test`). Step complete.
8. Client `net.rs` split into transport, session, commands, ingest, apply
   and interpolation (done, verbatim moves under `client/src/net/`);
   session events instead of cross-module writes are step 15 (done:
   `SessionEvent` with the outbox and its flush, `apply_server_snapshot`
   split into the `SnapshotApply` stages (one per entity kind) with
   `SnapshotApplied`, and the first consumers (the combat and mobile round
   resets on `RoundChanged`; career, social and the front end in
   `SessionReactions`), and the `ClientSession` accessors. Step complete;
   the optional slices 15f and 15g are listed in
   `docs/plans/client-10-15.md`).
9. One UI kit (theme, widgets, gestures, scroll, actions) and a modal
   registry (pilot done: `client/src/ui/` with theme, tap recognizer, typed
   actions and widgets; the pause menu and the practice sandbox use it;
   remaining steps in `docs/ui-kit.md`).
10. Client domain module, combat/player split, render backends behind
    `run_if`, plugin groups, QA behind a cargo feature (done: the domain
    module and the `in_models3d`/`in_sprite2d` backend gates; `combat.rs`
    and `player.rs` split into `client/src/combat/` and
    `client/src/player/`, verbatim moves; the `NetPlugins`, `UiPlugins`,
    `GameplayPlugins`, `PresentationPlugins` and `QaPlugins` groups in
    `client/src/plugins.rs`; the QA harnesses in `client/src/qa/` behind
    the default-on `qa` feature, with a no-`qa` clippy in CI). Step
    complete; the optional slices (store builds without QA, migrating
    imports off the re-export shims) are listed in
    `docs/plans/client-10-15.md`.
11. One debug tooling family shared by Combat Test, practice and offline
    (in progress: the toggles refuse worker-allocated rounds; `shared::debug`
    with `DebugCommand` and `DebugAccess`, the server's `debug/` module with
    `handle_debug` and `debug_access`, and the shared bot planners
    `skill_upgrade_order` and `plan_purchases` are done; so is the client
    `debug/` module with `DebugToggles`, `NetworkCommand::Debug` and
    `DebugPlugins`. One tools page driven by `DebugAccess` (11e) widens
    where the page and the re-send appear and waits for the owner's
    decision).
12. Data-driven hero and item catalogs (done: `shared/assets/catalog/`
    `heroes.json` and `items.json`, loaded and validated once by
    `shared::catalog`; the accessors keep their names and signatures, the
    Rust tables are gone, the item count is independent of the inventory
    capacity, and the Python scripts read the same files through
    `scripts/catalog.py`). Step complete.
