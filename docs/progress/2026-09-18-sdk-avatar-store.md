# 2026-09-18 — Ekza avatars reach Omoba through the SDK at runtime

Task: `.agent/tasks/SDK-AVATAR-STORE-20260918/`.

## Goal

Artists publish avatars to the Ekza registry, players buy them, and Omoba shows
a purchased avatar to its owner and to every other player, with all of it
flowing through `ekza-bevy-sdk`.

## What was validated first (live services, 2026-09-18)

- `registry.ekza.io/v1/avatars`: live, 6 devnet templates, renditions
  `universal/vrm-humanoid-v0` and `ios/arkit-body-v1` only, no `projectSupport`.
- `avatar.ekza.io/api/passport/*`: HTTP 404, the passport is not deployed.
- Library models on `ekza.mypinata.cloud`: HTTP 403 for anonymous clients.
- A live `registry_sync --project omoba` therefore stages 0 avatars. The SDK
  wire formats match the services (`ekza-mirror/backend`, `solana-avatars/app`);
  the gap is published data and deployment, not code.

## Changes

- SDK 0.4.0 (sibling repo): `passport::protected_slug`, `store::AvatarStore`
  (approved templates for a selector, offline catalogue, verified on-demand
  install), `NativeSession::from_parts`, rustls transport.
- `omoba-passport`: passport HTTP transport removed in favour of the SDK
  client; new engine-free `store` module (background catalogue, install on
  first use, change notifications).
- `shared`: `register_store_avatar` / `store_avatars`; lookups and slug
  normalization cover shipped plus registered entries; a registered entry must
  be named by the hash of its own passport boundary and cannot shadow a
  shipped slug.
- `server`: a store slug is admitted from the consumed ticket alone.
- `client`: `ekza://` asset source over the private settings directory, owned
  store avatars in the grid, install before the join ticket, on-demand install
  and respawn for remote players, thumbnails from the store.

## Checks

- SDK: `cargo test --no-default-features --features http` 30 passed; without
  features 21 passed; clippy clean.
- Omoba (SDK patched to the local checkout): `shared` 59, `omoba-passport` 8,
  `server` 173, `client` 378 passed.
- End to end against a local `ekza-mirror` registry serving the roundtrip
  fixture with an Omoba approval:
  `OMOBA_REGISTRY_URL=http://127.0.0.1:8019 cargo test -p omoba-passport --test store_e2e -- --ignored`
  lists Robert, installs 1,885,072 bytes, fetches the thumbnail, and
  `verify_local` accepts the file. The derived slug equals the one the older
  `passport-import` produced, so existing imports stay valid.

## Not verified

- A full match with two windows and a real paired wallet (needs the storefront
  rehearsal with a funded devnet wallet).
- iOS/Android builds with the new asset source.

## Remaining operator steps before this works in production

1. Commit and push `ekza-bevy-sdk` 0.4.0, then pin its revision in
   `client`, `shared`, `server` and `passport` `Cargo.toml` and refresh
   `Cargo.lock` (until then build with
   `--config 'patch."https://github.com/ekza-space/ekza-bevy-sdk".ekza-bevy-sdk.path="../ekza-bevy-sdk"'`).
2. Publish `desktop/humanoid-glb-v1` renditions (clips baked with
   `scripts/retarget_animations.py`) for the registry templates and approve
   them for `omoba` with `ekza-mirror/backend/app/passport_cli.py`; redeploy
   the registry catalogue.
3. Deploy the passport API (`solana-avatars/app`, stateful, not serverless) at
   the storefront origin and set `OMOBA_PASSPORT_URL` on game servers.
4. Follow-up: in-game wallet pairing UI instead of the terminal flow.

## Roster policy

The 15 shipped CC0 avatars stay as the default, free roster. Everything new
comes through the SDK store and is listed in its own "Your Ekza avatars" group
in the picker. A dev client started with `OMOBA_REGISTRY_URL=http://127.0.0.1:8019`
mounted the `ekza://` source without errors and persisted the catalogue
(`ekza-store/store.json` with Robert); the picker itself was checked by the
UI test, not by eye.
