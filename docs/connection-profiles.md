# Connection profiles, the OMOBA Dev app and its release

OMOBA ships as two apps that install side by side on one computer, iPhone or
Android phone:

| | OMOBA (production) | OMOBA Dev (dev contour) |
| --- | --- | --- |
| Build | default | `--contour dev` (`OMOBA_DEFAULT_CONFIG_DIR_SUFFIX=dev`) |
| iOS / Android id | `space.ekza.omoba.beta`, "Omoba Beta" | `space.ekza.omoba.dev`, "OMOBA Dev" |
| Desktop | `Omoba.app`, `space.ekza.omoba.desktop` | `Omoba Dev.app`, `space.ekza.omoba.desktop.dev`, `-dev` packages |
| Icon | production icon | the same icon with a red DEV ribbon (`scripts/make_dev_icons.py`) |
| Settings folder | `omoba-bevy` | `omoba-bevy-dev` |
| Marking | none | window title `OMOBA DEV`, `DEV` badge and profile badge on Home, `DEV` under the match score strip |
| Servers | production lobby and Ekza registry, always | the selected connection profile |
| Server UI | none | Home → Connection, Party → Connection → Change |

The identities live in `scripts/contours.py`, shared by `scripts/release.py`,
`mobile/android/build.py` and the Xcode build phase (`mobile/ios/xcode_build.py`).

## Production: no server selection

A production build has exactly one profile (`client/src/connection_profile.rs`):
the compiled lobby (`77.246.105.57:4000` unless a release passes `--server`), the
registry `https://registry.ekza.io`, the passport `https://avatar.ekza.io/api/passport`,
the market `app.omoba.io/market`. It spawns no server field, preset, SERVER button
or connection panel on desktop or phone, in Home, the party lobby or the legacy
phone bar (tests: `production_builds_spawn_no_server_or_connection_ui_on_desktop_or_phone`,
`only_a_contour_build_offers_the_connection_button_and_profile_badge`). The
`game_server_addr` an older build saved is no longer read, so it cannot pin a
build that has no way to change it. Likewise a match-resume hint
(`match-resume.json`) is only offered when it names the lobby this process connects
to; a hint an older build saved for a custom, LAN or local lobby is deleted at start
instead of sending the player back there (`a_hint_for_another_lobby_is_dropped_and_deleted`).

Developers and self-hosters of this open-source game override the endpoints on a
**desktop** with environment variables (phones read none):

| Variable | Overrides |
| --- | --- |
| `GAME_SERVER_ADDR=host:port` | the lobby (`make game` sets `127.0.0.1:4000`) |
| `OMOBA_REGISTRY_URL=https://…` | the Ekza registry (stores, account link) |
| `OMOBA_PASSPORT_URL=https://…/api/passport` | the Ekza passport (also `EKZA_PASSPORT_URL`) |
| `OMOBA_CLIENT_CONFIG_DIR=/path` | the settings folder |

The desktop OMOBA Dev app reads the same variables over its running profile,
but ignores one that names a production lobby, registry or passport (the same
refusal as a custom profile).

A self-hosted production build can also compile its own lobby and registry:
`scripts/release.py build <platform> --server host:port --registry https://…`.

## Dev: profiles Dev, Local, Custom

| Profile | Lobby | Registry | Market | Studio | Passport |
| --- | --- | --- | --- | --- | --- |
| **Dev** (default) | `lobby.dev.omoba.io:4100` (or the build's `--server`) | `https://registry.dev.ekza.io` | `app.dev.omoba.io/market` | `studio.dev.ekza.io/studio` | closed (`http://127.0.0.1:9/api/passport`) |
| **Local** | `127.0.0.1:4000` | `http://127.0.0.1:8018` | `127.0.0.1:3010/market` | `127.0.0.1:5188/studio` | closed |
| **Custom** | typed `host:port` | typed origin, optional (else Dev's) | Dev's | Dev's | closed |

There is no Production profile in the dev app. A custom lobby or registry that is
the production VPS (any port) or a production `omoba.io` / `ekza.io` name is refused
(`connection.error.production`), the same rule (`endpoint_allowed_for`) that already
refuses production for `GAME_SERVER_ADDR` and match allocations. A registry is
`https://` anywhere or `http://` on loopback only, without credentials, path, query
or fragment (the SDK's rule).

**Switching.** The Connection panel (kit modal: profile tiles, the profile's
endpoints, custom text fields, a numeric keypad on phones) shows the running profile.
**Use** asks to confirm; **Switch** saves the choice in
`<settings>/connection_profile.json`. The switch applies **at the next start**: the
Ekza stores, the account link and the `ekza://` asset source open once per process,
so the running game never mixes two registries. On a desktop **Restart now** starts
a fresh copy and exits; on a phone close the app and open it again. Choosing the
running profile cancels a saved switch. Home shows the running profile next to the
DEV badge.

**Per-profile state.** Each profile keeps its own folder,
`<settings>/profiles/<dev|local|custom-<hash>>/`, holding the Ekza account link
(`ekza-store/account/session.json`; the SDK refuses a link for another registry),
the avatar and weapon store caches (`ekza-store/`) and the match-resume hint. A
custom profile's folder is keyed by its lobby and registry, so another custom server
never sees its link. Graphics, audio, language, the profile card and the career
identity stay shared. A dev build from before profiles kept its store in
`<settings>/ekza-store`; the Dev profile adopts it once. Production keeps everything
in the settings folder itself, as before.

## Building and releasing the dev app

```sh
python3 scripts/release.py build all --contour dev --dry-run    # what each package bakes in
python3 scripts/release.py build macos --contour dev             # Omoba Dev.app (DEV icon)
python3 scripts/release.py build android --contour dev          # space.ekza.omoba.dev APK
python3 scripts/release.py build ios --contour dev              # space.ekza.omoba.dev archive
python3 scripts/release.py build ios --contour dev --upload     # … and upload to App Store Connect
python3 scripts/release.py ci --contour dev --ref v<version>-dev.1   # desktop + Android in CI
```

- `--server <dev VPS IPv4>:4100` replaces the Dev lobby when DNS is not set. Phone
  builds take no `--registry`/`--passport`: they compile the contour's own.
- Android: `mobile/android/build.py --contour dev` sets the package, the label
  `OMOBA Dev`, the `res-dev/` launcher icons and the dev compile defaults; the APK is
  `omoba-<version>-dev-android-<abi>-debug.apk` (`release.py` names it
  `Omoba-<version>-dev-android-<abi>.apk`). It is signed with the same playtest key.
- iOS: `OMOBA_CONTOUR=dev` (an Xcode build setting; `release.py` passes it) selects
  `PRODUCT_BUNDLE_IDENTIFIER = space.ekza.omoba.dev` and the `AppIconDev` icon set in
  `mobile/ios/Omoba.xcconfig`; `xcode_build.py` then names the app `OMOBA Dev`,
  compiles the Dev profile and refuses a dev build under the production bundle id (and
  the reverse). For a Run from Xcode, put `OMOBA_CONTOUR = dev` in
  `mobile/ios/Omoba.local.xcconfig`. `release.py` always passes `OMOBA_CONTOUR` and
  `OMOBA_GAME_SERVER` on the `xcodebuild` command line (empty server for production
  without `--server`), so these Xcode-run lines never reach a release archive, and it
  refuses an archive whose bundle id is not the requested app's. The LAN rehearsal
  (`.ekza-lan/client.json`) applies to the production app's Debug runs only; the dev
  app has Local and Custom.
- Icons: `python3 scripts/make_dev_icons.py` regenerates the dev icons from the
  production ones (stdlib only, deterministic); `--check` and
  `scripts/test_dev_icons.py` fail when a committed icon is stale.

## Owner checklist (once)

1. **App Store Connect record.** Apps → + → New App: platform iOS, name
   "OMOBA Dev" (any free name; the home-screen name comes from the build), bundle
   id `space.ekza.omoba.dev` (Xcode creates the App ID on the first automatic-signing
   archive; or add it under Certificates, Identifiers & Profiles → Identifiers), SKU
   e.g. `omoba-dev`. Do **not** submit it for review or external testing.
2. **TestFlight internal only.** In the record: TestFlight → Internal Testing → add a
   group with yourself (and team members). Export compliance: the same answer as the
   production app (`mobile/ios/EXPORT-COMPLIANCE.md`; `ITSAppUsesNonExemptEncryption`
   is already `false` in the plist).
3. **Signing.** Nothing new to install: the archive uses automatic signing with the
   team in `mobile/ios/Omoba.local.xcconfig` (`DEVELOPMENT_TEAM`); Xcode (signed in
   under Settings → Accounts) creates the distribution profile for the new id on the
   first `-allowProvisioningUpdates` archive.
4. **Build and upload from the Mac.** `python3 scripts/release.py build ios --contour dev --upload`.
   If the command-line export is refused, the signed archive opens in Xcode
   Organizer as "Omoba Dev <version> (<build>)": Distribute App → App Store Connect →
   Upload. Build numbers share the counter in `Omoba.local.xcconfig` with production.
   After processing, add the build to the internal group; install from the
   TestFlight app. It sits beside "Omoba Beta".
5. **Android side by side.** `python3 scripts/release.py build android --contour dev`
   (or the dev CI run), then `adb install -r dist/v<version>-dev/Omoba-<version>-dev-android-arm64.apk`
   or open the APK on the phone. It installs next to the production app
   (different package); uninstalling one never touches the other's data.
6. **First start.** The dev app opens on the Dev profile. Home → Connection to pick
   Local or Custom; confirm and restart.
