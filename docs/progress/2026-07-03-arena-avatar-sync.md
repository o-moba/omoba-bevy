# 2026-07-03 — Ekza Arena avatar sync (arena-sync)

## Goal

Let players use avatars published on-chain (Ekza Stellar → Ekza Arena bridge,
e.g. the Open Source Avatars dataset) as omoba characters, with the model
format checked transparently against what the game can load.

## Changes

- New workspace crate `arena-sync` (bin): reads `ArenaAssetData` accounts from
  the Solana RPC (raw `getProgramAccounts`, base64 + minimal borsh cursor —
  no anchor/solana crates), keeps `card_kind == Avatar`, fetches each card's
  metadata JSON, gates on the `format` classifier (`vrm`/`glb` only —
  mirrors the on-chain `ProjectProfile "omoba"` capability card in
  solana-stellar, see its `docs/INTEGRATION.md`), downloads model +
  thumbnail into `client/assets/avatars/<slug>.glb|png`, merges entries into
  `manifest.json` (collection "Ekza Arena", license/author from the card's
  attribution). Idempotent by slug.
- `shared::avatar_roster()` now loads the manifest from disk at runtime
  (`OMOBA_AVATAR_MANIFEST` env override, then repo-relative candidates),
  falling back to the embedded copy. Synced avatars show up in the
  team-select avatar grid after a client restart, no rebuild.
- Roster test asserts invariants instead of a fixed size window.

## Checks

- `cargo build --workspace` green; `cargo test -p shared` 7/7.
- Live run against localnet: 54 arena accounts → 14 avatar cards → 12 synced
  (12 OSA avatars: model GLB header validated, thumbnails saved, manifest
  28 entries total), 2 skipped with explicit reasons (non-fetchable IPFS-hash
  metadata pointers from test fixtures).
- Format gate proven live: cards seeded before the `format` field existed
  were skipped with a "format not supported" message until their metadata
  was classified.

## Remaining risks / follow-ups

- Client and server must read the same manifest file; a mismatch rejects
  joins for runtime-added avatars (documented in code).
- Arena-synced avatars use the shared humanoid animation set; VRM models with
  exotic rigs may need retargeting like the committed roster did.
- Thumbnails from the chain are PNG (roster ships JPEG) — both load, bevy
  `jpeg`+`png` features are enabled.
- Localnet pointers (`http://127.0.0.1:8787/...`) are only fetchable while
  the metadata server runs; production cards should pin metadata to IPFS.
