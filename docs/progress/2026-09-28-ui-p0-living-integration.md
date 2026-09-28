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
