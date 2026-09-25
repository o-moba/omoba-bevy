# 2026-09-25 — UI kit: screens on typed actions (roadmap 9b, items 3 and 4)

Items 3 and 4 of [ui-kit.md](../ui-kit.md) "Remaining migration"; tracker
row 9b in [REFACTORING.md](../REFACTORING.md). Scroll (item 1) and the modal
registry (item 2) are a separate PR and were not touched: every `scroll_*`
system and the `input_context` modal checks are as they were.

## Pattern

Each module registers `app.add_ui_action::<T>()`, puts `UiAction(T::…)` on
its buttons (with `ButtonStyle` where the old code painted hover colours and
a `TestId` carrying the old `Name`), and reads `Activated<T>` in a system
ordered `.after(UiSet::Dispatch)`. A handler that is gated clears its reader
so a press made while gated cannot fire later.

## Per module

- **Home** (`frontend/home.rs`): `HomeAction` (was a marker component);
  `home_actions` reads `Activated<HomeAction>`.
- **Card** (`frontend/card.rs`): `CardAction`; class and title tiles are
  `screen_tile`s whose `ButtonStyle::selected` `refresh_card_screen` flips;
  the accent swatches keep their own colour (no `ButtonStyle`).
- **Collection** (`frontend/collection.rs`): `CollectionAction`; avatar
  tiles, clip strip and showcase/equip tiles on `ButtonStyle`. While the
  preview drag owns the pointer (`block_actions`) presses are cleared, as the
  two existing drag tests pin.
- **Searching**, **Loading**, **Post-match**: `SearchingAction`,
  `LoadingAction`, `PostMatchAction` (were markers). Loading shares
  `draft::action_button`, now generic over the action type.
- **Draft** (`frontend/draft.rs`): `DraftAction`; `DraftSet::Input` is
  configured after `UiSet::Dispatch` (draft and loading handle presses
  there). Leave still wins over anything else pressed in the same frame.
- **Hero select** (`team.rs`): `HeroSelectAction::{Back, Class, Avatar,
  Sprite, LockIn(Team), ConnectWallet, ConnectAccount, RefreshStudio}`.
  `team_select_ui_system` is now only the press handler (after `Dispatch`,
  before `SendCommands`); `LockIn` calls `team::lock_in` as before. Class,
  avatar and sprite buttons are `ButtonKind::Tile` (the old
  `SELECT_BUTTON_*` colours are the tile colours), the Ekza buttons
  `ButtonKind::Link` and the lock-in `ButtonKind::Team(team)`; the colours
  moved to `ui::theme` (`LINK`, `LINK_HOVER`, `TEAM_*`) and
  `Team::ui_color`/`ui_hover_color` were removed. The wallet/account press
  handling left `wallet_connect_ui_system` (status and rebuild stay) and
  `refresh_picker_catalogue` is gone (the handler does it).
  `adapt_mobile_selection_contrast` runs after `UiSet::Paint`.
- **Career**: the existing `Action` enum is the `UiAction` type;
  `actions` reads `Activated<Action>` (career chain after `Dispatch`
  instead of after `Gesture`). Its own `Interaction`/`Pressable::effective`
  reading is gone; the tab buttons, which had no `Pressable`, now use the
  kit too. `web::act` and `devices::act` are unchanged.
- **Social**: `SocialAction` presses are applied by a new `social_actions`
  in `InputContextSet::Modal` after `Dispatch` and before `CareerUiSet`.
  `input` still runs in `InputContextSet::Social` (before the kit) and
  leaves a `ButtonFrame` (hero present, hero point, viewport, scale) in
  `SocialClient`; a frame on which it returned early leaves none and
  `social_actions` drops the presses.
- **Supporter** (StoreKit panel): `supporter::Action`; a disabled button is
  `Pressable { disabled: true }` (it used to have no action component).
  No `ButtonStyle`: these buttons never had a hover colour. Ids
  `SupporterClose`, `SupporterAura-{id}`, `SupporterEquip`, … .
- **Combat Test panel** (`sandbox/ui.rs`): moved completely. Buttons, tabs,
  value fields and toggles carry `UiAction<Action>` and `CombatTest…` ids;
  the value fields are only an `Edit` press (typing is `edit_keys`) and
  teleport picking is a world click (`teleport`), so nothing was risky. The
  panel chain runs after `Dispatch`; a closed panel clears the presses.
  `button` keeps `ButtonKind::Secondary`; fields and toggles stay flat.
- **Help overlay**: `HelpAction::Dismiss`, because the dismiss button was
  the last `MenuButton` user.

## Retired

`MenuButton`, `frontend::widgets::{button, tile, compact_tile}` with
`paint_buttons`/`repaint_changed_buttons`, `client/src/ui_theme.rs`, and the
palette re-export in `frontend::widgets`. `MenuTypography` and `MenuControl`
moved to `ui::widgets`; `frontend::widgets` keeps the layout nodes and the
phone readability pass (item 5).

## QA

`crate::qa::NamedPresses` presses by `Name`: `SyntheticPress` for a kit
button, `Interaction::Pressed` otherwise. `frontend_qa`, `frontend_flow_qa`,
`frontend_qa/avatar`, `beta_ui_qa`, `targeting_qa`, `navigation_qa`,
`map_qa`, `combat_qa`, `forest_pickup_qa`, `team_vision_qa`, `edge_hud_qa`
use it, `offline_qa` applies the same rule in its exclusive system, and
`sandbox/ui/qa.rs` sends `SyntheticPress` by action. Every pressed name is
unchanged. `social_qa` and `career_visual_qa` press no migrated button.

## Tests

One test per module that a press dispatches its action once (a second frame
does not repeat it) and a disabled button does nothing: home, card,
collection, searching, draft, loading, post-match, hero select (class tiles
and the lock-in through `lock_in`), career (also: a press while social is
gated is dropped), social (a gated frame drops it), supporter (a rendered
disabled button), Combat Test (a closed panel drops it), plus a theme test
for the new kinds. `ui::test_id::harness` gained `kit_app`, `spawn_ui`,
`set_disabled` and `drain_actions`. Existing tests needed only mechanical
edits (collection drag tests and the post-match retirement test spawn
`UiAction` instead of the marker; the help test is unchanged).
- Counts: client lib 577 → 591 (557 → 571 without `qa`); server 298 (+3 ignored), shared 94, passport 26 unchanged; Python script tests 124 (+1 skipped).
