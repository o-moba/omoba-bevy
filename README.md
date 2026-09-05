# omoba-bevy

Authoritative-server MOBA-style prototype (Bevy client, Rust UDP server).

## Prerequisites

- [Rust toolchain](https://rustup.rs/) (`rustc`, `cargo`).
- Git and a clone of this repository.

## First-time setup

From the **repository root** (the directory that contains this `README.md` and the `Makefile`):

```sh
cargo build --workspace
```

This compiles the `server` and `client` crates. Expect several minutes on the first run while dependencies build.

## Run the game (happy path)

All startup commands, environment variables, log expectations, and shutdown steps are documented in **[RUNBOOK.md](RUNBOOK.md)**.

Typical local two-client dev session (instant match start, dev mode):

```sh
make start
```

Full release-like matchmaking flow solo (release server + 9 fill bots + your
client; the match forms to a real 5v5 through the queue, and once it starts
the bots actively play — they push their lanes toward the enemy base and
fight players, minions, and towers with simple nearest-target AI):

```sh
make play-bots
```

Production-like server on its own (matches start only after a full 5v5 queue;
instant start is dev-only via `make server-dev` / `make start`):

```sh
make server
```

Stop background processes afterward:

```sh
make stop
```

## Command cheat sheet

| Command | What it does |
| --- | --- |
| `make start` | **Dev quick-start**: dev-mode server + 2 clients, instant match start |
| `make play-bots` | **Solo 5v5 demo**: release server + 9 fill bots + your client (you are the 10th player, full matchmaking flow) |
| `make server` | Server in **release** matchmaking mode (queue to full 5v5 before start) |
| `make server-dev` | Server in **dev** mode (first join starts the match; dev only) |
| `make game` | One client (defaults explicitly to `127.0.0.1:4000`; set `GAME_SERVER_ADDR` for another server) |
| `make game2d` | Same explicit local server default, forced into the genuine orthographic 2D game mode |
| `make start-release` | Release server + 1 client (waits in queue until filled) |
| `make bots` | Fill bots for a running server (`BOTS=<n>`, `BOTS_SERVER=<addr>`) |
| `make stop` | Kill server, clients, and bots |
| `make restart` | `stop` + dev quick-start |
| `make verify-gameplay` | Headless gameplay + matchmaking tests over real UDP |
| `make verify-task-12` | Live UDP QA matrix |

Key env vars: `OMOBA_MATCH_MODE` (`release` default / `dev`), `OMOBA_TEAM_SIZE`
(players per team, default 5 — `1` gives a quick 1v1 with release semantics),
`SERVER_ADDR`, `GAME_SERVER_ADDR`, `OMOBA_PLAYER_VISUAL_MODE`
(`models3d` default / `sprite2d` genuine orthographic XY renderer), and
`OMOBA_AUTOJOIN` (`<class>:<avatar|->:<team>[:<sprite-id>]`). Validate the
offline character/presentation contract with
`python3 scripts/validate_sprite_assets.py --self-test` and the tiled-world
contract with `python3 scripts/validate_world2d_assets.py --self-test`.
`python3 scripts/validate_2d_readability.py --json` additionally checks the
occupied (non-transparent) pixels at default zoom and maximum zoom-out.

Do not rely on tribal knowledge for ports or addresses: use the tables in `RUNBOOK.md` (`SERVER_ADDR`, `GAME_SERVER_ADDR`).

## Tester-facing documentation

| Document | Purpose |
| --- | --- |
| [RUNBOOK.md](RUNBOOK.md) | Server/client startup, env vars, troubleshooting with recovery steps |
| [docs/playtest-script.md](docs/playtest-script.md) | Timeboxed 10–20 minute MVP playtest checklist |
| [docs/bug-report-template.md](docs/bug-report-template.md) | Expected internal bug report format |
| [docs/mvp-scope-and-limitations.md](docs/mvp-scope-and-limitations.md) | MVP scope, limitations, MVP vs deferrable gaps |
| [tasks/MVP-CHECKLIST.md](tasks/MVP-CHECKLIST.md) | MVP-blocking vs later improvements |

## Controls and gameplay (MVP summary)

| Input | Action |
| --- | --- |
| **Arrow keys** (2D) | Pan the orthographic camera while follow is unlocked |
| **Mouse wheel** | Zoom the active camera; 2D zoom is clamped to the map |
| **Y** | Toggle hero follow/free camera; a minimap focus returns directly to the hero |
| **Space** | Restore hero follow and clear a minimap focus override |
| **Alt**, **right click**, **W A S D / mouse look** (3D only) | Legacy free-camera toggle and movement |
| Team / character UI | Click team and character before play; server snapshot is authoritative |
| **Click/tap ground** | Move the hero on desktop or touch devices |
| **Click/tap hostile** | Select it and use Q; the hero approaches into range automatically |
| **Tab** | Select nearest enemy target |
| **Middle mouse** | Select a target without immediately attacking |
| **Backspace** | Clear target |
| **Q / W / E / R** | Cast that class ability at the selected target |
| On-screen **skill** button | Touch/click alternative to Q/W/E/R |
| **Esc** | Pause menu |

Gameplay: lane map, minions, structures, combat, respawn, match phases (lobby → running → victory / rematch), and progression HUD as described in [docs/features.md](docs/features.md).

## Optional automated session check

After a successful build, you can run the UDP session harness (separate port from the default Makefile flow):

```sh
python3 scripts/verify_task_02_multiplayer_session_flow.py
```

See the script header for prerequisites; it spawns its own server on `127.0.0.1:4010`.

## UDP snapshot size

Snapshots remain one JSON object per UDP datagram. The client and harness use
65,536-byte receive storage and the server rejects a serialized snapshot above
the legal IPv4 UDP payload ceiling of 65,507 bytes instead of sending a partial
object. This removes the former 8,192-byte truncation/Serde EOF failure. Some
hosts impose a smaller practical send limit (the current macOS test host
reports `EMSGSIZE` above 9,216 bytes), so snapshot growth beyond that point
requires a separately designed payload-reduction or framing change.
