# Ekza main integration and demo preparation — 2026-10-01

## Change

Pin the workspace SDK dependency to `cef1d6ad43e364ad0a116dc7be74122854ee8b7e`, now on the SDK main branch. This includes the previously pinned LAN fix, scoped account persistence, catalogue availability regressions and the main branch license documents. No transport/authentication checks were relaxed. Repository version: 0.31.1.

## Demo gate

Follow [the production avatar runbook](../ekza-production-avatars.md). Publish one original VRM, process it, curate it, approve `omoba / desktop / humanoid-glb-v1`, then install/equip it through the live SDK and join a match observed by a second client. The same selector applies to current phone builds.

At integration time the public API exposes health but Studio status, profiles and v2 catalogue return 404. The frontend is deployed at https://ekza-studio.vercel.app/studio; this does not make uploads operational. Backend SSH closes before authentication. Worker deployment, large uploads, IPFS publication, account provisioning and real phone acceptance remain open.

## Verification

- `cargo test --locked -p omoba-passport`: 28 unit tests passed; three opt-in live integration tests remained ignored because their external fixtures are not configured.
- `cargo check --locked -p client -p server`: passed for the exact pinned SDK.
- SDK HTTP regression suite: 44 tests and two doc tests passed before publication to main.
- No model binaries, gameplay rules or production credentials changed. Prior twenty-model local verification is not a new cloud demonstration.
