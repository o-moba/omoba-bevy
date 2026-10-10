# 0.44.0 mobile distribution and Android launch investigation

The live lobby at `77.246.105.57:4000` was rechecked against the Windows/iOS
0.44.0 manifest: protocol 11, catalog `standard-kits-7`, geometry
`verdant-confluence-compact-v2`, gameplay `wildspark-2026-10-09-reference`.
The service runs `/opt/omoba/releases/0.44.0-beta-1952af2`; see the
[coordinated rollout](2026-10-10-release-044.md) for worker/gameplay/reconnect evidence.

## Windows public download

[Download OMOBA 0.44.0 for Windows x64](https://ekza.mypinata.cloud/ipfs/bafybeif7pb4xxtvh5rk7kmyemxwk3swzaptyj6mbc2re2kytzhrtdx7i4e/Omoba-0.44.0-windows-x64.zip).
The exact locally built archive is retained on Pinata/IPFS for download, with no
automatic unpinning. Public bytes were downloaded and verified against the local
artifact: 200774638 bytes, SHA-256
`16b0fcb96463fc4a7e3557650e187607b876040c7d0453c27a0c65739228217b`. Source is
`1952af24125dce8dc5be7926a56fced1a48f6b84`; the ZIP contains matching source and
compatibility manifests, assets, legal notices, client and practice server.
Unpack and start `Omoba.exe`; the public server is compiled as the initial address.
Saved settings can override it. The binary is unsigned and physical Windows
rendering/input testing remains outstanding. The GitHub prerelease remains a draft;
this IPFS link is the verified public distribution endpoint.

## iOS

Local release compilation and unsigned archive validation passed for 0.44.0 (22),
source `11660558950179b13fd48a309da33b335c55da6e` (clean main; runtime matches
Windows/server `1952af2`). Bundle ID remains `space.ekza.omoba.beta` and the default
server is the live endpoint. iPhone/iPad assets, encryption declaration and nonempty
matching arm64 crash symbols were verified. The executable UUID is
`428AE181-FAD5-312D-A19E-9E1C88AA14D4`.

Signing failed with `errSecInternalComponent`; macOS keychain access returned
`User interaction is not allowed`. The owner was asked to unlock the existing
login keychain. The retained archive is
`builds/mobile-0.44.0-22/Omoba-unsigned.xcarchive`.
**This archive is unsigned, not uploaded, not processed and not available in
TestFlight.** No credentials or certificates were replaced. Resume signing and
upload only after access to the existing identity is restored. No physical iOS
playtest is claimed.

## Android

The reported Android 12 version is above minimum API 26, but the phone model,
installed APK and crash evidence are still unknown. No Android device was
attached. The retained local 0.34.2 APK contains arm64-v8a, armeabi-v7a and x86_64
slices. All three have the old missing C++ ABI runtime issue repaired; remaining
`__cxa_finalize`/`__cxa_atexit` imports are explicitly supplied by Android libc.
That older APK is incompatible with the current server and was not redistributed.
It is not established that this is the APK on the affected phone.

Added `scripts/android_diagnose.py` and `make android-diagnose` to collect model,
OS, ABI, advertised Vulkan/OpenGL support, installed game version, exit history
and OMOBA crash groups. Collection is read-only unless `--launch` is explicit;
no log clearing, installation, key rotation or device serial is recorded.
Existing output is preserved. Tests cover ABI mismatch, native loader/GPU/memory
evidence, unrelated-crash filtering, missing devices and read-only behavior.
Documentation now separates OS, native ABI and GPU requirements. Universal ABI
packaging is already available; GLES fallback remains unimplemented and needs
renderer configuration plus physical-device validation. An unseen device's
root cause has not been claimed.

This change is developer tooling and documentation; no gameplay, protocol,
runtime version, signing identity or CI workflow changed. Local Python tooling
checks passed (240 script tests and 45 iOS tooling tests). Mocked phone actions
in those tests are not physical-device evidence.

Raw evidence is retained in `.agent/tasks/MOBILE-DISTRIBUTION-20261010/`.
