# OMOBA DEV lobby and dev tester builds

The DEV contour runs on a **dedicated dev VPS** (Ubuntu 24.04 or 26.04 amd64, about 2 vCPU / 4 GB; `vds-dev` runs 26.04),
never on the production VPS. Its ssh alias is `vds-dev` (root, key auth). Its address is
known only from `DEV_IP`, and every tool reads it from the environment.

| Piece | DEV | Production (never a target here) |
| --- | --- | --- |
| Host | `vds-dev`, `$DEV_IP` | `vds-eternal`, 77.246.105.57 |
| Unit | `omoba-lobby-dev` ([../omoba-lobby-dev.service](../omoba-lobby-dev.service)) | `omoba-lobby` |
| User, paths | `omoba-dev`, `/opt/omoba-dev`, `/var/lib/omoba-dev` | `omoba`, `/opt/omoba`, `/var/lib/omoba` |
| UDP | lobby 4100, match worker 41100 (capacity 1) | 4000, 41000–41001 |
| Registry | `https://registry.dev.ekza.io` | `https://registry.ekza.io` |
| Career database | `omoba_dev` (roles `omoba_dev_owner`/`_game`/`_portal`) in the dev Supabase Postgres on the same host (`supabase_db_ekza-dev`, 127.0.0.1:54322) | `omoba` in PostgreSQL 16 on the production VPS |
| Database URL | `/opt/omoba-dev/database.env` (root, 0600), generated on the host | `/opt/omoba/database.env` |
| Ekza passport | `http://127.0.0.1:9/api/passport`: a closed port, so wallet pairing and purchased-avatar tickets fail closed | `https://avatar.ekza.io/api/passport` |
| Client address | `lobby.dev.omoba.io:4100` | 77.246.105.57:4000 |
| Client settings folder | `omoba-bevy-dev` | `omoba-bevy` |

The dev lobby has no web UI, so there is nothing to mark noindex. Dev clients say
`OMOBA DEV` in the window title, show a `DEV` badge beside the Home title and under the
match score strip, point their market hints at `app.dev.omoba.io/market`, and are named
`Omoba Dev.app`. Only synthetic accounts and
guests use it. Nobody copies production data to it.

## Why the dev lobby needs a database

A public server role (`OMOBA_SERVER_ROLE=lobby`, and the match workers it starts) exits at
startup with `Public roles require OMOBA_DATABASE_URL` (`server/src/runtime/mod.rs`). Even
without that check, a lobby without storage could not match anybody: the career store then
ignores every account request (`career_backend.rs` `handle`), and the lobby only queues
signed account profiles (`match_service.rs` `enqueue`). So "guests only" is not a lobby
mode, and an opt-out flag would start a lobby that never forms a match.

The dev lobby therefore gets its own database in the Postgres the dev Supabase stack already
runs on the dev VPS. It is a separate database (`omoba_dev`) with its own roles, created the
way production's is: `omoba-account-api migrate` as the owner, then
`account-api/ops/grants.sql`, then the owner loses its login. `lobby_dev.py` does all of it
on the host, and only for a URL that names `omoba_dev_game@127.0.0.1:54322/omoba_dev`. It
refuses hosted Supabase (`*.supabase.co`, the poolers, the production project ref), the
production VPS, and production database and role names.

## Why a closed passport

The SDK's default passport, `avatar.ekza.io`, is production. A dev lobby that admits a
purchased avatar sends a state-changing ticket consume to the passport, and a dev client
pairs wallets with it. No dev passport exists yet, so the dev unit sets
`OMOBA_PASSPORT_URL=http://127.0.0.1:9/api/passport` (match workers inherit it). Dev builds
bake the same value as `OMOBA_DEFAULT_PASSPORT_URL`, and the dev Linux server is compiled
with it too. Free registry avatars and the shipped roster still work. Once a dev passport
exists, set its URL in `release.py` (`DEV_PASSPORT`, or `--passport https://<name>.dev.ekza.io/api/passport`),
in `lobby_dev.py` (`PASSPORT`) and in the unit, all in one change.

## Why a host name and not an IP

The client accepts `host:port` with a DNS name. `persistence::validate_game_server_addr`
validates DNS labels, and `net/transport.rs` `connect_udp_server` resolves the name with
`ToSocketAddrs` on the network thread. The match endpoint goes through the same path,
because the lobby hands out `OMOBA_MATCH_PUBLIC_HOST:port` as a free-form host
(`server/src/match_pool.rs`). `make compatibility-check` resolves names too. So
dev builds can bake `lobby.dev.omoba.io:4100` before the VPS exists, and a new VPS
address needs only a DNS change, not new tester builds.

The record must be **A only, no AAAA**. The unit binds IPv4 (`0.0.0.0`), and a UDP
"connect" succeeds without proving reachability, so a client would pick an IPv6
answer first and then time out. `lobby_dev.py` checks this from the VPS's own resolver.
It does not use the Mac's resolver, because a local proxy's fake-IP DNS can lie.

Without DNS, everything also works by IP:
`release.py build … --contour dev --server <DEV_IP>:4100` and
`lobby_dev.py ship … --public-host <DEV_IP>`.

## Owner inputs (once)

1. The rented VPS and its IPv4 → `DEV_IP`.
2. `~/.ssh/config` on the Mac. `HostName` must be the IPv4 literal: the tool refuses an alias that does not connect to exactly `$DEV_IP`.
   ```
   Host vds-dev
     HostName <DEV_IP>
     User root
     IdentityFile ~/.ssh/<dev key>
   ```
3. DNS at Name.com, zone `omoba.io`: `A lobby.dev → <DEV_IP>`, TTL 300, and no AAAA.
4. Firewall: allow UDP 4100 and 41100 in the provider panel. If the host bootstrap enables ufw, also run `ufw allow 4100/udp` and `ufw allow 41100/udp`. The tool reports missing ufw rules but never changes the firewall.
5. The dev Supabase stack running on `vds-dev` (ekza-registry `deploy/dev`, Supabase CLI project `ekza-dev`, standard port block). The tool needs its database container `supabase_db_ekza-dev` running with 5432 published on `127.0.0.1:54322` only, and refuses anything else. Nothing else from it is used.
6. Optional: `https://registry.dev.ekza.io` live (the ekza-registry dev package). Without it, the dev lobby still runs and serves matches, and avatars fall back to the shipped roster.

No password or key is an owner input. The game role's password is generated on the host and
stays in `/opt/omoba-dev/database.env`.

## Ship or update the lobby

Run this from a clean checkout of `origin/dev` (not a worktree with local edits):

```sh
export DEV_HOST=vds-dev DEV_IP=<dev VPS IPv4>
ssh -G "$DEV_HOST" | awk '$1=="hostname"{print $2}'          # must print $DEV_IP
python3 scripts/release.py build linux --contour dev --dry-run
python3 scripts/release.py build linux --contour dev         # Docker on macOS; shared target/linux-docker
V=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
P=dist/v$V-dev/omoba-$V-dev-linux-x64.tar.gz
python3 ops/vps/dev/lobby_dev.py ship --package "$P"           # plan only, no network
python3 ops/vps/dev/lobby_dev.py ship --package "$P" --check   # read-only host preflight
python3 ops/vps/dev/lobby_dev.py ship --package "$P" --apply
python3 ops/vps/dev/lobby_dev.py status
make compatibility-check GAME_SERVER_ADDR=lobby.dev.omoba.io:4100 \
  COMPATIBILITY_MANIFEST=dist/v$V-dev/compatibility.json
```

`release.py build linux` also builds the Bevy client. When only the lobby changes, or
`target/linux-docker` has been pruned and disk is short, build just the two binaries in the
same image, mounts and compiled defaults, then ship them with `--binary`:

```sh
T=$(cd "$(git rev-parse --path-format=absolute --git-common-dir)/.." && pwd)/target/linux-docker
docker run --rm --platform linux/amd64 -v "$PWD:/src" -v "$T:/cache" -w /src \
  -e CARGO_TARGET_DIR=/cache -e OMOBA_DEFAULT_GAME_SERVER_ADDR=lobby.dev.omoba.io:4100 \
  -e OMOBA_DEFAULT_REGISTRY_URL=https://registry.dev.ekza.io \
  -e OMOBA_DEFAULT_PASSPORT_URL=http://127.0.0.1:9/api/passport -e OMOBA_DEFAULT_CONFIG_DIR_SUFFIX=dev \
  rust:1.95.0-bookworm cargo build --locked --release -p server -p omoba-account-api
python3 ops/vps/dev/lobby_dev.py ship --binary "$T/release/server" --account-api "$T/release/omoba-account-api" \
  --version "$V" --check      # then --apply
```

The `-e` values are `release.py build linux --contour dev --dry-run`'s `compiled_defaults`, and
the toolchain is `rust-toolchain.toml`'s. Such a release records `"package": null` in `BUILD.json`.

The dev Linux package carries `omoba-server` (the lobby) and `omoba-account-api` (only its
`migrate` command runs, never the service).

What `--apply` does:

1. Preflight. It refuses on any production marker: unit `omoba-lobby`, user `omoba`, `/opt/omoba`, `/var/lib/omoba`, `/opt/ekza-studio`, `/opt/ekza-mirror`, the NPM edge, or address 77.246.105.57. The dev host's own `/opt/ekza-dev` (ekza-registry `deploy/dev`) is not a marker. It also refuses when dev match workers are running, when UDP 4100/41100 is held by something else, when less than 600 MiB is available, when `lobby.dev.omoba.io` is missing, wrong, or has AAAA records, when `supabase_db_ekza-dev` is not running or is published beyond `127.0.0.1:54322`, when an existing `database.env` is not root-owned 0600 or names anything but the dev URL, and when a database `omoba_dev` exists with another owner.
2. Stage. It creates user `omoba-dev`, `/opt/omoba-dev/releases/<version>-dev-<sha7>/` and `/var/lib/omoba-dev/{matches,lobby-outbox}`. Release directories are immutable.
3. Upload. It sends `server.part` and `omoba-account-api.part` (only the missing ones), then checks each sha256 on the host.
4. Database, before the running lobby is touched. Over the container's local socket it creates the three roles (NOLOGIN) and `omoba_dev` when missing. When `database.env` is missing, it generates the game role's password on the host and writes the file (root, 0600). It sets that password on `omoba_dev_game`, gives `omoba_dev_owner` a temporary login that expires in 15 minutes, runs the release's `omoba-account-api migrate`, applies `grants.sql` as the owner, and disables the owner login again. SQL goes over stdin and the URL over the child's environment, never argv. Error output is redacted. If any of this fails, the running lobby keeps running.
5. Activate. It points `current` at the release, installs the unit if it changed (`systemd-analyze verify`, then `daemon-reload`) and restarts. It then verifies `/proc/<MainPID>/exe` and UDP 4100. On any failure it rolls back to the previous release and unit. A failed first install stays stopped and disabled, so you can read `journalctl -u omoba-lobby-dev`.

Re-running the same command is a no-op when that release, that unit and its database
already exist. Every switch is appended to `/opt/omoba-dev/deployments.jsonl` (no secrets).

Rollback: `python3 ops/vps/dev/lobby_dev.py rollback --release <version>-dev-<sha7> --apply`
(the release must carry its `BUILD.json`). A rollback never runs migrations: schema changes
are forward-only, as in production, so roll back only to a release that works with the
current schema. It needs a valid `database.env`.

On the host: `systemctl status omoba-lobby-dev`, `journalctl -u omoba-lobby-dev -f`,
`systemctl show omoba-lobby-dev -p MemoryCurrent -p NRestarts`. An open UDP socket does
not prove gameplay. Finish with two signed-in dev clients reaching a match on 41100.

The database lives in the dev Supabase stack's Docker volume, so anything that recreates
that volume for project `ekza-dev` (`supabase stop --no-backup`, a volume removal, possibly
`supabase db reset`) loses it with everything else. The next `ship --apply`, even of the
release that is already running, recreates the roles, the database and the schema with the
password already in `database.env`. When the release is unchanged it does so without
restarting the lobby (status `DATABASE_PREPARED`), and the lobby's career worker reconnects
on its own. Dev accounts and results are synthetic and may be lost at any time.

Teardown, as root on `vds-dev` only:
```sh
systemctl disable --now omoba-lobby-dev && rm /etc/systemd/system/omoba-lobby-dev.service && systemctl daemon-reload
docker exec -i supabase_db_ekza-dev psql -X -v ON_ERROR_STOP=1 -U supabase_admin -d postgres <<'SQL'
DROP DATABASE IF EXISTS omoba_dev;
DROP ROLE IF EXISTS omoba_dev_game; DROP ROLE IF EXISTS omoba_dev_portal; DROP ROLE IF EXISTS omoba_dev_owner;
SQL
rm -rf /opt/omoba-dev /var/lib/omoba-dev && userdel omoba-dev
```

Limits: the unit caps the lobby and its one match worker at 512 MiB, 100% CPU and
96 tasks, with `OOMScoreAdjust=300`. The dev Supabase stack, registry and Studio share
the host, and under memory pressure the kernel kills the lobby before the database. The
hardening flags are the production unit's, so the dev unit doubles as a rehearsal.

Supabase Pro later: that move is for the Ekza registry's project. `omoba_dev` stays on the
dev host. If the dev Supabase stack is retired, give the lobby its own Postgres on the host
(same roles, same tool, a new `DB_CONTAINER`/`DB_PORT`). `lobby_dev.py` refuses hosted
Supabase URLs on purpose, so pointing the lobby at a hosted project would be a separate,
reviewed change.

Rehearse the database step locally (no VPS) with the opt-in smoke tests in
`scripts/test_lobby_dev.py` (`RealServerSmoke`, `RealDatabaseSmoke`; see its docstring).
They run the real server with the dev unit's environment and the real migration against a
disposable `public.ecr.aws/supabase/postgres` container.

## Dev tester builds

```sh
python3 scripts/release.py build all --contour dev --dry-run   # what would be baked in
python3 scripts/release.py build all --contour dev             # macOS, Windows (cargo-xwin), Linux, Android, iOS on this Mac
```

- Baked in: `OMOBA_DEFAULT_GAME_SERVER_ADDR=lobby.dev.omoba.io:4100` (`--server` overrides; prefer the IP when DNS is not set), `OMOBA_DEFAULT_REGISTRY_URL=https://registry.dev.ekza.io` (`--registry` overrides), `OMOBA_DEFAULT_PASSPORT_URL=http://127.0.0.1:9/api/passport` (`--passport` overrides, dev or loopback only) and `OMOBA_DEFAULT_CONFIG_DIR_SUFFIX=dev`.
- No build, dev or production, inherits a compile-time setting from the shell: every `OMOBA_DEFAULT_*`, `OMOBA_ACCOUNT_API_URL`, `OMOBA_PORTAL_ORIGIN` and `EKZA_DEV_HTTP_HOST` is dropped before the build sets its own. That covers macOS, Windows, Linux (native and Docker), Android and the iOS archive. `scripts/test_release.py` checks the list against every `option_env!` in the sources.
- Settings, identity key and the connection choice live in `~/.config/omoba-bevy-dev` (macOS/Linux) or `%APPDATA%\omoba-bevy-dev` (Windows); each connection profile keeps its Ekza session, avatar/weapon caches and match-resume hint in `profiles/<profile>/` there. A tester's production install in `omoba-bevy` is never read or written. `OMOBA_CLIENT_CONFIG_DIR` still overrides both.
- No production lobby at run time either. A dev build's profiles are Dev (this build's lobby and registry), Local and Custom (Home → Connection, applied at the next start; [connection profiles](../../../docs/connection-profiles.md)). Every address the client validates (custom, `GAME_SERVER_ADDR`, match allocations) is refused when it is the production VPS (77.246.105.57, any port) or an `omoba.io`/`ekza.io` name outside `dev.*`; a refused custom profile falls back to Dev.
- Packages: `dist/v<version>-dev/Omoba-<version>-dev-macos-arm64.zip`, `…-dev-windows-x64.zip` and `omoba-<version>-dev-linux-x64.tar.gz` (which also carries `omoba-account-api` for the dev lobby). The macOS app is `Omoba Dev.app` with bundle id `space.ekza.omoba.desktop.dev`.
- Phones: the OMOBA Dev app, `space.ekza.omoba.dev` ("OMOBA Dev", DEV-ribbon icon), installs beside the production `space.ekza.omoba.beta`. `release.py build android|ios --contour dev`; phone builds take only `--server`.
- Cross-contour refusals:
  - A dev build refuses the production VPS on any port, any non-dev `omoba.io`/`ekza.io` name, the production registry, and any passport that is not a dev name or loopback.
  - A production build refuses `*.dev.omoba.io`/`*.dev.ekza.io` (server, registry, passport), and also `$DEV_IP` when it is set.
- Distribution happens only with the owner's OK. Use `python3 scripts/release.py draft --contour dev --tag v<version>-dev.N`, which creates a draft prerelease targeting `dev`. Or use CI: `python3 scripts/release.py ci --contour dev --ref v<version>-dev.N`, which dispatches `release.yml` on `dev` with `contour=dev`. Add `--dry-run` to either command to print it without running it.

## DEV Account API

`account_api_dev.py` runs on `vds-dev` as root and defaults to a read-only preflight.
It is limited to `146.103.102.18`, the loopback dev Supabase database and the
unprivileged `omoba_dev_portal` role. It refuses known production paths and
exposed database ports. No credentials are accepted in arguments or printed.

Prerequisites: a completed Registry backup including `omoba_dev.dump`, exact DNS
`account.dev.omoba.io` → `146.103.102.18`, and Registry's Caddy fragment support
(PR21). The native DEV package must contain that exact API origin and
`https://app.dev.omoba.io` as its portal origin.

```sh
ssh vds-dev 'python3 -' < ops/vps/dev/account_api_dev.py
ssh vds-dev 'python3 - --apply --backup /opt/ekza-dev/backups/YYYYMMDDTHHMMSSZ' \
  < ops/vps/dev/account_api_dev.py
# Ship the frozen DEV server+Account API first with lobby_dev.py (above).
# Then replace FULL_SHA with its exact 40-character BUILD.json revision:
ssh vds-dev 'python3 - --apply --backup /opt/ekza-dev/backups/YYYYMMDDTHHMMSSZ --activate-revision FULL_SHA' \
  < ops/vps/dev/account_api_dev.py
```

Preparation creates/reuses root-owned `0600` files:
`/opt/omoba-dev/account-api.env`, `/opt/omoba-dev/account-api-ingress.key` and
`/opt/ekza-dev/secrets/caddy/account-api.caddy`. It enables only the portal runtime
login, preserves its password on retries and installs the bounded
`omoba-account-api-dev.service`. Activation checks DEV contour, revision, binary
SHA-256 and portal migrations 1–6 before any writes. Existing credential drift
is rejected without rotation. The operator owns these files; preserve a private
copy before any later intentional change. Never attach them to reports or PRs.

The service binds `127.0.0.1:40550`. The Caddy fragment exposes health checks and
an explicit POST allowlist of signed native account endpoints. Other `/v1/*`
routes require `X-Omoba-Portal-Key`, supplied by the website server as the private
`OMOBA_ACCOUNT_INGRESS_KEY`. This is an ingress gate; normal session/CSRF/signature
checks still apply. Caddy removes the key before forwarding and replaces the
client-IP header. Browser requests must never receive the key. Caddy's admin API
stays disabled. Validate and recreate only the dev Caddy container after editing
or removing the fragment, following the Registry dev runbook; ordinary app
rollbacks do not restore operator-managed fragments.

After activation check `/health/live` and `/health/ready` over HTTPS, anonymous
private-route 403 responses, invalid-signed native requests rejected by the API,
and a real website pairing flow. Check unit restarts, loopback binding, role
privileges and database schema. No payment provider is enabled by this helper.

Rollback: disable website account mode before stopping `omoba-account-api-dev`.
Keep credentials private and stable. Restore the previous reviewed Caddy fragment
(or remove only this new fragment), validate and recreate dev Caddy. Disable the
portal login only after the API is stopped. A binary rollback must be compatible
with the current schema; do not automatically downgrade or restore databases over
new user data. Use the pre-deploy backup only as a separately reviewed recovery.
