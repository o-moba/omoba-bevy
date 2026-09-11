# Omoba mobile beta build and platform boundaries

Android is the first phone package. Android, iOS and desktop retain the same native
UDP protocol and gameplay rules. These files prepare native application launch;
they do not publish a server, a download, or a store release.

## Desktop and mobile interface selection

The application chooses its UI once from Rust's compiled target OS:

| Compiled target | Interface |
| --- | --- |
| Android / iOS | Mobile: joystick, right-thumb abilities, phone HUD and touch menus |
| Windows / macOS / Linux | Desktop: keyboard/mouse controls and desktop HUD |

Window size, pixel resolution, DPI and touch hardware never select the UI family.
A small desktop window remains desktop; an Android tablet remains mobile.
Rotation only reflows the mobile layout or shows its landscape prompt. Phone
forms and keyboard/IME handlers are not installed in the desktop interface.

For development on a computer, `OMOBA_TOUCH_CONTROLS=1 cargo run -p client`
explicitly previews the mobile interface. This override requires a development
build with debug assertions; a normal release desktop build ignores it. Android
and iOS always keep their mobile UI. The capture script's `--touch-controls`
option uses this development preview and verifies that the rendered profile
matches the request. Details: [platform UI record](../docs/progress/2026-09-11-platform-ui.md).

## Current measured status — 2026-09-11

- Desktop and mobile share `client/src/lib.rs`; the desktop executable is a thin
  wrapper. Android uses Bevy 0.18's `#[bevy_main]` NativeActivity entry.
- Android's resolved dependency graph selects `native-activity` and does **not**
  enable the mutually exclusive `game-activity` feature. Desktop Bevy/SDK features
  remain unchanged. The Android SDK identity crate runs without its unused legacy
  HTTP model-loader feature; the shipped roster still loads through Bevy.
- Android starts with MSAA and directional shadows disabled, following Bevy 0.18's
  conservative mobile driver defaults. Desktop rendering remains unchanged.
- Phone windows use fullscreen landscape, VSync, a 60 Hz reactive foreground loop
  and a lower-power background loop. The manifests allow both landscape directions.
- Android assets are read through Bevy's APK AssetManager. Model-scale overrides
  and roster metadata are included with the client; model overrides have an Android
  embedded fallback because APK paths are not normal filesystem paths. iOS places
  assets beside the executable inside its `.app` bundle.
- Preferences use Android's application-private `internal_data_path`, iOS
  `Library/Application Support`, or the existing desktop directory. A developer's
  `OMOBA_CLIENT_CONFIG_DIR` override still takes precedence. The installed asset
  directory is never used for preference writes.
- The first actual `cargo check --locked --offline -p client --lib --target
  aarch64-linux-android` attempt stopped with E0463 because Android Rust `std` / `core`
  was absent. After the user freed disk space, Rust Android std 1.93.1 was downloaded
  from the official Rust distribution, verified against its published SHA256, and
  installed into the task-private `target/mobile/tooling/rust-1.93.1` toolchain.
  The global Rust installation remains unchanged.
- Android Platform 35 revision 2 and Build Tools 35.0.0 archives are staged and
  verified against Google's repository checksums. NDK r27c/27.2.12479018 is also
  staged and verified; a bounded byte-range download recovered from a stalled
  full-file transfer. Android packages reference the Android SDK License Agreement. No
  prior acceptance was found, so installation/use waits for the user's explicit
  acceptance. No license markers were created. The license text and official
  package metadata are in `target/mobile/tooling/downloads/`.
- The initial disk constraint has been resolved by the user's authorized cleanup;
  the latest platform preflight uses the private Rust toolchain and reports the
  remaining Android SDK/build-tools/NDK installation prerequisites.
- Xcode 26.2 provides the iPhone and Simulator SDKs. Rust's
  `aarch64-apple-ios-sim` target is missing, and this sandbox cannot connect to the
  CoreSimulator service. No iOS runtime test is claimed.
- **No APK, IPA, phone launch, real-device performance result, or public mobile
  beta has been produced by these checks.** Raw platform reports are generated in
  `target/mobile/verification/`; the parent beta task records the consolidated evidence.

## Android local test package

Prerequisites: Python 3.9+, JDK 17+, Rust with `aarch64-linux-android`, Android SDK
`platforms;android-35`, `build-tools;35.0.0`, and NDK r27 or newer. macOS SDK command
binaries may require Rosetta on Apple Silicon. Keep a dedicated writable Cargo home
and output directory if the normal toolchain directories are read-only. Reserve at
least 6 GiB for build output in addition to installed tools; a cold build may need more.
The script reports prerequisites and does not install or upgrade any toolchain.

```sh
python3 mobile/android/build.py --check --sdk /path/to/android-sdk --ndk /path/to/android-ndk
python3 mobile/android/build.py --sdk /path/to/android-sdk --ndk /path/to/android-ndk
```

`build.py` runs Cargo's Android cdylib target with the NDK compiler, includes the
existing `client/assets`, aligns the APK and signs a **local debug APK** using a
new development key in `target/mobile/android/local-debug.keystore`. It does not
use a production signing key. Keep this local key if subsequent sideloaded debug
builds must update the same installation; generating a different key requires
uninstalling the previous test app, which also removes its preferences.

The output name is `target/mobile/android/omoba-<version>-android-arm64-debug.apk`.
`--unsigned` deliberately skips signing and produces an APK that cannot be installed.
The package requires arm64, Android 8/API 26 or newer, touchscreen and Vulkan level 1;
these are build prerequisites, not a claim that every such device performs well.

After an actual successful build, use the installed SDK's adb to install the debug
APK on a consenting test device, then launch **Omoba Beta** from its icon. The game
lets a tester enter the same real LAN/server address used by the computer client.
`127.0.0.1` on a phone refers to the phone itself. Do not use it for a server running
on your computer. The existing server's UDP port must be reachable from both devices.

No live endpoint is compiled by default. For a controlled test build,
`--server` accepts a real `host:port` and sets `OMOBA_DEFAULT_GAME_SERVER_ADDR` at
compile time. Environment `GAME_SERVER_ADDR` and saved preferences retain their
usual precedence; the in-game connection form can choose another endpoint.
Malformed build-time configuration falls back to the local default without a panic.
No custom URL scheme or website-to-app deep link is registered in this beta package.

Android's current winit implementation exposes logical key characters while its
`KeyboardInput.text` is empty. NativeActivity soft keyboards vary, so the game also
provides an on-screen address pad. Test both before relying on OS keyboard entry.

## Early iOS Simulator package

```sh
python3 mobile/ios/build_simulator.py --check
python3 mobile/ios/build_simulator.py
```

This creates `target/mobile/ios-simulator/OmobaBeta.app` for an arm64 Apple Silicon
iOS Simulator and applies only ad-hoc local signing. The Info.plist contains
landscape orientations and the local-network permission explanation. Assets are
copied inside the bundle. The script prints `simctl install` and launch commands
but does not launch or modify simulator devices automatically.

A physical iPhone build, Apple developer provisioning, device signing, and TestFlight
require a separate verified step with the user's credentials and explicit release
authorization. This simulator scaffold does not imply those steps are complete.

## Required beta device evidence

Before publishing a phone download, verify: cold launch and all shipped assets;
server address entry and preference survival across force-stop/relaunch; a full
phone/computer match; two simultaneous thumbs including skill use during movement;
background/resume and interruption cancellation; both landscape orientations and
screen cutouts; disconnect/retry and Wi-Fi changes; sustained frame rate, memory,
battery and temperature on representative midrange and recent phones. Confirm the
APK's alignment/signature and Android 16 KiB page-size compatibility on real hardware.
Internet authentication, admission, authoritative movement, loss/jitter behavior,
server operations and public distribution remain the shared beta release gates.

The Android Cargo command is `cargo rustc --locked -p client --lib --target
aarch64-linux-android --crate-type cdylib`. Its `libclient.so` is placed at
`lib/arm64-v8a/libclient.so`; `android.app.lib_name=client` in NativeActivity selects
that exact library. `aapt2 -A client/assets` puts Bevy asset paths under APK `assets/`.
The packager aligns before signing and verifies both signature and final alignment,
following the official [zipalign](https://developer.android.com/tools/zipalign),
[apksigner](https://developer.android.com/tools/apksigner), and
[AAPT2](https://developer.android.com/tools/aapt2) command references.

Reference implementation: [Bevy 0.18 mobile example](https://github.com/bevyengine/bevy/tree/v0.18.0/examples/mobile)
and [Bevy 0.18 NativeActivity example](https://github.com/bevyengine/bevy/tree/v0.18.0/examples/mobile/android_basic).
