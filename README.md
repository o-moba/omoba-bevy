# omoba-bevy

MOBA-style native 3D beta (Bevy client, authoritative Rust UDP server).

For the September 8 controlled beta, use the
[tester/host guide](docs/progress/2026-09-07-beta-test-guide.md): the native
package includes `practice.sh`, `host.sh --humans N` and
`join-server.sh HOST:PORT`. See the
[current forest navigation iteration](docs/progress/2026-09-08-forest-navigation.md) and
[match readiness record](docs/progress/2026-09-07-beta-readiness.md) for measured
checks and remaining coverage. The commands below are development workflows.

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

The [2026-09-07 beta guide](docs/progress/2026-09-07-beta-test-guide.md) is the
current tester entry point. Earlier audits and MVP documents below retain
historical context; the [dated beta readiness record](docs/progress/2026-09-07-beta-readiness.md)
states the current delivery boundary.

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
| **P / Shop** | Browse items; buy while alive at your own base; Escape closes the shop |
| **Y** | Toggle hero follow/free camera; a minimap focus returns directly to the hero |
| **Space** | Restore hero follow and clear a minimap focus override |
| **Alt + right mouse** (3D only) | Hold to orbit the camera |
| Team / character UI | Click team and character before play; server snapshot is authoritative |
| **Right-click ground / minimap** | Set a route around solid forest and structures, shown on the minimap; another order replaces it |
| **Left-click hostile** | Select the exact target without moving or casting |
| **Left-click/drag minimap** | Pan the camera without issuing movement |
| **Right-click hostile** | Approach through normal navigation and repeat basic attacks |
| **Tab** | Select nearest enemy target |
| **S / Backspace** | Stop movement and attacks, and clear target selection |
| **Q / W / E / R** | Cast that class ability at the selected target |
| Phone **ATTACK** | Tap for a basic attack; stationary hold repeats without mana cost |
| Phone **drag ATTACK** | Extend reticle, preview a foe, release to lock and attack once; drag to X to cancel |
| On-screen **Q/W/E/R** | Separate abilities around ATTACK; tap uses locked target, drag aims a skill |
| **Esc** | Close Help/Shop first, otherwise open the pause menu |

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
