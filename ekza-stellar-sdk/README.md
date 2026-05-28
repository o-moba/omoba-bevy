# Ekza Stellar SDK

`ekza-stellar-sdk` is the first extracted SDK surface for using Ekza-Stellar universe character identities and 3D model metadata outside the Omoba Bevy game.

This slice intentionally stays local to the workspace. It is shaped so it can later be published and consumed as a dependency by other Bevy projects.

## Public Surface

- `EkzaCharacter` - stable serde-compatible character ids (`ipfs`, `toka`, `wang`, `cube`).
- `BUILTIN_MODEL_MANIFEST` - built-in character-to-model metadata.
- `is_valid_glb_bytes` - minimal GLB header validation helper.
- `bevy::EkzaModelCatalog` - Bevy resource-friendly catalog of `Scene` and `Gltf` handles.
- `bevy::load_builtin_model_catalog` - resolves local downloaded GLBs and caches remote GLBs under a consumer asset root.

## Current Boundaries

- This crate does not perform account auth, entitlement checks, CDN signing, or asset licensing enforcement yet.
- Remote model downloads are blocking and intended for the current desktop prototype path.
- Gameplay protocol and MOBA-specific state remain in the game crates for now.
