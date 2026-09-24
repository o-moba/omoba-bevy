# Refactoring tracker

This is the hand-over page for the architecture refactoring programme. Anyone
(a person or an agent) picking the work up should be able to continue from
this file alone: what is done, what is next, how each step is delivered, and
where the detailed plans live. Update it in the same pull request as the
step it describes.

Architecture report (before vs after, current architecture, verified improvement list O1–O30 and Q1–Q9): [ARCHITECTURE_REPORT.md](ARCHITECTURE_REPORT.md).
Roadmap source: the "Roadmap" section of [ARCHITECTURE.md](ARCHITECTURE.md).
Per-step notes: [progress/](progress/) (one dated note per merged step).
Release notes: `## [Unreleased]` in the root `CHANGELOG.md`.

## Process rules (agreed with the maintainer)

1. Every refactoring step lands through a pull request into `main`; never push
   refactoring commits to `main` directly.
2. Open the PR, then merge it immediately (the maintainer reviews on `main`);
   `merge` or `squash`, whichever gives one readable commit per step.
3. Before opening a PR the full gate must be green locally:
   `make check` (= `cargo fmt --all -- --check`,
   `cargo clippy --workspace --all-targets --no-deps -- -D warnings`,
   `cargo clippy -p client --lib --no-deps --no-default-features -- -D warnings`,
   `cargo test --workspace --locked --exclude harness`, Python script tests)
   plus the black-box harness against the freshly built server:
   `cargo build -p server && cargo test --locked -p harness -- --test-threads=1`
   (about five minutes; it is the only check that exercises a real match end
   to end, so never skip it for a server or protocol change). Reference
   counts at the time of writing: server 283 (+3 ignored), shared 88, client
   lib 561 (541 with `--no-default-features`, i.e. without the QA
   harnesses), harness 22 unit + 24 black-box.
4. One step per branch, named `refactor/<topic>` (docs-only: `docs/<topic>`),
   cut from the current `origin/main`.
5. No wire-visible change unless the step says so explicitly; `shared/` stays
   untouched in server-only and client-only steps. Snapshot bytes are pinned by
   `server/src/tests/player_view.rs` and the golden JSON tests in
   `shared/src/protocol/wire.rs`.
6. After merging, look at the CI run on `main` (Actions tab) and treat a red
   run as work now, even if the local gate was green.
7. Each PR updates: the roadmap line in `ARCHITECTURE.md`, a `CHANGELOG.md`
   entry under `## [Unreleased]`, a `docs/progress/<date>-<topic>.md` note,
   and the table below.
8. Keep diffs mechanical where they are large (renames, moves); behaviour
   changes go in their own small PR with a test that pins them.

## Status

| Step | Scope | State | PRs |
| --- | --- | --- | --- |
| 1 | CI workflow, pinned toolchain (1.94.1), `make check` | done | (direct, before the PR rule) |
| 2 | One wire protocol in `shared::protocol::wire`, golden JSON tests, harness on shared types | done | #18 |
| 3 | Balance constants (`shared::hero_balance`) and facing convention (`shared::math`) | done | #19, #20 |
| 4 | Crate hygiene: `career-store` crate, `skills` crate removed | done (shared I/O isolation open, see step 13) | #21 |
| 5 | Server `GameWorld` + `TickCtx`, `main.rs` split into runtime/sim/snapshot/entities | done | #22 |
| 8 | Client `net.rs` split into `client/src/net/` modules | done (session events are step 15) | #23 |
| 9 | Client UI kit pilot: theme, gesture, `UiAction`, widgets, `TestId`; pause menu + practice + career migrated | pilot done, rest is step 9b | #24 |
| 6a | Server one tick loop, one mana regen, one projectile path, ECS mirror removed | done | #25 |
| 6b | `PlayerState` as a view (`owner_view`/`public_view`), `HeroTimers`, `Hero`/`HeroEconomy` structs | done | #26, #27 |
| 6c | `StatModifiers` + `hero_stats.rs`; non-owner redaction of private economy on the wire (wire-visible, no version bump) | done | #29 |
| 7 | Server `MatchRules` policy object; career, transport and clock behind traits | done | #31, #32 |
| 14 | Server per-variant packet handlers, explicit imports instead of crate-root globs | done | #33 |
| 10 | Client domain module, combat/player split, render backends behind `run_if`, plugin groups, QA behind a cargo feature | done (optional 10h/10i open, see [plans/client-10-15.md](plans/client-10-15.md)) | #35, #36, #37 |
| 11 | One debug tooling family shared by Combat Test, practice and offline | pending | |
| 12 | Data-driven hero and item catalogs with validation tests | done: 12a-12e (this PR); 12f (optional client cross-checks) open | this PR |
| 13 | Roster/asset loading and SDK types out of the shared model | pending | |
| 15 | Client session events and staged snapshot application | in progress: 15a+15b1 (#39), 15b2+15c+15d (this PR) | #39, this PR |
| 9b | UI kit follow-ups: scroll unification, modal registry, frontend/social/supporter/sandbox screens, responsive layout, `TestId` in QA | pending (order in [ui-kit.md](ui-kit.md)) | |

Suggested order after 7: 14 (done), 10 (done), 15, 11, 12, 13, 9b (server first while
its structure is fresh, then the client). Each row is one to four PRs.

## How to continue

1. `git fetch origin main && git checkout -b refactor/<topic> origin/main`.
2. Read the roadmap line in `ARCHITECTURE.md`, the plan for the step below and
   the latest `docs/progress/` note that touched the same area.
3. If the step is large, first get a read-only plan (file/line inventory of
   readers and writers, ordered PR-sized slices, risks), then build one slice
   at a time; keep `cargo test -p server` (or `-p client --lib`) green after
   every slice.
4. Run the gate, commit with a message that names the step, push the branch,
   open the PR (body: summary, behaviour notes, test plan), merge it, update
   this table.

## Plans for the open steps

### 6c: StatModifiers and redaction (done, #29)
- `server/src/hero_stats.rs`: `StatModifiers { damage_mult, attack_speed_mult, move_speed_mult, armor, resistance, base_max_hp, god_mode, infinite_hp, infinite_resource, no_cooldowns, unlock_all, bypass_vision, grant_xp, respawns }` with `Default` = normal play; the single formulas for combat bonuses, basic damage/cooldown, ability cooldown, skill recovery, move speed, `max_hp`, `max_mana` (moved out of `sandbox.rs`). `sandbox::apply_actor` converts `ActorConfig` into modifiers + loadout once; `sandbox.is_some()` gates become flags; `god_mode`/`speed_mult` fields fold into the modifiers.
- Redaction: `public_view` blanks `gold`, `earned_gold`, `inventory`, `item_bonuses`, `last_purchase`, `basic_attack_request_id`, `utility.last_request_id` (all `#[serde(default)]`, no protocol bump); `build_players_snapshot` uses `owner_view` for the recipient and `public_view` for everyone else; vision tests pin it. Scoreboard gold comes from `LiveScoreboard`, not `PlayerState`.

### 7: MatchRules, career trait, Transport/Clock (done, #31 + #32)
- `server/src/match_rules.rs` (done, #31): `MatchMode`, `MatchConfig` and the env parsers moved here from `formation.rs`; `MatchRules::for_mode(mode, team_size)` derives every decision once (`team_assignment: Balanced | ClientChoice | PracticeSeat`, `start: FirstJoin | FullRoster`, `prematch_roster: Present | Full`, `fills_with_bots`, `debug_commands`, `combat_sandbox_allowed`, `career_credit`, `local_results`, `career_flow`); `ServerRuntime.rules` replaces `match_config`, `advance_formation_on_join`, `tick_match_formation` and `prematch::snapshot` take `MatchRules`. The scattered `mode ==` checks in `runtime/dispatch.rs`, `runtime/mod.rs`, `formation.rs`, `bots.rs`, `session.rs`, `sandbox.rs`, `prematch.rs` and `career_runtime.rs` read a field; the runtime conditions they combined with (`match_service.worker()`, the sandbox flag, the prematch-capable join) stay at the site. `mode` stays on the rules for the banner, the logs and the `match_mode` wire string. The per-mode table is pinned by `match_rules::tests::rules_table_per_mode`. There was no respawn policy to lift: respawns are a `StatModifiers` flag, not a mode decision. `targeting_qa::enabled(mode)` is an env-gated capture switch, not a match rule, and keeps its mode argument.
- `server/src/career_port.rs` (done, #32): `trait CareerPort` with exactly the methods the runtime called on the backend; `CareerRuntime.backend: Box<dyn CareerPort>`. `career_backend.rs` keeps one account state machine, `CareerBackend<L: JobLink>`, and moves the I/O into the link: `WorkerLink` (bounded channels to the Postgres worker thread) is the production `CareerBackend`, `MemoryLink` (test-only) is `MemoryCareer` with `test_backend` (the old fixture: enabled, acknowledged by hand through the `test_*` hooks, jobs captured in `link.jobs`), `immediate` (every job answered at once like a healthy worker) and `disabled` (guest-only). The `test_*` hooks are trait methods under `cfg(test)` because the fixtures reach the store only through the `Box<dyn CareerPort>`; the `test_with_database` constructor stays on the concrete Postgres type. Decision on the worker-vs-standalone split: it stays at the call sites and is not in the port's contract. The eleven `match_service.worker()` sites decide *whether* a round is durable, public casual or eligible for rating and what to write into `cancel.json`; the port only records what it is handed and acknowledges it. Folding that into the trait would move allocation policy into the store, which the in-memory implementation would then have to mirror; a `MatchRules`-like `AllocationRules` derived once from the worker manifest is the follow-up if those sites keep growing.
- `server/src/runtime/ports.rs` (done, #32): `trait Transport { recv, send_to, local_addr }` (+ `peek` under `cfg(test)` for the fixtures that wait for kernel delivery) with `UdpTransport` and `MemoryTransport`; `trait Clock { now }` with `SystemClock` and `ManualClock`. `ServerRuntime { transport, clock, .. }` replaces `socket`; `with_ports(transport, clock, server_epoch, career, config, map)` is the one constructor, `new_with_map`/`new` wrap it with the process ports, `for_test(MemoryTransport, ManualClock, MemoryCareer, config)` with the memory ones (the runtime's epoch is the store's). `Instant::now()` left the tick path only: `prepare_tick`, the receive loop and its budget, the admission completions, the sandbox's roster and snapshot throttles and the constructor; leaf helpers keep their `now` parameter, `run()`'s step timer stays on the real clock, and `CareerBackend`'s internal TTLs (`COSMETIC_CACHE_TTL`, `CHALLENGE_TTL`, presence) still use `Instant::now()` because they are the store's own clock, not the simulation's. The sandbox's virtual clock keeps its own `now` (it is scaled and pausable) but starts from the injected clock. Converted as proof, not mass-converted: `practice_solo_starts_with_labelled_heroes_without_database_ack_or_ranked_credit` (join as a datagram through the memory transport), `tuned_default_label_is_unrated_and_running_roster_rejects_new_players` (memory fixture on the pending store), new `tests::sessions::runtime_on_memory_transport_and_manual_clock_times_out_a_silent_endpoint` (whole runtime, snapshot throttle and `PLAYER_TIMEOUT` on the manual clock) and new `immediate_memory_career_acknowledges_start_and_settle_through_poll`. The other fixtures still bind `127.0.0.1:0` through `ServerRuntime::new`; moving them is mechanical and can go with step 14.

### 14: Per-variant packet handlers (done, #33)
- `server/src/runtime/handlers/` (done): one file per concern, each an `impl ServerRuntime` block with `pub(in crate::runtime) fn handle_<variant>(&mut self, addr, <fields>, now) -> ControlFlow<()>` and the old arm body moved verbatim: `join.rs` (`handle_join`, incl. team assignment per `rules.team_assignment`), `movement.rs` (`handle_transform`), `combat.rs` (`handle_cast`, `handle_basic_attack`, `handle_upgrade_skill`), `utility.rs` (`handle_utility`), `shop.rs` (`handle_buy_item`), `debug.rs` (`handle_set_god_mode`, `handle_set_speed_boost`, `handle_practice`, `handle_sandbox_packet`), `session.rs` (`handle_hello`, `handle_ping`, `handle_leave`, `handle_career_rematch`, `handle_request_rematch`). `Break` is every early `return` of the old arm and skips the post-command tail; `Continue` runs it.
- `runtime/dispatch.rs` keeps `receive_packets`, `handle_packet` and `handle_packet_authorized`: the pre-checks up to the career and practice join admission are unchanged; `Leave`, the career-flow `RequestRematch`, `Practice`, `Sandbox` and the paused-sandbox gate became the first arms (with guards) of the one `match`, in the old order, so the `unreachable!` arms for `Leave`, `Practice` and `Sandbox` are gone; `Career`/`Social`/`Prematch` keep one `unreachable!` (routed by `handle_packet`, never reach the authorized path). The sandbox's `now` is computed before the `match` (a pure read); the pre-check arms get the wall clock as before.
- Imports (done): `main.rs` is the module list and `fn main`; the 24 crate-root globs (`use balance::*`, `pub(crate) use entities::*`, `sim::{cast::*, …, *}`, …) and the root `use shared::…`/`use std::…`/`omoba_career_store` lines that fed them are gone; every module names its imports. The 26 `use crate::*;` and the 20 file-level `use super::*;` in direct children of the crate root (which meant the crate root) are gone, and four `tests/*.rs` files lost a `use super::*;` that only carried the root re-exports; 29 `use super::*;` remain, all in test modules whose parent is a real module. The four `use shared::{career,sandbox,utility,vision}::*;` domain globs are untouched.

### 10: Client domain and plugin groups (done, #35, #36 and this PR)
- `client/src/domain/` for client-side game model types now spread across `combat.rs`, `player.rs`, `targeting.rs`; split `combat.rs` (targeting, casting, feedback) and `player.rs` (prediction vs presentation); 2D/3D render backends registered behind `run_if(PlayerVisualMode == …)`; `PluginGroup`s (`NetPlugins`, `GameplayPlugins`, `PresentationPlugins`, `UiPlugins`, `QaPlugins`); QA modules behind a `qa` cargo feature (default on for dev builds).
- The file/line inventory and the ordered slices (10a-10i) are in [plans/client-10-15.md](plans/client-10-15.md) (sections 10.1-10.7), together with step 15.
- 10a (#35): `client/src/domain/{mod,round,team,stats,actors}.rs` with `RoundId` (+ tests), `Team` and its `shared::map::Team` bridges, `CombatStats` + `MAX_HP`/`MAX_MANA`, `Player`/`PlayerBody`/`VerticalVelocity`/`RemotePlayer`, `MovementTarget`/`MovementRoute`; the old paths re-export them. `CombatPointerInputSet`/`WorldMovementInputSet` moved to `input_context.rs` (re-exported from `combat`). `CombatRoundIdentity` and `MobileControls.round_identity` hold `Option<RoundId>` (same semantics; switching them to `RoundChanged` is 15c).
- 10d (#35): `sprite::in_models3d()`/`in_sprite2d()` (`resource_exists_and_equals`) gate the backend registration sites listed in plan 10.4, except `battlefield_atmosphere.rs` (screen-space mist without a mode guard, shown in both modes). Internal mode guards stay.
- 10b (#36): `combat.rs` → `client/src/combat/{mod,cooldown,feedback,round_reset,selection,cast,mobile,hotbar,bars,marker}.rs`, `targeting.rs` → `combat/targeting.rs` (`lib.rs` keeps `pub(crate) use combat::targeting;`), tests in `combat/tests.rs` and `combat/target_presentation_tests.rs`. Verbatim moves; `mod.rs` keeps `CombatPlugin` (registration unchanged) and `configure_target_presentation` and re-exports the surface other modules import, so no file outside `combat/` changed its imports.
- 10c (#36): `player.rs` → `client/src/player/{mod,input,motion,respawn_ui,animation}.rs` plus `tests.rs`/`animation_tests.rs`; `mod.rs` keeps `PlayerPlugin`, `DebugSpeedBoost`, the constants and `ground_origin_y`. Same re-export and visibility rules (`pub(super)` for items and fields the sibling files or the test modules use; nothing widened to `pub(crate)`).
- 10e (this PR): `client/src/plugins.rs` with `NetPlugins`, `UiPlugins`, `GameplayPlugins` (including the debug tooling until step 11) and `PresentationPlugins`; `main` adds `DefaultPlugins`, inserts `PlayerVisualMode::from_environment()` once (`SpriteVisualsPlugin` only `init_resource`s it now), then the four groups and `QaPlugins`. `UiKitPlugin` → `MobileControlsPlugin` → `MobileUiPlugin` stay first in `UiPlugins` (the only build-time reads); the QA plugins build last. System registration inside the plugins is unchanged.
- 10f (this PR): `client/Cargo.toml` `[features] default = ["qa"]`; the harnesses moved to `client/src/qa/` (renames; three intra-QA paths now say `super::`), `qa/mod.rs` owns `QaPlugins`, the supporter capture moved out of `SupporterPlugin` into `qa/supporter.rs` (`SupporterQaPlugin`), `animation_qa::run` in `main` and `sandbox/ui/qa.rs` (kept in place: it drives the panel's private types) are `cfg(feature = "qa")`, and the `OMOBA_QA_SUPPORTER`/`OMOBA_CAREER_QA_OUTPUT` checks in production code are `cfg!(feature = "qa") && …`. `frontend::bypass_for` is automation and unchanged. No script, Makefile target or packaging step changed.
- 10g (this PR): `make check-no-qa` (part of `make check`) and a CI step run `cargo clippy -p client --lib --no-deps --no-default-features -- -D warnings`. Production items only QA reads are `#[cfg(feature = "qa")]`, items QA and tests read are `#[cfg(any(test, feature = "qa"))]`; nothing uses `allow(dead_code)`. Client lib tests: 549 with the default features, 529 without.
- Optional follow-ups from the plan: 10h (mobile store builds with `--no-default-features`), 10i (migrate imports off the re-export shims).

### 15: Client session events (in progress: 15a + 15b1 in #39, 15b2 + 15c + 15d in this PR)
- `SessionEvent` and `SnapshotApplied` messages emitted by `net`; other modules react to them instead of `net` writing into their resources; `apply_server_snapshot` split into staged passes. Inventory, design, ordering hazards and slices 15a-15g: [plans/client-10-15.md](plans/client-10-15.md), "Step 15".
- 15a (#39): `SessionEvent` (`TransportStarted`, `Connected`, `Joined`, `Rejected`, `JoinExhausted`, `Disconnected`, `Left`, `ServerScopeReset`, `RoundChanged`) in `net/session.rs`, registered by `NetworkingPlugin`, re-exported as `crate::net::SessionEvent`. Every site queues into `ClientSession.outbox` (`pub(in crate::net)`); `flush_session_events` writes the queue at the end of `ApplySnapshot` and chained after `retry_pending_join` in `SessionLifecycle`. `NetworkState.last_round` gives `RoundChanged` the `CombatRoundIdentity` semantics; `ClientSession.announced_join` (reset in `clear_join_attempt`) makes `Joined` an edge. `TeardownReason` is `pub(crate)`. `SessionReactions` is configured after `SessionLifecycle` and has no members yet. No consumers.
- 15b1 (#39): `SnapshotApply::{Begin, Session, Resources, Entities, Finish}`, chained and inside `ApplySnapshot`; `StagedSnapshot { data, gate }`; `apply_snapshot_entities` is the old body from the Draft gate on, unchanged except that it reads and sets the gate; `SnapshotApplied { meta, your_id, round, outcome }` with `ApplyOutcome::{Full, Draft, LocalPending}` (no `local` field yet); `snapshot_apply_systems()` is used by the plugin, `net/test_fixtures.rs` and the offline lifecycle test. `SnapshotUiState` stays (three resources) until 15b2, because the entity stage would otherwise need 18 parameters.
- 15b2 (this PR): `Entities` is split into `LocalPlayer`, `RemotePlayers`, `Projectiles`, `Structures`, `Minions`, `Neutrals` (chained between `Resources` and `Finish`). `LocalPlayer` runs the Draft despawn or closes the gate (`LocalPending`); the other five run under `world_stages_open` (`gate == Full`). Per-stage filtered queries replace the `Transform` `ParamSet` (hero `With<Player>, Without<MainCamera>`, camera `With<MainCamera>, Without<Player>`, `With<NetworkProjectile>`, ...). One `local_hero_components` bundle replaces the three spawn copies; `SnapshotUiState` is gone; `SnapshotApplied.local: LocalHeroApply::{Unchanged, Updated { entity, corrected, dashed }, Spawned { entity, position, team }, Cleared}`.
- 15c (this PR): `combat/round_reset.rs` and `read_mobile_controls` read `SessionEvent::RoundChanged` (after `ApplySnapshot`, whose flush writes it, and before input); `CombatRoundIdentity` and `MobileControls.round_identity` are deleted. The teardown-gap assertion is `net::apply::tests::teardown_gap_keeps_the_last_round_so_only_the_next_round_is_a_change`. `shop.rs`, `edge_hud.rs`, `sandbox/mod.rs` and `frontend/draft.rs` keep polling (they also react to the teardown's zero ids).
- 15d (this PR): `SessionReactions` members: `career::clear_account_on_scope_reset` and `social::clear_on_scope_reset` (`ServerScopeReset`), `frontend::return_home_on_leave` (`Left` → `PendingScreen(Home)`). `update_session_lifecycle` lost its `CareerClient`, `SocialClient` and `PendingScreen` writes; `TeamSelection.team = None`, `take_return_to_lobby()` and the `CancelQueue` signing stay in `net` (hazards 8, 9). `scope_reset_clears_career_and_social_in_its_frame_so_the_next_view_survives` pins the same-frame clearing (hazard 7).
- Next: 15e (`ClientSession` accessors); 15f (camera through `SnapshotApplied`, its first reader, which also drops its `expect(dead_code)`) and 15g are optional.

### 11: One debug tooling family
- One `DebugCommand` family in `shared` covering Combat Test sandbox, practice (`PracticeCommand`) and offline; one tools UI page; the offline simulation implements the same commands so the pause-menu page works in all three modes.

### 12: Data-driven catalogs (done, this PR)
- Plan: [plans/steps-11-13.md](plans/steps-11-13.md), "Step 12". Slices 12a-12e landed together; 12f (client cross-checks: `combat_visuals.json` classes cover the catalog, `combat/targeting.rs` reads its attack-rate caps from `hero_balance`) is optional and open.
- 12a: `ItemId::ALL`; `ItemId::from_id` searches the enum, so wire decoding never touches the catalog; `shop::items()` replaced the direct `ITEMS` uses (client `shop.rs`, `sandbox/ui.rs`, `sandbox/presets.rs`, server `shop.rs` test, `sandbox/tests.rs`); harness `combat_actions.rs` reads the Warrior Q through `ability_for_class_slot`; `DEFAULT_MAX_HP` is the literal `220.0` with a test that it equals `base_hp(Warrior)`.
- 12b: `shared/assets/catalog/heroes.json` and `items.json` (`schema_version` 1) and `shared/src/catalog.rs`: two `LazyLock` tables from `include_str!`, private `Raw*` structs with `deny_unknown_fields`, validation at load (panics with the file and the entry), conversion into the unchanged `AbilityDefinition`/`ItemDefinition`/`BasicAttackDefinition` (strings leaked once, so the definitions stay `Copy`). A migration test compared every catalog field with the Rust tables bit for bit (`f32::to_bits`) and passed before 12c deleted it together with the tables.
- 12c: `basic_attack_for_class`, `HeroClass::{display_name, tagline, primary_role, abilities, ability}`, `hero_balance::{base_hp, basic_damage_multiplier, attack_rate_multiplier}`, `shop::{items, item, recommended_items}` and `ProjectileStyle::for_class` read the catalog (no longer `const fn`); the `*_ABILITIES` constants, `ITEMS` and the `ability` helper are gone. `shared::catalog::ensure_loaded()` runs first in server `runtime::run` and client `main`.
- 12d: the catalog length is `ItemId::ALL.len()`, not `INVENTORY_CAPACITY`; the "all items" test and preset sites take at most `INVENTORY_CAPACITY`; `practice::tests::duel_gold_cap_buys_a_full_inventory` replaces the comment claim.
- 12e: `scripts/catalog.py` reads the JSON; `combat_test.py` (`--hero` choices), `verify_beta_match.py` (item costs; offensive slots = abilities with `projectile_damage`) and `capture_combat.py` (`STYLES`) use it. `check_combat_balance.py` keeps taking its classes from its captures.

### 13: Shared I/O isolation
- Move avatar/sprite roster manifests and env-var reads out of `shared` into `client` (and the SDK types into a small adapter crate or `passport`); `shared` keeps the model only.

### 9b: UI kit follow-ups
- Order and details in [ui-kit.md](ui-kit.md).
