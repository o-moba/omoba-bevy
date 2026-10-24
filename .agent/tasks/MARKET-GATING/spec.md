# MARKET-GATING: compile-time market gates in the client and server

Frozen 2026-10-23 before implementation. Builds on `shared::market`
(`MARKET`, `FEATURES`, commit 428ccac).

## Problem

A free-market build (`OMOBA_MARKET=ru`/`cn`: `ekza` off, `supporter` off) still
starts the Ekza stores, pins Ekza hosts, shows Studio/wallet/market UI and the
Supporter purchase, and its server would ask the Ekza registry/passport.

## Acceptance criteria

- AC1: With `FEATURES.ekza` off the client opens no Ekza store, account link,
  sponsorship poll or `ekza://` source, skips `OMOBA_PASSPORT_CONNECT` pairing,
  and pins a closed loopback registry and passport (env overrides ignored); the
  in-world sculpture plugin is not registered.
- AC2: With `ekza` off Collection, hero select and the draft show no Studio
  tab/section, Ekza account/wallet controls, Ekza library/catalogue weapon
  groups or market hint; shipped avatars, weapons and heroes are unchanged.
- AC3: With `supporter` off neither Supporter plugin is registered, Profile shows
  no "Support OMOBA" button and its action is a no-op.
- AC4: A saved non-packaged handheld falls back to the class default when
  `ekza` is off; the saved avatar is validated as before.
- AC5: The server's passport admissions use no passport API and fetchers that
  fail at once when `ekza` is off: protected/`ekza-` items are denied, shipped
  ones free. Unit-tested.
- AC6: First-launch UI language is the market's (`OMOBA_LANGUAGE` and the saved
  choice still win); the settings folder is
  `omoba-bevy[-<market>][-<contour>]`.
- AC7: The global build behaves as before; every gate is a pure function of
  `Features`/a bool with unit tests for both markets.
- AC8: Global and RU `cargo check`/`clippy -D warnings` and the touched tests
  pass; one RU desktop capture each of Collection, hero select/draft and Profile
  (Russian UI) shows no Ekza/Studio/wallet/market/Supporter UI.

Out of scope: release tooling (`scripts/`), VK ID sign-in UI, iOS StoreKit
bridge compilation.
