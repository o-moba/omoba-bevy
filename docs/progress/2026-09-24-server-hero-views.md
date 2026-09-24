# 2026-09-24 — Server: hero timers and replicated views

## Goal
Roadmap step 6, second slice. `ConnectedPlayer.state` (the wire
`PlayerState`) carried derived copies next to the authoritative state: four
cooldown fields, three utility clocks and `shop_available`, refreshed at the
end of every tick by `refresh_basic_attack_cooldowns`,
`refresh_skill_cooldowns` and `refresh_utilities`, written directly from
eleven more places, and read back as gates by the utility handler. Two of
the refreshers also rewrote the real timers on the side. The seven timer
instants were loose fields on `ConnectedPlayer`.

## Changes
- `server/src/hero_timers.rs`: `HeroTimers` (`last_movement_at`,
  `last_cast_at`, `last_basic_attack_at`, `dash_ready_at`, `haste_ready_at`,
  `haste_expires_at`, `respawn_at`) with `new(now)` and `clear_cooldowns`;
  `ConnectedPlayer.timers` replaces the seven fields (mechanical rename,
  tests included; `last_seen` stays on `ConnectedPlayer` as transport
  state). Pure reads: `basic_attack_cooldown`, `basic_attack_remaining`,
  `skill_cooldown_left` (regardless of death, for the sandbox telemetry),
  `skill_cooldown_remaining`, `skill_recovery_remaining` (moved from
  `sandbox.rs`), `dash_remaining`, `haste_remaining`, `haste_active`. They
  use the refreshers' exact arithmetic (`saturating_sub` on `Duration`,
  `as_secs_f32`) and their exact gating (dead or `no_cooldowns` reads as
  ready for the combat clocks; `no_cooldowns` for the utility cooldowns;
  dead for haste).
- `normalize_hero_timers(world)`: the side effects the refreshers used to
  carry (clear `last_basic_attack_at` when dead or `no_cooldowns`, clear
  `dash_ready_at`/`haste_ready_at` when `no_cooldowns`, clear
  `haste_expires_at` when dead), run once in `runtime/tick.rs` after
  `handle_respawns` where the refreshers ran. The utility handler no longer
  applies them at request time; the gates it uses treat those cases as
  ready, so nothing observable changed.
- `ConnectedPlayer::owner_view(now, map, phase)` (`entities.rs`): the stored
  state plus the derived fields from `hero_timers` and
  `shop::shop_is_available`. `public_view` equals it for now (redaction is
  the next slice). `snapshot::build_players_snapshot(world, now)` maps every
  joined player through `owner_view`; `broadcast_snapshots` uses it and the
  `shop_available` loop is gone. `#[cfg(test)] ServerRuntime::player_view`
  is the test accessor.
- Removed: `refresh_basic_attack_cooldowns`, `refresh_skill_cooldowns`,
  `refresh_utility`/`refresh_utilities`, and the direct writes of the copy
  fields in `basic_attack.rs` (strike), `sim/cast.rs` (both casts),
  `session.rs` (`reset_player_round`, `handle_respawns`), `sandbox.rs`
  (`clear_cooldowns`, `apply_actor`), `combat_feedback.rs` (death),
  `career_runtime.rs` (leave, disconnect). The stored `PlayerState` keeps the
  fields at their defaults.
- Utility gates read `hero_timers::dash_remaining` / `haste_remaining`; the
  cast recovery gate reads `hero_timers::skill_recovery_remaining`; the
  sandbox telemetry reads `hero_timers::skill_cooldown_left`.
- Tests: new `server/src/tests/player_view.rs` drives a `ServerRuntime`
  through join, purchase, skill upgrade, cast, basic attack, dash, haste,
  death, respawn and (second test) sandbox `apply_actor` with and without
  `no_cooldowns` plus forced casts, and asserts the view's JSON bytes equal
  the stored state plus hand-computed remaining seconds at known offsets
  (`cooldown - elapsed` from `shared::hero_balance`). Tests that read the
  stored copies now read `rt.player_view(addr, now)` or the helpers; one
  test that seeded `dash_remaining_secs`/`haste_remaining_secs` on a
  reconnecting player dropped those two lines (the instants it also seeds
  are what is preserved).

## Behaviour
- Wire format and snapshot bytes unchanged for every recipient. During
  development the characterization test also asserted the helpers against
  the old stored copies at every step before the copies were removed.
- `shop::handle_purchase` still passes `player.timers.last_movement_at` as
  the sandbox `apply_actor` `now` (unchanged quirk; that call never moves or
  resets the actor, so the value is unused).

## Checks
- `cargo test -p server`: 276 passed, 3 ignored (274 before; two new tests).
- `cargo fmt --all`, `cargo clippy --workspace --all-targets --no-deps -- -D warnings`,
  `cargo test -p shared -p server`, `cargo test -p harness --no-run`,
  `cargo test -p client --lib` are green.

## Left for the next slices
- Redaction: `public_view` hides what other teams should not see; the
  broadcast then picks the view per recipient.
- `HeroEconomy` / `Hero` core split of `ConnectedPlayer`, `StatModifiers`.
