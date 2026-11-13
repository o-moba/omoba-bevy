# Account regions, connection profiles and the OMOBA Dev app

The player chooses an account region on first launch, before regional network
traffic. Returning installations restore the authenticated account binding (or a
single existing regional identity) automatically. Home, Party and phone controls
show no IP, port, server field or Dev/Local/Custom selector in either contour.
Settings → Account region is available outside an active match or purchase; it
preserves both profiles and requires a cold restart. The restart card blocks all
regional traffic. Desktop Restart starts a new process and exits the old one;
phones must close and reopen the app.

`shared/src/region.rs` owns the endpoint sets. A process binds exactly once.
`client/src/account_region.rs` stores bounded, private device metadata in
`omoba-bevy-device[-contour]/account_region.json`: schema, selected region,
pending switch and the authenticated public key/profile/region binding. No private
key or bearer credential belongs there. A corrupt record asks again. Credentials
are never submitted to two backends to discover an account's region.

| Account | Production lobby / API / portal | Dev lobby / API / portal |
| --- | --- | --- |
| International | `77.246.105.57:4000`, `account.omoba.io`, `app.omoba.io` | `lobby.dev.omoba.io:4100`, `account.dev.omoba.io`, `app.dev.omoba.io` |
| RU | `lobby.omoba.ru:4000`, `account.omoba.ru`, `omoba.ru` | `lobby.dev.omoba.ru:4100`, `account.dev.omoba.ru`, `dev.omoba.ru` |

These are routing targets, not a claim that all services are deployed. The RU dev
authority requires provisioning; unavailable RU services never fall back to global.
See the [RU release gates](plans/ru-release-2026-10-28.md).

Region folders preserve existing installations: `omoba-bevy`, `omoba-bevy-ru`
(and `-dev` forms). Each owns its key, legal acceptance, preferences, account link,
caches and match resume. With `OMOBA_CLIENT_CONFIG_DIR`, International retains the
legacy root and the unified client's RU state uses `regions/ru`; locked RU packages
retain their legacy root. Device-wide preference migration and iOS Keychain
persistence from the full LR-68 plan remain future work.

Production and dev are separate app identities (`scripts/contours.py`); dev retains
its DEV markings. Both known production IPs, `77.246.105.57` and
`212.113.117.105`, are denied in dev, including worker redirects. A bound region
also refuses known foreign-region lobby hosts. This static host policy is not a
signed server directory or a defense against arbitrary DNS rebinding.

Desktop developers can explicitly set `GAME_SERVER_ADDR`, `OMOBA_REGISTRY_URL`,
`OMOBA_PASSPORT_URL` and `OMOBA_CLIENT_CONFIG_DIR`; phones ignore endpoint runtime
overrides. Contour and known foreign-host guards still apply. Account API and
portal overrides after binding are limited to explicit insecure loopback rehearsals
(`OMOBA_ALLOW_INSECURE_LOCAL=1`, both endpoints loopback). RU disables Ekza and pins
its registry/passport to closed loopback. Existing raw-profile resolver helpers
remain internal for tooling/tests; saved `connection_profile.json` cannot reroute a
player installation.

`OMOBA_MARKET=ru` remains a locked RU client/server/API packaging option. Default
clients register the union of regional plugins and gate integrations at runtime.
Account API device replies include `account_region`; native clients reject a
foreign region before accepting a reply. An older API may omit the field while
its exact trusted regional origin continues to define the boundary.

Focused QA: a dev-contour native client with `OMOBA_ACCOUNT_REGION_QA=<output>`,
`OMOBA_CLIENT_CONFIG_DIR=<empty scratch folder>`, `OMOBA_TOUCH_CONTROLS=1`,
`OMOBA_LANGUAGE=ru`, `OMOBA_OFFLINE_PRACTICE=1` verifies first RU confirmation,
late identity initialization and a blocked cold switch at 852×393. It writes two
screenshots and a JSON result. This is a desktop phone preview, not a physical
phone test.

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
  the reverse). For Run or Archive from Xcode, select the shared **Omoba Dev** scheme.
  Its `Debug Dev` / `Release Dev` configurations pin the dev contour over local settings. `release.py` selects that scheme for dev and always passes `OMOBA_CONTOUR` and
  `OMOBA_GAME_SERVER` on the `xcodebuild` command line (empty server for production
  without `--server`), so these Xcode-run lines never reach a release archive, and it
  refuses an archive whose bundle id is not the requested app's. The LAN rehearsal
  (`.ekza-lan/client.json`) applies to the production app's Debug runs only; desktop dev rehearsals use explicit environment overrides.
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
6. **First start.** Confirm an account region; returning installations restore it.
   Settings → Account region changes it after a cold restart.
