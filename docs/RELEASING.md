# Releasing OMOBA

Prepare a reviewed release PR, freeze its source, and verify the matching server/client contract before deployment and distribution. See the [0.44.0 delivery record](progress/2026-10-10-release-044.md) for the verified local Windows/Linux rollout.

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
| Windows x64 | `Omoba-<v>-windows-x64.zip` (`Omoba.exe` + host script) | this Mac (cargo-xwin), GitHub Actions or a Windows PC | `make release-all` / `make release-ci` |
| Linux x64 | `omoba-<v>-linux-x64.tar.gz` (client, server, systemd example) | GitHub Actions, or Docker locally | `make release-ci` / `make release-linux` |
| Android | `Omoba-<v>-android-universal.apk` (arm64, armv7, x86_64) via `make release-all`; `Omoba-<v>-android-arm64.apk` otherwise | this Mac with SDK/NDK, or GitHub Actions | `make release-all` / `make release-android` / `make release-ci` |
| iPhone | TestFlight build (`.ipa` locally) | this Mac (Apple team) | `make release-testflight` |

`make release-check` prints what the current machine can build and the free disk space.

## One command on the Mac

`make release-all` builds all five platforms on an Apple-silicon Mac from the
current checkout, pointed at `PROD_SERVER` (default: the live test server), into
`dist/v<version>/` with `SHA256SUMS.txt` and `compatibility.json`. It stops when
less than ~20 GB is free (`OMOBA_MIN_FREE_GIB`). `make release-publish` then pins
each package to IPFS with `PINATA_JWT`, downloads it back through the gateway,
compares the SHA-256 and records the links in `dist/v<version>/ipfs.json`;
packages already pinned with the same hash are skipped.

One-time setup on the Mac:

```sh
rustup target add x86_64-pc-windows-msvc aarch64-apple-ios \
  aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
cargo install --locked cargo-xwin && brew install llvm android-ndk
sdkmanager "platforms;android-35" "build-tools;35.0.0"
```

- **Windows** is cross-built with cargo-xwin (Microsoft CRT/SDK cached in
  `target/xwin`, static CRT). The packager checks both executables are x64 PE.
- **Linux** runs in Docker (`rust:<toolchain>-bookworm`). When Docker cannot reach
  its registry (for example, a dead system proxy), the build fetches crates and the
  checksum-verified ALSA/udev `.deb` files on the host and compiles with
  `--network none`, using a Rust toolchain installed in
  `target/linux-docker/toolchain-<toolchain>`.
- **Android** is one universal APK signed with
  `~/.config/omoba/android-playtest.keystore`; keep that file, or updates stop
  installing over earlier builds.
- **iPhone:** the build number is the last one in the ignored
  `mobile/ios/Omoba.local.xcconfig` + 1. If command-line export is refused (Xcode
  needs an Apple ID under Settings → Accounts), the signed archive is kept in
  `dist/v<version>/` and opened in Xcode Organizer for Distribute App → App Store
  Connect → Upload; that number counts as used.

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

## Updates and contract changes (LR-14)

Policy (launch default O-12): client and server ship together. Balance values stay
compiled into both, so a contract change is a **forced update**. An incompatible
game shows **Update required** with a download button, and a newer compatible
release shows a dismissible **Update available** banner on Home. The game reads the
market's release catalog (`GET /v1/releases` of its Account API, the `releases.json`
installed as `OMOBA_RELEASE_CATALOG`) at boot and after a mismatch. The button opens a
page on the portal compiled into the build (`scripts/markets.py`): the catalog entry's
`update_url`, by default `<portal>/download?platform=<platform>`. The iPhone build has no
link: it updates through the App Store or TestFlight. A build without a portal (the
global market today) shows no button and tells players to download the new version from
the official site. The lobby's compatibility report carries its release as the minimum
and the portal's download page (`UpdateNotice`, [release-compatibility.md](release-compatibility.md)).

**Catalog fields** (`scripts/release.py catalog`, [account-api/README.md](../account-api/README.md#downloads)).
Every entry carries `channel` (`direct`). The game reads only the entries of its own
channel; store channels arrive with LR-68. On a market with a portal, each entry also
carries `update_url`, the page its update button opens. `catalog --min-release X` adds
`min_release`: games of that channel and platform below X show **Update required** even
while the servers still accept them. That is a forced update too, so it needs the same
player notice (step 3 below). `min_release` may not be newer than the release itself.

**What bumps a revision** (`shared/src/compatibility.rs` and the fields in
[release-compatibility.md](release-compatibility.md)):

- `GAMEPLAY_REVISION`: anything the client simulates, predicts or shows from its own
  compiled data that must equal the server's. That covers shared simulation rules
  (`common/src/sim/`, `common/src/command.rs`), movement, dash and collision, damage,
  cooldown, cost and range numbers, hero and item stats and growth (`common::balance`,
  the shared catalogs), targeting and vision rules, and offline-practice behaviour
  shared with the server.
- `PROTOCOL_VERSION`, `CATALOG_REVISION`, `GEOMETRY_ID`: a wire change, a skill or kit
  contract change, or a map geometry change. Each has its own rule in
  [release-compatibility.md](release-compatibility.md).
- No bump for server-only behaviour the client does not predict: bot decisions,
  matchmaking, match lifecycle, settlement, moderation, logging. The same goes for
  UI, text, audio, textures, animation and VFX-only changes.
- When unsure, bump. A needless forced update costs one download. A missing bump lets a
  mismatched client mispredict silently.

**Order of a contract-changing release:**

1. Build and stage the new server. Verify the new client manifest against the staged
   lobby and worker (`make compatibility-check`, [compatibility gate](#compatibility-gate)).
2. **Ship the clients first.** Publish the packages to the download host and install the
   new `releases.json`. Old clients now show **Update available**. A player who updates
   early sees "the servers are switching to your version" and connects by itself once
   they do (Home retries every 30 s).
3. **Notify players before the forced update.** Post the time of the server switch in the
   market's community channels, and leave at least the banner period between steps 2
   and 4. Without a notice, a server switch must not change the contract.
4. Switch the lobby and the workers. Old clients now get **Update required** and the
   download button.

**Alternative (O-12, not chosen for launch):** move balance values into server-sent
data so balance-only changes stop being contract changes. That needs a protocol for the
values, client-side caching and tests that prediction uses them. It is a later decision
for the owner.

## Turnkey release

1. Merge the release PR to `main` with CI green (`make check` locally first, and
   `make sdk-pin-check SDK_BRANCH=main` so the pinned SDK commit is on SDK `main`).
2. Bump `Cargo.toml` version, move `CHANGELOG.md` `[Unreleased]` entries into
   `## [x.y.z] - date`, update `docs/features.md`, commit to `main`.
3. Optional: choose the lobby clients use. Either pass
   `RELEASE_SERVER=host:port` to the make targets, set the repository
   variable `OMOBA_RELEASE_SERVER` for CI, or leave it empty (the public lobby).
   Production clients have no in-game server selection
   ([connection profiles](connection-profiles.md)).
4. **Desktop + Android:** push the tag (or run the workflow by hand):

   ```sh
   git tag -a v0.24.0 -m "OMOBA 0.24.0" && git push origin v0.24.0     # triggers .github/workflows/release.yml
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

## DEV-contour tester builds

Dev builds are the separate OMOBA Dev app for testers on the dev contour (dev lobby on
the dedicated dev VPS, `registry.dev.ekza.io`) on every platform. They never replace or
share state with a production install, and on phones they install beside it
(`space.ekza.omoba.dev`, "OMOBA Dev", DEV-ribbon icon). In-game, Home → Connection
switches between the Dev, Local and Custom profiles
([connection profiles, owner checklist](connection-profiles.md)):

```sh
python3 scripts/release.py build all --contour dev --dry-run   # what gets baked in
python3 scripts/release.py build all --contour dev             # every platform this host can build, into dist/v<version>-dev/
python3 scripts/release.py build ios --contour dev --upload    # OMOBA Dev to App Store Connect (internal TestFlight)
python3 scripts/release.py ci --contour dev --ref v<version>-dev.1 --dry-run
```

- Defaults: `lobby.dev.omoba.io:4100` (`--server <DEV_IP>:4100` when DNS is not set), `https://registry.dev.ekza.io`, passport `http://127.0.0.1:9/api/passport` (closed: there is no dev passport yet, and `avatar.ekza.io` is production; `--passport` takes a dev or loopback URL), settings folder `omoba-bevy-dev`, window title `OMOBA DEV`, `Omoba Dev.app` (bundle id `…desktop.dev`). The dev Linux package also carries `omoba-account-api`, the dev lobby's migration tool.
- `-dev` appears in every package and dist directory name. Dev releases use `v<version>-dev.N` tags only, are drafts and prereleases, and target `dev`. The workflow's `contour=dev` input also builds the dev APK.
- A dev build refuses production endpoints, and a production build refuses dev ones (and `$DEV_IP`). A dev client also refuses the production lobby at run time: it has no Production profile, and custom, `GAME_SERVER_ADDR` or allocated production addresses are rejected.
- No build inherits a compile-time setting from the shell (`OMOBA_DEFAULT_*`, `OMOBA_ACCOUNT_API_URL`, `OMOBA_PORTAL_ORIGIN`, `EKZA_DEV_HTTP_HOST`), including the iOS archive.
- The dev lobby itself is shipped with `ops/vps/dev/lobby_dev.py`; see [ops/vps/dev/README.md](../ops/vps/dev/README.md).

## Signing and trust

- **macOS:** ad-hoc signed, not notarized. Players right-click → Open once
  (or `xattr -dr com.apple.quarantine Omoba.app`). Notarization needs a
  Developer ID Application certificate and `notarytool` credentials.
- **Windows:** unsigned; SmartScreen shows "More info → Run anyway".
- **Android:** signed with one stable playtest key so updates install over each
  other. Locally it lives in `~/.config/omoba/android-playtest.keystore`; for CI
  store it as the secret `OMOBA_ANDROID_KEYSTORE_BASE64`
  (`base64 -i ~/.config/omoba/android-playtest.keystore | gh secret set OMOBA_ANDROID_KEYSTORE_BASE64`).
  Without a stable signing key, omit Android from the release; do not run a workflow that generates a replacement key.
  Store publication needs a real release key.
- **iPhone:** App Store distribution signing, managed by Xcode.

## Hosting for a playtest

Every desktop package contains a practice host (`Host Practice Server.command`,
`Host Practice Server.bat`, `host-practice.sh`). Players must reach the host's
**UDP 4000**: same LAN, a VPN (Tailscale/ZeroTier) or a router port forward. For
an always-on server, run the Linux package on a VPS with
`omoba-practice.service` and `ufw allow 4000/udp`. The public lobby with
PostgreSQL is documented in [public-mvp.md](public-mvp.md).
