# iPhone gameplay UX — 0.35.0

## Scope and behavior

The twelve gameplay requests are implemented in the isolated `fix/iphone-preview-connect` checkout. The four skills retain their sizes, order, vertical positions and spacing. Touch attack hints are hidden; aim projection runs after current camera movement. The phone HUD uses top allied portrait buttons, centered target health, compact menu/chat, bottom kill notices, and small minion/tower controls continuing the skill circle around attack. The separate recall button shows a seven-second countdown.

All six controls use one 104-point orbit with a uniform 42-degree step: four skills at 162/204/246/288 degrees, then tower at 330 and minion at 372. Fitting the rightmost 44-point hit circle requires the attack anchor to move from 76 to 124 points inside the safe right edge. The combat group therefore shifts left by 48 points at normal scale; its vertical anchor remains unchanged. The earlier separate compact quarter arc was rejected by the user and is superseded. Category artwork remains 36 points, with nonoverlapping 44-point hit circles. Drawing and touch ownership share the same calculated layout.

Below 700 points of usable width, Dash and Haste use a compact arrangement that keeps their touch targets and rank artwork outside the protected hero area. This does not change the shared skill/category circle or the standard 852×393 layout. The final full client suite passes 861 tests, with one existing ignored migration report.

Recall lives in the shared authoritative simulation, so online play and Offline Practice use the same cancellation and completion rules. Starting recall stops a queued route; movement, combat, damage, death and disconnection cancel it. The client flushes its current position before the request. Reconnection cannot complete an old channel. Recall retains health, resources and cooldowns; it does not grant healing.

Dash uses press-drag-release direction selection, a cancellation area, a world landing marker and same-frame camera recovery from manual pan. Active attack/dash pointers have a gold line and outlined thumb above the control faces, so dragging across a skill cannot hide the endpoint. Ally portraits select a follow target by network identity and team. Camera motion or local movement restores predictable control; touch panning has a 2.25 multiplier. A concealed local avatar keeps its existing material fade and gains a crossed-eye marker.

Graphics settings persist a 60/120 FPS ceiling; iOS receives display-link callbacks while other platforms use Winit deadlines. The on-screen number measures app frame cadence. Actual frequency depends on device support, system power/thermal policy and rendering cost. HUD settings move joystick and combat groups in bounded increments and independently reset their positions; they do not rearrange individual skills.

Tapping the local hero opens the reaction wheel. Server-confirmed reactions display a built-in placeholder until image loading completes. Existing trusted companion-pack entitlements remain supported; general account-owned SDK sticker packs are designed in [the asset contract](../sticker-assets.md) and remain a separate implementation.

## Verification and release boundary

The additional MegaAngel Home issue came from vertically centering the fitted silhouette instead of retaining the platform ground line. The preview now grounds the normalized model and fits its actual bind-pose vertices for every turntable angle, retaining the shared 87% image anchor used by Home. This avoids both the visible gap and excessive shrinking from a full bounding cylinder around wings. Results are cached until the model, normalized transform or projection changes. Normalization itself is unchanged; extreme animation poses and separate equipment remain outside this static envelope.

The selected visual scope is English at 852×393 logical points, using the native desktop renderer as an iPhone layout proxy. Synthetic fixtures are explicitly labeled and record geometry/readiness. They do not certify physical iPhone input, sustained 120 Hz, or multiplayer by themselves. Real UDP social broadcast and authoritative recall have separate tests.

The first fixture placed invisible roster actors near the hero and consequently showed their ordinary HP plates in the centre of the screen. Those captures are superseded. Roster-only fixture actors now stay off camera; a capture fails if more than the one real local hero's overhead plate is visible. Production overhead vitals remain intact.

Protocol 5 introduces recall commands and requires matching server and client releases. The hosted Beta still runs protocol 4 until a separately approved coordinated deployment. No production server or TestFlight release is changed by this task. The existing Android 0.34.2 universal APK predates these gameplay changes.

Detailed command results, screenshots and acceptance evidence are kept in `.agent/tasks/IPHONE-GAMEPLAY-UX-20261003/`. Final verification results are recorded there after the native capture pass.

## Release — 2026-10-03

Implementation merged and pushed to `main` as `ab24903`. Signed iOS **0.35.0 (17)** archived successfully and uploaded through Xcode Organizer; the UI confirmed “App upload complete”. The archive declares exempt encryption and includes matching arm64 dSYM UUID `8735C66E-7A84-361F-A211-29D9E85E1D22`. CLI export still reports “Failed to Use Accounts”; Organizer reused the existing signed-in account successfully.

Beta now runs `/opt/omoba/releases/0.35.0-beta-ab24903` (protocol 5). A real authenticated UDP client passed lobby allocation and entered a running match on port 41000, receiving all eight structures. Server SHA256: `90b45c0d9c3005baa3ffd545d92b5f4eb4d7a1506ece83a0966d3d789f283a34`. Previous `/opt/omoba/releases/0.34.1-beta-f1d32f5` remains for rollback. No database migration, credentials or service configuration changed. Protocol-4 clients require the new client update.

Apple processing and tester availability were not verified: browser access to App Store Connect was denied by its permission policy. Upload success must not be interpreted as a confirmed install or physical-device smoke test.

Local archive: `builds/mobile-0.35.0-17/Omoba.xcarchive`. Upload screenshot, logs, server identity and remote admission report: `.agent/tasks/TESTFLIGHT-0350-17/`. Previous UX evidence moved to the primary checkout at `.agent/tasks/IPHONE-GAMEPLAY-UX-20261003/`. The merged worktree and temporary Rust Docker image were removed; reusable platform build caches retained. The preceding primary-checkout 0.34.2 edits remain backed up in a named Git stash and are incorporated in this release.
