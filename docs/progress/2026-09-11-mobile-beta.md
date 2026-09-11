# Native phone beta implementation — 11 September 2026

Version: `0.18.0-rc.7`. Delivery status: implementation candidate; release gates remain open.

## Chosen product path

Keep one native game and one gameplay protocol across phones and computers. The
website explains access and distributes verified builds. A separate desktop
launcher and browser gameplay are not required for this beta. Android is the
first phone package; physical iOS distribution follows a verified build and
provisioning step. Match servers retain the existing authoritative combat,
progression, economy and victory rules. Device parity and human enjoyment need
mixed-device playtests before making competitive balance claims.

## Implemented surface

- Left fixed joystick with an analog dead zone and camera-relative movement.
- Large Q attack button at lower right; W/E/R in an arc, with drag-assisted
  directional target selection, release/cancel and separate rank-up buttons.
- Independent finger ownership; opening a modal, losing focus, dying, rotating
  or changing rounds clears gameplay intent. Portrait shows a rotate prompt.
- Compact landscape HUD, top-left minimap, reachable help/menu/shop, reflowed
  character selection, shop and results. Conservative gutters reserve edge
  space; actual cutout/inset behavior remains a device verification gate.
- Pre-join host-address entry with a numeric pad and optional native keyboard,
  validation and the existing preference/reconnect machinery. Host and phone
  must use the same reachable UDP endpoint; phone loopback is not the computer.
- Android NativeActivity manifest/package script, shared native entry, private
  settings storage, packaged model metadata and conservative Android rendering.
  iOS has a Simulator build scaffold, not a signed physical-device package.
- Omoba landing beta access, platform status and onboarding in the isolated
  `landing-v2-mobile-beta` clone. Download actions require real configured HTTPS
  release URLs. Current unavailable platforms have no pretend download action.

## Try the control layout on desktop

```sh
OMOBA_TOUCH_CONTROLS=1 GAME_SERVER_ADDR=192.168.1.20:4000 cargo run -p client
```

Use a real address reachable from the test client. The desktop flag exposes the
phone layout for QA; mouse input does not emulate two physical touch contacts.
See [mobile/README.md](../../mobile/README.md) for the native package commands.

The actual renderer capture script accepts `--touch-controls --width 844 --height
390` or `--width 932 --height 430` with `--scenario beta-ui`. Its result stage uses
an explicitly labelled fixture. It is UI evidence, not a complete human match.

## Delivery boundaries

The exact landing patch is now applied in `/Users/wotori/git/wotori/landing-v2`;
all 23 file hashes match the tested clone and no other app changed. The game
stays in the isolated clone: original feature-ref/worktree Git directories are
root-owned, and a fresh worktree attempt still fails with Permission denied.
No ownership changes were made. Raw results and the transferable game patch
are under `.agent/tasks/MOBILE-BETA-2026-09-11/`.

Explicitly authorized disk cleanup restored build capacity. The full Rust workspace run passed 361 tests;
a later client run after renderer fixes passed 213 tests. Native build, formatting,
Clippy and 54 Python tests pass. Actual Bevy captures pass seven stages each at
844×390, 932×430 and desktop 1280×720. Entry selection, help, gameplay, shop,
receipt-confirmed purchase, shop dismissal and the labelled result fixture were
inspected. The initial renderer attempt was blocked by GPU availability; after
permissions changed, actual rendering exposed and prompted fixes for a duplicate
shop label component, low-contrast selected tiles and desktop-only phone hints.
QA requests OS window focus once and keeps the production focus-loss gate intact.

Rust's Android target is now installed privately. Official Android SDK 35r2,
Build Tools 35.0.0 and NDK r27c archives were downloaded and their published
checksums verified. Android installation/use awaits the user's explicit SDK
license acceptance; no license markers or acceptance were fabricated. No APK,
IPA, actual-phone match, signed mobile download or public deployment is claimed.
Host-address form, pause/settings and portrait lack pixel evidence. Real phone
cutouts, performance, controls and lifecycle require hardware testing. Browser
visual access to the landing was explicitly declined; its production build,
TypeScript, URL policy and HTML checks passed.

The [task checklist](../../tasks/TASK-MOBILE-BETA-2026-09-11.md) and local
`evidence.md` distinguish completed source work, executed checks and open gates.
Public server admission/authentication, bounded receive work, movement authority
against packet-rate amplification, hidden information and command delivery
semantics remain explicit release work. Real phones must also prove frame time,
thermals, lifecycle, network changes, touch usability and complete mixed matches.
