# Client UI kit

`client/src/ui/` is the one place the client's overlays get their palette,
their tap recognizer, their scrolling, their modal stack, their button
actions and their widgets. Roadmap step 9
(`docs/ARCHITECTURE.md`) planned it; the pause menu and the practice sandbox
were the pilot. Step 9b items 3 and 4 moved the front-end screens, hero
select, career, social, supporter, the Combat Test panel and the help
overlay onto typed actions; items 5 to 7 put the phone/desktop sizes behind
one `metric` policy, moved the QA harnesses to `TestId` and the last HUD
buttons onto the kit. Every button the client draws is now a kit button (see
[Remaining migration](#remaining-migration)).

## Modules

| Module | Holds |
| --- | --- |
| `ui/mod.rs` | `UiKitPlugin`, `UiPlatform`, `UiSet` |
| `ui/tokens.rs` | Verdant Crown design tokens, generated from `client/ui/tokens/verdant-crown.json` by `client/build.rs` (`color`, `space`, `radius`, `border`, `size`, `motion` modules; `Metric`, `FontFamily`, `TextRole`/`TypeStyle`, `CubicBezier`; round-trip tables) — see [Verdant Crown foundation](#verdant-crown-foundation-0270) |
| `ui/kit_assets.rs` | the installed UI textures as types (`Icon`, `Frame`, `Sprite`, `Background`, generated from `client/assets/ui/verdant/manifest.json`), `KitImage` → `ImageNode` at the right density (`UiDensity`, 9-slice `slicer`), `CoverImage` |
| `ui/font_cmap.rs` | a face's characters from its `cmap` (display-face fallback) |
| `ui/theme.rs` | palette (legacy names as token aliases), fonts and text roles (`TextStyle`, `UiForm`, `apply_text_styles`, `perceptual`), `metric` (sizes and the responsive policy: `desktop_ui_scale`, `Form`, `menu_font`, `menu_control_height`, `pause_panel_height`, `phone_class_column`, `phone_shop_card`, `phone_font`/`PhoneText`, phone panel widths), `ButtonKind` (`Primary`, `Secondary`, `Tile`, `Danger`, `Link`, `Team(Team)`, `Skill`, `SkillUpgrade`, `ShopItem`, `Debug(DebugToggle)`) and its idle/hover/pressed colours |
| `ui/gesture.rs` | `Pressable`, `TapTracker`, `TAP_SLOP`, `recognize_presses`, `GestureEpoch`, `SyntheticPress`, `logical_ui_rect` |
| `ui/scroll.rs` | `ScrollArea` (`WheelScroll`, `DragScroll`, `ScrollPlatform`), `scroll_areas`, `max_offset`, `harness` (tests) |
| `ui/modal.rs` | `ModalId`, `ModalStack`, `ModalRoot`, `ModalAppExt::register_modal`, `ModalSet`, `ModalGate` |
| `ui/action.rs` | `UiAction<T>`, `Activated<T>`, `dispatch_actions::<T>`, `UiActionAppExt` |
| `ui/back.rs` | `BackInput` (Esc plus `BackPress`), `BackPress`, `clear_back_press` |
| `ui/focus.rs` | `UiFocus`, `FocusNav`, `navigate_focus`, `directional_neighbor`, `reveal_delta`, `FocusAdjustable`/`FocusAdjust` (Left/Right on a slider or cycle row), `FocusSkip` |
| `ui/widgets/` | `mod.rs`: `ButtonStyle`, `KitSkin`, `KitParts`, `PreviewState`, `paint_pressables`, `paint_slabs`, `paint_kit`, `paint_focus_ring` (`FocusRing`, `FocusHalo`), `spawn_button`/`button_node`/`ButtonSize`, `button`, `button_with_label`, `icon_button`, `adjust_row`, `toggle_row`, `value_label`; front-end `screen_button`, `screen_tile`, `compact_screen_tile`, `screen_label` and their phone metrics `MenuTypography`/`MenuControl`; captions are `impl UiLabel` (see [Text and language](#text-and-language)). `controls.rs`: icon button, stepper buttons, tabs, toggle, slider, cycle row, text input. `surfaces.rs`: panels, modal, list row, badge, tooltip, toast. `game.rs`: bars, ability button, item slot, shop card, hero tile, portrait, scoreboard row, timer ring, HUD plates |
| `ui/test_id.rs` | `TestId`, `NodeKey`/`node_key` (a node's `TestId`, else its `Name`), `harness::{TestIds, find, press, kit_app, spawn_ui, set_disabled, drain_actions}` (tests) |

The `crate::ui_theme` shim, the `frontend::widgets` palette re-export,
`MenuButton` and `frontend::widgets::{button, tile, compact_tile}` are gone:
every module imports `crate::ui::theme` (a few keep a local alias, `ui` or
`ui_theme`). `frontend::widgets` keeps the layout nodes (`screen_root`,
`panel_row`, `heading`, `label`) and the phone readability pass
(`adapt_phone_menu_readability`, which applies `metric::menu_font` and
`metric::menu_control_height`).

## Platform

`UiPlatform(UiProfile)` is inserted once by `UiKitPlugin` from
`platform::ui_profile()`. Systems read `Res<UiPlatform>` (career, collection,
card, home, help overlay); `MobileControls.enabled` is the runtime copy that
`MobileControlsPlugin` fills from it at build time, so a test that constructs
`MobileControls::default()` gets `enabled: false` and sets it itself.

## Order

`UiKitPlugin` configures `UiSet::Focus → Gesture → Scroll → Dispatch →
Paint` in `InputContextSet::Modal`, after `ModalSet::Early`:

0. **ModalSet::Early** – the registered modal sources update `ModalStack`
   (see [Modal registry](#modal-registry)).
1. **Focus** – `navigate_focus` moves the focused button on this frame's
   `FocusNav` messages and turns a confirm into a `SyntheticPress` (see
   [Focus](#focus)); it does nothing unless a driver enabled `UiFocus`.
2. **Gesture** – `recognize_presses` resets every `Pressable` (`touch_mode`
   from the platform, `activated = false`, `blocked` from the modal stack,
   `disabled` untouched), applies `SyntheticPress` messages, then feeds this
   frame's `TouchInput` (and, on a desktop build in touch mode, the left
   mouse button) to the `TapTracker`.
3. **Scroll** – `scroll_areas` moves every `ScrollArea` by wheel, page keys
   and touch drag (see [Scroll](#scroll)).
4. **Dispatch** – `dispatch_actions::<T>` (one per `add_ui_action::<T>()`)
   writes `Activated<T> { action, source }` for every button whose
   `Pressable::effective(Interaction)` became `Pressed` this frame. It runs on
   `Or<(Changed<Interaction>, Changed<Pressable>)>`, so a held click or a
   resting finger fires once.
5. **Paint** – `paint_pressables` colours `(Pressable, ButtonStyle,
   BackgroundColor)` from the effective interaction: idle, hover, or the
   kind's pressed colour (the hover colour except for `Skill` and an unowned
   `ShopItem`, which darken to `TILE` as they always did).
   `paint_focus_ring` places the focus ring on the focused button.

Modules consume `Activated<T>` after `UiSet::Dispatch` (the pause menu in
`PauseMenuSet::Visuals`). A handler that is gated (audio only while the
settings page is the front-most modal) still reads every message so a press
made while gated cannot fire later.

## Gesture semantics

`Pressable { touch_mode, activated, disabled, blocked }`:

- **Desktop** (`touch_mode == false`): `effective` returns Bevy's
  `Interaction` unchanged.
- **Touch**: a finger resting on the button is `Hovered`; the tap completes
  on `Ended` inside the same button rect and sets `activated` for one frame.
  Moving more than `TAP_SLOP` (10 px) cancels the tap for good (coming back does not revive
  it); only the first finger counts; `Canceled` clears it.
- `activated` wins in either mode, which is how `SyntheticPress(entity)`
  activates a button from a harness without touching `Interaction`.
- `disabled` (owned by the module): never hit-tested, never `Pressed`, never
  lit. No module sets it today.
- `blocked` (owned by the kit): set every frame when a modal is open and the
  button is not under the top modal's `ModalRoot`. Behaves like `disabled`,
  including for `SyntheticPress`. The pause menu is blocked this way while
  the phone server-address overlay covers it (this replaced
  `gate_buttons_behind_server_entry`).

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

## Scroll

`ScrollArea { wheel: Option<WheelScroll>, drag: Option<DragScroll>, key }`
on an `Overflow::scroll_y()` node (it requires `ScrollPosition`) replaces
the per-module scroll systems. `scroll_areas` (in `UiSet::Scroll`) is the
`mobile_ui`/`career` code made generic:

- **Wheel** (`line_step` per `MouseScrollUnit::Line`, `pixel_step` per pixel
  unit, default 1) and **PageUp/PageDown** (`page_step`, 0 = off) move every
  live area, or with `hover_only` only the one under the cursor.
- **Touch drag**: the first finger that starts on a live, overflowing area
  owns it until release or cancel (the smallest area wins where they
  nest); other fingers cannot steal it. Nothing moves until the finger is
  more than `threshold` logical px from its start; the first scrolling event
  catches up from the start point, then the content follows the finger.
  Touch positions are logical window pixels; the delta is converted with
  `window.scale_factor() * inverse_scale_factor`, so the content tracks the
  finger at any DPI and `UiScale`.
- **Platform**: each path is `All`, `Desktop` or `Phone`. `Phone` drag also
  needs the phone HUD landscape and focused; every drag needs the window
  focused.
- **Live**: visible (`InheritedVisibility`), measured (non-zero size) and
  allowed by the modal stack (with a modal open, only areas under the top
  modal's `ModalRoot`). A drag is dropped when its area stops being live.
- **Clamp**: `0 ..= (content − size) × inverse_scale_factor` (`max_offset`).
- **Key**: an area rebuilt under the finger (the draft panes) keeps the drag
  through the new entity with the same key; motion made while the new node
  is unmeasured is applied once layout measured it.

A tap and a scroll never share a gesture on phone menus: their threshold is
`TAP_SLOP`, the distance past which the recognizer cancels the tap for good.

| Panel | Module | Wheel | Page keys | Drag |
| --- | --- | --- | --- | --- |
| `PauseMenuMainSection`, `PauseMenuSettingsSection` | `pause_menu` | 32/line, desktop | – | `TAP_SLOP`, phone |
| `CareerBody` | `career` | 28/line, desktop | – | `TAP_SLOP`, phone |
| `HelpBody` (phone help cards) | `help_overlay` (also kept by `adapt_phone_layout`) | – | – | `TAP_SLOP`, phone |
| `ShopCards` | `mobile_ui` (inserted by `adapt_phone_layout`) | – | – | `TAP_SLOP`, phone |
| `SocialChatLog` | `social` | 28/line, desktop | – | – |
| `CombatTestBody` | `sandbox::ui` | 32/line, all | – | – |
| `DraftTeamRoster`, `DraftAvatarCatalogue`, `LoadingTeam-*` | `frontend::draft`, `loading` (`draft_pane`) | 24 per notch in either unit, under the cursor | – | from 0 px, keyed by pane |
| `CollectionGrid` | `frontend::collection` | 48/line | 300 | from 0 px |
| `AvatarGrid`, `SpriteCharacterGrid` | `team` | 48/line | 180 | – |
| supporter body | `supporter` | 24/line | – | from 0 px |
| `ScoreboardGreenRows`, `ScoreboardBlueRows` | `edge_hud` | 28/line, under the cursor | – | from 0 px |

Modules keep their own offset bookkeeping: the pause menu resets both
sections on navigation, career keeps the body offset across rebuilds of the
same page, the draft remembers pane offsets in `DraftScrollMemory`
(`remember_scroll`, after `UiSet::Scroll`), `team::restore_picker_scroll`
restores the model grid after a catalogue refresh. `scroll::harness`
(tests) spawns a window and the system, measures a node and sends wheel
lines or a drag.

## Modal registry

`ModalStack` is the list of open modals, bottom first: `push`, `pop` (from
anywhere), `set`, `is_open`, `contains`, `top`. It is ordered by
`ModalId::layer`, the z-index the modal's root is drawn at (shop 45,
scoreboard 90, pause 100, help 110, career 120, server entry 150, supporter
`GlobalZIndex` 1300), then by opening order, so `top()` is the modal in
front. `app.register_modal::<R>(id, |r| r.open)` adds a system that keeps
`id` in the stack while resource `R` exists and says open; it runs twice a
frame, in `ModalSet::Early` (before `UiSet::Focus`) and in `ModalSet::Late`
(the start of `InputContextSet::Resolve`). `InputContextPlugin` registers
all seven in one list (`register_modals`): `PauseMenuState.open`,
`CareerClient::modal_open`, `ShopState.open`, `SupporterUiState.open`,
`ScoreboardState.open`, `ServerEntry.open` (absent on desktop, so closed)
and `HelpOverlayShown` (the controls guide while it is on screen, over a
running match or a menu: its dismiss button is then the only focus
candidate and the HUD or Home under it waits). Each root carries
`ModalRoot(id)`.

`ModalGate` (a `SystemParam`) answers `owner(entity)` (the nearest
`ModalRoot` ancestor) and `allows(entity)`: everything while no modal is
open, otherwise only entities of the top modal. The recognizer sets
`Pressable::blocked` from it and the scroll system skips areas it does not
allow, so a button or panel outside the top modal (including front-end
screen buttons under a shell pause menu) does not react.

`input_context` reads `ModalStack::is_open()` in place of the six flags. It
keeps its own checks where the meaning is not "a modal panel is open":
help (blocks only during a running match), the sandbox (`blocks_world`
also covers teleport and field edit without a panel), social
(`blocks_gameplay` also covers the chat wheel and the frame after a send),
the front-end screen state, the hero picker root and a portrait or
unfocused phone.

## Focus

`ui::focus` is directional focus for kit buttons, for any input that is not
a pointer. Today the gamepad drives it (`docs/controller.md`); nothing in it
is controller specific.

- **Driver.** A driver calls `UiFocus::set_enabled(true)` each frame it owns
  the input and writes `FocusNav::{Up, Down, Left, Right, Confirm}`
  messages. While no driver is enabled the focus is dropped and no ring is
  drawn, so mouse and touch players never see it. The gamepad enables it
  while it owns input on a surface where gameplay is not allowed (a modal,
  a front-end screen, the help overlay).
- **Candidates.** Every `Pressable` that is not `disabled`, is allowed by
  `ModalGate` (the top modal's buttons, or the whole screen with no modal),
  is visible (its ancestors are walked directly, so a parent hidden this
  frame drops its buttons before visibility propagates) and is measured.
  A button clipped out of sight counts only inside a `ScrollArea`. They
  are ordered top-to-bottom, then left-to-right.
- **Navigation.** A direction moves to the nearest candidate that way,
  scored `along + |across| * 3` so the same row or column wins over a
  closer diagonal; at an edge the focus stays. When the set of candidates
  changes (a modal opened, a page changed, a section hid) or a modal bumps
  `GestureEpoch`, the first candidate is focused and the next `Confirm`
  must be a fresh press (a frame without one comes first).
- **Activation.** `Confirm` writes `SyntheticPress(focused)`, which the
  recognizer applies in `UiSet::Gesture` only when the button is neither
  `disabled` nor `blocked`: the same gate as a click or a tap, and the same
  one-frame `Activated<T>`. The click sound (`game_audio`) follows real
  pointer and key presses only, so a focus confirm is silent.
- **Scroll into view.** When the focus moves, every `ScrollArea` ancestor
  scrolls just enough (`reveal_delta`, clamped to the content) to show the
  focused button.
- **Ring.** `paint_focus_ring` draws one overlay node (`FocusRing`,
  `GlobalZIndex(5000)`, gold 3 px outline, not pickable) on the focused
  button's clipped rectangle. It is an overlay rather than an `Outline` on
  the button so outlines a screen owns (hero select's selected tiles) are
  left alone.

## Back

`ui::back::BackInput` is the one "back" signal: `Esc` plus `BackPress`, a
one-frame press from another source (a gamepad's East, written in
`PreUpdate`, cleared in `Last`). `just_pressed()` is either;
`pressed_on_keyboard()` is `Esc` only; `consume()` takes both, so later
readers this frame see nothing. It holds `ButtonInput<KeyCode>` mutably, so
a system that also reads other keys uses `keys()` / `keys_mut()`.

Every overlay that closes on back reads it where it always read `Esc`, with
the same schedule constraints, so the same overlay takes a press as before
(the first reader that acts consumes it):

| Reader | Scheduled | Closes |
| --- | --- | --- |
| `social::input` | `InputContextSet::Social` (before every modal) | chat or reaction wheel |
| `edge_hud::actions` | `Modal`, after `Dispatch`, before help and the shop | scoreboard |
| `help_overlay::toggle_help_overlay` | `HelpOverlaySet::Input` | help overlay (in a match or on a menu) |
| `shop::toggle_shop` | `ShopModalSet`, after help | shop |
| `supporter::keyboard_close` | `Modal`, before `toggle_pause_menu` | supporter panel |
| `career::dismiss_with_escape` | `Modal`, before `toggle_pause_menu` | career modal |
| `sandbox::ui::keys` | `Modal`, before `toggle_pause_menu` | Combat Test panel, teleport, field edit |
| `frontend::server_field::type_address` | before `PauseMenuSet::Close` | lobby server field edit |
| `pause_menu::toggle_pause_menu` | `PauseMenuSet::Close`, after help and the shop | toggles the pause menu |

The pause menu is the last reader: `Esc` opens or closes it; a back press
from another source only closes it (a gamepad opens it with Start, through
the `≡` button's path).

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

## Text and language

Widget captions are `impl crate::i18n::UiLabel`: a literal (`&str`,
`&String`, `String`) spawns a plain `Text`; a `Localized` key spawns
`(Text, Localized)` filled in the active language, and the i18n relabel
system rewrites it when Settings → Language changes. `button`,
`button_with_label`, `adjust_row`, `toggle_row` (label), `screen_button`,
`screen_tile`, `compact_screen_tile`, `screen_label` and
`frontend::widgets::{heading, label}` take it:

```rust
widgets::button(main, Localized::new("pause.button.settings"), ButtonKind::Secondary,
    PauseAction::OpenSettings, "SettingsButton");
widgets::icon_button(header, "×", ButtonKind::Secondary, PauseAction::Close, "PauseMenuCloseButton");
```

A caption the owner rewrites from state (the mute toggle) stays a literal
written with `tr`, re-run on a `Locale` change. `theme::apply_theme_font`
runs in `PostUpdate` (`I18nSystems::Font`, before UI and `Text2d` layout)
and picks the CJK font for UI and world text. `TestId`s and `Name`s are never
translated. `docs/i18n.md` has the patterns and the glossary.

## TestId policy

`TestId(Cow<'static, str>)` identifies every kit control and rewritable
value label; layout-only nodes keep a plain `Name`. The two are independent:
a `TestId` no longer mirrors itself into `Name` (item 6). Code that addresses
both kinds of node by string, the phone layout pass (`adapt_phone_layout`,
`phone_family`), the edge HUD layout and the QA node dumps, reads the
`NodeKey` query data (`key.as_str()`: the `TestId`, else the `Name`, empty
for neither); `crate::qa::QaName` is the same type. Under `cfg(test)`
`ui::test_id::harness` offers `find(world, id)`, `press(world, id)` (queues a
`SyntheticPress`) and the `TestIds` system param, plus `kit_app()`
(recognizer, dispatch sets, painter), `spawn_ui`, `set_disabled` and
`drain_actions::<T>` for screen tests. QA harnesses press kit buttons with
`SyntheticPress` (`audio_qa` directly, the rest through
`crate::qa::TestIdPresses`, see below) instead of writing
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
| `help_overlay.rs` | `HelpAction` | `dismiss_help_button` | the controls guide (`hud-help.md`): desktop cards + strip + dismiss, phone card list; see [Controls guide](#controls-guide-p0-c) |

QA harnesses press by `TestId` through `crate::qa::TestIdPresses` (and the
same lookup in `offline_qa`; `sandbox/ui/qa.rs` presses by action): every
button a harness presses is a kit button and gets a `SyntheticPress`, which
activates it once in desktop and touch mode and, like a real tap, does
nothing while a modal in front blocks it. The ids are the names the
harnesses always pressed. The harnesses' node dumps and lookups read
`QaName`, so a kit control is listed under its `TestId`.

## Responsive metrics (9b item 5)

`ui::theme::metric` is the one place that decides a phone or desktop size.
`Form::{Desktop, Phone}` (`Form::from_mobile(MobileControls)`) picks the
family; the functions return the pixel values the adapters used to compute
inline, unchanged:

| Policy | Desktop | Phone | Applied by |
| --- | --- | --- | --- |
| `menu_font(form, size, heading, ui_scale)` | designed size | at least 20 (heading) / 12 px ÷ `UiScale` | `frontend::widgets::adapt_phone_menu_readability` |
| `menu_control_height(form, height, ui_scale)` | designed height | at least `TOUCH_MIN` (44) ÷ `UiScale` | same |
| `pause_panel_height(form, in_settings, available)` | 560 settings / 380 main (`PAUSE_PANEL` 480×560 as spawned) | safe-area height / `min(height, 360)` | `pause_menu::size_desktop_pause_panel`, `mobile_ui::adapt_phone_layout` |
| `phone_class_column(width)` | – | `0.26 × width` in 150..=210 | `adapt_phone_layout` (hero select) |
| `phone_shop_card(width)` | – | `((width − 36) / 3, 103)` | `adapt_phone_layout` |
| `phone_font(PhoneText, original, width, ui_scale)` | – | the per-panel font table (bar ÷ `UiScale`, entry 11..=16, shop cards 12/15 below 650 px, shop 12..=18, summary 14, result 20, pause 14..=22) | `adapt_phone_layout` |
| `PHONE_PAUSE_W`, `PHONE_SERVER_W`, `PHONE_RESULT_W` | – | 650, 860, 640 caps | `adapt_phone_layout` |
| `PHONE_BAR_{HELP,MENU,SERVER,MIN}_W`, `TOUCH_MIN` | – | 48, 64, 88, 48 wide × 44 ÷ `UiScale` | `mobile_ui::sync_phone_ui` |

`theme::tests::metric_policy_keeps_the_phone_and_desktop_sizes` pins them.
The phone layout pass still positions each named node itself (safe-area
anchors and offsets are layout, not a size policy), and the gameplay HUD
docking (`match_hud::adapt_desktop_dock`, `minimap::adapt_minimap_edge`,
`shop::adapt_desktop_equipment_width`, `mobile_controls` layout) keeps its
world-relative geometry, which scales with `MobileControls::scale()`.

## HUD buttons on the kit (9b item 7)

| Module | Action type | Handler | Look |
| --- | --- | --- | --- |
| `combat/hotbar.rs` | `HotbarAction::{Cast(slot), Upgrade(slot)}` | `skill_button_system`, `skill_upgrade_input_system` (`InputContextSet::Actions`) | `ButtonKind::Skill` (slot: `PANEL`/`HOVER`/`TILE` pressed), `SkillUpgrade` (ready green) |
| `shop.rs` | `ShopAction::{Toggle, Close, Buy(item), QuickBuy(slot)}` | `toggle_shop` (after `Dispatch`), `purchase_buttons` (`Actions`) | item cards `ButtonKind::ShopItem` (`selected` = owned, `SHOP_OWNED`); gold button, quick-buy slots, OPEN SHOP and CLOSE keep their fixed colours |
| `mobile_ui.rs` | `PhoneAction` (bar and server entry) | `phone_menu_actions` (after `Dispatch`, before help) | fixed `TILE` |
| `edge_hud.rs` | `EdgeAction::{Score, Menu, Close}` | `actions` (after `Dispatch`) | fixed panel colours; the backdrop is a `Close` |
| `net/status_ui.rs` | `RetryPressed` | `handle_connection_retry_button` (`SessionRetryInput`, after `Dispatch`) | fixed green |
| `debug/hud.rs` | `DebugHudAction::{GodMode, SpeedBoost}` | `handle_debug_buttons` (after `Dispatch`) | `ButtonKind::Debug(DebugToggle)`, `selected` = on |

Each handler reads every `Activated<T>` before its gates (shop closed or
pending purchase, gameplay not allowed, debug access off), so a press made
while gated cannot fire later. These buttons sit outside every `ModalRoot`
except the shop cards and CLOSE (`ModalRoot(Shop)`), the scoreboard's Close
and backdrop (`ModalRoot(Scoreboard)`) and the server-entry keys
(`ModalRoot(ServerEntry)`), so with a modal open only the top modal's
buttons react.

## Verdant Crown foundation (0.27.0)

The kit is restyled on the approved Verdant Crown handoff
(`omoba-ui/handoff/`: `tokens.json`, `components/*.md|png`,
`assets/manifest.json`). Screens were not redesigned; they pick up the new
look where they use the kit. Screen steps (HUD, result, help, Home, …)
compose these parts.

### Tokens as data

- `client/ui/tokens/verdant-crown.json` is a byte copy of
  `omoba-ui/handoff/tokens.json`. `python3 scripts/sync_ui_tokens.py` copies
  and validates it (colour format, type-style fields, font files installed)
  and records its SHA-256 in `verdant-crown.lock.json`;
  `--check` (run by `scripts/test_sync_ui.py`) fails on a hand edit and, when
  the omoba-ui checkout sits next to the repo, on any drift from the handoff.
- `client/build.rs` (`build/ui_tokens.rs`) generates `OUT_DIR/ui_tokens.rs`,
  included by `ui::tokens`: `color::SURFACE_1`, `size::BUTTON_HEIGHT` (a
  `Metric { desktop, phone }`, `.at(form)`), `space::S16`, `radius::MD`,
  `border::FOCUS`, `motion::DURATION_PANEL_OPEN` (`Duration`),
  `motion::EASING_ENTER` (`CubicBezier::ease`), `FontFamily`, `TextRole` and
  `type_style::*`. A token the code names but the data lacks is a compile
  error; a key the generator cannot type fails the build;
  `generated_tokens_round_trip_the_json` checks every key against the tables.
- Naming: group → module, rest → upper-case constant; a `.desktop`/`.phone`
  pair → one `Metric`; a lone `.phone` keeps its suffix
  (`size::ABILITY_ATTACK_PHONE`); `space.8` → `space::S8`.
- Kit code (`ui/theme.rs`, `ui/widgets/`) names tokens only. Documented
  literal exceptions: the `OMOBA_DEBUG_UI` toggle colours, the legacy responsive screen policy in
  `metric` (`MENU_W`, `PAUSE_PANEL`, `phone_font`, … until the screens are
  redesigned) and component anatomy numbers the handoff states in px without
  a token (e.g. the 22 px level-up disc, 36 px chevrons, 280 px cycle
  control), each a named constant next to its component.
- The legacy palette names (`BACKDROP`, `PANEL`, `TILE`, `GOLD`, `IVORY`, …)
  are aliases of the tokens (mapping in `handoff/tokens.md`), so every
  screen already draws Verdant colours.

### Fonts and text roles

- Installed with the other assets (`client/assets/ui/verdant/fonts/`, OFL
  texts beside them): Cinzel SemiBold/Bold (display), Inter Regular/SemiBold
  (body), Barlow Condensed SemiBold/Bold (numbers), Noto Serif SC SemiBold
  (zh-Hans display subset). zh-Hans body text keeps the bundled Noto Sans CJK
  SC; text without a role keeps `ui/Inter.ttf` (`apply_theme_font`).
- `theme::TextStyle { role, keep_case, size }` on a `Text` picks family,
  size, `LineHeight` and case from `type.<role>.*`
  (`theme::role_text(TextRole::Button)`). `apply_text_styles` runs in
  `I18nSystems::Font` (after the relabel and every writer, before layout):
  - CJK text uses the role's CJK face when that face has every character
    (the Serif SC subset only holds the dictionaries' characters, read from
    its `cmap`), else Noto Sans CJK SC; number roles keep Barlow unless the
    text has CJK.
  - Latin text a display face cannot draw (a Cyrillic player tag in a
    heading) uses `type.name_lg`'s face at the same size, as written.
  - Upper case applies to the role's own Latin face only, never to CJK.
    `keep_case` leaves text that a module rewrites itself as written (the kit
    never fights an owner's writes; Cinzel draws lower case as small caps).
  - Size: `size.phone` on a phone with the `menu_font` minimums; on desktop
    never below `metric::DESKTOP_TEXT_FLOOR` (11) rendered px.
  - `UiForm` (from `UiPlatform`) is the layout family roles and sizes resolve
    for; the gallery switches it.
- Guard: `the_display_subset_covers_every_zh_hans_dictionary_character`
  fails when a zh-Hans string (outside the developer gallery's
  `kit.gallery.*`) has a character missing from the subset; re-run the
  handoff's `tools/fonts.py build` and `scripts/sync_ui_assets.py`.

### Assets

`python3 scripts/sync_ui_assets.py` installs every `ship: true` entry of the
handoff manifest (1x and 2x) into `client/assets/ui/verdant/` with the
licence texts and a trimmed `manifest.json`; `--check` verifies files, byte
counts and the 2.3 MiB budget (2,308,179 bytes today, fonts 1.78 MB).
`client/build.rs` (`build/ui_assets.rs`) turns that manifest into `Icon`,
`Frame`, `Sprite` and `Background` enums (paths, insets, atlas grids).
Widgets put `KitImage { source, tint, frame }` on a node;
`resolve_kit_images` keeps its `ImageNode` in step, loading `@2x` from 1.5
physical px per UI px (`UiDensity` = window scale × `UiScale`; the timer
ring stays 1x, `LowDensity`) and scaling 9-slice insets so corners keep their
logical size. `CoverImage` crops art like CSS `cover`. The game-icons.net
CC BY 3.0 credit line (with Lucide ISC and the font licences) is in Settings
(`pause.credits.icons`).

### Painting and states

- State model: idle · hover (pointer only) · pressed · focused · disabled
  (`Pressable::disabled`, never focusable) · selected (`ButtonStyle::selected`).
- `paint_pressables` owns the fill: from `ButtonKind` (legacy mapping), or
  from the control's `KitSkin` when the skin has its own fill rule (tabs are
  transparent until hovered, rows and fields keep their surface).
- `paint_slabs` draws the 9-slice slab of every Primary, Secondary, Danger
  and Team button (kit-spawned or screen-owned) per state
  (`frames/button-*`, a selected secondary shows its hover slab), unless the
  owner draws its own image (`NoSlab`, or a non-kit `ImageNode`). Team
  buttons get their 4 px team bar.
- `paint_kit` repaints kit-spawned controls' border, label, icon tint, press
  scale (`motion.press.scale`; abilities 0.94) and parts (`KitParts`: tab
  indicator, switch track/knob, slider thumb, cycle chevrons, ability rim).
- Focus ring (`paint_focus_ring`): 2 px `color.focus.ring` at 3 px
  (`FocusRingOffset` for the slider row: 6) with a `color.focus.halo` band
  filling the 6 px around the control, following its corner radius.
- `PreviewState { state, focused }` pins a control to a state (the gallery);
  `paint_preview_rings` draws a static ring for `focused`.
- `theme::perceptual(color)`: Bevy blends in linear light while the handoff
  sheets are browser (sRGB) renders, so translucent tokens drew too light
  (black veils) or too strong (light accents). The kit remaps their alpha with
  the display gamma for scrims, veils, glass, hairlines, bar tracks, damage
  trail and the focus halo. The token data is unchanged.

### Components

| Handoff sheet | Kit API |
| --- | --- |
| button | `button`, `button_with_label`, `screen_button`, `spawn_button(node, label, TextStyle, kind, icon, …)` with `button_node(ButtonSize::{Regular, Large, Hero}, kind, form)`; `ButtonKind::Link` is the tertiary text action (hover underline) |
| icon-button | `controls::icon_button`, `sized_icon_button(icon, form, …)`; header `widgets::icon_button("×")` keeps the 44 px touch minimum |
| stepper | `adjust_row` (`{id}-Down/-Value/-Up`), `controls::stepper_button` |
| tab | `controls::tab(label, icon, TabPlacement::{Top, Rail}, form, selected, …)` |
| toggle | `controls::toggle(label, on, action, id)` (row = hit target, `selected` = on, knob slides over `motion.duration.hover`); legacy `toggle_row` is a label/value slab button |
| slider | `controls::slider(label, value, form, id)` → `Slider { value, step }` + `SliderChanged`; drag, tap, Left/Right 5 % with hold-to-repeat |
| cycle-row | `controls::cycle_row(label, value, marker, previous, next, form, id)`: one focusable control, Left/Right or the chevrons |
| text-input | `controls::text_input(placeholder, icon, marker, action, id)`; `selected` = editing (caret), `InputError(true)` = error border |
| panel | `surfaces::plain_panel`, `framed_panel(form)` + `panel_header`, `ornament_frame`, `modal(title, close, form, id)` (opens with scale + scrim fade) |
| list-row | `surfaces::list_row(RowLeading, title, subtitle, action, id, trailing)` |
| badge | `surfaces::badge(text, BadgeKind, number)`; `surfaces::keycap(key)` is the ability key badge as an inline control legend |
| tooltip | `surfaces::tooltip_panel`; `Tooltip { title, body }` on a control shows it on hover (400 ms) or focus |
| toast | `surfaces::toast_panel`; `ToastRequest { kind, text }` shows a timed toast (max 3, in/hold/out) |
| bars | `game::bar(BarKind, BarValue, width, form, show_value)`; damage trail, dead state |
| ability-button | `game::ability_button(AbilityView, side, …)`, `ability_upgrade`; cooldown sweep and seconds, key, cost, pips, locked, no mana, ready flash, aiming glow |
| item-slot | `game::item_slot` (HUD), `item_slot_button`, `shop_card(ShopCard, form, …)` |
| hero-tile | `game::hero_tile(art, class, name, AvatarSource, form, …)`, `portrait` |
| scoreboard-row | `game::scoreboard_row(ScoreRow, form)` |
| timer-ring | `game::timer_ring(TimerRing, RingSize, time, caption)` |
| hud-plate | `game::hud_plate`, `plate_button` (`KitSkin::Plate`), `live_portrait`, `minimap_frame`, `player_status`, `score_strip`, `target_frame` (gallery composites; the live HUD composes the parts, see [In-match HUD](#in-match-hud-p0-a)) |
| help card (`hud-help.md`, P0) | `surfaces::info_card(node, icon, title, body, form) -> InfoCard { card, title, body }`: plain panel in `color.surface.2`, icon disc (32/24, `color.surface.3`, gold hairline) with a gold icon (20/16), `type.heading` gold title (16 px on a phone, `PhoneSized`), `type.body` secondary body; the caller sizes it and may add a legend row of `keycap`s and muted `badge`s |

### UI scale and preview cameras

- Desktop menus: `UiScale = clamp(min(w/1280, h/720), DESKTOP_SCALE_MIN, 2.0)`
  (R2.3; 1920×1080 → 1.5). A 1024×640 capture at 0.8 put the screens' legacy
  11–12 px labels at 9–10 px, so `DESKTOP_SCALE_MIN` is 1.0 until the
  screens use text roles (which never render below 11 px); screen steps lower
  it to 0.8. Windows smaller than the reference keep the legacy height shrink
  (`frontend::menu_scale`, down to `MIN_MENU_SCALE`) so short windows still
  fit the menus. Draft and loading stay at 1.0 on desktop: they lay out from
  the real window width. The desktop match follows the reference too (F8.1,
  see [In-match HUD](#in-match-hud-p0-a)): every world-anchored overlay
  divides its viewport point by `UiScale` (`hud_layout::world_to_ui`); below
  the reference it stays 1.0. Phones keep `frontend::menu_scale` and the
  `metric` minimums (the phone match is 1.0).
- Touch has no hover: in touch mode a held finger paints the pressed look
  (activation still happens on release) and a hovering pointer paints idle.
- The avatar preview and party stage cameras clear to transparent
  (`frontend::PREVIEW_CLEAR`, R2.4), so the menu background shows behind the
  models.

### Kit gallery

`OMOBA_UI_GALLERY=1 cargo run -p client` opens every component in every
state over the shell: F2 switches desktop/phone, F3 the language,
PageUp/PageDown or the tabs the page (buttons, controls, inputs, panels,
feedback, abilities, heroes, HUD, type specimen + icon sheet, focus over the
menu backgrounds). The first column of each row is live (hover, press,
gamepad focus); the others are pinned with `PreviewState`.
`OMOBA_UI_GALLERY_OUTPUT=<dir>` captures every page × profile × language at
1280×720 plus buttons and type at 1920×1080 and 1024×640 and exits.

### In-match HUD (P0-A)

The match HUD is built from `omoba-ui/handoff/screens/hud.md` (+ the
`target-*` and `skill-description` specs) with the parts above.

- **Layout** (`client/src/hud_layout.rs`): `HudLayout::desktop(viewport, pad)`
  / `HudLayout::phone(&MobileControls)` hold every region of the redlines in
  logical UI px (desktop edges anchored, phone on `MobileControls.safe`,
  top row 12 px from the screen top). An owner tags its root with
  `HudRegion::*` and `place_hud_regions` (PostUpdate, before layout) places
  it: `Box` (rect), `TopLeft`, `TopRight`, `TopCentre`, `BottomCentre`
  (grows upward). `hud_layout::tests` pin the redline coordinates;
  `ui_viewport(window, UiScale)` is the logical viewport and
  `world_to_ui(point, UiScale)` converts a camera projection (window px) to
  UI px for world-anchored nodes (boss plates, combat numbers, chat
  bubbles, the reaction wheel centre, the lock frame, the aim preview; the
  minimap converts its rect back for cursor hits).
- **Owners**: `match_hud` (status plate `MatchHudColumn`, buff chips
  `MatchBuffChips`, visibility), `edge_hud` (score strip `MatchScoreStrip` /
  `MatchScoreButton`, menu `MatchMenuButton`, target plate
  `TargetHealthRoot`), `combat::hotbar` (`SkillBarRoot`, `SkillUpgradeChip`,
  `ActionFeedback`, `SkillTooltip`), `combat::skill_card` (the shared
  desktop tooltip / phone hold card, `SkillCardView`), `shop` (desktop gold
  row `GoldShopButton` and `EquipmentHud`; phone `QuickBuyHud`), `social`
  (`SocialEntry`, `SocialStatus`), `minimap`, `team_vision`
  (`BrushStatus`), `net::offline` (practice badge / toast),
  `mobile_controls` (phone combat group).
- **Kit additions**: `KitSkin::Plate` + `plate_button` (a HUD plate that is
  a button: glass kept, border `gold.500` on hover), `live_portrait` /
  `PortraitView` / `paint_portraits` (art or icon, XP ring, level disc,
  dimmed when dead or offline, strong rim for bosses and the base),
  `ability_face` / `AbilityFace` (the ability face without a button, for the
  touch layer), `AbilityView::unlock_level` (the locked veil's `Lv N`; a
  veil without one is the dead state), a hugging cost pill and pips 18 px
  under the circle, `bar_parts` / `BarKindTag` (low-HP colours on the
  player's bar, `hud.target.defeated` for `respawn: Some(0)`, mana value
  `{mana}/{max}`), `TooltipText` (owner-written tooltip title/body), and
  `AbilityParts::key` (the key badge text; pad glyphs are written into it).
- **Target plate** (`edge_hud::target_details`): hero (nickname or class
  name, avatar or class icon, level disc from the live score, class icon,
  mana line on desktop, `edge.scoreboard.offline` muted when disconnected),
  minion (`edge.target.minion_melee|caster`), neutral (camp:
  `edge.target.neutral` + skull; boss: `boss.*` + crown, strong rim,
  `edge.target.boss` badge), structure (`edge.target.tower|base`, lane badge
  `lane.*`, lock and 55 % bar while protected). R7.1: a protected enemy
  structure is selectable for inspection (`TargetValidity::inspectable`;
  `valid` stays the attack rule): a click with nothing attackable under the
  pointer selects it, a right-click is refused with `combat.cast.protected`,
  and `clear_invalid_selection` keeps it while dropping any attack order.
- **Phone combat group** (`MobileControls::layout`): ATK 96 at safe
  right/bottom − 76, abilities 64 on R 104 (162°, 204°, 246°, 288°),
  DASH/HASTE/CANCEL/RANK/MIN/TWR 48 on R 168 (166° … 276°), joystick r 52 at
  safe left + 68 / bottom − 65; all × `combat_scale()`. The rank ring is the
  ability's disc with the + badge (hint) and, in rank mode, a gold rim.

### Known differences from the handoff sheets

- **Letter spacing:** Bevy 0.18 text has no tracking, so `letter_spacing_em`
  stays data; display text is set without the sheets' 0.06–0.18 em.
- **Ellipsis:** Bevy text cannot ellipsize; single-line labels clip.
- **Desaturation:** Bevy UI images cannot be desaturated; unaffordable or
  locked art is dimmed (`color.text.secondary` / `.disabled` tint).
- **Opacity:** nodes have no opacity; the unaffordable shop card dims its
  text and icon instead of 55 % opacity, the modal fades its scrim only.
- **Cycle row disabled** hides its chevrons (`cycle-row.md`); the sheet
  still draws them.
- **Toast icons** use the accent colour (`toast.md`); the sheet draws them
  neutral.
- **Header close button** keeps 44 px on desktop (pause menu contract); the
  kit icon button is 40.
- **HUD motion** (hud.md, target-hero.md): the target plate appears and
  leaves at once (no fade: nodes have no opacity), and a defeated target is
  cleared in the frame it dies (no drain + greyscale hold); the damage trail
  on its HP bar and the cooldown ready flash are drawn.
- **Dashed CANCEL rim** (phone) is solid `color.state.danger` (Bevy borders
  are solid).

## Controls guide (P0-C)

`help_overlay.rs` builds the guide from `omoba-ui/handoff/screens/hud-help.md`
at its authored size (desktop 1280×720, phone 844×390):

- **Desktop:** a framed panel 1088×672 (`HelpPanel`) with the eyebrow and
  title rows, a grid of six `info_card`s 336×192 (`HelpCard-move|attack|
  target|abilities|shop|objective`, grid `HelpBody`) whose legend row
  (`HelpCardInputs`) holds muted badges for mouse inputs
  (`help.input.left_click|right_click`) and keycaps for keys (the ability
  keys from `SKILL_SLOT_KEY_LABELS` and the upgrade key), the field/camera
  strip (`HelpField`, `HelpCamera`: an inline `type.eyebrow` label and a
  `type.caption` span), the 360-wide primary large dismiss
  (`HelpDismissButton`, `help.dismiss.button`) and the reopen hint
  (`HelpReopenHint`, `help.reopen`). A menu and the desktop match (F8.1)
  scale it through `UiScale`; a window below the reference (`UiScale` 1.0)
  scales the panel down (`UiTransform`) so the whole guide stays on screen.
- **Phone:** a plain panel in the safe area (`MobileControls.safe` +
  `space.screen_margin.phone`, 12 from the top and the safe bottom) with a
  title row (`help.phone.title` and the 240-wide dismiss) over a touch scroll
  list (`HelpBody`) of ten cards 96 high in two columns and the closing line
  (`help.phone.footer`); a 4 px gold thumb shows the scroll position.
- It opens with `motion.duration.panel_open` (scale in, scrim fade), is a
  modal (`ModalId::Help`) while shown, and closes with its button, Esc, a
  gamepad East or F1. From Settings → Controls (`pause.settings.controls`)
  closing it reopens Settings (DECISIONS R6.5, `SettingsHelpReturn`); from the
  Game menu it returns to the game.
- `OMOBA_HELP_QA_SHOTS=<dir>` (with `OMOBA_QA_WIDTH/HEIGHT`, `OMOBA_LANGUAGE`,
  `OMOBA_TOUCH_CONTROLS`) captures the controller focus on the dismiss
  button, the scrolled list, the close, and the Settings round trip.

## Remaining migration

In the order the roadmap intends, each a PR of its own:

1. **Scroll** – done: `ui::scroll::ScrollArea` and `scroll_areas` replace
   `TouchScrollPanel` and every per-module scroll system (the table under
   [Scroll](#scroll)).
2. **Modal registry** – done: `ui::modal::ModalStack` gates gestures and
   scroll, replaces the server-entry `Pressable::disabled` juggling and six
   of the `input_context` checks (see [Modal registry](#modal-registry)).
3. ~~Front-end screens~~ – done (see above).
4. ~~Career, social, supporter, sandbox~~ – done. The Combat Test panel moved
   completely: its text fields are keyboard-driven (`edit_keys`) and
   teleport picking is a world click (`teleport`), neither is a button.
5. **Responsive layout** – done: `ui::theme::metric` answers every
   phone/desktop size the readability pass, the phone layout pass, the phone
   bar and the desktop pause panel apply (see
   [Responsive metrics](#responsive-metrics-9b-item-5)).
6. **TestId in QA** – done: harness presses by `TestId`
   (`crate::qa::TestIdPresses`), dumps through `QaName`, and the `Name`
   mirror is gone (see [TestId policy](#testid-policy)).
7. **Colours and the last own buttons** – done: the combat skill bar, the
   shop, the phone bar and server entry, the edge HUD and scoreboard, the
   connection Retry button and the debug HUD toggles are kit buttons (see
   [HUD buttons on the kit](#hud-buttons-on-the-kit-9b-item-7)). None
   needed hold semantics: the desktop skill bar casts on press, and the
   phone's hold-to-repeat attack and drag-to-aim skills are
   `mobile_controls` touch input, not buttons. Buttons that still read
   `Interaction` do so only to keep world clicks off the UI
   (`combat::selection`, `player::input`) or to play the click sound
   (`game_audio`).
8. **Focus and back** – done (0.25.0): `ui::focus` and `ui::back` (see
   [Focus](#focus) and [Back](#back)); every `Esc` site reads `BackInput`.
