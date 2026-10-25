# RU market servers

One Russian VPS runs everything Russian players reach: PostgreSQL, the lobby and
match workers (UDP 4000, 41000–41001), the Account API (behind Caddy at
`account.omoba.ru`) and the player portal (`omoba.ru`). Keep it apart
from the global production host: each market has its own database, accounts and
device keys. Background and the release steps: [docs/markets.md](../../../docs/markets.md).

Only binaries from the RU Linux package (`python3 scripts/release.py build linux
--market ru`, `omoba-<version>-ru-linux-x64.tar.gz`) may run here: they are built
with `OMOBA_MARKET=ru` (no Ekza, no payments, VK ID sign-in).

> **Status (LR-01).** Everything up to the host is ready: [ru_host.py](ru_host.py)
> sets up a fresh Ubuntu host, ships the server package and the portal, checks the
> whole host and runs the reboot test. Still open before a public launch: automated
> off-host backups (LR-03), monitoring and alerts (LR-17) and the capacity
> measurement (LR-09). The owner buys the VPS, sets DNS and creates the VK ID app.
> See [the launch-readiness plan](../../../docs/plans/launch-readiness.md) and the
> [RU release checklist](release-checklist.md).

## Topology

Everything below runs on this one host; only Caddy listens publicly for HTTP.

| Caller | Reaches | How | Client address the Account API sees |
| --- | --- | --- | --- |
| Game (native signed routes) | `https://account.omoba.ru` → Caddy → `127.0.0.1:40550` | `@native` allowlist and health only; every other route answers 403 | `X-Real-IP`, overwritten by Caddy with the game's address |
| Browser | `https://omoba.ru` → Caddy → portal `127.0.0.1:3010` | the whole portal | — (Caddy overwrites `X-Real-IP` with the browser's address) |
| Portal (BFF, VK callback, server reads) | `http://127.0.0.1:40550` directly | loopback HTTP, no portal ingress key | `X-Real-IP`, the browser's address forwarded by the portal |

- `OMOBA_TRUST_LOOPBACK_PROXY=1` is set on the Account API and the portal. The
  API trusts `X-Real-IP` only from loopback and refuses a loopback request without
  it; the portal trusts it only because Caddy overwrites it and its own listener is
  loopback-only. Per-IP limits (social sign-in 30/min, pairing creation 5/min) are
  therefore per player, never per host.
- There is no portal ingress key on RU: the portal never goes through
  `account.omoba.ru`, and that name never passes a portal route.
- The portal refuses to serve (journal: `OMOBA portal misconfigured, refusing to
  serve`) with an Account API that is not on loopback and no valid
  `OMOBA_ACCOUNT_INGRESS_KEY`, or with the loopback API and no
  `OMOBA_TRUST_LOOPBACK_PROXY=1`.
- Never put another proxy in front of Caddy without changing `{remote_host}`
  (for example to `{client_ip}` with `trusted_proxies`): the limits would key on
  that proxy.

## What runs where

| Piece | Where | Started by |
| --- | --- | --- |
| PostgreSQL 16+ (Ubuntu package, ICU `und-x-icu`), database `omoba_ru`, `127.0.0.1:5432` only | cluster `/var/lib/postgresql` | `postgresql.service` |
| Lobby + match workers, user `omoba-ru` | `/opt/omoba-ru/current/omoba-server`, state in `/var/lib/omoba-ru` | [omoba-lobby-ru.service](omoba-lobby-ru.service) |
| Account API, user `omoba-ru`, `127.0.0.1:40550` | `/opt/omoba-ru/current/omoba-account-api` | [omoba-account-api-ru.service](omoba-account-api-ru.service) |
| Portal, user `omoba-portal-ru`, `127.0.0.1:3010` | `/opt/omoba-ru/portal/current/server.js` on `/opt/omoba-ru/node/current/bin/node` | [omoba-portal-ru.service](omoba-portal-ru.service) |
| Caddy, TCP 443 | `/etc/caddy/Caddyfile` = [Caddyfile.ru](Caddyfile.ru) | `caddy.service` |
| Per-source UDP limits (nftables table `inet omoba_limits`) | `/opt/omoba-ru/nftables-omoba.conf` = [nftables-omoba.conf](nftables-omoba.conf) | [omoba-limits-ru.service](omoba-limits-ru.service) (oneshot) |
| Env files (root, 0600) | `/opt/omoba-ru/{lobby,account-api,portal}.env` from the `*.env.example` files here | read by systemd |

All three OMOBA units restart on any exit every 5 s without a start limit
(`Restart=always`, `StartLimitIntervalSec=0`); the lobby and the Account API start
after `postgresql.service` and pull it in. A reboot brings everything back with no
manual step; `ru_host.py reboot-check --apply` proves it.

## Install and update: ru_host.py

[ru_host.py](ru_host.py) does every step below. It runs on the Mac and pipes itself to
`ssh <alias> python3 -`, so the host needs nothing but Ubuntu 24.04+ (x86-64) and
root login with a key. Without `--check` or `--apply` it contacts nothing; `--check`
is a read-only preflight on the host; `--apply` changes it. It refuses the
production and dev VPS (aliases, addresses, units, users, folders) on both sides, a
host whose `ssh -G` HostName is not `$RU_IP`, a login user other than root, and —
for bootstrap — an address that `whois` does not place in Russia
(`--country-confirmed` when the provider says it is). No secret is printed or put
in argv; a generated secret is never replaced.

```sh
# ~/.ssh/config:  Host vds-ru / HostName <RU IPv4> / User root / IdentityFile …
export RU_HOST=vds-ru RU_IP=<RU VPS IPv4>
python3 ops/vps/ru/ru_host.py names   # the RU names and the DNS A records to set; local only
```

The public names (lobby, Account API, portal) and the RU DNS zone come from the RU
profile in [scripts/markets.py](../../../scripts/markets.py) and are written nowhere
else: the tool reads them on the Mac and sends them to the host, where `status` and
`bootstrap` check DNS against them. It refuses a kit whose `Caddyfile.ru` sites or
`OMOBA_PORTAL_ORIGIN` lines do not match them, so a rename edits `markets.py` and then
the kit files it names.

1. **Bootstrap a fresh host** (idempotent; run it again after changing the kit):
   ```sh
   python3 ops/vps/ru/ru_host.py bootstrap --vkid-client-id <VK ID app id> \
       --node-tarball ~/Downloads/node-v22.<x>.<y>-linux-x64.tar.xz \
       --node-sha256 <its line in nodejs.org/dist/v22.<x>.<y>/SHASUMS256.txt>
   python3 ops/vps/ru/ru_host.py bootstrap … --check   # then --apply
   ```
   - Packages from Ubuntu's own mirrors only (`postgresql caddy ufw nftables openssh-server`);
     no third-party apt repository. PostgreSQL must be 16+ with the `und-x-icu`
     collation and listen on loopback only, or bootstrap stops before writing secrets.
   - Node.js ≥ 20.9 for the portal (Next.js 16): the uploaded official tarball
     (sha256 checked) in `/opt/omoba-ru/node/v<x.y.z>`, or Ubuntu's `nodejs` when it is
     new enough (24.04 ships 18.x, so pass the tarball there).
   - Users `omoba-ru` and `omoba-portal-ru` (system, nologin); folders
     `/opt/omoba-ru/{releases,portal/releases,node,incoming}`, `/var/lib/omoba-ru/{matches,lobby-outbox}` (0700).
   - Env files from the examples: database passwords, `OMOBA_PORTAL_SECRET` and
     `OMOBA_SOCIAL_SECRET` are generated on the host; `OMOBA_MATCH_PUBLIC_HOST` is
     `$RU_IP`; `--vkid-client-id` fills both VK ID lines (without it they stay
     placeholders and `ship` refuses). Rerunning only fills placeholders.
   - Roles `omoba_owner` (NOLOGIN), `omoba_game`, `omoba_portal` and database
     `omoba_ru` (UTF8, collation C.UTF-8; handles use `und-x-icu` explicitly).
   - [Caddyfile.ru](Caddyfile.ru) after `caddy validate`; the old file is kept as
     `Caddyfile.orig`.
   - Key-only SSH: `/etc/ssh/sshd_config.d/00-omoba-ru.conf` (sorts before
     cloud-init's file), checked with `sshd -t` and `sshd -T`. Refused while
     `/root/.ssh/authorized_keys` is empty.
   - Firewall (ufw): incoming denied except SSH, TCP 443, UDP 4000 and the arena
     range read from the lobby unit (`OMOBA_MATCH_FIRST_PORT` + `OMOBA_MATCH_CAPACITY`).
     Port 80 stays closed: Caddy falls back to the TLS-ALPN challenge on 443 for its
     certificates. If the provider has its own firewall, open the same ports there.
   - Per-source UDP limits ([Admission limits](#admission-limits-lr-16)):
     [nftables-omoba.conf](nftables-omoba.conf) as `/opt/omoba-ru/nftables-omoba.conf`
     after `nft -c` accepts it, and [omoba-limits-ru.service](omoba-limits-ru.service),
     enabled and started. A refused file leaves the loaded limits as they were.
   - The three unit files; `postgresql` and `caddy` enabled; an OMOBA unit is
     enabled once its release exists, so a boot never starts a missing binary.
2. **Lobby and Account API** from the RU Linux package:
   ```sh
   python3 scripts/release.py build linux --market ru
   python3 ops/vps/ru/ru_host.py ship --package dist/v<v>-ru/omoba-<v>-ru-linux-x64.tar.gz --apply
   ```
   Refused while a match runs. It stages `/opt/omoba-ru/releases/<v>-ru-<sha7>/`
   (immutable, sha256-checked), takes `pg_dump -Fc` into `/var/backups/omoba-ru/` when
   a schema exists, runs `omoba-account-api migrate` under a temporary, expiring
   owner login, applies `account-api/ops/grants.sql`, switches `current`, installs
   changed units, enables and restarts both services and checks UDP 4000, the
   lobby's executable and `/health/ready`. Any failure rolls back to the previous
   release. Shipping the running release again is a no-op.
3. **Portal** (see [Portal bundle](#portal-bundle)):
   ```sh
   python3 ops/vps/ru/ru_host.py portal-bundle --web ../omoba-web
   python3 ops/vps/ru/ru_host.py ship-portal --bundle dist/portal-ru/omoba-portal-ru-<rev7>.tar.gz --apply
   ```
4. **Check** with `python3 ops/vps/ru/ru_host.py status` (read-only, exit 0 only
   when nothing is missing) and once per new host `reboot-check --apply`.

Each change is appended to `/opt/omoba-ru/deployments.jsonl`.

## Portal bundle

The portal is omoba-web's `output: standalone` build, made with the RU environment
on the Mac (or in a `linux/amd64` container when a native addon matters):

```sh
cd ../omoba-web && git status --short   # clean checkout of the release commit
npm ci
OMOBA_MARKET=ru OMOBA_VKID_CLIENT_ID=<VK ID app id> npm run build
cd - && python3 ops/vps/ru/ru_host.py portal-bundle --web ../omoba-web
```

`portal-bundle` packs `.next/standalone` with `.next/static` and `public/`, without
`.next/cache` or any `.env*` file (Next.js copies them into the standalone folder),
owned by root and world-readable, as `dist/portal-ru/omoba-portal-ru-<rev7>.tar.gz`.
Both it and `ship-portal` refuse a build whose `server.js` does not say
`OMOBA_BUILT_MARKET` ru (with Ekza off) and `OMOBA_BUILT_CONTOUR` production.
Native addons built for macOS are reported (sharp falls back to its wasm build).
On the host the release's `.next/cache` links to `/var/cache/omoba-portal-ru`, the
unit's only writable folder.

## Check

- `python3 ops/vps/ru/ru_host.py status`: marker, Ubuntu, packages, PostgreSQL
  (version, ICU, loopback, roles, schema), Caddy and its Caddyfile, firewall, the
  per-source limits (file, unit, table `inet omoba_limits` loaded), sshd,
  users and folder modes, env files, Node.js, releases, units enabled and running,
  UDP 4000, `127.0.0.1:40550/health/ready`, the portal on `127.0.0.1:3010`, DNS (A
  records for the three names of `ru_host.py names` point here; no AAAA for the
  lobby), memory and disk.
- `python3 scripts/release.py preflight --market ru` from any computer: lobby DNS,
  `https://account.omoba.ru/health/ready`, the portal, and the topology: through
  the portal `GET /api/me` must be the Account API's 401 and `GET /api/releases` 200
  (a 403 there means a portal route was refused, LR-02), and
  `https://account.omoba.ru/v1/me` must be 403 (portal routes stay off the public
  name).
- `journalctl -u omoba-lobby-ru -u omoba-account-api-ru -u omoba-portal-ru -u caddy`.
- Releases: the [RU release checklist](release-checklist.md), with its result in
  `docs/progress/` for every release.

Downloads on the portal: `make release-ru-publish` also writes
`dist/v<version>-ru/releases.json` from the verified IPFS receipts; copy it to
`/opt/omoba-ru/releases.json` and set `OMOBA_RELEASE_CATALOG` to it in
`/opt/omoba-ru/account-api.env`.

## Admission limits (LR-16)

Two layers keep one source (an address, or a household or carrier NAT behind
one address) from crowding everyone else out. Both count **per source
address**: many players behind one address share one budget, so the defaults
leave room for about eight players on one address.

**In the kernel** ([nftables-omoba.conf](nftables-omoba.conf), its own table
`inet omoba_limits`, loaded by [omoba-limits-ru.service](omoba-limits-ru.service);
it never flushes ufw's rules). Over-rate datagrams are dropped before the game
reads them:

| Port | Rate per source address | Burst |
| --- | --- | --- |
| UDP 4000 (lobby) | 400 datagrams/s | 800 |
| UDP arena range (41000–41001) | 1200 datagrams/s | 2400 |

- To change a rate, edit the rule in `nftables-omoba.conf` and run `ru_host.py
  bootstrap --apply` (it runs `nft -c` first, then restarts the unit).
- The file's `OMOBA_LOBBY_PORT` and `OMOBA_ARENA_PORTS` must match the lobby unit
  (`OMOBA_MATCH_FIRST_PORT` … `+ OMOBA_MATCH_CAPACITY - 1`): `ru_host.py` refuses a
  kit where they differ, so a capacity change edits both files.
- On the host: `nft list table inet omoba_limits` shows the rules with their drop
  counters; `systemctl stop omoba-limits-ru` removes the table (emergency only;
  `status` then reports it).

**In the game** (lobby and match workers). Defaults are the production values; to
change one, add the line to `/opt/omoba-ru/lobby.env` (the workers inherit it) and
restart the lobby. A malformed value stops the lobby with a message naming it. The
lobby logs the values it runs with (`Admission limits: …`).

| Setting | Default | What it limits |
| --- | --- | --- |
| `OMOBA_LIMIT_PROBES_PER_IP` | 8 | Transport probes per second from one IP, and, counted separately, compatibility replies |
| `OMOBA_LIMIT_PROBES_PER_NETWORK` | 32 | The same per IPv4 /24 (IPv6 /48) |
| `OMOBA_LIMIT_PROBES_GLOBAL` | 64 | The same for everyone together |
| `OMOBA_LIMIT_PROBES_NEW_RESERVE` | 16 | Part of the global budget only a source's first datagram of the second may use (0 = none) |
| `OMOBA_LIMIT_ENDPOINTS_PER_IP` | 8 | Return-path endpoints, and career clients, one IP holds at once. An unproven endpoint expires after 5 s and a proven one after 30 s without traffic; an idle career client gives way to a newcomer of its IP after 30 s |
| `OMOBA_LIMIT_NEW_PROFILES_PER_IP` | 10 | Profiles new device keys may create from one IP in 24 hours; existing keys always sign in |
| `OMOBA_LIMIT_NEW_PROFILES_TOTAL` | 5000 | The same for everyone together (then «new profiles are paused») |
| `OMOBA_LIMIT_ARENAS_PER_IP` | 1 | Running or forming arenas one IP holds alone (every human of the match came from it) |
| `OMOBA_LIMIT_PRACTICE_ABANDONS` | 3 | vs Bots allocations from one IP left unplayed within 10 minutes before a cooldown |
| `OMOBA_LIMIT_PRACTICE_COOLDOWN_SECS` | 120 | That cooldown |
| `OMOBA_PROFILE_RETENTION_DAYS` | 30 | The lobby deletes, hourly and 200 at a time, profiles whose only key is older than this, that never played and that nothing else holds (a friend, a portal link, a sign-in, a Supporter grant); 0 = off, otherwise 7–3650 |

- One IP is an IPv4 address or an IPv6 /64. The new-profile counter is durable
  (PostgreSQL, career schema 4): it stores a salted hash of the address, never the
  address, and deletes rows after 48 hours.
- Refusals reach the player in their language: the new-profile limit at sign-in,
  `arena_limit` and `practice_cooldown` on the searching screen.
- The journal shows `PROFILE_RETENTION pruned=N …` when the retention job deleted
  profiles.

## Rollback

`python3 ops/vps/ru/ru_host.py rollback --kind server --release <v>-ru-<sha7> --apply`
(or `--kind portal --release <rev7>-<sha8>`) points `current` at a release that is
still staged and restarts; it runs no migration, so an older server only works on a
schema it understands. To undo a migration, restore the `pg_dump` taken by that
ship from `/var/backups/omoba-ru/` (`runuser -u postgres -- pg_restore --clean
--if-exists -d omoba_ru <dump>`) with the lobby and the Account API stopped. Never drop the database or delete match directories.
