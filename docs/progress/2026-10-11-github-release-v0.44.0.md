# v0.44.0 — Community Beta 1, the first GitHub release

## Goal

Publish the first tagged GitHub release with the exact packages already distributed
through omoba.io, including the Linux audio fix.

## Release

- Tag `v0.44.0` (annotated) on `main`, GitHub pre-release "OMOBA 0.44.0 — Community Beta 1"
  per the [release policy](../release-policy.md).
- Assets: Windows x64 zip, macOS Apple-silicon zip, Linux x64 tarball, universal Android
  APK, `SHA256SUMS.txt`, `SOURCES.txt` (per-package source commit) and `compatibility.json`
  (release 0.44.0, protocol 11, `standard-kits-7`, `verdant-confluence-compact-v2`,
  `wildspark-2026-10-09-reference`).
- Package provenance: Windows `1952af2`; macOS and Android `e72201f` (runtime equal to
  `1952af2`); Linux `096836e` (`1952af2` plus the Linux-only cpal ALSA fix, #82).
  Each package's SHA-256 equals the copy downloaded back from its IPFS mirror.
- The Android asset is the published universal APK (versionCode 44, playtest key now kept
  at `~/.config/omoba/android-playtest.keystore`).
- The tag push must not run `release.yml`: its draft job would rebuild and `--clobber` the
  verified assets (and CI has no Android playtest key). The tagged commit carries `[skip ci]`.

## Checks

- PR #82 CI: fmt/clippy/tests, headless gameplay, Android compile, Postgres, Python tooling.
- Linux release build of the fix in the offline Docker image without warnings.
- Server unchanged: `/opt/omoba/releases/0.44.0-beta-1952af2` serves the same contract.

## Remaining risks

- The Linux audio fix has not yet been confirmed on a physical PipeWire desktop.
- Physical-device coverage remains thin (Windows, Android); packages are not code-signed.
- iPhone build 0.44.0 (22) still needs its App Store Connect upload from Xcode Organizer.
