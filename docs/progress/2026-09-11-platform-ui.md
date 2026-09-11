# Platform-specific interface policy — 11 September 2026

Refinement of the same unreleased `0.18.0-rc.7` mobile beta candidate.

## Behavior

The pure policy in `client/src/platform/ui_profile.rs` resolves the interface
from `std::env::consts::OS`, which Rust supplies for the compiled target. Android
and iOS use Mobile. Windows, macOS and Linux use Desktop. There is no viewport,
DPI, orientation or touch-device heuristic. `MobileControls` resolves the policy
once at resource initialization; window changes only affect geometry and the
landscape/focus gates. Shared gameplay, protocol and assets remain shared.

`OMOBA_TOUCH_CONTROLS=1` is an explicit mobile preview on a desktop development
build (`debug_assertions`). A standard desktop release ignores it. Native
mobile targets cannot be switched to Desktop with this environment variable.
Phone forms and IME systems are installed only for the selected Mobile profile,
so Desktop retains keyboard/IME ownership and its own HUD.

The capture tool validates the actual profile for every beta UI stage, including
missing/malformed evidence, so requesting mobile preview cannot silently accept
desktop screenshots. The physical screen does not choose the profile.

## Verification

Current checks and independent review are recorded under
`.agent/tasks/PLATFORM-UI-2026-09-11/`. Tests exercise the target/preview matrix,
resize/orientation/DPI invariance, and absence of phone-only desktop side effects.
The exact policy module is also compiled separately with debug assertions on
and off and executed with preview 0/1 to verify actual development/release behavior
without rebuilding an entire release renderer. This is a selector probe, not a
full release client or Android/iOS package test.

Native renderer captures compare Desktop and Mobile preview at the same 1280x720
viewport, plus a phone-sized Mobile capture. Existing package, physical-device
and public-server release gates remain in the [mobile beta record](2026-09-11-mobile-beta.md).

Final verification: 216 client tests and 58 Python tests pass, along with native
build, formatting and workspace Clippy with warnings denied. Three native
renderer runs passed all 21 stages with the requested interface profile; all
image hashes were validated. The same 1280x720 window produced the correct
distinct Desktop and Mobile-preview HUDs, and 844x390 retained Mobile. Opposite
profile requests against each real readback were rejected by the capture verifier.
