# Changelog

All notable changes to this repository should be documented in this file.

The canonical repository version lives in `Cargo.toml` under `[workspace.package].version` and follows SemVer.

## [Unreleased]

### Added
- TASK-12: centralized server gameplay tuning in `server/src/balance.rs` with `docs/balance-tuning.md`; release gate checklist, manual QA matrix + run log, and release readiness report under `docs/`.
- `scripts/verify_task_12_qa_matrix_live_udp.py` and `make verify-task-12` for recorded two-client UDP join (M1) and cast/mana/damage smoke (M3).
- `PRIMARY_ABILITY_DAMAGE_BY_RANK` / `SKILL_SLOT_COUNT` in `balance.rs` for documented per-rank and four-slot progression hooks.
- Server regression test tying jungle camp templates to `balance` constants; `cast_drains_mana_respects_cooldown_and_blocks_empty_mana` for cast/mana/cooldown invariants.
- Added a reproducible multiplayer session verification harness at `scripts/verify_task_02_multiplayer_session_flow.py` that exercises sequential and simultaneous joins, repeated joins, timeout cleanup, reconnect-as-new-player, server restart recovery, and four-client snapshot consistency against the live UDP server.
- Added focused client coverage for authoritative local-player selection so duplicate local `Player` entities cannot silently break gameplay systems that rely on `Query::single()`.
- Added multiplayer session policy documentation and a `TASK-02` progress log with the recorded session matrix.
- Jungle neutral camps: three server-simulated camp types (Skirmisher, Bruiser, Spitter) with distinct HP, damage, attack range, and kill rewards; placement mirrors client jungle layout (off-lane).
- Server-authoritative neutral AI (idle, proximity/damage aggro, chase, attack, leash reset, respawn), snapshot sync, and `TargetKind::Neutral` for player casts.
- Client rendering for neutrals (sphere mesh) and HP bars consistent with other units; TAB and middle-click target selection includes neutrals.
- Level-based player progression driven by server-authoritative XP thresholds and stat scaling.
- Snapshot propagation of progression fields (`level`, `xp`, `next_level_xp`, `skill_points`) for synchronized client state.
- Local HUD progression readout for level, XP progress, and available skill points.

### Fixed
- Neutral kill XP now uses `grant_player_xp` so jungle rewards level the same way as other XP sources.

### Changed
- Server: allow `clippy::items_after_test_module` on the binary and `clippy::too_many_arguments` on `simulate_projectiles`; allow `clippy::assertions_on_constants` in `balance` unit tests (keeps `-D warnings` clean for `cargo clippy -p server`).

## [0.2.0] - 2026-04-01

### Added
- Implemented TASK-05 player leveling and stat progression, including XP thresholds, level-up scaling for HP/mana, and respawn compatibility with upgraded stats.
- Added progression-oriented server tests covering multi-level XP transitions and respawn behavior after scaling.
- Added client progression ingestion and HUD presentation of progression state.
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
- Added project-scoped Cursor subagents in `.cursor/agents/` (`verifier`, `code-reviewer`, `search-agent`, `reasoning-agent`) to improve verification, review, search, and architecture support workflows.

## [0.1.0] - 2026-04-01

### Added
- Initial repository version baseline from `[workspace.package].version`.
