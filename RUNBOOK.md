# Local Development Runbook

Single-machine multiplayer testing for omoba-bevy.

## Prerequisites

- Rust toolchain (`rustup`) installed.
- Repository cloned and dependencies fetched (`cargo build --workspace`).

## Quick Start (two-client local play)

```sh
make start
```

This launches:
1. The game server (background) bound to `0.0.0.0:4000`.
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

## Individual Commands

| Goal | Command |
|---|---|
| Start server only | `make server` |
| Start one client | `make game` |
| Start server + 2 clients | `make start` |
| Stop all local processes | `make stop` |
| Full restart | `make restart` |

## Environment Variables

| Variable | Default | Description |
|---|---|---|
| `SERVER_ADDR` | `0.0.0.0:4000` | Address the server binds to |
| `GAME_SERVER_ADDR` | `127.0.0.1:4000` | Server address clients connect to |

Example — run server on a non-default port:

```sh
SERVER_ADDR=0.0.0.0:5000 make server
# in another terminal:
GAME_SERVER_ADDR=127.0.0.1:5000 make game
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
| `Address already in use` on server start | Previous server still running | `make stop` |
| Client never receives first snapshot | Server not started or wrong port | Check `GAME_SERVER_ADDR` matches `SERVER_ADDR` |
| `Failed to bind client UDP socket` | OS port exhaustion (rare) | Restart the client |
| Client exits immediately | Build error or missing assets | Run `cargo build --workspace` and check for errors |

## Repeated Restart Behavior

After an abnormal termination (e.g. `kill -9`):

1. Run `make stop` to ensure no stale processes hold the port.
2. Run `make start` (or `make restart` which does both automatically).

No code edits are required between restarts.

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

**Preferences file** (optional server address + graphics): see `client/src/persistence.rs` (`OMOBA_CLIENT_CONFIG_DIR` or default OS path). **Env wins** over saved `game_server_addr` when `GAME_SERVER_ADDR` is set and non-empty.
