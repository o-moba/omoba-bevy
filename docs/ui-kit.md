# Client UI kit

`client/src/ui/` is the one place the client's overlays get their palette,
their tap recognizer, their button actions and their widgets. Roadmap step 9
(`docs/ARCHITECTURE.md`) planned it; the pause menu and the practice sandbox
were the pilot. Step 9b items 3 and 4 moved the front-end screens, hero
select, career, social, supporter, the Combat Test panel and the help
overlay onto typed actions; what still uses its own buttons is listed under
[Remaining migration](#remaining-migration).

## Modules

| Module | Holds |
| --- | --- |
| `ui/mod.rs` | `UiKitPlugin`, `UiPlatform`, `UiSet` |
| `ui/theme.rs` | palette, fonts, `metric`, `ButtonKind` (`Primary`, `Secondary`, `Tile`, `Danger`, `Link`, `Team(Team)`) and its idle/hover colours |
| `ui/gesture.rs` | `Pressable`, `TapTracker`, `recognize_presses`, `GestureEpoch`, `SyntheticPress`, `logical_ui_rect` |
| `ui/action.rs` | `UiAction<T>`, `Activated<T>`, `dispatch_actions::<T>`, `UiActionAppExt` |
| `ui/widgets.rs` | `ButtonStyle`, `paint_pressables`, `button`, `button_with_label`, `icon_button`, `adjust_row`, `toggle_row`, `value_label`; front-end `screen_button`, `screen_tile`, `compact_screen_tile`, `screen_label` and their phone metrics `MenuTypography`/`MenuControl` |
| `ui/test_id.rs` | `TestId`, `harness::{TestIds, find, press, kit_app, spawn_ui, set_disabled, drain_actions}` (tests) |

The `crate::ui_theme` shim, the `frontend::widgets` palette re-export,
`MenuButton` and `frontend::widgets::{button, tile, compact_tile}` are gone:
every module imports `crate::ui::theme` (a few keep a local alias, `ui` or
`ui_theme`). `frontend::widgets` keeps the layout nodes (`screen_root`,
`panel_row`, `heading`, `label`) and the phone readability pass
(`adapt_phone_menu_readability`, item 5).

## Platform

`UiPlatform(UiProfile)` is inserted once by `UiKitPlugin` from
`platform::ui_profile()`. Systems read `Res<UiPlatform>` (career, collection,
card, home, help overlay); `MobileControls.enabled` is the runtime copy that
`MobileControlsPlugin` fills from it at build time, so a test that constructs
`MobileControls::default()` gets `enabled: false` and sets it itself.

## Order

`UiKitPlugin` configures `UiSet::Gesture → Dispatch → Paint` at the start of
`InputContextSet::Modal`:

1. **Gesture** – `recognize_presses` resets every `Pressable` (`touch_mode`
   from the platform, `activated = false`, `disabled` untouched), applies
   `SyntheticPress` messages, then feeds this frame's `TouchInput` (and, on a
   desktop build in touch mode, the left mouse button) to the `TapTracker`.
2. **Dispatch** – `dispatch_actions::<T>` (one per `add_ui_action::<T>()`)
   writes `Activated<T> { action, source }` for every button whose
   `Pressable::effective(Interaction)` became `Pressed` this frame. It runs on
   `Or<(Changed<Interaction>, Changed<Pressable>)>`, so a held click or a
   resting finger fires once.
3. **Paint** – `paint_pressables` colours `(Pressable, ButtonStyle,
   BackgroundColor)` from the effective interaction.

Modules consume `Activated<T>` after `UiSet::Dispatch` (the pause menu in
`PauseMenuSet::Visuals`). A handler that is gated (audio only while the
settings page is the front-most modal) still reads every message so a press
made while gated cannot fire later.

## Gesture semantics

`Pressable { touch_mode, activated, disabled }`:

- **Desktop** (`touch_mode == false`): `effective` returns Bevy's
  `Interaction` unchanged.
- **Touch**: a finger resting on the button is `Hovered`; the tap completes
  on `Ended` inside the same button rect and sets `activated` for one frame.
  Moving more than 10 px cancels the tap for good (coming back does not revive
  it); only the first finger counts; `Canceled` clears it.
- `activated` wins in either mode, which is how `SyntheticPress(entity)`
  activates a button from a harness without touching `Interaction`.
- `disabled` (owned by the module): never hit-tested, never `Pressed`, never
  lit. The pause menu disables its buttons while the phone server-address
  overlay covers it.

The recognizer is active only in touch mode with the mobile HUD landscape and
focused and the window focused; `AppLifecycle::{WillSuspend, Suspended,
WillResume}`, a viewport change and a `GestureEpoch` bump drop the held tap.
Hit rects are DPI-corrected node rects intersected with `CalculatedClip`,
filtered by `InheritedVisibility` and non-zero size; on `Started` the hit with
the highest `ComputedNode::stack_index` wins. When the `MobileControls`
resource is absent (unit tests) orientation and focus do not veto.

`GestureEpoch` is bumped by a modal on navigation: the pause menu on
open/close, settings page and practice page changes; career on `modal`
changes. Career previously ran its own copy of the tracker gated only on
`touch_mode && window.focused`; it now shares the unified gate (landscape is
enforced by the rotate prompt on phones anyway) and gains the desktop mouse
emulation in mobile preview builds.

## Actions and widgets

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
enum PauseAction { Resume, Close, OpenSettings, BackFromSettings, Help, Exit,
    LeavePractice, ResetGraphics, Step(Setting, i8), Audio(AudioButton), OpenPractice }

app.add_ui_action::<PauseAction>();               // message + dispatcher
widgets::button(parent, "Settings", ButtonKind::Secondary, PauseAction::OpenSettings, "SettingsButton");
widgets::adjust_row(parent, "Level", "5".into(), Label::Level, A::LevelDown, A::LevelUp, "PauseMenuPracticeLevelControls");
widgets::toggle_row(parent, "God mode", "OFF", Label::GodMode, A::GodMode, "PauseMenuPracticeGodMode");

fn apply(mut activated: MessageReader<Activated<PauseAction>>, ...) {
    for Activated { action, .. } in activated.read() { match action { ... } }
}
```

`UiAction<T>` requires `Pressable`; `button` also adds `ButtonStyle` and a
`TestId`. The front-end screens use `screen_button(parent, text, kind,
action, id)` (a `Primary` call to action is 240×60 with a gold edge, the rest
44 px pills) and `screen_tile(parent, text, selected, action, id)`; a screen
that owns selection flips `ButtonStyle::selected` and the painter repaints.
A control that keeps a look of its own (card accent swatches, social wheel
choices, supporter buttons, Combat Test value fields and toggles) carries
`UiAction` and a `TestId` without `ButtonStyle`. `adjust_row` names its controls `{id}-Down`, `{id}-Value`, `{id}-Up`;
`toggle_row` names `{id}Button` and `{id}Value`; `button_with_label` gives the
caption a marker and its own id (the mute toggle). The primary call to action
is `ButtonKind::Primary` (the pause menu's Resume), not a name check.

## TestId policy

`TestId(Cow<'static, str>)` mirrors itself into `Name` on insert when the
entity has none, so QA node dumps and `Name` lookups keep resolving. Every
kit control and rewritable value label carries one; layout-only nodes keep a
plain `Name`. Under `cfg(test)` `ui::test_id::harness` offers `find(world,
id)`, `press(world, id)` (queues a `SyntheticPress`) and the `TestIds`
system param, plus `kit_app()` (recognizer, dispatch sets, painter),
`spawn_ui`, `set_disabled` and `drain_actions::<T>` for screen tests. QA
harnesses press kit buttons with `SyntheticPress` (`audio_qa` directly, the
rest through `crate::qa::NamedPresses`, see below) instead of writing
`Interaction::Pressed`.

## Screens on typed actions (9b items 3 and 4)

Each module registers `add_ui_action::<T>()` and reads `Activated<T>` in a
system ordered `.after(UiSet::Dispatch)`:

| Module | Action type | Handler (order) | Notes |
| --- | --- | --- | --- |
| `frontend/home.rs` | `HomeAction` | `home_actions` | |
| `frontend/card.rs` | `CardAction` | `card_actions` → `refresh_card_screen` flips tile `ButtonStyle::selected` | accent swatches without `ButtonStyle` |
| `frontend/collection.rs` | `CollectionAction` | `collection_actions` (in the drag chain) | a preview drag (`block_actions`) clears the presses |
| `frontend/searching.rs` | `SearchingAction` | `searching_actions` | |
| `frontend/draft.rs` | `DraftAction` | `draft_actions` in `DraftSet::Input` (now after `Dispatch`) | `action_button<T>` is shared with loading |
| `frontend/loading.rs` | `LoadingAction` | `loading_actions` in `DraftSet::Input` | |
| `frontend/postmatch.rs` | `PostMatchAction` | `post_match_actions` | |
| `team.rs` (hero select) | `HeroSelectAction` | `team_select_ui_system`, before `SendCommands` | `LockIn(team)` calls `team::lock_in`; tiles `Tile`, Ekza buttons `Link`, lock-in `Team(team)` |
| `career.rs` | `career::Action` | `actions` (career chain, now after `Dispatch`) | social gating clears the presses |
| `social.rs` | `SocialAction` | `social_actions` in `Modal` after `Dispatch`, before `CareerUiSet` | `input` (in `Social`, before the kit) leaves a `ButtonFrame`; a gated frame has none and the presses are dropped |
| `supporter.rs` | `supporter::Action` | `actions` | disabled buttons are `Pressable::disabled` |
| `sandbox/ui.rs` | `sandbox::ui::Action` | `actions` (panel chain, now after `Dispatch`) | a closed panel clears the presses; ids `CombatTest…` |
| `help_overlay.rs` | `HelpAction` | `dismiss_help_button` | needed to retire `MenuButton` |

QA harnesses press by `Name` through `crate::qa::NamedPresses` (and the
same rule in `offline_qa` and `sandbox/ui/qa.rs`): a kit button gets a
`SyntheticPress`, which activates it once in desktop and touch mode; any
other button still gets `Interaction::Pressed`. Every `Name` the harnesses
press is unchanged (`TestId` mirrors into `Name`).

## Remaining migration

In the order the roadmap intends, each a PR of its own:

1. **Scroll** – one touch/wheel scroll system for `TouchScrollPanel`,
   replacing `pause_menu::scroll_desktop_settings`, `career::scroll_desktop`
   and `mobile_ui::scroll_phone_panels` (and the screen scrollers:
   `collection::scroll_collection`, `draft::scroll_panels`,
   `team::scroll_avatar_roster`, `supporter::scroll_panel`,
   `social::scroll_chat`, the Combat Test `scroll`).
2. **Modal registry** – a stack of open modals that gates gestures, replaces
   `Pressable::disabled` juggling (server entry over the pause menu) and the
   per-module `modal_open`/`blocks_gameplay` checks in `input_context`. The
   handlers above still gate themselves (social `ButtonFrame`, career on
   social, sandbox on `open`) and clear their presses when gated.
3. ~~Front-end screens~~ – done (see above).
4. ~~Career, social, supporter, sandbox~~ – done. The Combat Test panel moved
   completely: its text fields are keyboard-driven (`edit_keys`) and
   teleport picking is a world click (`teleport`), neither is a button.
5. **Responsive layout** – phone metrics (`adapt_phone_menu_readability`,
   `adapt_phone_layout`, `size_desktop_pause_panel`) through one `metric`
   policy.
6. **TestId in QA** – harness lookups by `TestId` instead of `Name`, then
   the `Name` mirror can go.
7. **Colours and the last own buttons** – the remaining hand-painted or
   `Interaction`-reading buttons onto `UiAction`/`ButtonStyle`: the combat
   skill bar (`combat/hotbar.rs`), the shop (`shop.rs`), the phone bar and
   server entry (`mobile_ui.rs`), the edge HUD and scoreboard
   (`edge_hud.rs`), the connection panel (`net/status_ui.rs`) and the debug
   HUD (`debug/hud.rs`). Hero select is on `ButtonStyle` now (`Link`,
   `Team`); `ui/theme.rs` keeps the team colours.
