# 2026-09-25 — UI kit: scroll and modal registry (roadmap step 9b, items 1-2)

## Goal
Items 1 and 2 of [ui-kit.md](../ui-kit.md) "Remaining migration", in the
scoped form of [ARCHITECTURE_REPORT.md](../ARCHITECTURE_REPORT.md) §8.3 row 9b
and O23: one `ScrollArea` built from the already-debugged `mobile_ui`/`career`
code for every scroll system, and a minimal modal registry. `ScreenMetrics`
stays rejected. No wire change.

## Scroll (`client/src/ui/scroll.rs`)
- `ScrollArea { wheel: Option<WheelScroll>, drag: Option<DragScroll>, key }`
  with builders (`menu`, `phone_panel`, `wheel`, `desktop_wheel`,
  `pixel_step`, `page_keys`, `hover_only`, `touch_drag`, `keyed`);
  `scroll_areas` in the new `UiSet::Scroll` (between `Gesture` and
  `Dispatch`), registered by `UiKitPlugin`.
- Touch path from `mobile_ui::scroll_phone_panels`: first finger on a live,
  overflowing area owns it (smallest area wins), threshold then catch-up
  from the start point, logical px × `scale_factor` × `inverse_scale_factor`,
  clamp to content; hidden/unmeasured/blocked areas drop the drag. From the
  draft: the `key` lets a drag follow a pane rebuilt under the finger and
  keeps motion made while it is unmeasured.
- `gesture::TAP_SLOP` (10 px) is shared by `TapTracker` and the phone menu
  drag threshold, so a drag never presses.

Migrated (all removed):

| Old system | Panel(s) | Parameters now |
| --- | --- | --- |
| `mobile_ui::scroll_phone_panels` (+ `TouchScrollPanel`) | `HelpBody`, `ShopCards`, pause sections, `CareerBody` | drag `TAP_SLOP`, phone |
| `pause_menu::scroll_desktop_settings` | `PauseMenuMainSection`, `PauseMenuSettingsSection` | + wheel 32/line, desktop |
| `career::scroll_desktop` | `CareerBody` | + wheel 28/line, desktop |
| `social::scroll_chat` | `SocialChatLog` | wheel 28/line, desktop |
| `sandbox::ui::scroll` | `CombatTestBody` | wheel 32/line, all |
| `frontend::draft::scroll_panels` | draft and loading panes (`draft_pane`) | wheel 24 per notch (line or pixel), hover; drag 0, keyed |
| `frontend::collection::scroll_collection` + grid part of `drag_to_rotate` | `CollectionGrid` | wheel 48/line, page 300, drag 0 |
| `team::scroll_avatar_roster` | `AvatarGrid`, `SpriteCharacterGrid` | wheel 48/line, page 180 |
| `supporter::scroll_panel` | supporter body | wheel 24/line, drag 0 |
| `edge_hud::scroll_rows` | `ScoreboardGreenRows`, `ScoreboardBlueRows` | wheel 28/line, hover; drag 0 |

Module bookkeeping that stays: `pause_menu::reset_pause_scroll_on_navigation`,
career's offset preservation across rebuilds, `draft::remember_scroll`
(writes `DraftScrollMemory`, previously inside `scroll_panels`),
`team::restore_picker_scroll`, and `drag_to_rotate`'s grid-touch ownership
(blocks preview rotation and, after 4 px, collection actions).

## Modal registry (`client/src/ui/modal.rs`)
- `ModalId` (`Pause`, `Career`, `Shop`, `Supporter`, `Scoreboard`,
  `ServerEntry`) with `layer()` = the root's z-index; `ModalStack`
  (`push`/`pop`/`set`/`is_open`/`contains`/`top`) ordered by layer, then
  opening order.
- `ModalAppExt::register_modal::<R>(id, fn(&R) -> bool)`: a sync system in
  `ModalSet::Early` (in `InputContextSet::Modal`, before `UiSet::Gesture`) and
  `ModalSet::Late` (start of `InputContextSet::Resolve`). The six
  registrations live in `input_context::register_modals`.
- `ModalRoot(id)` on `PauseMenuRoot`, the career modal root, `ShopRoot`,
  `SupporterRoot`, `ScoreboardRoot`, `ServerEntryRoot`. `ModalGate`
  (`SystemParam`) resolves the owner by walking `ChildOf`.
- `Pressable::blocked` (kit-owned, set by `recognize_presses`): outside the
  top modal a button is not hit-tested, not `Pressed` (also for
  `SyntheticPress`), not lit. `pause_menu::gate_buttons_behind_server_entry`
  deleted.
- `input_context::resolve_input_context` reads `ModalStack::is_open()` for
  the six flags. Kept as own checks: help (only while running), sandbox
  `blocks_world` (teleport/edit without a panel), social `blocks_gameplay`
  (chat wheel, the frame after a send), front-end screen state, the hero
  picker root entity, phone portrait/unfocused.

## Behaviour notes
- Top-only input: buttons and scroll panels outside the top modal no longer
  react (shell pause menu over a front-end screen, career modal over the pause
  menu, any modal over the Combat Test body or chat log).
- Draft, loading, collection, supporter and scoreboard touch scrolling now
  converts the delta by `UiScale` like the phone menus, hit-tests with the
  DPI-corrected clipped rect, and must start on the panel (supporter and
  scoreboard accepted a drag from anywhere). Identical at `UiScale` 1 and DPI 1.
- A second finger on the collection grid while the first turns the preview
  scrolls the grid.
- Scrolling of the pause menu, career, phone panels, social and supporter
  moved from `PostUpdate` to `Update` (`UiSet::Scroll`); it still reads the
  previous frame's layout.

## Tests
- New: `ui::modal` (stack order by layer, registered sources and top-only
  gate), `ui::scroll` (wheel steps and clamp, drag threshold without a
  press, clamp and top-modal gating, keyed rebuild), pause
  `server_entry_over_the_pause_menu_blocks_its_buttons_until_it_closes`,
  `input_context` `gameplay_and_camera_are_blocked_while_any_modal_is_open`,
  one per migrated panel (pause and phone panels: existing layout/touch tests
  plus `help_and_shop_panels_become_phone_scroll_areas...`; career, social,
  sandbox, supporter, collection, scoreboard new; draft and team rewired).
- client lib 571 → 586 (566 without `qa`), shared 92.

## Checks
`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets
--no-deps -- -D warnings`, `cargo clippy -p client --lib --no-deps
--no-default-features -- -D warnings`, `cargo test -p client --lib` (and
`--no-default-features`), `cargo test -p shared`.
