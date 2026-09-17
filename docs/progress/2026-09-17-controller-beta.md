# Controller beta implementation

Implemented on feature/gamepad-controls from the Bevy 0.19.1 candidate, without
merging main or changing the previously uploaded TestFlight build. Source version
advances to 0.20.0-rc.7.

The input adapter emits shared movement/cast/attack intents, preserving server
validation and avoiding desktop auto-chase. Added radial dead zones, directional
target retention, explicit lock, skill release/chord handling, controller menu
focus and adaptive labels. iOS snapshots come from Apple Game Controller on the
main thread through the existing Swift/Cargo bridge build path.

Verified 406 client regressions, 37 iOS packaging tests, the arm64 iOS Rust target,
native Apple framework snapshots, Swift compilation for device/simulator and a
physical-arm64 Rust-to-Swift link. Native UI captures cover 844x390 mobile and
1280x720 desktop layouts with synthetic controller input. Screenshot review caught
and fixed overflowing skill captions and menu-hint overlap; independent review
caught and fixed missing optional controller support metadata in Info.plist.

Verification artifacts and independent review: `.agent/tasks/GAMEPAD-2026-09-17/`
in the canonical repository. Hardware controller/iPhone playtesting and a new
TestFlight upload remain separate release steps.
