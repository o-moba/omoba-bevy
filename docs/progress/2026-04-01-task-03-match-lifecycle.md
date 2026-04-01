# TASK-03: Match Lifecycle and Core Loop

**Date:** 2026-04-01  
**Branch:** `feature/task-03-match-lifecycle`  
**Status:** PASS (all 5 AC)

## Goal

Complete the full OMOBA lifecycle: Lobby → Running → Victory → rematch reset.

## Changes

### Server (`server/src/main.rs`)
- Added `GameState::Lobby` as the new default start state (was `Running`)
- Added `ClientPacket::RequestRematch` — client can trigger immediate rematch
- `Transform` packets now blocked outside `Running` state (movement was previously always accepted)
- Lobby → Running transition fires when first `Join` packet is received
- `VICTORY_REMATCH_DELAY = 10s` auto-resets the match after Victory
- `reset_match()` function: restores all structure HP, clears minions/projectiles, resets wave timer, teleports players to spawn with full HP/mana
- `ServerPacket::Snapshot` gains `rematch_in_secs: Option<u64>` — countdown broadcast to clients

### Client (`client/src/net.rs`)
- `GameState::Lobby` variant added (matches server)
- `NetworkCommand::RequestRematch` added for future UI trigger
- `GameStateSnapshot` resource gains `rematch_in_secs: Option<u64>`

### Client (`client/src/game_state.rs`)
- Lobby overlay: dark blue, "Waiting for match to start..."
- Victory overlay: unchanged winner text + "\nRematch in Xs..." countdown line
- Running: no overlay (unchanged)

## Checks

| Check | Result |
|-------|--------|
| `cargo build -p server -p client` | 0 errors |
| `cargo clippy -p server -- -D warnings` | 0 warnings |
| `cargo test -p server` | 4/4 pass |

## Remaining risks

- Client clippy has 70+ pre-existing warnings in untouched files (camera.rs, combat.rs, etc.) — not introduced by this task, tracked separately.
- `RequestRematch` is wired server-side and in `NetworkCommand` but no UI button yet triggers it — the auto-10s rematch is the primary path for now.
