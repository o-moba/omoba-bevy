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
