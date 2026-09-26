# Releasing OMOBA

One script, `scripts/release.py`, builds every package; `make release-*`
targets wrap it. Packages land in `dist/v<version>/` with `SHA256SUMS.txt`.
Nothing is published without an explicit step: GitHub releases are created as
**drafts**, and only `release-testflight` uploads to App Store Connect.

## Versioning

- The single source of truth is `[workspace.package].version` in `Cargo.toml`
  (SemVer). Tags are `v<version>` (for example `v0.24.0`).
- `0.x` releases are playtests: GitHub marks them **pre-release**. `1.0.0` is
  reserved for the public launch.
- Each release has a `## [x.y.z] - date` section in `CHANGELOG.md`; the release
  notes are taken from it (`make release-notes`).
- Android `versionCode` is derived from the version (`0.24.0` → `240009`);
  iOS build numbers come from `mobile/ios/Omoba.local.xcconfig` and are bumped
  after every TestFlight upload.

## Where each platform is built

| Platform | Artifact | Built by | Command |
| --- | --- | --- | --- |
| macOS (Apple silicon) | `Omoba-<v>-macos-arm64.zip` (`Omoba.app` + host script) | this Mac or CI | `make release-mac` |
| Windows x64 | `Omoba-<v>-windows-x64.zip` (`Omoba.exe` + host script) | GitHub Actions (or a Windows PC) | `make release-ci` |
| Linux x64 | `omoba-<v>-linux-x64.tar.gz` (client, server, systemd example) | GitHub Actions, or Docker locally | `make release-ci` / `make release-linux` |
| Android arm64 | `Omoba-<v>-android-arm64.apk` | GitHub Actions, or a machine with SDK/NDK | `make release-ci` / `make release-android` |
| iPhone | TestFlight build (`.ipa` locally) | this Mac (Apple team) | `make release-testflight` |

`make release-check` prints what the current machine can build.

## Turnkey release

1. Merge the release PR to `main` with CI green (`make check` locally first).
2. Bump `Cargo.toml` version, move `CHANGELOG.md` `[Unreleased]` entries into
   `## [x.y.z] - date`, update `docs/features.md`, commit to `main`.
3. Optional: choose the server clients start on. Either pass
   `RELEASE_SERVER=host:port` to the make targets, set the repository
   variable `OMOBA_RELEASE_SERVER` for CI, or leave it empty (players type the
   address in the party lobby → SERVER → Change).
4. **Desktop + Android:** push the tag (or run the workflow by hand):

   ```sh
   git tag v0.24.0 && git push origin v0.24.0     # triggers .github/workflows/release.yml
   # or: make release-ci RELEASE_SERVER=host:port
   ```

   The workflow builds macOS, Windows, Linux and Android in parallel and
   attaches them, plus `SHA256SUMS.txt`, to a **draft** release.
5. **iPhone:** `make release-testflight` (Xcode must be signed in to the team
   in `mobile/ios/Omoba.local.xcconfig`). Then in App Store Connect → TestFlight,
   add the processed build to your tester group.
6. Optional local extras: `make release-mac` / `make release-ios` and
   `make release-draft` upload local packages into the same draft (`--clobber`).
7. Review the draft on GitHub, download one package per platform, smoke it,
   then publish: `gh release edit v0.24.0 --draft=false`.

## Signing and trust

- **macOS:** ad-hoc signed, not notarized. Players right-click → Open once
  (or `xattr -dr com.apple.quarantine Omoba.app`). Notarization needs a
  Developer ID Application certificate and `notarytool` credentials.
- **Windows:** unsigned; SmartScreen shows "More info → Run anyway".
- **Android:** signed with one stable playtest key so updates install over each
  other. Locally it lives in `~/.config/omoba/android-playtest.keystore`; for CI
  store it as the secret `OMOBA_ANDROID_KEYSTORE_BASE64`
  (`base64 -i ~/.config/omoba/android-playtest.keystore | gh secret set OMOBA_ANDROID_KEYSTORE_BASE64`).
  Without the secret CI generates a fresh key per run (uninstall before updating).
  Store publication needs a real release key.
- **iPhone:** App Store distribution signing, managed by Xcode.

## Hosting for a playtest

Every desktop package contains a practice host (`Host Practice Server.command`,
`Host Practice Server.bat`, `host-practice.sh`). Players must reach the host's
**UDP 4000**: same LAN, a VPN (Tailscale/ZeroTier) or a router port forward. For
an always-on server, run the Linux package on a VPS with
`omoba-practice.service` and `ufw allow 4000/udp`. The public lobby with
PostgreSQL is documented in [public-mvp.md](public-mvp.md).
