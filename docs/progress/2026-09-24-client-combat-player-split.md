# 2026-09-24 — Client combat and player modules (roadmap step 10, slices 10b + 10c)

## Goal
Split the two largest gameplay files of the client, `combat.rs` (3836
lines) and `player.rs` (3135 lines), into module trees along the tables in
[plans/client-10-15.md](../plans/client-10-15.md) section 10.3. This is the
same kind of change as the `net.rs` → `net/` split (#23): verbatim moves,
only `use` lines and visibility change. No behaviour change and no wire
change; `shared/` and `server/` are untouched.

## 10b: `client/src/combat/`
| File | Lines | Contents |
| --- | ---: | --- |
| `mod.rs` | 130 | module list, re-exports, `CombatPlugin`, `configure_target_presentation` |
| `cooldown.rs` | 137 | `LocalCastCooldown`, `tick_local_cast_cooldown`, `sync_authoritative_cooldown_durations`, `effective_cast_duration`, `local_hero_class` |
| `feedback.rs` | 53 | `ActionFeedback`, `ActionFeedbackText`, `update_action_feedback`, `adapt_mobile_combat_feedback` |
| `round_reset.rs` | 60 | `CombatRoundIdentity`, `reset_round_input_state` |
| `selection.rs` | 529 | pick radii, `TargetState`, `WorldPointerState`, `TargetCandidates`, `select_target_system`, `find_nearest_enemy_target`, `find_target_near_screen`, `consider_screen_target`, `screen_pick_distance` |
| `cast.rs` | 418 | `PendingCastRequest`, `PendingCast`, `try_cast_slot`, `queue_cast_request`, `cast_spell_system`, `resolve_pending_cast_system`, `within_cast_range` |
| `mobile.rs` | 296 | `mobile_utility_system`, `mobile_cast_system`, `mobile_assisted_target`, `mobile_target_score` |
| `hotbar.rs` | 457 | skill slot constants and colours, slot components, `setup_combat_ui`, `update_skill_bar_system`, `skill_upgrade_input_system`, `skill_button_system` |
| `bars.rs` | 407 | bar constants, `CombatVisualAssets`, `CombatBars`, `CombatBarRoot`, `CombatBarAnchor`, `setup_combat_visual_assets`, the three bar systems, `compute_bar_world_y_for_entity` |
| `marker.rs` | 92 | marker constants, `TargetMarker`, `update_target_marker_system` |
| `targeting.rs` | 1739 | `client/src/targeting.rs`, byte-for-byte |
| `tests.rs` | 1395 | the 22 former inline tests |
| `target_presentation_tests.rs` | 238 | the 4 presentation tests (moved from `client/src/`) |

- `lib.rs` drops `mod targeting;` and adds `pub(crate) use combat::targeting;`,
  so every `crate::targeting::…` path (including the ones inside
  `combat/`) still resolves. `combat/mod.rs` declares it as
  `pub(crate) mod targeting;`.
- `mod.rs` re-exports what other modules import from `crate::combat`:
  `CombatStats`/`MAX_HP` (domain), `CombatPointerInputSet`/`WorldMovementInputSet`
  (input context), `LocalCastCooldown`, `effective_cast_duration`,
  `ActionFeedback`, `TargetState`, `WorldPointerState`, `TargetCandidates`,
  `PendingCast`, `CombatBarAnchor`. Each re-export keeps the item's own
  visibility (`pub` or `pub(crate)`).
- The plugin body is unchanged: same `add_systems` calls, order, sets and
  run conditions.
- `#[path = "target_presentation_tests.rs"]` is gone because the file now
  sits at the default location next to `mod.rs`.

## 10c: `client/src/player/`
| File | Lines | Contents |
| --- | ---: | --- |
| `mod.rs` | 89 | module list, re-exports, `DebugSpeedBoost`, `PLAYER_SIZE`/`JUMP_*`/`GRAVITY`/`GROUND_EPSILON`/`RESPAWN_DELAY_SECONDS`, `ground_origin_y`, `PlayerPlugin` |
| `input.rs` | 372 | `handle_player_input`, `move_player_mobile`, `mobile_screen_direction`, `secondary_move_pressed`, `plan_movement_routes`, `should_issue_ground_move`, `viewport_to_simulation_world` |
| `motion.rs` | 385 | `Jumping`, `move_player`, `SandboxVisualClock`, `animate_jump`, `apply_gravity`, `clip_static_movement`, `structure_revision`, `resolve_player_collisions`, `resolve_player_structure_overlap`, `hero_movement_multiplier` |
| `respawn_ui.rs` | 133 | `RespawnCountdown`, `RespawnCountdownText`, `setup_respawn_ui`, `respawn_countdown_system` |
| `animation.rs` | 951 | `register_hero_animation_systems`, `AvatarKey`, the animation library, sets, binding, state and playback, `start_hero_animation`, library setup, `sync_jump_fallback_mode`, humanoid requests, binding, sandbox preview/seek helpers, `sync_player_animation_state` |
| `tests.rs` | 501 | the 14 former `tests` |
| `animation_tests.rs` | 776 | the 12 former `animation_tests` |

- `mod.rs` re-exports `Player`, `PlayerBody`, `VerticalVelocity`,
  `MovementTarget`, `MovementRoute` (domain), `DEBUG_SPEED_MULTIPLIER`,
  `PLAYER_SPEED`, and from the new files `PlayerAnimationBinding`,
  `register_hero_animation_systems` (used by `animation_qa.rs`),
  `mobile_screen_direction` and `viewport_to_simulation_world`.
- `animation.rs` is over the ~900-line guide; it is kept as one file as the
  plan allows.

## Visibility
Nothing became `pub(crate)` or `pub`. Private items became `pub(super)`
(= `pub(in crate::combat)` / `pub(in crate::player)`) when another new
file or a test module uses them: the systems the plugins register, helpers
shared between files (`local_hero_class`, `queue_cast_request`,
`within_cast_range`, `avatar_key`, `hero_movement_multiplier`, …), and the
components/resources/constants that tests touch. Six types became
`pub(super)` only because `pub(super)` systems name them in their
signatures (`private_interfaces`): `CombatVisualAssets`, `CombatBars`,
`CombatBarRoot`, `SkillBarSlot`, `SkillUpgradeButton`, `SkillNameLabel`.

Fields and methods that became `pub(super)`:
- combat: `LocalCastCooldown.{total_secs, recovery_secs, pending_slot,
  prediction_grace_secs}`, `ActionFeedback.remaining`,
  `TargetState.marker_entity`, `PendingCast.request`, all four
  `PendingCastRequest` fields, `DesktopSkillIcon.slot`, `SkillRankLabel.slot`.
- player: `RespawnCountdown.{end_time, last_shown, last_hp}`,
  `Jumping.timer`, `SandboxVisualClock::delta`,
  `PlayerAnimationLibrary.sets` and `::should_use_jump_fallback`, every
  `CharacterAnimationSet` field (the tests build one) and `::node`,
  `PlayerAnimationBinding.playback`, `HeroAnimationPlayback.{state, alive}`
  and `::{new, observe_round, advance}`.

## Tests
- The test modules moved as files; their bodies and names are unchanged.
  They keep their `use super::*;` and gained `use super::<file>::{…}` and
  `use crate::…` lines for what the old single file used to import for
  them. Five statements in `combat/tests.rs` and one in `player/tests.rs` were
  re-wrapped by `rustfmt` after the four-space dedent.
- Client lib: 549 before and after. The sorted `--list` output differs
  only in the 16 `targeting::tests::…` names, now
  `combat::targeting::tests::…`. `combat::tests::…`,
  `combat::target_presentation_tests::…`, `player::tests::…` and
  `player::animation_tests::…` keep their names.
- Shared: 78, unchanged.

## Checks
- `cargo fmt --all -- --check` clean.
- `cargo clippy --workspace --all-targets --no-deps -- -D warnings` clean.
- `cargo test -p client --lib`: 549 passed. `cargo test -p shared`: 78 passed.

## Notes for the next slices
- 10e (plugin groups) can add `CombatPlugin`/`PlayerPlugin` as they are; the
  submodules do not register anything themselves.
- 10i (optional) can switch callers from the `crate::targeting` shim and
  the domain re-exports to their new paths.
