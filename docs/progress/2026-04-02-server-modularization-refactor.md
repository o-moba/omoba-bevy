# 2026-04-02 — Server modularization refactor

## Goal

Reduce `server/src/main.rs` complexity by moving logically grouped helper code into dedicated modules without changing gameplay behavior.

## Changes

- Added new server modules:
  - `server/src/progression.rs`
  - `server/src/neutrals.rs`
  - `server/src/world.rs`
  - `server/src/session.rs`
- Kept `main.rs` as runtime orchestration and simulation loop host, while reusing extracted helpers via module imports.
- Moved function groups by responsibility:
  - progression / leveling helpers
  - neutral camp templates + spawn builders
  - map + structure + lane path + wave spawn helpers
  - player connection/join/mana/respawn/rematch session helpers
- No protocol or balancing constant changes; this is a structural refactor.

## Verification

- `cargo check -p server` — PASS
- `cargo test -p server` — PASS
- `cargo check --workspace` — PASS
- `cargo fmt --all` — PASS

## Notes

- Existing untracked progress docs from earlier sessions were intentionally left untouched.
