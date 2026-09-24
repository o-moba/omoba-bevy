# Permanent Xcode project — 2026-09-22

Version: 0.22.0-rc.2. Task proof: `.agent/tasks/TASK-XCODE-TESTFLIGHT-2026-09-22/`.

The earlier TestFlight workflow used prepared archives, not an uncommitted Xcode
source project. Local archives for 0.21.0 builds 10 and 11 remain untouched.
A tracked `mobile/ios/Omoba.xcodeproj` now provides the shared Omoba application
scheme, native Archive action, signing settings, icon and privacy resources.

The script phase invokes locked Cargo on the current checkout. It stages the
physical-device executable, tracked assets/legal notices and matching nonempty
Rust dSYM. Xcode owns Info.plist processing, icon compilation, signing and archive
metadata. Incremental staging removes obsolete engine assets without replacing
Xcode-owned resources. Source changes during compilation fail the build.

The marketing version follows Cargo (numeric Apple version 0.22.0); build defaults
to 12, with ignored local overrides for team, next unused build, server and cache.
No credential, team ID, distribution profile or default public endpoint is included.
The existing device/simulator/archive scripts remain available.

A real unsigned Xcode 26.2 device archive completed; application metadata, arm64
platform, icon/privacy resources and matching dSYM UUID were inspected. Python
regressions and the final independent verification are recorded in the task proof.
The user's existing Makefile change and practice server were preserved.

Unsigned archive success is not Apple validation, distribution signing, upload,
TestFlight availability or physical-device gameplay verification. Those remain
separate account/device actions. No credentials or provisioning were changed.
