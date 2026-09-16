# TestFlight from the primary checkout

TestFlight installs the app over the internet. It does not require a USB cable,
wireless Xcode pairing, a registered device UDID, or Developer Mode on the tester's
phone. The distributor needs an active Apple Developer Program membership and
access to App Store Connect. The tester installs Apple's TestFlight app.

## Prepare locally

First create a signed physical-device app using [build_device.py](README.md).
The archive preparer reuses that executable and its bundled assets, and validates
the source signature, platform, matching dSYM and existing development profile. It compiles the
app icon with Xcode, adds distribution metadata and a required-reason API manifest,
sets an explicit build number, and re-signs a **copy** using the existing identity.
It never modifies the source app, provisions an account, creates certificates or
uploads anything. Xcode later replaces this development signature during App Store
distribution export. No adjacent worktree is needed.

Run from the repository root, using a fresh output for every attempt:

```sh
python3 -B mobile/ios/prepare_testflight.py \
  --app builds/iphone-signed-1/OmobaBeta.app \
  --dsym builds/iphone-signed-1/OmobaBeta.app.dSYM \
  --identity 'YOUR_EXISTING_DEVELOPMENT_CERTIFICATE_SHA1' \
  --build-number 2 \
  --output builds/testflight-2/OmobaBeta.xcarchive --check

# Repeat without --check to prepare the archive.
```

`--check` is read-only. Choose a build number not previously uploaded for this app
version. Supported numbers are 1..9999, with optional minor/patch components of
0..99. The marketing version comes from the input app, not the current checkout;
rebuild the game to update runtime code or its version. Selected Xcode SDK and the
input executable SDK must match.

The prepared `.xcarchive` includes `preparation.json` with hashes and explicit
status flags. `archive_prepared` and `development_signature_verified` can be true
while `distribution_signed`, `apple_validated`, `uploaded`, `testflight_available`
and `installed` remain false. Do not package this intermediate archive as a
TestFlight IPA. Preserve the source build report/logs to bind the executable to
its actual source revision. The archive includes `dSYMs/OmobaBeta.app.dSYM` and
records verified UUIDs plus symbol hashes. `--dsym` may be omitted when the
matching `OmobaBeta.app.dSYM` is next to the input app. Both preflight and archive
creation reject missing symbols, UUID mismatches, or symbol files with empty
debug-info/line sections. UUIDs are checked again against the re-signed archive
executable. These checks avoid the earlier Organizer missing-dSYM warning;
Apple's own validation still runs during distribution.

For older stripped builds, rebuild the source with the current device builder.
It retains `debug=1`, `strip=none`, and `split-debuginfo=packed` in both profiles.
Do not run `dsymutil` on a stripped old executable and assume an empty dSYM with
a matching UUID restores lost debug information. Keep the new dSYM alongside
the archived build for crash investigation.

The original, code-drawn beta icon is in `Assets.xcassets/AppIcon.appiconset`.
It uses the Omoba landing page palette and can be replaced with an approved
1024x1024 opaque PNG. `render_beta_icon.swift` reproduces it with AppKit using a
fresh output path; there is no third-party artwork or additional package dependency.

## Complete distribution in Apple tools

1. Sign in to App Store Connect and verify the paid developer membership/team.
   Resolve any agreements directly with the account holder.
2. Select or create an app record for the explicit bundle identifier
   `space.ekza.omoba.beta`. The existing wildcard **development** profile is not
   an App Store distribution profile. Do not change the bundle identifier merely
   to work around a provisioning error.
3. Open the archive in Xcode Organizer. Use **Distribute App → App Store Connect**
   (or the current **TestFlight Internal Only** option for a private internal test).
   Use the correct team and an Apple Distribution certificate/App Store profile,
   or Xcode's supported cloud signing. Creating credentials or changing signing
   access is an explicit account-owner step; never revoke certificates as an
   automatic repair. Run Apple's validation before upload.
4. Complete the encryption/export-compliance questions for the actual application.
   This script intentionally does not guess `ITSAppUsesNonExemptEncryption`.
   Review any Apple validation/privacy findings before releasing the build.
5. After upload processing succeeds, open **TestFlight → Internal Testing**, create
   or select a group, include the app owner's App Store Connect user, and assign
   the build. Install it through TestFlight on the iPhone. Internal testing does
   not require the external beta-review flow. A public/external test needs the
   applicable TestFlight review and tester information.

Do not put profiles, account keys, certificate files, team/device identifiers or
archives into Git. Passwords and verification codes belong only in Apple's UI.
Prepared archives retain private development-profile details until distribution
export; they are local artifacts, not public downloads. TestFlight builds expire
after 90 days; upload newer builds to continue testing.

## Privacy manifest scope

`PrivacyInfo.xcprivacy` declares required-reason APIs used by the compiled Rust
runtime and app: file metadata in app-owned storage (`C617.1`) and monotonic
interval/timer measurements (`35F9.1`). The September 16 binary imports `stat`,
`lstat`, `fstat` and `mach_absolute_time`. Source examples include profile-key file
validation in `client/src/career_identity.rs` and session/network timers in
`client/src/session_config.rs` and `client/src/net.rs`.

This is not a claim that the game collects no data. Profiles, nicknames, chat,
match statistics and account operations require a separate App Store privacy
disclosure review against the deployed services. No empty collected-data list or
blanket tracking/encryption declaration is inserted. Re-audit the manifest when
the app or SDKs change, and investigate Apple validation feedback rather than
adding unrelated API reasons to silence warnings.

## Playing after installation

TestFlight distributes the **client**, not a game server. For the existing home
kit, start `builds/iphone-home-2026-09-16/1-Start-server.command` on the Mac, put
Mac and iPhone on a network that allows them to communicate, and enter the printed
LAN `IP:4000` using **SERVER → CONNECT**. Xcode Wi-Fi pairing is not required for
this game connection. If LAN communication also fails, a reachable hosted game
server is required. No public 24/7 server deployment is claimed by this workflow.

## Verification

```sh
python3 -B -m unittest discover -s mobile/ios -p 'test_*.py' -v
```

Tests check rejected simulator executables, unsafe bundle paths, nested code,
preservation of existing output, build-number limits, and read-only preflight.
They also cover retained compiler debug settings, mismatched/missing/empty
symbols, verified dSYM copies, and unchanged input bundles.
Apple upload acceptance and actual touch/rendering/audio/network behavior still
need their respective live checks.

References:
- [TestFlight](https://developer.apple.com/testflight/)
- [Distribution preparation](https://developer.apple.com/documentation/xcode/preparing-your-app-for-distribution)
- [Upload builds](https://developer.apple.com/help/app-store-connect/manage-builds/upload-builds)
- [Internal testers](https://developer.apple.com/help/app-store-connect/test-a-beta-version/add-internal-testers)
- [Required API reasons](https://developer.apple.com/documentation/bundleresources/app-privacy-configuration/nsprivacyaccessedapitypes/nsprivacyaccessedapitypereasons)
