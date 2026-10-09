# Boot splash, surroundings orientation and iPad preview hold — 2026-10-06

Candidate: 0.44.0, `fix/boot-splash-surroundings-preview` (base `2eaa678`).
No server deployment, mobile archive, push or merge is part of this session.

## Goal

Owner feedback from an iPad playtest: the game opens on a black/empty screen
for a while; the painting below the world is turned the wrong way; the
iPhone 16 layout preview switches on and drops back to full screen.

## Changes

- `client/src/frontend/boot.rs` adds `BootSplashPlugin` and `ModalId::Boot`
  (`GlobalZIndex` 2100). The splash is spawned at `Startup`, tracks the theme
  fonts (the CJK body face only when the status line needs it) and the six Verdant world scenes, and counts a
  failed asset as settled. Phases: Loading → Closing (curtain in, 0.25 s) →
  Revealing (content despawned, curtain out, 0.4 s) → Off. Bevy UI has no group
  opacity, hence the dip through `color.bg.base` rather than a cross-fade.
  `enabled_for` shows it only when no `OMOBA_*` switch outside a short
  player-facing list is set, so existing and future harnesses are unaffected.
  After the independent verification pass: the pause and help key toggles wait
  for the splash, the 15 s timeout is wall-clock (splash time is clamped per
  frame), the CJK body face is awaited when the status line needs it, and the
  splash layer (2100) sits above the controller legend.
- `client/i18n/{en,ru,zh-Hans}/boot.json`: status lines; the tagline reuses
  `home.tagline`.
- `mobile/ios/Info.plist` + `Assets.xcassets/LaunchBackground.colorset`:
  `UILaunchScreen.UIColorName`, `#030C0B`.
- `client/src/verdant3d/surroundings.rs`: `painting_uv` maps the image's
  vertical axis to world -X and its horizontal axis to +Z. The previous
  uniform 4× mirrored tiling drew the oblique painting sideways and flipped it
  between the two halves of the map. Mirroring in depth would turn every other
  tile upside down, so the depth tile is 2 per plane, centred 28 m beyond the
  map centre, covering -146 m…+202 m against a reachable view of about
  -133 m…+188 m. Width tiles follow the camera pitch (ratio 0.759).
- `mobile/ios/PhoneLayoutPreview.swift`: the `UIDevice.orientationDidChange`
  observer is gone; `validate()` separates ineligibility and host geometry
  changes (end) from a drifted frame (re-apply, at most 5 times per 2 s and 20
  times per preview);
  `restore` takes and logs a reason.
- `client/src/qa/boot_qa.rs`: `OMOBA_BOOT_SPLASH_SHOTS` capture harness.
- `scripts/ui_screen_map.json`, `docs/ui-screens.md`: `test_ui_screen_map` was
  already failing on `main` (eight unmapped frames of the offline harness); the
  frames are now listed as focused-run frames, together with the new harness.

## Checks

- Unit tests: `frontend::boot`, `verdant3d::surroundings`, `ui::modal`,
  `phone_layout_preview`, `i18n`; `make check`.
- Desktop renderer, English, 1180×820: splash and revealed Home
  (`OMOBA_BOOT_SPLASH_SHOTS`); surroundings at the home corner and the away
  corner at maximum zoom, before and after (`OMOBA_OFFLINE_SMOKE_DIR` with
  `OMOBA_OFFLINE_REMATCH_QA_ONLY=1`).
- iPad (A16) simulator, iOS 26.3, standalone UIKit harness around the
  unmodified and the changed `PhoneLayoutPreview.swift`: the old bridge ends
  the preview on a posted device-orientation notification, the new one holds,
  restores a drifted frame, and still ends on toggle-off and resign-active.
  The same harness shows the launch screen white before and `#020C0B` after.

## Remaining risks

- `mobile/ios/build_device.py` and `build_simulator.py` write the plist but do
  not compile `Assets.xcassets`, so `make iphone` bundles still launch on the
  system background. The Xcode project and `prepare_testflight.py` compile it.
- Tooltips, toasts and the focus ring use higher global layers than the splash;
  none is expected during startup.

- No physical iPad run. The orientation notification is the one automatic
  restore the harness could trigger and a hand-held tablet produces
  constantly; if the preview still ends on the device, the new
  `Omoba phone preview ended: <reason>` log line names the cause.
- The harness is plain UIKit, not the game binary; no iOS game build was made.
  `ios-check.yml` should run on the branch before a device build.
- The splash covers the engine phase only. Time before the first engine frame
  shows the plain launch colour, without title or progress.
- Startup progress tracks fonts and Verdant scenes. Avatar previews, audio and
  first-use shader compilation are not measured; the minimum time covers them
  on desktop only.
- The painting's trees are about twice as large as before; a denser upright
  tiling needs artwork that tiles vertically.
