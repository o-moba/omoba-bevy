# Feature Inventory

Canonical version: `0.18.0-rc.2`

## Current Playable Surface

- **Verdant Confluence native arena (2026-09-05, iteration 03):** the supplied
  Blender scene now supplies the 3D environment, foliage and live faction
  structures. Walk surfaces match actor grounding, structures follow death and
  rematch, and cached original creatures replace the disputed imported models.
  The 15-hero roster and King Mutatio remain. A content-based package gate
  checks model bytes and provenance; an opt-in native capture scenario records
  the actual renderer. See the [integration record](progress/2026-09-05-verdant-runtime-release.md)
  and [current test guide](progress/2026-09-05-verdant-test-guide.md). This
  supersedes earlier roster, primitive-map and four-asset-blocker descriptions.

- **Native 3D release candidate (2026-09-05, iteration 02):** idempotent admission,
  full-state reconnect, complete rematches, release roster/debug policy, bounded
  framed UDP and ordered snapshots address the lifecycle/network audit findings.
  Shared input/modal rules, intentional orbit, scrollable roster, visible hotbar
  feedback, existing 3D action/death clips and non-color allegiance cues address
  the input/presentation findings. Offline mode-scoped loading and the native
  package remove developer-local runtime requirements. Protected bases, scenario
  bot upgrades/sustain/tower support and match metrics support controlled sessions.
  See the [current ledger](progress/2026-09-05-release-preparation.md) and
  [package/test guide](progress/2026-09-05-release-test-guide.md). External release
  gates remain blocked until actual evidence exists; the entries below describe
  historical feature delivery and are superseded where the ledger says so.

- **3D delivery and readiness audit (2026-09-05):** the locked SDK dependency,
  two offline slime minion models and 16-avatar shipped roster now survive a
  clean checkout. The [current assessment](progress/2026-09-05-3d-readiness-audit.md)
  records remaining reconnect/rematch, release debug-command, input and combat
  readability blockers. These findings qualify earlier feature descriptions;
  existing features are not a claim of unattended playtest readiness.

- **Pointer-first desktop/mobile combat (TASK-POINTER-COMBAT-MOBILE-01):** a
  primary click or touch tap resolves living hostile actors by their projected
  screen position with 48–68 logical-pixel hit radii, selects the exact
  authoritative `TargetId`, shows the existing marker, and requests Q. Empty
  ground moves on both input types; target, minimap, and UI presses are
  consumed before movement. Keyboard and 64-pixel hotbar buttons share one
  pending-cast path. An out-of-range unit cast follows the moving target until
  it enters the shared scaled range, emits once, and only then starts the local
  cooldown. Manual movement or invalidation cancels the pending request.
- **Stable 2D controls and proportional nameplates
  (TASK-2D-CONTROLS-FOLLOW-01):** click-to-move remains active independently
  of camera follow, `Y` toggles follow and clears a minimap focus when returning
  to the hero, and 2D right-click/Alt no longer capture the cursor or silently
  disable movement input. Hero labels are bounded by hero height and estimated
  text width, including the long Orchard Comet Centaur display name. Legacy 3D
  camera controls remain available.
- **2D production-readiness pass (TASK-2D-PRODUCTION-READINESS-01):** the
  orthographic camera now starts twice as close, and actor sizes are validated
  from occupied alpha pixels rather than transparent atlas cells. The six lane
  towers are one-to-one with authoritative owners and carry Green-square or
  Blue-diamond badges plus TOP/MID/BOT labels. The initial 18-minion wave uses
  larger state-aware sprites and the same non-color team language; owner
  removal recursively cleans its bounded visual cues. A headless ECS fixture
  proves exactly 24 primary proxies for six towers plus 18 minions, then 23
  after one minion disappears, with no duplicates on replay.
- **Complete UDP datagrams above 8 KiB:** client/harness receive storage is
  65,536 bytes and the server validates the whole serialized snapshot against
  the 65,507-byte IPv4 UDP payload ceiling before sending. Boundary tests cover
  8,191/8,192/8,193 bytes, malformed traffic, near-limit decode, and whole
  over-limit rejection. A real release 5v5 server test receives a complete,
  runtime-dependent snapshot satisfying `8192 < bytes <= 65507`, with 10
  players, 8 structures, and 18 minions. The measured macOS kernel send ceiling
  remains 9,216 bytes; current protocol-1 clients now use ≤1200-byte framed datagrams; this
  legacy whole-JSON path remains for compatibility scripts.

- **Genuine full-2D world (TASK-FULL-2D-WORLD):** `sprite2d` is now a true
  orthographic XY renderer rather than a billboard layer over the 3D arena.
  A centralized tested projection maps authoritative simulation XZ to render
  XY (`x → x`, `z → y`) and back for cursor picking. One `Camera2d` supports
  hero follow, bounded arrow-key free pan, clamped wheel zoom, resize-safe map
  edges, and minimap focus/recenter. `models3d` remains selectable and starts
  independently with its existing perspective scene.
- The deterministic 55×55 tiled map reproduces the authoritative three lanes,
  two bases, six towers, two base objectives, diagonal traversable river,
  three camps, and two boss pits. Original CC0 Higgsfield/Recraft terrain and
  prop atlases are declared in `client/assets/world2d/manifest.json`; forest
  and water visuals do not invent client-only collision. Static tiles plus
  props remain below 4,096 entities, transient VFX are capped at 256 and
  normally expire within two seconds, and cached atlas handles avoid per-frame
  asset creation. Validate with
  `python3 scripts/validate_world2d_assets.py --self-test`.
- Heroes, structures, team minions, normal neutrals, Wendigo, King Mutatio,
  projectiles, selection markers, health/mana bars, names, and combat effects
  all use Bevy 2D render components in this mode with deterministic foot-Y
  sorting and explicit layer bands. Gameplay, combat, AI, collision,
  matchmaking, buffs, victory, and reconnect remain server-authoritative.

- **Release-like 2D combat presentation (TASK-2D-RELEASE-VERTICAL-SLICE):**
  the five sprite heroes now cover idle, run, attack, cast, hit, and death.
  Accepted Q/W/E/R casts carry an authoritative monotonic cosmetic action
  sequence through snapshots, so local and remote clients play each one-shot
  once; HP loss interrupts with hit, death holds its last frame, and respawn
  resumes locomotion. Sprite manifest schema v2 keeps separate 8×2 locomotion
  and 8×4 action sheets with sheet/playback metadata. The same client-local
  mode now supplies art-directed billboards for towers, bases, team minions,
  normal neutrals, Wendigo, King Mutatio, and projectiles, plus bounded
  cast/hit/heal/death effects and a painted arena treatment. All visuals stay
  attached to the existing authoritative roots and do not change combat,
  collision, interpolation, AI, or map topology. The default 3D path remains
  intact. Asset contracts live in `client/assets/sprites/manifest.json` and
  `client/assets/presentation2d/manifest.json`; validate both with
  `python3 scripts/validate_sprite_assets.py --self-test`.

- **Selectable 2D sprite player visuals (TASK-2D-SPRITE-PROTOTYPE):** the
  pre-join screen selects either **3D Models** (the default and safe fallback)
  or **2D Sprites**. The sprite roster contains Mossback Teapot, Neon Axolotl
  Courier, Origami Storm Heron, Clockwork Turnip Oracle, and Void Jelly
  Astronaut; its five 2048×512 RGBA sheets follow an 8-column × 2-row contract
  (eight 6 fps idle frames, eight 12 fps run frames) declared once in
  `client/assets/sprites/manifest.json`. Sprite mode renders transparent,
  unlit camera-facing quads as children of the unchanged gameplay roots and
  chooses idle/run from local or interpolated remote movement with a 0.25 s
  idle grace. Renderer mode is client-local, while the optional validated
  sprite character id is replicated and retained through reconnect. Set
  `OMOBA_PLAYER_VISUAL_MODE=models3d|sprite2d` to choose the initial mode;
  unset or invalid values use `models3d`. Validate the offline asset contract
  with `python3 scripts/validate_sprite_assets.py --self-test`.
  The manifest and selection portrait strip now contain ten named slots. Four
  added heroes have complete runtime sheets; Orchard Comet Centaur still lacks
  its six generated runtime animation clips/sheets, so that selection remains
  an explicit release blocker rather than silently substituting another hero.

- **Matchmaking and gated match start (TASK-22):** in release mode (server
  default) players who join land in a queue; the match forms to a full 5v5
  roster, teams are assigned and balanced server-side, a 3-second countdown
  runs, and only then the match starts — a solo player cannot start an
  under-filled match. The client overlay walks through "Searching for
  match..." → "Waiting for players — X/10" → "Match found! Starting in N...".
  `OMOBA_MATCH_MODE=dev` keeps the instant-start dev flow (`make start`,
  `make server-dev`); `OMOBA_TEAM_SIZE` scales the roster (1–16 per team)
  for playtests. `make play-bots` / `make bots` fill the queue with UDP
  bots so one developer can walk the whole flow (see RUNBOOK.md).
- **Slime lane minions + visible camps (TASK-24):** lane minions are
  team-colored CC0 "Mimic Slime" models (green Classic / blue Water,
  Halloween Rising) with walk/attack animations driven by the replicated
  AI state, normalized to 0.6× hero height through the shared model-scale
  pipeline (`client/assets/minions/`, overrides keys
  `slime-green`/`slime-blue`). Decorative jungle boxes no longer spawn on
  neutral-camp or boss-pit anchors, so camps and raid bosses stand in open
  clearings instead of being hidden inside geometry.
- **Bot lane-push AI (TASK-23):** fill bots play once the match runs — each
  takes a lane, pushes its waypoints toward the enemy base, fights enemy
  players/minions with its class Q (server-authoritative ranges/cooldowns),
  sieges towers in reach, and rejoins the lane after a respawn
  (`harness/src/bot_ai.rs`). Simple nearest-target logic, no retreat or
  skill combos — built for playtesting matchmaking and basic playability.
- **Character scale normalization (TASK-20/21):** every character and boss GLB
  (legacy SDK models, roster avatars, raid bosses — authored anywhere from
  0.64 m to 2.41 m tall) is measured once in bind pose directly from the
  loaded glTF data and rescaled to the shared world-relative target height
  (default 1.15 world units, range 0.3–3.0), so all characters render at the
  same size by default (bosses keep their 3× presence multiplier). Persisted
  target heights from the legacy 0.26 scale migrate to the new default on
  load. Per-model size tweaks live in
  `client/assets/config/model_scale_overrides.json` (slug → multiplier,
  hot-reloaded while the game runs). `OMOBA_MEASURE_MODELS=1 cargo run -p
  client` runs a headless analyzer that prints the measured height table
  (`client/src/model_scale.rs`).
- **Spawn platform traversal (TASK-21):** the 46×46×0.7 base pads are
  walkable League-style — a client-side `MapLayout::terrain_height(x, z)`
  function describes the pad top plus a 6-unit ramp band that exactly matches
  four visible team-colored ramp slabs per pad. Local player gravity/jumps,
  remote players, and minions ground onto that surface; models rest on their
  measured foot offset. The server stays flat-ground authoritative (pure
  visual fake, no protocol changes).
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
  back safely (default model / Warrior).
  `OMOBA_AUTOJOIN=<class>:<slug>:<team>[:<sprite-id>]` joins without UI for
  automation.
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
- Directional sprite movement, richer tooltip UX, and balance passes over the class kits.

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
