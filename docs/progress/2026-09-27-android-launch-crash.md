# 2026-09-27 — Android launch crash and missing icon

## Goal
The Android debug APK installed without an icon and crashed immediately on launch
(also on a tester's Xiaomi). Find the cause and fix it on `main`.

## Findings
- The tester's crash report: `java.lang.UnsatisfiedLinkError ... dlopen failed:
  cannot locate symbol "__cxa_pure_virtual" referenced by .../lib/arm64/libclient.so`.
  The earlier "wrong ABI" theory (the `--universal` follow-up) did not apply: the APK
  installed and the arm64 library was loaded.
- `llvm-readelf --dyn-syms` on the built `libclient.so` listed 27 undefined C++ runtime
  symbols (`__cxa_*`, `__gxx_personality_v0`, `_Znwm`/`_ZdlPv`, `std::*_error` RTTI).
  NEEDED contained no C++ library.
- Source: `oboe-sys` 0.6.1 (Android audio via `cpal`) compiles C++ and emits
  `rustc-link-lib=c++_static`. With NDK r30 the ABI runtime lives in a separate
  `libc++abi.a`, which nothing linked. A shared library may link with undefined
  symbols, so the build succeeded and the failure surfaced only at `dlopen`.
- The manifest had no `android:icon`, and the packager linked no resources.

## Changes
- `mobile/android/build.py`: append `-C link-arg=-Wl,--no-undefined` and
  `-C link-arg=-lc++abi` to the Android rustflags; compile `mobile/android/res`
  with `aapt2 compile` and link it into the APK.
- `mobile/android/AndroidManifest.xml`: `android:icon="@mipmap/ic_launcher"`.
- `mobile/android/res/mipmap-{mdpi,hdpi,xhdpi,xxhdpi,xxxhdpi}/ic_launcher.png`:
  48–192 px, resized from `mobile/ios/Assets.xcassets/AppIcon.appiconset/AppIcon.png`.

## Checks
- Full `build.py` run with NDK r30 / build-tools 35: link succeeded with
  `--no-undefined`; `libclient.so` has no undefined C++ symbols, NEEDED is only
  `liblog`, `libOpenSLES`, `libdl`, `libandroid`, `libm`, `libc`; the library in the
  signed APK is byte-identical to the inspected one.
- `aapt2 dump badging`: application icon present for all five densities.
- `python3 -B scripts/test_package_licenses.py`: OK.

## Remaining risks
- Launch on a physical device not yet confirmed after the fix (no device or emulator
  was attached to this machine).
