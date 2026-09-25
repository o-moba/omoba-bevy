# 2026-09-25 — Android build: `make android` / `make android-check`

## Goal
User asked for a way to produce an Android build for a LAN playtest with
friends, and to have that path be reproducible by anyone who clones the repo
(discoverable through `make`, like the existing iPhone targets), without
changing what the underlying packager does.

## Changes
- `Makefile`: added `android-check` (wraps `mobile/android/build.py --check`)
  and `android` (wraps `mobile/android/build.py --output
  target/mobile/android`, forwarding `ANDROID_SERVER=host:port` as `--server`
  when set). Added both to `.PHONY` and to the `make help` overrides/footer.
- `README.md`: "Commands at a glance" table gained the two Android rows,
  including the exact output path
  (`target/mobile/android/omoba-<version>-android-arm64-debug.apk`); the
  iPhone/Android paragraph below it now explains the Android prerequisites
  and links `mobile/README.md#android-local-test-package`.
- `mobile/README.md`: Android section now leads with `make android-check` /
  `make android` (via `ANDROID_HOME`/`ANDROID_NDK_HOME`), keeping the direct
  `python3 mobile/android/build.py --sdk/--ndk` form for custom paths, and
  notes `ANDROID_SERVER`/`--server` for baking in a real address for testers.
- `make android` now also echoes the resulting APK's path after a successful
  build, so the target file is not just implied by the docs.

`mobile/android/build.py` itself is unchanged — it already read
`ANDROID_HOME`/`ANDROID_SDK_ROOT`/`ANDROID_NDK_HOME` as defaults, so the make
targets are a thin, no-behaviour-change wrapper.

## Checks
- `make help` prints the new targets and overrides correctly (verified).
- Installed the full toolchain on this machine (`brew install --cask
  android-commandlinetools android-ndk`, `rustup target add
  aarch64-linux-android`, `sdkmanager --licenses`, `platforms;android-35` +
  `build-tools;35.0.0`) and ran `make android-check` (`ready_to_build: true`,
  no problems) then `make android ANDROID_SERVER=192.168.1.77:4000`.
- Real build succeeded end-to-end: `cargo rustc --target aarch64-linux-android
  --crate-type cdylib` finished in 4m22s, `aapt2 link` + `zipalign` + a fresh
  local debug keystore + `apksigner sign`/`verify` all passed
  (`Verifies`, v2/v3 signature schemes true). Artifact:
  `target/mobile/android/omoba-0.23.0-rc.6-android-arm64-debug.apk` (~299 MB,
  dev profile with debug symbols — expected size for this profile, not a
  release build).
- Not verified: install/launch on a physical Android device, or an actual
  phone-to-phone/phone-to-PC match. The desktop client and full workspace
  test suite were separately verified working after the refactor earlier in
  this session (298+94 tests passing, live `make practice` session with no
  panics).

## Remaining risks / follow-ups
- Real-device install/launch and a live LAN match are still unverified.
- `target/mobile/android/` and `builds/` are git-ignored by design (per
  `mobile/README.md`); the APK itself is a local artifact, not part of this
  commit.
