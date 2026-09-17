# Controller beta

Source version: 0.20.0-rc.7 (built on Bevy 0.19.1). This feature is not in the
previously uploaded TestFlight build 5. A new signed build is required to test it
on an iPhone.

## Controls

| Action | PlayStation | Generic / Xbox-style |
| --- | --- | --- |
| Move / menu navigation | Left stick | Left stick |
| Directional aim | Right stick | Right stick |
| Repeat basic attack | Hold R2 | Hold RT |
| Skills 1 / 2 / 3 | Hold L1 / R1 / L2, release to cast | Hold LB / RB / LT, release to cast |
| Ultimate | Hold L2+R2, release either | Hold LT+RT, release either |
| Lock / unlock target | R3 | Right stick click |
| Cancel aim / back | Circle | B |
| Upgrade skill | Hold Triangle, press/release the skill binding | Hold Y, press/release the skill binding |
| Shop / reactions | D-pad right / left | D-pad right / left |
| Menu | Options | Menu / Start |
| Menu selection | Cross | A |

The basic attack has an 80 ms initial chord grace period. Once an ultimate chord
is recognized, both triggers must be released before another trigger action.
Releasing a skill while blocked by a menu, death, focus loss or disconnect never
casts it. After these transitions, center the sticks and release buttons to
rearm. On initial controller takeover, release controls once before playing.

Movement is camera-relative with a radial dead zone and analog speed. Right-stick
selection uses a bounded visual handle and directional assistance. R3 keeps an
explicit valid target; the controller does not automatically chase out-of-range
enemies. Skill range, mana, cooldown, visibility and team checks still use the
existing shared/client/server rules. Current abilities remain unit-targeted or
self-targeted; this does not introduce free ground-targeted skillshots.

Menu buttons support directional navigation and confirmation, including panel
scrolling. Text entry still uses keyboard/touch. A fresh touch or mouse/keyboard
input restores that input method and clears controller gestures. UI profile still
follows platform settings; input device selection is independent of screen size.

## Implementation

- `client/src/gamepad_controls.rs`: normalized device snapshots, ownership,
  gestures and safety gates; `gamepad_controls_tests.rs`: ECS regressions.
- `client/src/gamepad_ui.rs`: menu focus and controller legends.
- `client/src/gamepad_ios.rs`, `mobile/ios/OmobaGameController.swift`: copied
  main-thread snapshots, connection/lifecycle generations, no retained FFI pointers.
- `client/build.rs`: compiles native bridges for all iOS build paths.
- `mobile/ios/Info.plist`: declares optional ExtendedGamepad support. Touch
  interaction remains available; a controller is not required to install/play.
- Existing `player`, `targeting` and `combat` paths consume intents. A connected
  idle controller does not overwrite a desktop click route.

No new third-party production packages. Rumble, adaptive triggers, custom button
remapping and an on-controller text keyboard are not implemented. Generic labels
are used when the desktop device name does not identify a PlayStation controller.

## Hardware acceptance (pending)

Pair DualSense or DualShock 4 in iPhone Bluetooth settings, then launch a build
containing rc.7. Verify initial pairing, reconnect, app background/resume, stick
drift, simultaneous movement and all four skills, explicit lock, upgrades, menu
navigation, and touch takeover. Check Bluetooth latency and controller battery
loss during a match. Repeat a full bot match on desktop and iPhone; tune dead
zones and target assistance from actual play rather than simulated inputs.

Automated native-framework snapshots validate the bridge but are not a physical
Bluetooth or gameplay test. The client suite passes 406 tests, and iOS packaging
passes 37 tests. Native desktop/mobile UI captures use clearly labeled synthetic
controller input; they verify rendering, not a Bluetooth connection. See the task
evidence for commands, artifacts and the independent review.
