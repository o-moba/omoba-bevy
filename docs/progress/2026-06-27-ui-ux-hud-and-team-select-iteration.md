# 2026-06-27 — UI/UX iteration: team-select labels + HUD stat bars

## Goal
Review the open-source omoba client UI and ship one low-risk UX iteration on the
two highest-friction surfaces: the team/character select (onboarding) screen and
the in-match HUD.

## Review findings
- `client/src/team.rs`: team buttons were bare colored squares with no text — the
  first screen gave no word for "Green"/"Blue" or that clicking joins the match.
- `client/src/match_hud.rs`: HP/Mana were plain white numbers, no bars, no color.
  Hard to read at a glance during a fight; status block is a dense paragraph.
- `client/src/minimap.rs`, `pause_menu.rs`, `game_state.rs`: adequate for the
  prototype; left unchanged this pass.

## Changes
- `team.rs`: extracted `spawn_team_button`, added a `Green`/`Blue` text label
  (`team.as_str()`) inside each team button, and added a flow hint line under the
  team row.
- `match_hud.rs`: added a `HudBarsRoot` container with two color-coded bars
  (`HpBarFill`, `ManaBarFill`) via `spawn_stat_bar`. `update_stat_bars` drives fill
  width (`Val::Percent`) from `CombatStats` and tints HP green/amber/red by ratio
  (`hp_bar_color`). Bars show only while `GameState::Running`; hidden otherwise.
  Existing numeric status text and its unit tests are untouched.

## Checks
- `cargo build -p client` — pass (Bevy 0.18).
- `cargo test -p client match_hud` — 2 passed.

## Remaining risks / follow-ups
- Bars use a fixed 200px track; not yet responsive to window width.
- HUD status text still redundant (cast keys repeated across lines) — a later pass
  could collapse it now that bars carry HP/Mana visually.
- No live visual capture taken (headless); recommend an in-app screenshot pass.
