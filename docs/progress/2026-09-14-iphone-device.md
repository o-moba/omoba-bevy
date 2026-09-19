# Physical iPhone build — 2026-09-14

Base: `e62b728`; candidate version `0.19.0-rc.9`. Dedicated worktree and branch.
The user asked to run the game on their own iPhone. The existing Simulator-only
scaffold was insufficient for that request.

An actual `aarch64-apple-ios` client compiled successfully with Rust 1.93.1 and
Xcode 26.2, deployment target iOS 15.0. The local release profile uses opt-level1,
no debug data, no incremental compilation, no LTO and stripped symbols. The first
build completed in 2m10s. No game-source compatibility fix or production dependency
was needed; client/shared/passport behavior remains the existing main behavior.

The official matching Rust std component was hash-verified and installed into the
existing project-private toolchain. The first offline attempt using its old Cargo
cache lacked the newer SQLx index entry; the completed build reused the existing
current default Cargo cache with locked offline resolution.

The physical .app includes tracked assets and legal notices. An existing valid
development certificate and wildcard provisioning profile already include the
user's paired iPhone. Signature, physical Mach-O platform and entitlements were
verified without creating credentials or exporting private keys. Personal profiles,
device IDs and signed artifacts remain outside versioned source.

The candidate asset gate initially exposed a stale inventory. The five existing
map props match the original art library and their approved source commit; the
environment manifest matches the earlier foliage readability revision. Only the
policy and strict validator were updated; no model bytes changed. All 28 candidate
asset tests pass, including missing, altered, unlisted and provenance-tampered
props. The final package is freshly signed after this inventory refresh.

Thirteen focused Python packaging tests pass. Installation was attempted with
Apple devicectl, which could not locate the unavailable paired device. The signed
artifact is ready for a connected, unlocked, trusted iPhone; installation, actual
rendering, audio, touch and sustained performance are not yet accepted.

See `.agent/tasks/IPHONE-DEVICE-2026-09-14/` for real build/sign/install results and
limits, and [device instructions](../../mobile/ios/README.md) for repeatable commands.
No TestFlight, App Store, public download or server deployment was performed.
