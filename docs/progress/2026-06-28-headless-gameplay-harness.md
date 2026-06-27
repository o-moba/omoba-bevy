# 2026-06-28 — Headless gameplay test harness

## Goal

Add a clean, typed Rust harness that boots the **real** authoritative UDP server
and drives it with bot clients to assert gameplay rules — with no GPU, no
renderer, and no human. This supersedes the ad-hoc Python flow in
`scripts/verify_task_02_multiplayer_session_flow.py` for Rust-typed
gameplay-rule coverage.

## Changes

New workspace member crate `harness/` (`publish = false`, dev/test tool only).
The `server` and `client` crates were **not modified** — server regression risk
stays at zero. The change set is purely additive: the new crate, the `members`
line in the root `Cargo.toml` (+ `Cargo.lock`), a `Makefile` target, this doc,
and a CHANGELOG entry.

Crate layout:

- `harness/src/protocol.rs` — a documented **test mirror** of the server's
  UDP/JSON wire format (`ClientPacket`, `ServerPacket::Snapshot`, `PlayerState`,
  `Team`, `TargetId`/`TargetKind`). Only the fields the harness asserts on are
  modeled; `#[serde(default)]` keeps it tolerant of extra/missing fields. Source
  of truth: `server/src/main.rs` and `server/src/balance.rs`.
- `harness/src/server.rs` — `ServerProcess`: reserves a free loopback port,
  launches the server with `SERVER_ADDR=127.0.0.1:<port>`, waits for the
  `is listening` line on stdout, and **kills the child on `Drop`** (RAII).
  Resolution order for the binary: `HARNESS_SERVER_BIN` → prebuilt
  `target/debug/server` → `cargo run -q -p server`.
- `harness/src/bot.rs` — `Bot`: a connected `UdpSocket` with typed helpers
  (`join`, `send_transform`, `cast`/`cast_player`, `upgrade_skill`,
  `set_god_mode`, `set_speed_boost`, `ping`) and snapshot polling
  (`recv_snapshot`, `wait_for_player`, `latest_player`). `recv_snapshot` drains
  the socket to the **freshest** snapshot so assertions never read stale, queued
  state (the server emits a snapshot every ~50ms).
- `harness/src/lib.rs` — re-exports and crate docs (purpose + how to add a
  scenario).
- `harness/tests/gameplay.rs` — the integration scenarios.

## Scenarios and what they assert

1. `join_produces_snapshot_with_player` — a bot joins green/ipfs and appears in a
   snapshot at full HP on the expected team.
2. `god_mode_keeps_player_alive_under_damage` — two enemy bots (green vs blue)
   walk into cast range (driven toward the origin with the debug speed boost,
   converging via the server's own movement clamp). With god mode **on** the
   attacker casts for ~2s and the victim's HP holds at max; a **control** phase
   with god mode **off** then drives the same casts and asserts HP actually
   drops — proving the harness can detect damage, so the god-mode assertion is
   meaningful.
3. `speed_boost_widens_movement_clamp` — two same-team bots run the identical
   forward-drive loop; only the speed-boost flag differs. The unboosted run is
   clamped far short of the requested step (~10u) while the boosted run reaches
   meaningfully further (~25u, ≈2.6× per `DEBUG_SPEED_MULTIPLIER`). Asserts
   boosted > unboosted × 1.5.
4. `upgrade_skill_consumes_point_and_caps` — a fresh player has 0 skill points
   and all ranks at 1; an `upgrade_skill(0)` with no points is a server-side
   no-op (rank stays 1, points stay 0). A richer "spend a real point → rank rises
   → projectile hits harder" check is left to a server unit test (see the
   skill-rank tests in `server/src/main.rs`).

## How to run

```
make verify-gameplay
# == cargo build -p server && cargo test -p harness -- --test-threads=1
```

Sequential (`--test-threads=1`) keeps multiple spawned servers from contending
during a cold build; each test already uses a unique port so parallel runs are
also safe.

## Checks

- `cargo build --workspace` — passes.
- `cargo test -p harness -- --test-threads=1` — 4 passed (run twice, stable),
  ~12s wall time.
- `cargo clippy -p harness --tests` — clean.
- `cargo test -p server` — 25 passed; `cargo test -p client` — 25 passed.
- `git status` confirms no `server/` or `client/` source files were touched.

## What this can't cover (deliberate)

- Rendering, head-bar/HUD visuals, and any pixel-level correctness — needs a
  human or a screenshot pass.
- UI interactions (button clicks, key bindings, team-select flow) — the harness
  speaks the wire protocol, not the client UI.
- Long-form leveling / rank-damage scaling — better as a fast server unit test
  than a multi-minute integration scenario.

## Remaining risks

- The free-port reservation (bind `:0`, release, hand the port to the server)
  has a tiny TOCTOU window; in practice it is reliable and each test still gets a
  distinct port.
- Timing-based scenarios (speed boost, walking into range) use generous
  thresholds and timeouts rather than exact values, so they tolerate CI jitter.
