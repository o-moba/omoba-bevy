# Client UI kit

`client/src/ui/` is the one place the client's overlays get their palette,
their tap recognizer, their button actions and their widgets. Roadmap step 9
(`docs/ARCHITECTURE.md`) planned it; the pause menu and the practice sandbox
are the pilot, everything else still uses its own buttons and is listed under
[Remaining migration](#remaining-migration).

## Modules

| Module | Holds |
| --- | --- |
| `ui/mod.rs` | `UiKitPlugin`, `UiPlatform`, `UiSet` |
| `ui/theme.rs` | palette, fonts, `metric`, `ButtonKind`, `MenuButton` |
| `ui/gesture.rs` | `Pressable`, `TapTracker`, `recognize_presses`, `GestureEpoch`, `SyntheticPress`, `logical_ui_rect` |
| `ui/action.rs` | `UiAction<T>`, `Activated<T>`, `dispatch_actions::<T>`, `UiActionAppExt` |
| `ui/widgets.rs` | `ButtonStyle`, `paint_pressables`, `button`, `button_with_label`, `icon_button`, `adjust_row`, `toggle_row`, `value_label` |
| `ui/test_id.rs` | `TestId`, `harness::{TestIds, find, press}` (tests) |

`crate::ui_theme` and `crate::frontend::widgets` re-export the theme so the
other modules did not move; delete the shims when their users migrate.

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
`TestId`. `adjust_row` names its controls `{id}-Down`, `{id}-Value`, `{id}-Up`;
`toggle_row` names `{id}Button` and `{id}Value`; `button_with_label` gives the
caption a marker and its own id (the mute toggle). The primary call to action
is `ButtonKind::Primary` (the pause menu's Resume), not a name check.

## TestId policy

`TestId(Cow<'static, str>)` mirrors itself into `Name` on insert when the
entity has none, so QA node dumps and `Name` lookups keep resolving. Every
kit control and rewritable value label carries one; layout-only nodes keep a
plain `Name`. Under `cfg(test)` `ui::test_id::harness` offers `find(world,
id)`, `press(world, id)` (queues a `SyntheticPress`) and the `TestIds`
system param. QA harnesses that used to write `Interaction::Pressed` on a
kit button send `SyntheticPress` instead (`audio_qa` does); the other
harnesses only press non-migrated buttons and are unchanged.

## Remaining migration

In the order the roadmap intends, each a PR of its own:

1. **Scroll** – one touch/wheel scroll system for `TouchScrollPanel`,
   replacing `pause_menu::scroll_desktop_settings`, `career::scroll_desktop`
   and `mobile_ui::scroll_phone_panels`.
2. **Modal registry** – a stack of open modals that gates gestures, replaces
   `Pressable::disabled` juggling (server entry over the pause menu) and the
   per-module `modal_open`/`blocks_gameplay` checks in `input_context`.
3. **Front-end screens** – home, hero select, collection, card, draft,
   searching, post-match onto `UiAction`/`ButtonStyle`; retire
   `MenuButton`, `frontend::widgets::{button, tile}` and the shims.
4. **Career, social, supporter, sandbox** – typed actions (career already has
   `Pressable`; its `Action` enum becomes `UiAction<Action>`).
5. **Responsive layout** – phone metrics (`adapt_phone_menu_readability`,
   `adapt_phone_layout`, `size_desktop_pause_panel`) through one `metric`
   policy.
6. **TestId in QA** – harness lookups by `TestId` instead of `Name`, then
   the `Name` mirror can go.
7. **Colours** – the remaining hand-painted buttons (combat skill bar, team
   select, shop) onto `ButtonStyle`.
