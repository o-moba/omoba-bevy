# 2026-09-24 — Server: `StatModifiers` / `hero_stats.rs` and snapshot redaction

## Goal
Roadmap step 6, final slice, in two parts. Part A: the sandbox overlay
(`ConnectedPlayer.sandbox: Option<ActorConfig>`, `sandbox_infinite_hp`,
`god_mode`, `speed_mult`) was consulted ad hoc all over the crate, the
effective-stat formulas lived in `sandbox.rs`, and the max-HP/mana pools
were computed three ways. One overlay struct and one formula module replace
them, with no wire change. Part B: `public_view` finally redacts, and the
broadcast picks the view per recipient. This is the one wire-visible step.

## Part A: `hero_stats.rs`
- `StatModifiers` (`ConnectedPlayer.modifiers`): `damage_mult`,
  `attack_speed_mult`, `move_speed_mult`, `armor`, `resistance`,
  `base_max_hp: Option<f32>`, `god_mode`, `infinite_hp`, `infinite_resource`,
  `no_cooldowns`, `unlock_all`, `bypass_vision`, `grant_xp`, `respawns`.
  `Default` is normal play (multipliers 1, no mitigation, no override, every
  rule enforced). It sits on `ConnectedPlayer` because it replaces fields
  that lived there and follows the same reconnect and round-reset lifecycle
  (`reset_player_round` resets it whole).
- Formulas, each the only copy: `combat_bonuses` (item multipliers floored at
  one, then scaled by the modifiers), `basic_attack_damage`,
  `basic_attack_cooldown`, `ability_cooldown`, `skill_recovery`,
  `attack_speed` (telemetry), `move_speed` (bots, sandbox step, telemetry),
  `movement_envelope` (the transform authority; keeps the historical factor
  order so accepted positions are bit-identical), `max_hp`, `max_mana`,
  `resize_pools`, `mitigate`.
- Callers: `hero_timers` (cooldowns, recovery), `sim/cast.rs` (unlock,
  resource, cooldown gates, vision bypass, ability damage bonus),
  `basic_attack.rs` (cooldown gate, vision bypass, damage),
  `combat_feedback.rs` (god mode, mitigation, infinite HP), `sim/mod.rs`
  (`restore_god_mode_players`), `sim/minions.rs` and `sim/neutrals.rs`
  (`grant_xp`), `session.rs` (envelope, respawn flag, reset, pools),
  `runtime/dispatch.rs` (toggles), `career_runtime.rs` (eligibility),
  `bots.rs` (dummy base HP, movement step), `balance_probe.rs`, `shop.rs`.
- `sandbox.rs`: `SandboxRuntime::apply_actor(p, c, reset, now)` converts the
  `ActorConfig` into modifiers (`bypass_vision: true`, `grant_xp: false`,
  `respawns: !is_bot`, `base_max_hp: Some(c.max_hp)`, the dummy's
  `infinite_hp` applied after the sync as before) and the loadout, and
  stores the config in `SandboxRuntime.actors` by hero id.
  `actor_config(p)` is the telemetry echo: the stored config with ranks,
  inventory, god mode and move speed read back from the hero, which
  replaces the hand-copies in `apply_skill_upgrade`, the debug toggles and
  the shop. `AddXp`, `GrantItem` and the resets seed from it. The `sync`
  disable path drops the stored config with the actor. `economy.item_bonuses`
  is now the pure item value; the sandbox multipliers are modifiers and
  `combat_bonuses` combines them, so the owner view's `item_bonuses` field
  is `combat_bonuses(self)` (same bytes as the folded value it carried).
- Debug toggles: `SetGodMode` writes `modifiers.god_mode` and, outside the
  Combat Sandbox, `modifiers.infinite_resource` (the toggle always meant
  invulnerability plus a full pool; a sandbox actor's resources stay its
  own setting). `SetSpeedBoost` writes `modifiers.move_speed_mult`.
  `restore_god_mode_players` restores HP and cancels the respawn on
  `god_mode` and refills mana on `infinite_resource`.

### The three max-pool computations
- Sandbox: `c.max_hp + (level - 1) * LEVEL_UP_HP_BONUS + items.max_hp`,
  mana `MAX_MANA + (level - 1) * LEVEL_UP_MANA_BONUS + items.max_mana`.
- Normal play: `base_hp(class)` at join, `+ LEVEL_UP_HP_BONUS` per level-up,
  `+ item.max_hp` per purchase (same for mana from `MAX_MANA`).
- Practice dummy: `DUMMY_MAX_HP` flat at spawn (level 1, no items).
All three agree with `max_hp(player) = base_max_hp.unwrap_or(base_hp(class))
+ (level - 1) * LEVEL_UP_HP_BONUS + items.max_hp` (the constants are
integers, so the sums are exact in any order); the dummy is the override
`base_max_hp = Some(DUMMY_MAX_HP)`. `reset_player_round`, `apply_actor`
(through `resize_pools`) and the dummy use the closed forms. Level-ups and
purchases keep adding their delta to `hero.max_hp` incrementally: tests
hand-set pools (`forest_pickups`, `objective_balance_tests`) and the
pre-join placeholder starts from the legacy `MAX_HP`, so recomputing from
the closed form there would change behaviour, not just move code.

## Part B: redaction
- `ConnectedPlayer::public_view` = `owner_view` with `gold`, `earned_gold`,
  `inventory`, `item_bonuses`, `last_purchase`, `basic_attack_request_id`
  and `utility.last_request_id` at their defaults. Level, XP, ranks and the
  cooldown copies stay public (the sandbox UI reads a remote actor's XP;
  hiding enemy cooldowns is a separate product decision).
- `snapshot::build_players_snapshot(world, recipient: Option<u64>, now)`:
  the recipient's entry through `owner_view`, everyone else through
  `public_view`; `broadcast_snapshots` calls it per recipient, so the
  sandbox (which skips the vision filter) is redacted too.
- Client check (read-only): `PlayerEquipment` is built only for the local
  player (`net/apply.rs` skips `player.id == your_id` for remote entities),
  `targeting.rs` and `mobile_controls.rs` read the local equipment, the
  scoreboard (`edge_hud.rs`) reads `LiveScoreboard`, whose earned gold comes
  from the round ledger. `harness/` tests read only their own player;
  `harness/src/bin/bots.rs` logs every player's economy (zeros for others).

## Tests
- `vision/tests.rs`: `visible_enemy_is_replicated_with_its_private_economy_blanked`
  (position, HP, level, ranks, dash sequence intact; entry equals
  `public_view`; restoring the blanked fields gives the owner view byte for
  byte) and `recipient_gets_its_own_owner_view_and_teammates_are_redacted`
  (own entry byte-identical to `owner_view`, teammate equals `public_view`,
  the unfiltered player list the sandbox broadcast uses is redacted too).
- `tests/player_view.rs` now asserts `public_view` is the redacted owner
  view at every step, and compares the sandbox actor through
  `SandboxRuntime::actor_config`. Everything else is a mechanical rename
  (`player.god_mode` → `player.modifiers.god_mode`, `speed_mult` →
  `modifiers.move_speed_mult`, `sandbox::effective_*` → `hero_stats::*`,
  `build_players_snapshot` recipient argument).

## Behaviour
- Part A: snapshot bytes, sandbox telemetry and accepted positions are
  unchanged. Two intentional side effects: a sandbox purchase no longer
  re-applies the whole `ActorConfig`, so it no longer resets the actor's
  unspent skill points; development god mode now also skips the mana cost
  of a cast (its pool was already refilled every tick and the recovery
  window allows one cast per tick, so no accepted or rejected request
  changes).
- Part B: non-owners receive the private economy and request marks at
  their serde defaults. No protocol version bump.

## Checks
- `cargo test -p server`: 276 passed, 3 ignored before; 278 passed, 3
  ignored after (two tests added).
- `cargo fmt --all`, `cargo clippy --workspace --all-targets --no-deps -- -D warnings`,
  `cargo test -p shared -p server`, `cargo test -p harness --no-run`,
  `cargo test -p client --lib` are green.
