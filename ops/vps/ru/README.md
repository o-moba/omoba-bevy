# RU market servers

One Russian VPS runs everything Russian players reach: the lobby and match
workers (UDP 4000, 41000–41001), the Account API (behind Caddy at
`account.ru.omoba.io`) and the player portal (`app.ru.omoba.io`). Keep it apart
from the global production host: each market has its own database, accounts and
device keys. Background and the release steps: [docs/markets.md](../../../docs/markets.md).

Only binaries from the RU Linux package (`python3 scripts/release.py build linux
--market ru`, `omoba-<version>-ru-linux-x64.tar.gz`) may run here: they are built
with `OMOBA_MARKET=ru` (no Ekza, no payments, VK ID sign-in).

## Install

1. User and folders: `useradd --system --home /var/lib/omoba-ru omoba-ru`;
   `/opt/omoba-ru/releases/<version>/` (unpacked package), `/opt/omoba-ru/current`
   → that release, `/var/lib/omoba-ru/{matches,lobby-outbox}` owned by `omoba-ru`,
   mode 700.
2. Database `omoba_ru` with an owner role for migrations and the unprivileged
   runtime roles `omoba_game` (lobby) and `omoba_portal` (Account API):
   ```sh
   OMOBA_DATABASE_URL=postgres://omoba_owner:…@127.0.0.1:5432/omoba_ru \
     /opt/omoba-ru/current/omoba-account-api migrate
   psql "postgres://omoba_owner:…@127.0.0.1:5432/omoba_ru" \
     -v game_role=omoba_game -v portal_role=omoba_portal -f account-api/ops/grants.sql
   ```
   Then disable the owner's login. Migrations never run at service start.
3. `/opt/omoba-ru/lobby.env` (root, 600): `OMOBA_DATABASE_URL` for `omoba_game`
   and `OMOBA_MATCH_PUBLIC_HOST=<this host's public IPv4>`.
4. `/opt/omoba-ru/account-api.env` from [account-api.env.example](account-api.env.example).
   Generate both secrets with `openssl rand -hex 32`; never rotate
   `OMOBA_SOCIAL_SECRET`.
5. Portal: build omoba-web with [portal.env.example](portal.env.example)
   (`npm ci && npm run build`, `node scripts/start.mjs`) as its own service on
   127.0.0.1:3010.
6. Caddy: [Caddyfile.ru](Caddyfile.ru) with the portal's ingress key. Only the
   game's native routes (including `/v1/auth/social/link`) are public; every
   other Account API route needs the portal key.
7. Units: copy `omoba-lobby-ru.service` and `omoba-account-api-ru.service` to
   `/etc/systemd/system/`, `systemctl daemon-reload`, `systemctl enable --now
   omoba-lobby-ru omoba-account-api-ru`. Open UDP 4000 and 41000–41001, TCP 443.

## Check

- `python3 scripts/release.py preflight --market ru` from any computer: lobby DNS,
  `https://account.ru.omoba.io/health/ready`, the portal.
- `journalctl -u omoba-lobby-ru -u omoba-account-api-ru`.
- A real RU client: play a bot match, then Profile → «Войти через VK ID».

Rollback: stop the units, point `current` at the previous release, start again.
Never drop the database or delete match directories; back up with `pg_dump -Fc`
before schema changes.
