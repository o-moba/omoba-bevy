# Releasing OMOBA

Follow the owner-approved [release policy](release-policy.md) and [tester guide](beta-testing.md). This page documents the commands; the policy defines release acceptance.

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

## Compatibility gate

The release builder writes `compatibility.json` beside the packages. Before distribution,
probe the target lobby **and** staged worker with that artifact's manifest. Different
release labels can be compatible; matching labels alone are insufficient. See the
[compatibility contract and server-first rollout procedure](release-compatibility.md).

```sh
make compatibility-check GAME_SERVER_ADDR=HOST:PORT COMPATIBILITY_MANIFEST=dist/v0.41.0/compatibility.json
```

A nonzero result blocks distribution until diagnosed; do not bypass an unverified
server. Direct Xcode/server builds must retain a manifest from the same frozen source.

## Turnkey release

1. Prepare the version, changelog and release notes in a short-lived branch. Merge the release PR into `main` after the applicable checks pass; freeze that commit.
2. Build/stage the matching server and verify the contract, signed gameplay and reconnect. Deploy under authorization with rollback retained and no active human matches interrupted. If building from the tag first, keep its release a draft until this gate passes.
3. Leave the initial endpoint unset to use the built-in **OMOBA Beta** server, or pass `RELEASE_SERVER=host:port` / repository variable `OMOBA_RELEASE_SERVER` for an intentional override. Existing client preferences are preserved.
4. Create an annotated tag on the frozen commit and push it:

   ```sh
   git tag -a v0.43.0 -m "OMOBA 0.43.0 community beta"
   git push origin v0.43.0
   ```

   The existing workflow builds desktop and Android packages and creates a **draft prerelease**, including `SHA256SUMS.txt`, `compatibility.json` and `SOURCE_COMMIT.txt`. It refuses to overwrite a published release. Never force-update a published tag.
5. For iOS, build the frozen source with a new build number; run `make release-testflight` only under upload authorization. After Apple processing, assign the build to the intended tester group. External testers may require beta review.
6. Review checksums, source identity, compatibility against the live endpoint, installation instructions and known limitations. Publish the approved draft with `gh release edit v0.43.0 --draft=false`. Once public, corrections use a new version/tag.
7. Record deployment and distribution evidence. Delete merged work branches, retain release tags and symbols, and start the next fix/feature on a new short-lived branch.

## Signing and trust

- **macOS:** ad-hoc signed, not notarized. Players right-click → Open once
  (or `xattr -dr com.apple.quarantine Omoba.app`). Notarization needs a
  Developer ID Application certificate and `notarytool` credentials.
- **Windows:** unsigned; SmartScreen shows "More info → Run anyway".
- **Android:** signed with one stable playtest key so updates install over each
  other. Locally it lives in `~/.config/omoba/android-playtest.keystore`; for CI
  store it as the secret `OMOBA_ANDROID_KEYSTORE_BASE64`
  (`base64 -i ~/.config/omoba/android-playtest.keystore | gh secret set OMOBA_ANDROID_KEYSTORE_BASE64`).
  Tagged/manual releases fail without that secret. PR-only build checks may use an ephemeral key and must not be distributed as updates.
  Store publication needs a real release key.
- **iPhone:** App Store distribution signing, managed by Xcode.

## Hosting for a playtest

Every desktop package contains a practice host (`Host Practice Server.command`,
`Host Practice Server.bat`, `host-practice.sh`). Players must reach the host's
**UDP 4000**: same LAN, a VPN (Tailscale/ZeroTier) or a router port forward. For
an always-on server, run the Linux package on a VPS with
`omoba-practice.service` and `ufw allow 4000/udp`. The public lobby with
PostgreSQL is documented in [public-mvp.md](public-mvp.md).
