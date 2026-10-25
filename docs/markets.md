# Markets: one build per region

OMOBA is built separately for each market. A market decides which services and
storefront rules a build has, which servers it talks to, the first-launch
language and how players sign in. It is a compile-time choice: a Russian build
cannot be switched to the Ekza ecosystem or to payments at run time, and a
typo in a market or feature name fails the build.

> **Planned (ADR-025, LR-68):** one app per platform for Russia and the
> international market. The account's region, confirmed on first launch, will
> choose servers, sign-in and payments at run time. The compile-time market then
> stays only for server binaries, CI and the RU-locked builds until LR-68 R3.
> This page describes what is built today.

| | `global` (default) | `ru` | `cn` |
| --- | --- | --- | --- |
| Who | Europe and the rest of the world | Russia | mainland China |
| Ekza ecosystem (`ekza`) | on | **off** | off |
| Paid cosmetics (`supporter`) | on | **off** (free game) | off |
| Sign in with VK ID (`vk_sign_in`) | off | **on** | off |
| First-launch language | English | Russian | Simplified Chinese |
| Lobby | `77.246.105.57:4000` | `lobby.omoba.ru:4000` | not decided |
| Account API / portal | none in production | `account.omoba.ru` / `omoba.ru` | not decided |
| Settings folder | `omoba-bevy` | `omoba-bevy-ru` | `omoba-bevy-cn` |
| Packages | `Omoba-<v>-*` | `Omoba-<v>-ru-*` in `dist/v<v>-ru/` | — |
| Downloads | IPFS links | `omoba.ru/downloads/` on the RU host, IPFS mirror, status «Открытая бета» | — |
| Android app | `space.ekza.omoba.beta` | `space.ekza.omoba.ru`, "OMOBA" | — |
| Platforms | macOS, Windows, Linux, Android, iOS | macOS, Windows, Linux, Android | none yet (blocked) |
| Legal documents to accept on first launch | none | `omoba.ru/terms`, `omoba.ru/privacy`, version `0-draft` | none |
| Age sign | none | `12+` (placeholder until the owner confirms it, LR-07) | none |

Players can still pick any language in Settings. The RU servers are planned names
under `omoba.ru` (registered at REG.RU); a build bakes them in and they answer once the RU servers
are deployed (`make release-ru-preflight` checks).

## Where a market lives

- `shared/src/market.rs` — `MARKET` and `FEATURES`, read by every crate (client,
  server, Account API). `OMOBA_MARKET` selects the market;
  `OMOBA_FEATURE_EKZA|SUPPORTER|VK_SIGN_IN=on|off` flips one feature of its preset
  for a test build. Both are `const`: unknown values are a compile error.
- `scripts/markets.py` — the same presets plus each market's servers, settings
  folder, Android identity, platforms, download path and release status label, for
  `scripts/release.py` and
  `mobile/android/build.py`. `scripts/test_release.py` checks it against the Rust
  module.
- `Market::legal` (Rust) and `Legal` (`markets.py`) — the legal documents, source
  link, support and community channels and age sign of a market (see
  [Legal documents and links](#legal-documents-and-links-lr-06)). The two are
  checked field for field by `test_release.py`.
- `release.py build … --market ru [--feature ekza=on] [--server H:P]
  [--account-api URL --portal URL]` bakes the market and its endpoints into every
  package. A build refuses another market's servers, the global public lobby in a
  market build, and `--registry/--passport` without the `ekza` feature.
- The portal (omoba-web) reads the same `OMOBA_MARKET` / `OMOBA_FEATURE_*`
  variables (`docs/markets.md` there).

The market is independent of the build **contour** (`docs/connection-profiles.md`):
`--market ru --contour dev` is the OMOBA Dev tester app with RU features, the dev
lobby and the `omoba-bevy-ru-dev` folder.

## What each feature switches

**`ekza` off** — the game contains only its packaged content and never contacts
an Ekza host. The client starts no Ekza store (registry catalogue, weapon store,
community placements, Ekza account restore), hides every Studio, Ekza library,
wallet and market element in Collection, hero select and draft, and does not
register the in-world community sculpture. Registry and passport endpoints are
closed loopback ports and desktop overrides are ignored. A saved Ekza weapon from
another build falls back to the class default. The server admits only shipped
avatars and weapons and never calls the registry or passport. The Account API
serves no Ekza library route. The portal has no market, sponsor or creator pages.

**`supporter` off** — nothing is sold. The client registers no Hero Supporter
panel or StoreKit bridge and shows no "Support OMOBA" button. The Account API
serves no `supporter` route and refuses to start with Apple or Solana settings.
The portal has no Supporter pages.

**`vk_sign_in` on** — sign in with VK ID (VK, Одноклассники and Mail.ru are one
VK ID app) on the portal and from the game, below.

## Legal documents and links (LR-06)

A market's legal table (`shared/src/market.rs` `Market::legal`, `scripts/markets.py`
`Legal`) holds:

| Field | RU value | Shown |
| --- | --- | --- |
| `version` | `0-draft` (the Russian drafts, LR-04) | first-launch card |
| `privacy`, `terms` | `https://omoba.ru/privacy`, `https://omoba.ru/terms` | card, Settings → About, package README |
| `source` | `https://github.com/o-moba/omoba-bevy/tree/v{version}-ru` | About, README (`{version}` is the release) |
| `support_email`, `community_vk`, `community_telegram` | empty until the owner provides them (LR-21) | About, README; hidden while empty |
| `age_rating` | `12+` (placeholder, LR-07) | card, boot splash, About, README |

The global and CN markets have none of these, so their game shows no card and no
About page, and their README is unchanged.

**First launch (game).** A market with a `version` shows a card before anything
else can be used: the documents open in the browser, the age sign and the
language switch are on it, and **Continue** stays disabled until the player ticks
«Я принимаю Пользовательское соглашение и Политику конфиденциальности». Until
then the game opens no lobby connection, does not read the release catalog and
offers no VK sign-in, device link or website pairing (one run condition,
`client/src/legal.rs` `bound_and_accepted`). The acceptance is stored in the
market's settings folder as `legal_acceptance.json`
(`{region, doc_version, accepted_at}`, Unix seconds); a new `version` asks again.
It is not sent to a server yet. Automation gets no exception: a harness for an RU
build writes that file into its own `OMOBA_CLIENT_CONFIG_DIR` first.

**Settings → About** lists the build label, the age sign, the documents, this
release's source and the channels that are set.

**Packages.** The README of an RU package ends with the age sign and the links
(`release.py legal_readme`). `release.py catalog --market ru` refuses to write the
download catalog until the public tag `v<version>-ru` exists on GitHub, so the
source link always resolves.

When the counsel-approved texts replace the drafts, give them a new `version` in
both tables (and the portal's): every player is asked again.

## Sign in with VK ID

The game's account is still its device key (`docs/supporter.md`, "One account on
phone and PC"); VK becomes a way to reach that account from the website and from
new devices. Nobody receives a VK token except the Account API, which exchanges
the authorization code itself and keeps no token, name or e-mail: a VK user is
stored only as an HMAC of its user id under `OMOBA_SOCIAL_SECRET`
(`account-api/migrations/007_social_sign_in.sql`).

**Registration on the website.** The player presses «Войти через VK ID» (or OK,
Mail.ru) on `omoba.ru`. The portal sends the VK callback to
`POST /v1/auth/social/vk`. An unknown VK account gets a 30-minute browser ticket,
not a game profile: the site asks the player to install OMOBA and press
«Войти через VK ID» in the game.

**Sign-in from the game.** Profile → «Войти через VK ID» creates a device request
(the same native enrollment as "Use an existing account") and opens
`<portal>/connect?code=<code>` in the browser; the game checks the request every
three seconds for five minutes.

- The VK account is new: after VK sign-in the portal offers the sign-in to the
  game's request (`POST /v1/auth/social/attach`). The game asks «Привязать вход
  через VK к этому игровому профилю?» and, on confirmation, signs
  `omoba.social.link.v1` with its active key (`POST /v1/auth/social/link`). From
  then on VK signs into that profile on the website and on other devices.
- The VK account is already linked: the browser is signed in as that account and
  approves the game's request (the existing device approval). The game shows the
  account and switches to it after a restart; the previous local profile is
  preserved, never merged. On the device that already plays as that account the
  game says so and changes nothing.

Guarantees: a website sign-in never creates or moves a game profile; a link needs
the game's own signature over its pending request; one VK account per profile;
a code is single-use and expires in five minutes; unlinking needs a fresh sign-in
(`DELETE /v1/me/social/vk`). Contract: `account-api/docs/openapi.json`; tests:
`account-api/tests/social_postgres.rs`.

**Protect progress (LR-18).** Progress follows the VK-linked account: after a
reinstall or on a new phone, Profile → «Войти через VK ID» with the same VK ID
brings back the same profile. After the first completed match, and again after the
first ranked match, Home asks to link VK ID or save a recovery code until the profile
is protected or the player declined three times; Profile shows the protection, and
«Защитить прогресс» creates recovery codes in the game
(`POST /v1/auth/devices/recovery-codes`, signed by the device key). «Ввести код
восстановления» sits next to the VK and device options. Support restores nothing by
hand: [account recovery](account-recovery.md).

## Adding a market

1. Add the variant and its preset, locale, storage suffix and legal table to
   `shared/src/market.rs`, and the same entry with servers, platforms, zones and
   `Legal` to `scripts/markets.py` (`cargo test -p shared market`, `make test-scripts`).
2. Make sure `client/i18n/<locale>/` exists.
3. If it needs new behaviour, add a feature to both files and gate it at one
   choke point (the market tests show the pattern).
4. Add its presets to the portal (`omoba-web` `src/lib/market.mjs`).
5. Remove `blocked` only when its servers exist and `release.py preflight
   --market <key>` passes.

## RU release runbook

Before following it, read [the launch-readiness plan](plans/launch-readiness.md):
its M0 specs are the launch blockers.

The steps that need accounts, money or legal decisions are the owner's.

1. **Hosting in Russia.** Personal data of Russian players (the VK-linked
   account, sessions, IP-based rate limits) must be stored in Russia (152-ФЗ):
   one Russian VPS with Ubuntu 24.04+ (2 vCPU / 4 GB is enough to start), root
   login with an SSH key. `ops/vps/ru/ru_host.py bootstrap` installs PostgreSQL 16+
   and Caddy on it. The Roskomnadzor notice as a personal-data operator and a
   Russian privacy policy / user agreement are the owner's.
2. **DNS.** A records for the RU lobby, Account API and portal names of the RU
   profile in `scripts/markets.py` → the RU VPS (`ops/vps/ru/ru_host.py names`
   prints them; `ru_host.py status` checks them).
3. **VK ID app.** At id.vk.ru create a *Web* app, trusted redirect URL
   `https://omoba.ru/api/auth/vk/callback`, scope `vkid.personal_info`,
   enable sign-in with OK and Mail.ru. Its numeric app id is
   `OMOBA_VKID_CLIENT_ID` for both the portal and the Account API. No secret is
   used (public PKCE client).
4. **Packages.** `make release-ru` builds `dist/v<version>-ru/`: macOS, Windows,
   Linux (with the RU `omoba-server` and `omoba-account-api`) and Android
   (`space.ekza.omoba.ru`). `make release-ru-publish` uploads them to the RU host
   (`omoba.ru/downloads/`), pins the IPFS mirror and writes `releases.json`
   (`ops/vps/ru/README.md#downloads`).
5. **Servers** with `ops/vps/ru/ru_host.py` (`ops/vps/ru/README.md`): `bootstrap`
   sets up the host (PostgreSQL with ICU, Caddy with `Caddyfile.ru`, firewall,
   key-only SSH, users, env files with secrets made on the host, the database and
   the three units); `ship --package <RU Linux package>` migrates the database with
   the RU `omoba-account-api migrate`, applies `account-api/ops/grants.sql` and
   starts the lobby and the Account API; `portal-bundle` + `ship-portal` install the
   RU portal (omoba-web with `OMOBA_MARKET=ru`, calling the Account API on loopback:
   `ops/vps/ru/README.md#topology`). Backups: the owner provides storage in Russia,
   makes the backup key and names the escrow holder; `bootstrap` installs the nightly
   backup, the monthly restore drill and their check, and `ship` backs up before every
   migration (`ops/vps/ru/README.md#backups-escrow-and-restore-drill`).
6. **Check.** `ru_host.py status` (the whole host, read-only) and, on a new host,
   `ru_host.py reboot-check --apply`; `make release-ru-preflight` (DNS,
   `/health/ready`, portal, and the portal ↔ Account API path: `/api/me` 401 and
   `/api/releases` 200 through the portal, a public `/v1/me` refused with 403), then
   one real round: install the RU package, play a bot match, Profile →
   «Войти через VK ID», link, sign in on the portal, sign in on a second device.
   Every release then runs the [RU release checklist](../ops/vps/ru/release-checklist.md)
   on every platform and stores its result in `docs/progress/`.
7. **Distribution.** Publish the GitHub release `v<version>-ru` first (`release.py
   draft --market ru`, then `gh release edit v<version>-ru --draft=false`): the game's
   About page and the README link its source, and `catalog` refuses until the tag
   exists (LR-06). Then install `releases.json` as the RU Account API's
   `OMOBA_RELEASE_CATALOG`: the portal's download page and the game's update button
   then link the RU host, with IPFS as the mirror and the status «Открытая бета».
   `make release-ru-verify-downloads` checks every listed package byte for byte; a
   test download from a Russian network and `ru_host.py downloads` prove the counts
   (LR-23). RuStore / VK Play later (store accounts and age rating are the owner's).

iOS is not packaged for RU: it needs its own App Store Connect record.
