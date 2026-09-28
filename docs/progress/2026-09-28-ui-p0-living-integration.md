# 2026-09-28 — P0 UI integration and living scenes (0.28.0)

The unmerged Verdant Crown HUD and result branches were integrated on
`codex/ui-release-integration`, their QA conflicts were combined, and F11 from
the design handoff was implemented on top.

## Player-facing result

- The desktop/phone HUD and result/loading work now coexist in one branch.
- Home and searching use the Stage scene; party lobby, collection and profile
  use Arena; post-match selects Victory or Defeat from the outcome.
- Career, Supporter and Settings add Arena only outside a running match. Live
  match menus intentionally keep the world behind their panels.
- Settings has a persisted Reduce motion toggle. It keeps static scene layers
  and fades, but removes parallax, rays, fog, runes and particles.
- Living art loads 720p on phones and small desktop windows, 1080p above an
  800 px physical desktop height. A phone session falls back to plate-only at
  menu frame-time p90 above 25 ms.
- Controls on painted scenes use the handoff's dark focus keyline. Result
  summary copy has a glass legibility plate for the brightest victory frame.

## Implementation

`ui/living_background.rs` is ordinary Bevy UI: cover-cropped plate and
foreground layers, tintable shared sprites, post-layout `UiTransform` motion
and deterministic particles. It does not use a shader or mutate layout every
frame. Back/front particles are separated around the foreground in tree order;
only the two photographic layers use the special 720/1080 density rule.

The client enables Bevy WebP and installs the handoff assets through the
existing manifest/token sync pipeline. `client_preferences.json` schema 6 adds
the optional `reduce_motion` boolean.

## Verification

- `scripts/sync_ui_tokens.py --check`
- `scripts/sync_ui_assets.py --check`
- `scripts/capture_ui_audit.py --check` — 55 screens, 13 documented gaps
- Native GPU kit gallery — 44/44 captures completed before the final z-order
  and transition-safety fixes; focused recaptures checked the living page.
- Native GPU result QA — eight fixtures completed (victory, defeat, loading
  and controller-focus states).
- `cargo test -p client --lib --locked` — recorded in the task evidence.

The remaining release-candidate checks are physical iPhone/Android performance
and a physical gamepad focus pass; neither is certified by desktop preview.

## Home and settings release polish

The follow-up replaces Home's interim full-width opaque panel with the P1-A
Stage composition. Desktop now uses the ornament frame, separate identity,
showcase, hero plate, hero-sized PLAY control and four icon navigation tiles.
Phone owns its 844×390 safe-area layout and rebuilds once the menu `UiScale`
settles, so the identity chip, hero, actions and bottom navigation render in
physical-looking redline pixels instead of shrinking a second time.

The same native shell pass exposed a stacking regression in Settings: the
Arena background was inserted after the existing panel and covered it. The
panel now has an explicit foreground z-layer. `App::run()` errors are also
propagated to the process, making harness failures non-zero.

Verification after the polish: `cargo test -p client --lib --locked` — 764
passed, 0 failed, 1 ignored; native shell QA — 11 desktop and 14 phone frames.
The exact Settings tab-rail redesign, Draft/Loading redline migration and the
remaining P2 screens stay on the tracked roadmap rather than being claimed by
this release-polish commit.
