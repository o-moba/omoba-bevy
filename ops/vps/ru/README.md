# RU market servers

One Russian VPS runs everything Russian players reach: PostgreSQL, the lobby and
match workers (UDP 4000, 41000–41001), the Account API (behind Caddy at
`account.omoba.ru`), the player portal (`omoba.ru`) and the game downloads
(`omoba.ru/downloads/`, [Downloads](#downloads)). Keep it apart
from the global production host: each market has its own database, accounts and
device keys. Background and the release steps: [docs/markets.md](../../../docs/markets.md).

Only binaries from the RU server bundle may run here ([Server bundle](#server-bundle)):
they are built with `OMOBA_MARKET=ru` (no Ekza, Hero Supporter through YooKassa only, VK
ID sign-in; the YooKassa shop settings and webhook arrive with LR-67b).

> **Status (LR-01, LR-03, LR-23, LR-17 ops).** Everything up to the host is ready: [ru_host.py](ru_host.py)
> sets up a fresh Ubuntu host, ships the server package and the portal, checks the
> whole host and runs the reboot test; [ru_backup.py](ru_backup.py) backs it up every
> night, encrypted and off-host, before every migration, and drills a restore every
> month ([Backups](#backups-escrow-and-restore-drill)); the host serves the game
> packages and counts their downloads ([Downloads](#downloads)); every unit failure,
> low disk, a stopped backup timer and a stuck match worker alert the owner, the journal
> is bounded, and an external probe watches the public names every minute
> ([Monitoring and alerts](#monitoring-and-alerts)). Still open before a public launch:
> the lobby's own metrics and their alerts (LR-17, server part) and the capacity
> measurement (LR-09). The owner buys the VPS, sets DNS, creates the VK ID app, provides
> the backup storage in Russia, makes the backup key, names the escrow holder, creates
> the Telegram bot and picks the probe host.
> See [the launch-readiness plan](../../../docs/plans/launch-readiness.md) and the
> [RU release checklist](release-checklist.md).

## Topology

Everything below runs on this one host; only Caddy listens publicly for HTTP.

| Caller | Reaches | How | Client address the Account API sees |
| --- | --- | --- | --- |
| Game (native signed routes) | `https://account.omoba.ru` → Caddy → `127.0.0.1:40550` | `@native` allowlist and health only; every other route answers 403 | `X-Real-IP`, overwritten by Caddy with the game's address |
| Browser | `https://omoba.ru` → Caddy → portal `127.0.0.1:3010` | the whole portal | — (Caddy overwrites `X-Real-IP` with the browser's address) |
| Browser, game update button | `https://omoba.ru/downloads/<tag>/<package>` → Caddy `file_server` on `/opt/omoba-ru/downloads` | the packages only, no listing ([Downloads](#downloads)) | — (static files; Caddy logs the request, shortened) |
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
| PostgreSQL 16+ (Ubuntu package, ICU `und-x-icu`), database `omoba_ru`, `127.0.0.1:5432` only | cluster `/var/lib/postgresql` | `postgresql@<version>-main.service` (`postgresql.service` is postgresql-common's wrapper, a oneshot that stays active) |
| Lobby + match workers, user `omoba-ru` | `/opt/omoba-ru/current/omoba-server`, state in `/var/lib/omoba-ru` | [omoba-lobby-ru.service](omoba-lobby-ru.service) |
| Account API, user `omoba-ru`, `127.0.0.1:40550` | `/opt/omoba-ru/current/omoba-account-api` | [omoba-account-api-ru.service](omoba-account-api-ru.service) |
| Portal, user `omoba-portal-ru`, `127.0.0.1:3010` | `/opt/omoba-ru/portal/current/server.js` on `/opt/omoba-ru/node/current/bin/node` | [omoba-portal-ru.service](omoba-portal-ru.service) |
| Caddy, TCP 443 | `/etc/caddy/Caddyfile` = [Caddyfile.ru](Caddyfile.ru) | `caddy.service` |
| Per-source UDP limits (nftables table `inet omoba_limits`) | `/opt/omoba-ru/nftables-omoba.conf` = [nftables-omoba.conf](nftables-omoba.conf) | [omoba-limits-ru.service](omoba-limits-ru.service) (oneshot) |
| Downloads (LR-23): packages, root 0755/0644, and Caddy's log of them | `/opt/omoba-ru/downloads/<tag>/`; `/var/log/caddy/omoba-ru-downloads.log` (caddy, 0640) | `release.py publish --market ru` (rsync); read by `caddy.service` |
| Env files (root, 0600) | `/opt/omoba-ru/{lobby,account-api,portal}.env` from the `*.env.example` files here | read by systemd |
| Backups (LR-03), root | [ru_backup.py](ru_backup.py) as `/opt/omoba-ru/backup/ru_backup.py`; settings `/opt/omoba-ru/backup.env` (root 0600, from [backup.env.example](backup.env.example)), keys in `/opt/omoba-ru/backup-keys/` (0700); host copies in `/var/backups/omoba-ru/` (0700) | [omoba-backup-ru.timer](omoba-backup-ru.timer) nightly, [omoba-restore-drill-ru.timer](omoba-restore-drill-ru.timer) monthly, [omoba-backup-check-ru.timer](omoba-backup-check-ru.timer) hourly |
| Monitoring and alerts (LR-17), root | [ru_monitor.py](ru_monitor.py) as `/opt/omoba-ru/monitor/ru_monitor.py`; settings `/opt/omoba-ru/alerts.env` (root 0600, from [alerts.env.example](alerts.env.example)); state in `/var/lib/omoba-ru-monitor/`; journal limits `/etc/systemd/journald.conf.d/50-omoba-ru.conf` ([journald-omoba-ru.conf](journald-omoba-ru.conf)); the PostgreSQL alert drop-in `/etc/systemd/system/postgresql@.service.d/50-omoba-ru-alert.conf` ([postgresql-alert-ru.conf](postgresql-alert-ru.conf)) | [omoba-alert-ru@.service](omoba-alert-ru@.service) on any unit's failure (`OnFailure=`), [omoba-monitor-ru.timer](omoba-monitor-ru.timer) every 5 minutes |

All three OMOBA units restart on any exit every 5 s without a start limit
(`Restart=always`, `StartLimitIntervalSec=0`), and each failure alerts the owner
([Monitoring and alerts](#monitoring-and-alerts)); the lobby and the Account API start
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
   - Packages from Ubuntu's own mirrors only (`postgresql caddy ufw nftables openssh-server gnupg`);
     no third-party apt repository. PostgreSQL must be 16+ with the `und-x-icu`
     collation and listen on loopback only, or bootstrap stops before writing secrets.
   - Node.js ≥ 20.9 for the portal (Next.js 16): the uploaded official tarball
     (sha256 checked) in `/opt/omoba-ru/node/v<x.y.z>`, or Ubuntu's `nodejs` when it is
     new enough (24.04 ships 18.x, so pass the tarball there).
   - Users `omoba-ru` and `omoba-portal-ru` (system, nologin); folders
     `/opt/omoba-ru/{releases,portal/releases,node,incoming,backup,backup-keys}`,
     `/var/lib/omoba-ru/{matches,lobby-outbox}` and `/var/backups/omoba-ru` (0700 but `backup`).
   - Env files from the examples: database passwords, `OMOBA_PORTAL_SECRET` and
     `OMOBA_SOCIAL_SECRET` are generated on the host; `OMOBA_MATCH_PUBLIC_HOST` is
     `$RU_IP`; `--vkid-client-id` fills both VK ID lines (without it they stay
     placeholders and `ship` refuses). Rerunning only fills placeholders.
   - Roles `omoba_owner` (NOLOGIN), `omoba_game`, `omoba_portal` and database
     `omoba_ru` (UTF8, collation C.UTF-8; handles use `und-x-icu` explicitly).
   - [Caddyfile.ru](Caddyfile.ru) after `caddy validate`; the old file is kept as
     `Caddyfile.orig`. Before it, the download folder `/opt/omoba-ru/downloads` and
     Caddy's download log `/var/log/caddy/omoba-ru-downloads.log`, owned by `caddy`
     (created before `caddy validate`, which runs as root, could create it root-owned).
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
   - The backup kit: `ru_backup.py`, its three services and timers (the timers enabled
     and started), and `/opt/omoba-ru/backup.env` from `backup.env.example`, written
     once with placeholders and never replaced. Until the owner fills it, the nightly
     backup fails and alerts, and `status` lists what is missing.
   - The monitoring kit (LR-17), installed before the units that name it: `ru_monitor.py`,
     the alert template `omoba-alert-ru@.service`, the host check and its timer (enabled
     and started), the drop-in that gives every PostgreSQL cluster unit the alert,
     `/opt/omoba-ru/alerts.env` from `alerts.env.example` (written once, never replaced)
     and the journal limits, after which `systemd-journald` restarts.
2. **Lobby and Account API** from the RU server bundle ([Server bundle](#server-bundle)):
   ```sh
   python3 scripts/release.py build server --market ru
   python3 ops/vps/ru/ru_host.py ship --package dist/v<v>-ru/server/omoba-<v>-ru-server-linux-x64.tar.gz --apply
   ```
   Refused while a match runs. It stages `/opt/omoba-ru/releases/<v>-ru-<sha7>/`
   (immutable, sha256-checked); when a schema exists it runs the pre-migration backup
   (`ru_backup.py backup --reason pre-migration`: an encrypted set off-host and a plain
   rollback dump in `/var/backups/omoba-ru/`) and refuses to migrate while backups are
   not configured or the backup fails (before launch, without storage,
   `--local-backup-only` takes only the rollback dump); then it runs
   `omoba-account-api migrate` under a temporary, expiring
   owner login, applies `account-api/ops/grants.sql`, switches `current`, installs
   changed units, enables and restarts both services and checks UDP 4000, the
   lobby's executable and `/health/ready`. Any failure rolls back to the previous
   release. Shipping the running release again is a no-op.
3. **Portal** (see [Portal bundle](#portal-bundle)):
   ```sh
   python3 ops/vps/ru/ru_host.py portal-bundle --web ../omoba-web
   python3 ops/vps/ru/ru_host.py ship-portal --bundle dist/portal-ru/omoba-portal-ru-<rev7>.tar.gz --apply
   ```
4. **Backups and escrow**: [set them up once](#set-up-once-owner).
5. **Check** with `python3 ops/vps/ru/ru_host.py status` (read-only, exit 0 only
   when nothing is missing) and once per new host `reboot-check --apply`.

Each change is appended to `/opt/omoba-ru/deployments.jsonl`.

## Server bundle

The RU lobby and Account API ship as their own bundle, never inside a player package
(LR-22): the RU Linux player package (`omoba-<v>-ru-linux-x64.tar.gz`) holds the game,
its practice server and the Russian README, and no `omoba-account-api`.

```sh
python3 scripts/release.py build server --market ru   # also part of `build all --market ru` / make release-ru
# -> dist/v<v>-ru/server/omoba-<v>-ru-server-linux-x64.tar.gz (+ SHA256SUMS.txt there)
```

| Inside `omoba-<v>-ru-server/` | |
| --- | --- |
| `omoba-server` | the lobby and its match workers ([omoba-lobby-ru.service](omoba-lobby-ru.service)) |
| `omoba-account-api` | the Account API and `omoba-account-api migrate` ([omoba-account-api-ru.service](omoba-account-api-ru.service)) |
| `legal/`, `ATTRIBUTION.md`, `README.txt` | licence notices and an operator note |

Both are built in one cargo invocation with the RU market's environment (the same
values as the RU clients), on Linux or in the pinned `rust:<toolchain>-bookworm`
container. The bundle sits in `server/` beside the player packages: `release.py draft`
attaches and `publish` offers only the player packages. `ru_host.py ship` accepts only
this file name (`RU_PACKAGE`, x86-64 ELF binaries); a player package or another
market's file is refused before anything is contacted.

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
  UDP 4000, `127.0.0.1:40550/health/ready`, the portal on `127.0.0.1:3010`, the
  download folder and Caddy's download log (owned by `caddy`; `facts.downloads` lists
  the published versions), DNS (A
  records for the three names of `ru_host.py names` point here; no AAAA for the
  lobby), memory and disk, and the backups: the installed kit and timers and
  `ru_backup.py check --remote` (a set off-host from the last 26 h, no failed run or
  drill, a successful drill in the last 35 days, a current escrow, complete settings),
  and the monitoring: the installed kit, its timer, `alerts.env`, the journal limits and
  `ru_monitor.py check` (free disk, match workers).
- `python3 scripts/release.py preflight --market ru` from any computer: lobby DNS, the
  lobby's UDP compatibility check (with `--contract dist/v<v>-ru/compatibility.json`,
  compatible with that release), `https://account.omoba.ru/health/ready`, the portal,
  and the topology: through
  the portal `GET /api/me` must be the Account API's 401 and `GET /api/releases` 200
  (a 403 there means a portal route was refused, LR-02), and
  `https://account.omoba.ru/v1/me` must be 403 (portal routes stay off the public
  name).
- `journalctl -u omoba-lobby-ru -u omoba-account-api-ru -u omoba-portal-ru -u caddy`.
- Releases: the [RU release checklist](release-checklist.md), with its result in
  `docs/progress/` for every release.

Downloads on the portal: see [Downloads](#downloads).

## Downloads

The RU packages (Windows, Linux, macOS, Android) are served from this host
(LR-23, O-10); the Pinata IPFS copies are the mirror. The download path, the portal
origin and the release status come from the RU profile in
[scripts/markets.py](../../../scripts/markets.py) (`downloads_path`, `portal`,
`release_label` «Открытая бета»): no fourth DNS name, the portal's Caddy site serves
`/downloads/`.

| Piece | Where |
| --- | --- |
| Packages | `/opt/omoba-ru/downloads/v<version>/<package>`, root, folders 0755, files 0644; served at `https://omoba.ru/downloads/v<version>/<package>` without a listing; rsync's in-progress dot files are hidden |
| Catalog | `dist/v<version>-ru/releases.json` → `/opt/omoba-ru/releases.json`, `OMOBA_RELEASE_CATALOG` in `account-api.env`; the portal's download page, the game's **Update available** banner and its **Update required** button read it (`GET /v1/releases`, LR-14) |
| Download log | `/var/log/caddy/omoba-ru-downloads.log` (caddy 0640): JSON, only `/downloads/` requests, address shortened to /24 (IPv4) or /48 (IPv6), no request headers (no cookies, no user agent); rolled at 50 MiB, kept 90 days |

Publish a release (on the Mac, after `make release-ru`):

```sh
make release-ru-publish   # RU_DOWNLOADS_TARGET defaults to ${RU_HOST:-vds-ru}:/opt/omoba-ru/downloads
# = release.py publish --market ru --rsync-target …  then  release.py catalog --market ru
scp dist/v<version>-ru/releases.json $RU_HOST:/opt/omoba-ru/releases.json
# once: OMOBA_RELEASE_CATALOG=/opt/omoba-ru/releases.json in /opt/omoba-ru/account-api.env,
# then systemctl restart omoba-account-api-ru
make release-ru-verify-downloads   # release.py verify-downloads --market ru
```

- `publish --market ru` refuses a target on the production or dev VPS (also through
  the ssh alias's HostName) and needs `PINATA_JWT`. It copies the new packages with
  `rsync --ignore-existing` into `<target>/v<version>/` (a published file is never
  replaced), then downloads each one from `https://omoba.ru/downloads/…` and compares
  SHA-256 and size; `dist/v<version>-ru/downloads.json` records the verified links.
  Then it pins the IPFS mirror as before (`ipfs.json`). `--dry-run` prints the rsync
  command and the URLs and contacts nothing.
- `catalog --market ru` refuses until the public GitHub tag `v<version>-ru` exists (the
  game's About page and the package README link its source, LR-06). It writes
  `releases.json` from `downloads.json`: `url` on the RU
  host, `mirror_url` the IPFS copy with the same SHA-256, `test_status` «Открытая бета»,
  `region` `ru`, `channel` `direct`, `update_url` the portal's download page for the
  platform (account-api/README.md#downloads). `--min-release X` adds `min_release`:
  games older than X must update even without a server switch (a forced update: post
  the player notice first).
- `verify-downloads --market ru` reads the catalog the portal serves
  (`https://omoba.ru/api/releases`, or `--catalog <file>` before installing it),
  downloads every package and its mirror, and checks the host, region, channel,
  status, SHA-256 and size. It exits 0 only when every RU platform is listed and correct.
- A package rebuilt under a published name fails the read-back: publish a new version,
  or remove the stale file on the host by hand first.

For a release that changes the compatibility contract, install the new catalog and
post the player notice **before** shipping the server
([RELEASING.md](../../../docs/RELEASING.md#updates-and-contract-changes-lr-14)).

**Counting downloads.** `python3 ops/vps/ru/ru_host.py downloads [--since YYYY-MM-DD]
[--until YYYY-MM-DD]` pipes [ru_downloads.py](ru_downloads.py) to the host (the same
target guards as `status`) and prints totals per platform, per version and per UTC day;
no address leaves the host. A download is a GET of a package answered 200 or 206,
counted once per shortened address, file and day (a resumed download counts once;
players sharing a carrier NAT or a /24 on the same day count once, so the numbers are
a floor); `completed` counts answers that sent the whole file at once, `requests` every
GET. The log keeps 90 days: record the totals of a period in `docs/progress/` before
they roll off. The shortened addresses in the log stay on this host in Russia; the
privacy policy (LR-04, owner) must name this log and its 90 days.

**A test download from a Russian network** (once per new host, and in the release
checklist): from a Russian connection (a phone on a Russian carrier without VPN, or a
computer in Russia), download one package from the portal's download page, compare its
SHA-256 with the page, then run `ru_host.py downloads --since <today>` and record the
count for that platform with the date, network and platform in `docs/progress/`.

## Monitoring and alerts

LR-17. The owner learns of an outage within minutes without logging in. On the host,
[ru_monitor.py](ru_monitor.py) reports what the host itself can see; outside it, the
[external probe](../../probe/README.md) reports what players would see, including the
whole host being gone. Both send to the RU host's one Telegram bot; without the bot,
alerts stay in the journal.

| Alert | Source | When | Repeats while it lasts |
| --- | --- | --- | --- |
| A unit failed: lobby, Account API, portal, backup, restore drill, backup check, host check, per-source limits (LR-16), a PostgreSQL cluster | `OnFailure=omoba-alert-ru@%n.service` in every RU unit and, by [postgresql-alert-ru.conf](postgresql-alert-ru.conf), in every `postgresql@<version>-<name>.service` → `ru_monitor.py unit-failed` | at once, also on each crash before `Restart=` brings a service back (systemd 254+; Ubuntu 24.04 has 255) | hourly for a service; daily for a timer job, whose tool sends the details itself |
| An enabled RU service or Caddy is not running | `ru_monitor.py check --alert` ([omoba-monitor-ru.timer](omoba-monitor-ru.timer), every 5 minutes) | at the next check | hourly |
| A PostgreSQL cluster that starts at boot (`/etc/postgresql/<version>/<name>/`, as `pg_lsclusters` lists it) is not running, or port 5432 accepts no connection (`pg_isready`, no login). `postgresql.service` is only postgresql-common's wrapper, active whatever its clusters do, so the check watches `postgresql@<version>-<name>.service` | the same check; a crash also alerts at once through the drop-in, under the same alert | at the next check | hourly |
| A backup, drill or backup-check timer is disabled or stopped: no backup is made, or a missing one is no longer reported | the same check | at the next check | hourly |
| Free disk below 15% on a filesystem the host writes to (`/`, PostgreSQL, `/var/lib/omoba-ru`, `/var/backups/omoba-ru`, `/var/log`, `/opt/omoba-ru`) | the same check | at the next check | hourly |
| A live match worker has written no heartbeat (`<match root>/<allocation>/status.json`) for 5 minutes while the lobby has run that long (the lobby relaunches a stale worker after 100 s) | the same check | at the next check | hourly |
| No backup for 26 h, a failed backup or drill, no drill for 35 days, a stale escrow | `ru_backup.py check --alert`, hourly (LR-03) | at once | every 6 h |
| The lobby's UDP compatibility check, `/health/ready` or the portal fails from outside | the external probe, every minute | after 2 failed rounds in a row | hourly |

Each alert says once when its problem is gone: "resolved" after 15 minutes without it,
or "answers again" from the probe. Messages carry unit names, paths, numbers and
allocation ids, never a log line, an address or a secret. Not watched yet, because they
need the lobby's own metrics (LR-17, server part): the queue's capacity wait, failed
allocations and the database connection state.

**Set up once (owner).**

1. Create a bot with @BotFather and a chat (or channel) for the alerts, and add the bot
   to it. Read the chat id, for example from
   `https://api.telegram.org/bot<token>/getUpdates` after a message in the chat.
2. On the host, uncomment and fill `OMOBA_TELEGRAM_BOT_TOKEN` and `OMOBA_TELEGRAM_CHAT_ID`
   in `/opt/omoba-ru/alerts.env`. Then `python3 /opt/omoba-ru/monitor/ru_monitor.py
   test-alert` must print `sent`, and the message must arrive. The backup alerts use
   the same bot unless `backup.env` names its own. The token is not in the escrow: a
   lost one is replaced in @BotFather.
3. Install the [external probe](../../probe/README.md) on a machine outside this host,
   with the same bot.

**Knobs** in `alerts.env`: `OMOBA_ALERT_DISK_MIN_FREE_PERCENT` (15),
`OMOBA_ALERT_WORKER_STALE_SECONDS` (300), `OMOBA_ALERT_REPEAT_MINUTES` (60),
`OMOBA_ALERT_JOB_REPEAT_HOURS` (24) and `OMOBA_ALERT_RECOVERY_MINUTES` (15).
`ru_monitor.py check` prints the host's state now, and
`journalctl -u 'omoba-alert-ru@*' -u omoba-monitor-ru` shows what was alerted.

**Journal limits.** [journald-omoba-ru.conf](journald-omoba-ru.conf) keeps the journal on
disk (`Storage=persistent`), at most 1 GiB (`SystemMaxUse=1G`) and 30 days
(`MaxRetentionSec=30day`, one file a day so whole days age out). Some lobby and Account
API lines carry a player's IP address, so the privacy policy (LR-04) states these 30
days, next to the 90 days of the download log. The match archive and `worker.log` are
bounded by the lobby itself (LR-17, server part).

## Backups, escrow and restore drill

[ru_backup.py](ru_backup.py) runs on the host as root; `ru_host.py bootstrap` installs
it with its units. It reads only `/opt/omoba-ru/backup.env`
([backup.env.example](backup.env.example)) and never prints a secret.

| What | Where | When | Kept |
| --- | --- | --- | --- |
| Backup set `omoba-ru-<UTC>-<reason>.tar.gpg`: `database.dump` (`pg_dump -Fc` of `omoba_ru`, career and portal), `state.tar.gz` (`/var/lib/omoba-ru/matches` with `archive/` and every allocation `outbox/`, and `/var/lib/omoba-ru/lobby-outbox`), `manifest.json` (rows and a digest per table, both schema version lists, sha256s); encrypted with GnuPG to the backup public key and signed with its secret key | **off-host:** the storage in Russia named by `OMOBA_BACKUP_TARGET` (`s3://` S3-compatible object storage or `sftp://` an SFTP account; the owner provides it); on the host: `/var/backups/omoba-ru/sets/` | nightly 00:30 UTC (03:30 Moscow), at the next boot after a missed night; before every migration (`ship`) | off-host: the newest set of each of the last 14 days with a set and of each of the last 8 weeks; host: the newest 2 |
| Rollback dump (plain `pg_dump -Fc`) | `/var/backups/omoba-ru/omoba_ru-before-<release>-<UTC>.dump` (root 0600) | before every migration | the newest 3 |
| Restore drill: the newest off-host set restored into `omoba_ru_restore_drill`, compared with its manifest (tables, row counts, the rows' digests, schema versions, the state archive), then dropped | the same PostgreSQL | the 2nd of every month, 02:30 UTC | result in `/var/backups/omoba-ru/state.json` |
| Secrets escrow: `account-api.env` (`OMOBA_PORTAL_SECRET`, `OMOBA_SOCIAL_SECRET`), `lobby.env`, `portal.env`, `backup.env`, `host.json`, `backup-keys/`, encrypted to the escrow holder's public key | **offline with the escrow holder**; never on the backup storage | after bootstrap and after any change to those files | the holder keeps every version |

`fetch` and the drill refuse a set the backup key did not sign: a set planted on the
storage (by anyone with its login or the public key) never reaches `pg_restore`, which
runs as the database superuser. A set that cannot be uploaded stays on the host and goes
up with the next run. A failed
backup or drill alerts at once; the hourly check alerts when no backup succeeded for
26 h, when no drill succeeded for 35 days, and when the escrow is missing or stale.
Alerts go to the journal (`journalctl -u omoba-backup-ru -u omoba-restore-drill-ru -u
omoba-backup-check-ru`) and to Telegram through the host's bot in `alerts.env` (or a bot
of its own in `backup.env`): one message per problem at most every 6 h, and one when
everything passes again. A failed backup unit also alerts like any RU unit, at most
daily ([Monitoring and alerts](#monitoring-and-alerts)).

**Escrow holder:** not named yet (owner checklist, LR-03). Record here who holds the
backup secret key and the secrets escrow, how to reach them and where the files are
kept; date the latest escrow below.

| Escrow holder | Reach | Kept where | Backup key fingerprint | Latest escrow |
| --- | --- | --- | --- | --- |
| — | — | — | — | — |

### Set up once (owner)

1. **Storage in Russia** (152-FZ): an S3-compatible bucket (a key limited to that
   bucket) or an SFTP account on another server in Russia. Not the RU VPS itself. The
   host prunes old sets, so its key may delete; where the provider offers bucket
   versioning or object lock, keep deleted objects for 30 days, so a compromised host
   cannot erase the history.
2. **Backup key** on the Mac: `python3 ops/vps/ru/ru_backup.py keygen --out <folder>`
   writes `backup-public.asc` and `backup-secret.asc`. The escrow holder keeps
   `backup-secret.asc` offline: without it no backup can be read.
3. **Keys on the host** (the host signs every set with the secret key, and the drill
   decrypts with it):
   ```sh
   scp <folder>/backup-public.asc <folder>/backup-secret.asc $RU_HOST:/opt/omoba-ru/backup-keys/
   ssh $RU_HOST chmod 600 /opt/omoba-ru/backup-keys/backup-secret.asc
   ```
   SFTP only: the login key as `backup-keys/ssh_key` (0600) and the server's host key
   as `backup-keys/known_hosts` (`ssh-keyscan -p <port> <host>`, fingerprint compared
   with the provider's).
4. **Settings:** `ssh $RU_HOST` and edit `/opt/omoba-ru/backup.env` (the storage, its
   key or login, optionally the Telegram bot); then
   `python3 /opt/omoba-ru/backup/ru_backup.py config` must print `ok`.
5. **First backup and drill** on the host: `systemctl start omoba-backup-ru` and
   `systemctl start omoba-restore-drill-ru` (each waits for the other's lock), then
   `python3 /opt/omoba-ru/backup/ru_backup.py list` and `… check --remote`.
6. **Escrow** from the Mac, with the holder's public key; plaintext never leaves the host:
   ```sh
   python3 ops/vps/ru/ru_host.py escrow --recipient-file holder-public.asc \
       --out omoba-ru-secrets-$(date +%F).asc
   ```
   The holder checks it (`gpg --decrypt <file> | tar -tzv`) and stores it offline;
   delete the local copy. Repeat after any change to those files (`status` says when).
   The Android release keystore (LR-24) goes the same way, on the Mac:
   `python3 ops/vps/ru/ru_backup.py escrow --recipient-file holder-public.asc --file
   <keystore> > android-release-key.asc`.
7. Fill in the escrow table above and run `python3 ops/vps/ru/ru_host.py status`.

### Restore

Stop writers first: `systemctl stop omoba-lobby-ru omoba-account-api-ru omoba-portal-ru`.
Never restore over the running database as a test; the monthly drill does that safely.

- **Undo a migration on this host:** restore the rollback dump of that ship (next section).
- **From an off-host set on this host** (database damaged or lost):
  ```sh
  python3 /opt/omoba-ru/backup/ru_backup.py fetch --set latest --out /var/backups/omoba-ru/restore
  runuser -u postgres -- pg_restore --clean --if-exists --exit-on-error -d omoba_ru \
      < /var/backups/omoba-ru/restore/database.dump
  ```
  `fetch` keeps nothing unless the set's signature and checksums are right; `--set
  <name>` picks an older set (`ru_backup.py list`). To bring back the match and
  outbox state as well: `tar -xzf /var/backups/omoba-ru/restore/state.tar.gz -C /` and
  `chown -R omoba-ru:omoba-ru /var/lib/omoba-ru`. Matches settled after the set was
  taken are lost (their outbox files were already acknowledged). Start the services,
  then delete the `restore` folder.
- **Onto a new host** (the VPS is lost): a new VPS and DNS as in LR-01. From the escrow,
  put `account-api.env`, `portal.env`, `lobby.env` (with `OMOBA_MATCH_PUBLIC_HOST` set to
  the new address), `backup.env` and `backup-keys/` into `/opt/omoba-ru/` (root, 0600;
  folders 0700) before `ru_host.py bootstrap`, which then keeps those secrets and
  passwords; leave out `host.json`. Run `bootstrap --apply`, then on the host
  `ru_backup.py fetch --set latest --out /var/backups/omoba-ru/restore` and
  `runuser -u postgres -- pg_restore --exit-on-error -d omoba_ru < …/database.dump` into
  the empty database, restore the state as above, then `ship` (it re-applies the grants)
  and `ship-portal`. Check before announcing it: ratings and match history on the
  portal, sign-in with an existing recovery code, and VK sign-in of an existing linked
  account (it works only with the escrowed `OMOBA_SOCIAL_SECRET`).

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
schema it understands. To undo a migration, restore the rollback dump taken by that
ship, `/var/backups/omoba-ru/omoba_ru-before-<release>-<UTC>.dump` (the newest three
are kept; older ones are in the off-host sets), with the lobby and the Account API
stopped: `runuser -u postgres -- pg_restore --clean --if-exists -d omoba_ru < <dump>`.
Never drop the database or delete match directories.
