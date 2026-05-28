# 2026-05-28 - Ekza Bevy SDK extraction

## Goal

Start turning the in-game 3D model integration layer into a reusable SDK that other developers can later consume as a dependency for Ekza-Stellar universe models.

## What changed

- Extracted `ekza-bevy-sdk` into the standalone sibling repository at `../ekza-bevy-sdk`.
- Updated Omoba Bevy to consume the SDK through sibling path dependencies from the client and server crates.
- Converted the SDK manifest away from Omoba workspace inheritance so it can build on its own.
- Moved stable character identity into `ekza_bevy_sdk::EkzaCharacter` while preserving existing snake_case serde values (`ipfs`, `toka`, `wang`, `cube`).
- Added built-in model manifest metadata for IPFS, Toka, Wang, and Cube.
- Added SDK GLB header validation helpers.
- Added a headless SDK `model_cache` example to resolve and validate the built-in model sources against Omoba's local asset root.
- Added a Bevy feature module with `EkzaModelCatalog`, local GLB handle resolution, and remote GLB caching under a consumer asset root.
- Updated the client to consume the SDK model catalog instead of owning character/model catalog logic directly.
- Updated the server protocol type to use the shared SDK character id.

## Verification

- `cargo fmt --all` - PASS
- `cargo fmt --all -- --check` - PASS in both `omoba-bevy` and `../ekza-bevy-sdk`
- `cargo test --all-targets` - PASS in `../ekza-bevy-sdk` (7 SDK tests plus example compilation)
- `cargo test --no-default-features` - PASS in `../ekza-bevy-sdk` (core SDK without Bevy)
- `cargo run --example model_cache -- --asset-root ../omoba-bevy/client/assets --all` - PASS (IPFS remote cache/local file, Toka local GLB, Wang local GLB, Cube fallback)
- `cargo test --workspace` - PASS (25 client tests, 24 server tests, 5 skills tests)
- `cargo test -p server` - PASS (24 server tests with the SDK consumed without default features)
- `cargo build --workspace` - PASS

## Remaining risks / next slice

- The SDK is now standalone locally, but it is still not a published package.
- Remote model download is still blocking and should become async/non-blocking before a polished SDK release.
- Entitlements, licensing, account auth, signed manifests, CDN configuration, and example consumer apps remain out of scope for this slice.
