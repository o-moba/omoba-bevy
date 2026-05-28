# 2026-05-28 - Ekza-Stellar SDK extraction

## Goal

Start turning the in-game 3D model integration layer into a reusable SDK that other developers can later consume as a dependency for Ekza-Stellar universe models.

## What changed

- Added `ekza-stellar-sdk` as a workspace library crate.
- Moved stable character identity into `ekza_stellar_sdk::EkzaCharacter` while preserving existing snake_case serde values (`ipfs`, `toka`, `wang`, `cube`).
- Added built-in model manifest metadata for IPFS, Toka, Wang, and Cube.
- Added SDK GLB header validation helpers.
- Added a Bevy feature module with `EkzaModelCatalog`, local GLB handle resolution, and remote GLB caching under a consumer asset root.
- Updated the client to consume the SDK model catalog instead of owning character/model catalog logic directly.
- Updated the server protocol type to use the shared SDK character id.

## Verification

- `cargo fmt --all` - PASS
- `cargo test --workspace` - PASS (23 client tests, 3 SDK tests, 17 server tests, 5 skills tests)
- `cargo build --workspace` - PASS

## Remaining risks / next slice

- The SDK is still a workspace crate, not a published package.
- Remote model download is still blocking and should become async/non-blocking before a polished SDK release.
- Entitlements, licensing, account auth, signed manifests, CDN configuration, and example consumer apps remain out of scope for this slice.
