# Mobile HUD interaction follow-up — 2026-10-29

The owner reviewed the compact HUD and requested a floating left joystick,
kill notices below purchases, and a direction cue on the selected enemy portrait.
The follow-up stays on PR141 (`fix/compact-mobile-hud` into dev).

`mobile_controls.rs` now reanchors only for a fresh exclusive movement owner in
the lower-left region. Drag leaves the base fixed; release centers the thumb and
stops movement while retaining the base. Clamping is neutral on press. Saved
combat offsets resolve independently so starting movement cannot shift buttons
under another finger. The base resets on resize/settings changes and fits short
movement strips. Frontend/gamepad/focus/death gates keep their existing behavior.

`hud_layout.rs` and `tactical_hud.rs` place compact kill notices directly below
quick-buy on the left. The pure stack geometry also bounds fresh movement input;
short screens use one row while ordinary phones retain two. Desktop is unchanged.

`enemy_portraits.rs` adds a small gold bearing inside the selected portrait's
existing touch cell. It uses the current camera and entity transforms, including
2D mapping, and requires the exact live visible enemy identity. No vector is
shown for hidden/dead/missing/stale targets or position-free public death timers.
Arrow paint does not intercept taps and stationary geometry does not dirty UI
layout each frame.

Evidence and current check receipts are kept in
`.agent/tasks/MOBILE-HUD-INTERACTION-20261029/`. Native verification uses only the
existing English 844×384 phone profile, with two affected presentation states
showing a released stick at different bases and opposite known enemy bearings.
This is scripted native-desktop rendering, not manual Android or multiplayer
proof. The local Android candidate uses the existing Beta signer and increased
build 450010; its exact source/hash/build result is recorded in `android/artifact.json`.

The known adjacent omoba-ui CJK-font handoff drift is unchanged; isolated Git
verification retains the normal CI installed-asset/token-lock checks. No shared
assets/tokens, dependencies, server rules/protocol, infrastructure or CI changed.
Physical installation remains separate because the phone is disconnected.
