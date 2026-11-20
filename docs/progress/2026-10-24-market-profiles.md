# Markets and sign in with VK ID

Goal: build OMOBA separately for the Russian, Chinese and global (European)
markets from one codebase, and prepare the Russian market as a free game without
the Ekza ecosystem, with registration and sign-in through VK ID on the website
and from the game. Branch `feat/market-profiles` (omoba-bevy and omoba-web).

## Changes

- `shared::market`: markets `global`/`ru`/`cn` as presets of `ekza`,
  `supporter`, `vk_sign_in`; `OMOBA_MARKET` and `OMOBA_FEATURE_*` are read at
  compile time and fail the build on an unknown value.
- Account API, portal schema 7: `social.rs` and
  `migrations/007_social_sign_in.sql` (keyed identity hashes, browser tickets,
  links to pending enrollments); `POST /v1/auth/social/{vk,ticket,attach,link}`,
  `DELETE /v1/me/social/vk`, `social` in `GET /v1/me`; enrollment status
  `social_pending`. Free-market builds serve no Supporter or Ekza library route
  and refuse billing settings; a build without `vk_sign_in` refuses VK settings.
  OpenAPI and README updated; `ops/grants.sql` grants the three tables.
- Client: Profile and the device panel offer «Войти через VK ID» where the
  market has it (`career_devices.rs`): the device request opens
  `<portal>/connect?code=…`, is checked every three seconds, then links this
  profile (signed by the active key, `CareerIdentity::sign_social_link`) or
  switches after restart. `omoba_passport::device_account` gained
  `link_social` and `sign_in_url`.
- Market gates in client and server: `docs/progress/2026-10-23-market-gating.md`.
- Release: `scripts/markets.py`, `release.py --market/--feature/--account-api/
  --portal`, `markets`/`preflight`/`catalog` commands, Android `--market`
  (`space.ekza.omoba.ru`), `make release-ru*`, `ops/vps/ru/`.
- Docs: `docs/markets.md` (architecture, VK ID flows, RU runbook).
- omoba-web (same branch name): `OMOBA_MARKET`/`OMOBA_FEATURE_*` with the same
  presets, VK ID start/callback routes, `/connect`, registration view, Ekza and
  Supporter pages and BFF routes off by feature, `tests/vk-sign-in.e2e.mjs`.

## Checks

- Account API on a disposable PostgreSQL 18 (loopback 55581): unit 15; Postgres
  suites class builds 4, devices 7, Ekza library 1, core 6, social 5 (website
  registration and game link, new device, provider and callback checks, HTTP
  routes, restricted portal role), supporter 4, recognition 3 — pass.
- Workspace, global and `OMOBA_MARKET=ru`: check, clippy `-D warnings`, client
  1418, server 343, shared 147, passport 50, harness — pass.
- Release scripts: 49 unit tests (Python 3.13) — pass.
- Portal: `npm test` 170, typecheck, global and RU builds, market e2e, and
  `npm run test:vk-sign-in`: the RU portal build, the Account API built for RU,
  PostgreSQL and Chrome, with VK ID replaced by a loopback stand-in — website
  registration, the game's signed link, VK sign-in through OK and a second device
  — pass.

## Not verified / remaining

- Real VK ID: no VK ID app exists yet (owner: create it, docs/markets.md step 3).
- RU servers, DNS, hosting in Russia, packages for RU, legal documents: owner
  steps in the runbook; `release.py preflight --market ru` reports them.
- Game screenshots of the RU build: the screen was locked during the session, so
  the native captures rendered black; behaviour is covered by tests only.
- iOS RU: not packaged (needs its own App Store Connect record).
