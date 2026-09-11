# Mobile beta delivery — 11 September 2026

Status: source implementation and native UI verification delivered; Android package and real-device acceptance remain open. Source baseline: `0.18.0-rc.6`; candidate version: `0.18.0-rc.7`. Target: an installable phone client playing the same matches as desktop, with the existing Omoba website guiding players into the beta. Checked implementation items below do not certify a released mobile beta.

## Implementation

- [x] Two-thumb control implementation: left joystick, right attack/ability cluster, independent finger tracking, target assistance and cancel/release handling.
- [x] Landscape phone layout implementation: safe gutters, compact resources/objectives, top-left minimap and reflowed entry/help/shop/results. Seven actual UI stages passed at both 844×390 and 932×430; desktop 1280×720 also passed.
- [x] Mobile lifecycle implementation: rotation notice, focus-loss cleanup and existing reconnect path.
- [x] Native platform entry, assets/preferences and build scripts; Android compile and early iOS preflight attempted.
- [ ] Produce an installable Android APK. Rust Android target is installed privately and official SDK/build-tools/NDK archives are verified. Installation/use awaits the user’s Android SDK license acceptance; no APK exists yet.
- [x] Omoba landing: beta access, accurate platform status, installation/play instructions and responsive source layout. Production build, type and policy checks pass.
- [x] Inspect the current game renderer at 844×390, 932×430 and desktop 1280×720: 21 captured stages pass the layout/readback checks.
- [ ] Inspect landing in the browser; access was explicitly declined.
- [ ] Add pixel evidence for host-address form, pause/settings, portrait and actual device cutouts.
- [x] Versioned changelog, feature inventory, progress record and independent source review. See task evidence for individual verdicts and remaining verification gaps.

## Next executable steps

1. Obtain the pending Android SDK license acceptance, install the already verified tools and build the debug APK.
2. Install the APK on an actual phone and connect it to the same controlled LAN server as a computer.
3. Run the mixed-device and lifecycle checklist below; fix control/layout/performance findings before signing a distributable beta.
4. Verify the browser landing with permitted access, enable only verified release downloads, and transfer the reviewed game patch into its canonical repository. The landing patch is already applied to its original source.
5. Complete public-server hardening and obtain approval for concrete deployment/signing changes before public access.

## Public beta release gates

- [ ] Install and play a full phone-versus-desktop match on actual target hardware.
- [ ] Measure sustained frame time, memory, thermal behavior and startup on baseline phones.
- [ ] Exercise packet loss/jitter, app background/foreground and Wi-Fi/mobile-network changes.
- [ ] Protect server admission and reconnect; bound peers and receive work; eliminate per-packet movement allowance amplification.
- [ ] Define server-owned hidden information and reliable command semantics for competitive play.
- [ ] Build and verify the Linux server and supported desktop distributions.
- [ ] Select the public endpoint, prepare supervision/logging/backups and approve deployment changes.
- [ ] Produce approved signed distribution artifacts, verify downloads and then enable landing download links.
- [ ] Observe mixed human matches for control usability, match comprehension, fairness and desire to replay.

## Workspace and evidence

Game implementation: `/Users/wotori/git/ekza/omoba-bevy-mobile-beta`, branch `feature/mobile-beta-2026-09-11`.

Landing implementation: `/Users/wotori/git/ekza/landing-v2-mobile-beta`, branch `feature/omoba-mobile-beta-2026-09-11`; original source is `/Users/wotori/git/wotori/landing-v2`.

The landing’s exact 23-file patch is now applied in its original source and all file hashes match the tested clone. The game remains isolated because its canonical feature-ref/worktree metadata directories are owned by root; a renewed worktree attempt still fails with Permission denied. No ownership changes were made, and original game work in progress is preserved. Workflow evidence: `.agent/tasks/MOBILE-BETA-2026-09-11/`.
