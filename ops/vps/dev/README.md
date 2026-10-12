# OMOBA DEV lobby and dev tester builds

The DEV contour runs on a **dedicated dev VPS** (Ubuntu 24.04 amd64, about 2 vCPU / 4 GB),
never on the production VPS. Its ssh alias is `vds-dev` (root, key auth). Its address is
known only from `DEV_IP`, and every tool reads it from the environment.

| Piece | DEV | Production (never a target here) |
| --- | --- | --- |
| Host | `vds-dev`, `$DEV_IP` | `vds-eternal`, 77.246.105.57 |
| Unit | `omoba-lobby-dev` ([../omoba-lobby-dev.service](../omoba-lobby-dev.service)) | `omoba-lobby` |
| User, paths | `omoba-dev`, `/opt/omoba-dev`, `/var/lib/omoba-dev` | `omoba`, `/opt/omoba`, `/var/lib/omoba` |
| UDP | lobby 4100, match worker 41100 (capacity 1) | 4000, 41000–41001 |
| Registry | `https://registry.dev.ekza.io` | `https://registry.ekza.io` |
| Database | none: guests only, career records refused | `omoba` in PostgreSQL |
| Client address | `lobby.dev.omoba.io:4100` | 77.246.105.57:4000 |
| Client settings folder | `omoba-bevy-dev` | `omoba-bevy` |

The dev lobby has no web UI, so there is nothing to mark noindex. Dev clients say
`OMOBA DEV` in the window title and are named `Omoba Dev.app`. Only synthetic accounts and
guests use it. Nobody copies production data to it.

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
5. Optional: `https://registry.dev.ekza.io` live (the ekza-registry dev package). Without it, the dev lobby still serves guest matches. Studio avatars then fall back to the shipped roster.

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

What `--apply` does:

1. Preflight. It refuses on any production marker: unit `omoba-lobby`, user `omoba`, `/opt/omoba`, `/var/lib/omoba`, `/opt/ekza-*`, the NPM edge, or address 77.246.105.57. It also refuses when dev match workers are running, when UDP 4100/41100 is held by something else, when less than 600 MiB is available, or when `lobby.dev.omoba.io` is missing, wrong, or has AAAA records.
2. Stage. It creates user `omoba-dev`, `/opt/omoba-dev/releases/<version>-dev-<sha7>/` and `/var/lib/omoba-dev/{matches,lobby-outbox}`. Release directories are immutable.
3. Upload. It sends `omoba-server` as `server.part`, then checks the sha256 on the host.
4. Activate. It points `current` at the release, installs the unit if it changed (`systemd-analyze verify`, then `daemon-reload`) and restarts. It then verifies `/proc/<MainPID>/exe` and UDP 4100. On any failure it rolls back to the previous release and unit. A failed first install stays stopped and disabled, so you can read `journalctl -u omoba-lobby-dev`.

Re-running the same command is a no-op when that release and unit already run. Every
switch is appended to `/opt/omoba-dev/deployments.jsonl`.

Rollback: `python3 ops/vps/dev/lobby_dev.py rollback --release <version>-dev-<sha7> --apply`
(the release must carry its `BUILD.json`).

On the host: `systemctl status omoba-lobby-dev`, `journalctl -u omoba-lobby-dev -f`,
`systemctl show omoba-lobby-dev -p MemoryCurrent -p NRestarts`. An open UDP socket does
not prove gameplay; finish with a real dev client reaching a match on 41100.

Teardown, as root on `vds-dev` only:
`systemctl disable --now omoba-lobby-dev && rm /etc/systemd/system/omoba-lobby-dev.service && systemctl daemon-reload && rm -rf /opt/omoba-dev /var/lib/omoba-dev && userdel omoba-dev`.

Limits: the unit caps the lobby and its one match worker at 512 MiB, 100% CPU and
96 tasks, with `OOMScoreAdjust=300`. The dev Supabase stack, registry and Studio share
the host, and under memory pressure the kernel kills the lobby before the database. The
hardening flags are the production unit's, so the dev unit doubles as a rehearsal.

Later, only with an explicit OK: give the dev lobby a dev database. That needs an
`EnvironmentFile`, which the unit check currently refuses, plus its own roles, created
the same way as production (`account-api/ops/grants.sql`) but never in the production
PostgreSQL.

## Dev tester builds

```sh
python3 scripts/release.py build all --contour dev --dry-run   # what would be baked in
python3 scripts/release.py build all --contour dev             # macOS, Windows (cargo-xwin), Linux on this Mac
```

- Baked in: `OMOBA_DEFAULT_GAME_SERVER_ADDR=lobby.dev.omoba.io:4100` (`--server` overrides; prefer the IP when DNS is not set), `OMOBA_DEFAULT_REGISTRY_URL=https://registry.dev.ekza.io` (`--registry` overrides) and `OMOBA_DEFAULT_CONFIG_DIR_SUFFIX=dev`. Values exported in the shell are never inherited.
- Settings, saved server, identity key, Ekza session and avatar/weapon caches live in `~/.config/omoba-bevy-dev` (macOS/Linux) or `%APPDATA%\omoba-bevy-dev` (Windows). A tester's production install in `omoba-bevy` is never read or written. `OMOBA_CLIENT_CONFIG_DIR` still overrides both.
- Packages: `dist/v<version>-dev/Omoba-<version>-dev-macos-arm64.zip`, `…-dev-windows-x64.zip` and `omoba-<version>-dev-linux-x64.tar.gz`. The macOS app is `Omoba Dev.app` with bundle id `space.ekza.omoba.desktop.dev`.
- Desktop only. An APK or iPhone build would share the production app id, so `release.py` refuses `android`/`ios` for `--contour dev`.
- Cross-contour refusals:
  - A dev build refuses the production VPS on any port, any non-dev `omoba.io`/`ekza.io` name and the production registry.
  - A production build refuses `*.dev.omoba.io`/`*.dev.ekza.io`, and also `$DEV_IP` when it is set.
- Distribution happens only with the owner's OK. Use `python3 scripts/release.py draft --contour dev --tag v<version>-dev.N`, which creates a draft prerelease targeting `dev`. Or use CI: `python3 scripts/release.py ci --contour dev --ref v<version>-dev.N`, which dispatches `release.yml` on `dev` with `contour=dev`. Add `--dry-run` to either command to print it without running it.
