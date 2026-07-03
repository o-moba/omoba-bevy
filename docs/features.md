# Feature Inventory

Canonical version: `0.5.0`

## Current Playable Surface

- **Environment decoration (TASK-18):** the arena is dressed with stylized
  low-poly vegetation and props assembled purely from Bevy mesh primitives —
  3 tree variants (oak/pine/birch), 2 bush variants, grass tufts, 4 flower
  variants (white/yellow/red/violet), and 2 rock variants — placed by a
  deterministic seeded scatter (`client/src/decor.rs`, inline splitmix64
  PRNG, no external assets or new dependencies). Forest belts hug the arena
  edges, trees and boulders ring the jungle blocks, and grass/flowers fill
  the open meadow, while exclusion zones derived from the real map constants
  keep lanes, base pads, towers, neutral camp clearings, the river, and the
  jungle blocks clear. Purely cosmetic and client-side: no collision, no
  server or networking changes. Fixed budget: 396 props = 970 entities
  (ceiling 1200) spawned once at `Startup` under a single `DecorRoot`,
  reusing 5 shared mesh and 12 shared material handles for batching. **F4**
  toggles decoration visibility (client-local debug toggle, logged).
- **Hero classes (TASK-17):** four playable classes — Warrior, Mage, Ranger,
  Cleric — each with a distinct Q/W/E/R kit (16 ability definitions in the
  `shared` crate; projectile damage, self-heal, and self-mana-restore
  primitives with per-class numbers). The server resolves the kit
  authoritatively per player: per-slot cooldowns, unlock gating by level
  (Q@1/W@2/E@4/R@6), rank scaling up to max rank 3, and skill upgrades capped
  at the shared max rank. Class selection happens on the pre-join screen and
  is carried in the join packet.
- **CC0 VRM avatar roster (TASK-17):** 16 CC0 avatars (Open Source Avatars
  collections) staged as GLB under `client/assets/avatars/` with embedded
  retargeted animation clips (`idle`/`walk`/`attack`/`cast`/`death` from the
  Quaternius Universal Animation Library, CC0) and a provenance manifest
  (slug, name, collection, license, source URL, author, thumbnail). Avatar
  selection is a thumbnail grid on the pre-join screen; the chosen slug
  replicates to all clients, models load lazily, and every roster avatar
  plays idle when stationary and walk while moving (idle-grace hysteresis
  smooths snapshot interpolation). Unknown avatar slugs and class ids fall
  back safely (default model / Warrior). `OMOBA_AUTOJOIN=<class>:<slug>:<team>`
  joins without UI for automation.
- **Raid bosses with team buffs (TASK-19):** two epic neutral objectives built
  on the jungle-neutral system — **Wendigo** (bottom river/jungle pit, spawns
  at 60 s match time, 900 HP) and **King Mutatio** (top jungle pit, spawns at
  180 s, 1500 HP), in 180°-symmetric pits derived from the map formula. Bosses
  aggro when attacked, leash back to their pit at full HP, and respawn 180 s
  after death (camps keep 40 s). Killing a boss grants the killer's whole team
  a replicated timed buff: Wendigo's Favor (+15% ability damage, 90 s) or
  Mutatio's Might (+25% ability damage +2 HP/s regen, 90 s); a re-kill
  refreshes, both buffs stack multiplicatively, and the server applies the
  damage multiplier and regen authoritatively. The client renders each boss
  with its staged CC0 model (`client/assets/bosses/`, own manifest — boss
  slugs are never player-selectable) scaled to raid presence, with HP bar,
  floating nameplate, idle/walk animation from the replicated AI state, and a
  match-HUD indicator showing the local team's active buffs with remaining
  seconds. All tuning lives in named constants in `server/src/balance.rs`.

- Headless Bevy-scheduled authoritative UDP loop with periodic player snapshots; player mana regeneration and `projectile -> minion` damage now run through ECS/message-driven systems bridged to the current authoritative state maps.
- Server-authoritative hardening for player movement and casts: client transforms are speed/map clamped, non-finite positions are ignored, and casts require the authoritative caster position to be in range of the live target.
- Local multiplayer flow with server startup plus multi-client local play via `make start`.
- Team join flow with character selection and player spawning.
- Ekza Bevy SDK extraction: shared sibling `ekza-bevy-sdk` repository owns stable character ids, built-in 3D model manifest metadata, GLB validation, and Bevy model catalog loading for future dependency publishing.
- VRM avatar support: VRM 0.x avatars (glTF 2.0 binary with extra `VRM`/spring-bone/blendshape extensions) load through the existing glTF model catalog by staging them as `.glb` (the VRM extensions are `extensionsUsed`-only, so Bevy's loader ignores them and keeps the mesh + skeleton). Ships one selectable CC0 humanoid, `Paco` (ToxSam 100Avatars R3); see `ATTRIBUTION.md` and `scripts/convert_vrm_to_glb.py`. The avatar carries no animation clips, so it renders as a static skinned mesh via `NormalizeModelScale` like other models.
- Core combat loop with projectiles, structures, minions, death, respawn, mana regeneration, and base-destruction win condition.
- Map layout with three lanes and simple jungle blocks.
- Player progression with level-based XP thresholds, HP/mana scaling on level-up, and tracked skill points.
- In-game local HUD display for level and XP progression.
- Persistent local client preferences (graphics, character, optional server address, stable client session id) with safe clamping on load; override directory with `OMOBA_CLIENT_CONFIG_DIR` for tests or portable installs.
- In-game match HUD (below minimap): level, XP, skill points, upgrade key label (`U`), local HP/mana, target hints, objective line, per-slot class ability lines (name, effect numbers, cooldown/lock state), and F1 help reminder; bottom-right skill bar shows `Q`–`R` keys with the selected class's ability names and ranks.
- F1 toggle help overlay with movement, camera, targeting, casting, objective, and pause guidance; does not reset simulation when toggled. The panel is shown only while the match is `Running` (toggle state is preserved when returning to a live match so lobby/victory screens are not covered).

## Multiplayer Session Reliability

- **TASK-14 (client)**: Explicit session states, non-blocking wait when the server is down, bounded `WaitingForServer` timeout, stale snapshot detection while connected, UDP transport error thresholds, snapshot-channel disconnect detection when the UDP thread ends, full teardown (replicated entities + team overlay) on disconnect, manual reconnect via **Retry** (no silent rejoin into a match), pause menu auto-closes on **Disconnected**, minimap hidden unless **Connected**.
- Named timing constants and failure-detection summary: `docs/network-client-session.md` and `client/src/session_config.rs`.
- Join is authoritative on the server: the client may optimistically pick a team and character, but the snapshot for `your_id` is the source of truth for spawn side, team, and character.
- Repeated `Join` packets from the same UDP endpoint are deterministic: the last processed `Join` wins for team, character, spawn position, HP, mana, gold, and XP reset.
- If a client stops sending packets, the server removes that player after `PLAYER_TIMEOUT = 5s`; remaining clients stop receiving that player in snapshots after the timeout expires.
- Reconnect policy for this version: clients that send a valid stable `client_session_id` in `Join` can reclaim a timed-out player slot/id for a short server-side window. Legacy clients without a session id still use endpoint identity and reconnect as a new player.
- If the server restarts while clients stay open, the next packet from an existing client creates a fresh default session on the restarted server. Team and character return to defaults until that client sends `Join` again.
- The client applies only the latest queued snapshot per frame. This "last snapshot wins for the current frame" behavior is intentional for now and validated by the session-flow checks.

## Release Gaps Tracked In Tasks

- Runtime and startup stability hardening.
- Account-backed identity, cryptographic session authentication, and long-lived reconnect across server restarts.
- Publishable SDK packaging: registry metadata, versioning policy, examples, entitlement/auth hooks, and non-blocking asset delivery are still future work.
- Full reconnect slot reclaim across disconnects and NAT changes.
- Ability VFX/animation sync for attack/cast/death clips (embedded in every roster avatar, not yet gameplay-triggered), richer tooltip UX, and balance passes over the class kits.

## Release gate and balance (TASK-12)

- Authoritative tuning constants: `server/src/balance.rs` (see `docs/balance-tuning.md`).
- Release checklist, manual QA matrix, and readiness report: `docs/release-gate-checklist.md`, `docs/manual-qa-matrix.md`, `docs/release-readiness-report.md`.
- Live UDP QA smoke (two clients + cast): `make verify-task-12` or `python3 scripts/verify_task_12_qa_matrix_live_udp.py` (after `cargo build -p server`).
- Headless gameplay rule harness (typed Rust, no GPU/human): `make verify-gameplay` boots the real server on a per-test port and drives it with bot clients to assert god mode, the movement-authority clamp, and skill-point gating (`harness/` crate; see `docs/progress/2026-06-28-headless-gameplay-harness.md`).
- Expanded skill roster, tooltip UX, balance passes, and release-scale QA beyond the current cast-and-HUD surface.

## Operations and playtest documentation

- [README.md](../README.md) — setup, controls summary, links to tester docs.
- [RUNBOOK.md](../RUNBOOK.md) — startup, env vars, troubleshooting with recovery steps.
- [docs/playtest-script.md](playtest-script.md) — timeboxed MVP session checklist.
- [docs/bug-report-template.md](bug-report-template.md) — internal report format.
- [docs/mvp-scope-and-limitations.md](mvp-scope-and-limitations.md) — explicit MVP scope and limitations.
- [tasks/MVP-CHECKLIST.md](../tasks/MVP-CHECKLIST.md) — MVP-blocking vs deferrable classification.
