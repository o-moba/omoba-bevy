# Changelog

All notable changes to this repository should be documented in this file.

The canonical repository version lives in `Cargo.toml` under `[workspace.package].version` and follows SemVer.

## [Unreleased]

### Added
- **TASK03 — upgradable skills:** the primary ability (Q) now scales with an
  authoritative per-slot rank. Server tracks `ranks: [u8;4]` in `PlayerState`,
  handles a new `UpgradeSkill { slot }` packet (spends one skill point, capped at
  `MAX_SKILL_RANK`), and the projectile damage is `PRIMARY_ABILITY_DAMAGE_BY_RANK`
  by Q rank (now an increasing table 20→52). Client shows each slot's `Lv N` and an
  upgrade ↑ button (lit when a point is spendable); the `U` key upgrades Q.
- **TASK04 — God Mode (debug):** a left-side toggle button makes the local player
  invulnerable for gameplay debugging. Authoritative: `ClientPacket::SetGodMode`
  sets a server-side `god_mode` flag that skips all player damage (projectile,
  neutral, and minion attacks). Not networked back; the requesting client owns it.

### Added
- **TASK05 — Debug Speed Boost:** a button next to God Mode (bottom-left) toggles an
  authoritative movement multiplier (`DEBUG_SPEED_MULTIPLIER`). The server widens the
  movement-authority clamp so the boosted client is not rubber-banded; the client
  moves faster locally. Re-asserted on (re)connect.

### Fixed
- Minions now always prioritize enemy minions over players: while any enemy minion
  is within vision a minion never targets a player (overrides sticky player aggro);
  players are only chosen when no enemy minion is in range. Covered by
  `minion_prefers_enemy_minion_over_closer_player`.
- Debug toggles (God Mode / Speed Boost) now take effect reliably: a single
  edge-triggered send could be lost (UDP, connection races, fresh server session),
  leaving the server flag unset — so god mode "did nothing" and speed boost
  rubber-banded. The client now re-asserts the current toggle state to the server
  ~2x/sec (idempotent; server logs only on change), and the snapshot reconcile
  widens its local snap threshold while boosting so the boosted player is not
  snapped back. Verified end-to-end against a live server (`set_god_mode` /
  `set_speed_boost` received and applied).
- Debug toggles can now be driven by keyboard (**F2** god mode, **F3** speed boost)
  as a reliable fallback if the on-screen buttons do not receive clicks; toggling
  logs `[debug] god_mode/speed_boost -> <bool>` client-side, and the server logs
  receipt, to diagnose the command path end to end.
- God Mode now reliably keeps the player alive: besides skipping damage at every
  player-damage site, the server restores god-mode players to full HP **and full
  mana** each tick (after damage, before respawn) and on toggle, so no missed path
  can kill them and abilities can be cast freely.
- Stopped per-frame `"idle/walk animations were not found"` log spam for models
  without locomotion clips: the animation library now skips a GLTF once evaluated
  (gating on `evaluated_characters`) instead of re-checking every frame.

### Changed
- Skill upgrade arrows now appear **only when a point can actually be spent** on that
  slot (hidden otherwise) instead of always showing dimmed.
- God Mode debug button moved to the bottom-left, on the same line as the skill bar,
  and is re-asserted to the server after a (re)connect (the server resets the flag for
  a fresh session).
- First level is reachable in ~3 minion kills (`LEVEL_XP_THRESHOLDS[0]` 120 → 90) so the
  skill-upgrade flow is easy to exercise during playtests.

### Fixed
- Target selection (`Tab` nearest-enemy and middle-click) no longer picks **friendly minions**: minion candidates now skip same-team units like players and structures do, so the enemy base tower can be selected near friendly minion waves (`client/src/combat.rs`).
- Head HP/Mana bar for player models is now anchored to a deterministic normalized head height (`NormalizeModelScale.head_local_y`) instead of unstable per-frame AABB sampling, so the bar sits above the head for every character regardless of GLB pivot (previously drifted to mid-body for `wang`/`toka`).

### Added
- **UI/UX iteration:** team-select buttons now show their `Green`/`Blue` labels (previously bare colored squares) plus a flow hint ("Pick a character, then a team to join the match."); the in-match HUD gains color-coded HP and Mana bars (HP tints green/amber/red by ratio) shown only while the match is `Running`.
- Production authority hardening: stable optional client session ids in join packets, client-side persistence for the id, and server-side reclaim of timed-out player slots from a new UDP endpoint.
- Standalone sibling `ekza-bevy-sdk` repository with stable Ekza-Stellar character ids, built-in 3D model manifest metadata, GLB validation helpers, and a Bevy-gated model catalog loader for local/remote GLB assets.
- SDK model validation module and examples: typed GLB validation reports, configurable rules, issue enums, a `model_check` CLI example, a headless `model_cache` built-in source verifier, and an interactive Bevy `model_viewer` viewport.
- TASK-12: centralized server gameplay tuning in `server/src/balance.rs` with `docs/balance-tuning.md`; release gate checklist, manual QA matrix + run log, and release readiness report under `docs/`.
- `scripts/verify_task_12_qa_matrix_live_udp.py` and `make verify-task-12` for recorded two-client UDP join (M1) and cast/mana/damage smoke (M3).
- `PRIMARY_ABILITY_DAMAGE_BY_RANK` / `SKILL_SLOT_COUNT` in `balance.rs` for documented per-rank and four-slot progression hooks.
- Server regression test tying jungle camp templates to `balance` constants; `cast_drains_mana_respects_cooldown_and_blocks_empty_mana` for cast/mana/cooldown invariants.
- **TASK-13 (client):** Match HUD column (level, XP, skill points, upgrade key hint, HP/mana, target summary, objective line, F1 reminder); bottom skill bar with four labeled slots (`Q`–`R`) wired to the same cast action until the server exposes distinct skills; centralized key labels in `input_bindings`; F1 help overlay with control and objective copy; clearer victory/defeat next-step text; help panel only renders during `Running` so lobby/victory overlays stay readable on reconnect/snapshot transitions; bracketed skill slot line and display strings derived from `SKILL_SLOT_KEY_LABELS`; unit tests for binding/display consistency.
- **TASK-14**: Client connection lifecycle (`Connecting` / `WaitingForServer` / `Connected` / `Disconnected`), bounded wait and stale-snapshot handling with named thresholds in `session_config`, transport failure signaling from the UDP thread, teardown that clears replicated entities and re-opens team select, manual **Retry** after disconnect, ingest/apply snapshot pipeline ordering for Bevy 0.18, snapshot-channel disconnect detection (`NetIncomingDisconnected`), pause menu closes on **Disconnected**, minimap visible only while **Connected**, preferences save when the resolved server address resource updates, settings panel shows current server address text, and `docs/network-client-session.md` for constants and manual QA cross-reference.
- Client preferences file: graphics (lighting, model scale), character selection, and optional `game_server_addr` with load precedence `GAME_SERVER_ADDR` → saved file → default; pause menu **Reset graphics to defaults**; paths documented in `client/src/persistence.rs` and `RUNBOOK.md`.
- **TASK-15:** Playtest and ops documentation — root `README.md`, `docs/playtest-script.md` (10–20 minute session), `docs/bug-report-template.md`, `docs/mvp-scope-and-limitations.md`, `tasks/MVP-CHECKLIST.md` (MVP vs deferrable), expanded `RUNBOOK.md` troubleshooting with recovery steps, cross-links from `docs/features.md`, and `.gitignore` whitelists so these paths stay versioned (previously blanket-ignored).
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
- Server movement/cast authority: transform packets are clamped by server speed/time/map bounds instead of being trusted as teleports, and cast requests now require authoritative range checks against live player, minion, structure, and neutral target positions.
- Client/server character identity now uses the shared `ekza-bevy-sdk::EkzaCharacter` type while preserving existing snake_case packet values.
- Server: allow `clippy::items_after_test_module` on the binary and `clippy::too_many_arguments` on `simulate_projectiles`; allow `clippy::assertions_on_constants` in `balance` unit tests (keeps `-D warnings` clean for `cargo clippy -p server`).
- Server refactor: split monolithic `server/src/main.rs` logic into focused modules (`progression`, `neutrals`, `world`, `session`) while preserving runtime behavior and test coverage.
- Server runtime loop now runs under a headless Bevy `App` + `ScheduleRunnerPlugin`; mana regeneration was moved to ECS (`Player`/`Health`/`Mana` components and systems) with a sync bridge to the existing authoritative state maps.
- Server ECS combat slice: introduced `server/src/gameplay/combat.rs` and `GameplayPlugin` for message-driven `projectile -> minion` collision/damage resolution (`DamageEvent`), with minion ECS mirroring and legacy minion-hit handling removed from `simulate_projectiles`.

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
