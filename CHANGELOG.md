# Changelog

All notable changes to this repository should be documented in this file.

The canonical repository version lives in `Cargo.toml` under `[workspace.package].version` and follows SemVer.

## [Unreleased]

### Added
- Jungle neutral camps: three server-simulated camp types (Skirmisher, Bruiser, Spitter) with distinct HP, damage, attack range, and kill rewards; placement mirrors client jungle layout (off-lane).
- Server-authoritative neutral AI (idle, proximity/damage aggro, chase, attack, leash reset, respawn), snapshot sync, and `TargetKind::Neutral` for player casts.
- Client rendering for neutrals (sphere mesh) and HP bars consistent with other units; TAB and middle-click target selection includes neutrals.
- **TASK-03**: Full match lifecycle — `Lobby → Running → Victory → (rematch) → Running` state machine on both server and client.
  - Server starts in `Lobby`; match begins when the first player sends a `Join` packet.
  - Victory state blocks all movement and cast input from clients.
  - Auto-rematch after 10 s or immediately on `RequestRematch` packet; resets structures, minions, projectiles, and players without restarting binaries.
  - Client UI: Lobby overlay ("Waiting for match to start..."), Victory overlay with winner text and rematch countdown, no overlay during Running.
  - Added lifecycle state diagram in `.agent/tasks/TASK-03/spec.md`.
- Synchronized agent workflow guidance for Cursor and Claude, including task reuse, repo task proof loop continuation, and mandatory `git worktree` usage for non-trivial isolated work.
- Repository-level documentation rules for changelog maintenance, feature inventory updates, progress logging, and SemVer-based version handling.
- Initial `docs/features.md` and `docs/progress/` structure for ongoing release tracking.
- Added `docs/agents/README.md` with copy-paste prompts for single-task and parallel task execution.
- Standardized agent prompts and repo-facing coordination docs on English for this international project.

## [0.1.0] - 2026-04-01

### Added
- Initial repository version baseline from `[workspace.package].version`.
