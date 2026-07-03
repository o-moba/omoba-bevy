# 2026-07-03 — Playable demo: hero classes + VRM avatar roster (TASK-17 phase B)

## Goal

Finish TASK-17's phase B on top of the committed avatar assets (phase A,
`ca1a560`): four playable classes with distinct Q/W/E/R kits resolved
authoritatively on the server, class + avatar + team selection in the pre-join
flow, roster avatars animating in-game for local and remote players, and the
full quality gate (build/test/clippy, docs, version 0.4.0).

## Changes

- `shared` crate (new workspace member; now a path dependency of `client` and
  `server`):
  - `HeroClass` (Warrior/Mage/Ranger/Cleric) with 4 distinct abilities each
    (16 `AbilityDefinition`s), one effect primitive per ability (projectile
    damage / self heal / self mana restore), `max_rank = 3`, and the existing
    `rank_effect_scale` / `scaled_mana_cost` / `scaled_cooldown` /
    `scaled_cast_range` / `unlocked_slots_for_level` mechanics unchanged.
  - Wire-safe encoding: `HeroClass` serializes as a snake_case string and
    decodes unknown ids as the default class instead of failing the packet.
  - Avatar roster embedded at compile time from
    `client/assets/avatars/manifest.json` (`avatar_roster()`,
    `avatar_definition()`, `normalize_avatar_slug()`), so client and server
    agree on the shipped set and unknown slugs normalize to `None`.
- `server`:
  - `ClientPacket::Join` gained `hero_class` + `avatar` (serde defaults);
    `ClientPacket::Cast` gained a `slot` index; `PlayerState` replicates
    `hero_class` + `avatar` to every client.
  - `handle_cast_request` resolves the caster's class kit authoritatively:
    unlock gating by level, per-slot cooldowns (`last_cast_at: [Option<Instant>; 4]`),
    rank-scaled mana/cooldown/range/damage, self-target heals/mana restores
    without projectiles. Skill upgrades cap at the shared max rank (3).
  - Legacy single-spell balance constants removed from `balance.rs` (numbers
    live in the shared kits now).
- `client`:
  - Pre-join flow: class buttons (name + kit tagline), 16-avatar thumbnail
    grid, then team; join carries class + avatar; `OMOBA_AUTOJOIN` env hook
    joins without UI for automation/evidence.
  - Roster avatar models load lazily (`AvatarAssetCache` +
    `PlayerModelResolver`), spawn for local and remote players, and feed the
    existing idle/walk animation graph via a generalized `AvatarKey`
    (legacy character or roster slug). Idle-grace hysteresis (0.25 s) stops
    Walk/Idle flapping caused by snapshot interpolation. The VRM double-sided
    material fix now covers every roster avatar (previously Paco-only).
  - Slot-aware casting with per-slot local cooldowns; hotbar shows the class's
    ability names + ranks; the match HUD lists each slot's ability with effect
    numbers and cooldown/lock state.
- `harness`: protocol mirror extended (class + avatar on join, slot on cast,
  replicated `hero_class`/`avatar` in `PlayerState`); new end-to-end tests:
  two clients join with different class+avatar and both see both loadouts with
  class-distinct Q mana costs; locked slots are rejected; hostile
  class/avatar values fall back without crashing the server.
- Assets/scripts: 12 roster thumbnails were JPEG data with a `.png` name —
  renamed to `.jpg`, manifest updated, `scripts/stage_avatars.py` now sniffs
  the magic bytes, and the client enables Bevy's `jpeg` feature.
- Workspace: version 0.3.0 -> 0.4.0; `shared` added to members; Bevy-style
  clippy lints (`type_complexity`, `too_many_arguments`, `collapsible_if`)
  allowed workspace-wide so `clippy -D warnings` gates real issues (the
  pre-existing code was not clean under clippy 1.93); remaining findings
  fixed.

## Checks

- `cargo build --workspace` — pass (`.agent/tasks/TASK-17-.../raw/build.txt`).
- `cargo test --workspace` — pass: 25 client + 29 server + 7 shared + 5 skills
  unit tests, 7 harness end-to-end scenarios (`raw/test-workspace.txt`,
  `raw/test-unit.txt`, `raw/test-integration.txt`).
- `cargo clippy --workspace --all-targets -- -D warnings` — pass (`raw/lint.txt`).
- `python3 scripts/validate_avatar_assets.py` — PASS after thumbnail renames
  (`raw/validate-avatars-phaseb.txt`).
- Live run (real server + real client with `OMOBA_AUTOJOIN=mage:agnes:green` +
  scripted UDP bot joining as cleric/cool-tiger and pacing): server log shows
  both joins with class + avatar; client log shows roster models loading,
  "Animation set ready for Roster(...)" for both avatars, and Idle<->Walk
  transitions tracking the bot's movement phases
  (`raw/ac5-server.log`, `raw/ac5-client.log`, `raw/ac5-bot.log`).

## Remaining risks

- Attack/cast/death clips are embedded in every roster avatar but not yet
  triggered by gameplay events (stretch scope, tracked as a release gap).
- Remote players keep the model they spawned with; a mid-session avatar change
  (only possible via rejoin today) refreshes the animation but not the mesh —
  same limitation as before for legacy characters.
- Class kit numbers are first-pass balance; no balance testing beyond the
  authoritative-mechanics tests.
- macOS screen-capture permission blocked screenshots in the evidence run;
  AC5 evidence is log-based (allowed by the frozen spec).
