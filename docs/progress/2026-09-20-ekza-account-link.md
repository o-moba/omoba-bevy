# Ekza account in the avatar picker — 2026-09-20

Goal: the player's own story for the demo: "I saved an avatar on Ekza and it is in my
game", with no Solana wallet.

## Design
- Same device flow shape as wallet pairing, against the registry instead of the wallet
  storefront: `POST /v1/account/device`, poll, then `GET /v1/account/library`.
  `ekza-bevy-sdk` 0.6.0 `account` module; `omoba_passport::account` only picks the
  registry origin (`OMOBA_REGISTRY_URL`) and Omoba's selector.
- The session is presentation only. `library_avatars()` is the subset of free store
  avatars whose slug the account's library contains; `community_avatars()` is the rest.
  Admission is unchanged and server-side (see 2026-09-19-ekza-community-avatars.md).
- `poll_account()` opens the browser once per link, stores the session on approval and
  afterwards refreshes the library on a worker thread at most every 15 seconds. A failed
  refresh keeps what is shown.

## Changes
- `passport/src/account.rs`; `client/src/passport.rs` account state, status line,
  `library_avatars`; `client/src/team.rs` second status line, "Connect Ekza account"
  button, "My Ekza library" group, overlay rebuild when the library or community list
  changes. SDK pinned to `31b0335` (0.6.0).

## Checks
- `cargo check -p client`, `cargo test -p server -p shared -p omoba-passport`: see the
  Ekza umbrella evidence `.agent/tasks/T6-2-ACCOUNT-LINK/evidence.md`.
- The flow itself was proven with the SDK example `account_pair` against a live local
  registry and a real browser: code shown, confirmed in Studio, library empty, avatar
  saved in the store, library re-read with one free avatar.

## Remaining risk
- Not run in a game window: the button, the two status lines and the new group were
  compile-checked only. The layout adds two rows above the grid; on a small window they
  may need spacing work.
- The token lives in memory, so the player connects again after every restart.
