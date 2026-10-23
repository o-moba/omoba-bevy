# Compile-time market gates

Goal: an `OMOBA_MARKET=ru` (or `cn`) build is a plain free game. It contacts no
Ekza host, shows no Ekza, Studio, wallet or market UI, sells nothing, and keeps
the shipped heroes, avatars, weapons and gameplay. The global build behaves as
before. Builds on `shared::market` (`MARKET`, `FEATURES`). Task artifacts:
`.agent/tasks/MARKET-GATING/`.

## Gates (one feature each, pure functions tested for both markets)

`ekza` off:

- Client startup: `passport::initialize_store` returns `None` (no avatar or
  weapon store, account link, sponsorship placements, no `ekza://` source and
  no account poll in `lib.rs`); `passport::initialize` skips
  `OMOBA_PASSPORT_CONNECT` pairing; `passport::connect` and
  `ekza_account::connect_account` never start a flow.
- `connection_profile::for_market`: every profile runs with `CLOSED_REGISTRY`
  (`http://127.0.0.1:9`) and `CLOSED_PASSPORT`, no market and no Studio;
  `install` ignores `OMOBA_REGISTRY_URL`/`OMOBA_PASSPORT_URL` (logged). The dev
  connection panel lists only the lobby (`mobile_ui::detail_rows`).
- `plugins::presentation_plugins` disables `SponsorshipPlugin`.
- Collection: `CollectionFilter::tabs` (no Studio tab), `avatar_sections` (no
  Studio section, shared with the draft), no wallet button, no weapon refresh,
  `HandheldGroup::listed` (shipped choices only, no library/catalogue/market
  hint). Draft: no "My Ekza library" row, no EKZA STUDIO pass. Hero select:
  `team::ekza_rows_hidden` extends the offline predicate.
- `persistence::restorable_handheld`: a saved non-packaged item becomes the
  class default.
- Server: `PassportAdmissions::for_market(false)` has no passport API and
  fetchers that fail at once, so protected avatars and `ekza-` weapons are
  denied with no request; shipped content stays free.

`supporter` off:

- `plugins::ui_plugins` disables `SupporterPlugin` and `SupporterStoreKitPlugin`;
  `QaPlugins` disables the two Supporter QA harnesses, which drive the first
  plugin's resources.
- `career::account_links` drops "Support OMOBA"; `Action::Supporter` is a no-op.
  `career_web::challenge_key` shows a website-approval text without Supporter
  management (`web.challenge_free`, en/ru/zh-Hans).

Market-wide:

- `i18n::plugin::market_locale`: first-launch language is the market's;
  `OMOBA_LANGUAGE` and the saved choice still win.
- `platform::config_directory_name`: `omoba-bevy` + `-<market>` (non-global) +
  `-<contour>`: `omoba-bevy`, `omoba-bevy-dev`, `omoba-bevy-ru`,
  `omoba-bevy-ru-dev`.
- `build_info::with_market`: the Home/boot build line ends in ` · ru` for a
  non-global market; `main` logs `Market: <describe>`.

## Not changed

- The iOS StoreKit Swift bridge is still compiled into the shared iOS bridge
  library (`client/build.rs`); with Supporter off it is never called.
- The custom-profile registry field of the dev connection panel stays (its
  value is replaced by the closed registry when Ekza is off).
- Release tooling (`scripts/`), VK ID sign-in UI.
