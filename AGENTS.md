# AGENTS.md

## Cursor Cloud specific instructions

### Overview

**omoba-bevy** is a multiplayer MOBA-style game built with the Bevy engine (Rust). The Cargo workspace has three crates:

| Crate | Purpose |
|---|---|
| `shared` | Shared types (abilities, skill slots) used by both client and server |
| `server` | Authoritative UDP game server (binds `0.0.0.0:4000` by default) |
| `client` | Bevy 3D game client (connects to server over UDP) |

No external services (databases, Docker, etc.) are required. All server state is in-memory.

### System dependencies (pre-installed in snapshot)

The following apt packages are required for building (especially for Bevy/OpenSSL):

```
libssl-dev pkg-config libudev-dev libasound2-dev libxkbcommon-dev libwayland-dev libvulkan-dev
```

Rust edition 2024 requires **Rust 1.85+**. The snapshot has `stable` (1.94+) set as the default toolchain via `rustup`.

### Build / Test / Run

See `RUNBOOK.md` for full local development instructions. Quick reference:

| Action | Command |
|---|---|
| Build workspace | `cargo build --workspace` |
| Test (all) | `cargo test --workspace` |
| Lint (clippy) | `cargo clippy --workspace` |
| Format check | `cargo fmt --check` |
| Start server only | `make server` (or `cargo run -p server`) |
| Start one client | `make game` (or `cargo run -p client`) |
| Start server + 2 clients | `make start` |
| Stop all processes | `make stop` |
| Python test harness | `python3 scripts/verify_task_02_multiplayer_session_flow.py` (needs running server on port 4010) |

### Known issue: main branch compilation errors

As of the latest main merge, `server` and `client` crates have code-level compilation errors from bad merges:

- **server**: duplicate function definitions (`handle_respawns`, `reset_match`), missing constants (`MAX_MELEE_SKILL_RANK`, `MELEE_BASE_DAMAGE`, etc.), wrong field names (`pending_ability_feedback` vs `pending_skill_feedback`, `last_ranged_shot_at`), and a missing function (`try_ranged_shot_ability`).
- **client**: syntax error in `client/src/net.rs` line 978 (`))` instead of `})`).

The `shared` crate compiles and passes all tests cleanly. These are code issues, not environment issues.

### Environment variables

| Variable | Default | Description |
|---|---|---|
| `SERVER_ADDR` | `0.0.0.0:4000` | Address the server binds to |
| `GAME_SERVER_ADDR` | `127.0.0.1:4000` | Server address clients connect to |

### Client display requirement

The Bevy client requires a GPU/display. On headless Cloud Agent VMs, use `Xvfb` or similar virtual framebuffer to run the client. The server runs headlessly without issues.
