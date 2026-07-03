# 2026-07-03 — TASK-19: Raid Bosses (Epic Neutral Objectives with Team Buffs)

Branch: `task/TASK-19-raid-bosses` (worktree; version bump deferred to merge time).

## Goal

Add two MOBA-style epic neutral bosses on top of the jungle-neutral system —
**Wendigo** (bottom pit, dragon slot) and **King Mutatio** (top pit, Baron
slot) — with scheduled spawns, boss-grade stats, per-type respawn, and
replicated team-wide buffs on kill; render them on the client with staged CC0
models, HP bar, nameplate, animation, and a HUD buff indicator.

## Changes

### Assets (`client/assets/bosses/`, scripts)
- Staged `wendigo-hollow.glb` (Halloween Rising, Polygonal-Mind, CC0) and
  `king-mutatio.glb` (ToxSam, CC0) with thumbnails and a dedicated
  `manifest.json` (same schema as the avatar roster). Retargeted
  idle/walk/attack/cast/death clips via the existing pipeline (the Wendigo's
  nonstandard `Hips2` node resolves through the VRM humanoid map).
- `scripts/stage_avatars.py`: new `--set bosses` staging set; roster behavior
  unchanged. `scripts/validate_avatar_assets.py`: `--roster-min/--roster-max`
  flags (defaults keep the 10..20 roster window); the boss dir validates with
  exactly 2 entries. Boss slugs stay out of the player roster manifest, so
  `normalize_avatar_slug` rejects them by construction (shared test).

### Server
- `NeutralCampType::{WendigoBoss, KingMutatioBoss}` (wire: `wendigo_boss`,
  `king_mutatio_boss`); boss templates in `neutral_template()`;
  `boss_blueprints()` places the pits at 180°-symmetric points using the same
  map formula as the camps. Bosses build dormant (`hp = 0`) and are armed at
  the Lobby→Running transition via `schedule_boss_spawns` (`dead_until = now +
  delay`), so the existing respawn machinery spawns them on schedule at full
  HP.
- Named constants in `balance.rs`: spawn delays 60 s / 180 s, boss respawn
  180 s (camps keep 40 s via a per-type cooldown), Wendigo 900 HP / 20 dmg /
  150 g / 200 xp, Mutatio 1500 HP / 28 dmg / 250 g / 300 xp, boss leash 18.
- `TeamBuffs` state keyed by (team, kind) with absolute expiries:
  `WendigoFavor` (+15% ability damage, 90 s) and `MutatioMight` (+25% ability
  damage + 2 HP/s regen, 90 s). Kill grants/refreshes the killer's team buff in
  `apply_neutral_damage`; `handle_cast_request` multiplies outgoing ability
  damage by the team's active multipliers (multiplicative stacking);
  `regenerate_team_buff_hp` ticks the regen clamped to max HP. Replicated via
  a new `#[serde(default)] team_buffs` snapshot field. `reset_match` clears
  buffs and restarts the boss schedule.

### Client
- New `bosses.rs`: preloads the two boss GLBs, attaches the scene (scaled
  ~3x player height via `NormalizeModelScale::scaled_by`) plus a projected
  screen-space nameplate, drives idle/walk from the replicated neutral AI
  state, and force-patches VRM materials double-sided. `net.rs` mirrors the
  new camp types + `team_buffs` and spawns boss neutrals without the sphere
  mesh. `match_hud.rs` shows the local team's active buffs with remaining
  seconds (logged on transitions for evidence runs).

### Harness
- Protocol mirror gains `neutrals` + `team_buffs` snapshot parsing; new
  integration test `bottom_boss_spawns_on_schedule_with_boss_stats` (boss
  absent early, appears ~60 s with 900/900 HP in the bottom pit, top boss
  still gated — per spec A5, kill/buff/respawn timing stays in unit tests).

## Checks

- `cargo build --workspace`, `cargo test --workspace` (incl. harness),
  `cargo clippy --workspace --all-targets -- -D warnings` — all green.
- Server unit tests cover: schedule gating/spawn, pit symmetry, boss respawn
  cooldown vs camp cooldown, buff apply/expiry/refresh/team scoping,
  authoritative buffed damage (single + multiplicative), regen clamping,
  rematch reset, wire formats, boss leash behavior.
- Asset validation passes for `client/assets/avatars` (unchanged invocation)
  and `client/assets/bosses` (`--roster-min 2 --roster-max 2`).
- Live evidence run (server + autojoin client + scripted killer bot): bottom
  boss visible at ~60 s, killed by Q casts, team buff replicated and shown in
  the HUD, top boss visible at ~180 s. Logs in
  `.agent/tasks/TASK-19-raid-bosses/raw/`.

## Remaining risks / follow-ups

- Boss attack/death clips are embedded but not gameplay-triggered (idle/walk
  only), matching the roster-avatar status quo.
- No minimap boss icons or kill announcements (explicit non-goals).
- Final `[workspace.package].version` bump and CHANGELOG release header happen
  at merge time per branch policy.
