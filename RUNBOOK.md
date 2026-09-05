# Local Development Runbook

Single-machine multiplayer testing for omoba-bevy.

Entry point: see [README.md](README.md) for first-time setup and a controls summary. This file focuses on processes, logs, and recovery.

## Prerequisites

- Rust toolchain (`rustup`) installed.
- Repository cloned and dependencies fetched (`cargo build --workspace`).

## Match Modes (TASK-22)

The server has two explicit match-start modes, selected with
`OMOBA_MATCH_MODE`:

| Mode | Behavior | When to use |
|---|---|---|
| `release` (**default**) | Production-like matchmaking: players who join land in a queue, the match forms to a full 5v5 roster (`2 × OMOBA_TEAM_SIZE`), teams are **assigned by the server** (balanced, client choice is a preference only), a 3-second countdown runs, and only then the match starts. A solo player waits at "Searching for match..." forever — by design. | Anything player-facing / release-like. |
| `dev` | Instant start: the first join flips the match to Running immediately and the client-chosen team is honored. This is the historical development behavior. | Local development and debugging ONLY. **Never ship dev mode.** |

The bare server (`make server`, `cargo run -p server`) runs **release** mode —
you cannot accidentally start an under-filled match in a production-like
setup. Every quick dev flow goes through an explicit dev target.

Client-side search states you should see in release mode after picking a
class/avatar/team: `Searching for match...` → `Waiting for players — X/10` →
`Match found! Starting in N...` → match begins.

## Quick Start (dev mode, two-client local play)

```sh
make start
```

This launches (all in **dev** match mode — instant start):
1. The game server (background) bound to `0.0.0.0:4000` with `OMOBA_MATCH_MODE=dev`.
2. A first game client (background) connecting to `127.0.0.1:4000`.
3. A second game client (foreground) connecting to `127.0.0.1:4000`.

Stop everything cleanly:

```sh
make stop
```

Restart from scratch (stop → wait → start):

```sh
make restart
```

## Full Match Flow Solo (release mode + bots)

One developer can walk through the real matchmaking UX end-to-end:

```sh
make play-bots
```

This starts a release-mode server, nine fill bots in the background, and your
client in the foreground. Pick a class/avatar/team in the client — you are
the 10th player: the overlay shows the queue filling, `Match found!`, the
countdown, and the match starts as a real 5v5. Once the match runs, the bots
play their lanes: each pushes Mid/Top/Bot toward the enemy base, fights the
enemy players and minions it meets with its Q ability, sieges towers in
reach, and rejoins its lane after a respawn. It is deliberately simple
nearest-target AI (no retreat, no combos) — built to playtest matchmaking
and basic playability, not to be challenging opponents.

Manual variant (separate terminals):

```sh
make server            # terminal 1: release-mode server
make game              # terminal 2: your client, join the queue
make bots              # terminal 3: fill the remaining 9 seats
# make bots BOTS=4     #   ...or any other number of bots
```

Use `make game2d` instead of `make game` to launch that client directly in
the genuine orthographic 2D game mode. It uses a planar XY render world and
keeps server-authoritative simulation coordinates in XZ (`x → x`, `z → y`).
The mode starts one `Camera2d`; it does not render the 3D arena, GLBs, mesh
billboards, or directional-light scene. Both client commands explicitly target
`127.0.0.1:4000` by default so an old persisted server address cannot redirect
the local run; `GAME_SERVER_ADDR=<host:port>` overrides that default.

2D camera controls: mouse wheel zooms within map-safe limits; **Y** toggles
hero-follow/free-pan; arrow keys pan while unlocked; **Space** restores follow
and clears a minimap focus override. Pressing **Y** while a minimap focus is
active also returns directly to the hero. Minimap clicks focus the camera
without leaking a world movement command. Ground movement remains available
while camera follow is unlocked; right-click and Alt are reserved for their
legacy 3D camera behavior and do not capture or toggle the 2D camera.

Desktop/mobile combat input uses one pointer intent per press. Click or tap
empty ground to move. Click or tap a living hostile actor to select it and
request Q; the gold marker confirms selection, and the hero approaches the
moving target when it is outside the shared ability range before emitting one
server-authoritative cast. Tab still selects the nearest hostile, middle-click
selects without attacking, and Q/W/E/R or the four 64-pixel on-screen buttons
cast the chosen slot. Ground, minimap, and HUD presses are isolated so a touch
cannot both cast and redirect movement.

Validate the offline assets before a playtest:

```sh
python3 scripts/validate_world2d_assets.py --self-test
python3 scripts/validate_sprite_assets.py --self-test
python3 scripts/validate_2d_readability.py --json
```

Checking matchmaking behavior quickly without a GPU:

```sh
make verify-gameplay   # includes the release-mode formation harness test
```

## Individual Commands

| Goal | Command |
|---|---|
| Start server (release matchmaking) | `make server` |
| Start server (dev instant-start) | `make server-dev` |
| Start one client | `make game` |
| Start one 2D client | `make game2d` |
| Dev quick-start: dev server + 2 clients | `make start` |
| Release server + 1 client | `make start-release` |
| Full solo 5v5 demo (server + 9 bots + client) | `make play-bots` |
| Fill bots for a running server | `make bots` (`BOTS=<n>`, `BOTS_SERVER=<addr>`) |
| Stop all local processes (incl. bots) | `make stop` |
| Full dev restart | `make restart` |
| Headless gameplay + matchmaking tests | `make verify-gameplay` |

## Environment Variables

| Variable | Default | Description |
|---|---|---|
| `SERVER_ADDR` | `0.0.0.0:4000` | Address the server binds to |
| `GAME_SERVER_ADDR` | `127.0.0.1:4000` | Server address clients connect to |
| `OMOBA_MATCH_MODE` | `release` | `release` = queue to full 5v5 before start; `dev` = instant start on first join (dev only) |
| `OMOBA_TEAM_SIZE` | `5` | Players per team in release mode (clamped 1–16); `1` gives a 1v1 with release semantics for quick checks |
| `OMOBA_AUTOJOIN` | unset | Client joins without UI: `<class>:<avatar-slug|->:<team>` |

Example — run server on a non-default port:

```sh
SERVER_ADDR=0.0.0.0:5000 make server
# in another terminal:
GAME_SERVER_ADDR=127.0.0.1:5000 make game
```

Example — release-semantics 1v1 sanity check on one machine:

```sh
OMOBA_TEAM_SIZE=1 make server      # terminal 1
make game                          # terminal 2: joins, waits at 1/2
make bots BOTS=1                   # terminal 3: fills the seat, match starts
```

## Expected Startup Log Output

### Server

```
UDP game server is listening on 0.0.0.0:4000
Player 1 connected from 127.0.0.1:<port>
Player 1 joined team Green as Ipfs
```

### Client

```
Connecting to server at 127.0.0.1:4000
UDP socket connected to 127.0.0.1:4000; waiting for first snapshot
First snapshot received from 127.0.0.1:4000; connection is live
```

If the client prints "Connecting to server…" but never prints "First snapshot received…", the
server is not running or is on a different address/port.

## Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| `Address already in use` on server start | Previous server still running | Run `make stop`; if it persists, find and terminate the process bound to the port (`lsof -i :4000` on macOS/Linux) and retry |
| Client stuck on "Connecting…" / no snapshot | Server not running, wrong host, or firewall | 1) Confirm server log shows listening. 2) Ensure client `GAME_SERVER_ADDR` host and **port** match `SERVER_ADDR` (default `127.0.0.1:4000`). 3) Allow UDP for local binaries in OS firewall. 4) Retry `make restart` after `make stop` |
| Client never receives first snapshot | Port mismatch between server bind and client target | Parse port from `SERVER_ADDR` (e.g. `0.0.0.0:5000` → clients use `127.0.0.1:5000` in `GAME_SERVER_ADDR`) |
| Second machine on LAN cannot connect | Client still points at localhost | On the client machine set `GAME_SERVER_ADDR=<server-LAN-IP>:4000`; on the server keep `SERVER_ADDR=0.0.0.0:4000` so it accepts non-local interfaces |
| `Connection refused` (tools) or immediate disconnect | Nothing listening on declared port | Start server first; verify with server log line; check VPN or corporate proxy is not blocking UDP loopback |
| Repeated JSON EOF at column `8192` | Client/server binaries are from different revisions, or an old client still uses the former 8 KiB receive buffer | Rebuild both with `cargo build --workspace`, restart the server and client, and confirm both use the same checkout. Current clients receive up to a complete legal 65,507-byte IPv4 UDP payload. |
| Server logs `Failed to send complete ... snapshot` / `Message too long` | The populated one-datagram JSON snapshot exceeds the host kernel's UDP send ceiling (measured as 9,216 bytes on the current macOS host) | Reduce the playtest roster/entity load. Do not raise buffers blindly: payload reduction or versioned fragmentation/compression is a separate protocol change. |
| Towers or minions seem absent in 2D | Old assets/binary, extreme camera state, or the match has not reached `running` | Rebuild, use `make game2d`, press **Space** to restore hero follow, and verify the server snapshot has 8 structures and 18 minions with `cargo test -p harness --test udp_datagrams -- --nocapture`. Green actors use square badges; Blue actors use diamonds; lane towers also show TOP/MID/BOT. |
| `Failed to bind client UDP socket` | OS port exhaustion (rare) | Close other clients; reboot if needed; retry `make game` |
| Client exits immediately | Build error or asset load failure | Run `cargo build --workspace` from repo root and fix compile errors; check client stderr for asset path errors (the client uses `client/assets/` under the crate, including an auto-created downloads subfolder) |
| `make stop` did not clear processes | Processes not started via `make start` | Manually kill `target/debug/server` and `target/debug/client` (or `pkill -f target/debug/server` / `client`) then confirm the port is free |

## Repeated Restart Behavior

After an abnormal termination (e.g. `kill -9`):

1. Run `make stop` to ensure no stale processes hold the port.
2. Run `make start` (or `make restart` which does both automatically).

No code edits are required between restarts.

## Authority and Reconnect Notes

- The server treats client movement packets as requested positions and clamps accepted movement by player speed, elapsed server time, and map bounds. Large or non-finite transform jumps are ignored or reduced to legal movement.
- Cast requests are server validated against the authoritative caster position, target team/state, mana, cooldown, and range.
- The Bevy client stores a stable `client_session_id` in the preferences file. A new UDP endpoint with the same valid session id can reclaim a recently timed-out player slot/id; clients without that id keep the older reconnect-as-new-player behavior.
- The session id is not a login credential or anti-cheat secret. It is only a local playtest continuity token.

## TASK-14 — Network failure and session resilience (manual QA)

Use two terminals (or `make start` / `make stop`). Constants live in `client/src/session_config.rs`.

| Step | Action | Expected client state / UI |
|------|--------|----------------------------|
| 1 | Start **client only** (no server) | **WaitingForServer** (or brief **Connecting**): panel explains address, retry interval, max wait; viewport keeps rendering; after `T_WAIT_MAX` → **Disconnected** with **Retry** visible. |
| 2 | Start server while client waits or after **Retry** | First qualifying snapshot → **Connected**; status text updates; join flow works. |
| 3 | Reach **Connected**, play briefly, **kill server** | Within `T_STALE_SNAPSHOT` (or transport threshold if errors surface), **Disconnected**; replicated world cleared; team select returns; game-state overlay hidden. |
| 4 | Restart server, click **Retry**, join again | Returns to normal **Connected** flow (manual path — no auto-rejoin into match). If the client UDP thread exited (e.g. bind failure), restart the client instead — **Retry** does not respawn that thread. |
| 5 | Spam team join (double-click / rapid confirm) | At most one local `Player` after sync; `Join` idempotent while `join_flow_committed`. |
| 6 | Block UDP (e.g. firewall) while **Connected** | Stale snapshot or transport rule → **Disconnected** same as server kill. |

**Preferences file** (optional server address + graphics + stable client session id): see `client/src/persistence.rs` (`OMOBA_CLIENT_CONFIG_DIR` or default OS path). **Env wins** over saved `game_server_addr` when `GAME_SERVER_ADDR` is set and non-empty.
