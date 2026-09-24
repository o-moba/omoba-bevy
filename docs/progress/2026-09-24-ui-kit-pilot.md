# 2026-09-24 — UI kit pilot (pause menu, practice sandbox)

## Goal
Roadmap step 9: one client UI kit instead of every overlay owning its palette,
tap handling, button painting and marker components. This is the first three
PR-sized steps: the kit skeleton and theme, the shared tap recognizer, and the
typed actions and widgets, piloted on the pause menu and the practice sandbox.

## Changes
- `client/src/ui/{mod,theme,gesture,action,widgets,test_id}.rs` — the kit;
  `docs/ui-kit.md` documents the API, the gesture semantics, the `TestId`
  policy and the remaining migration steps.
- `UiPlatform` replaces `CareerUiProfile` and the direct `ui_profile()` calls in
  career, collection, card, home, help overlay and `frontend::widgets`;
  `MobileControlsPlugin` copies it into `MobileControls.enabled`.
- `pause_menu.rs`: 19 marker structs and 10 handler systems became
  `PauseAction`, `Setting`, `SettingLabel` and `apply_pause_{navigation,
  settings,audio,session}` plus `update_setting_labels`; buttons come from
  `ui::widgets`, colours from `paint_pressables`, Resume is
  `ButtonKind::Primary`. `practice_sandbox.rs`: `PracticeButton` became
  `PracticeAction` read from `Activated<_>`.
- Career: `CareerTap`/`collect_career_taps` deleted; buttons carry `Pressable`,
  `bump_gesture_epoch_on_navigation` on modal changes, `actions` reads
  `Pressable::effective`.
- `audio_qa` presses kit buttons through `SyntheticPress`; the other QA
  harnesses only press non-migrated buttons.
- `edge_hud::MatchHelpButton` removed (it was only the pause menu's marker).

## Checks
- `cargo test -p client --lib`, `cargo clippy --workspace --all-targets
  --no-deps -- -D warnings`, `cargo fmt --all`, `cargo test -p shared -p server`.
- The named pause and career tests kept passing with renames plus the resource
  registrations the generic recognizer needs (`UiPlatform`, `Activated<_>`).
