# Install a development build on your iPhone

`build_device.py` packages the native `aarch64-apple-ios` executable. It can sign
with an **existing** valid Apple Development identity and a matching, unexpired
iOS development profile. It never creates certificates, contacts provisioning
services, installs an app, or exports a private key. No extra Python or Rust
production dependency is required.

The Simulator script is separate: its arm64 executable cannot run on an iPhone.
The device builder checks Mach-O platform metadata, not just the CPU name.

## Build and package

Run from the primary repository root (`omoba-bevy`). No adjacent worktree or
private project toolchain is required. With a selected Xcode installation and
Rust installed through rustup, install the device standard library once:

```sh
rustup target add aarch64-apple-ios
make iphone-check
make iphone
```

`make iphone` creates an unsigned physical-device package. Use the existing
credential workflow below to sign it for installation. `builds/` holds retained
packages; `target/iphone-cargo` holds disposable compiler output. Neither belongs
in Git. Subsequent builds need a fresh output, e.g. `make iphone
IPHONE_OUTPUT=builds/iphone-2` on one line. Never remove `builds/` as a cache.

Use Xcode with the iPhoneOS SDK and the Rust `aarch64-apple-ios` standard library.
Select the intended Xcode/toolchain before running this script. The existing
Bevy application entry is also the physical-device entry; no Swift bridge is
needed. Check prerequisites without writing or starting Cargo:

```sh
python3 mobile/ios/build_device.py --check
```

For a source build, use a fresh output directory and optionally reuse a Cargo
cache. The default `dev` profile keeps the workspace optimizations and disables
debug data/incremental compilation to reduce build storage. `--build-profile
release` is also available. At least 6 GiB free is required by fresh-build
preflight; actual peak usage varies.

```sh
python3 mobile/ios/build_device.py \
  --output builds/iphone-build-1 \
  --target-dir target/iphone-cargo \
  --server 'YOUR_REACHABLE_SERVER:PORT'
```

The server is an editable initial value. `127.0.0.1` on the phone means the
phone itself; use your server's reachable LAN/hosted address. Permit local
network access when iOS asks for a LAN playtest.

To reuse a compiled executable, `--binary` skips Cargo entirely:

```sh
python3 mobile/ios/build_device.py \
  --binary /absolute/path/to/aarch64-apple-ios/debug/client \
  --output builds/iphone-unsigned-1
```

Unsigned output is useful for reviewing its contents, but is not installable.
Only tracked `client/assets` files are copied, including model/sprite registries,
fonts, reactions, audio and their bundled notices. Required project notices
are copied into `assets/legal`. Untracked files and symlinks are rejected or
excluded. A fresh output prevents old removed assets from surviving a rebuild.

## Sign using existing local credentials

Supply an exact existing identity name or certificate SHA-1 fingerprint and an
existing profile path. Optionally check that the profile includes your physical
phone's UDID. Keep these personal values, profiles and generated app out of Git.
The default bundle identifier is `space.ekza.omoba.beta`; a wildcard development
profile can permit it. A different explicit `--bundle-id` creates a separate app
identity and will not share the existing app's sandbox/settings.

```sh
python3 mobile/ios/build_device.py \
  --binary /absolute/path/to/aarch64-apple-ios/debug/client \
  --profile /absolute/path/to/existing.mobileprovision \
  --identity 'EXISTING_CERTIFICATE_SHA1' \
  --device-udid 'YOUR_IPHONE_UDID' \
  --output builds/iphone-signed-1 --check

# Repeat without --check after reviewing the preflight result.
```

The script validates expiration, iOS development use, app identifier, team/prefix
consistency, optional registered-device membership and certificate membership.
It embeds the profile, signs with minimal validated development entitlements,
then verifies the signature and reads those entitlements back. Signing may ask
for local Keychain permission; the script does not change Keychain access rules.
The app contains personal provisioning data as required by iOS; do not publish
this local development artifact as a public download.

## Install and launch separately

The repository also includes an installer that selects an available, enrolled
physical iPhone and checks the existing profile expiration and Developer Mode:

```sh
python3 mobile/ios/install_device.py --app builds/iphone/OmobaBeta.app --check
python3 mobile/ios/install_device.py --app builds/iphone/OmobaBeta.app
```

The first command checks readiness; the second installs and launches. With several
eligible phones, specify the desired identifier using `--device`. It never
uninstalls the existing app. Connect/unlock/trust the iPhone before running it.

Prepared home-playtest kits under `builds/` include `1-Start-server.command` and
`2-Install-on-iPhone.command`. The first runs a bundled Mac practice server and
prints its current LAN address; the second runs the installer. These kits need
neither a Cargo build nor PostgreSQL at play time. Connect both devices to the
same Wi-Fi, allow local-network access on the phone, then enter the printed
`IP:4000` through the game's **SERVER** button and select **CONNECT**. Practice
uses server bots and produces local results without permanent career/rating credit.

Connect/unlock/trust the phone, enable Developer Mode when required, and use the
device identifier reported by Xcode/devicectl. This identifier can differ from
the provisioning UDID. On current Xcode, the explicit actions are:

```sh
xcrun devicectl device install app --device 'YOUR_DEVICE_IDENTIFIER' \
  builds/iphone-signed-1/OmobaBeta.app
xcrun devicectl device process launch --device 'YOUR_DEVICE_IDENTIFIER' \
  space.ekza.omoba.beta
```

Installing an update should preserve the app sandbox. Do not uninstall as an
automatic recovery step. If no matching profile or trusted phone is available,
the prepared unsigned app remains reviewable; a signing/install success must
not be claimed until the corresponding Apple tool actually succeeds.

`device-build.json` records every packaged file hash and distinguishes signed,
installed and launched state. Installation/launch are always false in this
builder's report because they happen separately. It does not assert the supplied
binary matches the checkout: retain the actual Cargo build log/source hashes
alongside the report. First-device checks still include launch, touch/keyboard,
sound, joining a real server, background/resume, frame rate and thermal behavior.
This workflow does not publish to TestFlight or the App Store.

## Verification and references

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s mobile/ios -p 'test_build_device.py' -v
```

Tests use synthetic Mach-O/profile fixtures and an isolated temporary Git tree.
They do not use real signing credentials or prove physical gameplay.

Installer selection and failure handling can be checked independently with:

```sh
python3 -B -m unittest discover -s mobile/ios -p 'test_install_device.py' -v
```

- [Bevy 0.18 iOS example](https://github.com/bevyengine/bevy/tree/v0.18.0/examples/mobile)
- [Rust iOS targets and SDK requirements](https://doc.rust-lang.org/rustc/platform-support/apple-ios.html)
- [Apple: Run an app on a device](https://help.apple.com/xcode/mac/current/en.lproj/dev5a825a1ca.html)
- [Apple: Enable Developer Mode](https://developer.apple.com/documentation/xcode/enabling-developer-mode-on-a-device)
