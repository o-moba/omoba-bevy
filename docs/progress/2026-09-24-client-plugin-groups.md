# 2026-09-24 — Client plugin groups and the qa feature (roadmap step 10, slices 10e + 10f + 10g)

## Goal
Finish step 10 of the roadmap with the last three slices from
[plans/client-10-15.md](../plans/client-10-15.md) (sections 10.1, 10.2, 10.5):
- 10e: register the client's plugins as a few named `PluginGroup`s instead
  of one long `main` with tuple-size workarounds, and choose the render
  backend once in `main`.
- 10f: put the opt-in native QA harnesses behind a `qa` cargo feature (on by
  default) and give them their own module tree.
- 10g: prove that the client builds without them, in CI and in `make check`.

No behaviour change and no wire change. `shared/` and `server/` are
untouched; nothing under `scripts/`, `mobile/` or the packaging changed.

## 10e: `client/src/plugins.rs`
| Group | Plugins, in build order |
| --- | --- |
| `NetPlugins` | `ClientPersistencePlugin`, `NetworkingPlugin`, `MatchServicePlugin`, `CareerIdentityPlugin` |
| `UiPlugins` | `UiKitPlugin`, `MobileControlsPlugin`, `MobileUiPlugin`, `FrontendPlugin`, `TeamSelectPlugin`, `GameStateUiPlugin`, `MatchHudPlugin`, `EdgeHudPlugin`, `MinimapPlugin`, `ShopPlugin`, `HelpOverlayPlugin`, `PauseMenuPlugin`, `SocialPlugin`, `CareerPlugin`, `SupporterPlugin`, `SupporterStoreKitPlugin` |
| `GameplayPlugins` | `MapsPlugin`, `InputContextPlugin`, `PlayerPlugin`, `CombatPlugin`, `SandboxPlugin`, `PracticeSandboxPlugin`, `GodModePlugin`, `DebugConsolePlugin` |
| `PresentationPlugins` | `CameraPlugin`, `SetupPlugin`, `ModelScalePlugin`, `CombatVisualsPlugin`, `CombatFeedbackPlugin`, `GameVfxPlugin`, `ReactionVisualsPlugin`, `TeamVisionPlugin`, `GameAudioPlugin`, `MapVisualsPlugin`; `SpriteVisualsPlugin`, `Presentation2dPlugin`, `World2dPlugin`; `Presentation3dPlugin`, `Verdant3dPlugin`, `DecorPlugin`, `JungleVisualsPlugin`, `MinionVisualsPlugin`, `BossesPlugin`, `ProjectileVisualsPlugin`, `BattlefieldAtmospherePlugin` |
| `QaPlugins` (`qa/mod.rs`) | `FrontendQaPlugin`, `VisualQaPlugin`, `SocialQaPlugin`, `SupporterQaPlugin`, `TeamVisionQaPlugin`, `AudioQaPlugin`, `OfflineQaPlugin`, `CareerVisualQaPlugin`, `MapQaPlugin`, `CombatQaPlugin`, `ForestPickupQaPlugin`, `TargetingQaPlugin` |

All 60 plugins `main` added before are in exactly one group (49
production, 11 QA; `SupporterQaPlugin` is new, see 10f). The debug tooling
sits in `GameplayPlugins` until step 11 gives it a `DebugPlugins` group.

`main` (in `lib.rs`) is now: the animation audit early exit (`qa` only),
`sandbox::validate_launch`, passport, the model measurement tool, asset
sources, `DefaultPlugins`, `insert_resource(PlayerVisualMode::from_environment())`,
`.add_plugins((NetPlugins, UiPlugins, GameplayPlugins, PresentationPlugins))`
and, under `cfg(feature = "qa")`, `.add_plugins(qa::QaPlugins)`. The 15
`use …Plugin` lines and the "tuple is at Bevy's 15-element limit" split are
gone.

### Build order
Only the order of `Plugin::build` calls changed; every plugin registers the
same systems, sets, run conditions and ordering constraints as before.
Checked every `impl Plugin` body for world access (`world()`,
`world_mut()`, `contains_resource`, `get_resource`, `resource::<…>`), for
`FromWorld` resources, `init_state`/`insert_state`, observers and
`is_plugin_added`:
- `UiKitPlugin` inserts `UiPlatform` unless present; `MobileControlsPlugin`
  reads it to set `MobileControls::enabled`; `MobileUiPlugin` reads
  `enabled` (after its own `init_resource`, so the wrong order would install
  nothing on phones rather than panic). The three are first in `UiPlugins`,
  in that order, as before (UiKit was 22nd, MobileControls 36th, MobileUi
  37th in the old list).
- `FrontendPlugin` `init_resource`s `ScreenDriverPaused`; `FrontendQaPlugin`
  and `AvatarQaPlugin` insert `ScreenDriverPaused(true)`. QA still builds
  after the frontend. The QA plugins' `WinitSettings::continuous()`
  overwrites `platform::configure_app` (mobile) and `DefaultPlugins`, as
  before.
- `BetaUiQaPlugin` reads only its own resource; `VisualQaPlugin` and
  `FrontendQaPlugin` still nest their sub-harnesses; `MinimapPlugin` still
  nests `MinimapRoutePlugin`.
- `AvatarPreview`'s `FromWorld` reads `Assets<Image>`, which
  `DefaultPlugins` provides.
- Resources initialised by two plugins (`CameraSettings`, `AudioSettings`,
  `DebugConsole`, `CareerIdentity`, `MobileControls`, `HelpOverlayVisible`)
  all use `init_resource`, so the first one wins with the same default
  whatever the order.
- Nothing else reads the world during `build`.

`PlayerVisualMode`: `main` inserts `from_environment()` after
`DefaultPlugins` (so an invalid `OMOBA_PLAYER_VISUAL_MODE` warning still
reaches the log) and before every group. `SpriteVisualsPlugin` now does
`init_resource::<PlayerVisualMode>()`, which keeps the value from `main`
and gives apps that add the plugin alone (the `sprite` backend tests) the
default `Models3d`, the value `from_environment()` returned in tests before.
`from_environment` became `pub(crate)`. The missing-mode test still removes
the resource after adding the plugins.

### Executor order
Because plugins are added in a different order, systems are inserted into
the schedules in a different order. The multi-threaded executor's relative
order of systems that are mutually unordered and conflict may differ from
before (that order was never guaranteed). No ordering constraint was added,
removed or changed. The plan suggests running the capture scripts as a
smoke test; that needs a display and a server and was not run here.

## 10f: the `qa` feature and `client/src/qa/`
- `client/Cargo.toml`: `[features] default = ["qa"]`, `qa = []`. The
  feature only gates client code; Bevy's features come from the dependency
  entries, and `Cargo.lock` does not change.
- Moved (renames, names kept):

  | From `client/src/` | To `client/src/qa/` | Change |
  | --- | --- | --- |
  | `animation_qa.rs` | `animation_qa.rs` | none |
  | `visual_qa.rs` | `visual_qa.rs` | `crate::beta_ui_qa` / `crate::navigation_qa` → `super::…` |
  | `beta_ui_qa.rs`, `edge_hud_qa.rs` | same names | none (`#[path = "edge_hud_qa.rs"]` resolves next to it) |
  | `navigation_qa.rs`, `targeting_qa.rs`, `combat_qa.rs`, `map_qa.rs`, `forest_pickup_qa.rs`, `team_vision_qa.rs` | same names | none |
  | `frontend_qa.rs`, `frontend_qa/avatar.rs` | same names | `crate::frontend_flow_qa` → `super::frontend_flow_qa` |
  | `frontend_flow_qa.rs` | same name | doc link `crate::frontend_qa` → `super::frontend_qa` |
  | `social_qa.rs`, `audio_qa.rs`, `offline_qa.rs`, `career_visual_qa.rs` | same names | none |

- `qa/mod.rs` declares them (`animation_qa` is `pub(crate)` for `main`,
  the rest private) and owns `QaPlugins`.
- New `qa/supporter.rs`: `SupporterQa` and `capture_preview` moved from
  `supporter.rs` verbatim; the `OMOBA_SUPPORTER_QA_DIR` block that
  `SupporterPlugin::build` ran first is now `SupporterQaPlugin::build`
  (same resources, same `PostUpdate … .after(UiSystems::Layout)`
  registration), placed in `QaPlugins` where `SupporterPlugin` used to
  build relative to the other harnesses.
- `cfg(feature = "qa")` hooks: `mod qa` and `app.add_plugins(qa::QaPlugins)`
  in `lib.rs`; the `OMOBA_ANIMATION_QA` early exit in `main`;
  `sandbox/ui.rs` `mod qa` and `qa::install(app)`. `sandbox/ui/qa.rs` stays
  in place: it drives the panel through its private `Action`, `Tab`,
  `Toggle`, `Field`, `Body` and `keys`, so moving it would widen them.
- Env checks inside production code became `cfg!(feature = "qa") && …`:
  `SupporterUiState::default` (`OMOBA_QA_SUPPORTER` opens the panel for the
  supporter capture) and `career_identity.rs` (`OMOBA_CAREER_QA_OUTPUT`
  skips the private identity during the career capture; now one helper,
  `career_qa_capture()`, instead of two copies).
- Unchanged on purpose: `frontend::bypass_for` (automation: any
  `OMOBA_*_QA_DIR`/`_QA_OUTPUT` or `OMOBA_AUTOJOIN` skips the shell) and
  `model_scale::run_model_measurement_analyzer` (`OMOBA_MEASURE_MODELS`, a
  measuring tool, not a harness).
- No production module referenced a QA module, so no re-export was needed.
  Only historical `docs/progress/` notes mention the old paths.
- `grep` over `scripts/`, `mobile/`, the `Makefile` and CI: nothing passes
  `--no-default-features` for the client, so every script still gets the
  harnesses.

## 10g: the build without QA
`cargo clippy -p client --lib --no-deps --no-default-features -- -D warnings`
is a new step in the CI `rust` job (after the workspace clippy) and the new
`make check-no-qa` target, which `make check` runs after `lint`.

Items it reported, gated instead of allowed:

| Item | Read by | Gate |
| --- | --- | --- |
| `combat::CombatBarAnchor` re-export | `team_vision_qa` | `feature = "qa"` |
| `humanoid::RuntimeHumanoidBindingError` re-export | `animation_qa` | `feature = "qa"` |
| `HumanoidRuntimeLibrary::semantic_nodes` | `animation_qa` | `feature = "qa"` |
| `CareerClient::present_visual_fixture` | `career_visual_qa` | `feature = "qa"` |
| `DamageNumber.event_id` (field and its initialiser) | `combat_qa` | `feature = "qa"` |
| `ParticleSlot::sample` | `combat_qa`, `forest_pickup_qa` | `feature = "qa"` |
| `MapPropInstance.role` (field, both initialisers, the `role` lookup) | `map_qa` | `feature = "qa"` |
| `SocialClient::{wheel_center, qa_send_chat, qa_close, qa_diagnostics}` | `social_qa` | `feature = "qa"` |
| `player::PlayerAnimationBinding` re-export, `PlayerAnimationBinding::is_running` | `animation_qa`, player animation tests | `any(test, feature = "qa")` |
| `ProjectilePresentationRoot.owner` (field and both initialisers) | `combat_qa`, presentation2d tests | `any(test, feature = "qa")` |
| `MapVisualRegistry::replace_json`, `MapVisualCache::counts` | `map_qa`, map-visual, world2d and presentation2d tests | `any(test, feature = "qa")` |
| `MinimapQaScene` (+ `diagnostics`), `MinimapCamp.{index, alive}`, `RouteSegment.0` | visual, beta UI, navigation and team-vision QA, minimap tests | `any(test, feature = "qa")` |
| `ShopState::purchase_pending` | beta UI and edge HUD QA, shop tests | `any(test, feature = "qa")` |

The plan suggested `any(test, feature = "qa")` for all of them; items no
test reads use `feature = "qa"` so that `cargo test --no-default-features`
is warning-free too. Components keep their place in every spawn bundle
(`MinimapCamp` and `RouteSegment` become field-less markers without the
feature; `RouteSegment` is built by a `cfg`'d `let` so the bundle is the
same tuple). Nothing uses `allow(dead_code)`.

## Tests
- `cargo test -p client --lib`: 549 passed (unchanged).
- `cargo test -p client --lib --no-default-features`: 529 passed. The 20
  missing tests are the ones inside the QA modules (visual 5, social 4,
  beta UI 3, edge HUD 2, frontend 2, career visual 2, frontend flow 1,
  map 1). The plan's estimate was 523.
- `cargo test -p shared`: 78 passed.
- Python script tests: 86 run, OK (1 skipped).

## Checks
- `cargo fmt --all -- --check` clean.
- `cargo clippy --workspace --all-targets --no-deps -- -D warnings` clean.
- `cargo clippy -p client --lib --no-deps --no-default-features -- -D warnings` clean.

## Follow-ups
- 10h (optional): release/TestFlight and Android store builds with
  `--no-default-features`.
- 10i (optional): migrate imports off the re-export shims.
- Step 11 moves `SandboxPlugin`, `PracticeSandboxPlugin`, `GodModePlugin`
  and `DebugConsolePlugin` into a `DebugPlugins` group.
