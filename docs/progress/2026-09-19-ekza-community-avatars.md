# Ekza community avatars — 2026-09-19

Goal: a creator's avatar, approved for Omoba in Ekza Studio, is wearable by anyone in a
match without a Solana wallet. This closes the loop "published, approved, played".

## Design
- The slug stays the SDK hash of identity and rendition (`ekza-<sha256>`), so a free
  avatar pins exact bytes exactly like a purchased one. Only the proof differs: a
  purchased avatar needs a consumed passport ticket, a free one needs the registry to
  list it as `access: "free"` for project `omoba`.
- Authority is the server's own registry read (`omoba_passport::community::fetch_free`),
  kept in `FreeCatalogue` inside the admission gate. `AvatarDefinition::free` on the
  client is a presentation hint only.
- The read happens on a worker thread and completes through the existing
  `CompletedAdmission` channel, so the game tick never blocks.

## Changes
- `shared`: `AvatarDefinition::free`.
- `passport`: `community` module; the client store carries the SDK's `free` flag.
- `server`: `FreeCatalogue`, `register_free`, ticketless branch in `PassportAdmissions::begin`.
- `client`: "Ekza community avatars" group, `can_select` and `ticket_for_slug` treat a
  free avatar as needing no wallet.
- SDK pinned to `b66a6da` (0.5.0).

## Checks
- `cargo test -p server -p shared -p omoba-passport`: server 176 passed, shared 59,
  passport 8 (+12 ignored live tests). New admission tests: free admitted with no
  ticket (first sight Pending, then Free, also with padded whitespace); an owned template
  in the same feed refused; an unlisted well-formed slug refused; retry and TTL windows;
  outage keeps known avatars; forged slug, non-free item and another game's approval are
  never registered. One existing test adapted: an unlisted ticketless store slug is now
  refused after one registry read instead of instantly, and again instantly within the
  retry window.
- `cargo check -p client`: clean.
- Live, against a local Ekza registry holding one Studio avatar approved for Omoba
  (real VRM, real builder): `cargo test -p omoba-passport --test community_e2e -- --ignored`
  passed. It reads the free list, installs through the client store with hash
  verification and runs `verify_local`, the game's own Rust humanoid profile check, on the
  bytes the registry's builder produced.
- Live, real server binary on a side port with `OMOBA_REGISTRY_URL` set, JSON joins over
  UDP: the approved free slug joined with no ticket; `ekza-aaaa…` was rejected with
  `avatar_not_authorized`; the shipped `anna` joined as before. The slug computed
  independently (SHA-256 of `id + "\n" + rendition sha256`) equals the SDK's.

## Remaining risk
- Not checked in a window: the picker group and the model on screen (needs an interactive
  client run). The load path is the one purchased avatars already use.
- Revocation is bounded by the five minute TTL, not instant.
- A production server reads `https://registry.ekza.io` unless `OMOBA_REGISTRY_URL` is set;
  that registry does not serve `/v2/avatars` until it is redeployed. Until then the read
  fails and only purchased and shipped avatars are admitted, as before.
