# Controller support

Source version 0.25.0 (Bevy 0.18). Desktop (macOS, Windows, Linux) and iPhone.
**Hardware acceptance is unverified:** no physical controller or iPhone was
used to test this release; see [Hardware acceptance](#hardware-acceptance-pending).

## Controls

| Action | PlayStation | Generic / Xbox-style |
| --- | --- | --- |
| Move | Left stick | Left stick |
| Aim | Right stick | Right stick |
| Basic attack (repeat while held) | Hold R2 | Hold RT |
| Skills 1 / 2 / 3 | Hold L1 / R1 / L2 to aim, release to cast | Hold LB / RB / LT, release |
| Ultimate | Hold L2+R2 (either order), release either | Hold LT+RT, release either |
| Upgrade a skill | Hold Triangle, press and release the skill binding | Hold Y, then the skill binding |
| Lock / unlock target | R3 | Right stick click |
| Cancel (in play) / back (menus) | Circle | B |
| Pause menu | Options | Menu / Start |
| Shop / reactions (in play) | D-pad right / left | D-pad right / left |
| Menu navigation | D-pad or left stick | D-pad or left stick |
| Select (menus) | Cross | A |

- **Movement** is camera-relative with analog speed: a half tilt walks at half
  speed. A radial dead zone of 0.18 removes drift without bending diagonals;
  NaN or out-of-range readings count as centred. While a controller owns input
  the camera stays locked on the hero, as on a phone.
- **Skills** go through the same pending-cast path as a key or a tap. Aiming
  uses the touch aim assist (45° cone, in range, visible hostile); with an R3
  lock the skill goes to the locked target. A controller never walks to cast:
  an out-of-range target is reported instead.
- **Basic attack** waits 80 ms so the second trigger of the ultimate chord can
  arrive on the next frame, then attacks the aimed (or locked) target on the
  shared attack cooldown. It never chases: a target out of reach is not
  attacked. The aim keeps its previous candidate while it scores within 12 px
  of the best one, which absorbs snapshot jitter.
- **Ultimate chord**: once L2+R2 is recognised, both triggers must be up before
  any trigger acts again, so releasing the chord never casts skill 3 or starts
  an attack.
- **Lock**: R3 locks the current candidate and R3 again unlocks. A lock that
  becomes invalid (dead, hidden, friendly, a different unit, despawned) cancels
  the held attack or skill instead of switching to another unit.
- **Upgrades** use the hotbar's rule (a skill point, the slot unlocked and below
  the maximum rank); an upgrade never casts.

## Who owns input

A connected but idle controller never takes over: the mouse keeps its route
and the phone keeps its touch HUD. A new button press or a deliberate stick
push hands input to the controller (and drops what the mouse or touch had
queued). Any keyboard key, mouse button or touch hands it back. A stick left
tilted when input was handed back does not steal it again until it returns to
centre.

While the controller owns input, the phone's touch HUD hides (the skill bar
shows the controller's bindings instead) and returns on the next touch; the
hotbar shows `L1 R1 L2 L2+R2` (PlayStation) or `LB RB LT LT+RT`; a legend strip
explains the layout (one line of menu controls on menus).

## Safety cancels

Opening any modal, losing window or app focus, disconnecting, switching or
replacing the controller, dying, a round change and Circle/B during play all
drop the controller's held skill, queued cast, basic attack order, aim preview,
selection and movement. Cooldowns are kept. After a cancel the controller
waits for released buttons and centred sticks before it acts again, so a
button still held when a menu closes never fires.

## Menus

Menus use the UI kit's generic focus layer (`client/src/ui/focus.rs`, see
[the UI kit](ui-kit.md#focus)): the D-pad or left stick moves a gold ring
between the buttons of the front-most modal (or the whole screen when no modal
is open), Cross/A presses the focused button through the same gate as a click,
and a list scrolls to show the focused button. A new page or modal starts on
its first button and needs a fresh press. Circle/B is a back press
(`ui::back::BackInput`, the same signal as Esc): it closes the front-most
overlay, and closes the pause menu, but never opens it. Options/Start opens
the pause menu where the `≡` button can and closes it when it is on top.
Text entry (chat, server address, nickname) still needs a keyboard or touch.
Menu selection is silent: the click sound follows pointer and key presses.

## Platforms

| Platform | Source | Status |
| --- | --- | --- |
| macOS, Windows, Linux | Bevy `Gamepad` (gilrs, in Bevy's default features) | implemented, unit and ECS tested |
| iPhone | Apple GameController through `mobile/ios/OmobaGameController.swift` (gilrs has no iOS backend) | implemented, bridge harness tested on macOS; `cargo check --target aarch64-apple-ios` passes |
| Android | none | out of scope: the Android build uses Bevy without gilrs, so controllers are not read |

On desktop the controller in use stays selected while connected; otherwise the
lowest-numbered connected one is used. PlayStation names are shown for Sony's
USB vendor id or a DualShock/DualSense device name, generic names otherwise.

`mobile/ios/Info.plist` declares `GCSupportsControllerUserInteraction` and
`GCSupportedGameControllers = [ExtendedGamepad]` without making a controller
required (no `GCRequiresControllerUserInteraction`, no `game-controller`
device capability). The Swift bridge is compiled by `client/build.rs` with the
StoreKit and browser bridges and links `GameController.framework`.

## Implementation

- `client/src/gamepad/snapshot.rs` – `PadSnapshot`, the 16-bit button layout
  (South 0 … Select 15, `CHORD = L2 | R2`), `radial_dead_zone`, the desktop
  and iOS sources.
- `gamepad/ios.rs` – FFI to `omoba_gamecontroller_poll`: caller-owned stack
  storage, no retained pointers, clamped axes, masked buttons, identity 0
  rejected; main thread only (the sampler holds `NonSendMarker`).
- `gamepad/ownership.rs` – takeover and handoff; `gamepad/gesture.rs` – the
  pure gesture state machine; both have no ECS.
- `gamepad/gameplay.rs` – `resolve_gamepad` (in `GamepadInputSet`, after the
  input context, before pointer combat) applies the safety cancels and writes
  the intents; `pad_combat` (in the combat chain, after the touch attack,
  before the attack resolver) uses `mobile_assisted_target`,
  `queue_cast_request`/`PendingCast`, `BasicAttackState::start(.., false)` and
  `NetworkCommand::UpgradeSkill`. No new network command, protocol or server
  change.
- `player::input::move_player_analog` – the analog step shared by the thumb
  stick and the controller (`analog_source`).
- `gamepad/legend.rs` – skill labels, the aim caption and the legend strip.
- `mobile/ios/test_game_controller.swift` – native harness: axes, all 16
  buttons, the trigger chord, stable selection, lifecycle and disconnect
  identities, replacement, and the C ABI guards (short buffer, null
  pointers, off-main-thread calls).

Rumble, adaptive triggers, button remapping and an on-controller keyboard are
not implemented. No third-party production crate was added.

## Hardware acceptance (pending)

Automated tests drive Bevy `Gamepad` components and Apple's writable
GameController snapshots; they are not a Bluetooth or gameplay test. Before
calling this shipped, pair a DualSense or DualShock 4 and an Xbox controller on
desktop, and a DualSense on an iPhone with a new signed build, and check:
pairing and reconnect, app background and resume, stick drift, movement with
all four skills, the ultimate chord in both orders, lock and unlock,
upgrades, menu navigation in the pause menu, shop, career and hero select,
touch or mouse takeover, and a full bot match. Tune the dead zone and aim
assist from real play.
