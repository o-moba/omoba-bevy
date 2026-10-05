# Community beta: getting started

Download a numbered **Pre-release** from [GitHub Releases](https://github.com/o-moba/omoba-bevy/releases). Use the version named by the test coordinator; source `main` may already contain the next iteration. For the first community beta use **0.43.0**. Installation availability is stated on its release page, including the separate TestFlight status.

## Install and connect

- **Android:** download the arm64 APK (modern 64-bit Android devices), allow installation from your browser/file manager, then install. This is not a Play Store release or a universal 32-bit APK. Updates use the same signing key; do not uninstall first unless instructed, since local identity/preferences can be lost.
- **macOS:** use the Apple Silicon zip, unzip and open `Omoba.app`. The current beta is ad-hoc signed, not notarized. macOS may require its explicit Open/security approval. Intel macOS is not included in this package.
- **Windows:** unzip the entire x64 archive and run `Omoba.exe`. The beta is not publisher-signed; Windows may show a reputation warning.
- **Linux:** unpack the x64 tarball and run `./omoba`. A working graphics driver and system audio/runtime libraries are required.
- **iPhone/iPad:** use the matching processed build in TestFlight after the coordinator adds you or provides an approved tester link. A GitHub zip is not an iOS installer. External testing may require Apple's beta review.

Fresh installs select **OMOBA Beta** (`77.246.105.57:4000`). If an old custom address was saved, use Home's server button → OMOBA Beta → Connect. Wait for verified compatibility/online status. Version 0.43.0 expects protocol 10 / standard-kits-5 / compact geometry v2; older 0.41 clients must be upgraded for this beta. A mismatch means update/check the selected server, not reinstall repeatedly.

Online matches, including server bot practice, require Internet access. The host currently admits at most **two simultaneous rooms**; a solo bot match uses one. **Offline Practice** runs on the device without Internet, progression or server match history. Use it while rooms are full or to isolate device/input problems.

## First test session

1. Confirm the version and select Beta. Join a bot match, choose a hero, move, attack, aim a skill and purchase an affordable item near base.
2. Try dash/haste and the seven-second recall; movement or combat should cancel recall. Look at the minimap, towers and team/enemy death timers.
3. Briefly lose connectivity or background the app, then return and attempt to resume the same match. Report any abandoned-match screen or inability to return Home. Mobile OS suspension is part of this device test, not covered by desktop automation.
4. Finish a match and inspect the results, then start another. Try an Offline Practice session too.
5. Report avatar clipping, touch targeting, chat/keyboard, sound, measured FPS, overheating or battery drain with device details and a short recording where possible.

## Limits and feedback

This is an early community beta: balance, performance and mobile polish remain under test. Physical iPhone/iPad acceptance is a separate gate from automated builds. 120 FPS is a requested cap, not a guaranteed delivered rate. Payment/Supporter purchasing is not a beta acceptance target; progress is test data and long-term preservation is not promised.

Submit a [bug report](https://github.com/o-moba/omoba-bevy/issues/new/choose) with the exact version/build, OS/device, mode, endpoint, steps, expected/actual result and approximate time/timezone for a server incident. Never attach private keys, recovery codes, signing files or unredacted personal data. Feature ideas use the separate suggestion template.
