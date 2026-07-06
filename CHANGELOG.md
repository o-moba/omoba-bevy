# Changelog

All notable changes to this repository should be documented in this file.

The canonical repository version lives in `Cargo.toml` under `[workspace.package].version` and follows SemVer.

## [0.10.0] - 2026-07-06

### Added
- **TASK-22 — matchmaking and gated match start.** The server now has two
  explicit match modes (`OMOBA_MATCH_MODE`): `release` (default) queues
  joining players, forms the match to a full 5v5 roster
  (`2 × OMOBA_TEAM_SIZE`, default 5, clamped 1–16), assigns balanced teams
  server-side (client team choice becomes a preference), runs a 3-second
  countdown, and only then starts the match; `dev` preserves the historical
  instant start on first join for local development. New replicated match
  phases `forming { ready, needed }` and `starting { countdown_ms }`;
  countdown rolls back to forming if a player drops, and an empty queue
  returns to lobby. Joins beyond a full roster are rejected and logged.
- **Client matchmaking UX.** The lobby overlay now walks through the search
  states: "Searching for match...", "Waiting for players — X/10",
  "Match found! Starting in N...". The client adopts the server-assigned
  team on spawn (release-mode balancing) instead of waiting for its
  requested team to be acked.
- **Fill bots for solo testing.** `cargo run -p harness --bin bots`
  (`make bots`, `make play-bots`) joins N dummy UDP clients (round-robin
  classes/avatars, join-resend until acked, ping keepalive, light wander)
  so one developer can fill a 5v5 queue and walk the full matchmaking flow.
- **Makefile/dev-flow split.** `make server` (release), `make server-dev`,
  `make start` (dev quick-start, unchanged UX), `make start-release`,
  `make play-bots`, `make bots BOTS=<n>`, extended `make stop`; RUNBOOK and
  README document modes, env vars, and the solo bot flow, and state
  explicitly that instant start is dev-only.
- **Tests.** 10 new server unit tests (mode parsing, formation gating at
  9/10, countdown + rollback, 5v5 balancing, full-match rejection, dev
  instant start), a client overlay-state test, and a release-mode harness
  integration test (`OMOBA_TEAM_SIZE=1`: solo waits, second player triggers
  countdown → running, teams balance 1v1). Existing harness gameplay tests
  now run the server explicitly in dev mode.

## [0.9.1] - 2026-07-06

### Fixed
- **Characters walked backwards.** Movement code aligned the entity's +Z axis
  with the walk direction, but character models face -Z (Bevy forward), so
  every model rendered 180° from its heading. Local yaw now points -Z along
  the movement direction (`(-dx).atan2(-dz)`); the flipped yaw replicates
  as-is, so remote players match. Raid-boss models (same VRM-staged facing)
  keep the server's +Z yaw convention and get a 180° model-child rotation
  instead.

## [0.9.0] - 2026-07-06

### Added
- **TASK-21 — world-relative character size.** Characters were normalized to
  0.26 world units in a world tuned for `PLAYER_SIZE = 1.0` (46-unit base
  pads, 4-unit jungle blocks, camera at ~19 units) — barely visible.
  `DEFAULT_MODEL_TARGET_HEIGHT` is now 1.15 with range [0.3, 3.0]; persisted
  target heights saved under the legacy scale (below the new minimum) are
  migrated to the new default on load instead of being clamped.
- **TASK-21 — spawn platform traversal (League-style).** The base pad is now
  walkable: `MapLayout::terrain_height(x, z)` describes the pad top and a
  6-unit linear ramp band around it, matching four new visible ramp slabs
  spawned per pad (corner-overlapping, team-colored). Local player gravity,
  the jump-fallback hop, remote players, and minions all ground onto that
  surface client-side; the server keeps its flat ground plane (no protocol
  change). Normalized models now also expose a measured foot offset, so
  character feet rest on the surface instead of the entity origin floating
  at cube half-height (models used to sink ~0.2 into the 0.7-tall pad).

## [0.8.0] - 2026-07-05

### Added
- **TASK-20 — character scale normalization module.** New
  `client/src/model_scale.rs` owns all model-size logic. Every character and
  boss GLB is measured once in bind pose straight from the loaded glTF data
  (node transforms × mesh bounds — independent of animation state or spawn
  timing; raw heights ranged 0.64 m..2.41 m across legacy models, roster
  avatars, and bosses) and its root is rescaled absolutely to the shared
  target height, so all characters render at exactly the same size by
  default. Per-model multipliers live in
  `client/assets/config/model_scale_overrides.json` (slug → multiplier,
  missing = 1.0), hot-reloaded ~1 s while the game runs. A headless analyzer
  mode (`OMOBA_MEASURE_MODELS=1 cargo run -p client`) prints the measured
  height table using the same code path the game uses.

### Fixed
- **Model rescaling no longer compounds.** The old normalization sampled the
  world-space AABB after spawn (timing/animation dependent) and re-applied
  relative factors on top of the already-scaled transform when the target
  height changed. Scales are now always derived from the raw measured height;
  the AABB fallback for primitive stand-ins remembers its first raw
  measurement and stays absolute too.

## [0.7.1] - 2026-07-03

### Fixed
- **Server: pre-join ghost players.** Any packet (including the transport's
  immediate `Ping` heartbeat) used to create a fully joined default player
  (Green, default character) that appeared in everyone's snapshots and could
  start the match. Endpoints are now tracked as `joined = false` until their
  `Join` packet arrives: they still receive snapshots (for addressing) but are
  excluded from the replicated player list and from all gameplay (movement,
  casting, skill upgrades, rematch/god-mode/speed-boost requests, minion and
  tower and neutral targeting, buff regen, and kill-reward splits).
- **Client: lobby overlay blocked the character-select screen.** The
  full-screen "Waiting for match to start..." overlay is now `Pickable::IGNORE`
  and only shows once the local join is committed, so the pre-join class/
  avatar/team select UI stays visible and clickable on a fresh server.
- **Client: team click while disconnected silently lost the join.** Picking a
  team with a dead transport no longer commits the selection and despawns the
  select overlay (which stranded the player); it now triggers the same
  reconnect flow as the Retry button and keeps the select screen up.

## [0.7.0] - 2026-07-03

### Added
- **TASK-19 — raid bosses: epic neutral objectives with team buffs.**
  - **Two raid bosses on the neutrals system**: Wendigo (bottom pit, spawns at
    60 s match time, 900 HP) and King Mutatio (top pit, spawns at 180 s,
    1500 HP), placed at 180°-rotationally-symmetric pits derived with the same
    map formula as the jungle camps. Bosses aggro when attacked, use a larger
    leash (full-HP reset at the pit), and respawn 180 s after death while
    camps keep their 40 s cooldown. All tuning is named constants in
    `server/src/balance.rs`.
  - **Team buffs on boss kill**, replicated via a new additive
    `team_buffs` snapshot field (`serde(default)`): Wendigo's Favor (+15%
    ability damage, 90 s) and Mutatio's Might (+25% ability damage plus
    2 HP/s team regen, 90 s). Re-kills refresh the timer; both buffs combine
    multiplicatively; the server applies the damage multiplier and the regen
    authoritatively. Rematch (`reset_match`) clears buffs and restarts the
    boss spawn schedule.
  - **Client presentation**: bosses render their staged CC0 GLB models
    (`client/assets/bosses/`, staged/retargeted/validated by the existing
    avatar pipeline with a dedicated manifest) scaled to ~3x player height,
    with HP bar, floating nameplate, idle/walk animation driven by the
    replicated AI state, and a match-HUD indicator listing the local team's
    active buffs with remaining seconds. Boss slugs live outside the player
    roster manifest, so they are never selectable as player avatars
    (covered by a shared-crate test).
  - **Tests**: server unit coverage for the spawn schedule, boss stats/pits,
    per-type respawn, buff apply/expiry/refresh/team scoping and authoritative
    buffed damage/regen; harness integration coverage for live boss spawn
    timing and stats over the wire; HUD text unit tests; asset validation for
    the boss directory (`--roster-min/--roster-max`).


## [0.6.0] - 2026-07-03

### Added
- **Ekza Arena avatar sync (`arena-sync` crate).** New workspace tool that
  pulls Avatar cards from the on-chain Ekza Arena registry (raw JSON-RPC
  `getProgramAccounts` + minimal borsh parsing, no anchor client), fetches
  each card's metadata, enforces the model-format classifier (only `vrm` /
  `glb` — what Bevy's glTF loader handles; mirrors the on-chain
  `ProjectProfile "omoba"` in solana-stellar), downloads the model +
  thumbnail into `client/assets/avatars/`, and idempotently merges the
  entries into `manifest.json` under collection "Ekza Arena".
  Usage: `cargo run -p arena-sync -- [--rpc …] [--dry-run]`.

### Changed
- **Avatar roster is now file-first.** `shared::avatar_roster()` reads
  `client/assets/avatars/manifest.json` at runtime (override with
  `OMOBA_AVATAR_MANIFEST`); the compile-time embedded manifest is only the
  fallback. Avatars synced from the chain appear in the team-select grid
  after a client restart — no rebuild. Client and server must share the same
  manifest file, otherwise the server's slug validation rejects runtime-added
  avatars.
- Roster unit test now checks invariants (lower bound, unique slugs,
  non-empty license/source) instead of a hard 10–20 size window.

## [0.5.0] - 2026-07-03

### Added
- **TASK-18 — environment decoration: procedural vegetation from primitives.**
  - New client-only `DecorPlugin` (`client/src/decor.rs`) that dresses the
    arena with stylized low-poly props assembled purely from Bevy primitives
    (Cuboid, Sphere, Cylinder, Cone, Capsule3d): 3 tree variants, 2 bush
    variants, grass tufts, 4 flower variants, and 2 rock variants — no
    external art assets.
  - Deterministic seeded scatter (`generate_layout`, inline splitmix64 PRNG,
    no new dependencies): forest belts along the arena edges, trees/boulders
    ringing the jungle blocks, grass/flowers/bushes across the open meadow.
    Exclusion zones derived from the real map constants keep lanes, base
    pads, towers, neutral camp clearings, the river, and the jungle blocks
    completely clear; covered by unit tests.
  - Purely cosmetic: no collision, no server/shared changes, no networking.
    Fixed layout: 396 props = 970 entities (budget ceiling 1200), spawned
    once at `Startup` under a single `DecorRoot`, reusing 5 shared mesh and
    12 shared material handles so Bevy batches instances.
  - Client-local F4 debug toggle hides/shows the whole decoration layer
    (Visibility flip on `DecorRoot`, logged).
  - `MapLayout` now exposes the lane/river/jungle-block/camp geometry as
    shared methods consumed by both the map renderer and the decor layout,
    so the exclusion math cannot drift from the rendered map.

## [0.4.0] - 2026-07-03

### Added
- **TASK-17 — playable demo: hero classes + VRM avatar roster.**
  - **Four hero classes with distinct Q/W/E/R kits** (Warrior, Mage, Ranger,
    Cleric; 16 distinct ability definitions) defined in the `shared` crate and
    resolved **authoritatively on the server** per player. Kits reuse the
    projectile-damage / self-heal / self-mana-restore primitives with per-class
    numbers; rank mechanics unchanged (max rank 3, `rank_effect_scale`,
    cooldown/range scaling) and slot unlock levels preserved (Q@1/W@2/E@4/R@6,
    now enforced server-side per cast). Casts carry a slot index and cool down
    per slot; skill upgrades cap at the shared max rank.
  - **CC0 avatar roster (16 VRM avatars)** staged as GLB with embedded
    retargeted clips (`idle`/`walk`/`attack`/`cast`/`death`, Quaternius UAL,
    CC0) under `client/assets/avatars/` with a provenance manifest. The
    manifest is embedded in the `shared` crate so client and server agree on
    the shipped set; unknown slugs fall back to the default model (and unknown
    class ids decode as Warrior) without breaking packets.
  - **Pre-join selection flow**: class buttons (name + kit summary), a
    16-avatar thumbnail grid, then team; the join packet carries
    `{team, character, hero_class, avatar, session_id}` and the server
    replicates class + avatar to every client. `OMOBA_AUTOJOIN=<class>:<slug>:<team>`
    joins without UI for automation/evidence runs.
  - **Runtime avatar animation**: roster avatars load lazily, spawn for local
    and remote players, and drive the idle/walk locomotion graph from movement
    state (with a short idle-grace hysteresis so snapshot interpolation does
    not flap the animation). The VRM double-sided material fix now covers the
    whole roster (previously Paco-only).
  - **Class-aware HUD**: the hotbar shows the selected class's ability names
    and per-slot rank; the match HUD lists each slot's ability with effect
    numbers, per-slot cooldown, and lock level.
  - **New tests**: shared kit/unlock/rank/roster unit tests; server tests for
    per-class cast resolution, self-target heals, unlock gating, rank caps, and
    avatar-slug normalization; harness end-to-end scenarios for two clients
    joining with different class+avatar (replication + distinct kit costs),
    locked-slot rejection, and hostile class/avatar values falling back safely.

### Fixed
- Roster thumbnails that were actually JPEG data under a `.png` name are now
  staged with their real extension (Bevy picks the image decoder by extension);
  the client enables the `jpeg` Bevy feature. `scripts/stage_avatars.py` sniffs
  the magic bytes when staging.

### Earlier unreleased work shipped with this release

### Added
- **VRM avatar support + one CC0 humanoid (`Paco`):** the engine now loads VRM 0.x
  avatars through the existing glTF model catalog. VRM 0.x files are glTF 2.0
  binary containers whose VRM-specific data (`VRM`, spring bones, blendshapes) is
  listed under `extensionsUsed` only — never `extensionsRequired` — so Bevy's
  standard `GltfLoader` ignores it and still loads the mesh + skeleton. The
  avatar is staged as `.glb` (a byte-identical glTF 2.0 container) so the asset
  server selects the glTF loader by extension; the validate-and-copy step lives
  in `scripts/convert_vrm_to_glb.py`. Added a new `EkzaCharacter::Paco` variant
  (SDK enum + `ALL` + `BUILTIN_MODEL_MANIFEST` → `downloaded/paco.glb`), so it
  appears in the character-select UI and spawns/normalizes like other models.
  Avatar: *Paco* (Avatar 211) from ToxSam's **100Avatars R3**, **CC0** — see
  `ATTRIBUTION.md`. The avatar ships **no animation clips**, so it renders as a
  static skinned mesh via the existing anim-less fallback (no idle/walk
  locomotion); see the progress note for how to add clips later.
- **Headless gameplay test harness (`harness/` crate, `publish = false`):** a new
  workspace member that spins up the *real* UDP server on a unique loopback port
  per test and drives it with typed bot clients over the JSON wire protocol — no
  GPU, no renderer, no human. Layered into `protocol` (a documented test mirror of
  the server wire format), `server` (`ServerProcess`, RAII: kills the child on
  drop), and `bot` (`Bot`, typed packet senders + freshest-snapshot polling).
  Integration scenarios in `harness/tests/gameplay.rs` assert: join → snapshot at
  full HP; god mode prevents all damage (with a no-god-mode control proving damage
  is detectable); speed boost widens the movement-authority clamp; and an
  `upgrade_skill` with zero points is a server-side no-op. Run via
  `make verify-gameplay` (builds the server first, then `cargo test -p harness
  -- --test-threads=1`). No server/client source was modified.
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
