# 2026-09-28 — Verdant Crown UI foundation (0.27.0)

Proof-loop task `UI-FOUNDATION` (rebrand steps F1–F6, program HQ
`omoba-ui/rebrand/`). Goal: tokens, fonts, a restyled kit with every
component state, assets, UI scale and a gallery, so the screen steps (P0–P2)
only compose kit parts. Screens were not redesigned.

## Changes

- **Tokens as data (F1, DECISIONS R4):** `client/ui/tokens/verdant-crown.json`
  is a hash-locked byte copy of `omoba-ui/handoff/tokens.json`
  (`scripts/sync_ui_tokens.py`, `--check`); `client/build.rs` generates typed
  constants (`ui::tokens`: colours, `Metric` desktop/phone sizes, spacing,
  radii, borders, motion with `Duration`/`CubicBezier`, `FontFamily`,
  `TextRole`/`TypeStyle`) and round-trip tables. The legacy palette names are
  token aliases, so every screen draws Verdant colours.
- **Fonts and text roles (F2):** Cinzel, Inter, Barlow Condensed and a Noto
  Serif SC subset installed with their OFL texts; `TextStyle` roles pick face,
  size per profile, line height and case, pair CJK faces by coverage (read
  from each face's `cmap`), keep owner-written labels as written and hold an
  11 px floor on desktop.
- **Kit restyle (F3):** 9-slice slabs per state for primary, secondary,
  danger and team buttons (kit and screen-owned), tertiary text buttons,
  native tiles, new focus ring with halo, `PreviewState`, and every handoff
  component: icon button, stepper, tabs, toggle, slider, cycle row, text
  input, panels and modal, list row, badge, tooltip, toast, bars, ability
  button, item slot and shop card, hero tile and portrait, scoreboard row,
  timer ring, HUD plates. Focus: `FocusAdjustable` (slider, cycle row take
  Left/Right with hold-to-repeat), `FocusSkip`.
- **Assets (F4):** 121 handoff assets (1x + 2x) in `client/assets/ui/verdant/`
  (`scripts/sync_ui_assets.py`, budget check), typed as `Icon`/`Frame`/
  `Sprite`/`Background`; `KitImage` resolves density and 9-slice insets.
  Credits line (game-icons.net CC BY 3.0, Lucide ISC, fonts OFL) in Settings.
- **UI scale and previews (F5):** desktop menus use R2.3's formula with the
  floor raised to 1.0 after the 1024×640 legibility check (legacy labels hit
  9–10 px at 0.8); the desktop match stays 1.0 until world-anchored overlays
  divide by `UiScale`. Preview and party-stage cameras clear to transparent
  (R2.4).
- **Gallery (F6):** `OMOBA_UI_GALLERY=1` / `OMOBA_UI_GALLERY_OUTPUT=<dir>`
  (`qa/ui_gallery.rs`), 10 pages × desktop/phone × en/zh-Hans plus
  1920×1080 and 1024×640 shots.
- `theme::perceptual`: translucent tokens remapped for Bevy's linear-light
  blending so they read like the browser-rendered handoff sheets.

## Checks

See `CHANGELOG.md` (0.27.0) for the gate numbers; proof-loop evidence is in
`.agent/tasks/UI-FOUNDATION/` (gallery captures, before/after `ui-audit`
captures of existing screens, handoff side-by-sides, raw gate logs).

## Remaining risks

- Letter spacing, ellipsis, image desaturation and node opacity are not
  available in Bevy 0.18 UI (listed in `docs/ui-kit.md`).
- Hero select's wallet/account buttons (`ButtonKind::Link`) now render as
  tertiary text; the mapping says Secondary + link icon (screen step).
- The desktop match does not scale yet (world-anchored overlays).
- Physical phone and controller checks, CI and `ios-check.yml` run after the
  push.
