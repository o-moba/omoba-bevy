# RU market servers

One Russian VPS runs everything Russian players reach: the lobby and match
workers (UDP 4000, 41000–41001), the Account API (behind Caddy at
`account.ru.omoba.io`) and the player portal (`app.ru.omoba.io`). Keep it apart
from the global production host: each market has its own database, accounts and
device keys. Background and the release steps: [docs/markets.md](../../../docs/markets.md).

Only binaries from the RU Linux package (`python3 scripts/release.py build linux
--market ru`, `omoba-<version>-ru-linux-x64.tar.gz`) may run here: they are built
with `OMOBA_MARKET=ru` (no Ekza, no payments, VK ID sign-in).

> **Not deployable as is.** The launch-readiness audit found blockers in this kit;
> fix them first: there is no portal unit, backup job or monitoring yet (LR-01,
> LR-03, LR-17); two arenas per host is unmeasured (LR-09). See
> [the launch-readiness plan](../../../docs/plans/launch-readiness.md).

## Topology

Everything below runs on this one host; only Caddy listens publicly for HTTP.

| Caller | Reaches | How | Client address the Account API sees |
| --- | --- | --- | --- |
| Game (native signed routes) | `https://account.ru.omoba.io` → Caddy → `127.0.0.1:40550` | `@native` allowlist and health only; every other route answers 403 | `X-Real-IP`, overwritten by Caddy with the game's address |
| Browser | `https://app.ru.omoba.io` → Caddy → portal `127.0.0.1:3010` | the whole portal | — (Caddy overwrites `X-Real-IP` with the browser's address) |
| Portal (BFF, VK callback, server reads) | `http://127.0.0.1:40550` directly | loopback HTTP, no portal ingress key | `X-Real-IP`, the browser's address forwarded by the portal |

- `OMOBA_TRUST_LOOPBACK_PROXY=1` is set on the Account API and the portal. The
  API trusts `X-Real-IP` only from loopback and refuses a loopback request without
  it; the portal trusts it only because Caddy overwrites it and its own listener is
  loopback-only. Per-IP limits (social sign-in 30/min, pairing creation 5/min) are
  therefore per player, never per host.
- There is no portal ingress key on RU: the portal never goes through
  `account.ru.omoba.io`, and that name never passes a portal route.
- The portal refuses to serve (journal: `OMOBA portal misconfigured, refusing to
  serve`) with an Account API that is not on loopback and no valid
  `OMOBA_ACCOUNT_INGRESS_KEY`, or with the loopback API and no
  `OMOBA_TRUST_LOOPBACK_PROXY=1`.
- Never put another proxy in front of Caddy without changing `{remote_host}`
  (for example to `{client_ip}` with `trusted_proxies`): the limits would key on
  that proxy.

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
   127.0.0.1:3010. It calls the Account API on `http://127.0.0.1:40550`
   ([Topology](#topology)).
6. Caddy: [Caddyfile.ru](Caddyfile.ru) as is. Only the game's native routes
   (including `/v1/auth/social/link`) and health are public on
   `account.ru.omoba.io`; `app.ru.omoba.io` proxies the portal and sets
   `X-Real-IP`.
7. Units: copy `omoba-lobby-ru.service` and `omoba-account-api-ru.service` to
   `/etc/systemd/system/`, `systemctl daemon-reload`, `systemctl enable --now
   omoba-lobby-ru omoba-account-api-ru`. Open UDP 4000 and 41000–41001, TCP 443.

## Check

- `python3 scripts/release.py preflight --market ru` from any computer: lobby DNS,
  `https://account.ru.omoba.io/health/ready`, the portal, and the topology: through
  the portal `GET /api/me` must be the Account API's 401 and `GET /api/releases` 200
  (a 403 there means a portal route was refused, LR-02), and
  `https://account.ru.omoba.io/v1/me` must be 403 (portal routes stay off the public
  name).
- `journalctl -u omoba-lobby-ru -u omoba-account-api-ru`.
- A real RU client: play a bot match, then Profile → «Войти через VK ID».

Downloads on the portal: `make release-ru-publish` also writes
`dist/v<version>-ru/releases.json` from the verified IPFS receipts; copy it to
`/opt/omoba-ru/releases.json` and set `OMOBA_RELEASE_CATALOG` to it.

Rollback: stop the units, point `current` at the previous release, start again.
Never drop the database or delete match directories; back up with `pg_dump -Fc`
before schema changes.
